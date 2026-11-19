use crate::{
    catalog::Catalog, document::Document, jobs::JobRequest, neb::frame,
    settings::CalculationSettings, transition::EndpointMatch,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    Mp2,
    Ccsdt,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    pub source_job: String,
    pub path_job: String,
    pub settings: CalculationSettings,
    pub engine_version: String,
    // Reactant, product, saddle.
    pub energies_hartree: [f64; 3],
}

impl Baseline {
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.settings.validate()?;
        if uuid::Uuid::parse_str(&self.source_job).is_err()
            || uuid::Uuid::parse_str(&self.path_job).is_err()
            || self.energies_hartree.iter().any(|x| !x.is_finite())
            || self.settings.method == crate::settings::SCREENING
            || self.engine_version.is_empty()
            || self.engine_version.len() > 64
        {
            return Err("refinement_source".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub method: Method,
    pub baseline: Baseline,
    pub reactant: Vec<[f64; 3]>,
    pub product: Vec<[f64; 3]>,
}

impl Input {
    pub fn validate(&self, request: &JobRequest) -> Result<(), String> {
        if request.optimize
            || request.neb.is_some()
            || request.transition.is_some()
            || request.thermochemistry.is_some()
            || request.method != "RHF"
            || request.document.spin != 0
            || !(2..=10).contains(&request.document.atoms.len())
        {
            return Err("refinement_scope".into());
        }
        self.baseline.validate()?;
        frame(&request.document, &self.reactant)?;
        frame(&request.document, &self.product)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub method: Method,
    pub baseline: Baseline,
    pub energies_hartree: [f64; 3],
    pub reference_hartree: [f64; 3],
    pub triples_hartree: [f64; 3],
    pub orbital_count: usize,
}

impl Report {
    pub fn validate(&self, doc: &Document, energy: f64) -> Result<(), String> {
        self.baseline.validate()?;
        if doc.spin != 0
            || !(2..=10).contains(&doc.atoms.len())
            || !(2..=64).contains(&self.orbital_count)
            || self.energies_hartree[2] != energy
            || self
                .energies_hartree
                .iter()
                .chain(&self.reference_hartree)
                .chain(&self.triples_hartree)
                .any(|value| !value.is_finite())
            || (self.method == Method::Mp2 && self.triples_hartree != [0.0; 3])
        {
            return Err("result_invalid".into());
        }
        Ok(())
    }
}

pub fn from_transition(
    store: &Catalog,
    id: &str,
    revision: u64,
    method: Method,
    basis: String,
) -> Result<JobRequest, String> {
    let path = verified_path(store, id)?;
    let mut request = JobRequest::with_settings(
        path.document,
        revision,
        CalculationSettings {
            basis,
            ..Default::default()
        },
    )?;
    request.refinement = Some(Input {
        method,
        baseline: path.baseline,
        reactant: path.reactant,
        product: path.product,
    });
    request.validate()?;
    Ok(request)
}

pub(crate) struct VerifiedPath {
    pub document: Document,
    pub baseline: Baseline,
    pub seed_image: usize,
    pub reactant: Vec<[f64; 3]>,
    pub product: Vec<[f64; 3]>,
}

pub(crate) fn verified_path(store: &Catalog, id: &str) -> Result<VerifiedPath, String> {
    let project = store.restore(id)?;
    let result = project.calculation.ok_or("refinement_source")?;
    let ts = result.transition.ok_or("refinement_source")?;
    if ts.imaginary_count != 1
        || ![EndpointMatch::Reactant, EndpointMatch::Product]
            .iter()
            .all(|endpoint| {
                ts.branches
                    .iter()
                    .any(|branch| branch.converged && branch.endpoint.as_ref() == Some(endpoint))
            })
    {
        return Err("refinement_source".into());
    }
    let source = store.restore(&ts.source_job)?;
    let source_result = source.calculation.ok_or("refinement_source")?;
    let band = source_result
        .neb
        .filter(|path| path.climb)
        .ok_or("refinement_source")?;
    if source.settings != project.settings || source_result.engine_version != result.engine_version
    {
        return Err("refinement_source".into());
    }
    Ok(VerifiedPath {
        document: project.document.ok_or("refinement_source")?,
        seed_image: ts.seed_image,
        baseline: Baseline {
            source_job: id.into(),
            path_job: ts.source_job,
            settings: project.settings,
            engine_version: result.engine_version,
            energies_hartree: [
                band.images[0].energy_hartree,
                band.images[4].energy_hartree,
                result.energy_hartree,
            ],
        },
        reactant: band.positions[0].clone(),
        product: band.positions[4].clone(),
    })
}
