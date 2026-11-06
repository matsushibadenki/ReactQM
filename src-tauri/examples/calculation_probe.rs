//! Actual Rust → Python → PySCF → HDF5 → project save/reload integration probe.
use reactqm_lib::{
    document::Document,
    jobs::{execute, JobRequest, WorkerConfig},
    project::Project,
};
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
fn main() -> Result<(), String> {
    let directory = std::env::temp_dir().join(format!("reactqm-probe-{}", uuid::Uuid::new_v4()));
    let document = Document::from_xyz("2\nH2\nH 0 0 0\nH 0 0 0.74", 0, 0)?;
    let request = JobRequest::new(document.clone(), 1)?;
    let result = execute(
        &request,
        &directory,
        &WorkerConfig::development(),
        Arc::new(AtomicBool::new(false)),
        Duration::from_secs(120),
        |cycle| println!("SCF cycle {cycle}"),
    )?;
    if (result.energy_hartree + 1.1167593074).abs() > 1e-6 {
        return Err("H2 energy regression".into());
    }
    if !directory.join("result.h5").is_file() {
        return Err("Missing HDF5 artifact".into());
    }
    let project = Project {
        document: Some(document),
        calculation: Some(result.clone()),
    };
    let path = directory.join("workspace.json");
    project.save(&path)?;
    if Project::load(&path)? != project {
        return Err("Project roundtrip failed".into());
    }
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
    println!("PASS: Rust/Worker calculation, HDF5, result identity, save/reload");
    std::fs::remove_dir_all(directory).map_err(|e| e.to_string())?;
    Ok(())
}
