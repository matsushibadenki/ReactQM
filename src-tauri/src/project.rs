use crate::document::Document;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Calculation {
    pub job_id: String,
    pub input_hash: String,
    pub method: String,
    pub basis: String,
    pub energy_hartree: f64,
    pub converged: bool,
    pub engine_version: String,
    pub elapsed_seconds: f64,
}

impl Calculation {
    pub fn validate(&self, document: &Document) -> Result<(), String> {
        if uuid::Uuid::parse_str(&self.job_id).is_err()
            || self.input_hash != document.fingerprint()
            || !self.converged
            || !self.energy_hartree.is_finite()
            || !self.elapsed_seconds.is_finite()
            || self.elapsed_seconds < 0.0
            || self.method != "RHF"
            || self.basis != "sto-3g"
            || self.engine_version.len() > 64
            || document.spin != 0
        {
            return Err("result_invalid".into());
        }
        Ok(())
    }
}

#[derive(Clone, Default, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub document: Option<Document>,
    pub calculation: Option<Calculation>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedProject {
    schema_version: u32,
    project: Project,
}

impl Project {
    pub fn validate(&self) -> Result<(), String> {
        let doc = self.document.as_ref().ok_or("no_document")?;
        doc.validate()?;
        if let Some(result) = &self.calculation {
            result.validate(doc)?;
        }
        Ok(())
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        atomic_json(
            path,
            &SavedProject {
                schema_version: 1,
                project: self.clone(),
            },
        )
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|e| format!("storage: {e}"))?
            .take(131_073)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("storage: {e}"))?;
        if bytes.len() > 131_072 {
            return Err("project_size".into());
        }
        let saved: SavedProject =
            serde_json::from_slice(&bytes).map_err(|e| format!("project_invalid: {e}"))?;
        if saved.schema_version != 1 {
            return Err("document_version".into());
        }
        saved.project.validate()?;
        Ok(saved.project)
    }
}

pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let parent = path.parent().ok_or("storage")?;
    fs::create_dir_all(parent).map_err(|e| format!("storage: {e}"))?;
    let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(value)?)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok::<_, std::io::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result.map_err(|e| format!("storage: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn results_cannot_attach_to_other_inputs_or_nonconverged_states() {
        let doc = Document::from_xyz("2\nH2\nH 0 0 0\nH 0 0 .74", 0, 0).unwrap();
        let mut result = Calculation {
            job_id: uuid::Uuid::new_v4().to_string(),
            input_hash: doc.fingerprint(),
            method: "RHF".into(),
            basis: "sto-3g".into(),
            energy_hartree: -1.1167593074,
            converged: true,
            engine_version: "2.15.0".into(),
            elapsed_seconds: 1.0,
        };
        assert!(result.validate(&doc).is_ok());
        let mut changed = doc.clone();
        changed.atoms[1].position_angstrom[2] = 1.0;
        assert!(result.validate(&changed).is_err());
        result.converged = false;
        assert!(result.validate(&doc).is_err());
        result.converged = true;
        result.energy_hartree = f64::NAN;
        assert!(result.validate(&doc).is_err());
    }
    #[test]
    fn save_reload_and_invalid_input_preserve_existing_save() {
        let dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let path = dir.join("workspace.json");
        let project = Project {
            document: Some(Document::from_xyz("2\nH2\nH 0 0 0\nH 0 0 .74", 0, 0).unwrap()),
            calculation: None,
        };
        project.save(&path).unwrap();
        assert_eq!(Project::load(&path).unwrap(), project);
        assert!(Project::default().save(&path).is_err());
        assert_eq!(Project::load(&path).unwrap(), project);
        fs::write(&path, b"{broken").unwrap();
        assert!(Project::load(&path).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
