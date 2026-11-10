//! RDKit import transaction: bounded Worker, then revision-checked shared Document commit.
use crate::{
    document::Document,
    jobs::WorkerConfig,
    project::Project,
    state::{AppState, WorkspaceSnapshot},
};
use serde::Deserialize;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};
use tauri::Manager;
#[derive(Default)]
pub struct StructureService {
    gate: Mutex<()>,
    pub stopping: AtomicBool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    document: Option<Document>,
    error: Option<String>,
}
pub fn generate(
    format: &str,
    text: &str,
    seed: u32,
    config: &WorkerConfig,
    stopping: &AtomicBool,
    timeout: Duration,
) -> Result<Document, String> {
    if !["smiles", "sdf"].contains(&format)
        || text.trim().is_empty()
        || text.len() > 16384
        || seed > i32::MAX as u32
        || text
            .chars()
            .any(|c| c.is_control() && !['\r', '\n', '\t'].contains(&c))
    {
        return Err("structure_input".into());
    }
    let request = serde_json::to_vec(
        &serde_json::json!({"schema_version":1,"format":format,"text":text,"seed":seed}),
    )
    .map_err(|_| "structure_protocol")?;
    let bytes = crate::valence::worker_response(config, stopping, timeout, "structure", request)
        .map_err(|e| e.replace("valence_", "structure_"))?;
    let response: Response = serde_json::from_slice(&bytes).map_err(|_| "structure_protocol")?;
    match (response.document, response.error) {
        (Some(document), None) => {
            document.validate()?;
            let source = document.source.as_ref().ok_or("structure_protocol")?;
            if source.format != format
                || source.text != text
                || (source.method == "etkdgv3" && source.seed != Some(seed))
                || document.spin != 0
            {
                return Err("structure_protocol".into());
            }
            Ok(document)
        }
        (None, Some(error))
            if [
                "structure_input",
                "structure_single",
                "structure_unsupported",
                "structure_embed",
                "atom_limit",
                "coordinate",
                "atom_overlap",
            ]
            .contains(&error.as_str()) =>
        {
            Err(error)
        }
        _ => Err("structure_protocol".into()),
    }
}
#[tauri::command]
pub async fn import_structure(
    app: tauri::AppHandle,
    expected_revision: u64,
    format: String,
    text: String,
    seed: u32,
) -> Result<WorkspaceSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let service = app.state::<StructureService>();
        let _gate = service.gate.lock().map_err(|e| e.to_string())?;
        {
            let state = app.state::<AppState>();
            let workspace = state.0.lock().map_err(|e| e.to_string())?;
            crate::check_revision(&workspace, expected_revision)?;
        }
        let document = generate(
            &format,
            &text,
            seed,
            &WorkerConfig::development(),
            &service.stopping,
            Duration::from_secs(20),
        )?;
        if service.stopping.load(Ordering::SeqCst) {
            return Err("queue_stopping".into());
        }
        let snapshot = {
            let state = app.state::<AppState>();
            let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
            commit(&mut workspace, expected_revision, document)?
        };
        crate::publish(&app, snapshot)
    })
    .await
    .map_err(|e| e.to_string())?
}
fn commit(
    workspace: &mut crate::state::Workspace,
    expected: u64,
    document: Document,
) -> Result<WorkspaceSnapshot, String> {
    crate::check_revision(workspace, expected)?;
    workspace.replace_project(Project {
        settings: workspace.project.settings.clone(),
        document: Some(document),
        calculation: None,
    })?;
    Ok(workspace.snapshot())
}
pub fn shutdown(app: &tauri::AppHandle) {
    let service = app.state::<StructureService>();
    service.stopping.store(true, Ordering::SeqCst);
    drop(service.gate.lock());
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_smiles_repeat_save_undo_and_quantum_request() {
        let config = WorkerConfig::development();
        let stop = AtomicBool::new(false);
        let a = generate("smiles", "O", 42, &config, &stop, Duration::from_secs(20)).unwrap();
        let b = generate("smiles", "O", 42, &config, &stop, Duration::from_secs(20)).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.formula(), "H2O");
        crate::jobs::JobRequest::new(a.clone(), 1)
            .unwrap()
            .validate()
            .unwrap();
        let mut workspace = crate::state::Workspace::default();
        let p = Project {
            settings: Default::default(),
            document: Some(a.clone()),
            calculation: None,
        };
        workspace.replace_project(p.clone()).unwrap();
        workspace
            .replace_project(Project {
                settings: Default::default(),
                document: Some(Document::from_xyz("2\nH2\nH 0 0 0\nH 0 0 .74", 0, 0).unwrap()),
                calculation: None,
            })
            .unwrap();
        workspace.history(false).unwrap();
        assert_eq!(workspace.project, p);
        let revision = workspace.document_revision;
        assert!(
            matches!(commit(&mut workspace, revision - 1, b.clone()), Err(e) if e == "stale_document")
        );
        assert_eq!(workspace.project, p);
        let dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let path = dir.join("project.json");
        p.save(&path).unwrap();
        assert_eq!(Project::load(&path).unwrap(), p);
        let mut corrupt = a;
        corrupt.source.as_mut().unwrap().text = "N".into();
        assert!(corrupt.validate().is_err());
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(
            generate("smiles", "C.C", 42, &config, &stop, Duration::from_secs(20)).unwrap_err(),
            "structure_single"
        );
        assert_eq!(
            generate(
                "smiles",
                "C",
                42,
                &config,
                &AtomicBool::new(true),
                Duration::from_secs(20)
            )
            .unwrap_err(),
            "queue_stopping"
        );
        assert_eq!(
            generate("smiles", "C", 42, &config, &stop, Duration::ZERO).unwrap_err(),
            "structure_timeout"
        );
    }
}
