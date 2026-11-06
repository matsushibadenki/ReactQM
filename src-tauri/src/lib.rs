pub mod document;
pub mod jobs;
pub mod project;
pub mod renderer;
pub mod state;

use document::Document;
use jobs::{JobRequest, WorkerConfig};
use project::Project;
use state::{ActiveJob, AppState, JobSnapshot, Locale, WorkspaceSnapshot};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
use tauri::{Emitter, Manager, State};

fn storage_directory(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    // Explicit override for isolated development / integration tests.
    if let Some(path) = std::env::var_os("REACTQM_DATA_DIR") {
        return Ok(PathBuf::from(path));
    }
    app.path()
        .app_data_dir()
        .map_err(|e| format!("storage: {e}"))
}
fn publish(
    app: &tauri::AppHandle,
    snapshot: WorkspaceSnapshot,
) -> Result<WorkspaceSnapshot, String> {
    app.emit("workspace-changed", &snapshot)
        .map_err(|e| e.to_string())?;
    Ok(snapshot)
}
fn check_revision(workspace: &state::Workspace, expected: u64) -> Result<(), String> {
    if workspace.document_revision != expected {
        Err("stale_document".into())
    } else {
        Ok(())
    }
}

#[tauri::command]
fn workspace_snapshot(state: State<'_, AppState>) -> Result<WorkspaceSnapshot, String> {
    Ok(state.0.lock().map_err(|e| e.to_string())?.snapshot())
}
#[tauri::command]
fn set_locale(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    locale: Locale,
) -> Result<WorkspaceSnapshot, String> {
    let snapshot = {
        let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
        workspace.locale = locale;
        workspace.revision += 1;
        workspace.snapshot()
    };
    publish(&app, snapshot)
}
#[tauri::command]
fn import_xyz(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    text: String,
    charge: i32,
    spin: u32,
    expected_revision: u64,
) -> Result<WorkspaceSnapshot, String> {
    let document = Document::from_xyz(&text, charge, spin)?;
    let snapshot = {
        let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
        check_revision(&workspace, expected_revision)?;
        workspace.replace_project(Project {
            document: Some(document),
            calculation: None,
        })?;
        workspace.snapshot()
    };
    publish(&app, snapshot)
}
#[tauri::command]
fn history_step(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    redo: bool,
    expected_revision: u64,
) -> Result<WorkspaceSnapshot, String> {
    let snapshot = {
        let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
        check_revision(&workspace, expected_revision)?;
        workspace.history(redo)?;
        workspace.snapshot()
    };
    publish(&app, snapshot)
}
#[tauri::command]
fn save_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    expected_revision: u64,
) -> Result<WorkspaceSnapshot, String> {
    let path = storage_directory(&app)?.join("workspace.json");
    let snapshot = {
        let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
        workspace.ensure_idle()?;
        check_revision(&workspace, expected_revision)?;
        workspace.project.save(&path)?;
        workspace.saved_project = Some(workspace.project.clone());
        workspace.saved_available = true;
        workspace.revision += 1;
        workspace.snapshot()
    };
    publish(&app, snapshot)
}
#[tauri::command]
fn load_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    expected_revision: u64,
) -> Result<WorkspaceSnapshot, String> {
    let project = Project::load(&storage_directory(&app)?.join("workspace.json"))?;
    let snapshot = {
        let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
        check_revision(&workspace, expected_revision)?;
        workspace.replace_project(project.clone())?;
        workspace.saved_project = Some(project);
        workspace.saved_available = true;
        workspace.revision += 1;
        workspace.snapshot()
    };
    publish(&app, snapshot)
}
#[tauri::command]
fn cancel_job(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<WorkspaceSnapshot, String> {
    let snapshot = {
        let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
        workspace.cancel()?;
        workspace.snapshot()
    };
    publish(&app, snapshot)
}
#[tauri::command]
fn start_calculation(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    expected_revision: u64,
) -> Result<WorkspaceSnapshot, String> {
    let directory = storage_directory(&app)?.join("jobs");
    let config = WorkerConfig::development();
    if !config.python.is_file() {
        return Err("worker_missing".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let (request, snapshot) = {
        let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
        workspace.ensure_idle()?;
        check_revision(&workspace, expected_revision)?;
        let request = JobRequest::new(
            workspace.project.document.clone().ok_or("no_document")?,
            workspace.document_revision,
        )?;
        workspace.active_job = Some(ActiveJob {
            id: request.job_id.clone(),
            cancel: cancel.clone(),
        });
        workspace.job = Some(JobSnapshot {
            id: request.job_id.clone(),
            status: "running".into(),
            cycle: 0,
            error: None,
        });
        workspace.project.calculation = None;
        workspace.revision += 1;
        (request, workspace.snapshot())
    };
    // Emit failures must not strand an active job. Every started job reaches cleanup.
    let _ = app.emit("workspace-changed", &snapshot);
    std::thread::spawn(move || {
        let directory = directory.join(&request.job_id);
        let result = jobs::execute(
            &request,
            &directory,
            &config,
            cancel.clone(),
            Duration::from_secs(120),
            |cycle| {
                let state = app.state::<AppState>();
                if let Ok(mut workspace) = state.0.lock() {
                    if let Some(job) = &mut workspace.job {
                        if job.id == request.job_id {
                            job.cycle = cycle;
                        }
                    }
                    workspace.revision += 1;
                    let _ = app.emit("workspace-changed", workspace.snapshot());
                };
            },
        );
        let state = app.state::<AppState>();
        if let Ok(mut workspace) = state.0.lock() {
            if workspace.active_job.as_ref().map(|j| j.id.as_str()) != Some(request.job_id.as_str())
            {
                return;
            }
            let result = if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                Err("cancelled".into())
            } else {
                result
            };
            let (status, error) = match result {
                Ok(result) if workspace.document_revision == request.document_revision => {
                    workspace.project.calculation = Some(result);
                    ("succeeded", None)
                }
                Ok(_) => ("failed", Some("stale_document".into())),
                Err(error) if error == "cancelled" => ("cancelled", None),
                Err(error) if error == "timed_out" => ("timed_out", Some(error)),
                Err(error) => ("failed", Some(error)),
            };
            if let Some(job) = &mut workspace.job {
                job.status = status.into();
                job.error = error;
            }
            workspace.active_job = None;
            workspace.revision += 1;
            let snapshot = workspace.snapshot();
            // Preserve the terminal status even when the application project is not saved.
            if let Err(error) = project::atomic_json(&directory.join("status.json"), &snapshot.job)
            {
                if let Some(job) = &mut workspace.job {
                    job.error = Some(error);
                }
            }
            let _ = app.emit("workspace-changed", workspace.snapshot());
            let exit = workspace.exit_after_job;
            drop(workspace);
            if exit {
                app.exit(0);
            }
        };
    });
    Ok(snapshot)
}

#[tauri::command]
async fn update_viewport(
    window: tauri::WebviewWindow,
    request: renderer::ViewportRequest,
) -> Result<renderer::ViewportSnapshot, String> {
    if window.label() != "main" {
        return Err("viewport_window".into());
    }
    #[cfg(target_os = "macos")]
    {
        let app = window.app_handle().clone();
        let (sender, mut receiver) = tauri::async_runtime::channel(1);
        window
            .with_webview(move |webview| {
                let result = (|| {
                    let state = app.state::<AppState>();
                    let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
                    let state::Workspace {
                        renderer,
                        project,
                        document_revision,
                        ..
                    } = &mut *workspace;
                    renderer.update(
                        webview.inner().cast(),
                        request,
                        project.document.as_ref(),
                        *document_revision,
                    )
                })();
                let _ = sender.try_send(result);
            })
            .map_err(|e| e.to_string())?;
        receiver.recv().await.ok_or("viewport_closed")?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = request;
        Err("renderer_unsupported".into())
    }
}
#[tauri::command]
fn detach_viewport(window: tauri::WebviewWindow, client: String) -> Result<(), String> {
    let app = window.app_handle().clone();
    window
        .run_on_main_thread(move || {
            if let Ok(mut workspace) = app.state::<AppState>().0.lock() {
                workspace.renderer.detach(&client);
            }
        })
        .map_err(|e| e.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| {
            let exists = storage_directory(app.handle())
                .map(|dir| dir.join("workspace.json").is_file())
                .unwrap_or(false);
            app.state::<AppState>().0.lock().unwrap().saved_available = exists;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            update_viewport,
            detach_viewport,
            workspace_snapshot,
            set_locale,
            import_xyz,
            history_step,
            save_project,
            load_project,
            start_calculation,
            cancel_job
        ])
        .build(tauri::generate_context!())
        .expect("ReactQM failed to start")
        .run(|app, event| {
            if let tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::Destroyed,
                ..
            } = &event
            {
                if label == "main" {
                    if let Ok(mut workspace) = app.state::<AppState>().0.lock() {
                        workspace.renderer.close();
                    }
                }
            }
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if let Ok(mut workspace) = app.state::<AppState>().0.lock() {
                    if workspace.active_job.is_some() {
                        workspace.exit_after_job = true;
                        let _ = workspace.cancel();
                        api.prevent_exit();
                    }
                }
            }
        });
}
