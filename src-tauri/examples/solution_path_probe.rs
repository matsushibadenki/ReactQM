//! Receipt and queue integration using fixed HCN/HNC gas reference fixtures.
use reactqm_lib::{
    catalog::{Catalog, Claim},
    document::Document,
    jobs::{JobRequest, WorkerConfig},
    refinement::Baseline,
    scheduler::run_claimed,
    settings::CalculationSettings,
    solution_path::Input,
    thermochemistry::{Medium, Solvent, SolventModel},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

fn main() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!("reactqm-pcm-path-{}", uuid::Uuid::new_v4()));
    let document = Document::from_xyz("3\nHCN/HNC saddle\nC -.00847226371 0 -.00333052927\nN -.10077008431 0 1.21459495222\nH 1.10924234802 0 .43873557705", 0, 0)?;
    let settings = CalculationSettings {
        conv_tol: 1e-10,
        ..CalculationSettings::default()
    };
    let mut job = JobRequest::with_settings(document.clone(), 1, settings.clone())?;
    let solvent = Solvent {
        model: SolventModel::CPcm,
        medium: Medium::Water,
    };
    job.solution_path = Some(Input {
        solvent,
        seed_image: 2,
        baseline: Baseline {
            source_job: uuid::Uuid::new_v4().to_string(),
            path_job: uuid::Uuid::new_v4().to_string(),
            settings,
            engine_version: "2.15.0".into(),
            energies_hartree: [-91.67520896744168, -91.64443723161071, -91.56485101667221],
        },
        reactant: vec![
            [0., 0., 0.00230468417],
            [0., 0., 1.15531220276],
            [0., 0., -1.06761688693],
        ],
        product: vec![
            [0., 0., -0.00055220749],
            [0., 0., 1.16973363427],
            [0., 0., 2.18081857322],
        ],
    });
    let store = Catalog::open(&root)?;
    store.enqueue(&job, None)?;
    drop(store);
    let store = Catalog::open(&root)?;
    assert_eq!(
        store.queue_page(None)?.items[0].solution_path,
        Some(solvent)
    );
    store.set_queue_paused(false)?;
    let Claim::Ready { request, directory } = store.claim_next()? else {
        return Err("missing claim".into());
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let result = run_claimed(
        &store,
        &request,
        &directory,
        &WorkerConfig::development(),
        cancel.clone(),
        |phase| {
            if phase == 1 {
                store.request_cancel(&request.job_id).unwrap();
                cancel.store(true, Ordering::SeqCst);
            }
        },
    );
    assert_eq!(result.unwrap_err(), "cancelled");
    assert!(store.restore(&request.job_id).is_err());
    let retry = request.retry(1)?;
    store.enqueue(&retry, Some(&request.job_id))?;
    let Claim::Ready { request, directory } = store.claim_next()? else {
        return Err("missing retry".into());
    };
    let mut phases = Vec::new();
    let result = run_claimed(
        &store,
        &request,
        &directory,
        &WorkerConfig::development(),
        Arc::new(AtomicBool::new(false)),
        |phase| {
            phases.push(phase);
            println!("phase {phase}");
        },
    )?;
    assert_eq!(phases, vec![0, 1, 2, 3, 4, 5]);
    drop(store);
    let store = Catalog::open(&root)?;
    let restored = store.restore(&request.job_id)?;
    assert_eq!(restored.calculation.as_ref(), Some(&result));
    assert_ne!(restored.document.as_ref(), Some(&document));
    let report = result.solution_path.unwrap();
    assert!(report.path_verified);
    let barriers = report
        .endpoint_energies_hartree
        .map(|e| (result.energy_hartree - e) * 2625.499639);
    assert!((barriers[0] - 290.96).abs() < 0.02 && (barriers[1] - 211.24).abs() < 0.02);
    assert!(report.lengths.iter().all(|length| *length > 1));
    println!(
        "Water barriers {barriers:?}, IRC frames {:?}",
        report.lengths
    );
    std::fs::write(directory.join("trajectory.bin"), b"damaged").map_err(|e| e.to_string())?;
    assert_eq!(
        store.restore(&request.job_id).unwrap_err(),
        "artifact_invalid"
    );
    std::fs::remove_dir_all(root).map_err(|e| e.to_string())
}
