use crate::renderer::assets::AssetServer;
use bevy_ecs::prelude::Resource;
use bevy_ecs::world::World;
use std::sync::Arc;

pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[derive(Resource)]
pub struct WgpuDevice(pub Arc<wgpu::Device>);

#[derive(Resource)]
pub struct WgpuQueue(pub Arc<wgpu::Queue>);

#[derive(Resource)]
pub struct WgpuRenderState(pub eframe::egui_wgpu::RenderState);

/// Initializes the core rendering resources, including the AssetServer,
/// and inserts them into the world.
pub fn initialize_renderer(world: &mut World, device: &wgpu::Device) {
    let asset_server = AssetServer::new(device);
    world.insert_resource(asset_server);
}

pub struct KaiRenderer {
    // This struct would hold render passes, pipelines, etc.
}

impl KaiRenderer {
    pub fn new(_device: &wgpu::Device, _config: &wgpu::SurfaceConfiguration) -> Self {
        Self {}
    }

    pub fn render(&mut self, _world: &mut World) {
        // The render loop would get the AssetServer from the world.
        // let asset_server = world.get_resource::<AssetServer>().unwrap();
    }
}
