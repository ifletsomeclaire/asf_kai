use bevy_ecs::{
    prelude::{Query, Res, Resource, With, Entity},
};
use bevy_transform::components::GlobalTransform;
use glam::Mat4;
use wgpu::{include_wgsl, util::DeviceExt, PipelineCompilationOptions};
use log;

use crate::{
    ecs::{
        animation::{AnimatedInstance, BoneMatrices},
        camera::Camera,
    },
    renderer::{
        assets::{
            animated_meshlet::AnimatedDrawCommand,
            AssetServer,
        },
        core::{WgpuDevice, WgpuQueue},
        pipelines::tonemapping::{DepthTexture, HdrTexture, IdTexture},
    },
};

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
                        binding: 1, // Bone Matrices (now include world transform)
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
    // Add ID texture to the system parameters
    id_texture: Res<IdTexture>,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera>>,
    animated_instance_query: Query<(Entity, &AnimatedInstance, &BoneMatrices, &GlobalTransform)>,
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

    // Track bone matrix offsets for each instance
    let mut bone_matrix_offsets = Vec::new();
    
    // Iterate over the entities that are actually in the scene right now.
    for (instance_index, (entity, instance, bone_matrices, _transform)) in
        animated_instance_query.iter().enumerate()
    {
        // Note: We no longer need to track transforms separately since bone matrices include world transform

        // Track where this instance's bone matrices start
        let bone_matrix_offset = all_bone_matrices.len() as u32;
        log::debug!("[Animated Render] Instance[{}] '{}': bone_matrix_offset = {} (adding {} matrices)", 
            instance_index, instance.model_name, bone_matrix_offset, bone_matrices.matrices.len());
        
        // Log bone matrix details for first few bones
        if instance_index < 2 {
            for (i, matrix) in bone_matrices.matrices.iter().take(3).enumerate() {
                let pos = matrix.transform_point3(glam::Vec3::ZERO);
                log::debug!("[Animated Render]   Bone {} matrix: pos=[{:.3}, {:.3}, {:.3}]", 
                    i, pos.x, pos.y, pos.z);
            }
        }
        
        bone_matrix_offsets.push(bone_matrix_offset);
        
        // Add this instance's bone matrices
        all_bone_matrices.extend_from_slice(&bone_matrices.matrices);

        // Find the meshlets associated with this instance's model name.
        if let Some(model_meshlets_list) = animated_meshlet_manager.model_meshlets.get(&instance.model_name) {
            log::debug!("[Animated Render] Creating draw commands for model '{}' (instance {}):", instance.model_name, instance_index);
            for (meshlet_group_idx, model_meshlets) in model_meshlets_list.iter().enumerate() {
                log::debug!("[Animated Render]   Meshlet group {}: {} meshlets, texture_id={}", 
                    meshlet_group_idx, model_meshlets.meshlet_indices.len(), model_meshlets.texture_id);
                for (meshlet_idx, &meshlet_id) in model_meshlets.meshlet_indices.iter().enumerate() {
                    if meshlet_idx < 3 { // Only print first 3 for brevity
                        log::debug!("[Animated Render]     Meshlet[{}]: id={}, bone_set_id={}", 
                            meshlet_idx, meshlet_id, instance_index);
                    }
                    // Create a new draw command with the correct, frame-specific IDs.
                    draw_commands.push(AnimatedDrawCommand {
                        meshlet_id,
                        bone_set_id: instance_index as u32, // Use instance index as bone_set_id for proper indexing
                        transform_id: instance_index as u32, // Use instance index as transform_id for positioning
                        entity_id: entity.index(),  // Direct Entity ID
                        texture_id: model_meshlets.texture_id,
                    });
                }
                if model_meshlets.meshlet_indices.len() > 3 {
                    log::debug!("[Animated Render]     ... and {} more meshlets", model_meshlets.meshlet_indices.len() - 3);
                }
            }
        } else {
            log::warn!("[Animated Render] WARNING: No meshlets found for model '{}'", instance.model_name);
        }
    }

    if draw_commands.is_empty() {
        return; // Nothing to draw this frame.
    }

    log::info!("[Animated Render] Drawing {} commands", draw_commands.len());
    if !draw_commands.is_empty() {
        log::debug!("[Animated Render] First draw command: meshlet_id={}, bone_set_id={}, transform_id={}, entity_id={}, texture_id={}", 
            draw_commands[0].meshlet_id, draw_commands[0].bone_set_id, 
            draw_commands[0].transform_id, draw_commands[0].entity_id, draw_commands[0].texture_id);
        log::debug!("[Animated Render]   Last draw command: meshlet_id={}, bone_set_id={}, transform_id={}, entity_id={}, texture_id={}", 
            draw_commands[draw_commands.len()-1].meshlet_id, draw_commands[draw_commands.len()-1].bone_set_id, 
            draw_commands[draw_commands.len()-1].transform_id, draw_commands[draw_commands.len()-1].entity_id, draw_commands[draw_commands.len()-1].texture_id);
    }
    
    log::debug!("[Animated Render] Total bone matrices: {}", all_bone_matrices.len());
    
    log::debug!("[Animated Render] Bone matrix layout:");
    let mut offset = 0;
    for (i, (_entity, instance, bone_matrices, _)) in animated_instance_query.iter().enumerate() {
        log::debug!("[Animated Render]   Instance[{}] '{}': offset={}, count={}", 
            i, instance.model_name, offset, bone_matrices.matrices.len());
        offset += bone_matrices.matrices.len();
    }
    
    log::debug!("[Animated Render] Draw command summary:");
    log::debug!("[Animated Render]   Total draw commands: {}", draw_commands.len());
    if !draw_commands.is_empty() {
        log::debug!("[Animated Render]   First draw command: meshlet_id={}, bone_set_id={}, transform_id={}, entity_id={}, texture_id={}", 
            draw_commands[0].meshlet_id, draw_commands[0].bone_set_id, 
            draw_commands[0].transform_id, draw_commands[0].entity_id, draw_commands[0].texture_id);
        log::debug!("[Animated Render]   Last draw command: meshlet_id={}, bone_set_id={}, transform_id={}, entity_id={}, texture_id={}", 
            draw_commands[draw_commands.len()-1].meshlet_id, draw_commands[draw_commands.len()-1].bone_set_id, 
            draw_commands[draw_commands.len()-1].transform_id, draw_commands[draw_commands.len()-1].entity_id, draw_commands[draw_commands.len()-1].texture_id);
    }
    
    // Log GPU buffer creation details
    log::debug!("[Animated Render] Creating GPU buffers:");
    log::debug!("[Animated Render]   Bone matrix buffer: {} matrices, {} bytes", 
        all_bone_matrices.len(), all_bone_matrices.len() * std::mem::size_of::<Mat4>());
    log::debug!("[Animated Render]   Indirection buffer: {} commands, {} bytes", 
        draw_commands.len(), draw_commands.len() * std::mem::size_of::<AnimatedDrawCommand>());

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

    // Note: We no longer need a separate transform buffer since bone matrices include world transform

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
            // Note: Binding 2 (transform_buffer) is no longer used since bone matrices include world transform
        ],
    });

    // --- 4. Render ---
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Animated Render Encoder"),
    });

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("D3 Animated Render Pass"),
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

        render_pass.set_pipeline(&pipeline.pipeline);

        render_pass.set_bind_group(0, &camera_bind_group, &[]);
        render_pass.set_bind_group(1, animated_meshlet_manager.mesh_bind_group.as_ref().unwrap(), &[]);
        render_pass.set_bind_group(2, &instance_data_bind_group, &[]);
        render_pass.set_bind_group(3, asset_server.texture_bind_group.as_ref().unwrap(), &[]);

        // Draw the commands generated for this frame.
        render_pass.draw(0..(128 * 3), 0..draw_commands.len() as u32);
    }

    queue.submit(Some(encoder.finish()));
    
    // GPU picking integration
    // if let Some(mut gpu_picking) = world.get_resource_mut::<gpu_picking::GPUPicking>() {
    //     let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
    //         label: Some("GPU Picking Encoder"),
    //     });
    //     gpu_picking.pick(&queue, &mut encoder);
    //     queue.submit(Some(encoder.finish()));
    // }
}
