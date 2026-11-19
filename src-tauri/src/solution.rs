use crate::{document::Document, jobs::JobRequest, thermochemistry::Solvent};
use serde::{Deserialize, Serialize};

pub const STEPS: u32 = 50;

pub fn validate(request: &JobRequest) -> Result<(), String> {
    if request.method == "RHF"
        && request.document.spin == 0
        && (2..=10).contains(&request.document.atoms.len())
        && !request.optimize
        && request.neb.is_none()
        && request.transition.is_none()
        && request.thermochemistry.is_none()
        && request.refinement.is_none()
        && request.dynamics.is_none()
    {
        Ok(())
    } else {
        Err("solution_settings".into())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub solvent: Solvent,
    pub ase_version: String,
    pub steps: u32,
    pub max_force_ev_per_angstrom: f64,
    pub positions_angstrom: Vec<[f64; 3]>,
    pub frequencies_cm1: Vec<f64>,
    pub minimum_confirmed: bool,
}

impl Report {
    pub fn valid(&self, document: &Document) -> bool {
        let count = document.atoms.len();
        (2..=10).contains(&count)
            && self
                .positions_angstrom
                .iter()
                .copied()
                .eq(document.atoms.iter().map(|atom| atom.position_angstrom))
            && (0.0..0.01).contains(&self.max_force_ev_per_angstrom)
            && self.steps <= STEPS
            && (3 * count - 6..=3 * count - 5).contains(&self.frequencies_cm1.len())
            && !self.frequencies_cm1.is_empty()
            && self.frequencies_cm1.iter().all(|value| value.is_finite())
            && self.minimum_confirmed == self.frequencies_cm1.iter().all(|value| *value > 20.0)
            && !self.ase_version.is_empty()
    }
}
