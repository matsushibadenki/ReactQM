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
    let screening = std::env::args()
        .skip(1)
        .any(|argument| argument == "--screening");
    let root = std::env::temp_dir().join(format!("reactqm-dft-{}", uuid::Uuid::new_v4()));
    let settings = if screening {
        CalculationSettings {
            method: "GFN2-xTB".into(),
            basis: "gfn2".into(),
            max_cycle: 250,
            conv_tol: 0.0,
            ..Default::default()
        }
    } else {
        CalculationSettings {
            method: "RKS".into(),
            xc: Some("PBE".into()),
            basis: "6-31g".into(),
            grid_level: Some(3),
            ..Default::default()
        }
    };
    let xyz = if screening {
        "2\nHCl screening\nH 0 0 0\nCl 0 0 1.27"
    } else {
        "2\nH2 DFT\nH 0 0 0\nH 0 0 .74"
    };
    let request = JobRequest::with_settings(Document::from_xyz(xyz, 0, 0)?, 1, settings.clone())?;
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
    let label = if screening {
        assert!((result.energy_hartree - (-5.052792620592)).abs() < 1e-7);
        "GFN2-xTB (HCl)"
    } else {
        "PBE/6-31G"
    };
    println!(
        "PASS {label}: {:.12} Eh; queue/restart, receipt, restore, retry, settings and project v2",
        result.energy_hartree
    );
    if screening {
        use reactqm_lib::xtb_followup::{from_history, Decision};
        let outside = from_history(&store, &result.job_id, 1, 1)?;
        assert_eq!(
            outside.xtb_followup.as_ref().unwrap().decision,
            Decision::OutOfScope
        );
        let mut followups = vec![outside];
        store.set_queue_paused(false)?;
        for (distance, expected) in [
            (0.74, Decision::Dft),
            (0.7766987531713656, Decision::BelowThreshold),
        ] {
            let xyz = format!("2\nH2 screening\nH 0 0 0\nH 0 0 {distance}");
            let source = JobRequest::with_settings(
                Document::from_xyz(&xyz, 0, 0)?,
                1,
                request.effective_settings(),
            )?;
            store.enqueue(&source, None)?;
            let Claim::Ready { request, directory } = store.claim_next()? else {
                return Err("Missing xTB job".into());
            };
            let source = run_claimed(
                &store,
                &request,
                &directory,
                &WorkerConfig::development(),
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )?;
            let followup = from_history(&store, &source.job_id, 1, 1)?;
            assert_eq!(followup.xtb_followup.as_ref().unwrap().decision, expected);
            followups.push(followup);
            if expected == Decision::Dft {
                let stop = from_history(&store, &source.job_id, 0, 1)?;
                assert_eq!(
                    stop.xtb_followup.as_ref().unwrap().decision,
                    Decision::NoBudget
                );
                followups.push(stop);
            }
        }
        for followup in &followups {
            store.enqueue(followup, None)?;
        }
        drop(store);
        let store = Catalog::open(&root)?;
        store.set_queue_paused(false)?;
        for expected in followups {
            let Claim::Ready { request, directory } = store.claim_next()? else {
                return Err("Missing xTB follow-up".into());
            };
            assert_eq!(expected.job_id, request.job_id);
            let mut progress = Vec::new();
            let result = run_claimed(
                &store,
                &request,
                &directory,
                &WorkerConfig::development(),
                Arc::new(AtomicBool::new(false)),
                |step| progress.push(step),
            )?;
            assert_eq!(
                store.restore(&result.job_id)?.calculation.as_ref(),
                Some(&result)
            );
            let report = result.xtb_followup.unwrap();
            assert_eq!(
                report.policy_v1.decision == Decision::Dft,
                !progress.is_empty()
            );
            println!(
                "{:?}: xTB force {:.6}, DFT force {:?} eV/Å; {:.10} Eh",
                report.policy_v1.decision,
                report.policy_v1.max_force_ev_per_angstrom,
                report.dft_max_force_ev_per_angstrom,
                result.energy_hartree
            );
        }
    } else {
        drop(store);
    }
    std::fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    Ok(())
}
