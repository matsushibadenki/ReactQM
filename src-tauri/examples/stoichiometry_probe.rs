//! Real gas species and a first-order saddle through the saved job and queue workflow.
use reactqm_lib::{
    catalog::{Catalog, Claim},
    document::Document,
    jobs::{JobRequest, WorkerConfig},
    project::Calculation,
    scheduler::run_claimed,
    settings::CalculationSettings,
    stoichiometry::{from_history, Selection},
};
use std::sync::{atomic::AtomicBool, Arc};

fn run_next(store: &Catalog, progress: impl FnMut(u32)) -> Result<Calculation, String> {
    store.set_queue_paused(false)?;
    let Claim::Ready { request, directory } = store.claim_next()? else {
        return Err("Missing queue claim".into());
    };
    run_claimed(
        store,
        &request,
        &directory,
        &WorkerConfig::development(),
        Arc::new(AtomicBool::new(false)),
        progress,
    )
}

fn main() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!("reactqm-stoich-{}", uuid::Uuid::new_v4()));
    let store = Catalog::open(&root)?;
    let settings = CalculationSettings {
        method: "UHF".into(),
        ..Default::default()
    };
    let mut ids = Vec::new();
    for (xyz, spin) in [
        ("1\nH\nH 0 0 0", 1),
        ("2\nH2\nH 0 0 0\nH 0 0 .71222985387448", 0),
        (
            "3\nH3 saddle\nH 0 0 -.913166185108227\nH 0 0 0\nH 0 0 .913166185108227",
            1,
        ),
    ] {
        let request =
            JobRequest::with_settings(Document::from_xyz(xyz, 0, spin)?, 1, settings.clone())?;
        store.enqueue(&request, None)?;
        ids.push(run_next(&store, |_| {})?.job_id);
    }
    let term = |index: usize, coefficient| Selection {
        job_id: ids[index].clone(),
        coefficient,
    };
    let association = from_history(
        &store,
        1,
        298.15,
        false,
        vec![term(0, 2)],
        vec![term(1, 1)],
        None,
    )?;
    let exchange = from_history(
        &store,
        1,
        298.15,
        false,
        vec![term(0, 1), term(1, 1)],
        vec![term(1, 1), term(0, 1)],
        Some(ids[2].clone()),
    )?;
    assert_eq!(
        from_history(
            &store,
            1,
            298.15,
            false,
            vec![term(0, 1)],
            vec![term(1, 1)],
            None
        )
        .unwrap_err(),
        "stoichiometry_balance"
    );
    let association_id = association.job_id.clone();
    let exchange_id = exchange.job_id.clone();
    store.enqueue(&association, None)?;
    store.enqueue(&exchange, None)?;
    drop(store);
    let store = Catalog::open(&root)?;
    assert_eq!(
        store
            .queue_page(None)?
            .items
            .iter()
            .filter(|item| item.stoichiometry)
            .count(),
        2
    );
    for (request, steps) in [(association, 2), (exchange, 3)] {
        let mut events = Vec::new();
        let result = run_next(&store, |step| events.push(step))?;
        assert_eq!(events, (1..=steps).collect::<Vec<_>>());
        assert_eq!(result.job_id, request.job_id);
    }
    let check = |value: f64, expected: f64| assert!((value - expected).abs() < 0.02);
    drop(store);
    let store = Catalog::open(&root)?;
    let exchange = store
        .restore(&exchange_id)?
        .calculation
        .unwrap()
        .stoichiometry
        .unwrap();
    check(exchange.atmosphere_kj_mol.forward.unwrap(), 116.03);
    check(exchange.molar_kj_mol.forward.unwrap(), 108.11);
    check(exchange.molar_kj_mol.reverse.unwrap(), 108.11);
    let association = store
        .restore(&association_id)?
        .calculation
        .unwrap()
        .stoichiometry
        .unwrap();
    check(association.atmosphere_kj_mol.reaction, -425.21);
    check(association.molar_kj_mol.reaction, -433.14);
    println!(
        "Association: {:?} → {:?}; exchange: {:?} → {:?}",
        association.atmosphere_kj_mol,
        association.molar_kj_mol,
        exchange.atmosphere_kj_mol,
        exchange.molar_kj_mol
    );
    drop(store);
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())
}
