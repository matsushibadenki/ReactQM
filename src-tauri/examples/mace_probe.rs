//! Validate the official MACE model through the real persistent job queue.
use reactqm_lib::{
    catalog::{Catalog, Claim},
    document::Document,
    jobs::{JobRequest, WorkerConfig},
    scheduler::run_claimed,
};
use std::sync::{atomic::AtomicBool, Arc};

fn main() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!("reactqm-mace-{}", uuid::Uuid::new_v4()));
    let water = "3\nDistorted water\nO 0 0 0\nH .8 0 .58\nH -.76 0 .58";
    assert_eq!(
        JobRequest::mace(Document::from_xyz(water, 1, 1)?, 1).unwrap_err(),
        "mace_requires_neutral"
    );
    assert_eq!(
        JobRequest::mace(Document::from_xyz(water, 0, 2)?, 1).unwrap_err(),
        "mace_requires_singlet"
    );
    let disconnected = "3\nDistorted water\nO 0 0 0\nH 8 0 .58\nH -.76 0 .58";
    assert_eq!(
        JobRequest::mace(Document::from_xyz(disconnected, 0, 0)?, 1).unwrap_err(),
        "mace_geometry_disconnected"
    );
    let doc = Document::from_xyz(water, 0, 0)?;
    let request = JobRequest::mace(doc.clone(), 1)?;
    let store = Catalog::open(&root)?;
    store.enqueue(&request, None)?;
    drop(store);
    let store = Catalog::open(&root)?;
    assert!(store.queue_page(None)?.items[0].mace);
    store.set_queue_paused(false)?;
    let Claim::Ready { request, directory } = store.claim_next()? else {
        return Err("Missing MACE job".into());
    };
    let mut progress = Vec::new();
    let result = run_claimed(
        &store,
        &request,
        &directory,
        &WorkerConfig::development(),
        Arc::new(AtomicBool::new(false)),
        |cycle| progress.push(cycle),
    )?;
    assert_eq!(progress, [1]);
    drop(store);
    let store = Catalog::open(&root)?;
    let restored = store.restore(&result.job_id)?;
    assert_eq!(restored.document, Some(doc));
    assert_eq!(restored.calculation.as_ref(), Some(&result));
    let report = result.mace.unwrap();
    let diagnostics = report
        .diagnostics
        .as_ref()
        .ok_or("Missing extrapolation diagnostics")?;
    assert_eq!(
        diagnostics.disagreement.is_some(),
        std::env::var_os("REACTQM_MACE_MEDIUM_MODEL").is_some()
    );
    assert!((report.energy_ev + 2081.09634372).abs() < 1e-5);
    assert_eq!(report.forces_ev_per_angstrom.len(), 3);
    assert!((report.forces_ev_per_angstrom[1][0] + 1.27834468).abs() < 1e-5);
    println!(
        "Energy {:.8} eV; forces {:?} eV/Å",
        report.energy_ev, report.forces_ev_per_angstrom
    );
    drop(store);
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())
}
