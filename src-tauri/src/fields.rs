//! Read a single verified field from the Worker and extract its surface in Rust.
use crate::{catalog::CatalogState, jobs::WorkerConfig, state::AppState};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::{sync::atomic::AtomicBool, time::Duration};
use tauri::Manager;

const SIZE: usize = 40;
const HEADER: usize = 52;
const BYTES: usize = HEADER + 4 * SIZE.pow(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Density,
    Homo,
    Lumo,
    HomoAlpha,
    LumoAlpha,
    HomoBeta,
    LumoBeta,
}

pub struct Surface {
    pub phases: [Vec<[[f32; 3]; 2]>; 2], // Position and normal per vertex, triangle list.
    pub radius: f32,
    pub density: bool,
}

fn mesh(bytes: &[u8], field: Field, center: Vec3) -> Result<Surface, String> {
    if bytes.len() != BYTES || u32::from_le_bytes(bytes[..4].try_into().unwrap()) != SIZE as u32 {
        return Err("field_invalid".into());
    }
    let numbers: Vec<f32> = bytes[4..HEADER]
        .as_chunks::<8>()
        .0
        .iter()
        .map(|&slice| f64::from_le_bytes(slice) as f32)
        .collect();
    if numbers.iter().any(|n| !n.is_finite() || n.abs() > 21000.0)
        || numbers[3..].iter().any(|&n| n <= 0.0)
    {
        return Err("field_invalid".into());
    }
    let origin = Vec3::from_slice(&numbers[..3]) - center;
    let step = Vec3::from_slice(&numbers[3..]);
    let samples: Vec<f32> = bytes[HEADER..]
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .map(f32::from_le_bytes)
        .collect();
    if samples.iter().any(|v| !v.is_finite() || v.abs() > 1e8) {
        return Err("field_invalid".into());
    }
    let mut phases = [Vec::new(), Vec::new()];
    let iso = if field == Field::Density { 0.02 } else { 0.03 };
    let corners = [
        [0, 0, 0],
        [1, 0, 0],
        [0, 1, 0],
        [1, 1, 0],
        [0, 0, 1],
        [1, 0, 1],
        [0, 1, 1],
        [1, 1, 1],
    ];
    let tetrahedra = [
        [0, 1, 3, 7],
        [0, 3, 2, 7],
        [0, 2, 6, 7],
        [0, 6, 4, 7],
        [0, 4, 5, 7],
        [0, 5, 1, 7],
    ];
    for x in 0..SIZE - 1 {
        for y in 0..SIZE - 1 {
            for z in 0..SIZE - 1 {
                let values =
                    corners.map(|[i, j, k]| samples[((x + i) * SIZE + (y + j)) * SIZE + z + k]);
                if values.iter().all(|value| value.abs() < iso) {
                    continue;
                }
                let positions = corners.map(|[i, j, k]| {
                    origin + step * Vec3::new((x + i) as f32, (y + j) as f32, (z + k) as f32)
                });
                for (phase, output) in phases
                    .iter_mut()
                    .enumerate()
                    .take(if field == Field::Density { 1 } else { 2 })
                {
                    let signed = values.map(|v| if phase == 0 { v } else { -v });
                    if signed.iter().all(|&v| v < iso) || signed.iter().all(|&v| v >= iso) {
                        continue;
                    }
                    for tetra in tetrahedra {
                        polygonize(
                            tetra.map(|i| positions[i]),
                            tetra.map(|i| signed[i]),
                            iso,
                            output,
                        );
                    }
                    if output.len() > 1_000_000 {
                        return Err("field_invalid".into());
                    }
                }
            }
        }
    }
    if phases.iter().all(Vec::is_empty) {
        return Err("field_empty".into());
    }
    let radius = phases
        .iter()
        .flatten()
        .map(|vertex| Vec3::from_array(vertex[0]).length())
        .fold(0.0, f32::max);
    Ok(Surface {
        phases,
        radius,
        density: field == Field::Density,
    })
}

fn polygonize(points: [Vec3; 4], values: [f32; 4], iso: f32, output: &mut Vec<[[f32; 3]; 2]>) {
    let mut inside = [0; 4];
    let mut outside = [0; 4];
    let (mut ni, mut no) = (0, 0);
    for (index, value) in values.into_iter().enumerate() {
        if value >= iso {
            inside[ni] = index;
            ni += 1;
        } else {
            outside[no] = index;
            no += 1;
        }
    }
    if ni == 0 || no == 0 {
        return;
    }
    let edge =
        |a: usize, b: usize| points[a].lerp(points[b], (iso - values[a]) / (values[b] - values[a]));
    let outward = points[outside[0]] - points[inside[0]];
    let mut triangle = |a: Vec3, mut b: Vec3, mut c: Vec3| {
        let mut normal = (b - a).cross(c - a);
        if normal.dot(outward) < 0.0 {
            std::mem::swap(&mut b, &mut c);
            normal = -normal;
        }
        if let Some(normal) = normal.try_normalize() {
            output.extend([a, b, c].map(|point| [point.to_array(), normal.to_array()]));
        }
    };
    match ni {
        1 => triangle(
            edge(inside[0], outside[0]),
            edge(inside[0], outside[1]),
            edge(inside[0], outside[2]),
        ),
        3 => triangle(
            edge(outside[0], inside[0]),
            edge(outside[0], inside[1]),
            edge(outside[0], inside[2]),
        ),
        _ => {
            let [a, b, c, d] = [
                edge(inside[0], outside[0]),
                edge(inside[0], outside[1]),
                edge(inside[1], outside[0]),
                edge(inside[1], outside[1]),
            ];
            triangle(a, b, c);
            triangle(b, d, c);
        }
    }
}

pub fn load(
    catalog: &crate::catalog::Catalog,
    id: &str,
    field: Field,
    config: &WorkerConfig,
) -> Result<Surface, String> {
    let (project, artifact, digest) = catalog.field_artifact(id)?;
    let result = project.calculation.ok_or("field_unavailable")?;
    if !result.fields.contains(&field) {
        return Err("field_unavailable".into());
    }
    let document = project.document.ok_or("field_unavailable")?;
    let center = crate::renderer::scene::Scene::from_document(Some(&document)).center;
    let request = serde_json::json!({"artifact": artifact, "sha256": digest, "job_id": id, "input_hash": result.input_hash, "field": field});
    let bytes = crate::valence::worker_response_bounded(
        config,
        &AtomicBool::new(false),
        Duration::from_secs(15),
        "field",
        serde_json::to_vec(&request).map_err(|_| "field_invalid")?,
        BYTES,
    )
    .map_err(|code| {
        if code == "worker_missing" {
            code
        } else {
            "field_invalid".into()
        }
    })?;
    mesh(&bytes, field, center)
}

#[tauri::command]
pub async fn set_electronic_field(
    app: tauri::AppHandle,
    revision: u64,
    job_id: String,
    field: Option<Field>,
) -> Result<(), String> {
    let ticket = {
        let state = app.state::<AppState>();
        let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
        crate::check_revision(&workspace, revision)?;
        let result = workspace
            .project
            .calculation
            .as_ref()
            .ok_or("field_unavailable")?;
        if result.job_id != job_id || field.is_some_and(|value| !result.fields.contains(&value)) {
            return Err("field_unavailable".into());
        }
        workspace.renderer.begin_field()
    };
    if let Some(field) = field {
        tauri::async_runtime::spawn_blocking(move || {
            let catalog = app.state::<CatalogState>();
            let surface = load(catalog.get()?, &job_id, field, &WorkerConfig::development())?;
            let state = app.state::<AppState>();
            let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
            crate::check_revision(&workspace, revision)?;
            if workspace
                .project
                .calculation
                .as_ref()
                .is_none_or(|result| result.job_id != job_id)
            {
                return Err("field_unavailable".into());
            }
            workspace
                .renderer
                .finish_field(ticket, revision, job_id, surface);
            Ok::<(), String>(())
        })
        .await
        .map_err(|_| "field_invalid")??;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_distinguishes_phases_and_rejects_malformed_data() {
        let mut grid = Vec::new();
        grid.extend_from_slice(&(SIZE as u32).to_le_bytes());
        for value in [0.0f64, 0.0, 0.0, 0.1, 0.1, 0.1] {
            grid.extend_from_slice(&value.to_le_bytes());
        }
        for x in 0..SIZE {
            for _ in 0..SIZE * SIZE {
                grid.extend_from_slice(&((x as f32 - 19.5) * 0.01).to_le_bytes());
            }
        }
        let surface = mesh(&grid, Field::Lumo, Vec3::ZERO).unwrap();
        for (phase, vertices) in surface.phases.iter().enumerate() {
            assert!(!vertices.is_empty());
            let x = if phase == 0 { 2.25 } else { 1.65 };
            let expected_normal = if phase == 0 { -1.0 } else { 1.0 };
            assert!(vertices.iter().all(|[position, normal]| {
                (position[0] - x).abs() < 1e-5 && (normal[0] - expected_normal).abs() < 1e-5
            }));
        }
        assert!(mesh(&grid[..grid.len() - 1], Field::Lumo, Vec3::ZERO).is_err());
    }
}
