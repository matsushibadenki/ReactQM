//! Real PySCF dynamics through the persistent desktop queue.
use reactqm_lib::{
    catalog::{Catalog, Claim},
    document::Document,
    dynamics::{Ensemble, Input},
    jobs::{JobRequest, WorkerConfig},
    scheduler::run_claimed,
    settings::CalculationSettings,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

fn main() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!("reactqm-md-{}", uuid::Uuid::new_v4()));
    let document = Document::from_xyz("2\nHydrogen\nH 0 0 0\nH 0 0 0.74", 0, 0)?;
    for (ensemble, abort) in [
        (Ensemble::Nve, false),
        (Ensemble::Langevin, false),
        (Ensemble::Nve, true),
    ] {
        let mut job =
            JobRequest::with_settings(document.clone(), 1, CalculationSettings::default())?;
        job.dynamics = Some(Input {
            ensemble,
            steps: if abort { 500 } else { 12 },
            timestep_fs: 0.25,
            temperature_k: 300.0,
            seed: 42,
        });
        let store = Catalog::open(&root)?;
        store.enqueue(&job, None)?;
        drop(store);
        let store = Catalog::open(&root)?;
        assert_eq!(store.queue_page(None)?.items[0].dynamics, job.dynamics);
        store.set_queue_paused(false)?;
        let Claim::Ready { request, directory } = store.claim_next()? else {
            return Err("missing dynamics claim".into());
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let mut progress = Vec::new();
        let result = run_claimed(
            &store,
            &request,
            &directory,
            &WorkerConfig::development(),
            cancel.clone(),
            |step| {
                progress.push(step);
                if abort && step == 1 {
                    store.request_cancel(&request.job_id).unwrap();
                    cancel.store(true, Ordering::SeqCst);
                }
            },
        );
        drop(store);
        let store = Catalog::open(&root)?;
        if abort {
            assert_eq!(result.unwrap_err(), "cancelled");
            assert!(store.restore(&job.job_id).is_err());
            println!("Cancelled real worker after frame 1");
        } else {
            let result = result?;
            assert_eq!(progress, (0..=12).collect::<Vec<_>>());
            let restored = store.restore(&job.job_id)?;
            assert_eq!(restored.calculation.as_ref(), Some(&result));
            let report = result.dynamics.unwrap();
            let frames = report.read_frames(&directory, restored.document.as_ref().unwrap())?;
            assert_eq!(frames.len(), 13);
            assert_ne!(frames[0], frames[12]);
            println!(
                "{ensemble:?}: {} frames, {:.2} K, NVE max deviation {:?} eV",
                frames.len(),
                report.mean_temperature_k,
                report.nve_max_energy_deviation_ev
            );
            std::fs::write(directory.join("trajectory.bin"), b"damaged")
                .map_err(|e| e.to_string())?;
            assert_eq!(store.restore(&job.job_id).unwrap_err(), "artifact_invalid");
        }
    }
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())
}
