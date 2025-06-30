use bevy_ecs::prelude::*;
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use russimp::scene::{PostProcess, Scene};
use std::collections::HashMap;
use wgpu::util::DeviceExt;
use bevy_transform::components::{GlobalTransform, Transform};

use crate::renderer::{core::WgpuDevice, d3_pipeline::D3Pipeline};

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 4],
    // pub normal: [f32; 3],
    // pub uv: [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MeshDescription {
    index_count: u32,
    first_index: u32,
    base_vertex: i32,
    _padding: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Instance {
    model_matrix: [[f32; 4]; 4],
    mesh_id: u32,
    _padding: [u32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct InstanceLookup {
    instance_id: u32,
    first_vertex_of_instance: u32,
}

#[derive(Component)]
pub struct Model {
    pub mesh_id: u32, // The *original* mesh_id from the file
}

#[derive(Resource)]
pub struct StaticModelData {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub mesh_description_buffer: wgpu::Buffer,
    pub mesh_descriptions: Vec<MeshDescription>, // We need this to calculate instance lookups
    pub mesh_id_remap: HashMap<u32, u32>,
}

#[derive(Resource)]
pub struct PerFrameSceneData {
    pub mesh_bind_group: wgpu::BindGroup,
    pub total_vertices_to_draw: u32,
}

pub fn load_static_models_system(mut commands: Commands, device: Res<WgpuDevice>) {
    // --- 1. Load Unique Meshes ---
    let mut all_vertices: Vec<Vertex> = Vec::new();
    let mut all_indices: Vec<u32> = Vec::new();
    let mut unique_meshes = HashMap::new(); // K: original mesh_index, V: (index_count, first_index, base_vertex)

    // For this example, we load one model.
    let scene = Scene::from_file(
        "assets/models/cube.gltf",
        vec![
            PostProcess::Triangulate,
            PostProcess::JoinIdenticalVertices,
        ],
    )
    .unwrap();

    // We no longer create instances here. We just gather mesh data.
    for (mesh_id, mesh) in scene.meshes.iter().enumerate() {
        if !unique_meshes.contains_key(&mesh_id) {
            log::info!(
                "Loading new unique mesh: {} ({} vertices, {} faces)",
                mesh.name,
                mesh.vertices.len(),
                mesh.faces.len()
            );

            let base_vertex = all_vertices.len() as i32;
            let first_index = all_indices.len() as u32;

            let vertices: Vec<Vertex> = mesh
                .vertices
                .iter()
                .map(|v| Vertex {
                    position: [v.x, v.y, v.z, 1.0],
                })
                .collect();

            let indices: Vec<u32> = mesh.faces.iter().flat_map(|f| f.0.clone()).collect();
            let index_count = indices.len() as u32;

            all_vertices.extend(vertices);
            all_indices.extend(indices);
            unique_meshes.insert(mesh_id, (index_count, first_index, base_vertex));
        }
    }

    // --- 2. Create MeshDescription Vec from Unique Meshes ---
    let mut mesh_id_remap: HashMap<u32, u32> = HashMap::new();
    let mut mesh_descriptions = Vec::new();
    // Sort by original_mesh_id to have a deterministic order
    let mut sorted_unique_meshes: Vec<_> = unique_meshes.iter().collect();
    sorted_unique_meshes.sort_by_key(|(k, _)| **k);

    for (original_mesh_id, (index_count, first_index, base_vertex)) in sorted_unique_meshes {
        let new_id = mesh_descriptions.len() as u32;
        mesh_id_remap.insert(*original_mesh_id as u32, new_id);
        mesh_descriptions.push(MeshDescription {
            index_count: *index_count,
            first_index: *first_index,
            base_vertex: *base_vertex,
            _padding: 0,
        });
    }

    log::info!("Total unique meshes loaded: {}", mesh_descriptions.len());

    // --- 3. Create GPU Buffers for static data ---
    let vertex_buffer = device.0.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Global Vertex Buffer"),
        contents: bytemuck::cast_slice(&all_vertices),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let index_buffer = device.0.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Global Index Buffer"),
        contents: bytemuck::cast_slice(&all_indices),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let mesh_description_buffer = device.0.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Mesh Description Buffer"),
        contents: bytemuck::cast_slice(&mesh_descriptions),
        usage: wgpu::BufferUsages::STORAGE,
    });

    // --- 4. Store static data in a resource ---
    commands.insert_resource(StaticModelData {
        vertex_buffer,
        index_buffer,
        mesh_description_buffer,
        mesh_descriptions,
        mesh_id_remap,
    });

    // --- 5. Spawn entities that will be rendered ---
    // This part should probably be in another system, but for now let's spawn a grid of cubes here.
    // This is like "scene setup".
    let cube_mesh_id = 0; // In cube.gltf, there's only one mesh, so its id is 0.

    let grid_size = 10;
    for i in 0..grid_size {
        for j in 0..grid_size {
            for k in 0..grid_size {
                let translation = Vec3::new(
                    i as f32 * 2.0,
                    j as f32 * 2.0,
                    k as f32 * 2.0,
                );
                commands.spawn((
                    Model {
                        mesh_id: cube_mesh_id,
                    },
                    Transform::from_translation(translation),
                ));
            }
        }
    }
}

pub fn prepare_scene_data_system(
    mut commands: Commands,
    device: Res<WgpuDevice>,
    pipeline: Res<D3Pipeline>,
    static_model_data: Res<StaticModelData>,
    query: Query<(&Model, &GlobalTransform)>,
) {
    let instances: Vec<Instance> = query
        .iter()
        .map(|(model, transform)| {
            let remapped_mesh_id = static_model_data
                .mesh_id_remap
                .get(&model.mesh_id)
                .unwrap();
            Instance {
                model_matrix: transform.compute_matrix().to_cols_array_2d(),
                mesh_id: *remapped_mesh_id,
                _padding: [0; 3],
            }
        })
        .collect();

    if instances.is_empty() {
        // If there are no instances, we can remove any existing scene data
        // to prevent rendering from the previous frame.
        commands.remove_resource::<PerFrameSceneData>();
        return;
    }

    // --- Create InstanceLookup Buffer ---
    let mut instance_lookups = Vec::new();
    let mut total_vertices_to_draw = 0;
    for (instance_id, instance) in instances.iter().enumerate() {
        let mesh_desc = &static_model_data.mesh_descriptions[instance.mesh_id as usize];
        let first_vertex_of_instance = total_vertices_to_draw;
        for _ in 0..mesh_desc.index_count {
            instance_lookups.push(InstanceLookup {
                instance_id: instance_id as u32,
                first_vertex_of_instance,
            });
        }
        total_vertices_to_draw += mesh_desc.index_count;
    }

    if total_vertices_to_draw == 0 {
        log::warn!("No vertices to draw for the current instances.");
        commands.remove_resource::<PerFrameSceneData>();
        return;
    }

    log::info!(
        "Per frame - Instances: {}, Vertices to draw: {}",
        instances.len(),
        total_vertices_to_draw
    );

    // --- Create GPU Buffers for per-frame data ---
    let instance_buffer = device.0.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Instance Buffer"),
        contents: bytemuck::cast_slice(&instances),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let instance_lookup_buffer = device.0.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Instance Lookup Buffer"),
        contents: bytemuck::cast_slice(&instance_lookups),
        usage: wgpu::BufferUsages::STORAGE,
    });

    // --- Create Bind Group ---
    let mesh_bind_group = device.0.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("mesh_bind_group (per-frame)"),
        layout: &pipeline.mesh_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: static_model_data.vertex_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: static_model_data.index_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: static_model_data
                    .mesh_description_buffer
                    .as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: instance_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: instance_lookup_buffer.as_entire_binding(),
            },
        ],
    });

    // --- Insert resource for rendering system ---
    commands.insert_resource(PerFrameSceneData {
        mesh_bind_group,
        total_vertices_to_draw,
    });
} 