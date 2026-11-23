use crate::{catalog::Catalog, jobs::JobRequest, settings::CalculationSettings};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub fn settings() -> CalculationSettings {
    CalculationSettings {
        method: "RKS".into(),
        basis: "def2-svp".into(),
        xc: Some("PBE0".into()),
        grid_level: Some(3),
        ..Default::default()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WarningV1 {
    pub minimum_ratio: f64,
    pub connectivity_ratio: f64,
}

impl WarningV1 {
    fn triggered(&self, max_disagreement: f64) -> bool {
        self.minimum_ratio < 0.80 || self.connectivity_ratio > 1.20 || max_disagreement > 0.50
    }

    fn valid(&self, max_disagreement: f64) -> bool {
        (0.599999..=1.300001).contains(&self.minimum_ratio)
            && (self.minimum_ratio..=1.300001).contains(&self.connectivity_ratio)
            && self.triggered(max_disagreement)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub source_job: String,
    pub max_disagreement: f64,
    pub rank: usize,
    pub budget: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning_v1: Option<WarningV1>,
}

impl Selection {
    pub fn valid(&self) -> bool {
        uuid::Uuid::parse_str(&self.source_job).is_ok()
            && self.max_disagreement.is_finite()
            && self.max_disagreement >= 0.0
            && (1..=20).contains(&self.budget)
            && (1..=self.budget).contains(&self.rank)
            && self
                .warning_v1
                .as_ref()
                .is_none_or(|warning| warning.valid(self.max_disagreement))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub selection: Selection,
    pub forces_ev_per_angstrom: Vec<[f64; 3]>,
}

pub fn from_history(
    store: &Catalog,
    ids: Vec<String>,
    budget: usize,
    revision: u64,
    warnings_only: bool,
) -> Result<Vec<JobRequest>, String> {
    if !(1..=100).contains(&ids.len()) || !(1..=20.min(ids.len())).contains(&budget) {
        return Err("mace_reference_selection".into());
    }
    let ids: HashSet<_> = ids.into_iter().collect();
    let mut candidates = Vec::new();
    let mut eligible = false;
    for id in ids {
        let project = store.restore(&id)?;
        let document = project.document.ok_or("mace_reference_selection")?;
        let diagnostics = project
            .calculation
            .and_then(|job| job.mace)
            .and_then(|report| report.diagnostics);
        if let Some(diagnostics) =
            diagnostics.filter(|_| document.atoms.iter().all(|atom| atom.atomic_number <= 10))
        {
            if let Some(disagreement) = diagnostics.disagreement {
                eligible = true;
                let warning = WarningV1 {
                    minimum_ratio: diagnostics.minimum_ratio,
                    connectivity_ratio: diagnostics.connectivity_ratio,
                };
                let score = disagreement.max_force_ev_per_angstrom;
                if !warnings_only || warning.triggered(score) {
                    candidates.push((id, score, document, warnings_only.then_some(warning)));
                }
            }
        }
    }
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut seen = HashSet::new();
    let mut requests = Vec::new();
    for (id, score, document, warning_v1) in candidates {
        let geometry = document
            .atoms
            .iter()
            .map(|atom| {
                (
                    atom.atomic_number,
                    atom.position_angstrom
                        .map(|v| if v == 0.0 { 0 } else { v.to_bits() }),
                )
            })
            .collect::<Vec<_>>();
        if !seen.insert(geometry) {
            continue;
        }
        let mut request = JobRequest::with_settings(document, revision, settings())?;
        request.mace_reference = Some(Selection {
            source_job: id,
            max_disagreement: score,
            rank: requests.len() + 1,
            budget,
            warning_v1,
        });
        request.validate()?;
        requests.push(request);
        if requests.len() == budget {
            break;
        }
    }
    if requests.is_empty() && !eligible {
        return Err("mace_reference_selection".into());
    }
    Ok(requests)
}
