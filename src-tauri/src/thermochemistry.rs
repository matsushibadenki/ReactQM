use crate::{
    catalog::Catalog, document::Document, jobs::JobRequest, neb::frame, transition::EndpointMatch,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StandardState {
    #[default]
    Atmosphere,
    Molar,
}
impl StandardState {
    fn is_atmosphere(value: &Self) -> bool {
        *value == Self::Atmosphere
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SolventModel {
    CPcm,
    IefPcm,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Medium {
    Water,
    Acetonitrile,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Solvent {
    pub model: SolventModel,
    pub medium: Medium,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub source_job: String,
    pub temperature_kelvin: f64,
    #[serde(default, skip_serializing_if = "StandardState::is_atmosphere")]
    pub standard_state: StandardState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solvent: Option<Solvent>,
    pub reactant: Vec<[f64; 3]>,
    pub product: Vec<[f64; 3]>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Corrections {
    pub zpe_hartree: f64,
    pub gibbs_hartree: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Species {
    pub electronic_hartree: f64,
    pub frequencies_cm1: Vec<f64>,
    pub max_force_ev_per_angstrom: f64,
    pub corrections: Option<Corrections>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solvated_electronic_hartree: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub source_job: String,
    pub temperature_kelvin: f64,
    #[serde(default, skip_serializing_if = "StandardState::is_atmosphere")]
    pub standard_state: StandardState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solvent: Option<Solvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standard_shift_hartree: Option<f64>,
    pub reactant: Species,
    pub product: Species,
    pub saddle: Species,
}

impl Input {
    pub fn validate(&self, request: &JobRequest) -> Result<(), String> {
        if request.optimize
            || request.neb.is_some()
            || request.transition.is_some()
            || request.method == crate::settings::SCREENING
            || !(2..=16).contains(&request.document.atoms.len())
            || !(1.0..=2000.0).contains(&self.temperature_kelvin)
            || uuid::Uuid::parse_str(&self.source_job).is_err()
        {
            return Err("thermo_input".into());
        }
        frame(&request.document, &self.reactant)?;
        frame(&request.document, &self.product)?;
        Ok(())
    }
}

impl Report {
    pub fn validate(&self, doc: &Document, energy: f64) -> Result<(), String> {
        let dimension = 3 * doc.atoms.len();
        if !(2..=16).contains(&doc.atoms.len())
            || !(1.0..=2000.0).contains(&self.temperature_kelvin)
            || uuid::Uuid::parse_str(&self.source_job).is_err()
            || self.saddle.electronic_hartree != energy
        {
            return Err("result_invalid".into());
        }
        match (self.standard_state, self.standard_shift_hartree) {
            (StandardState::Atmosphere, None) => {}
            (StandardState::Molar, Some(shift)) => {
                let t = self.temperature_kelvin;
                let expected = 3.166811563e-6 * t * (1000.0 * 8.314462618 * t / 101325.0).ln();
                if !shift.is_finite() || (shift - expected).abs() > 1e-8 {
                    return Err("result_invalid".into());
                }
            }
            _ => return Err("result_invalid".into()),
        }
        for (species, negative) in [(&self.reactant, 0), (&self.product, 0), (&self.saddle, 1)] {
            let frequencies = &species.frequencies_cm1;
            if !(dimension - 6..=dimension - 5).contains(&frequencies.len())
                || frequencies.iter().any(|value| !value.is_finite())
                || !species.electronic_hartree.is_finite()
                || self.solvent.is_some() != species.solvated_electronic_hartree.is_some()
                || species
                    .solvated_electronic_hartree
                    .is_some_and(|value| !value.is_finite())
                || !species.max_force_ev_per_angstrom.is_finite()
                || species.max_force_ev_per_angstrom < 0.0
            {
                return Err("result_invalid".into());
            }
            if let Some(correction) = &species.corrections {
                if !correction.zpe_hartree.is_finite()
                    || correction.zpe_hartree < 0.0
                    || !correction.gibbs_hartree.is_finite()
                    || species.max_force_ev_per_angstrom >= 0.03
                    || frequencies.iter().any(|value| value.abs() <= 20.0)
                    || frequencies.iter().filter(|&&value| value < 0.0).count() != negative
                {
                    return Err("result_invalid".into());
                }
            }
        }
        Ok(())
    }
}

pub fn from_transition(
    store: &Catalog,
    id: &str,
    revision: u64,
    temperature_kelvin: f64,
    standard_state: StandardState,
    solvent: Option<Solvent>,
) -> Result<JobRequest, String> {
    let project = store.restore(id)?;
    let result = project.calculation.ok_or("thermo_source")?;
    let ts = result.transition.ok_or("thermo_source")?;
    if ts.imaginary_count != 1
        || ![EndpointMatch::Reactant, EndpointMatch::Product]
            .iter()
            .all(|endpoint| {
                ts.branches
                    .iter()
                    .any(|branch| branch.converged && branch.endpoint.as_ref() == Some(endpoint))
            })
    {
        return Err("thermo_source".into());
    }
    let source = store.restore(&ts.source_job)?;
    let source_result = source.calculation.ok_or("thermo_source")?;
    let band = source_result
        .neb
        .filter(|path| path.climb)
        .ok_or("thermo_source")?;
    if source.settings != project.settings || source_result.engine_version != result.engine_version
    {
        return Err("thermo_source".into());
    }
    let mut request = JobRequest::with_settings(
        project.document.ok_or("thermo_source")?,
        revision,
        project.settings,
    )?;
    request.thermochemistry = Some(Input {
        source_job: id.into(),
        temperature_kelvin,
        standard_state,
        solvent,
        reactant: band.positions[0].clone(),
        product: band.positions[4].clone(),
    });
    request.validate()?;
    Ok(request)
}
