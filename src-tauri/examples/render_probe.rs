//! Real GPU regression probe. Image readback stays in this Rust process, never in IPC.
use reactqm_lib::{
    document::Document,
    renderer::{
        gpu::{GpuContext, RenderCore, ViewRegion},
        scene::{Camera, Scene},
    },
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gpu = pollster::block_on(GpuContext::initialize())?;
    let core = RenderCore::new(&gpu, wgpu::TextureFormat::Rgba8UnormSrgb)?;
    let doc = Document::from_xyz("3\nWater\nO 0 0 0\nH .7586 0 .5043\nH -.7586 0 .5043", 0, 0)?;
    let scene = Scene::from_document(Some(&doc));
    let mut camera = Camera::default();
    let mut previous = Vec::new();
    let mut reference = Vec::new();
    for (name, width, height, selected, bonds, dark) in [
        ("water", 512, 384, None, true, false),
        ("cropped", 512, 192, None, true, false),
        ("selected", 512, 384, Some(0), true, false),
        ("rotated", 512, 384, None, false, true),
        ("portrait", 256, 512, None, true, false),
    ] {
        if name == "rotated" {
            camera.orbit(100.0, 50.0);
        }
        let targets = core.targets(&gpu, width, height);
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("probe output"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: core.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let binding = core.upload(
            &gpu,
            &scene.instances_for_camera(
                selected,
                bonds,
                &camera,
                if name == "cropped" {
                    512.0 / 384.0
                } else {
                    width as f32 / height as f32
                },
            ),
        );
        core.draw(
            &gpu,
            &texture.create_view(&Default::default()),
            &targets,
            &binding,
            &scene,
            &camera,
            bonds,
            dark,
            if name == "cropped" {
                Some(ViewRegion {
                    aspect: 512.0 / 384.0,
                    crop: [0.0, 0.5, 1.0, 0.5],
                })
            } else {
                None
            },
        );
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("probe readback"),
            size: (width * height * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit(Some(encoder.finish()));
        let (tx, rx) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                tx.send(result).unwrap();
            });
        gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv()??;
        let bytes = buffer.slice(..).get_mapped_range().to_vec();
        if name == "water" {
            reference = bytes.clone();
        }
        if name == "cropped" {
            let expected = &reference[512 * 192 * 4..];
            let differing = bytes
                .iter()
                .zip(expected)
                .filter(|(a, b)| a.abs_diff(**b) > 2)
                .count();
            assert!(
                differing < bytes.len() / 100,
                "cropping moved the molecule: {differing} channels differ"
            );
        }
        let background = &bytes[..4];
        let foreground = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| *pixel != background)
            .count();
        assert!(foreground > 500, "blank or missing molecule: {name}");
        assert_ne!(bytes, previous, "render state did not change");
        if name == "water" {
            assert!(
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|p| p[0] > p[1].saturating_add(50) && p[0] > p[2].saturating_add(50))
                    .count()
                    > 500,
                "oxygen missing"
            );
        }
        if name == "selected" {
            assert!(
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|p| p[0] > p[2].saturating_add(80) && p[1] > p[2].saturating_add(40))
                    .count()
                    > 500,
                "selection missing"
            );
        }
        let directory = std::env::var("REACTQM_RENDER_OUTPUT")
            .unwrap_or_else(|_| "/private/tmp/reactqm-render".into());
        std::fs::create_dir_all(&directory)?;
        let file = std::fs::File::create(format!("{directory}/{name}.png"))?;
        let mut encoder = png::Encoder::new(file, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&bytes)?;
        println!("{name}: {width}×{height}, {foreground} foreground pixels");
        previous = bytes;
    }
    println!(
        "Adapter: {} / {:?}; MSAA {}×",
        gpu.adapter_info.name, gpu.adapter_info.backend, core.samples
    );
    Ok(())
}
