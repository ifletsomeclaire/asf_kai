use bevy_ecs::prelude::*;
use std::num::NonZeroU64;
use wgpu::util::DeviceExt;

use super::core::{HDR_FORMAT, WgpuDevice, WgpuQueue};
use crate::ecs::{
    camera::Camera,
    model::PerFrameSceneData,
};

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
                    min_binding_size: Some(NonZeroU64::new(std::mem::size_of::<[f32; 16]>() as u64).unwrap()),
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
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
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

#[derive(Resource)]
pub struct CameraBindGroup(pub wgpu::BindGroup);

pub fn update_camera_buffer_system(
    mut commands: Commands,
    device: Res<WgpuDevice>,
    camera: Res<Camera>,
    pipeline: Res<D3Pipeline>,
) {
    let view_proj = camera.build_view_projection_matrix();
    let uniform_buffer = device.0.create_buffer_init(&wgpu::util::BufferInitDescriptor {
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

pub fn render_d3_pipeline_system(
    _device: Res<WgpuDevice>,
    _queue: Res<WgpuQueue>,
    pipeline: Res<D3Pipeline>,
    hdr_texture: Res<super::tonemapping_pass::HdrTexture>,
    scene_data: Option<Res<PerFrameSceneData>>,
    camera_bind_group: Option<Res<CameraBindGroup>>,
) {
    if scene_data.is_none() || camera_bind_group.is_none() {
        return;
    }
    let scene_data = scene_data.unwrap();
    let camera_bind_group = camera_bind_group.unwrap();

    let mut encoder = _device.0.create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(0, &camera_bind_group.0, &[]);
        render_pass.set_bind_group(1, &scene_data.mesh_bind_group, &[]);
        render_pass.draw(0..scene_data.total_vertices_to_draw, 0..1);
    }

    _queue.0.submit(Some(encoder.finish()));
} 