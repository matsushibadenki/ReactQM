use super::{
    gpu::{GpuContext, RenderCore, Targets, ViewRegion},
    scene::{Camera, Scene},
    Bounds,
};
use dispatch2::MainThreadBound;
use objc2::{rc::Retained, MainThreadMarker};
use objc2_app_kit::NSView;
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_quartz_core::{CALayer, CAMetalLayer, CATransaction};

struct Attachment {
    clip: Retained<CALayer>,
    metal: Retained<CAMetalLayer>,
}
impl Drop for Attachment {
    fn drop(&mut self) {
        self.clip.removeFromSuperlayer();
    }
}
pub struct Viewport {
    // Surface must be destroyed before the retained native layer.
    surface: wgpu::Surface<'static>,
    attachment: MainThreadBound<Attachment>,
    core: RenderCore,
    config: wgpu::SurfaceConfiguration,
    targets: Option<Targets>,
}
fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
    CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
}
impl Viewport {
    pub fn new(gpu: &GpuContext, webview: *mut std::ffi::c_void) -> Result<Self, String> {
        let mtm = MainThreadMarker::new().ok_or("viewport_thread")?;
        // Tauri guarantees this pointer is a live WKWebView in with_webview on the main thread.
        let view = unsafe { &*webview.cast::<NSView>() };
        view.setWantsLayer(true);
        let parent = view.layer().ok_or("viewport_layer")?;
        let clip = CALayer::new();
        let metal = CAMetalLayer::new();
        clip.setMasksToBounds(true);
        clip.setGeometryFlipped(false);
        clip.setZPosition(1000.0);
        clip.setHidden(true);
        clip.addSublayer(&metal);
        parent.addSublayer(&clip);
        let attachment = MainThreadBound::new(Attachment { clip, metal }, mtm);
        let pointer = Retained::as_ptr(&attachment.get(mtm).metal)
            .cast_mut()
            .cast();
        // The retained CAMetalLayer outlives the surface (field order above).
        let surface = unsafe {
            gpu.instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(pointer))
        }
        .map_err(|e| e.to_string())?;
        let mut config = surface
            .get_default_config(&gpu.adapter, 1, 1)
            .ok_or("viewport_adapter")?;
        let capabilities = surface.get_capabilities(&gpu.adapter);
        config.format = capabilities
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(config.format);
        config.present_mode = wgpu::PresentMode::Fifo;
        let core = RenderCore::new(gpu, config.format)?;
        Ok(Self {
            surface,
            attachment,
            core,
            config,
            targets: None,
        })
    }
    pub fn pixel_size(&self) -> [u32; 2] {
        [self.config.width, self.config.height]
    }
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        gpu: &GpuContext,
        webview: *mut std::ffi::c_void,
        b: Bounds,
        scene: &Scene,
        camera: &Camera,
        selected: Option<usize>,
        show_bonds: bool,
    ) -> Result<bool, String> {
        let mtm = MainThreadMarker::new().ok_or("viewport_thread")?;
        let view = unsafe { &*webview.cast::<NSView>() };
        let native = self.attachment.get(mtm);
        let x = b.x.max(0.0);
        let y = b.y.max(b.clip_top);
        let right = (b.x + b.width).min(b.window_width);
        let bottom = (b.y + b.height).min(b.window_height);
        let visible = b.visible && right > x && bottom > y && b.width >= 1.0 && b.height >= 1.0;
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        native.clip.setHidden(!visible);
        if !visible {
            CATransaction::commit();
            return Ok(false);
        }
        let scale = view.window().map_or(1.0, |w| w.backingScaleFactor());
        let max = gpu.device.limits().max_texture_dimension_2d.min(4096) as f64;
        // WKWebView may include a title/sharing bar while innerHeight excludes it.
        // Derive CSS-point scale and content origin instead of hard-coding a titlebar height.
        let insets = view.safeAreaInsets();
        let css_scale =
            (view.bounds().size.width - insets.left - insets.right) / b.window_width.max(1.0);
        let top = insets
            .top
            .max(view.bounds().size.height - b.window_height * css_scale - insets.bottom);
        let resolution = (scale * css_scale).min(max / (right - x).max(bottom - y));
        let width = ((right - x) * resolution).round().max(1.0) as u32;
        let height = ((bottom - y) * resolution).round().max(1.0) as u32;
        let parent = view.layer().ok_or("viewport_layer")?;
        let native_y = if parent.isGeometryFlipped() {
            top + y * css_scale
        } else {
            view.bounds().size.height - top - bottom * css_scale
        };
        native.clip.setFrame(rect(
            insets.left + x * css_scale,
            native_y,
            (right - x) * css_scale,
            (bottom - y) * css_scale,
        ));
        // Render only the visible crop; projection preserves the full viewport camera.
        native.metal.setFrame(rect(
            0.0,
            0.0,
            (right - x) * css_scale,
            (bottom - y) * css_scale,
        ));
        native.metal.setContentsScale(resolution / css_scale);
        if self.targets.is_none() || self.config.width != width || self.config.height != height {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&gpu.device, &self.config);
            self.targets = Some(self.core.targets(gpu, width, height));
        }
        native
            .metal
            .setDrawableSize(CGSize::new(width as f64, height as f64));
        CATransaction::commit();
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&gpu.device, &self.config);
                self.surface
                    .get_current_texture()
                    .map_err(|e| e.to_string())?
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(false),
            Err(error) => {
                native.clip.setHidden(true);
                return Err(error.to_string());
            }
        };
        let binding = self
            .core
            .upload(gpu, &scene.instances(selected, show_bonds));
        self.core.draw(
            gpu,
            &frame.texture.create_view(&Default::default()),
            self.targets.as_ref().unwrap(),
            &binding,
            scene,
            camera,
            show_bonds,
            b.dark,
            Some(ViewRegion {
                aspect: (b.width / b.height) as f32,
                crop: [
                    ((x - b.x) / b.width) as f32,
                    ((y - b.y) / b.height) as f32,
                    ((right - x) / b.width) as f32,
                    ((bottom - y) / b.height) as f32,
                ],
            }),
        );
        frame.present();
        Ok(true)
    }
}
