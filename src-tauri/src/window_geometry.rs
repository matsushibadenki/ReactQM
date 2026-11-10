//! Native panel geometry, independent of document and dock-layout revisions.
use crate::{catalog::CatalogState, layout::Panel};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, path::Path, sync::Mutex};
use tauri::{Emitter, Manager};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Geometry {
    // Desktop coordinates stay physical; only client size is DPI independent.
    x: i32,
    y: i32,
    width: f64,
    height: f64,
}
impl Geometry {
    fn valid(self) -> bool {
        self.x.abs_diff(0) <= 1_000_000
            && self.y.abs_diff(0) <= 1_000_000
            && self.width.is_finite()
            && self.height.is_finite()
            && (360.0..=16384.0).contains(&self.width)
            && (320.0..=16384.0).contains(&self.height)
    }
    fn fit(self, areas: &[Area]) -> Option<Self> {
        let area = areas.iter().filter(|a| a.valid()).max_by_key(|a| {
            // Prefer the monitor containing the saved title bar; otherwise nearest.
            let dx = (self.x as i64
                - (self.x as i64).clamp(a.x as i64, a.x as i64 + a.width as i64))
                as i128;
            let dy = (self.y as i64
                - (self.y as i64).clamp(a.y as i64, a.y as i64 + a.height as i64))
                as i128;
            -(dx * dx + dy * dy)
        })?;
        let width = self.width.min(area.width as f64 / area.scale).max(360.0);
        // Reserve native decoration space above/below the client area.
        let height = self
            .height
            .min(area.height as f64 / area.scale - 48.0)
            .max(320.0);
        let outer_width = (width * area.scale).ceil() as i64;
        let outer_height = ((height + 48.0) * area.scale).ceil() as i64;
        let x = (self.x as i64).clamp(
            area.x as i64,
            (area.x as i64 + area.width as i64 - outer_width).max(area.x as i64),
        );
        let y = (self.y as i64).clamp(
            area.y as i64,
            (area.y as i64 + area.height as i64 - outer_height).max(area.y as i64),
        );
        Some(Self {
            x: x as i32,
            y: y as i32,
            width,
            height,
        })
    }
}
#[derive(Clone, Copy)]
struct Area {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
}
impl Area {
    fn valid(&self) -> bool {
        self.width > 0 && self.height > 0 && self.scale.is_finite() && self.scale > 0.0
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    schema_version: u32,
    result: Option<Geometry>,
    history: Option<Geometry>,
}
impl Default for Saved {
    fn default() -> Self {
        Self {
            schema_version: 1,
            result: None,
            history: None,
        }
    }
}
impl Saved {
    fn get(&self, panel: Panel) -> Option<Geometry> {
        match panel {
            Panel::Result => self.result,
            Panel::History => self.history,
        }
    }
    fn set(&mut self, panel: Panel, geometry: Geometry) {
        match panel {
            Panel::Result => self.result = Some(geometry),
            Panel::History => self.history = Some(geometry),
        }
    }
    fn load(path: &Path) -> Result<Self, String> {
        let mut bytes = Vec::new();
        File::open(path)
            .and_then(|f| f.take(4097).read_to_end(&mut bytes))
            .map_err(|e| format!("storage: {e}"))?;
        if bytes.len() > 4096 {
            return Err("layout_invalid".into());
        }
        let saved: Self = serde_json::from_slice(&bytes).map_err(|_| "layout_invalid")?;
        if saved.schema_version != 1
            || [saved.result, saved.history]
                .into_iter()
                .flatten()
                .any(|g| !g.valid())
        {
            return Err("layout_invalid".into());
        }
        Ok(saved)
    }
}
#[derive(Default)]
struct State {
    saved: Saved,
    invalid: bool,
    generation: u64,
    writing: bool,
}
#[derive(Default)]
pub struct GeometryService(Mutex<State>);
fn path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(crate::storage_directory(app)?.join("panel-geometry.json"))
}
fn report(app: &tauri::AppHandle, error: String) {
    let _ = app.emit("panel-error", error);
}
pub fn setup(app: &tauri::AppHandle) -> Result<(), String> {
    let path = path(app)?;
    if !path.try_exists().map_err(|e| format!("storage: {e}"))? {
        return Ok(());
    }
    let service = app.state::<GeometryService>();
    let mut state = service.0.lock().unwrap();
    match Saved::load(&path) {
        Ok(saved) => {
            state.saved = saved;
            Ok(())
        }
        Err(error) => {
            state.invalid = true;
            Err(error)
        }
    }
}
pub fn reset(app: &tauri::AppHandle) -> Result<(), String> {
    app.state::<CatalogState>().get()?;
    let service = app.state::<GeometryService>();
    let mut state = service.0.lock().unwrap();
    crate::project::atomic_json(&path(app)?, &Saved::default())?;
    state.saved = Saved::default();
    state.invalid = false;
    state.generation += 1;
    Ok(())
}
pub fn restore(window: &tauri::WebviewWindow, panel: Panel) -> Result<(), String> {
    let app = window.app_handle();
    let saved = app
        .state::<GeometryService>()
        .0
        .lock()
        .unwrap()
        .saved
        .get(panel);
    if let Some(saved) = saved {
        apply(window, saved)?;
    }
    Ok(())
}
fn apply(window: &tauri::WebviewWindow, geometry: Geometry) -> Result<(), String> {
    let monitors = window
        .available_monitors()
        .map_err(|e| format!("panel_window: {e}"))?;
    let areas: Vec<_> = monitors
        .iter()
        .map(|m| {
            let a = m.work_area();
            Area {
                x: a.position.x,
                y: a.position.y,
                width: a.size.width,
                height: a.size.height,
                scale: m.scale_factor(),
            }
        })
        .collect();
    if let Some(fit) = geometry.fit(&areas) {
        window
            .set_size(tauri::LogicalSize::new(fit.width, fit.height))
            .map_err(|e| format!("panel_window: {e}"))?;
        window
            .set_position(tauri::PhysicalPosition::new(fit.x, fit.y))
            .map_err(|e| format!("panel_window: {e}"))?;
    }
    Ok(())
}
fn read(window: &tauri::WebviewWindow) -> Option<Geometry> {
    if !window.is_visible().ok()?
        || window.is_minimized().ok()?
        || window.is_maximized().ok()?
        || window.is_fullscreen().ok()?
    {
        return None;
    }
    let position = window.outer_position().ok()?;
    let size = window
        .inner_size()
        .ok()?
        .to_logical::<f64>(window.scale_factor().ok()?);
    let g = Geometry {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    };
    g.valid().then_some(g)
}
/// Re-show also recovers a panel after its monitor has been disconnected.
pub fn recover(window: &tauri::WebviewWindow) -> Result<(), String> {
    if let Some(g) = read(window) {
        apply(window, g)?;
    }
    Ok(())
}
pub fn capture(app: &tauri::AppHandle, panel: Panel) {
    let Some(window) = app.get_webview_window(panel.label()) else {
        return;
    };
    let Some(geometry) = read(&window) else {
        return;
    };
    let service = app.state::<GeometryService>();
    let mut state = service.0.lock().unwrap();
    if state.invalid || state.saved.get(panel) == Some(geometry) {
        return;
    }
    state.saved.set(panel, geometry);
    state.generation += 1;
    if state.writing {
        return;
    }
    state.writing = true;
    let app = app.clone();
    std::thread::spawn(move || loop {
        let generation = app.state::<GeometryService>().0.lock().unwrap().generation;
        std::thread::sleep(std::time::Duration::from_millis(350));
        let service = app.state::<GeometryService>();
        let mut state = service.0.lock().unwrap();
        if generation != state.generation {
            continue;
        }
        if !state.invalid {
            if let Err(error) = save(&app, &state.saved) {
                report(&app, error);
            }
        }
        state.writing = false;
        break;
    });
}
fn save(app: &tauri::AppHandle, saved: &Saved) -> Result<(), String> {
    app.state::<CatalogState>().get()?;
    crate::project::atomic_json(&path(app)?, saved)
}
/// Synchronous flush before exit; do not rely on a delayed task surviving shutdown.
pub fn flush(app: &tauri::AppHandle) {
    let service = app.state::<GeometryService>();
    let state = service.0.lock().unwrap();
    if state.writing && !state.invalid {
        if let Err(error) = save(app, &state.saved) {
            report(app, error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn area(x: i32, scale: f64) -> Area {
        Area {
            x,
            y: 24,
            width: 1920,
            height: 1056,
            scale,
        }
    }
    #[test]
    fn unplugged_monitor_and_negative_origins_are_recovered() {
        let g = Geometry {
            x: -1800,
            y: 70,
            width: 480.0,
            height: 640.0,
        };
        assert_eq!(g.fit(&[area(-1920, 1.0), area(0, 1.0)]), Some(g));
        let recovered = g.fit(&[area(0, 1.0)]).unwrap();
        assert_eq!(recovered.x, 0);
        assert_eq!(recovered.width, 480.0);
        assert!(g.fit(&[]).is_none());
    }
    #[test]
    fn dpi_and_work_area_bound_size_and_title_bar() {
        let g = Geometry {
            x: 1900,
            y: -200,
            width: 1400.0,
            height: 1200.0,
        };
        let fit = g.fit(&[area(0, 2.0)]).unwrap();
        assert_eq!(fit.width, 960.0);
        assert_eq!(fit.height, 480.0);
        assert_eq!(fit.x, 0);
        assert_eq!(fit.y, 24);
        assert!(!Geometry {
            width: f64::NAN,
            ..g
        }
        .valid());
        assert!(!Geometry { x: i32::MIN, ..g }.valid());
    }
    #[test]
    fn bounded_storage_rejects_invalid_data_and_roundtrips_both_panels() {
        let dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let path = dir.join("geometry.json");
        let mut saved = Saved::default();
        let g = Geometry {
            x: -1500,
            y: 24,
            width: 480.0,
            height: 640.0,
        };
        saved.set(Panel::Result, g);
        saved.set(Panel::History, Geometry { x: 100, ..g });
        crate::project::atomic_json(&path, &saved).unwrap();
        let loaded = Saved::load(&path).unwrap();
        assert_eq!(loaded.result, saved.result);
        assert_eq!(loaded.history, saved.history);
        std::fs::write(
            &path,
            b"{\"schema_version\":1,\"result\":{\"x\":0,\"y\":0,\"width\":0,\"height\":640}}",
        )
        .unwrap();
        assert!(Saved::load(&path).is_err());
        std::fs::write(&path, b"{\"schema_version\":2}").unwrap();
        assert!(Saved::load(&path).is_err());
        std::fs::write(&path, vec![b' '; 4097]).unwrap();
        assert!(Saved::load(&path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
