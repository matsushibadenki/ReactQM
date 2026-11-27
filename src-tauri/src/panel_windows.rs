//! Native utility windows project the same AppState. Window creation never holds its mutex.
use crate::{
    catalog::CatalogState,
    layout::{Action, Panel, PanelLayout, Zone},
    state::{AppState, Locale},
};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{Emitter, Manager};

#[derive(Default)]
pub struct LayoutService {
    gate: tauri::async_runtime::Mutex<()>,
    error: Mutex<Option<String>>,
    pub closing: AtomicBool,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutView {
    pub layout: PanelLayout,
    pub error: Option<String>,
}
#[derive(Clone, Serialize)]
struct PanelFocus {
    panel: Panel,
    revision: u64,
}

fn view(app: &tauri::AppHandle) -> Result<LayoutView, String> {
    Ok(LayoutView {
        layout: app
            .state::<AppState>()
            .0
            .lock()
            .map_err(|e| e.to_string())?
            .layout
            .clone(),
        error: app
            .state::<LayoutService>()
            .error
            .lock()
            .map_err(|e| e.to_string())?
            .clone(),
    })
}
fn publish(app: &tauri::AppHandle) -> Result<LayoutView, String> {
    let snapshot = view(app)?;
    let _ = app.emit("layout-changed", &snapshot);
    Ok(snapshot)
}
fn title(panel: Panel, locale: Locale) -> String {
    let name = match (panel, locale) {
        (Panel::Result, Locale::En) => "Calculation",
        (Panel::Result, Locale::Ja) => "計算",
        (Panel::Result, Locale::ZhCn) => "计算",
        (Panel::History, Locale::En) => "Calculation history",
        (Panel::History, Locale::Ja) => "計算履歴",
        (Panel::History, Locale::ZhCn) => "计算历史",
    };
    format!("ReactQM · {name}")
}
pub fn update_titles(app: &tauri::AppHandle, locale: Locale) {
    for panel in Panel::ALL {
        if let Some(window) = app.get_webview_window(panel.label()) {
            let _ = window.set_title(&title(panel, locale));
        }
    }
}
fn create(app: &tauri::AppHandle, panel: Panel) -> Result<tauri::WebviewWindow, String> {
    if let Some(window) = app.get_webview_window(panel.label()) {
        return Ok(window);
    }
    let locale = app
        .state::<AppState>()
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .locale;
    let window = tauri::WebviewWindowBuilder::new(
        app,
        panel.label(),
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title(title(panel, locale))
    .inner_size(480.0, 640.0)
    .min_inner_size(360.0, 320.0)
    .center()
    .visible(false)
    .disable_drag_drop_handler()
    .build()
    .map_err(|e| format!("panel_window: {e}"))?;
    if let Err(error) = crate::window_geometry::restore(&window, panel) {
        let _ = window.destroy();
        return Err(error);
    }
    Ok(window)
}
#[tauri::command]
pub fn layout_snapshot(app: tauri::AppHandle) -> Result<LayoutView, String> {
    view(&app)
}

#[tauri::command]
pub async fn update_layout(
    app: tauri::AppHandle,
    expected_revision: u64,
    action: Action,
) -> Result<LayoutView, String> {
    change(&app, Some(expected_revision), Some(action), None).await
}
async fn change(
    app: &tauri::AppHandle,
    expected: Option<u64>,
    action: Option<Action>,
    redock: Option<Panel>,
) -> Result<LayoutView, String> {
    let service = app.state::<LayoutService>();
    let _gate = service.gate.lock().await;
    if service.closing.load(Ordering::SeqCst) {
        return Err("queue_stopping".into());
    }
    app.state::<CatalogState>().get()?; // Reuse the lifetime workspace lease.
    let old = app
        .state::<AppState>()
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .layout
        .clone();
    if expected.is_some_and(|revision| revision != old.revision) {
        return Err("stale_layout".into());
    }
    let focus = match &action {
        Some(Action::Activate { panel } | Action::Move { panel, .. }) => Some(*panel),
        Some(Action::Reset) => Some(Panel::Result),
        _ => redock,
    };
    let reset = matches!(action, Some(Action::Reset));
    if service.error.lock().map_err(|e| e.to_string())?.is_some() && !reset {
        return Err("layout_invalid".into());
    }
    let next = if let Some(panel) = redock {
        if old.zone(panel) != Zone::Floating {
            return view(app);
        }
        old.redock(panel)
    } else {
        old.apply(action.ok_or("layout_invalid")?)?
    };
    let path = crate::storage_directory(app)?.join("layout.json");
    let mut created = Vec::new();
    for panel in Panel::ALL {
        if next.zone(panel) == Zone::Floating && old.zone(panel) != Zone::Floating {
            match create(app, panel) {
                Ok(window) => created.push(window),
                Err(error) => {
                    for window in created {
                        let _ = window.destroy();
                    }
                    return Err(error);
                }
            }
        }
    }
    if reset {
        crate::window_geometry::reset(app)?;
    } else {
        for panel in Panel::ALL {
            if old.zone(panel) == Zone::Floating && next.zone(panel) != Zone::Floating {
                crate::window_geometry::capture(app, panel);
            }
        }
        crate::window_geometry::flush(app);
    }
    if let Err(error) = next.save(&path) {
        for window in created {
            let _ = window.destroy();
        }
        return Err(error);
    }
    app.state::<AppState>()
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .layout = next.clone();
    *service.error.lock().map_err(|e| e.to_string())? = None;
    for panel in Panel::ALL {
        if next.zone(panel) != Zone::Floating {
            if let Some(window) = app.get_webview_window(panel.label()) {
                let _ = window.destroy();
            }
        }
    }
    let snapshot = publish(app)?;
    for window in created {
        if let Err(error) = window.show() {
            *service.error.lock().map_err(|e| e.to_string())? =
                Some(format!("panel_window: {error}"));
            return publish(app);
        }
        let _ = window.set_focus();
    }
    if let Some(panel) = focus {
        let label = if next.zone(panel) == Zone::Floating {
            panel.label()
        } else {
            "main"
        };
        if let Some(window) = app.get_webview_window(label) {
            window
                .unminimize()
                .map_err(|e| format!("panel_window: {e}"))?;
            if next.zone(panel) == Zone::Floating {
                crate::window_geometry::recover(&window)?;
            }
            let _ = window.show();
            let _ = window.set_focus();
            let _ = app.emit_to(
                label,
                "panel-focus",
                PanelFocus {
                    panel,
                    revision: next.revision,
                },
            );
        }
    }
    Ok(snapshot)
}
pub fn setup(app: &tauri::AppHandle) -> Result<(), String> {
    let result = (|| {
        app.state::<CatalogState>().get()?;
        let path = crate::storage_directory(app)?.join("layout.json");
        if path.try_exists().map_err(|e| format!("storage: {e}"))? {
            PanelLayout::load(&path)
        } else {
            Ok(PanelLayout::default())
        }
    })();
    match result {
        Ok(layout) => {
            app.state::<AppState>()
                .0
                .lock()
                .map_err(|e| e.to_string())?
                .layout = layout
        }
        Err(error) => {
            *app.state::<LayoutService>()
                .error
                .lock()
                .map_err(|e| e.to_string())? = Some(error)
        }
    }
    if let Err(error) = crate::window_geometry::setup(app) {
        *app.state::<LayoutService>()
            .error
            .lock()
            .map_err(|e| e.to_string())? = Some(error);
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let service = app.state::<LayoutService>();
        let _gate = service.gate.lock().await;
        let panels = match view(&app) {
            Ok(snapshot) => snapshot.layout.panels,
            Err(error) => {
                let _ = app.emit("panel-error", error);
                return;
            }
        };
        for placement in panels {
            if service.closing.load(Ordering::SeqCst) {
                break;
            }
            if placement.zone == Zone::Floating {
                if let Err(error) = create(&app, placement.id)
                    .and_then(|window| window.show().map_err(|e| format!("panel_window: {e}")))
                {
                    match service.error.lock() {
                        Ok(mut stored) => *stored = Some(error),
                        Err(poisoned) => {
                            let _ = app.emit("panel-error", poisoned.to_string());
                            return;
                        }
                    }
                }
            }
        }
        if let Err(error) = publish(&app) {
            let _ = app.emit("panel-error", error);
        }
    });
    Ok(())
}
pub fn close_requested(app: &tauri::AppHandle, panel: Panel, api: &tauri::CloseRequestApi) {
    if app.state::<LayoutService>().closing.load(Ordering::SeqCst) {
        return;
    }
    api.prevent_close();
    crate::window_geometry::capture(app, panel);
    crate::window_geometry::flush(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = change(&app, None, None, Some(panel)).await {
            let _ = app.emit("panel-error", error);
        }
    });
}
