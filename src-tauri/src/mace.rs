use crate::document::Document;
use serde::{Deserialize, Serialize};

pub const METHOD: &str = "MACE-OFF23";
pub const MODEL: &str = "small";
const CHECKPOINT: &str = "165cce4cfec5a34b9c64d4ebf95de15d71106bb584b7291c8470f0749977c46f";

fn radius(z: u8) -> Option<f64> {
    Some(match z {
        1 => 0.31,
        6 => 0.76,
        7 => 0.71,
        8 => 0.66,
        9 => 0.57,
        15 => 1.07,
        16 => 1.05,
        17 => 1.02,
        35 => 1.20,
        53 => 1.39,
        _ => return None,
    })
}

pub fn validate_scope(document: &Document) -> Result<(), &'static str> {
    if document.charge != 0 {
        return Err("mace_requires_neutral");
    }
    if document.spin != 0 {
        return Err("mace_requires_singlet");
    }
    let atoms = &document.atoms;
    if !(2..=64).contains(&atoms.len()) {
        return Err("mace_atom_count");
    }
    let radii = atoms
        .iter()
        .map(|atom| radius(atom.atomic_number).ok_or("mace_unsupported_element"))
        .collect::<Result<Vec<_>, _>>()?;
    if atoms
        .iter()
        .map(|atom| u32::from(atom.atomic_number))
        .sum::<u32>()
        % 2
        != 0
    {
        return Err("mace_odd_electron_count");
    }
    let mut neighbors = vec![Vec::new(); atoms.len()];
    for (i, atom) in atoms.iter().enumerate() {
        for j in 0..i {
            let squared = atom
                .position_angstrom
                .iter()
                .zip(atoms[j].position_angstrom)
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f64>();
            let ratio = squared.sqrt() / (radii[i] + radii[j]);
            if ratio < 0.60 {
                return Err("mace_geometry_overlap");
            }
            if ratio <= 1.30 {
                neighbors[i].push(j);
                neighbors[j].push(i);
            }
        }
    }
    let mut seen = vec![false; atoms.len()];
    let mut pending = vec![0];
    seen[0] = true;
    while let Some(index) = pending.pop() {
        for &next in &neighbors[index] {
            if !seen[next] {
                seen[next] = true;
                pending.push(next);
            }
        }
    }
    if seen.iter().all(|item| *item) {
        Ok(())
    } else {
        Err("mace_geometry_disconnected")
    }
}

const MEDIUM_CHECKPOINT: &str = "4842c52ad210d6e1f84d6cf1ffa70fae25a7e0d755ed55cf223f43913f587db7";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Disagreement {
    pub checkpoint_sha256: String,
    pub max_force_ev_per_angstrom: f64,
    pub rms_force_ev_per_angstrom: f64,
    pub atom: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Diagnostics {
    pub schema_version: u32,
    pub minimum_ratio: f64,
    pub closest_pair: [usize; 2],
    pub connectivity_ratio: f64,
    pub connectivity_pair: [usize; 2],
    pub disagreement: Option<Disagreement>,
}

impl Diagnostics {
    fn valid(&self, atoms: usize) -> bool {
        let valid_pair = |[a, b]: [usize; 2]| a < atoms && b < atoms && a != b;
        self.schema_version == 1
            && (0.599999..=1.300001).contains(&self.minimum_ratio)
            && (self.minimum_ratio..=1.300001).contains(&self.connectivity_ratio)
            && valid_pair(self.closest_pair)
            && valid_pair(self.connectivity_pair)
            && self.disagreement.as_ref().is_none_or(|value| {
                value.checkpoint_sha256 == MEDIUM_CHECKPOINT
                    && value.atom < atoms
                    && value.max_force_ev_per_angstrom.is_finite()
                    && (0.0..=value.max_force_ev_per_angstrom + 1e-12)
                        .contains(&value.rms_force_ev_per_angstrom)
            })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub energy_ev: f64,
    pub checkpoint_sha256: String,
    pub torch_version: String,
    pub ase_version: String,
    pub forces_ev_per_angstrom: Vec<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Diagnostics>,
}

impl Report {
    pub fn valid(&self, document: &Document, hartree: f64) -> bool {
        validate_scope(document).is_ok()
            && self
                .diagnostics
                .as_ref()
                .is_none_or(|value| value.valid(document.atoms.len()))
            && self.energy_ev.is_finite()
            && (self.energy_ev - hartree * 27.211386245988).abs() <= 1e-8
            && self.checkpoint_sha256 == CHECKPOINT
            && [&self.torch_version, &self.ase_version]
                .iter()
                .all(|version| !version.is_empty() && version.len() <= 64)
            && self.forces_ev_per_angstrom.len() == document.atoms.len()
            && self
                .forces_ev_per_angstrom
                .iter()
                .flatten()
                .all(|value| value.is_finite())
    }
}
