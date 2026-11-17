use crate::{document::Document, jobs::JobRequest, neb::frame};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionInput {
    pub source_job: String,
    pub seed_image: usize,
    pub reactant: Vec<[f64; 3]>,
    pub product: Vec<[f64; 3]>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EndpointMatch {
    Reactant,
    Product,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Branch {
    pub converged: bool,
    #[serde(rename = "match")]
    pub endpoint: Option<EndpointMatch>,
    pub reactant_rmsd_angstrom: f64,
    pub product_rmsd_angstrom: f64,
    pub max_force_ev_per_angstrom: f64,
    pub steps: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TransitionResult {
    pub sella_version: String,
    pub steps: u32,
    pub frequencies_cm1: Vec<f64>,
    pub imaginary_count: usize,
    pub branches: Vec<Branch>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positions_angstrom: Vec<[f64; 3]>,
    pub max_force_ev_per_angstrom: f64,
    pub source_job: String,
    pub seed_image: usize,
}

impl TransitionInput {
    pub fn validate(&self, request: &JobRequest) -> Result<(), String> {
        if request.optimize
            || request.neb.is_some()
            || request.method == crate::settings::SCREENING
            || !(2..=16).contains(&request.document.atoms.len())
            || !(1..=3).contains(&self.seed_image)
            || uuid::Uuid::parse_str(&self.source_job).is_err()
        {
            return Err("transition_source".into());
        }
        frame(&request.document, &self.reactant)?;
        frame(&request.document, &self.product)?;
        Ok(())
    }
}

impl TransitionResult {
    pub fn validate(&self, document: &Document) -> Result<(), String> {
        let dimension = 3 * document.atoms.len();
        if !(2..=16).contains(&document.atoms.len())
            || uuid::Uuid::parse_str(&self.source_job).is_err()
            || !(1..=3).contains(&self.seed_image)
            || self.sella_version.is_empty()
            || self.sella_version.len() > 64
            || self.steps > 35
            || !(0.0..0.02).contains(&self.max_force_ev_per_angstrom)
            || !(dimension - 6..=dimension - 5).contains(&self.frequencies_cm1.len())
            || self.frequencies_cm1.iter().any(|value| !value.is_finite())
            || self.imaginary_count
                != self
                    .frequencies_cm1
                    .iter()
                    .filter(|&&value| value < -20.0)
                    .count()
            || self.branches.len() != if self.imaginary_count == 1 { 2 } else { 0 }
            || !self
                .positions_angstrom
                .iter()
                .copied()
                .eq(document.atoms.iter().map(|atom| atom.position_angstrom))
        {
            return Err("result_invalid".into());
        }
        for branch in &self.branches {
            let a = branch.reactant_rmsd_angstrom;
            let b = branch.product_rmsd_angstrom;
            let f = branch.max_force_ev_per_angstrom;
            if [a, b, f]
                .iter()
                .any(|&value| !value.is_finite() || value < 0.0)
                || branch.steps > 121
                || (branch.converged && f >= 0.03)
            {
                return Err("result_invalid".into());
            }
            let expected = if branch.converged && a.min(b) <= 0.35 && (a - b).abs() >= 0.05 {
                Some(if a < b {
                    EndpointMatch::Reactant
                } else {
                    EndpointMatch::Product
                })
            } else {
                None
            };
            if branch.endpoint != expected {
                return Err("result_invalid".into());
            }
        }
        Ok(())
    }
}

pub fn from_path(
    store: &crate::catalog::Catalog,
    id: &str,
    revision: u64,
) -> Result<JobRequest, String> {
    let project = store.restore(id)?;
    let result = project.calculation.ok_or("transition_source")?;
    let path = result
        .neb
        .filter(|path| path.climb)
        .ok_or("transition_source")?;
    let peak = path
        .images
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.energy_hartree.total_cmp(&b.energy_hartree))
        .ok_or("transition_source")?
        .0;
    if !(1..path.images.len() - 1).contains(&peak) {
        return Err("transition_peak".into());
    }
    let seed = frame(
        &project.document.ok_or("transition_source")?,
        &path.positions[peak],
    )?;
    let mut request = JobRequest::with_settings(seed, revision, project.settings)?;
    request.transition = Some(TransitionInput {
        source_job: id.into(),
        seed_image: peak,
        reactant: path.positions[0].clone(),
        product: path.positions[4].clone(),
    });
    request.validate()?;
    Ok(request)
}
