use crate::{document::Document, jobs::JobRequest};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NebInput {
    pub climb: bool,
    pub product: Vec<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart: Option<Vec<Vec<[f64; 3]>>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub energy_hartree: f64,
    pub max_force_ev_per_angstrom: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NebResult {
    pub steps: u32,
    pub ase_version: String,
    pub climb: bool,
    pub images: Vec<Image>,
    pub max_neb_force_ev_per_angstrom: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positions: Vec<Vec<[f64; 3]>>,
}

impl NebInput {
    pub fn validate(&self, request: &JobRequest) -> Result<(), String> {
        if request.optimize
            || request.method == crate::settings::SCREENING
            || !(2..=16).contains(&request.document.atoms.len())
        {
            return Err("neb_scope".into());
        }
        request.document_at(&self.product)?;
        if let Some(images) = &self.restart {
            self.validate_frames(request, images)?;
        } else {
            for weight in [0.25, 0.5, 0.75] {
                let image: Vec<_> = request
                    .document
                    .atoms
                    .iter()
                    .zip(&self.product)
                    .map(|(atom, end)| {
                        std::array::from_fn(|axis| {
                            atom.position_angstrom[axis] * (1.0 - weight) + end[axis] * weight
                        })
                    })
                    .collect();
                request
                    .document_at(&image)
                    .map_err(|_| "neb_interpolation")?;
            }
        }
        Ok(())
    }

    pub fn validate_frames(
        &self,
        request: &JobRequest,
        frames: &[Vec<[f64; 3]>],
    ) -> Result<(), String> {
        if frames.len() != 5
            || frames[4] != self.product
            || !frames[0].iter().copied().eq(request
                .document
                .atoms
                .iter()
                .map(|atom| atom.position_angstrom))
        {
            return Err("neb_checkpoint".into());
        }
        for frame in frames {
            request.document_at(frame)?;
        }
        Ok(())
    }
}

impl NebResult {
    pub fn validate(&self, document: &Document, converged: bool) -> Result<(), String> {
        if self.steps > 50
            || self.ase_version.is_empty()
            || self.ase_version.len() > 64
            || self.images.len() != 5
            || self.positions.len() != 5
            || !self.max_neb_force_ev_per_angstrom.is_finite()
            || self.max_neb_force_ev_per_angstrom < 0.0
            || (converged && self.max_neb_force_ev_per_angstrom >= 0.05)
            || self.images.iter().any(|image| {
                !image.energy_hartree.is_finite()
                    || !image.max_force_ev_per_angstrom.is_finite()
                    || image.max_force_ev_per_angstrom < 0.0
            })
            || !self.positions[0]
                .iter()
                .copied()
                .eq(document.atoms.iter().map(|atom| atom.position_angstrom))
        {
            return Err("result_invalid".into());
        }
        for positions in &self.positions {
            frame(document, positions)?;
        }
        Ok(())
    }
}

pub fn frame(document: &Document, positions: &[[f64; 3]]) -> Result<Document, String> {
    if positions.len() != document.atoms.len() {
        return Err("result_invalid".into());
    }
    let mut image = document.clone();
    image.source = None;
    image.bonds = None;
    for (atom, position) in image.atoms.iter_mut().zip(positions) {
        atom.position_angstrom = *position;
    }
    image.validate()?;
    Ok(image)
}
