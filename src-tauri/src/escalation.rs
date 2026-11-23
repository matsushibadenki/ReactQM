use crate::{
    catalog::Catalog, document::Document, jobs::JobRequest, settings::CalculationSettings,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Mp2,
    Ccsdt,
    BelowThreshold,
    OutOfScope,
    NoBudget,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub dft_job: String,
    pub mace_job: String,
    pub max_force_difference: f64,
    pub dft_energy_hartree: f64,
    pub budget: u8,
    pub orbitals: usize,
    pub decision: Decision,
}

fn orbitals(document: &Document) -> usize {
    document
        .atoms
        .iter()
        .map(|atom| match atom.atomic_number {
            1..=2 => 5,
            3..=4 => 9,
            5..=10 => 14,
            _ => 0,
        })
        .sum()
}

impl Input {
    pub fn decide(&self, document: &Document) -> Decision {
        let electrons = document
            .atoms
            .iter()
            .map(|atom| usize::from(atom.atomic_number))
            .sum::<usize>();
        if document.charge != 0
            || document.spin != 0
            || !(2..=10).contains(&document.atoms.len())
            || document.atoms.iter().any(|atom| atom.atomic_number > 10)
            || self.orbitals > 64
            || electrons < 2
            || electrons >= 2 * self.orbitals
        {
            Decision::OutOfScope
        } else if self.max_force_difference <= 0.50 {
            Decision::BelowThreshold
        } else if self.budget == 0 {
            Decision::NoBudget
        } else if self.max_force_difference > 2.0 && self.orbitals <= 32 && self.budget >= 3 {
            Decision::Ccsdt
        } else {
            Decision::Mp2
        }
    }

    pub fn valid(&self, document: &Document) -> bool {
        uuid::Uuid::parse_str(&self.dft_job).is_ok()
            && uuid::Uuid::parse_str(&self.mace_job).is_ok()
            && self.max_force_difference.is_finite()
            && self.max_force_difference >= 0.0
            && self.dft_energy_hartree.is_finite()
            && self.budget <= 3
            && self.orbitals == orbitals(document)
            && self.decision == self.decide(document)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub policy_v1: Input,
    pub rhf_hartree: Option<f64>,
    pub triples_hartree: Option<f64>,
}

impl Report {
    pub fn valid(&self, document: &Document, energy: f64) -> bool {
        self.policy_v1.valid(document)
            && match self.policy_v1.decision {
                Decision::Mp2 | Decision::Ccsdt => {
                    self.rhf_hartree.is_some_and(f64::is_finite)
                        && self.triples_hartree.is_some_and(|x| {
                            x.is_finite()
                                && (self.policy_v1.decision == Decision::Ccsdt || x == 0.0)
                        })
                }
                _ => {
                    self.rhf_hartree.is_none()
                        && self.triples_hartree.is_none()
                        && energy == self.policy_v1.dft_energy_hartree
                }
            }
    }
}

pub fn settings() -> CalculationSettings {
    CalculationSettings {
        basis: "def2-svp".into(),
        ..Default::default()
    }
}

pub fn from_history(
    store: &Catalog,
    id: &str,
    budget: u8,
    revision: u64,
) -> Result<JobRequest, String> {
    let reference = store.restore(id)?;
    let result = reference.calculation.ok_or("escalation_source")?;
    let report = result.mace_reference.ok_or("escalation_source")?;
    let source = store.restore(&report.selection.source_job)?;
    let document = reference.document.ok_or("escalation_source")?;
    if source.document.as_ref() != Some(&document) {
        return Err("escalation_source".into());
    }
    let mace = source
        .calculation
        .and_then(|job| job.mace)
        .ok_or("escalation_source")?;
    let gap = report
        .forces_ev_per_angstrom
        .iter()
        .zip(&mace.forces_ev_per_angstrom)
        .map(|(a, b)| {
            a.iter()
                .zip(b)
                .map(|(x, y)| (x - y).powi(2))
                .sum::<f64>()
                .sqrt()
        })
        .fold(0.0, f64::max);
    let mut input = Input {
        dft_job: id.into(),
        mace_job: report.selection.source_job,
        max_force_difference: gap,
        dft_energy_hartree: result.energy_hartree,
        budget,
        orbitals: orbitals(&document),
        decision: Decision::BelowThreshold,
    };
    input.decision = input.decide(&document);
    let mut request = JobRequest::with_settings(document, revision, settings())?;
    request.escalation = Some(input);
    request.validate()?;
    Ok(request)
}
