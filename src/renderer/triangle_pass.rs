use bevy_ecs::prelude::*;
use std::num::NonZeroU64;
use wgpu::util::DeviceExt;

use super::core::{HDR_FORMAT, WgpuDevice, WgpuQueue};
use crate::ecs::rotation::RotationAngle;

#[derive(Resource)]
pub struct TriangleRenderResources {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group: wgpu::BindGroup,
    pub uniform_buffer: wgpu::Buffer,
}

pub fn clear_hdr_texture_system(
    device: Res<WgpuDevice>,
    queue: Res<WgpuQueue>,
    hdr_texture: Res<super::tonemapping_pass::HdrTexture>,
) {
    let device = &device.0;
    let queue = &queue.0;

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("clear_hdr_encoder"),
    });

    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("clear_hdr_pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &hdr_texture.view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        occlusion_query_set: None,
        timestamp_writes: None,
    });

    queue.submit(Some(encoder.finish()));
}

pub fn setup_triangle_pass_system(mut commands: Commands, device_res: Res<WgpuDevice>) {
    let device = &device_res.0;
    let triangle_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("custom3d"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/shader.wgsl").into()),
    });

    let triangle_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("custom3d"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(4),
                },
                count: None,
            }],
        });

    let triangle_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("custom3d"),
        bind_group_layouts: &[&triangle_bind_group_layout],
        push_constant_ranges: &[],
    });

    let triangle_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("custom3d"),
        layout: Some(&triangle_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &triangle_shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &triangle_shader,
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

    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("custom3d"),
        contents: bytemuck::cast_slice(&[0.0_f32]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
    });

    let triangle_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("custom3d"),
        layout: &triangle_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform_buffer.as_entire_binding(),
        }],
    });

    commands.insert_resource(TriangleRenderResources {
        pipeline: triangle_pipeline,
        bind_group: triangle_bind_group,
        uniform_buffer,
    });
}

pub fn render_triangle_system(
    device: Res<WgpuDevice>,
    queue: Res<WgpuQueue>,
    triangle_resources: Res<TriangleRenderResources>,
    hdr_texture: Res<super::tonemapping_pass::HdrTexture>,
    rotation: Res<RotationAngle>,
) {
    let device = &device.0;
    let queue = &queue.0;

    queue.write_buffer(
        &triangle_resources.uniform_buffer,
        0,
        bytemuck::cast_slice(&[rotation.0]),
    );

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("hdr_render_encoder"),
    });

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hdr_render_pass"),
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

        render_pass.set_pipeline(&triangle_resources.pipeline);
        render_pass.set_bind_group(0, &triangle_resources.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    queue.submit(Some(encoder.finish()));
}
