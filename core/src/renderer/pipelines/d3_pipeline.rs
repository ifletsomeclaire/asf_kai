use bevy_ecs::{
    prelude::{Res,Query},
};
use bevy_transform::components::GlobalTransform;
use wgpu::{include_wgsl, util::DeviceExt, PipelineCompilationOptions};

use crate::{
    ecs::camera::Camera,
    renderer::{
        assets::AssetServer,
        core::{WgpuDevice,  WgpuQueue},
        pipelines::tonemapping::{DepthTexture, HdrTexture, IdTexture},
    },
};

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub struct D3Pipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub camera_bind_group_layout: wgpu::BindGroupLayout,
}

impl D3Pipeline {
    pub fn new(
        device: &wgpu::Device,
        asset_server: &AssetServer,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(include_wgsl!("../../shaders/d3.wgsl"));

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
                label: Some("camera_bind_group_layout"),
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("D3 Pipeline Layout"),
            bind_group_layouts: &[
                &camera_bind_group_layout,
                asset_server.meshlet_manager.mesh_bind_group_layout.as_ref().unwrap(),
                asset_server.meshlet_manager.instance_bind_group_layout.as_ref().unwrap(),
                asset_server.texture_bind_group_layout.as_ref().unwrap(),
            ],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("D3 Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main".into(),
                buffers: &[], // Vertex data is pulled from storage buffers
                compilation_options: PipelineCompilationOptions::default(),
            },
            cache: None,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main".into(),
                targets: &[
                    Some(wgpu::ColorTargetState {  // Color output
                        format: surface_format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {  // Entity ID output
                        format: wgpu::TextureFormat::R32Uint,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
                compilation_options: PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                // cull_mode: Some(wgpu::Face::Back),
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        Self {
            pipeline,
            camera_bind_group_layout,
        }
    }
}



pub fn render_d3_pipeline_system(
    device: Res<WgpuDevice>,
    queue: Res<WgpuQueue>,
    asset_server: Res<AssetServer>,
    depth_texture: Res<DepthTexture>,
    hdr_texture: Res<HdrTexture>,
    id_texture: Res<IdTexture>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
) {
    // If the mesh bind group doesn't exist on the asset server, it's because
    // no renderable meshlets were loaded. In this case, there is nothing
    // for this pipeline to draw, so we return early.
    if asset_server.meshlet_manager.mesh_bind_group.is_none() {
        return;
    }

    // Try to get the camera's data. If it doesn't exist, we can't render.
    let Ok((camera, transform)) = camera_query.single() else {
        return;
    };

    let pipeline = D3Pipeline::new(
        &device,
        &asset_server,
        wgpu::TextureFormat::Rgba16Float, // HDR format
    );

    let view = transform.compute_matrix().inverse();
    let proj = camera.projection_matrix();
    let view_proj = proj * view;

    let camera_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("camera_uniform_buffer"),
        contents: bytemuck::cast_slice(view_proj.as_ref()),
        usage: wgpu::BufferUsages::UNIFORM,
    });

    let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("camera_bind_group"),
        layout: &pipeline.camera_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: camera_uniform_buffer.as_entire_binding(),
        }],
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Render Encoder"),
    });

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("D3 Render Pass"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {  // Color
                    view: &hdr_texture.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load, 
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {  // Entity IDs
                    view: &id_texture.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,  // Changed from Clear to Load
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_texture.view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,  // Changed from Clear to Load
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(0, &camera_bind_group, &[]);
        render_pass.set_bind_group(1, asset_server.meshlet_manager.mesh_bind_group.as_ref().unwrap(), &[]);
        render_pass.set_bind_group(
            2,
            asset_server.meshlet_manager.instance_bind_group.as_ref().unwrap(),
            &[],
        );
        render_pass.set_bind_group(3, asset_server.texture_bind_group.as_ref().unwrap(), &[]);
        
        // Draw all the meshlets.
        render_pass.draw(
            0..(128 * 3), // Max triangles * 3
            0..asset_server.meshlet_manager.draw_commands.len() as u32,
        );
    }

    queue.submit(Some(encoder.finish()));
}
