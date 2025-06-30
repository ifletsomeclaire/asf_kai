mod app;
mod config;
mod ecs;
mod renderer;

use config::Config;

fn main() -> eframe::Result<()> {
    let config = Config::load();

    let native_options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        vsync: config.vsync,
        ..Default::default()
    };

    eframe::run_native(
        "My egui App",
        native_options,
        Box::new(|cc| Ok(Box::new(app::Custom3d::new(cc).unwrap()))),
    )
}
