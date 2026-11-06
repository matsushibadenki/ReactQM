fn main() {
    match pollster::block_on(reactqm_lib::renderer::GpuContext::initialize()) {
        Ok(context) => println!(
            "wgpu device ready: {} ({:?})",
            context.adapter_info.name, context.adapter_info.backend
        ),
        Err(error) => {
            eprintln!("GPU probe failed: {error}");
            std::process::exit(1);
        }
    }
}
