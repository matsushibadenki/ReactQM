use reactqm_lib::{
    catalog::{Catalog, Claim},
    document::Document,
    jobs::{JobRequest, WorkerConfig},
    neb::{frame, NebInput},
    scheduler::run_claimed,
    settings::CalculationSettings,
    thermochemistry::{Medium, Solvent, SolventModel, StandardState},
    transition::{self, EndpointMatch},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

fn main() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!("reactqm-neb-{}", uuid::Uuid::new_v4()));
    let document = Document::from_xyz("3\nH + H2\nH 0 0 -1.57\nH 0 0 -.83\nH 0 0 1.57", 0, 1)?;
    for climb in [false, true] {
        let mut request = JobRequest::with_settings(
            document.clone(),
            1,
            CalculationSettings {
                method: "UHF".into(),
                ..Default::default()
            },
        )?;
        request.neb = Some(NebInput {
            climb,
            product: vec![[0., 0., -1.57], [0., 0., 0.83], [0., 0., 1.57]],
            restart: None,
        });
        let catalog = Catalog::open(&root)?;
        catalog.enqueue(&request, None)?;
        catalog.set_queue_paused(false)?;
        let Claim::Ready { request, directory } = catalog.claim_next()? else {
            return Err("missing NEB claim".into());
        };
        let config = WorkerConfig::development();
        let cancel = Arc::new(AtomicBool::new(false));
        let stopped = run_claimed(
            &catalog,
            &request,
            &directory,
            &config,
            cancel.clone(),
            |step| {
                if step >= 2 {
                    cancel.store(true, Ordering::SeqCst);
                }
            },
        );
        assert_eq!(stopped.unwrap_err(), "cancelled");
        drop(catalog);
        let catalog = Catalog::open(&root)?;
        assert!(catalog.page(None)?.items[0].resumable);
        let resume = catalog.resume_source(&request.job_id)?.retry(2)?;
        assert!(resume.neb.as_ref().unwrap().restart.is_some());
        catalog.enqueue(&resume, Some(&request.job_id))?;
        catalog.set_queue_paused(false)?;
        let Claim::Ready { request, directory } = catalog.claim_next()? else {
            return Err("missing resumed NEB claim".into());
        };
        run_claimed(
            &catalog,
            &request,
            &directory,
            &config,
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )?;
        let project = catalog.restore(&request.job_id)?;
        project.save(&root.join("path-project.json"))?;
        let path = project.calculation.unwrap().neb.unwrap();
        for coordinates in &path.positions {
            assert_eq!(frame(&document, coordinates)?.atoms.len(), 3);
        }
        let barrier = path.images[2].energy_hartree - path.images[0].energy_hartree;
        assert!((barrier - 0.035767928).abs() < 5e-5);
        println!("{}: stop/reopen/resume, five images, max projected force {:.6} eV/Å, center rise {barrier:.9} Eh",
                 if climb { "CI-NEB" } else { "NEB" }, path.max_neb_force_ev_per_angstrom);
        if climb
            && std::env::args()
                .any(|argument| matches!(argument.as_str(), "--transition" | "--thermochemistry"))
        {
            let transition = transition::from_path(&catalog, &request.job_id, 2)?;
            assert_eq!(transition.transition.as_ref().unwrap().seed_image, 2);
            catalog.enqueue(&transition, None)?;
            drop(catalog);
            let catalog = Catalog::open(&root)?;
            assert!(catalog.queue_page(None)?.items[0].transition);
            catalog.set_queue_paused(false)?;
            let Claim::Ready { request, directory } = catalog.claim_next()? else {
                return Err("missing transition claim".into());
            };
            run_claimed(
                &catalog,
                &request,
                &directory,
                &config,
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )?;
            drop(catalog);
            let catalog = Catalog::open(&root)?;
            let project = catalog.restore(&request.job_id)?;
            let saved = root.join("transition-project.json");
            project.save(&saved)?;
            assert_eq!(reactqm_lib::project::Project::load(&saved)?, project);
            let comparison =
                reactqm_lib::barriers::compare(&catalog, &[resume.job_id, request.job_id.clone()])?;
            let comparison = serde_json::to_value(comparison).map_err(|error| error.to_string())?;
            assert_eq!(comparison["comparable"], true);
            assert_eq!(comparison["rows"][0]["status"], "unconfirmed");
            assert_eq!(comparison["rows"][1]["status"], "confirmed");
            let ci_barrier = comparison["rows"][0]["forward"].as_f64().unwrap();
            let ts_barrier = comparison["rows"][1]["forward"].as_f64().unwrap();
            assert!((ci_barrier - 93.915).abs() < 0.1);
            assert!((ts_barrier - ci_barrier).abs() < 0.2);
            println!("Barriers: CI-NEB {ci_barrier:.4}, verified TS {ts_barrier:.4} kJ/mol");
            let result = project.calculation.unwrap().transition.unwrap();
            assert_eq!(result.imaginary_count, 1);
            assert!((result.frequencies_cm1[0] + 3031.3).abs() < 25.0);
            assert!(result
                .branches
                .iter()
                .any(|branch| branch.endpoint == Some(EndpointMatch::Reactant)));
            assert!(result
                .branches
                .iter()
                .any(|branch| branch.endpoint == Some(EndpointMatch::Product)));
            println!(
                "TS: imaginary {:.2} cm^-1; IRC forward {:?}, reverse {:?}",
                result.frequencies_cm1[0], result.branches[0], result.branches[1]
            );
            if std::env::args().any(|arg| arg == "--thermochemistry") {
                let source = reactqm_lib::thermochemistry::from_transition(
                    &catalog,
                    &request.job_id,
                    3,
                    350.0,
                    StandardState::Molar,
                    Some(Solvent {
                        model: SolventModel::CPcm,
                        medium: Medium::Water,
                    }),
                    false,
                )?;
                catalog.enqueue(&source, None)?;
                catalog.set_queue_paused(false)?;
                let Claim::Ready { request, directory } = catalog.claim_next()? else {
                    return Err("missing thermochemistry claim".into());
                };
                run_claimed(
                    &catalog,
                    &request,
                    &directory,
                    &config,
                    Arc::new(AtomicBool::new(false)),
                    |_| {},
                )?;
                drop(catalog);
                let catalog = Catalog::open(&root)?;
                let project = catalog.restore(&request.job_id)?;
                let saved = root.join("thermochemistry-project.json");
                project.save(&saved)?;
                assert_eq!(reactqm_lib::project::Project::load(&saved)?, project);
                let report = project.calculation.unwrap().thermochemistry.unwrap();
                assert_eq!(report.temperature_kelvin, 350.0);
                assert!(report.reactant.corrections.is_none());
                assert!(report.product.corrections.is_none());
                assert!(report.saddle.corrections.is_some());
                assert_eq!(report.standard_state, StandardState::Molar);
                assert!(report.saddle.solvated_electronic_hartree.is_some());
                println!("Thermochemistry 350 K: linked CI-NEB/IRC and save/restore verified; nonstationary H + H2 endpoints blocked");
            }
        }
    }
    std::fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    Ok(())
}
