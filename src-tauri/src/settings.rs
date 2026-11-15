//! Versioned, bounded settings for the validated calculation adapters.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CalculationSettings {
    pub method: String,
    pub basis: String,
    pub xc: Option<String>,
    pub max_cycle: u32,
    pub conv_tol: f64,
    pub grid_level: Option<u32>,
}
impl Default for CalculationSettings {
    fn default() -> Self {
        Self {
            method: "RHF".into(),
            basis: "sto-3g".into(),
            xc: None,
            max_cycle: 50,
            conv_tol: 1e-9,
            grid_level: None,
        }
    }
}
pub const SCREENING: &str = "GFN2-xTB";

impl CalculationSettings {
    pub fn validate_screening_input(
        &self,
        document: &crate::document::Document,
    ) -> Result<(), String> {
        if document.spin != 0 {
            return Err("screening_spin".into());
        }
        if document.atoms.iter().any(|atom| atom.atomic_number > 86) {
            return Err("screening_elements".into());
        }
        Ok(())
    }

    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
    pub fn is_unrestricted(&self) -> bool {
        matches!(self.method.as_str(), "UHF" | "UKS")
    }
    pub fn for_spin(mut self, spin: u32) -> Self {
        if spin > 0 {
            self.method = match self.method.as_str() {
                SCREENING => {
                    return Self {
                        method: "UHF".into(),
                        ..Self::default()
                    }
                }
                "RHF" => "UHF".into(),
                "RKS" => "UKS".into(),
                _ => self.method,
            };
        }
        self
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.method == SCREENING {
            return if self.basis == "gfn2"
                && self.max_cycle == 250
                && self.conv_tol == 0.0
                && self.xc.is_none()
                && self.grid_level.is_none()
            {
                Ok(())
            } else {
                Err("calculation_settings".into())
            };
        }
        if !["sto-3g", "6-31g", "def2-svp"].contains(&self.basis.as_str())
            || !(1..=50).contains(&self.max_cycle)
            || !(1e-12..=1e-6).contains(&self.conv_tol)
        {
            return Err("calculation_settings".into());
        }
        match self.method.as_str() {
            "RHF" | "UHF" if self.xc.is_none() && self.grid_level.is_none() => Ok(()),
            "RKS" | "UKS"
                if matches!(self.xc.as_deref(), Some("PBE" | "PBE0" | "B3LYP"))
                    && matches!(self.grid_level, Some(3 | 5)) =>
            {
                Ok(())
            }
            _ => Err("calculation_settings".into()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        jobs::JobRequest,
        project::{Calculation, Project},
    };
    #[test]
    fn invalid_settings_results_and_legacy_serialization_are_checked() {
        let doc = Document::from_xyz("2\nH2\nH 0 0 0\nH 0 0 .74", 0, 0).unwrap();
        let old = JobRequest::new(doc.clone(), 1).unwrap();
        let json = serde_json::to_string(&old).unwrap();
        assert!(!json.contains("settings"));
        let loaded: JobRequest = serde_json::from_str(&json).unwrap();
        loaded.validate().unwrap();
        let settings = CalculationSettings {
            method: "RKS".into(),
            xc: Some("PBE".into()),
            grid_level: Some(3),
            basis: "6-31g".into(),
            ..Default::default()
        };
        let request = JobRequest::with_settings(doc.clone(), 2, settings.clone()).unwrap();
        let retry = request.retry(3).unwrap();
        assert_eq!(retry.settings, request.settings);
        let mut result = Calculation {
            optimization: None,
            spin_squared: None,
            fields: Vec::new(),
            job_id: request.job_id.clone(),
            input_hash: doc.fingerprint(),
            method: "RKS".into(),
            basis: "6-31g".into(),
            settings: Some(settings.clone()),
            energy_hartree: -1.0,
            converged: true,
            engine_version: "fixture".into(),
            elapsed_seconds: 0.0,
        };
        request.validate_result(&result).unwrap();
        result.settings.as_mut().unwrap().xc = Some("PBE0".into());
        assert!(request.validate_result(&result).is_err());
        result.settings = None;
        assert!(request.validate_result(&result).is_err());
        let path =
            std::env::temp_dir().join(format!("reactqm-settings-{}.json", uuid::Uuid::new_v4()));
        let project = Project {
            document: Some(doc.clone()),
            calculation: None,
            settings,
        };
        project.save(&path).unwrap();
        assert_eq!(Project::load(&path).unwrap(), project);
        std::fs::remove_file(&path).unwrap();
        let legacy =
            serde_json::json!({"schema_version":1,"project":{"document":doc,"calculation":null}});
        std::fs::write(&path, legacy.to_string()).unwrap();
        assert_eq!(
            Project::load(&path).unwrap().settings,
            CalculationSettings::default()
        );
        std::fs::remove_file(path).unwrap();
        let mut bad = CalculationSettings {
            xc: Some("PBE".into()),
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        bad = CalculationSettings {
            conv_tol: f64::NAN,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
    }
}
