//! Real sequential Worker execution, persistent FIFO, queued cancel and failure isolation.
use reactqm_lib::{
    catalog::{Catalog, Claim},
    document::Document,
    jobs::{JobRequest, WorkerConfig},
    scheduler::run_claimed,
};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};
fn main() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!("reactqm-queue-probe-{}", uuid::Uuid::new_v4()));
    let request = |name: &str, distance: f64| {
        JobRequest::new(
            Document::from_xyz(&format!("2\n{name}\nH 0 0 0\nH 0 0 {distance}"), 0, 0)?,
            1,
        )
    };
    let inputs = [
        request("H2 first", 0.74)?,
        request("cancelled without launch", 0.8)?,
        request("exit", 0.74)?,
        request("H2 second", 1.0)?,
    ];
    let store = Catalog::open(&root)?;
    for input in &inputs {
        store.enqueue(input, None)?;
    }
    store.cancel_queued(&inputs[1].job_id)?;
    drop(store);
    let store = Catalog::open(&root)?;
    let page = store.queue_page(None)?;
    if !page.paused || page.waiting != 3 || !matches!(store.claim_next()?, Claim::Empty) {
        return Err("restart did not preserve paused queue".into());
    }
    store.set_queue_paused(false)?;
    let mut energies = Vec::new();
    let mut order = Vec::new();
    loop {
        match store.claim_next()? {
            Claim::Empty => break,
            Claim::Rejected { error, .. } => return Err(error),
            Claim::Ready { request, directory } => {
                if !matches!(store.claim_next()?, Claim::Empty) {
                    return Err("overlapping execution claim".into());
                }
                let config = if request.document.name == "exit" {
                    WorkerConfig {
                        python: WorkerConfig::development().python,
                        root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support"),
                    }
                } else {
                    WorkerConfig::development()
                };
                let result = run_claimed(
                    &store,
                    &request,
                    &directory,
                    &config,
                    Arc::new(AtomicBool::new(false)),
                    |_| {},
                );
                if request.document.name == "exit" {
                    if result.unwrap_err() != "worker_exit" {
                        return Err("unexpected fault classification".into());
                    }
                } else {
                    energies.push(result?.energy_hartree);
                    store.restore(&request.job_id)?;
                }
                order.push(request.job_id);
            }
        }
    }
    if order
        != vec![
            inputs[0].job_id.clone(),
            inputs[2].job_id.clone(),
            inputs[3].job_id.clone(),
        ]
        || energies.len() != 2
        || (energies[0] + 1.1167593074).abs() > 1e-6
        || (energies[0] - energies[1]).abs() < 1e-3
    {
        return Err("FIFO or engine regression".into());
    }
    if root
        .join("jobs")
        .join(&inputs[1].job_id)
        .join("worker.log")
        .exists()
    {
        return Err("cancelled waiting job was launched".into());
    }
    println!("PASS: persisted FIFO, restart pause/resume, queued cancellation without spawn, real H2 calculations, no overlapping claims and continuation after Worker failure; energies {energies:?}");
    drop(store);
    std::fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    Ok(())
}
