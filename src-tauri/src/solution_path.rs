use crate::{
    catalog::Catalog,
    document::Document,
    jobs::JobRequest,
    refinement::Baseline,
    thermochemistry::Solvent,
    transition::{EndpointMatch, TransitionInput, TransitionResult},
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub solvent: Solvent,
    pub baseline: Baseline,
    pub seed_image: usize,
    pub reactant: Vec<[f64; 3]>,
    pub product: Vec<[f64; 3]>,
}

impl Input {
    pub fn validate(&self, request: &JobRequest) -> Result<(), String> {
        crate::solution::validate(request)?;
        self.baseline.validate()?;
        if self.baseline.settings != request.effective_settings() {
            return Err("solution_path_source".into());
        }
        TransitionInput {
            source_job: self.baseline.path_job.clone(),
            seed_image: self.seed_image,
            reactant: self.reactant.clone(),
            product: self.product.clone(),
        }
        .validate(request)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub solvent: Solvent,
    pub baseline: Baseline,
    pub saddle: TransitionResult,
    pub endpoint_energies_hartree: [f64; 2],
    pub endpoint_minima: [bool; 2],
    pub path_verified: bool,
    pub lengths: [usize; 2],
    pub frames_sha256: String,
}

impl Report {
    pub fn validate(&self, document: &Document, energy: f64) -> Result<(), String> {
        self.baseline.validate()?;
        self.saddle.validate(document)?;
        let connected = [EndpointMatch::Reactant, EndpointMatch::Product]
            .iter()
            .all(|endpoint| {
                self.saddle
                    .branches
                    .iter()
                    .any(|branch| branch.converged && branch.endpoint.as_ref() == Some(endpoint))
            });
        let verified = connected
            && self.endpoint_minima == [true; 2]
            && self.saddle.imaginary_count == 1
            && self
                .saddle
                .frequencies_cm1
                .iter()
                .all(|value| value.abs() > 20.0)
            && self
                .endpoint_energies_hartree
                .iter()
                .all(|value| *value < energy);
        if self.saddle.source_job != self.baseline.path_job
            || self.path_verified != verified
            || self
                .endpoint_energies_hartree
                .iter()
                .any(|value| !value.is_finite())
            || self.lengths
                != std::array::from_fn(|i| {
                    self.saddle
                        .branches
                        .get(i)
                        .map_or(0, |branch| branch.steps + 1)
                })
        {
            return Err("result_invalid".into());
        }
        Ok(())
    }

    pub fn frame_count(&self) -> usize {
        self.lengths.iter().sum()
    }

    pub(crate) fn verify_frames(&self, directory: &Path, base: &Document) -> Result<(), String> {
        crate::dynamics::read_frames(directory, base, self.frame_count(), &self.frames_sha256)?;
        Ok(())
    }
}

pub fn from_transition(
    store: &Catalog,
    id: &str,
    revision: u64,
    solvent: Solvent,
) -> Result<JobRequest, String> {
    let path = crate::refinement::verified_path(store, id).map_err(|_| "solution_path_source")?;
    let mut request =
        JobRequest::with_settings(path.document, revision, path.baseline.settings.clone())?;
    request.solution_path = Some(Input {
        solvent,
        baseline: path.baseline,
        seed_image: path.seed_image,
        reactant: path.reactant,
        product: path.product,
    });
    request.validate()?;
    Ok(request)
}
