use crate::{document::Document, jobs::JobRequest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Ensemble {
    Nve,
    Langevin,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub ensemble: Ensemble,
    pub steps: u32,
    pub timestep_fs: f64,
    pub temperature_k: f64,
    pub seed: u32,
}

impl Input {
    pub fn validate(&self, request: &JobRequest) -> Result<(), String> {
        if (1..=500).contains(&self.steps)
            && (0.05..=2.0).contains(&self.timestep_fs)
            && (1.0..=2000.0).contains(&self.temperature_k)
            && (2..=10).contains(&request.document.atoms.len())
            && matches!(request.method.as_str(), "RHF" | "UHF" | "RKS" | "UKS")
            && !request.optimize
            && request.neb.is_none()
            && request.transition.is_none()
            && request.thermochemistry.is_none()
            && request.refinement.is_none()
        {
            Ok(())
        } else {
            Err("dynamics_settings".into())
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Report {
    pub input: Input,
    pub frames: u32,
    pub ase_version: String,
    pub numpy_version: String,
    pub mean_temperature_k: f64,
    pub nve_max_energy_deviation_ev: Option<f64>,
    pub frames_sha256: String,
}

impl Report {
    pub fn valid(&self) -> bool {
        self.frames == self.input.steps.saturating_add(1)
            && (2..=501).contains(&self.frames)
            && self.mean_temperature_k.is_finite()
            && self.mean_temperature_k >= 0.0
            && match (self.input.ensemble, self.nve_max_energy_deviation_ev) {
                (Ensemble::Nve, Some(value)) => value.is_finite() && value >= 0.0,
                (Ensemble::Langevin, None) => true,
                _ => false,
            }
    }

    pub fn read_frames(&self, directory: &Path, base: &Document) -> Result<Vec<Document>, String> {
        if !self.valid() || !(2..=10).contains(&base.atoms.len()) {
            return Err("artifact_invalid".into());
        }
        read_frames(directory, base, self.frames as usize, &self.frames_sha256)
    }
}

pub(crate) fn read_frames(
    directory: &Path,
    base: &Document,
    frames: usize,
    digest: &str,
) -> Result<Vec<Document>, String> {
    let frame_size = base.atoms.len() * 24;
    let size = frames * frame_size;
    let mut bytes = Vec::with_capacity(size);
    File::open(directory.join("trajectory.bin"))
        .and_then(|file| file.take(size as u64 + 1).read_to_end(&mut bytes))
        .map_err(|_| "artifact_invalid")?;
    if bytes.len() != size || format!("{:x}", Sha256::digest(&bytes)) != digest {
        return Err("artifact_invalid".into());
    }
    bytes
        .chunks_exact(frame_size)
        .map(|frame| {
            let positions = frame
                .chunks_exact(24)
                .map(|atom| {
                    std::array::from_fn(|axis| {
                        f64::from_le_bytes(atom[axis * 8..axis * 8 + 8].try_into().unwrap())
                    })
                })
                .collect::<Vec<_>>();
            crate::neb::frame(base, &positions)
        })
        .collect()
}

struct Trajectory {
    job_id: String,
    digest: String,
    frames: Vec<Document>,
    center: glam::Vec3,
    radius: f32,
}

#[derive(Default)]
pub struct Playback(std::sync::Mutex<Option<Trajectory>>);

#[tauri::command]
pub async fn show_frame(
    app: tauri::AppHandle,
    expected_revision: u64,
    job_id: String,
    index: usize,
) -> Result<(), String> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::state::AppState>();
        let playback = app.state::<Playback>();
        let mut cached = playback.0.lock().map_err(|_| "storage")?;
        let (base, report) = {
            let workspace = state.0.lock().map_err(|_| "storage")?;
            crate::check_revision(&workspace, expected_revision)?;
            let report = workspace
                .project
                .calculation
                .as_ref()
                .filter(|result| result.job_id == job_id)
                .and_then(|result| {
                    result
                        .dynamics
                        .as_ref()
                        .map(|report| (report.frames as usize, report.frames_sha256.clone()))
                        .or_else(|| {
                            result
                                .solution_path
                                .as_ref()
                                .map(|report| (report.frame_count(), report.frames_sha256.clone()))
                        })
                })
                .ok_or("result_invalid")?;
            (
                workspace.project.document.clone().ok_or("no_document")?,
                report,
            )
        };
        if !cached
            .as_ref()
            .is_some_and(|path| path.job_id == job_id && path.digest == report.1)
        {
            let catalog = app.state::<crate::catalog::CatalogState>();
            let store = catalog.get()?;
            let restored = store.restore(&job_id)?;
            if restored.document.as_ref() != Some(&base)
                || !restored.calculation.as_ref().is_some_and(|result| {
                    result.dynamics.as_ref().is_some_and(|value| {
                        value.frames_sha256 == report.1 && value.frames as usize == report.0
                    }) || result.solution_path.as_ref().is_some_and(|value| {
                        value.frames_sha256 == report.1 && value.frame_count() == report.0
                    })
                })
            {
                return Err("artifact_invalid".into());
            }
            let frames = read_frames(&store.directory(&job_id)?, &base, report.0, &report.1)?;
            let center = crate::renderer::scene::Scene::from_document(Some(&base)).center;
            let radius = frames
                .iter()
                .flat_map(|frame| &frame.atoms)
                .map(|atom| {
                    glam::Vec3::from_array(atom.position_angstrom.map(|v| v as f32))
                        .distance(center)
                        + 0.8
                })
                .fold(0.5, f32::max);
            *cached = Some(Trajectory {
                job_id: job_id.clone(),
                digest: report.1,
                frames,
                center,
                radius,
            });
        }
        let path = cached.as_ref().ok_or("artifact_invalid")?;
        let frame = path.frames.get(index).ok_or("result_invalid")?;
        let mut workspace = state.0.lock().map_err(|_| "storage")?;
        crate::check_revision(&workspace, expected_revision)?;
        if workspace
            .project
            .calculation
            .as_ref()
            .map(|result| &result.job_id)
            != Some(&job_id)
        {
            return Err("result_invalid".into());
        }
        workspace
            .renderer
            .preview_path_image(frame, expected_revision, path.center, path.radius);
        workspace.neb_image = Some((expected_revision, job_id, frame.clone()));
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}
