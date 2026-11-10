//! Native window geometry regression; uses isolated storage and no chemistry jobs.
use reactqm_lib::{
    catalog::{Catalog, CatalogState},
    layout::Panel,
    window_geometry::{self, GeometryService},
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::Manager;
fn pause() {
    std::thread::sleep(Duration::from_millis(250));
}
fn probe(app: &tauri::AppHandle) -> Result<(), String> {
    window_geometry::setup(app)?;
    let make = || {
        tauri::WebviewWindowBuilder::new(
            app,
            "panel-result",
            tauri::WebviewUrl::App("index.html".into()),
        )
        .visible(false)
        .title("ReactQM geometry probe")
        .inner_size(480.0, 420.0)
        .min_inner_size(360.0, 320.0)
        .build()
        .map_err(|e| e.to_string())
    };
    let window = make()?;
    window
        .set_position(tauri::PhysicalPosition::new(80, 100))
        .map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    pause();
    window_geometry::capture(app, Panel::Result);
    // Exercise the coalesced background save before testing the shutdown flush.
    std::thread::sleep(Duration::from_millis(800));
    let saved_path = std::path::PathBuf::from(std::env::var_os("REACTQM_DATA_DIR").unwrap())
        .join("panel-geometry.json");
    if !saved_path.exists() {
        return Err("debounced native geometry save missing".into());
    }
    window_geometry::flush(app);
    let expected_pos = window.outer_position().map_err(|e| e.to_string())?;
    let expected_size = window.inner_size().map_err(|e| e.to_string())?;
    window.destroy().map_err(|e| e.to_string())?;
    pause();
    // Reload from disk, so the test exercises persistence rather than just memory.
    window_geometry::setup(app)?;
    let window = make()?;
    window_geometry::restore(&window, Panel::Result)?;
    window.show().map_err(|e| e.to_string())?;
    pause();
    if window.outer_position().map_err(|e| e.to_string())? != expected_pos
        || window.inner_size().map_err(|e| e.to_string())? != expected_size
    {
        return Err("restored native geometry differs".into());
    }
    window
        .set_position(tauri::PhysicalPosition::new(90000, 90000))
        .map_err(|e| e.to_string())?;
    pause();
    window_geometry::recover(&window)?;
    pause();
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let monitors = window.available_monitors().map_err(|e| e.to_string())?;
    if !monitors.iter().any(|m| {
        let a = m.work_area();
        pos.x >= a.position.x
            && pos.y >= a.position.y
            && (pos.x as i64) < a.position.x as i64 + a.size.width as i64
            && (pos.y as i64) < a.position.y as i64 + a.size.height as i64
    }) {
        return Err("native offscreen recovery failed".into());
    }
    window_geometry::capture(app, Panel::Result);
    window_geometry::flush(app);
    window_geometry::reset(app)?;
    println!("PASS: native size/position persistence, debounced save, disk reload, window recreation, offscreen recovery, reset; monitors={}", monitors.len());
    Ok(())
}
fn main() {
    let root = std::env::temp_dir().join(format!("reactqm-window-probe-{}", uuid::Uuid::new_v4()));
    std::env::set_var("REACTQM_DATA_DIR", &root);
    let outcome = Arc::new(Mutex::new(None));
    let result = outcome.clone();
    let storage = root.clone();
    tauri::Builder::default()
        .manage(GeometryService::default())
        .setup(move |app| {
            app.manage(CatalogState(Catalog::open(&storage)));
            let app = app.handle().clone();
            let result = result.clone();
            std::thread::spawn(move || {
                *result.lock().unwrap() = Some(probe(&app));
                app.exit(0);
            });
            Ok(())
        })
        .build(reactqm_lib::native_context())
        .expect("native probe app")
        .run(|_, _| {});
    match outcome.lock().unwrap().take() {
        Some(Ok(())) => {}
        other => {
            eprintln!("Native geometry probe failed: {other:?}");
            std::process::exit(1);
        }
    }
    std::thread::sleep(Duration::from_millis(500));
    let _ = std::fs::remove_dir_all(root);
}
