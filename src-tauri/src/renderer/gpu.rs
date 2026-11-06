use super::scene::{Camera, Instance, Scene};
use wgpu::util::DeviceExt;

pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_info: wgpu::AdapterInfo,
}
impl GpuContext {
    pub async fn initialize() -> Result<Self, String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .map_err(|error| error.to_string())?;
        let adapter_info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|error| error.to_string())?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            adapter_info,
        })
    }
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    view_projection: [[f32; 4]; 4],
    eye: [f32; 4],
}
struct Mesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
}
impl Mesh {
    fn upload(device: &wgpu::Device, vertices: &[Vertex], indices: &[u32]) -> Self {
        Self {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("molecule vertices"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("molecule indices"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            count: indices.len() as u32,
        }
    }
    fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, instances: std::ops::Range<u32>) {
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.count, 0, instances);
    }
}
fn sphere() -> (Vec<Vertex>, Vec<u32>) {
    let (rows, columns) = (20, 32);
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for row in 0..=rows {
        for column in 0..=columns {
            let theta = std::f32::consts::PI * row as f32 / rows as f32;
            let phi = std::f32::consts::TAU * column as f32 / columns as f32;
            let normal = [
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            ];
            vertices.push(Vertex {
                position: normal,
                normal,
            });
        }
    }
    for row in 0..rows {
        for column in 0..columns {
            let a = row * (columns + 1) + column;
            let b = a + columns + 1;
            indices.extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    (vertices, indices)
}
fn cylinder() -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for column in 0..=20 {
        let angle = std::f32::consts::TAU * column as f32 / 20.0;
        let normal = [angle.cos(), 0.0, angle.sin()];
        for y in [-0.5, 0.5] {
            vertices.push(Vertex {
                position: [normal[0], y, normal[2]],
                normal,
            });
        }
        if column < 20 {
            let a = column * 2;
            indices.extend_from_slice(&[a, a + 1, a + 2, a + 2, a + 1, a + 3]);
        }
    }
    (vertices, indices)
}

pub struct RenderCore {
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    sphere: Mesh,
    cylinder: Mesh,
    pub format: wgpu::TextureFormat,
    pub samples: u32,
}
/// Visible rectangle in normalized coordinates of the original DOM viewport.
#[derive(Clone, Copy)]
pub struct ViewRegion {
    pub aspect: f32,
    pub crop: [f32; 4],
}
impl ViewRegion {
    pub fn matrix(&self, camera: &Camera, radius: f32) -> glam::Mat4 {
        let [x, y, w, h] = self.crop;
        let crop = glam::Mat4::from_scale_rotation_translation(
            glam::Vec3::new(1.0 / w, 1.0 / h, 1.0),
            glam::Quat::IDENTITY,
            glam::Vec3::new((1.0 - 2.0 * x - w) / w, (h + 2.0 * y - 1.0) / h, 0.0),
        );
        crop * camera.matrix(radius, self.aspect)
    }
}
pub struct Targets {
    pub depth: wgpu::TextureView,
    pub multisample: Option<wgpu::TextureView>,
    pub width: u32,
    pub height: u32,
}
impl RenderCore {
    pub fn new(gpu: &GpuContext, format: wgpu::TextureFormat) -> Result<Self, String> {
        let device = &gpu.device;
        let samples = if gpu
            .adapter
            .get_texture_format_features(format)
            .flags
            .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4)
        {
            4
        } else {
            1
        };
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("molecular shading"),
            source: wgpu::ShaderSource::Wgsl(include_str!("molecule.wgsl").into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera and instances"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("molecule pipeline layout"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("molecule pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3],
                }],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        if let Some(error) = pollster::block_on(device.pop_error_scope()) {
            return Err(error.to_string());
        }
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (vertices, indices) = sphere();
        let sphere = Mesh::upload(device, &vertices, &indices);
        let (vertices, indices) = cylinder();
        let cylinder = Mesh::upload(device, &vertices, &indices);
        Ok(Self {
            pipeline,
            bind_layout,
            uniform,
            sphere,
            cylinder,
            format,
            samples,
        })
    }
    pub fn targets(&self, gpu: &GpuContext, width: u32, height: u32) -> Targets {
        let texture = |format, samples, label| {
            gpu.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        Targets {
            depth: texture(
                wgpu::TextureFormat::Depth32Float,
                self.samples,
                "molecule depth",
            ),
            multisample: if self.samples > 1 {
                Some(texture(self.format, self.samples, "molecule MSAA"))
            } else {
                None
            },
            width,
            height,
        }
    }
    pub fn upload(&self, gpu: &GpuContext, instances: &[Instance]) -> wgpu::BindGroup {
        // A nonempty buffer is required even when the scene has zero draw calls.
        let empty = [Instance::zeroed()];
        use bytemuck::Zeroable;
        let buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("molecular instances"),
                contents: bytemuck::cast_slice(if instances.is_empty() {
                    &empty
                } else {
                    instances
                }),
                usage: wgpu::BufferUsages::STORAGE,
            });
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("molecule binding"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffer.as_entire_binding(),
                },
            ],
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        gpu: &GpuContext,
        view: &wgpu::TextureView,
        targets: &Targets,
        binding: &wgpu::BindGroup,
        scene: &Scene,
        camera: &Camera,
        show_bonds: bool,
        dark: bool,
        region: Option<ViewRegion>,
    ) {
        let region = region.unwrap_or(ViewRegion {
            aspect: targets.width as f32 / targets.height as f32,
            crop: [0.0, 0.0, 1.0, 1.0],
        });
        let aspect = region.aspect;
        let uniform = Uniform {
            view_projection: region.matrix(camera, scene.radius).to_cols_array_2d(),
            eye: camera.eye(scene.radius, aspect).extend(1.0).to_array(),
        };
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("molecule frame"),
            });
        {
            let color = if dark {
                wgpu::Color {
                    r: 0.018,
                    g: 0.025,
                    b: 0.038,
                    a: 1.0,
                }
            } else {
                wgpu::Color {
                    r: 0.90,
                    g: 0.925,
                    b: 0.95,
                    a: 1.0,
                }
            };
            let (attachment, resolve) = match &targets.multisample {
                Some(msaa) => (msaa, Some(view)),
                None => (view, None),
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("molecule pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: attachment,
                    depth_slice: None,
                    resolve_target: resolve,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, binding, &[]);
            let atoms = scene.atoms.len() as u32;
            self.sphere.draw(&mut pass, 0..atoms);
            if show_bonds {
                self.cylinder
                    .draw(&mut pass, atoms..atoms + scene.bonds.len() as u32 * 2);
            }
        }
        gpu.queue.submit(Some(encoder.finish()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn triangle_winding_and_normals_match_for_both_meshes() {
        for (vertices, indices) in [sphere(), cylinder()] {
            for triangle in indices.as_chunks::<3>().0 {
                let a = vertices[triangle[0] as usize];
                let b = vertices[triangle[1] as usize];
                let c = vertices[triangle[2] as usize];
                let cross = (glam::Vec3::from(b.position) - glam::Vec3::from(a.position))
                    .cross(glam::Vec3::from(c.position) - glam::Vec3::from(a.position));
                if cross.length() > 1e-6 {
                    assert!(cross.dot(glam::Vec3::from(a.normal)) > 0.0);
                }
            }
        }
    }
}
