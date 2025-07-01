use bevy_ecs::prelude::*;
use bytemuck::{Pod, Zeroable};
use redb::{Database, TableDefinition};
use std::collections::HashMap;
use types::Model as TypesModel;
use wgpu::util::DeviceExt;
use bevy_transform::components::GlobalTransform;

use crate::renderer::{core::WgpuDevice, d3_pipeline::D3Pipeline};

const MODEL_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("models");

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 4],
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

#[derive(Component, Clone, Copy)]
pub struct Model {
    pub mesh_id: u32,
}

pub struct ModelInfo {
    pub name: String,
    pub mesh_id: u32,
}

#[derive(Resource, Default)]
pub struct AvailableModels {
    pub models: Vec<ModelInfo>,
}

#[derive(Resource)]
pub struct StaticModelData {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub mesh_description_buffer: wgpu::Buffer,
    pub mesh_descriptions: Vec<MeshDescription>,
    pub mesh_id_remap: HashMap<String, u32>,
}

#[derive(Resource)]
pub struct PerFrameSceneData {
    pub mesh_bind_group: wgpu::BindGroup,
    pub total_vertices_to_draw: u32,
}

pub fn load_models_from_db_system(
    mut commands: Commands,
    device: Res<WgpuDevice>,
) -> Result<(), anyhow::Error> {
    let mut all_vertices: Vec<Vertex> = Vec::new();
    let mut all_indices: Vec<u32> = Vec::new();
    let mut mesh_descriptions = Vec::new();
    let mut mesh_id_remap: HashMap<String, u32> = HashMap::new();
    let mut available_models = AvailableModels::default();

    let db = Database::open("assets/models.redb")?;
    let read_txn = db.begin_read()?;
    let table = read_txn.open_table(MODEL_TABLE)?;

    log::info!("Loading models from database...");
    let mut count = 0;
    for result in table.range::<&str>(..)? {
        count += 1;
        let (key, value) = result?;
        let model_name = key.value().to_string();
        log::info!("  - Loading model: {}", model_name);
        let model_data = value.value();
        let model: TypesModel = bincode::deserialize(model_data)?;

        for (mesh_index, mesh) in model.meshes.iter().enumerate() {
            let base_vertex = all_vertices.len() as i32;
            let first_index = all_indices.len() as u32;

            let vertices: Vec<Vertex> = mesh
                .vertices
                .iter()
                .map(|v| Vertex {
                    position: [v.x, v.y, v.z, 1.0],
                })
                .collect();

            let index_count = mesh.indices.len() as u32;

            all_vertices.extend(vertices);
            all_indices.extend(mesh.indices.clone());

            let new_mesh_id = mesh_descriptions.len() as u32;
            let unique_model_name = if model.meshes.len() > 1 {
                format!("{}-{}", model_name, mesh_index)
            } else {
                model_name.clone()
            };

            mesh_id_remap.insert(unique_model_name.clone(), new_mesh_id);
            available_models.models.push(ModelInfo {
                name: unique_model_name,
                mesh_id: new_mesh_id,
            });

            mesh_descriptions.push(MeshDescription {
                index_count,
                first_index,
                base_vertex,
                _padding: 0,
            });
        }
    }
    log::info!("Finished loading {} models.", count);

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

    commands.insert_resource(StaticModelData {
        vertex_buffer,
        index_buffer,
        mesh_description_buffer,
        mesh_descriptions,
        mesh_id_remap,
    });

    commands.insert_resource(available_models);

    Ok(())
}

pub fn prepare_scene_data_system(
    mut commands: Commands,
    device: Res<WgpuDevice>,
    pipeline: Res<D3Pipeline>,
    static_model_data: Res<StaticModelData>,
    available_models: Res<AvailableModels>,
    query: Query<(&Model, &GlobalTransform)>,
) {
    let instances: Vec<Instance> = query
        .iter()
        .filter_map(|(model, transform)| {
            let model_info = available_models.models.get(model.mesh_id as usize)?;
            static_model_data
                .mesh_id_remap
                .get(&model_info.name)
                .map(|remapped_id| Instance {
                    model_matrix: transform.compute_matrix().to_cols_array_2d(),
                    mesh_id: *remapped_id,
                    _padding: [0; 3],
                })
        })
        .collect();

    if instances.is_empty() {
        commands.remove_resource::<PerFrameSceneData>();
        return;
    }

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

    let mesh_bind_group = device.0.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Mesh Bind Group"),
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

    commands.insert_resource(PerFrameSceneData {
        mesh_bind_group,
        total_vertices_to_draw,
    });
} 