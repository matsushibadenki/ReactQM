//! Rust owns molecular geometry, camera, picking and GPU resources.
mod elements;
pub mod gpu;
#[cfg(target_os = "macos")]
mod native;
pub mod scene;
use crate::document::Document;
pub use gpu::GpuContext;
use scene::{Camera, Scene};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Deserialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub window_width: f64,
    pub window_height: f64,
    #[serde(default)]
    pub clip_top: f64,
    pub visible: bool,
    pub dark: bool,
}
impl Bounds {
    pub fn validate(&self) -> Result<(), String> {
        if [
            self.x,
            self.y,
            self.width,
            self.height,
            self.window_width,
            self.window_height,
            self.clip_top,
        ]
        .iter()
        .any(|v| !v.is_finite() || v.abs() > 100_000.0)
            || self.width < 0.0
            || self.height < 0.0
            || self.window_width < 0.0
            || self.window_height < 0.0
            || self.clip_top < 0.0
        {
            return Err("viewport_bounds".into());
        }
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    Orbit { dx: f32, dy: f32 },
    Zoom { delta: f32 },
    Fit,
    Pick { x: f32, y: f32 },
    Bonds,
    SelectNext,
}
#[derive(Deserialize)]
pub struct ViewportRequest {
    pub client: String,
    pub attach: bool,
    pub bounds: Bounds,
    pub actions: Vec<Action>,
    pub document_revision: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewportSnapshot {
    pub ready: bool,
    pub document_revision: u64,
    pub selected: Option<Selection>,
    pub show_bonds: bool,
    pub bond_count: usize,
    pub bond_source: String,
    pub bond_status: String,
    pub bond_engine_version: Option<String>,
    pub double_bond_count: usize,
    pub triple_bond_count: usize,
    pub pixel_size: [u32; 2],
    pub frame_count: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub index: usize,
    pub symbol: String,
    pub position: [f64; 3],
    pub inferred_neighbors: usize,
    pub bonds: Vec<(usize, u8)>,
}

pub struct Renderer {
    // Drop the surface before its device and native layer attachment.
    #[cfg(target_os = "macos")]
    viewport: Option<native::Viewport>,
    pub context: Option<GpuContext>,
    scene: Scene,
    bond_analysis: Option<(String, Option<String>)>,
    camera: Camera,
    selected: Option<usize>,
    show_bonds: bool,
    document_revision: Option<u64>,
    client: String,
    frames: u64,
    field_ticket: u64,
    field: Option<(u64, String, crate::fields::Surface)>,
    molecule_radius: f32,
}
impl Default for Renderer {
    fn default() -> Self {
        Self {
            #[cfg(target_os = "macos")]
            viewport: None,
            context: None,
            scene: Scene::default(),
            bond_analysis: None,
            camera: Camera::default(),
            selected: None,
            show_bonds: true,
            document_revision: None,
            client: String::new(),
            frames: 0,
            field_ticket: 0,
            field: None,
            molecule_radius: 0.0,
        }
    }
}
impl Renderer {
    pub(crate) fn invalidate_scene(&mut self) {
        self.document_revision = None;
    }
    pub(crate) fn preview_path_image(
        &mut self,
        document: &Document,
        revision: u64,
        center: glam::Vec3,
        radius: f32,
    ) {
        self.scene = Scene::from_document(Some(document));
        self.scene.center = center;
        self.scene.radius = radius;
        self.molecule_radius = radius;
        self.document_revision = Some(revision);
        self.selected = None;
        self.bond_analysis = None;
    }
    fn sync_scene(&mut self, document: Option<&Document>, revision: u64) {
        if self.document_revision != Some(revision) {
            self.scene = Scene::from_document(document);
            self.molecule_radius = self.scene.radius;
            self.camera = Camera::default();
            self.selected = None;
            self.bond_analysis = None;
            self.document_revision = Some(revision);
        }
    }
    pub(crate) fn begin_field(&mut self) -> u64 {
        self.field_ticket += 1;
        self.field = None;
        self.field_ticket
    }

    pub(crate) fn finish_field(
        &mut self,
        ticket: u64,
        revision: u64,
        job_id: String,
        surface: crate::fields::Surface,
    ) {
        if self.field_ticket == ticket {
            self.field = Some((revision, job_id, surface));
        }
    }

    /// CPU-only update under AppState; native/GPU work remains on the main thread.
    pub(crate) fn apply_bond_analysis(
        &mut self,
        document: &Document,
        revision: u64,
        status: String,
        version: Option<String>,
        bonds: Option<Vec<scene::DisplayBond>>,
    ) -> Result<(), String> {
        self.sync_scene(Some(document), revision);
        let replacement = bonds.unwrap_or_else(|| Scene::from_document(Some(document)).bonds);
        if document.bonds.is_none() {
            self.scene.set_bonds(replacement)?;
        }
        self.bond_analysis = Some((status, version));
        Ok(())
    }
    pub(crate) fn editor_bonds(
        &mut self,
        document: &Document,
        revision: u64,
    ) -> Vec<crate::document::Bond> {
        self.sync_scene(Some(document), revision);
        self.scene
            .bonds
            .iter()
            .map(|bond| {
                let mut atoms = bond.atoms;
                atoms.sort_unstable();
                crate::document::Bond {
                    atoms,
                    order: bond.order,
                }
            })
            .collect()
    }

    pub(crate) fn editor_drag(
        &self,
        revision: u64,
        aspect: f32,
        start: [f32; 2],
        end: [f32; 2],
    ) -> Result<(usize, [f64; 3]), String> {
        if self.document_revision != Some(revision)
            || !aspect.is_finite()
            || !(0.001..=1000.0).contains(&aspect)
            || start
                .into_iter()
                .chain(end)
                .any(|v| !(0.0..=1.0).contains(&v))
        {
            return Err("atom_edit".into());
        }
        let index = self
            .scene
            .pick(&self.camera, aspect, start[0], start[1])
            .ok_or("atom_edit")?;
        let local = self.scene.atoms[index].position - self.scene.center;
        let matrix = self.camera.matrix(self.scene.radius, aspect);
        let depth = matrix.project_point3(local).z;
        let inverse = matrix.inverse();
        let point = |[x, y]: [f32; 2]| {
            inverse.project_point3(glam::Vec3::new(2.0 * x - 1.0, 1.0 - 2.0 * y, depth))
        };
        let delta = (point(end) - point(start)).to_array().map(f64::from);
        Ok((index, delta))
    }

    pub fn ready(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.viewport.is_some() && self.frames > 0
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
    pub fn detach(&mut self, client: &str) {
        if client == self.client {
            self.close();
        }
    }
    pub fn close(&mut self) {
        #[cfg(target_os = "macos")]
        {
            self.viewport = None;
        }
        self.client.clear();
        self.frames = 0;
    }
    #[cfg(target_os = "macos")]
    pub(crate) fn update(
        &mut self,
        webview: *mut std::ffi::c_void,
        request: ViewportRequest,
        document: Option<&Document>,
        revision: u64,
        job_id: Option<&str>,
    ) -> Result<ViewportSnapshot, String> {
        request.bounds.validate()?;
        if request.client.len() > 100 || request.actions.len() > 128 {
            return Err("viewport_request".into());
        }
        if request.attach {
            self.client = request.client.clone();
        }
        if request.client != self.client {
            return Err("viewport_stale".into());
        }
        if self.context.is_none() {
            self.context = Some(pollster::block_on(GpuContext::initialize())?);
        }
        self.sync_scene(document, revision);
        if self
            .field
            .as_ref()
            .is_some_and(|(source_revision, source_job, _)| {
                *source_revision != revision || Some(source_job.as_str()) != job_id
            })
        {
            self.field = None;
        }
        let surface = self.field.as_ref().map(|(_, _, mesh)| mesh);
        self.scene.radius = surface.map_or(self.molecule_radius, |mesh| {
            self.molecule_radius.max(mesh.radius * 1.12)
        });
        let gpu = self.context.as_ref().unwrap();
        if self.viewport.is_none() {
            self.viewport = Some(native::Viewport::new(gpu, webview)?);
        }
        let viewport = self.viewport.as_mut().unwrap();
        let aspect = (request.bounds.width / request.bounds.height.max(1.0)).max(0.001) as f32;
        // Never apply queued picking/rotation against a replacement document.
        if request.document_revision == revision {
            for action in request.actions {
                match action {
                    Action::Orbit { dx, dy } if dx.is_finite() && dy.is_finite() => self
                        .camera
                        .orbit(dx.clamp(-1000.0, 1000.0), dy.clamp(-1000.0, 1000.0)),
                    Action::Zoom { delta } if delta.is_finite() => {
                        self.camera.zoom_by(delta.clamp(-1000.0, 1000.0))
                    }
                    Action::Fit => self.camera = Camera::default(),
                    Action::Pick { x, y }
                        if (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y) =>
                    {
                        self.selected = self.scene.pick(&self.camera, aspect, x, y)
                    }
                    Action::Bonds => self.show_bonds = !self.show_bonds,
                    Action::SelectNext => {
                        self.selected = if self.scene.atoms.is_empty() {
                            None
                        } else {
                            Some(
                                self.selected
                                    .map_or(0, |i| (i + 1) % self.scene.atoms.len()),
                            )
                        }
                    }
                    _ => return Err("viewport_action".into()),
                }
            }
        }
        let rendered = viewport.render(
            gpu,
            webview,
            request.bounds,
            &self.scene,
            &self.camera,
            self.selected,
            self.show_bonds,
            surface.map(|mesh| (self.field_ticket, mesh)),
        )?;
        if rendered {
            self.frames += 1;
        }
        let selected = self.selected.and_then(|index| {
            document.map(|doc| Selection {
                index: index + 1,
                symbol: self.scene.symbol(index).into(),
                position: doc.atoms[index].position_angstrom,
                bonds: self
                    .scene
                    .bonds
                    .iter()
                    .filter_map(|bond| {
                        let [a, b] = bond.atoms;
                        if index == a {
                            Some((b + 1, bond.order))
                        } else if index == b {
                            Some((a + 1, bond.order))
                        } else {
                            None
                        }
                    })
                    .collect(),
                inferred_neighbors: self
                    .scene
                    .bonds
                    .iter()
                    .filter(|bond| bond.atoms.contains(&index))
                    .count(),
            })
        });
        Ok(ViewportSnapshot {
            ready: self.frames > 0,
            document_revision: revision,
            selected,
            show_bonds: self.show_bonds,
            bond_count: self.scene.bonds.len(),
            bond_source: if document.is_some_and(|doc| doc.bonds.is_some()) {
                "manual"
            } else if self
                .bond_analysis
                .as_ref()
                .is_some_and(|(status, _)| status == "inferred")
            {
                "rdkit"
            } else {
                "geometry"
            }
            .into(),
            bond_status: self
                .bond_analysis
                .as_ref()
                .map_or("pending", |(status, _)| status.as_str())
                .into(),
            bond_engine_version: self
                .bond_analysis
                .as_ref()
                .and_then(|(_, version)| version.clone()),
            double_bond_count: self.scene.bonds.iter().filter(|b| b.order == 2).count(),
            triple_bond_count: self.scene.bonds.iter().filter(|b| b.order == 3).count(),
            pixel_size: viewport.pixel_size(),
            frame_count: self.frames,
        })
    }
}

#[cfg(test)]
mod bond_tests {
    use super::*;
    #[test]
    fn drag_unprojects_on_the_atom_depth_and_checks_revision() {
        let doc = Document::from_xyz("1\nH\nH 0 0 0", 0, 1).unwrap();
        let mut renderer = Renderer::default();
        renderer.sync_scene(Some(&doc), 5);
        let (index, delta) = renderer
            .editor_drag(5, 2.0, [0.5, 0.5], [0.6, 0.4])
            .unwrap();
        assert_eq!(index, 0);
        let matrix = renderer.camera.matrix(renderer.scene.radius, 2.0);
        let target = matrix.project_point3(glam::Vec3::from_array(delta.map(|v| v as f32)));
        assert!((target.x - 0.2).abs() < 1e-4 && (target.y - 0.2).abs() < 1e-4);
        assert!((target.z - matrix.project_point3(glam::Vec3::ZERO).z).abs() < 1e-4);
        assert!(renderer
            .editor_drag(4, 2.0, [0.5, 0.5], [0.6, 0.4])
            .is_err());
    }

    #[test]
    fn assignment_replaces_hints_without_resetting_view_and_clears_on_revision() {
        let doc = Document::from_xyz("3\nCO2\nO -1.16 0 0\nC 0 0 0\nO 1.16 0 0", 0, 0).unwrap();
        let mut renderer = Renderer::default();
        renderer.sync_scene(Some(&doc), 7);
        renderer.camera.orbit(30.0, 20.0);
        renderer.selected = Some(1);
        let yaw = renderer.camera.yaw;
        renderer
            .apply_bond_analysis(
                &doc,
                7,
                "inferred".into(),
                Some("fixture".into()),
                Some(vec![
                    scene::DisplayBond {
                        atoms: [0, 1],
                        order: 2,
                    },
                    scene::DisplayBond {
                        atoms: [1, 2],
                        order: 2,
                    },
                ]),
            )
            .unwrap();
        assert_eq!(renderer.scene.bond_instance_count(), 8);
        assert_eq!(renderer.selected, Some(1));
        assert_eq!(renderer.camera.yaw, yaw);
        renderer
            .apply_bond_analysis(&doc, 7, "failed".into(), None, None)
            .unwrap();
        assert_eq!(renderer.scene.bond_instance_count(), 4);
        assert_eq!(renderer.camera.yaw, yaw);
        renderer.sync_scene(Some(&doc), 8);
        assert!(renderer.bond_analysis.is_none());
        assert_eq!(renderer.selected, None);
    }
}
