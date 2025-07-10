use bevy_ecs::{
    prelude::{Query, Res, Resource, With},
};
use bevy_transform::components::GlobalTransform;
use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::{include_wgsl, util::DeviceExt, PipelineCompilationOptions};

use crate::{
    ecs::{
        animation::{AnimatedInstance, BoneMatrices},
        camera::Camera,
    },
    renderer::{
        assets::AssetServer,
        core::{WgpuDevice, WgpuQueue},
        pipelines::tonemapping::{DepthTexture, HdrTexture},
    },
};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct AnimatedDrawCommand {
    meshlet_id: u32,
    bone_set_id: u32, // An index pointing to the start of a block of 256 matrices
    transform_id: u32,
    texture_id: u32,
}

#[derive(Resource)]
pub struct D3AnimatedPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub camera_bind_group_layout: wgpu::BindGroupLayout,
    pub instance_data_bind_group_layout: wgpu::BindGroupLayout,
}

#[derive(Resource)]
pub struct CameraUniformBuffer(pub wgpu::Buffer);

impl D3AnimatedPipeline {
    pub fn new(device: &wgpu::Device, asset_server: &AssetServer, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(include_wgsl!("../../shaders/d3_animated.wgsl"));

        // @group(0)
        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("d3_animated_camera_bgl"),
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
            });

        // @group(2) - Per-draw instance data
        let instance_data_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("d3_animated_instance_data_bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0, // Indirection Buffer (Draw Commands)
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1, // Bone Matrices
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2, // Transform Buffer
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
            label: Some("D3 Animated Pipeline Layout"),
            bind_group_layouts: &[
                &camera_bind_group_layout,
                asset_server
                    .animated_meshlet_manager
                    .mesh_bind_group_layout
                    .as_ref()
                    .unwrap(),
                &instance_data_bind_group_layout,
                asset_server.texture_bind_group_layout.as_ref().unwrap(),
            ],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("D3 Animated Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main".into(),
                buffers: &[],
                compilation_options: PipelineCompilationOptions::default(),
            },
            cache: None,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main".into(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, //None for now, can be Some(wgpu::Face::Back)
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
            instance_data_bind_group_layout,
        }
    }
}

pub fn render_d3_animated_pipeline_system(
    device: Res<WgpuDevice>,
    queue: Res<WgpuQueue>,
    pipeline: Res<D3AnimatedPipeline>,
    asset_server: Res<AssetServer>,
    depth_texture: Res<DepthTexture>,
    hdr_texture: Res<HdrTexture>,
    camera_buffer: Res<CameraUniformBuffer>,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera>>,
    animated_instance_query: Query<(&AnimatedInstance, &BoneMatrices, &GlobalTransform)>,
) {
    let animated_meshlet_manager = &asset_server.animated_meshlet_manager;

    // Exit if there are no loaded animated models or instances to draw.
    if animated_meshlet_manager.mesh_bind_group.is_none() || animated_instance_query.is_empty() {
        return;
    }

    let Ok((camera, camera_transform)) = camera_query.single() else {
        return;
    };

    // --- 1. Prepare Per-Frame Data (Dynamically) ---
    let mut draw_commands = Vec::new();
    let mut all_bone_matrices = Vec::new();
    let mut all_transforms = Vec::new();

    // Track bone matrix offsets for each instance
    let mut bone_matrix_offsets = Vec::new();
    
    // Iterate over the entities that are actually in the scene right now.
    for (instance_index, (instance, bone_matrices, transform)) in
        animated_instance_query.iter().enumerate()
    {
        // Add the current entity's transform to our list for this frame.
        all_transforms.push(transform.compute_matrix());
        let transform_id = (all_transforms.len() - 1) as u32;

        // Track where this instance's bone matrices start
        let bone_matrix_offset = all_bone_matrices.len() as u32;
        bone_matrix_offsets.push(bone_matrix_offset);
        
        // Add this instance's bone matrices
        all_bone_matrices.extend_from_slice(&bone_matrices.matrices);

        // Find the meshlets associated with this instance's model name.
        if let Some(model_meshlets_list) = animated_meshlet_manager.model_meshlets.get(&instance.model_name) {
            for model_meshlets in model_meshlets_list {
                for &meshlet_id in &model_meshlets.meshlet_indices {
                    // Create a new draw command with the correct, frame-specific IDs.
                    draw_commands.push(AnimatedDrawCommand {
                        meshlet_id,
                        bone_set_id: bone_matrix_offset, // Use actual offset, not instance index
                        transform_id,
                        texture_id: model_meshlets.texture_id,
                    });
                }
            }
        }
    }

    if draw_commands.is_empty() {
        return; // Nothing to draw this frame.
    }

    println!("[Animated Render] Drawing {} commands", draw_commands.len());
    if !draw_commands.is_empty() {
        println!("[Animated Render] First draw command: meshlet_id={}, bone_set_id={}, transform_id={}, texture_id={}", 
            draw_commands[0].meshlet_id,
            draw_commands[0].bone_set_id, 
            draw_commands[0].transform_id,
            draw_commands[0].texture_id
        );
    }
    println!("[Animated Render] Total bone matrices: {}", all_bone_matrices.len());
    println!("[Animated Render] Total transforms: {}", all_transforms.len());

    // --- 2. Update GPU Buffers ---
    let view_proj = camera.projection_matrix() * camera_transform.compute_matrix().inverse();
    queue.write_buffer(
        &camera_buffer.0,
        0,
        bytemuck::cast_slice(view_proj.as_ref()),
    );

    // Create new buffers for this frame's data. This is necessary because the data changes every frame.
    let bone_matrix_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("per_frame_bone_matrix_buffer"),
        contents: bytemuck::cast_slice(&all_bone_matrices),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let transform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("per_frame_animated_transform_buffer"),
        contents: bytemuck::cast_slice(&all_transforms),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let indirection_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("per_frame_animated_indirection_buffer"),
        contents: bytemuck::cast_slice(&draw_commands),
        usage: wgpu::BufferUsages::STORAGE,
    });

    // --- 3. Create Bind Groups ---
    let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("d3_animated_camera_bg"),
        layout: &pipeline.camera_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: camera_buffer.0.as_entire_binding(),
        }],
    });

    let instance_data_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("d3_animated_instance_data_bg"),
        layout: &pipeline.instance_data_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: indirection_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: bone_matrix_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: transform_buffer.as_entire_binding(),
            },
        ],
    });

    // --- 4. Render ---
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Animated Render Encoder"),
    });

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("D3 Animated Render Pass"),
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
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        render_pass.set_pipeline(&pipeline.pipeline);

        render_pass.set_bind_group(0, &camera_bind_group, &[]);
        render_pass.set_bind_group(1, animated_meshlet_manager.mesh_bind_group.as_ref().unwrap(), &[]);
        render_pass.set_bind_group(2, &instance_data_bind_group, &[]);
        render_pass.set_bind_group(3, asset_server.texture_bind_group.as_ref().unwrap(), &[]);

        // Draw the commands generated for this frame.
        render_pass.draw(0..(128 * 3), 0..draw_commands.len() as u32);
    }

    queue.submit(Some(encoder.finish()));
}
