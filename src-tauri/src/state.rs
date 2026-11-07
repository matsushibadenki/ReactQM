use crate::{
    layout::PanelLayout,
    project::{Calculation, Project},
    renderer::Renderer,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
pub enum Locale {
    #[serde(rename = "en")]
    En,
    #[default]
    #[serde(rename = "ja")]
    Ja,
    #[serde(rename = "zh-CN")]
    ZhCn,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSnapshot {
    pub id: String,
    pub status: String,
    pub cycle: u32,
    pub error: Option<String>,
}

pub struct ActiveJob {
    pub id: String,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct Workspace {
    pub project: Project,
    pub renderer: Renderer,
    pub layout: PanelLayout,
    pub locale: Locale,
    pub revision: u64,
    pub document_revision: u64,
    pub undo: Vec<Project>,
    pub redo: Vec<Project>,
    pub saved_project: Option<Project>,
    pub saved_available: bool,
    pub exit_after_job: bool,
    pub job: Option<JobSnapshot>,
    pub active_job: Option<ActiveJob>,
    pub result_targets: HashMap<String, u64>,
}

#[derive(Default)]
pub struct AppState(pub Mutex<Workspace>);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSnapshot {
    pub revision: u64,
    pub document_revision: u64,
    pub locale: Locale,
    pub atom_count: usize,
    pub name: String,
    pub formula: String,
    pub charge: i32,
    pub spin: u32,
    pub renderer_ready: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub dirty: bool,
    pub has_saved: bool,
    pub calculation: Option<Calculation>,
    pub job: Option<JobSnapshot>,
    pub busy: bool,
}

impl Workspace {
    pub fn replace_project(&mut self, project: Project) -> Result<(), String> {
        if project == self.project {
            if self.active_job.is_none() {
                self.job = None;
            }
            self.revision += 1;
            return Ok(());
        }
        self.undo.push(self.project.clone());
        if self.undo.len() > 32 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.project = project;
        self.document_revision += 1;
        self.revision += 1;
        if self.active_job.is_none() {
            self.job = None;
        }
        Ok(())
    }
    pub fn history(&mut self, redo: bool) -> Result<(), String> {
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        let project = from.pop().ok_or("history_empty")?;
        to.push(std::mem::replace(&mut self.project, project));
        self.document_revision += 1;
        self.revision += 1;
        if self.active_job.is_none() {
            self.job = None;
        }
        Ok(())
    }
    /// Targets are session-local; a job recovered from a previous app session writes history only.
    pub fn complete_target(&mut self, id: &str, result: Option<Calculation>) {
        let target = self.result_targets.remove(id);
        if target != Some(self.document_revision) {
            return;
        }
        if let (Some(calculation), Some(document)) = (result, self.project.document.as_ref()) {
            if calculation.job_id == id && calculation.validate(document).is_ok() {
                self.project.calculation = Some(calculation);
            }
        }
    }
    pub fn cancel(&mut self) -> Result<(), String> {
        let active = self.active_job.as_ref().ok_or("no_job")?;
        active.cancel.store(true, Ordering::SeqCst);
        if let Some(job) = self.job.as_mut() {
            job.status = "cancelling".into();
        }
        self.revision += 1;
        Ok(())
    }
    pub fn snapshot(&self) -> WorkspaceSnapshot {
        let doc = self.project.document.as_ref();
        WorkspaceSnapshot {
            revision: self.revision,
            document_revision: self.document_revision,
            locale: self.locale,
            atom_count: doc.map_or(0, |doc| doc.atoms.len()),
            name: doc.map_or_else(String::new, |doc| doc.name.clone()),
            formula: doc.map_or_else(String::new, |doc| doc.formula()),
            charge: doc.map_or(0, |doc| doc.charge),
            spin: doc.map_or(0, |doc| doc.spin),
            renderer_ready: self.renderer.ready(),
            can_undo: !self.undo.is_empty(),
            can_redo: !self.redo.is_empty(),
            dirty: self.project.document.is_some()
                && self.saved_project.as_ref() != Some(&self.project),
            has_saved: self.saved_available,
            calculation: self.project.calculation.clone(),
            job: self.job.clone(),
            busy: self.active_job.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    fn project(distance: f64) -> Project {
        Project {
            document: Some(
                Document::from_xyz(&format!("2\nH2\nH 0 0 0\nH 0 0 {distance}"), 0, 0).unwrap(),
            ),
            calculation: None,
        }
    }
    #[test]
    fn history_restores_documents_and_invalidates_redo_branch() {
        let mut state = Workspace::default();
        state.replace_project(project(0.74)).unwrap();
        state.replace_project(project(1.0)).unwrap();
        let revision = state.revision;
        state.history(false).unwrap();
        assert_eq!(state.project, project(0.74));
        assert!(state.revision > revision);
        state.history(true).unwrap();
        assert_eq!(state.project, project(1.0));
        state.history(false).unwrap();
        state.replace_project(project(1.2)).unwrap();
        assert!(!state.snapshot().can_redo);
    }
    #[test]
    fn edits_are_independent_of_running_jobs_and_cancel_sets_signal() {
        let mut state = Workspace {
            active_job: Some(ActiveJob {
                id: "job".into(),
                cancel: Arc::new(AtomicBool::new(false)),
            }),
            ..Default::default()
        };
        assert!(state.replace_project(project(0.74)).is_ok());
        state.cancel().unwrap();
        assert!(state.active_job.unwrap().cancel.load(Ordering::SeqCst));
    }
    #[test]
    fn queued_results_cannot_overwrite_edits_or_a_new_session() {
        let mut state = Workspace::default();
        state.replace_project(project(0.74)).unwrap();
        let document = state.project.document.clone().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let calculation = Calculation {
            job_id: id.clone(),
            input_hash: document.fingerprint(),
            method: "RHF".into(),
            basis: "sto-3g".into(),
            energy_hartree: -1.1167593074,
            converged: true,
            engine_version: "fixture".into(),
            elapsed_seconds: 0.1,
        };
        state.complete_target(&id, Some(calculation.clone()));
        assert!(state.project.calculation.is_none());
        state
            .result_targets
            .insert(id.clone(), state.document_revision);
        state.replace_project(project(1.0)).unwrap();
        state.complete_target(&id, Some(calculation.clone()));
        assert!(state.project.calculation.is_none());
        state.replace_project(project(0.74)).unwrap();
        state
            .result_targets
            .insert(id.clone(), state.document_revision);
        state.complete_target(&id, Some(calculation));
        assert!(state.project.calculation.is_some());
        assert!(state.result_targets.is_empty());
    }
}
