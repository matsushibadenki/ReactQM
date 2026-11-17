//! Exercise the real thermochemistry Worker and receipt-backed Eyring comparison.
use reactqm_lib::{
    catalog::{Catalog, Claim},
    document::Document,
    jobs::{JobRequest, WorkerConfig},
    rates,
    scheduler::run_claimed,
    thermochemistry::{Input, Medium, Solvent, SolventModel, StandardState},
};
use std::sync::{atomic::AtomicBool, Arc};

fn main() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!("reactqm-rate-{}", uuid::Uuid::new_v4()));
    let store = Catalog::open(&root)?;
    let document = Document::from_xyz(
        "3\nHCN / HNC saddle\nC -.00847226371 0 -.00333052927\nN -.10077008431 0 1.21459495222\nH 1.10924234802 0 .43873557705",
        0, 0,
    )?;
    let fixture_id = uuid::Uuid::new_v4().to_string();
    let mut ids = Vec::new();
    for (temperature_kelvin, standard_state, solvent) in [
        (298.15, StandardState::Atmosphere, None),
        (
            298.15,
            StandardState::Molar,
            Some(Solvent {
                model: SolventModel::CPcm,
                medium: Medium::Water,
            }),
        ),
        (500.0, StandardState::Atmosphere, None),
    ] {
        let mut request = JobRequest::new(document.clone(), 1)?;
        request.thermochemistry = Some(Input {
            source_job: fixture_id.clone(),
            temperature_kelvin,
            standard_state,
            solvent,
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
        store.enqueue(&request, None)?;
        store.set_queue_paused(false)?;
        let Claim::Ready { request, directory } = store.claim_next()? else {
            return Err("missing rate fixture claim".into());
        };
        run_claimed(
            &store,
            &request,
            &directory,
            &WorkerConfig::development(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )?;
        ids.push(request.job_id);
    }
    drop(store);
    let store = Catalog::open(&root)?;
    let snapshot = |ids: &[String]| -> Result<serde_json::Value, String> {
        serde_json::to_value(rates::compare(&store, ids)?).map_err(|error| error.to_string())
    };
    let solution = snapshot(&ids[..2])?;
    let temperature = snapshot(&[ids[0].clone(), ids[2].clone()])?;
    assert_eq!(solution["comparable"], true);
    assert_eq!(temperature["comparable"], true);
    let k = |row: &serde_json::Value, environment: &str, direction: &str| {
        10f64.powf(
            row[environment][direction]["log10PerSecond"]
                .as_f64()
                .unwrap(),
        )
    };
    for (row, env, forward, reverse) in [
        (&solution["rows"][0], "gas", 1.46027e-34, 9.94433e-21),
        (&solution["rows"][1], "gas", 1.46027e-34, 9.94433e-21),
        (&solution["rows"][1], "solution", 8.85165e-35, 4.02700e-21),
        (&temperature["rows"][1], "gas", 3.96220e-15, 6.25492e-7),
    ] {
        let observed = [k(row, env, "forward"), k(row, env, "reverse")];
        assert!((observed[0] / forward - 1.0).abs() < 0.01);
        assert!((observed[1] / reverse - 1.0).abs() < 0.01);
        println!(
            "{} K, {}, {env}: {:.4e} / {:.4e} s^-1",
            row["temperatureKelvin"], row["standardState"], observed[0], observed[1]
        );
    }
    drop(store);
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    Ok(())
}
