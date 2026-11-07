pub mod catalog;
pub mod document;
pub mod jobs;
pub mod layout;
pub mod panel_windows;
pub mod project;
pub mod renderer;
pub mod scheduler;
pub mod state;

use catalog::{Catalog, CatalogState, JobPage, QueuePage};
use document::Document;
use jobs::JobRequest;
use project::Project;
use scheduler::Scheduler;
use state::{AppState, JobSnapshot, Locale, WorkspaceSnapshot};
use std::path::PathBuf;
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
    panel_windows::update_titles(&app);
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
    app.state::<CatalogState>().get()?;
    let path = storage_directory(&app)?.join("workspace.json");
    let snapshot = {
        let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
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
    id: Option<String>,
) -> Result<WorkspaceSnapshot, String> {
    let snapshot = {
        let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
        let active_id = workspace.active_job.as_ref().ok_or("no_job")?.id.clone();
        if id.as_ref().is_some_and(|id| id != &active_id) {
            return Err("no_job".into());
        }
        let id = active_id;
        app.state::<CatalogState>().get()?.request_cancel(&id)?;
        workspace.cancel()?;
        workspace.snapshot()
    };
    publish(&app, snapshot)
}
#[tauri::command]
fn start_calculation(
    app: tauri::AppHandle,
    expected_revision: u64,
) -> Result<WorkspaceSnapshot, String> {
    launch_calculation(app, expected_revision, None)
}

fn launch_calculation(
    app: tauri::AppHandle,
    expected_revision: u64,
    source: Option<JobRequest>,
) -> Result<WorkspaceSnapshot, String> {
    let runner = app.state::<Scheduler>();
    let gate = runner.gate.lock().map_err(|e| e.to_string())?;
    if gate.stopping {
        return Err("queue_stopping".into());
    }
    let snapshot = {
        let state = app.state::<AppState>();
        let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
        check_revision(&workspace, expected_revision)?;
        let (request, retry_of) = if let Some(source) = source {
            (
                source.retry(workspace.document_revision)?,
                Some(source.job_id),
            )
        } else {
            (
                JobRequest::new(
                    workspace.project.document.clone().ok_or("no_document")?,
                    workspace.document_revision,
                )?,
                None,
            )
        };
        app.state::<CatalogState>()
            .get()?
            .enqueue(&request, retry_of.as_deref())?;
        let revision = workspace.document_revision;
        workspace
            .result_targets
            .insert(request.job_id.clone(), revision);
        workspace.revision += 1;
        workspace.snapshot()
    };
    drop(gate);
    runner.wake();
    let _ = app.emit("job-history-changed", ());
    publish(&app, snapshot)
}
#[tauri::command]
async fn queue_snapshot(app: tauri::AppHandle, after: Option<i64>) -> Result<QueuePage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<CatalogState>().get()?.queue_page(after)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
fn set_queue_paused(app: tauri::AppHandle, paused: bool) -> Result<WorkspaceSnapshot, String> {
    let runner = app.state::<Scheduler>();
    let gate = runner.gate.lock().map_err(|e| e.to_string())?;
    if gate.stopping {
        return Err("queue_stopping".into());
    }
    app.state::<CatalogState>()
        .get()?
        .set_queue_paused(paused)?;
    drop(gate);
    runner.wake();
    let _ = app.emit("job-history-changed", ());
    let snapshot = {
        let state = app.state::<AppState>();
        let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
        workspace.revision += 1;
        workspace.snapshot()
    };
    publish(&app, snapshot)
}
#[tauri::command]
fn cancel_queued_job(app: tauri::AppHandle, id: String) -> Result<WorkspaceSnapshot, String> {
    app.state::<CatalogState>().get()?.cancel_queued(&id)?;
    let _ = app.emit("job-history-changed", ());
    let snapshot = {
        let state = app.state::<AppState>();
        let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
        workspace.result_targets.remove(&id);
        workspace.revision += 1;
        workspace.snapshot()
    };
    publish(&app, snapshot)
}

#[tauri::command]
async fn list_jobs(app: tauri::AppHandle, before: Option<i64>) -> Result<JobPage, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<CatalogState>().get()?.page(before))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn restore_job(
    app: tauri::AppHandle,
    id: String,
    expected_revision: u64,
) -> Result<WorkspaceSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let project = app.state::<CatalogState>().get()?.restore(&id);
        let _ = app.emit("job-history-changed", ());
        let project = project?;
        let snapshot = {
            let state = app.state::<AppState>();
            let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
            check_revision(&workspace, expected_revision)?;
            workspace.replace_project(project)?;
            workspace.job = Some(JobSnapshot {
                id,
                status: "succeeded".into(),
                cycle: 0,
                error: None,
            });
            workspace.snapshot()
        };
        publish(&app, snapshot)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn retry_job(
    app: tauri::AppHandle,
    id: String,
    expected_revision: u64,
) -> Result<WorkspaceSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let request = app.state::<CatalogState>().get()?.retry_source(&id)?;
        launch_calculation(app, expected_revision, Some(request))
    })
    .await
    .map_err(|e| e.to_string())?
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

fn stop_execution(app: &tauri::AppHandle, request_exit: bool) -> bool {
    app.state::<panel_windows::LayoutService>()
        .closing
        .store(true, std::sync::atomic::Ordering::SeqCst);
    app.state::<Scheduler>().shutdown();
    let _ = app
        .state::<CatalogState>()
        .get()
        .and_then(|store| store.set_queue_paused(true));
    if let Ok(mut workspace) = app.state::<AppState>().0.lock() {
        if let Some(active) = &workspace.active_job {
            let _ = app
                .state::<CatalogState>()
                .get()
                .and_then(|store| store.request_cancel(&active.id));
            if request_exit {
                workspace.exit_after_job = true;
            }
            let _ = workspace.cancel();
            return true;
        }
    }
    false
}

pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .manage(Scheduler::default())
        .manage(panel_windows::LayoutService::default())
        .setup(|app| {
            let catalog = storage_directory(app.handle()).and_then(|root| Catalog::open(&root));
            app.manage(CatalogState(catalog));
            let exists = storage_directory(app.handle())
                .map(|dir| dir.join("workspace.json").is_file())
                .unwrap_or(false);
            app.state::<AppState>().0.lock().unwrap().saved_available = exists;
            Scheduler::start(app.handle().clone());
            panel_windows::setup(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            update_viewport,
            detach_viewport,
            workspace_snapshot,
            panel_windows::layout_snapshot,
            panel_windows::update_layout,
            set_locale,
            import_xyz,
            history_step,
            save_project,
            load_project,
            queue_snapshot,
            set_queue_paused,
            cancel_queued_job,
            list_jobs,
            restore_job,
            retry_job,
            start_calculation,
            cancel_job
        ])
        .build(tauri::generate_context!())
        .expect("ReactQM failed to start")
        .run(|app, event| {
            if let tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } = &event
            {
                if let Some(panel) = layout::Panel::from_label(label) {
                    panel_windows::close_requested(app, panel, api);
                }
            }
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
                    app.exit(0);
                }
            }
            if let tauri::RunEvent::Exit = event {
                stop_execution(app, false);
                app.state::<Scheduler>().join();
            }
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if stop_execution(app, true) {
                    api.prevent_exit();
                }
            }
        });
}
