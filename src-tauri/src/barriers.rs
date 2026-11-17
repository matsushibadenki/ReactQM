//! Electronic activation energies derived from verified job artifacts.
use crate::{
    catalog::{Catalog, CatalogState},
    document::Document,
    settings::CalculationSettings,
    transition::EndpointMatch,
};
use serde::Serialize;
use tauri::{AppHandle, Manager};

const HARTREE_TO_KJ_MOL: f64 = 2625.499639;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Barrier {
    id: String,
    source_id: String,
    kind: &'static str,
    status: &'static str,
    settings: CalculationSettings,
    engine_version: String,
    forward: f64,
    reverse: f64,
    reaction_energy: f64,
    peak_image: usize,
    profile: Vec<f64>,
}

#[derive(Serialize)]
pub struct Comparison {
    rows: Vec<Barrier>,
    comparable: Option<bool>,
}

type Endpoints = (Vec<u8>, i32, u32, Vec<[f64; 3]>, Vec<[f64; 3]>);

fn endpoint_identity(doc: &Document, positions: &[Vec<[f64; 3]>]) -> Endpoints {
    (
        doc.atoms.iter().map(|atom| atom.atomic_number).collect(),
        doc.charge,
        doc.spin,
        positions[0].clone(),
        positions[4].clone(),
    )
}

fn difference(a: f64, b: f64) -> Result<f64, String> {
    let value = (a - b) * HARTREE_TO_KJ_MOL;
    value
        .is_finite()
        .then_some(value)
        .ok_or_else(|| "barrier_source".into())
}

fn summarize(store: &Catalog, id: &str) -> Result<(Barrier, Endpoints), String> {
    let project = store.restore(id)?;
    let result = project.calculation.as_ref().ok_or("barrier_source")?;
    let transition = result.transition.as_ref();
    let source_id = transition.map_or(id, |ts| ts.source_job.as_str());
    let source = transition.map(|_| store.restore(source_id)).transpose()?;
    let source = source.as_ref().unwrap_or(&project);
    let path_result = source.calculation.as_ref().ok_or("barrier_source")?;
    let path = path_result.neb.as_ref().ok_or("barrier_source")?;
    let document = source.document.as_ref().ok_or("barrier_source")?;
    let identity = endpoint_identity(document, &path.positions);
    let (peak, maximum) = path
        .images
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.energy_hartree.total_cmp(&b.energy_hartree))
        .ok_or("barrier_source")?;
    let (kind, status, energy) = if let Some(ts) = transition {
        if !path.climb || ts.seed_image != peak || project.settings != source.settings {
            return Err("barrier_source".into());
        }
        let matched = |endpoint| {
            ts.branches
                .iter()
                .any(|branch| branch.converged && branch.endpoint == Some(endpoint))
        };
        let confirmed = ts.imaginary_count == 1
            && matched(EndpointMatch::Reactant)
            && matched(EndpointMatch::Product);
        (
            "transition",
            if confirmed {
                "confirmed"
            } else {
                "unconfirmed"
            },
            result.energy_hartree,
        )
    } else {
        (
            if path.climb { "ci_neb" } else { "neb" },
            if peak == 0 || peak == 4 {
                "endpoint_peak"
            } else {
                "unconfirmed"
            },
            maximum.energy_hartree,
        )
    };
    let start = path.images[0].energy_hartree;
    let end = path.images[4].energy_hartree;
    let profile = path
        .images
        .iter()
        .map(|image| difference(image.energy_hartree, start))
        .collect::<Result<Vec<_>, _>>()?;
    let comparable_version = if result.engine_version == path_result.engine_version {
        result.engine_version.clone()
    } else {
        format!("{} / {}", path_result.engine_version, result.engine_version)
    };
    Ok((
        Barrier {
            id: id.into(),
            source_id: source_id.into(),
            kind,
            status,
            settings: project.settings,
            engine_version: comparable_version,
            forward: difference(energy, start)?,
            reverse: difference(energy, end)?,
            reaction_energy: difference(end, start)?,
            peak_image: peak + 1,
            profile,
        },
        identity,
    ))
}

pub fn compare(store: &Catalog, ids: &[String]) -> Result<Comparison, String> {
    if !(1..=2).contains(&ids.len()) || (ids.len() == 2 && ids[0] == ids[1]) {
        return Err("barrier_source".into());
    }
    let rows = ids
        .iter()
        .map(|id| summarize(store, id))
        .collect::<Result<Vec<_>, _>>()?;
    let comparable = match rows.as_slice() {
        [(a, a_endpoints), (b, b_endpoints)] => Some(
            a_endpoints == b_endpoints
                && a.settings == b.settings
                && a.engine_version == b.engine_version
                && !a.engine_version.contains(" / "),
        ),
        _ => None,
    };
    Ok(Comparison {
        rows: rows.into_iter().map(|(row, _)| row).collect(),
        comparable,
    })
}

#[tauri::command]
pub async fn compare_barriers(app: AppHandle, ids: Vec<String>) -> Result<Comparison, String> {
    tauri::async_runtime::spawn_blocking(move || compare(app.state::<CatalogState>().get()?, &ids))
        .await
        .map_err(|_| "barrier_source")?
}
