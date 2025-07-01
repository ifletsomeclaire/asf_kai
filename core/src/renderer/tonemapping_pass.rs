use bevy_ecs::{event::EventReader, prelude::*};
use eframe::egui_wgpu::{self, CallbackTrait, wgpu};
use std::sync::Arc;

use super::{
    core::{HDR_FORMAT, WgpuDevice, WgpuRenderState},
    events::ResizeEvent,
};

#[derive(Resource)]
pub struct HdrTexture {
    pub view: wgpu::TextureView,
}

#[derive(Resource, Clone)]
pub struct TonemappingPass {
    pub pipeline: Arc<wgpu::RenderPipeline>,
    pub bind_group_layout: Arc<wgpu::BindGroupLayout>,
}

#[derive(Resource)]
pub struct TonemappingBindGroup(pub wgpu::BindGroup);

pub struct FinalBlitCallback {}

impl CallbackTrait for FinalBlitCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        _resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        Vec::new()
    }

    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let pass: &TonemappingPass = resources.get().unwrap();
        let bind_group: &TonemappingBindGroup = resources.get().unwrap();
        render_pass.set_pipeline(&pass.pipeline);
        render_pass.set_bind_group(0, &bind_group.0, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

pub fn resize_hdr_texture_system(
    mut commands: Commands,
    mut resize_events: EventReader<ResizeEvent>,
    device: Res<WgpuDevice>,
    tonemapping_pass: Res<TonemappingPass>,
    wgpu_render_state: Res<WgpuRenderState>,
) {
    for event in resize_events.read() {
        if event.0.width == 0 || event.0.height == 0 {
            return;
        }

        // First, remove the old texture resource
        commands.remove_resource::<HdrTexture>();

        let new_size = event.0;
        let device = &device.0;

        // Create new HdrTexture
        let hdr_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr_texture"),
            size: new_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let hdr_view = hdr_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let hdr_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hdr_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Create new bind group
        let tonemap_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tonemapping"),
            layout: &tonemapping_pass.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&hdr_sampler),
                },
            ],
        });

        // Insert new bind group directly into callback resources
        wgpu_render_state
            .0
            .renderer
            .write()
            .callback_resources
            .insert(TonemappingBindGroup(tonemap_bind_group));

        // Insert new texture resource
        commands.insert_resource(HdrTexture {
            view: hdr_view,
        });
    }
}

pub fn setup_tonemapping_pass_system(
    mut commands: Commands,
    device_res: Res<WgpuDevice>,
    wgpu_render_state_res: Res<WgpuRenderState>,
    initial_size: Res<crate::app::InitialSize>,
) {
    let device = &device_res.0;
    let wgpu_render_state = &wgpu_render_state_res.0;

    let size = initial_size.0;
    let hdr_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hdr_texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: HDR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let hdr_view = hdr_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let hdr_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("hdr_sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    let tonemap_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("tonemapping"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/tonemapping.wgsl").into()),
    });

    let tonemap_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tonemapping"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

    let tonemap_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("tonemapping"),
        bind_group_layouts: &[&tonemap_bind_group_layout],
        push_constant_ranges: &[],
    });

    let tonemap_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("tonemapping"),
        layout: Some(&tonemap_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &tonemap_shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &tonemap_shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu_render_state.target_format.into())],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    });

    let tonemap_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tonemapping"),
        layout: &tonemap_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&hdr_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&hdr_sampler),
            },
        ],
    });

    commands.insert_resource(TonemappingPass {
        pipeline: Arc::new(tonemap_pipeline),
        bind_group_layout: Arc::new(tonemap_bind_group_layout),
    });
    commands.insert_resource(TonemappingBindGroup(tonemap_bind_group));

    commands.insert_resource(HdrTexture {
        view: hdr_view,
    });
}
