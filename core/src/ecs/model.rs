use bevy_ecs::prelude::*;
use bytemuck::{Pod, Zeroable};
use redb::{Database, TableDefinition};
use std::env;
use std::path::PathBuf;
use types::Model as TypesModel;
use wgpu::util::DeviceExt;
use bevy_transform::components::GlobalTransform;
use indexmap::IndexMap;
use crate::renderer::assets::{AssetServer, Handle, GpuMesh};
use crate::renderer::{
    core::{WgpuDevice, WgpuQueue},
    d3_pipeline::D3Pipeline,
};
use log::{info, warn};
use glam::Vec3;
use bevy_transform::components::Transform;
use std::collections::HashMap;

const MODEL_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("models");

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MeshDescription {
    pub index_count: u32,
    pub first_index: u32,
    pub base_vertex: i32,
    pub _padding: u32,
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
    local_vertex_index: u32,
}

#[derive(Component, Clone)]
pub struct Model {
    pub mesh_name: String,
}

pub struct ModelInfo {
    pub name: String,
}

#[derive(Resource, Default)]
pub struct AvailableModels {
    pub models: Vec<ModelInfo>,
}

#[derive(Resource)]
pub struct PerFrameSceneData {
    pub mesh_bind_group: wgpu::BindGroup,
    pub total_vertices_to_draw: u32,
}

pub fn load_models_from_db_system(
    mut commands: Commands,
    mut asset_server: ResMut<AssetServer>,
    queue: Res<WgpuQueue>,
) -> Result<(), anyhow::Error> {
    println!("--- Running load_models_from_db_system ---");
    let mut available_models = AvailableModels::default();

    let mut workspace_root = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    workspace_root.pop(); // Go up to the workspace root from the crate root
    let db_path = workspace_root.join("assets/models.redb");

    let db = Database::open(&db_path)?;
    let read_txn = db.begin_read()?;
    let table = read_txn.open_table(MODEL_TABLE)?;

    // Load all models and populate AvailableModels
    for result in table.range::<&str>(..)? {
        let (key, value) = result?;
        let filename = key.value().to_string();
        let model_data = value.value();
        let model: TypesModel = bincode::deserialize(model_data)?;

        // A model can have multiple meshes
        for cpu_mesh in &model.meshes {
            let handle = asset_server.load_mesh(cpu_mesh, &queue.0).unwrap();
            asset_server.register_mesh_handle(&cpu_mesh.name, handle);
            available_models.models.push(ModelInfo { name: cpu_mesh.name.clone() });
        }

        if filename == "cube.gltf" {
            if let Some(mesh) = model.meshes.first() {
                commands.spawn((
                    Model {
                        mesh_name: mesh.name.clone(),
                    },
                    Transform::default(),
                    GlobalTransform::default(),
                ));
            }
        }
    }
    
    println!("--- Finished load_models_from_db_system ---");
    commands.insert_resource(available_models);

    Ok(())
}

pub fn prepare_scene_data_system(
    mut commands: Commands,
    device: Res<WgpuDevice>,
    asset_server: Res<AssetServer>,
    query: Query<(&Model, &GlobalTransform)>,
    pipeline: Res<D3Pipeline>,
) {
    if query.is_empty() {
        commands.remove_resource::<PerFrameSceneData>();
        return;
    }

    let mut mesh_descriptions = Vec::new();
    let mut handle_to_mesh_id = HashMap::new();
    let mut instances = Vec::new();

    for (model, transform) in query.iter() {
        if let Some(handle) = asset_server.get_mesh_handle(&model.mesh_name) {
            let mesh_id = *handle_to_mesh_id.entry(handle.clone()).or_insert_with(|| {
                let id = mesh_descriptions.len() as u32;
                if let Some(gpu_mesh) = asset_server.get_gpu_mesh(handle) {
                    mesh_descriptions.push(MeshDescription {
                        index_count: gpu_mesh.index_count,
                        first_index: (gpu_mesh.index_buffer_offset / std::mem::size_of::<u32>() as u64) as u32,
                        base_vertex: (gpu_mesh.vertex_buffer_offset / std::mem::size_of::<Vertex>() as u64) as i32,
                        _padding: 0,
                    });
                }
                id
            });

            instances.push(Instance {
                model_matrix: transform.compute_matrix().to_cols_array_2d(),
                mesh_id,
                _padding: [0; 3],
            });
        }
    }

    if instances.is_empty() {
        commands.remove_resource::<PerFrameSceneData>();
        return;
    }

    let mut instance_lookups = Vec::new();
    let mut total_vertices_to_draw = 0;
    for (instance_id, instance) in instances.iter().enumerate() {
        if let Some(mesh_desc) = mesh_descriptions.get(instance.mesh_id as usize) {
            for i in 0..mesh_desc.index_count {
                instance_lookups.push(InstanceLookup {
                    instance_id: instance_id as u32,
                    local_vertex_index: i,
                });
            }
            total_vertices_to_draw += mesh_desc.index_count;
        }
    }

    let mesh_description_buffer = device.0.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Per-Frame Mesh Description Buffer"),
        contents: bytemuck::cast_slice(&mesh_descriptions),
        usage: wgpu::BufferUsages::STORAGE,
    });
    
    let instance_buffer = device.0.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Per-Frame Instance Buffer"),
        contents: bytemuck::cast_slice(&instances),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let instance_lookup_buffer = device.0.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Per-Frame Instance Lookup Buffer"),
        contents: bytemuck::cast_slice(&instance_lookups),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let mesh_bind_group = device.0.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Mesh Bind Group"),
        layout: &pipeline.mesh_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: asset_server.get_vertex_buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: asset_server.get_index_buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: mesh_description_buffer.as_entire_binding(),
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