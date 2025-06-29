mod app;
mod config;
mod ecs;
mod render;

use app::Custom3d;
use config::Config;

fn main() -> eframe::Result<()> {
    let config = Config::load();

    let native_options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        // wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
        //     present_mode: if config.vsync {
        //         eframe::wgpu::PresentMode::AutoNoVsync
        //     } else {
        //         eframe::wgpu::PresentMode::AutoVsync
        //     },
        //     wgpu_setup:eframe::egui_wgpu::WgpuSetup::CreateNew(WgpuSetupCreateNew {
        //         instance_descriptor: todo!(),
        //         power_preference: todo!(),
        //         native_adapter_selector: todo!(),
        //         device_descriptor: todo!(),
        //     })
        //     ..Default::default()
        // },
        vsync: config.vsync,
        ..Default::default()
    };

    eframe::run_native(
        "eframe custom 3D",
        native_options,
        Box::new(|cc| {
            let mut custom_3d = Custom3d::new(cc).expect("Failed to create wgpu context");
            custom_3d.world.insert_resource(config);
            Ok(Box::new(custom_3d))
        }),
    )
}
