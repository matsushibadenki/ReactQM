//! Real DFT queue → Worker → durable receipt → restart/restore/retry/project.
use reactqm_lib::{
    catalog::{Catalog, Claim},
    document::Document,
    jobs::{JobRequest, WorkerConfig},
    project::Project,
    scheduler::run_claimed,
    settings::CalculationSettings,
};
use std::sync::{atomic::AtomicBool, Arc};
fn main() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!("reactqm-dft-{}", uuid::Uuid::new_v4()));
    let settings = CalculationSettings {
        method: "RKS".into(),
        xc: Some("PBE".into()),
        basis: "6-31g".into(),
        grid_level: Some(3),
        ..Default::default()
    };
    let request = JobRequest::with_settings(
        Document::from_xyz("2\nH2 DFT\nH 0 0 0\nH 0 0 .74", 0, 0)?,
        1,
        settings.clone(),
    )?;
    let store = Catalog::open(&root)?;
    store.enqueue(&request, None)?;
    drop(store);
    let store = Catalog::open(&root)?;
    let page = store.queue_page(None)?;
    assert!(page.paused);
    assert_eq!(page.items[0].settings, Some(settings.clone()));
    store.set_queue_paused(false)?;
    let Claim::Ready { request, directory } = store.claim_next()? else {
        return Err("claim".into());
    };
    let result = run_claimed(
        &store,
        &request,
        &directory,
        &WorkerConfig::development(),
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )?;
    assert_eq!(result.settings, Some(settings.clone()));
    drop(store);
    let store = Catalog::open(&root)?;
    let project = store.restore(&request.job_id)?;
    assert_eq!(project.settings, settings);
    let path = root.join("project.json");
    project.save(&path)?;
    assert_eq!(Project::load(&path)?, project);
    let retry = store.retry_source(&request.job_id)?.retry(2)?;
    assert_eq!(retry.settings, request.settings);
    assert_eq!(store.page(None)?.items[0].settings, Some(settings));
    println!("PASS PBE/6-31G: {:.12} Eh; queue/restart, receipt, restore, retry, settings and project v2",result.energy_hartree);
    drop(store);
    std::fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    Ok(())
}
