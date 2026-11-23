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
    if diagnostics.disagreement.is_some() {
        let compressed = water.replace("H .8 ", "H .54 ");
        let strong = JobRequest::mace(Document::from_xyz(&compressed, 0, 0)?, 1)?;
        store.enqueue(&strong, None)?;
        store.set_queue_paused(false)?;
        let Claim::Ready { request, directory } = store.claim_next()? else {
            return Err("Missing second MACE job".into());
        };
        run_claimed(
            &store,
            &request,
            &directory,
            &WorkerConfig::development(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )?;
        let candidates = vec![
            result.job_id.clone(),
            strong.job_id.clone(),
            strong.job_id.clone(),
        ];
        let mild_only = reactqm_lib::mace_reference::from_history(
            &store,
            vec![result.job_id.clone()],
            1,
            1,
            true,
        )?;
        assert!(mild_only.is_empty());
        let jobs = reactqm_lib::mace_reference::from_history(&store, candidates, 1, 1, true)?;
        assert_eq!(jobs.len(), 1);
        assert_eq!(
            jobs[0].mace_reference.as_ref().unwrap().source_job,
            strong.job_id
        );
        store.enqueue_reference_batch(&jobs)?;
        drop(store);
        let store = Catalog::open(&root)?;
        assert_eq!(
            store.queue_page(None)?.items[0]
                .mace_reference
                .as_ref()
                .unwrap()
                .rank,
            1
        );
        store.set_queue_paused(false)?;
        let Claim::Ready { request, directory } = store.claim_next()? else {
            return Err("Missing reference job".into());
        };
        let reference = run_claimed(
            &store,
            &request,
            &directory,
            &WorkerConfig::development(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )?;
        let restored = store.restore(&reference.job_id)?;
        assert_eq!(restored.calculation.as_ref(), Some(&reference));
        let report = reference.mace_reference.unwrap();
        assert_eq!(report.selection.source_job, strong.job_id);
        assert!(report.selection.warning_v1.is_some());
        assert!((report.forces_ev_per_angstrom[1][0] - 12.66660494).abs() < 1e-4);
        println!(
            "Reference rank {}, disagreement {:.6} eV/Å; forces {:?}",
            report.selection.rank, report.selection.max_disagreement, report.forces_ev_per_angstrom
        );
        use reactqm_lib::escalation::{from_history, Decision};
        for (budget, expected) in [
            (0, Decision::NoBudget),
            (1, Decision::Mp2),
            (3, Decision::Ccsdt),
        ] {
            let job = from_history(&store, &reference.job_id, budget, 1)?;
            assert_eq!(job.escalation.as_ref().unwrap().decision, expected);
            store.enqueue(&job, None)?;
        }
        drop(store);
        let store = Catalog::open(&root)?;
        store.set_queue_paused(false)?;
        let mut energies = Vec::new();
        for expected in [Decision::NoBudget, Decision::Mp2, Decision::Ccsdt] {
            let Claim::Ready { request, directory } = store.claim_next()? else {
                return Err("Missing escalation job".into());
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
            assert_eq!(
                store.restore(&result.job_id)?.calculation.as_ref(),
                Some(&result)
            );
            let policy = result.escalation.unwrap();
            assert_eq!(policy.policy_v1.decision, expected);
            assert_eq!(progress.is_empty(), expected == Decision::NoBudget);
            assert_eq!(policy.rhf_hartree.is_none(), expected == Decision::NoBudget);
            assert!((policy.policy_v1.max_force_difference - 3.93222363).abs() < 1e-4);
            energies.push(result.energy_hartree);
            println!(
                "{expected:?}: {:.10} Eh; gap {:.6} eV/Å",
                result.energy_hartree, policy.policy_v1.max_force_difference
            );
        }
        assert_ne!(energies[1], energies[2]);
    } else {
        drop(store);
    }
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())
}
