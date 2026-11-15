//! Real RDKit → Rust geometry → GPU readback. No frame data enters application IPC.
use reactqm_lib::{
    catalog::{Catalog, Claim},
    document::{Bond, Document},
    fields::{self, Field},
    jobs::{JobRequest, WorkerConfig},
    renderer::{
        gpu::{GpuContext, RenderCore, SurfaceMeshes, ViewRegion},
        scene::{Camera, Scene},
    },
    scheduler::run_claimed,
    valence::analyze,
};
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
fn render(
    gpu: &GpuContext,
    core: &RenderCore,
    scene: &Scene,
    camera: &Camera,
    size: [u32; 2],
    dark: bool,
    region: Option<ViewRegion>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    render_surface(gpu, core, scene, camera, size, dark, region, None)
}

#[allow(clippy::too_many_arguments)]
fn render_surface(
    gpu: &GpuContext,
    core: &RenderCore,
    scene: &Scene,
    camera: &Camera,
    size: [u32; 2],
    dark: bool,
    region: Option<ViewRegion>,
    surface: Option<&SurfaceMeshes>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let [width, height] = size;
    let targets = core.targets(gpu, width, height);
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bond probe"),
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
    let aspect = region
        .as_ref()
        .map_or(width as f32 / height as f32, |r| r.aspect);
    let mut instances = scene.instances_for_camera(None, true, camera, aspect);
    if let Some(surface) = surface {
        instances.extend(surface.instances());
    }
    let binding = core.upload(gpu, &instances);
    core.draw_with_surface(
        gpu,
        &texture.create_view(&Default::default()),
        &targets,
        &binding,
        scene,
        camera,
        true,
        dark,
        region,
        surface,
    );
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bond readback"),
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
    Ok(buffer.slice(..).get_mapped_range().to_vec())
}
fn bands(bytes: &[u8], width: u32, height: u32, x: u32) -> usize {
    let bg = &bytes[..3];
    let mut previous = false;
    let mut count = 0;
    for y in 0..height {
        let pixel = &bytes[((y * width + x) * 4) as usize..][..3];
        let on = pixel
            .iter()
            .zip(bg)
            .map(|(a, b)| a.abs_diff(*b) as u32)
            .sum::<u32>()
            > 45;
        if on && !previous {
            count += 1;
        }
        previous = on;
    }
    count
}
fn save(
    name: &str,
    bytes: &[u8],
    width: u32,
    height: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::var("REACTQM_BOND_RENDER_OUTPUT")
        .unwrap_or_else(|_| "/private/tmp/reactqm-bond-render".into());
    std::fs::create_dir_all(&directory)?;
    let file = std::fs::File::create(format!("{directory}/{name}.png"))?;
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(bytes)?;
    Ok(())
}
fn verify_fields(gpu: &GpuContext, core: &RenderCore) -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join(format!("reactqm-fields-{}", uuid::Uuid::new_v4()));
    let config = WorkerConfig::development();
    let catalog = Catalog::open(&directory)?;
    let document = Document::from_xyz("2\nH2\nH 0 0 0\nH .74 0 0", 0, 0)?;
    let request = JobRequest::new(document.clone(), 1)?;
    catalog.enqueue(&request, None)?;
    catalog.set_queue_paused(false)?;
    let Claim::Ready {
        request,
        directory: job_dir,
    } = catalog.claim_next()?
    else {
        return Err("missing claim".into());
    };
    let result = run_claimed(
        &catalog,
        &request,
        &job_dir,
        &config,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )?;
    assert!((result.energy_hartree + 1.1167593074).abs() < 1e-7);
    drop(catalog);
    let catalog = Catalog::open(&directory)?;
    let camera = Camera {
        yaw: 0.0,
        pitch: 0.0,
        zoom: 1.0,
    };
    for (field, name) in [
        (Field::Homo, "homo"),
        (Field::Lumo, "lumo"),
        (Field::Density, "density"),
    ] {
        let surface = fields::load(&catalog, &request.job_id, field, &config)?;
        let mut scene = Scene::from_document(Some(&document));
        scene.radius = scene.radius.max(surface.radius);
        let meshes = SurfaceMeshes::upload(gpu, &surface);
        let image = render_surface(
            gpu,
            core,
            &scene,
            &camera,
            [512, 384],
            false,
            None,
            Some(&meshes),
        )?;
        let phase = |test: fn(i32, i32, i32) -> bool| {
            image
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| test(i32::from(p[0]), i32::from(p[1]), i32::from(p[2])))
                .count()
        };
        if field == Field::Density {
            assert!(phase(|r, g, b| g - r > 30 && g - b > 15) > 1000);
        } else {
            let blue = phase(|r, g, b| b - r > 30 && b - g > 15);
            let red = phase(|r, g, b| r - g > 30 && r - b > 15);
            assert!(if field == Field::Lumo {
                blue.min(red) > 1000
            } else {
                blue.max(red) > 1000
            });
        }
        save(name, &image, 512, 384)?;
    }
    let artifact = job_dir.join("result.h5");
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(artifact)?
        .write_all(b"changed")?;
    assert!(
        matches!(fields::load(&catalog, &request.job_id, Field::Homo, &config), Err(error) if error == "artifact_invalid")
    );
    drop(catalog);
    std::fs::remove_dir_all(directory)?;
    println!(
        "PASS real PySCF H2: HOMO, two LUMO phases, total density and tamper rejection; {} / {:?}",
        gpu.adapter_info.name, gpu.adapter_info.backend
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gpu = pollster::block_on(GpuContext::initialize())?;
    let core = RenderCore::new(&gpu, wgpu::TextureFormat::Rgba8UnormSrgb)?;
    if std::env::args().any(|argument| argument == "--fields") {
        return verify_fields(&gpu, &core);
    }
    let co2 = Document::from_xyz("3\nCO2\nO -1.16 0 0\nC 0 0 0\nO 1.16 0 0", 0, 0)?;
    let n2 = Document::from_xyz("2\nN2\nN -.55 0 0\nN .55 0 0", 0, 0)?;
    let camera = Camera {
        yaw: 0.0,
        pitch: 0.0,
        zoom: 1.0,
    };
    let mut double_scene = Scene::from_document(Some(&co2));
    let hints = render(&gpu, &core, &double_scene, &camera, [512, 384], false, None)?;
    let analysis = analyze(
        &co2,
        &WorkerConfig::development(),
        &AtomicBool::new(false),
        Duration::from_secs(10),
    )?;
    double_scene.set_bonds(analysis.display_bonds().ok_or("CO2 assignment")?)?;
    let double = render(&gpu, &core, &double_scene, &camera, [512, 384], false, None)?;
    let point = camera
        .matrix(double_scene.radius, 512.0 / 384.0)
        .project_point3(glam::Vec3::new(-0.58, 0.0, 0.0) - double_scene.center);
    let x = ((point.x + 1.0) * 0.5 * 512.0).round() as u32;
    assert_eq!(bands(&hints, 512, 384, x), 1, "fallback rod");
    assert_eq!(
        bands(&double, 512, 384, x),
        2,
        "double rods must be visibly separated"
    );
    assert_ne!(hints, double);
    save("co2-hints", &hints, 512, 384)?;
    save("co2-double", &double, 512, 384)?;
    let cropped = render(
        &gpu,
        &core,
        &double_scene,
        &camera,
        [512, 192],
        false,
        Some(ViewRegion {
            aspect: 512.0 / 384.0,
            crop: [0.0, 0.5, 1.0, 0.5],
        }),
    )?;
    let differing = cropped
        .iter()
        .zip(&double[512 * 192 * 4..])
        .filter(|(a, b)| a.abs_diff(**b) > 2)
        .count();
    assert!(
        differing < cropped.len() / 100,
        "multiple rods moved during crop"
    );
    save("co2-cropped", &cropped, 512, 192)?;
    let mut triple_scene = Scene::from_document(Some(&n2));
    let analysis = analyze(
        &n2,
        &WorkerConfig::development(),
        &AtomicBool::new(false),
        Duration::from_secs(10),
    )?;
    triple_scene.set_bonds(analysis.display_bonds().ok_or("N2 assignment")?)?;
    let triple = render(&gpu, &core, &triple_scene, &camera, [512, 384], false, None)?;
    assert_eq!(
        bands(&triple, 512, 384, 256),
        3,
        "triple rods must be visibly separated"
    );
    let mut edited = n2.clone();
    edited.bonds = Some(vec![Bond {
        atoms: [0, 1],
        order: 3,
    }]);
    edited.validate()?;
    let edited_scene = Scene::from_document(Some(&edited));
    let edited_triple = render(&gpu, &core, &edited_scene, &camera, [512, 384], false, None)?;
    assert_eq!(
        bands(&edited_triple, 512, 384, 256),
        3,
        "manual triple rods"
    );
    edited.bonds = Some(Vec::new());
    let no_bond = render(
        &gpu,
        &core,
        &Scene::from_document(Some(&edited)),
        &camera,
        [512, 384],
        false,
        None,
    )?;
    assert_eq!(bands(&no_bond, 512, 384, 256), 0);
    save("n2-triple", &triple, 512, 384)?;
    let portrait = render(&gpu, &core, &triple_scene, &camera, [256, 512], true, None)?;
    assert_eq!(
        bands(&portrait, 256, 512, 128),
        3,
        "dark portrait triple rods"
    );
    save("n2-portrait-dark", &portrait, 256, 512)?;
    let mut rotated = camera;
    rotated.orbit(100.0, 50.0);
    let rotated_image = render(&gpu, &core, &double_scene, &rotated, [512, 384], true, None)?;
    save("co2-rotated-dark", &rotated_image, 512, 384)?;
    assert_ne!(double, rotated_image);
    println!("PASS: real RDKit CO2/N2; single/double/triple pixel bands 1/2/3, partial crop, rotation, dark portrait; {} / {:?}",gpu.adapter_info.name,gpu.adapter_info.backend);
    Ok(())
}
