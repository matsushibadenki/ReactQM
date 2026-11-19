//! Real PCM optimization and Hessian through the persistent queue.
use reactqm_lib::{
    catalog::{Catalog, Claim},
    document::Document,
    jobs::{JobRequest, WorkerConfig},
    scheduler::run_claimed,
    settings::CalculationSettings,
    thermochemistry::{Medium, Solvent, SolventModel},
};
use std::sync::{atomic::AtomicBool, Arc};

fn main() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!("reactqm-sol-{}", uuid::Uuid::new_v4()));
    let document = Document::from_xyz("3\nWater\nO 0 0 0\nH .76 0 .58\nH -.76 0 .58", 0, 0)?;
    for (model, medium) in [
        (SolventModel::CPcm, Medium::Water),
        (SolventModel::IefPcm, Medium::Acetonitrile),
    ] {
        let solvent = Solvent { model, medium };
        let mut job =
            JobRequest::with_settings(document.clone(), 1, CalculationSettings::default())?;
        job.solution = Some(solvent);
        let store = Catalog::open(&root)?;
        store.enqueue(&job, None)?;
        drop(store);
        let store = Catalog::open(&root)?;
        assert_eq!(store.queue_page(None)?.items[0].solution, Some(solvent));
        store.set_queue_paused(false)?;
        let Claim::Ready { request, directory } = store.claim_next()? else {
            return Err("Missing solution job".into());
        };
        let mut progress = Vec::new();
        let result = run_claimed(
            &store,
            &request,
            &directory,
            &WorkerConfig::development(),
            Arc::new(AtomicBool::new(false)),
            |step| progress.push(step),
        )?;
        drop(store);
        let store = Catalog::open(&root)?;
        let project = store.restore(&request.job_id)?;
        assert_ne!(project.document.as_ref(), Some(&document));
        assert_eq!(project.calculation.as_ref(), Some(&result));
        let report = result.solution.unwrap();
        assert!(report.minimum_confirmed);
        assert_eq!(report.frequencies_cm1.len(), 3);
        assert_eq!(progress.first(), Some(&0));
        assert_eq!(progress.last(), Some(&51));
        println!(
            "{solvent:?}: {} steps, {:.5} eV/Å, {:?} cm^-1",
            report.steps, report.max_force_ev_per_angstrom, report.frequencies_cm1
        );
    }
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())
}
