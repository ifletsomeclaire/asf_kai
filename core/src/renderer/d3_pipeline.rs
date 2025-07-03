use bevy_ecs::prelude::*;
use bevy_transform::components::GlobalTransform;
use std::num::NonZeroU64;
use wgpu::util::DeviceExt;

use super::{
    core::{HDR_FORMAT, WgpuDevice, WgpuQueue},
    scene::{FrameRenderData, MeshBindGroup},
    tonemapping_pass::HdrTexture,
};
use crate::ecs::camera::Camera;

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[derive(Resource)]
pub struct DepthTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

#[derive(Resource)]
pub struct D3Pipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub camera_bind_group_layout: wgpu::BindGroupLayout,
    pub mesh_bind_group_layout: wgpu::BindGroupLayout,
}

pub fn setup_d3_pipeline_system(mut commands: Commands, device_res: Res<WgpuDevice>) {
    let device = &device_res.0;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("3d shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/d3.wgsl").into()),
    });

    let camera_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: Some(
                        NonZeroU64::new(std::mem::size_of::<[f32; 16]>() as u64).unwrap(),
                    ),
                },
                count: None,
            }],
        });

    let mesh_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("3d_pipeline_layout"),
        bind_group_layouts: &[&camera_bind_group_layout, &mesh_bind_group_layout],
        push_constant_ranges: &[],
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("3d_pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(HDR_FORMAT.into())],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    });

    commands.insert_resource(D3Pipeline {
        pipeline,
        camera_bind_group_layout,
        mesh_bind_group_layout,
    });
}

pub fn setup_depth_texture_system(
    mut commands: Commands,
    device: Res<WgpuDevice>,
    hdr_texture: Res<HdrTexture>,
) {
    let size = hdr_texture.size;
    let desc = wgpu::TextureDescriptor {
        label: Some("depth_texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[DEPTH_FORMAT],
    };
    let texture = device.0.create_texture(&desc);
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    commands.insert_resource(DepthTexture { texture, view });
}

#[derive(Resource)]
pub struct CameraBindGroup(pub wgpu::BindGroup);

pub fn update_camera_buffer_system(
    mut commands: Commands,
    device: Res<WgpuDevice>,
    query: Query<(&Camera, &GlobalTransform)>,
    pipeline: Res<D3Pipeline>,
) {
    if let Ok((camera, transform)) = query.single() {
        let proj = camera.projection_matrix();
        let view = transform.compute_matrix().inverse();
        let view_proj = proj * view;

        let uniform_buffer = device
            .0
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("camera_uniform_buffer"),
                contents: bytemuck::cast_slice(view_proj.as_ref()),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

        let bind_group = device.0.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera_bind_group"),
            layout: &pipeline.camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        commands.insert_resource(CameraBindGroup(bind_group));
    }
}

pub fn render_d3_pipeline_system(
    device: Res<WgpuDevice>,
    queue: Res<WgpuQueue>,
    pipeline: Res<D3Pipeline>,
    hdr_texture: Res<super::tonemapping_pass::HdrTexture>,
    depth_texture: Res<DepthTexture>,
    frame_data: Option<Res<FrameRenderData>>,
    mesh_bind_group: Option<Res<MeshBindGroup>>,
    camera_bind_group: Option<Res<CameraBindGroup>>,
) {
    let (Some(frame_data), Some(mesh_bind_group), Some(camera_bind_group)) =
        (frame_data, mesh_bind_group, camera_bind_group)
    else {
        return;
    };

    let mut encoder = device
        .0
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("3d_render_encoder"),
        });

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3d_render_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &hdr_texture.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_texture.view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(0, &camera_bind_group.0, &[]);
        render_pass.set_bind_group(1, &mesh_bind_group.0, &[]);

        // The single draw call for the entire scene.
        render_pass.draw(0..frame_data.total_indices_to_draw, 0..1);
    }

    queue.0.submit(Some(encoder.finish()));
}
