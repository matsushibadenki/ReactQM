//! One-time import and reindexing of legacy job directories.
use super::{check_id, digest, read_json, storage, Catalog};
use crate::{jobs::JobRequest, project::Calculation, state::JobSnapshot};
use std::{fs, time::UNIX_EPOCH};

impl Catalog {
    pub(super) fn import_legacy(&self) -> Result<(), String> {
        let done: bool = self
            .connection
            .lock()
            .map_err(storage)?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM catalog_meta WHERE key='legacy_imported')",
                [],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if done {
            return Ok(());
        }
        let mut entries = fs::read_dir(self.root.join("jobs"))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        entries.sort_by_key(|entry| {
            (
                entry.metadata().and_then(|m| m.modified()).ok(),
                entry.file_name(),
            )
        });
        for entry in entries {
            let id = entry.file_name().to_string_lossy().into_owned();
            if check_id(&id).is_err() || !entry.file_type().map_err(storage)?.is_dir() {
                continue;
            }
            let existing = self.row(&id).ok();
            if existing
                .as_ref()
                .is_some_and(|(_, status, receipt)| status != "interrupted" || receipt.is_some())
            {
                continue;
            }
            let Ok(request) = read_json::<JobRequest>(&entry.path().join("input.json")) else {
                continue;
            };
            if request.job_id != id || request.validate().is_err() {
                continue;
            }
            let created = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis().min(i64::MAX as u128) as i64);
            let has_receipt = fs::symlink_metadata(entry.path().join("completion.json")).is_ok();
            if existing.is_none() {
                self.insert(&request, "interrupted", None, !has_receipt, created)?;
            }
            let status = read_json::<JobSnapshot>(&entry.path().join("status.json"))
                .ok()
                .filter(|s| s.id == id);
            if let Some(status) = status.as_ref().filter(|s| {
                ["failed", "cancelled", "timed_out", "cancelling"].contains(&s.status.as_str())
            }) {
                let terminal = if status.status == "cancelling" {
                    "cancelled"
                } else {
                    &status.status
                };
                self.update(&id, terminal, status.error.as_deref(), None, None)?;
            } else if has_receipt {
                // Reindexing must preserve an existing baseline, never rehash changed files as new truth.
                match self.verify(&request, None) {
                    Ok(calculation) => {
                        let receipt = digest(&entry.path().join("completion.json"))?.1;
                        self.update(
                            &id,
                            "succeeded",
                            None,
                            Some(calculation.energy_hartree),
                            Some(&receipt),
                        )?;
                    }
                    Err(_) => self.update(&id, "corrupt", Some("artifact_invalid"), None, None)?,
                }
            } else if status.as_ref().is_some_and(|s| s.status == "succeeded") {
                if let Ok(calculation) = read_json::<Calculation>(&entry.path().join("result.json"))
                {
                    self.update(&id, "running", None, None, None)?;
                    if self.complete(&request, &calculation).is_ok() {
                        continue;
                    }
                }
                self.update(&id, "corrupt", Some("artifact_invalid"), None, None)?;
            } else {
                self.update(&id, "interrupted", Some("job_interrupted"), None, None)?;
            }
        }
        self.connection
            .lock()
            .map_err(storage)?
            .execute(
                "INSERT INTO catalog_meta (key,value) VALUES ('legacy_imported','1')",
                [],
            )
            .map_err(storage)?;
        Ok(())
    }
}
