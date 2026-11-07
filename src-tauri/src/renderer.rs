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
    pub pixel_size: [u32; 2],
    pub frame_count: u64,
}
#[derive(Serialize)]
pub struct Selection {
    pub index: usize,
    pub symbol: String,
    pub position: [f64; 3],
}

pub struct Renderer {
    // Drop the surface before its device and native layer attachment.
    #[cfg(target_os = "macos")]
    viewport: Option<native::Viewport>,
    pub context: Option<GpuContext>,
    scene: Scene,
    camera: Camera,
    selected: Option<usize>,
    show_bonds: bool,
    document_revision: Option<u64>,
    client: String,
    frames: u64,
}
impl Default for Renderer {
    fn default() -> Self {
        Self {
            #[cfg(target_os = "macos")]
            viewport: None,
            context: None,
            scene: Scene::default(),
            camera: Camera::default(),
            selected: None,
            show_bonds: true,
            document_revision: None,
            client: String::new(),
            frames: 0,
        }
    }
}
impl Renderer {
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
        let gpu = self.context.as_ref().unwrap();
        if self.viewport.is_none() {
            self.viewport = Some(native::Viewport::new(gpu, webview)?);
        }
        let viewport = self.viewport.as_mut().unwrap();
        if self.document_revision != Some(revision) {
            self.scene = Scene::from_document(document);
            self.camera = Camera::default();
            self.selected = None;
            self.document_revision = Some(revision);
        }
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
        )?;
        if rendered {
            self.frames += 1;
        }
        let selected = self.selected.and_then(|index| {
            document.map(|doc| Selection {
                index: index + 1,
                symbol: self.scene.symbol(index).into(),
                position: doc.atoms[index].position_angstrom,
            })
        });
        Ok(ViewportSnapshot {
            ready: self.frames > 0,
            document_revision: revision,
            selected,
            show_bonds: self.show_bonds,
            bond_count: self.scene.bonds.len(),
            pixel_size: viewport.pixel_size(),
            frame_count: self.frames,
        })
    }
}
