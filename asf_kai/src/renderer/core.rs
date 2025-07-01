use bevy_ecs::prelude::Resource;
use std::sync::Arc;

pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[derive(Resource)]
pub struct WgpuDevice(pub Arc<wgpu::Device>);

#[derive(Resource)]
pub struct WgpuQueue(pub Arc<wgpu::Queue>);

#[derive(Resource)]
pub struct WgpuRenderState(pub eframe::egui_wgpu::RenderState);
