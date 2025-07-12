use crate::renderer::{
    core::{HDR_FORMAT, WgpuDevice, WgpuQueue, WgpuRenderState},
    pipelines::d3_pipeline::DEPTH_FORMAT,
    events::ResizeEvent,
};
use bevy_derive::{Deref, DerefMut};
use bevy_ecs::{event::EventReader, prelude::*};
use eframe::egui_wgpu::{self, CallbackTrait, wgpu};
use std::sync::Arc;

#[derive(Resource, Clone)]
pub struct TonemappingPass {
    pub pipeline: Arc<wgpu::RenderPipeline>,
    pub bind_group_layout: Arc<wgpu::BindGroupLayout>,
}

#[derive(Resource)]
pub struct HdrTexture {
    pub view: wgpu::TextureView,
    pub size: wgpu::Extent3d,
}

#[derive(Resource)]
pub struct IdTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

#[derive(Resource)]
pub struct DepthTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

#[derive(Resource, Deref, DerefMut)]
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
    hdr_texture: ResMut<HdrTexture>,
    mut depth_texture: ResMut<DepthTexture>,
    mut id_texture: ResMut<IdTexture>,
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
        let hdr_texture_inner = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr_texture"),
            size: new_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let hdr_view = hdr_texture_inner.create_view(&wgpu::TextureViewDescriptor::default());
        let hdr_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hdr_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Create new DepthTexture
        let depth_texture_inner = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth_texture"),
            size: new_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[DEPTH_FORMAT],
        });
        depth_texture.texture = depth_texture_inner;
        depth_texture.view = depth_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Create new ID texture
        let id_texture_inner = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("id_texture"),
            size: new_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[wgpu::TextureFormat::R32Uint],
        });
        id_texture.texture = id_texture_inner;
        id_texture.view = id_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

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
            size: new_size,
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
    let hdr_texture_desc = wgpu::TextureDescriptor {
        label: Some("hdr_texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: HDR_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[HDR_FORMAT],
    };
    let hdr_texture = device.create_texture(&hdr_texture_desc);
    let hdr_view = hdr_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let hdr_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("hdr_sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    let depth_texture_desc = wgpu::TextureDescriptor {
        label: Some("depth_texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[DEPTH_FORMAT],
    };
    let depth_texture = device.create_texture(&depth_texture_desc);
    let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
    commands.insert_resource(DepthTexture {
        texture: depth_texture,
        view: depth_view,
    });

    // Create ID texture for GPU picking
    let id_texture_desc = wgpu::TextureDescriptor {
        label: Some("id_texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Uint,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[wgpu::TextureFormat::R32Uint],
    };
    let id_texture = device.create_texture(&id_texture_desc);
    let id_view = id_texture.create_view(&wgpu::TextureViewDescriptor::default());
    commands.insert_resource(IdTexture {
        texture: id_texture,
        view: id_view,
    });

    let tonemap_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("tonemapping"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/tonemapping.wgsl").into()),
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
        size,
    });
}



pub fn clear_hdr_and_id_texture_system(
    device: Res<WgpuDevice>,
    queue: Res<WgpuQueue>,
    hdr_texture: Res<HdrTexture>,
    id_texture: Res<IdTexture>,
    depth_texture: Res<DepthTexture>,
) {
    
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("clear_textures_encoder"),
    });
   
    // Clear both HDR and ID textures in one pass
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("clear_textures_pass"),
        color_attachments: &[
            Some(wgpu::RenderPassColorAttachment {
                view: &hdr_texture.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            }),
            Some(wgpu::RenderPassColorAttachment {
                view: &id_texture.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            }),
        ],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &depth_texture.view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
    });

    queue.submit(Some(encoder.finish()));
}
