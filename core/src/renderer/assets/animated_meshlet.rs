use glam::Mat4;
use redb::{ReadOnlyTable, ReadableTable};
use std::collections::HashMap;
use types::{AnimatedModel, SkinnedVertex, AABB, Skeleton, Animation};
use wgpu::util::DeviceExt;
use bevy_ecs::prelude::Resource;

use crate::renderer::assets::static_meshlet::{DrawCommand, MeshletDescription};

pub struct ModelMeshlets {
    pub meshlet_indices: Vec<u32>, // Indices into the global meshlets array
    pub texture_id: u32,
}

#[derive(Resource)]
pub struct AnimatedMeshletManager {
    // CPU data
    pub skeletons: HashMap<String, Skeleton>,
    pub animations: HashMap<String, Animation>,
    pub vertices: Vec<SkinnedVertex>,
    pub meshlet_vertex_indices: Vec<u32>,
    pub meshlet_triangle_indices: Vec<u8>,
    pub meshlets: Vec<MeshletDescription>,
    pub transforms: Vec<Mat4>,
    pub draw_commands: Vec<DrawCommand>,
    pub model_meshlets: HashMap<String, Vec<ModelMeshlets>>, // Maps model name to its meshlets

    // GPU resources
    pub vertex_buffer: Option<wgpu::Buffer>,
    pub meshlet_vertex_index_buffer: Option<wgpu::Buffer>,
    pub meshlet_triangle_index_buffer: Option<wgpu::Buffer>,
    pub meshlet_description_buffer: Option<wgpu::Buffer>,
    pub transform_buffer: Option<wgpu::Buffer>,
    pub indirection_buffer: Option<wgpu::Buffer>,

    pub mesh_bind_group_layout: Option<wgpu::BindGroupLayout>,
    pub mesh_bind_group: Option<wgpu::BindGroup>,
    pub instance_bind_group_layout: Option<wgpu::BindGroupLayout>,
    pub instance_bind_group: Option<wgpu::BindGroup>,
}

impl AnimatedMeshletManager {
    pub fn new(
        device: &wgpu::Device,
        model_table: &ReadOnlyTable<&str, &[u8]>,
        animation_table: &ReadOnlyTable<&str, &[u8]>,
        texture_map: &HashMap<String, u32>,
    ) -> Self {
        let mut all_vertices = Vec::new();
        let mut all_meshlet_vertex_indices = Vec::<u32>::new();
        let mut all_meshlet_triangle_indices = Vec::new();
        let mut all_meshlets = Vec::new();
        let mut draw_commands: Vec<DrawCommand> = Vec::new();
        let mut model_meshlets = HashMap::new();

        let mut skeletons = HashMap::new();
        let animations: HashMap<String, Animation> = animation_table
            .iter()
            .unwrap()
            .filter_map(|result| {
                result.ok().and_then(|(name, anim_data)| {
                    bincode::deserialize::<Animation>(anim_data.value())
                        .ok()
                        .map(|anim| (name.value().to_string(), anim))
                })
            })
            .collect();

        let models: Vec<AnimatedModel> = model_table
            .iter()
            .unwrap()
            .filter_map(|result| {
                result.ok().and_then(|(_, model_data)| {
                    bincode::deserialize::<AnimatedModel>(model_data.value()).ok()
                })
            })
            .collect();

        println!("[Asset Loading] Found {} animated models in the database.", models.len());

        let aabbs: Vec<AABB> = models.iter().map(|model| model.aabb).collect();
        let transforms = crate::renderer::assets::layout_models_in_a_row(&aabbs);

        for (transform_id, model) in models.iter().enumerate() {
            println!("[Asset Loading] Loading animated model: '{}'", model.name);
            skeletons.insert(model.name.clone(), model.skeleton.clone());
            
            let mut model_meshlets_list = Vec::new();

            for mesh in &model.meshes {
                if let Some(mesh_meshlets) = &mesh.meshlets {
                    let vertex_base = all_vertices.len() as u32;
                    let meshlet_vertex_index_base = all_meshlet_vertex_indices.len() as u32;

                    all_vertices.extend_from_slice(&mesh.vertices);
                    let remapped_vertex_indices: Vec<u32> = mesh_meshlets
                        .vertices
                        .iter()
                        .map(|&i| vertex_base + i)
                        .collect();
                    all_meshlet_vertex_indices.extend(remapped_vertex_indices);

                    let triangle_base = all_meshlet_triangle_indices.len() as u32;
                    all_meshlet_triangle_indices.extend(&mesh_meshlets.triangles);

                    let texture_id = mesh
                        .texture_name
                        .as_ref()
                        .and_then(|name| texture_map.get(name).copied())
                        .unwrap_or(0);

                    println!(
                        "  -> Mesh '{}' has {} vertices and {} meshlets.",
                        mesh.name,
                        mesh.vertices.len(),
                        mesh.meshlets.as_ref().map_or(0, |m| m.meshlets.len())
                    );

                    let mut meshlet_indices = Vec::new();

                    for m in &mesh_meshlets.meshlets {
                        let desc = MeshletDescription {
                            vertex_list_offset: meshlet_vertex_index_base + m.vertex_offset,
                            triangle_list_offset: triangle_base + m.triangle_offset,
                            triangle_count: m.triangle_count,
                            vertex_count: m.vertex_count,
                        };
                        all_meshlets.push(desc);

                        let meshlet_id = (all_meshlets.len() - 1) as u32;
                        meshlet_indices.push(meshlet_id);

                        let draw_command = DrawCommand {
                            meshlet_id,
                            transform_id: transform_id as u32,
                            texture_id,
                            _padding: 0,
                        };
                        draw_commands.push(draw_command);
                    }

                    model_meshlets_list.push(ModelMeshlets {
                        meshlet_indices,
                        texture_id,
                    });

                }
            }

            println!("  -> Stored {} mesh groups for this model.", model_meshlets_list.len());
            model_meshlets.insert(model.name.clone(), model_meshlets_list);
        }

        println!(
            "[Asset Loading] AnimatedMeshletManager created. Total vertices: {}, Total meshlets: {}",
            all_vertices.len(),
            all_meshlets.len()
        );

        println!("[AnimatedMeshletManager] Total meshlets created: {}", all_meshlets.len());
        println!("[AnimatedMeshletManager] Total draw commands: {}", draw_commands.len());
        if !all_meshlets.is_empty() {
            println!("[AnimatedMeshletManager] First meshlet: vertex_count={}, triangle_count={}", 
                all_meshlets[0].vertex_count,
                all_meshlets[0].triangle_count
            );
        }

        let vertex_buffer =
            Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Skinned Vertex Buffer"),
                contents: bytemuck::cast_slice(&all_vertices),
                usage: wgpu::BufferUsages::STORAGE,
            }));
        let meshlet_vertex_index_buffer =
            Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Animated Meshlet Vertex Index Buffer"),
                contents: bytemuck::cast_slice(&all_meshlet_vertex_indices),
                usage: wgpu::BufferUsages::STORAGE,
            }));
        let meshlet_triangle_index_buffer =
            Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Animated Meshlet Triangle Index Buffer"),
                contents: bytemuck::cast_slice(&all_meshlet_triangle_indices),
                usage: wgpu::BufferUsages::STORAGE,
            }));
        let meshlet_description_buffer =
            Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Animated Meshlet Description Buffer"),
                contents: bytemuck::cast_slice(&all_meshlets),
                usage: wgpu::BufferUsages::STORAGE,
            }));
        let transform_buffer =
            Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Animated Transform Buffer"),
                contents: bytemuck::cast_slice(&transforms),
                usage: wgpu::BufferUsages::STORAGE,
            }));
        let indirection_buffer =
            Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Animated Indirection Buffer"),
                contents: bytemuck::cast_slice(&draw_commands),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT,
            }));

        let mesh_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Animated Mesh Data Bind Group Layout"),
                entries: &[
                    // Static Mesh Data
                    wgpu::BindGroupLayoutEntry { // vertices
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry { // meshlet_vertex_indices
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry { // meshlet_triangle_indices
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry { // meshlet_descriptions
                        binding: 3,
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

        let mesh_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Animated Mesh Bind Group"),
            layout: &mesh_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: vertex_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: meshlet_vertex_index_buffer
                        .as_ref()
                        .unwrap()
                        .as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: meshlet_triangle_index_buffer
                        .as_ref()
                        .unwrap()
                        .as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: meshlet_description_buffer
                        .as_ref()
                        .unwrap()
                        .as_entire_binding(),
                },
            ],
        });

        let instance_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Animated Instance Data Bind Group Layout"),
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
                ],
            });

        let instance_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Animated Instance Bind Group"),
            layout: &instance_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: indirection_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: transform_buffer.as_ref().unwrap().as_entire_binding(),
                },
            ],
        });

        Self {
            skeletons,
            animations,
            vertices: all_vertices,
            meshlet_vertex_indices: all_meshlet_vertex_indices,
            meshlet_triangle_indices: all_meshlet_triangle_indices,
            meshlets: all_meshlets,
            transforms,
            draw_commands,
            model_meshlets, // Initialize the new field

            vertex_buffer,
            meshlet_vertex_index_buffer,
            meshlet_triangle_index_buffer,
            meshlet_description_buffer,
            transform_buffer,
            indirection_buffer,

            mesh_bind_group_layout: Some(mesh_bind_group_layout),
            mesh_bind_group: Some(mesh_bind_group),
            instance_bind_group_layout: Some(instance_bind_group_layout),
            instance_bind_group: Some(instance_bind_group),
        }
    }
} 