use crate::{catalog::Catalog, document::Document, jobs::JobRequest};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Dft,
    OutOfScope,
    BelowThreshold,
    NoBudget,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub source_job: String,
    pub source_engine: String,
    pub max_force_ev_per_angstrom: f64,
    pub source_energy_hartree: f64,
    pub budget: u8,
    pub decision: Decision,
}

impl Policy {
    fn decide(&self, document: &Document) -> Decision {
        if document.charge != 0
            || document.spin != 0
            || !(2..=16).contains(&document.atoms.len())
            || document.atoms.iter().any(|atom| atom.atomic_number > 10)
        {
            Decision::OutOfScope
        } else if self.max_force_ev_per_angstrom <= 0.30 {
            Decision::BelowThreshold
        } else if self.budget == 0 {
            Decision::NoBudget
        } else {
            Decision::Dft
        }
    }

    pub fn valid(&self, document: &Document) -> bool {
        uuid::Uuid::parse_str(&self.source_job).is_ok()
            && !self.source_engine.is_empty()
            && self.source_engine.len() <= 64
            && self.max_force_ev_per_angstrom.is_finite()
            && self.max_force_ev_per_angstrom >= 0.0
            && self.source_energy_hartree.is_finite()
            && self.budget <= 1
            && self.decision == self.decide(document)
    }

    pub fn settings(&self) -> crate::settings::CalculationSettings {
        if self.decision == Decision::Dft {
            crate::mace_reference::settings()
        } else {
            crate::settings::CalculationSettings {
                method: crate::settings::SCREENING.into(),
                basis: "gfn2".into(),
                max_cycle: 250,
                conv_tol: 0.0,
                ..Default::default()
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub policy_v1: Policy,
    pub dft_max_force_ev_per_angstrom: Option<f64>,
}

impl Report {
    pub fn valid(&self, document: &Document, energy: f64) -> bool {
        self.policy_v1.valid(document)
            && match self.dft_max_force_ev_per_angstrom {
                Some(force) => {
                    self.policy_v1.decision == Decision::Dft && force.is_finite() && force >= 0.0
                }
                None => {
                    self.policy_v1.decision != Decision::Dft
                        && energy == self.policy_v1.source_energy_hartree
                }
            }
    }
}

pub fn from_history(
    store: &Catalog,
    id: &str,
    budget: u8,
    revision: u64,
) -> Result<JobRequest, String> {
    let project = store.restore(id)?;
    let source = project
        .calculation
        .filter(|job| job.method == crate::settings::SCREENING && job.xtb_followup.is_none())
        .ok_or("xtb_followup_source")?;
    let document = project.document.ok_or("xtb_followup_source")?;
    let force = source.xtb_max_force.ok_or("xtb_followup_force_missing")?;
    let mut policy = Policy {
        source_job: id.into(),
        source_engine: source.engine_version,
        max_force_ev_per_angstrom: force,
        source_energy_hartree: source.energy_hartree,
        budget,
        decision: Decision::BelowThreshold,
    };
    policy.decision = policy.decide(&document);
    let mut request = JobRequest::with_settings(document, revision, policy.settings())?;
    request.xtb_followup = Some(policy);
    request.validate()?;
    Ok(request)
}
