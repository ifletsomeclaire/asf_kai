mod app;
mod config;
mod ecs;
mod renderer;

use std::sync::Arc;

use config::Config;
use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetupCreateNew};
use wgpu::{FeaturesWGPU, FeaturesWebGPU};

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt::init();
    let config = Config::load();

    let native_options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        vsync: config.vsync,
        wgpu_options: WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(WgpuSetupCreateNew {
                device_descriptor: Arc::new(|adapter| {
                    let base_limits = if adapter.get_info().backend == wgpu::Backend::Gl {
                        wgpu::Limits::downlevel_webgl2_defaults()
                    } else {
                        wgpu::Limits::default()
                    };

                    wgpu::DeviceDescriptor {
                        label: Some("egui wgpu device"),
                        required_features: wgpu::Features {
                            features_webgpu: FeaturesWebGPU::default(),
                            features_wgpu: FeaturesWGPU::PUSH_CONSTANTS,
                        },
                        required_limits: wgpu::Limits {
                            // When using a depth buffer, we have to be able to create a texture
                            // large enough for the entire surface, and we want to support 4k+ displays.
                            max_texture_dimension_2d: 8192,
                            ..base_limits
                        },
                        memory_hints: wgpu::MemoryHints::default(),
                        trace: wgpu::Trace::Off,
                    }
                }),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    eframe::run_native(
        "My egui App",
        native_options,
        Box::new(|cc| Ok(Box::new(app::Custom3d::new(cc).unwrap()))),
    )
}
