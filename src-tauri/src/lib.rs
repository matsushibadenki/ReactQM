pub mod document;
pub mod renderer;
pub mod state;

use state::{AppState, Locale, WorkspaceSnapshot};
use tauri::{Emitter, State};

#[tauri::command]
fn workspace_snapshot(state: State<'_, AppState>) -> Result<WorkspaceSnapshot, String> {
    Ok(state
        .0
        .lock()
        .map_err(|error| error.to_string())?
        .snapshot())
}

#[tauri::command]
fn set_locale(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    locale: Locale,
) -> Result<WorkspaceSnapshot, String> {
    let snapshot = {
        let mut workspace = state.0.lock().map_err(|error| error.to_string())?;
        workspace.locale = locale;
        workspace.snapshot()
    };
    app.emit("workspace-changed", &snapshot)
        .map_err(|error| error.to_string())?;
    Ok(snapshot)
}

pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![workspace_snapshot, set_locale])
        .run(tauri::generate_context!())
        .expect("ReactQM failed to start");
}
