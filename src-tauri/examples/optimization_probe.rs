//! Exercise the desktop queue and artifact contract with the real ASE/PySCF Worker.
use reactqm_lib::{
    catalog::{Catalog, Claim},
    document::Document,
    jobs::{JobRequest, WorkerConfig},
    scheduler::run_claimed,
    settings::CalculationSettings,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

fn main() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!("reactqm-opt-{}", uuid::Uuid::new_v4()));
    let mut job = JobRequest::new(Document::from_xyz("2\nH2\nH 0 0 0\nH 0 0 1.1", 0, 0)?, 1)?;
    let open_shell = std::env::args().any(|argument| argument == "--open-shell");
    if open_shell {
        job = JobRequest::with_settings(
            Document::from_xyz("2\nH2+\nH 0 0 0\nH 0 0 1.55", 1, 1)?,
            1,
            CalculationSettings {
                method: "UKS".into(),
                xc: Some("PBE".into()),
                grid_level: Some(3),
                ..CalculationSettings::default()
            },
        )?;
    }
    job.optimize = true;
    let store = Catalog::open(&root)?;
    store.enqueue(&job, None)?;
    store.set_queue_paused(false)?;
    let Claim::Ready { request, directory } = store.claim_next()? else {
        return Err("Missing optimization claim".into());
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let stopped = run_claimed(
        &store,
        &request,
        &directory,
        &WorkerConfig::development(),
        cancel.clone(),
        |step| {
            if step >= 1 {
                cancel.store(true, Ordering::SeqCst);
            }
        },
    );
    assert_eq!(stopped.unwrap_err(), "cancelled");
    drop(store);

    let store = Catalog::open(&root)?;
    assert!(store.page(None)?.items[0].resumable);
    let resumed = store.resume_source(&job.job_id)?.retry(2)?;
    assert_ne!(resumed.input_hash, job.input_hash);
    store.enqueue(&resumed, Some(&job.job_id))?;
    store.set_queue_paused(false)?;
    let Claim::Ready { request, directory } = store.claim_next()? else {
        return Err("Missing resumed optimization claim".into());
    };
    let result = run_claimed(
        &store,
        &request,
        &directory,
        &WorkerConfig::development(),
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )?;
    drop(store);
    let store = Catalog::open(&root)?;
    let project = store.restore(&resumed.job_id)?;
    project.save(&root.join("optimized-project.json"))?;
    let optimized = project.document.unwrap();
    let distance =
        (optimized.atoms[0].position_angstrom[2] - optimized.atoms[1].position_angstrom[2]).abs();
    if open_shell {
        assert_eq!(optimized.spin, 1);
        assert_eq!(project.settings.method, "UKS");
        assert!((0.85..1.35).contains(&distance));
        assert!((result.spin_squared.unwrap() - 0.75).abs() < 0.1);
    } else {
        assert!((distance - 0.712).abs() < 0.01);
    }
    assert!(result.optimization.unwrap().max_force_ev_per_angstrom < 0.05);
    println!("PASS: queue, cancel after progress, reopen, resume, verify and restore optimized geometry ({distance:.6} Å)");
    drop(store);
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    Ok(())
}
