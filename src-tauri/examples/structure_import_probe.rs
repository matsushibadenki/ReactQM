//! Real SMILES -> seeded RDKit -> Rust Document -> PySCF -> durable restore.
use reactqm_lib::{
    catalog::Catalog,
    jobs::{execute, JobRequest, WorkerConfig},
    project::Project,
    structure::generate,
};
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
fn main() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!("reactqm-import-probe-{}", uuid::Uuid::new_v4()));
    let config = WorkerConfig::development();
    let document = generate(
        "smiles",
        "O",
        42,
        &config,
        &AtomicBool::new(false),
        Duration::from_secs(20),
    )?;
    let repeated = generate(
        "smiles",
        "O",
        42,
        &config,
        &AtomicBool::new(false),
        Duration::from_secs(20),
    )?;
    if document != repeated || document.formula() != "H2O" {
        return Err("structure reproducibility mismatch".into());
    }
    let analysis = reactqm_lib::valence::analyze(
        &document,
        &config,
        &AtomicBool::new(false),
        Duration::from_secs(10),
    )?;
    if analysis
        .display_bonds()
        .ok_or("missing source bonds")?
        .len()
        != 2
    {
        return Err("source graph mismatch".into());
    }
    let request = JobRequest::new(document.clone(), 1)?;
    let store = Catalog::open(&root)?;
    let directory = store.begin(&request, None)?;
    let calculation = execute(
        &request,
        &directory,
        &config,
        Arc::new(AtomicBool::new(false)),
        Duration::from_secs(120),
        |_| {},
    )?;
    if !calculation.converged || !(-76.0..-74.0).contains(&calculation.energy_hartree) {
        return Err("water calculation regression".into());
    }
    store.complete(&request, &calculation)?;
    drop(store);
    let store = Catalog::open(&root)?;
    let project = store.restore(&request.job_id)?;
    if project.document.as_ref() != Some(&document)
        || project.calculation.as_ref() != Some(&calculation)
    {
        return Err("source/result restore mismatch".into());
    }
    let path = root.join("project.json");
    project.save(&path)?;
    if Project::load(&path)? != project {
        return Err("project provenance mismatch".into());
    }
    if store.retry_source(&request.job_id)?.document != document {
        return Err("retry provenance mismatch".into());
    }
    println!("PASS: seeded SMILES water reproducibility, source graph, real PySCF {} Eh, catalog reopen, checksum restore, project save/load, retry provenance", calculation.energy_hartree);
    drop(store);
    std::fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    Ok(())
}
