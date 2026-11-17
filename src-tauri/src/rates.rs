//! Canonical transition-state-theory estimates from verified thermochemistry.
use crate::{
    catalog::{Catalog, CatalogState},
    settings::CalculationSettings,
    thermochemistry::{Report, Solvent, Species, StandardState},
};
use serde::Serialize;
use tauri::{AppHandle, Manager};

const BOLTZMANN: f64 = 1.380649e-23;
const PLANCK: f64 = 6.62607015e-34;
const GAS_CONSTANT: f64 = 8.31446261815324;
const EH_TO_KJ_MOL: f64 = 2625.499639;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    pub barrier_kj_mol: f64,
    pub log10_per_second: Option<f64>,
}

#[derive(Serialize)]
pub struct Pair {
    pub forward: Option<Estimate>,
    pub reverse: Option<Estimate>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conditions {
    id: String,
    source: String,
    settings: CalculationSettings,
    engine_version: String,
    temperature_kelvin: f64,
    standard_state: StandardState,
    solvent: Option<Solvent>,
    gas: Pair,
    solution: Option<Pair>,
}

#[derive(Serialize)]
pub struct Comparison {
    rows: Vec<Conditions>,
    comparable: Option<bool>,
}

fn gibbs(species: &Species, solution: bool) -> Option<f64> {
    let gas = species.corrections.as_ref()?.gibbs_hartree;
    if solution {
        Some(gas + (species.solvated_electronic_hartree? - species.electronic_hartree))
    } else {
        Some(gas)
    }
}

fn estimate(saddle: &Species, endpoint: &Species, t: f64, solution: bool) -> Option<Estimate> {
    let barrier = (gibbs(saddle, solution)? - gibbs(endpoint, solution)?) * EH_TO_KJ_MOL;
    let logarithm = ((BOLTZMANN * t / PLANCK).ln() - barrier * 1000.0 / (GAS_CONSTANT * t))
        / std::f64::consts::LN_10;
    barrier.is_finite().then_some(Estimate {
        barrier_kj_mol: barrier,
        log10_per_second: (barrier > 0.0 && logarithm.is_finite()).then_some(logarithm),
    })
}

pub fn rates(report: &Report, solution: bool) -> Pair {
    let t = report.temperature_kelvin;
    Pair {
        forward: estimate(&report.saddle, &report.reactant, t, solution),
        reverse: estimate(&report.saddle, &report.product, t, solution),
    }
}

pub fn compare(store: &Catalog, ids: &[String]) -> Result<Comparison, String> {
    if !(1..=2).contains(&ids.len()) || (ids.len() == 2 && ids[0] == ids[1]) {
        return Err("rate_source".into());
    }
    let rows = ids
        .iter()
        .map(|id| {
            let project = store.restore(id)?;
            let calculation = project.calculation.ok_or("rate_source")?;
            let report = calculation.thermochemistry.ok_or("rate_source")?;
            Ok::<_, String>(Conditions {
                id: id.clone(),
                source: report.source_job.clone(),
                settings: project.settings,
                engine_version: calculation.engine_version,
                temperature_kelvin: report.temperature_kelvin,
                standard_state: report.standard_state,
                solvent: report.solvent,
                gas: rates(&report, false),
                solution: report.solvent.map(|_| rates(&report, true)),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let comparable = match rows.as_slice() {
        [a, b] => Some(
            a.source == b.source
                && a.settings == b.settings
                && a.engine_version == b.engine_version,
        ),
        _ => None,
    };
    Ok(Comparison { rows, comparable })
}

#[tauri::command]
pub async fn compare_rates(app: AppHandle, ids: Vec<String>) -> Result<Comparison, String> {
    tauri::async_runtime::spawn_blocking(move || compare(app.state::<CatalogState>().get()?, &ids))
        .await
        .map_err(|_| "rate_source")?
}
