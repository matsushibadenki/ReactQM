//! Two real correlated calculation jobs at fixed HCN/HNC geometries.
use reactqm_lib::{
    catalog::{Catalog, Claim},
    document::Document,
    jobs::{JobRequest, WorkerConfig},
    project::Project,
    refinement::{Baseline, Input, Method},
    scheduler::run_claimed,
    settings::CalculationSettings,
};
use std::sync::{atomic::AtomicBool, Arc};

fn main() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!("reactqm-refine-{}", uuid::Uuid::new_v4()));
    let document = Document::from_xyz("3\nHCN/HNC saddle\nC -.00847226371 0 -.00333052927\nN -.10077008431 0 1.21459495222\nH 1.10924234802 0 .43873557705", 0, 0)?;
    let baseline = Baseline {
        source_job: uuid::Uuid::new_v4().to_string(),
        path_job: uuid::Uuid::new_v4().to_string(),
        settings: CalculationSettings::default(),
        engine_version: "2.15.0".into(),
        energies_hartree: [-91.67520896744168, -91.64443723161071, -91.56485101667221],
    };
    for (method, expected) in [
        (Method::Mp2, [289.28246, 198.31931]),
        (Method::Ccsdt, [264.27673, 190.98098]),
    ] {
        let mut job = JobRequest::with_settings(
            document.clone(),
            1,
            CalculationSettings {
                basis: "6-31g".into(),
                ..CalculationSettings::default()
            },
        )?;
        job.refinement = Some(Input {
            method,
            baseline: baseline.clone(),
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
        assert_eq!(store.queue_page(None)?.items[0].refinement, Some(method));
        store.set_queue_paused(false)?;
        let Claim::Ready { request, directory } = store.claim_next()? else {
            return Err("missing refinement claim".into());
        };
        run_claimed(
            &store,
            &request,
            &directory,
            &WorkerConfig::development(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )?;
        drop(store);
        let store = Catalog::open(&root)?;
        let project = store.restore(&request.job_id)?;
        let saved = root.join("refinement-project.json");
        project.save(&saved)?;
        assert_eq!(Project::load(&saved)?, project);
        let report = project.calculation.unwrap().refinement.unwrap();
        assert_eq!(report.baseline, baseline);
        assert_eq!(report.orbital_count, 20);
        let observed = [0, 1].map(|index| {
            (report.energies_hartree[2] - report.energies_hartree[index]) * 2625.499639
        });
        for (value, expected) in observed.into_iter().zip(expected) {
            assert!((value - expected).abs() < 0.02);
        }
        assert_eq!(report.triples_hartree == [0.; 3], method == Method::Mp2);
        println!("{method:?}: forward {:.5}, reverse {:.5} kJ/mol; queue/reopen/restore and saved baseline", observed[0], observed[1]);
    }
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    Ok(())
}
