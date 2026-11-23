pub mod barriers;
pub mod catalog;
pub mod document;
pub mod dynamics;
mod editor;
pub mod escalation;
pub mod fields;
pub mod jobs;
pub mod layout;
pub mod mace;
pub mod mace_reference;
pub mod neb;
pub mod panel_windows;
pub mod project;
pub mod rates;
mod reaction;
pub mod refinement;
pub mod renderer;
pub mod scheduler;
pub mod settings;
pub mod solution;
pub mod solution_path;
pub mod state;
pub mod stoichiometry;
pub mod structure;
pub mod thermochemistry;
pub mod transition;
pub mod valence;
pub mod window_geometry;
pub mod xtb_followup;

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
        let settings = workspace.project.settings.clone().for_spin(document.spin);
        workspace.replace_project(Project {
            settings,
            document: Some(document),
            calculation: None,
        })?;
        workspace.snapshot()
    };
    publish(&app, snapshot)
}
#[tauri::command]
fn set_multiplicity(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    expected_revision: u64,
    multiplicity: u32,
) -> Result<WorkspaceSnapshot, String> {
    let spin = multiplicity.checked_sub(1).ok_or("electron_spin")?;
    let snapshot = {
        let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
        check_revision(&workspace, expected_revision)?;
        let mut project = workspace.project.clone();
        let document = project.document.as_mut().ok_or("no_document")?;
        if document.spin != spin {
            document.spin = spin;
            document.validate()?;
            project.calculation = None;
            project.settings = project.settings.for_spin(spin);
            workspace.replace_project(project)?;
        }
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
fn set_calculation_settings(
    app: tauri::AppHandle,
    expected_revision: u64,
    settings: settings::CalculationSettings,
) -> Result<WorkspaceSnapshot, String> {
    settings.validate()?;
    let snapshot = {
        let state = app.state::<AppState>();
        let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
        check_revision(&workspace, expected_revision)?;
        let mut project = workspace.project.clone();
        if let Some(document) = &project.document {
            if settings.method == settings::SCREENING {
                settings.validate_screening_input(document)?;
            } else if document.spin > 0 && !settings.is_unrestricted() {
                return Err("rhf_spin".into());
            }
        }
        project.settings = settings;
        workspace.replace_project(project)?;
        workspace.snapshot()
    };
    publish(&app, snapshot)
}
#[tauri::command]
fn start_calculation(
    app: tauri::AppHandle,
    expected_revision: u64,
) -> Result<WorkspaceSnapshot, String> {
    launch_calculation(app, expected_revision, None, false)
}

#[tauri::command]
async fn start_mace(
    app: tauri::AppHandle,
    expected_revision: u64,
) -> Result<WorkspaceSnapshot, String> {
    let request = {
        let state = app.state::<AppState>();
        let workspace = state.0.lock().map_err(|_| "storage")?;
        check_revision(&workspace, expected_revision)?;
        JobRequest::mace(
            workspace.project.document.clone().ok_or("no_document")?,
            expected_revision,
        )?
    };
    queue_verified_job(app, expected_revision, move |_| {
        if !std::env::var_os("REACTQM_MACE_MODEL")
            .map(PathBuf::from)
            .is_some_and(|path| path.is_absolute() && path.is_file())
        {
            return Err("mace_checkpoint_missing".into());
        }
        Ok(request)
    })
    .await
}

#[tauri::command]
async fn start_xtb_followup(
    app: tauri::AppHandle,
    expected_revision: u64,
    id: String,
    budget: u8,
) -> Result<WorkspaceSnapshot, String> {
    queue_verified_job(app, expected_revision, move |store| {
        xtb_followup::from_history(store, &id, budget, expected_revision)
    })
    .await
}

#[tauri::command]
async fn start_escalation(
    app: tauri::AppHandle,
    expected_revision: u64,
    id: String,
    budget: u8,
) -> Result<WorkspaceSnapshot, String> {
    queue_verified_job(app, expected_revision, move |store| {
        escalation::from_history(store, &id, budget, expected_revision)
    })
    .await
}

#[tauri::command]
async fn start_mace_reference(
    app: tauri::AppHandle,
    expected_revision: u64,
    ids: Vec<String>,
    budget: usize,
    warnings_only: Option<bool>,
) -> Result<WorkspaceSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = app.state::<CatalogState>();
        let store = catalog.get()?;
        let requests = mace_reference::from_history(
            store,
            ids,
            budget,
            expected_revision,
            warnings_only.unwrap_or(false),
        )?;
        if requests.is_empty() {
            return Err("mace_reference_no_warning".into());
        };
        let runner = app.state::<Scheduler>();
        let gate = runner.gate.lock().map_err(|_| "storage")?;
        if gate.stopping {
            return Err("queue_stopping".into());
        }
        let snapshot = {
            let state = app.state::<AppState>();
            let mut workspace = state.0.lock().map_err(|_| "storage")?;
            check_revision(&workspace, expected_revision)?;
            store.enqueue_reference_batch(&requests)?;
            workspace.revision += 1;
            workspace.snapshot()
        };
        drop(gate);
        runner.wake();
        let _ = app.emit("job-history-changed", ());
        publish(&app, snapshot)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn mace_force(
    app: tauri::AppHandle,
    expected_revision: u64,
    job_id: String,
    index: usize,
) -> Result<[f64; 3], String> {
    let state = app.state::<AppState>();
    let workspace = state.0.lock().map_err(|_| "storage")?;
    check_revision(&workspace, expected_revision)?;
    workspace
        .project
        .calculation
        .as_ref()
        .filter(|result| result.job_id == job_id)
        .and_then(|result| {
            result
                .mace
                .as_ref()
                .and_then(|report| report.forces_ev_per_angstrom.get(index))
                .or_else(|| {
                    result
                        .mace_reference
                        .as_ref()
                        .and_then(|report| report.forces_ev_per_angstrom.get(index))
                })
        })
        .copied()
        .ok_or_else(|| "result_invalid".into())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoichiometrySelection {
    temperature_kelvin: f64,
    quasi_rrho: bool,
    reactants: Vec<stoichiometry::Selection>,
    products: Vec<stoichiometry::Selection>,
    saddle: Option<String>,
}

#[tauri::command]
async fn start_stoichiometry(
    app: tauri::AppHandle,
    expected_revision: u64,
    selection: StoichiometrySelection,
) -> Result<WorkspaceSnapshot, String> {
    queue_verified_job(app, expected_revision, move |store| {
        stoichiometry::from_history(
            store,
            expected_revision,
            selection.temperature_kelvin,
            selection.quasi_rrho,
            selection.reactants,
            selection.products,
            selection.saddle,
        )
    })
    .await
}

#[tauri::command]
async fn start_solution_path(
    app: tauri::AppHandle,
    id: String,
    expected_revision: u64,
    solvent: thermochemistry::Solvent,
) -> Result<WorkspaceSnapshot, String> {
    queue_verified_job(app, expected_revision, move |store| {
        solution_path::from_transition(store, &id, expected_revision, solvent)
    })
    .await
}

#[tauri::command]
async fn start_solution(
    app: tauri::AppHandle,
    expected_revision: u64,
    solvent: thermochemistry::Solvent,
) -> Result<WorkspaceSnapshot, String> {
    let mut request = {
        let state = app.state::<AppState>();
        let workspace = state.0.lock().map_err(|_| "storage")?;
        check_revision(&workspace, expected_revision)?;
        JobRequest::with_settings(
            workspace.project.document.clone().ok_or("no_document")?,
            expected_revision,
            workspace.project.settings.clone(),
        )?
    };
    request.solution = Some(solvent);
    queue_verified_job(app, expected_revision, move |_| Ok(request)).await
}

#[tauri::command]
async fn start_dynamics(
    app: tauri::AppHandle,
    expected_revision: u64,
    input: dynamics::Input,
) -> Result<WorkspaceSnapshot, String> {
    let mut request = {
        let state = app.state::<AppState>();
        let workspace = state.0.lock().map_err(|_| "storage")?;
        check_revision(&workspace, expected_revision)?;
        JobRequest::with_settings(
            workspace.project.document.clone().ok_or("no_document")?,
            expected_revision,
            workspace.project.settings.clone(),
        )?
    };
    request.dynamics = Some(input);
    queue_verified_job(app, expected_revision, move |_| Ok(request)).await
}

#[tauri::command]
async fn start_transition(
    app: tauri::AppHandle,
    id: String,
    expected_revision: u64,
) -> Result<WorkspaceSnapshot, String> {
    queue_verified_job(app, expected_revision, move |store| {
        transition::from_path(store, &id, expected_revision)
    })
    .await
}

async fn queue_verified_job(
    app: tauri::AppHandle,
    expected_revision: u64,
    create: impl FnOnce(&Catalog) -> Result<JobRequest, String> + Send + 'static,
) -> Result<WorkspaceSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let source = create(app.state::<CatalogState>().get()?)?;
        let runner = app.state::<Scheduler>();
        let gate = runner.gate.lock().map_err(|_| "storage")?;
        if gate.stopping {
            return Err("queue_stopping".into());
        }
        let snapshot = {
            let state = app.state::<AppState>();
            let mut workspace = state.0.lock().map_err(|_| "storage")?;
            check_revision(&workspace, expected_revision)?;
            app.state::<CatalogState>().get()?.enqueue(&source, None)?;
            workspace.revision += 1;
            workspace.snapshot()
        };
        drop(gate);
        runner.wake();
        let _ = app.emit("job-history-changed", ());
        publish(&app, snapshot)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn start_thermochemistry(
    app: tauri::AppHandle,
    id: String,
    expected_revision: u64,
    temperature_kelvin: f64,
    standard_state: thermochemistry::StandardState,
    solvent: Option<thermochemistry::Solvent>,
    quasi_rrho: Option<bool>,
) -> Result<WorkspaceSnapshot, String> {
    queue_verified_job(app, expected_revision, move |store| {
        thermochemistry::from_transition(
            store,
            &id,
            expected_revision,
            temperature_kelvin,
            standard_state,
            solvent,
            quasi_rrho.unwrap_or_default(),
        )
    })
    .await
}

#[tauri::command]
async fn start_refinement(
    app: tauri::AppHandle,
    id: String,
    expected_revision: u64,
    method: refinement::Method,
    basis: String,
) -> Result<WorkspaceSnapshot, String> {
    queue_verified_job(app, expected_revision, move |store| {
        refinement::from_transition(store, &id, expected_revision, method, basis)
    })
    .await
}

#[tauri::command]
fn start_optimization(
    app: tauri::AppHandle,
    expected_revision: u64,
) -> Result<WorkspaceSnapshot, String> {
    launch_calculation(app, expected_revision, None, true)
}

#[tauri::command]
async fn resume_optimization(
    app: tauri::AppHandle,
    id: String,
    expected_revision: u64,
) -> Result<WorkspaceSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = app.state::<CatalogState>();
        let store = catalog.get()?;
        let original = store.retry_source(&id)?;
        let request = store.resume_source(&id)?;
        launch_calculation(
            app,
            expected_revision,
            Some((request, Some(original.input_hash))),
            true,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

fn launch_calculation(
    app: tauri::AppHandle,
    expected_revision: u64,
    source: Option<(JobRequest, Option<String>)>,
    optimize: bool,
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
        let (mut request, retry_of, resume_input) = if let Some((source, resume_input)) = source {
            (
                source.retry(workspace.document_revision)?,
                Some(source.job_id),
                resume_input,
            )
        } else {
            (
                JobRequest::with_settings(
                    workspace.project.document.clone().ok_or("no_document")?,
                    workspace.document_revision,
                    workspace.project.settings.clone(),
                )?,
                None,
                None,
            )
        };
        if retry_of.is_none() {
            request.optimize = optimize;
        }
        app.state::<CatalogState>()
            .get()?
            .enqueue(&request, retry_of.as_deref())?;
        let revision = workspace.document_revision;
        let matching_input = workspace.project.document.as_ref().is_some_and(|document| {
            let hash = document.fingerprint();
            hash == request.input_hash || resume_input.as_ref() == Some(&hash)
        });
        if request.refinement.is_none()
            && request.thermochemistry.is_none()
            && request.transition.is_none()
            && request.neb.is_none()
            && request.dynamics.is_none()
            && request.solution.is_none()
            && request.solution_path.is_none()
            && request.stoichiometry.is_none()
            && request.method != mace::METHOD
            && request.mace_reference.is_none()
            && request.escalation.is_none()
            && request.xtb_followup.is_none()
            && (!request.optimize
                || (matching_input && workspace.project.settings == request.effective_settings()))
        {
            workspace
                .result_targets
                .insert(request.job_id.clone(), revision);
        }
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
        launch_calculation(app, expected_revision, Some((request, None)), false)
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
                        neb_image,
                        ..
                    } = &mut *workspace;
                    let job = project
                        .calculation
                        .as_ref()
                        .map(|result| result.job_id.as_str());
                    if neb_image.as_ref().is_some_and(|(revision, source, _)| {
                        *revision != *document_revision || Some(source.as_str()) != job
                    }) {
                        *neb_image = None;
                        renderer.invalidate_scene();
                    }
                    renderer.update(
                        webview.inner().cast(),
                        request,
                        neb_image
                            .as_ref()
                            .map(|(_, _, document)| document)
                            .or(project.document.as_ref()),
                        *document_revision,
                        project
                            .calculation
                            .as_ref()
                            .map(|result| result.job_id.as_str()),
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
    for panel in layout::Panel::ALL {
        window_geometry::capture(app, panel);
    }
    window_geometry::flush(app);
    app.state::<structure::StructureService>()
        .stopping
        .store(true, std::sync::atomic::Ordering::SeqCst);
    app.state::<valence::ValenceService>()
        .stopping
        .store(true, std::sync::atomic::Ordering::SeqCst);
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

/// Shared native context avoids embedding the macOS plist twice in native probes.
pub fn native_context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .manage(dynamics::Playback::default())
        .manage(reaction::ReactionService::default())
        .manage(valence::ValenceService::default())
        .manage(structure::StructureService::default())
        .manage(Scheduler::default())
        .manage(panel_windows::LayoutService::default())
        .manage(window_geometry::GeometryService::default())
        .setup(|app| {
            let catalog = storage_directory(app.handle()).and_then(|root| Catalog::open(&root));
            app.manage(CatalogState(catalog));
            let exists = storage_directory(app.handle())
                .map(|dir| dir.join("workspace.json").is_file())
                .unwrap_or(false);
            app.state::<AppState>().0.lock().unwrap().saved_available = exists;
            let main_config = app
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == "main")
                .ok_or_else(|| std::io::Error::other("Missing main window configuration"))?;
            tauri::WebviewWindowBuilder::from_config(app, main_config)?.build()?;
            Scheduler::start(app.handle().clone());
            panel_windows::setup(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            update_viewport,
            detach_viewport,
            workspace_snapshot,
            reaction::reaction_snapshot,
            reaction::capture_endpoint,
            reaction::save_atom_mapping,
            reaction::start_neb,
            start_dynamics,
            start_solution,
            start_solution_path,
            start_stoichiometry,
            start_mace,
            start_mace_reference,
            start_escalation,
            start_xtb_followup,
            mace_force,
            dynamics::show_frame,
            start_transition,
            start_thermochemistry,
            start_refinement,
            barriers::compare_barriers,
            rates::compare_rates,
            reaction::show_neb_image,
            valence::atom_valence,
            valence::analyze_bonds,
            panel_windows::layout_snapshot,
            panel_windows::update_layout,
            set_locale,
            import_xyz,
            set_multiplicity,
            editor::edit_molecule,
            fields::set_electronic_field,
            structure::import_structure,
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
            start_optimization,
            resume_optimization,
            set_calculation_settings,
            cancel_job
        ])
        .build(native_context())
        .expect("ReactQM failed to start")
        .run(|app, event| {
            if let tauri::RunEvent::WindowEvent { label, event, .. } = &event {
                if matches!(
                    event,
                    tauri::WindowEvent::Moved(_)
                        | tauri::WindowEvent::Resized(_)
                        | tauri::WindowEvent::ScaleFactorChanged { .. }
                ) {
                    if let Some(panel) = layout::Panel::from_label(label) {
                        window_geometry::capture(app, panel);
                    }
                }
            }
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
                valence::shutdown(app);
                structure::shutdown(app);
                app.state::<Scheduler>().join();
            }
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if stop_execution(app, true) {
                    api.prevent_exit();
                }
            }
        });
}
