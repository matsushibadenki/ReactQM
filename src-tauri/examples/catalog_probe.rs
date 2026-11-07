//! Real engine → durable catalog → restart → checksum-verified restore → new attempt.
use reactqm_lib::{
    catalog::Catalog,
    document::Document,
    jobs::{execute, JobRequest, WorkerConfig},
};
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
fn main() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!("reactqm-catalog-probe-{}", uuid::Uuid::new_v4()));
    let store = Catalog::open(&root)?;
    let request = JobRequest::new(Document::from_xyz("2\nH2\nH 0 0 0\nH 0 0 .74", 0, 0)?, 1)?;
    let directory = store.begin(&request, None)?;
    let result = execute(
        &request,
        &directory,
        &WorkerConfig::development(),
        Arc::new(AtomicBool::new(false)),
        Duration::from_secs(120),
        |cycle| println!("SCF cycle {cycle}"),
    )?;
    store.complete(&request, &result)?;
    drop(store);
    let store = Catalog::open(&root)?;
    let project = store.restore(&request.job_id)?;
    if project.calculation.as_ref() != Some(&result)
        || (result.energy_hartree + 1.1167593074).abs() > 1e-6
    {
        return Err("catalog result mismatch".into());
    }
    let retry = store.retry_source(&request.job_id)?.retry(2)?;
    let retry_directory = store.begin(&retry, Some(&request.job_id))?;
    let next = execute(
        &retry,
        &retry_directory,
        &WorkerConfig::development(),
        Arc::new(AtomicBool::new(false)),
        Duration::from_secs(120),
        |_| {},
    )?;
    store.complete(&retry, &next)?;
    if (next.energy_hartree - result.energy_hartree).abs() > 1e-10
        || store.page(None)?.items.len() != 2
    {
        return Err("retry regression".into());
    }
    std::fs::write(directory.join("result.h5"), b"damaged").map_err(|e| e.to_string())?;
    if store.restore(&request.job_id).is_ok() || store.restore(&retry.job_id).is_err() {
        return Err("corruption isolation failed".into());
    }
    println!("PASS: real H2, durable SQLite history, reopen, verified restore, repeat attempt and corruption isolation; energy {} Eh",result.energy_hartree);
    drop(store);
    std::fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    Ok(())
}
