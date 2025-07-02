use crate::renderer::assets::{
    AssetServer, DeallocationMessage, MeshHandle, TextureHandle,
};
use crate::renderer::{
    core::{WgpuDevice, WgpuQueue},
    d3_pipeline::D3Pipeline,
};
use bevy_ecs::prelude::*;
use bevy_transform::components::GlobalTransform;
use bevy_transform::components::Transform;
use bytemuck::{Pod, Zeroable};
use image;
use redb::{Database, TableDefinition, ReadableTable, ReadableTableMetadata};
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use types::Model as TypesModel;
use wgpu::util::DeviceExt;

const MODEL_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("models");
const TEXTURE_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("textures");
const TEXTURE_NAME: &str = "StylizedWater.png";

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 4],
    pub normal: [f32; 3],
    pub _padding1: u32,
    pub tex_coords: [f32; 2],
    pub _padding2: [u32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MeshDescription {
    pub index_count: u32,
    pub first_index: u32,
    pub base_vertex: i32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Instance {
    model_matrix: [[f32; 4]; 4],
    mesh_id: u32,
    texture_array_index: u32,
    _padding: [u32; 2],
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
    pub mesh_handle: MeshHandle,
    pub texture_handle: Option<TextureHandle>,
}

pub struct ModelInfo {
    pub name: String,
    pub mesh_handle: MeshHandle,
    pub texture_handle: Option<TextureHandle>,
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

#[derive(Resource)]
pub struct AssetReports {
    pub vertex_total_free: u32,
    pub vertex_largest_free: u32,
    pub index_total_free: u32,
    pub index_largest_free: u32,
    pub model_count: u64,
    pub texture_count: u64,
    pub database_file_size: u64,
    pub last_generated: std::time::Instant,
}

impl Default for AssetReports {
    fn default() -> Self {
        Self {
            vertex_total_free: 0,
            vertex_largest_free: 0,
            index_total_free: 0,
            index_largest_free: 0,
            model_count: 0,
            texture_count: 0,
            database_file_size: 0,
            last_generated: std::time::Instant::now() - std::time::Duration::from_secs(10), // Make it old so first run triggers
        }
    }
}

pub fn load_models_from_db_system(
    mut commands: Commands,
    mut asset_server: ResMut<AssetServer>,
    queue: Res<WgpuQueue>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Running load_models_from_db_system ---");
    let mut available_models = AvailableModels::default();

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap();
    let db_path = workspace_root.join("assets/models.redb");

    let db = Database::open(&db_path)?;
    let read_txn = db.begin_read()?;

    // --- Load ALL textures from the database ---
    let texture_table = read_txn.open_table(TEXTURE_TABLE)?;
    for result in texture_table.range::<&str>(..)? {
        let (key, value) = result?;
        let texture_name = key.value();
        let texture_bytes = value.value();
        let texture_image = image::load_from_memory(texture_bytes)?;
        let texture_handle = asset_server.load_texture(&texture_image, &queue.0).unwrap();
        println!("[CORE] Loaded texture '{}' from DB.", texture_name);
        asset_server.register_texture_handle(texture_name, texture_handle);
    }

    // Load all models and populate AvailableModels
    let model_table = read_txn.open_table(MODEL_TABLE)?;
    for result in model_table.range::<&str>(..)? {
        let (key, value) = result?;
        let filename = key.value().to_string();
        let model: TypesModel = bincode::deserialize(value.value())?;

        // A model can have multiple meshes
        for cpu_mesh in &model.meshes {
            let (mesh_handle, _) = asset_server.load_mesh(cpu_mesh, &queue.0).unwrap();
            let mesh_name = cpu_mesh.name.clone();
            asset_server.register_mesh_handle(&mesh_name, mesh_handle.clone());

            let texture_handle = if let Some(texture_name) = &cpu_mesh.texture_name {
                // If the texture isn't in the DB, this will fail. For now, we just won't assign one.
                let handle = asset_server.get_texture_handle(texture_name).cloned();
                println!(
                    "[CORE] Mesh '{}' requests texture '{}'. Handle found: {}",
                    cpu_mesh.name,
                    texture_name,
                    handle.is_some()
                );
                handle
            } else {
                println!("Mesh '{}' has no texture.", cpu_mesh.name);
                None
            };

            available_models.models.push(ModelInfo {
                name: mesh_name,
                mesh_handle,
                texture_handle,
            });
        }

        if filename == "cube.gltf" {
            if let Some(mesh) = model.meshes.first() {
                let mesh_handle = asset_server.get_mesh_handle(&mesh.name).unwrap().clone();
                let texture_handle = if let Some(texture_name) = &mesh.texture_name {
                    asset_server.get_texture_handle(texture_name).cloned()
                } else {
                    None
                };

                commands.spawn((
                    Model {
                        mesh_name: mesh.name.clone(),
                        mesh_handle,
                        texture_handle,
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
        let mesh_id = *handle_to_mesh_id
            .entry(model.mesh_handle.id())
            .or_insert_with(|| {
                let id = mesh_descriptions.len() as u32;
                if let Some(gpu_mesh) = asset_server.get_gpu_mesh(&model.mesh_handle) {
                    mesh_descriptions.push(MeshDescription {
                        index_count: gpu_mesh.index_count,
                        first_index: (gpu_mesh.index_buffer_offset
                            / std::mem::size_of::<u32>() as u64)
                            as u32,
                        base_vertex: (gpu_mesh.vertex_buffer_offset
                            / std::mem::size_of::<Vertex>() as u64)
                            as i32,
                    });
                }
                id
            });

        let texture_array_index = if let Some(handle) = &model.texture_handle {
            if let Some(gpu_texture) = asset_server.get_gpu_texture(handle) {
                let index = gpu_texture.texture_array_index;
                println!(
                    "[CORE] Model '{}' -> Texture Index: {}",
                    model.mesh_name, index
                );
                index
            } else {
                eprintln!(
                    "[CORE] Model '{}' has a texture handle but NO GpuTexture!",
                    model.mesh_name
                );
                u32::MAX // Sentinel for no texture
            }
        } else {
            println!("[CORE] Model '{}' has no texture handle.", model.mesh_name);
            u32::MAX // Sentinel for no texture
        };

        instances.push(Instance {
            model_matrix: transform.compute_matrix().to_cols_array_2d(),
            mesh_id,
            texture_array_index,
            _padding: [0; 2],
        });
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

    let mesh_description_buffer = device
        .0
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Per-Frame Mesh Description Buffer"),
            contents: bytemuck::cast_slice(&mesh_descriptions),
            usage: wgpu::BufferUsages::STORAGE,
        });

    let instance_buffer = device
        .0
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Per-Frame Instance Buffer"),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::STORAGE,
        });

    let instance_lookup_buffer = device
        .0
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
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
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(asset_server.get_texture_view()),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::Sampler(asset_server.get_texture_sampler()),
            },
        ],
    });

    commands.insert_resource(PerFrameSceneData {
        mesh_bind_group,
        total_vertices_to_draw,
    });
}

pub fn process_asset_deallocations_system(mut asset_server: ResMut<AssetServer>) {
    while let Ok(message) = asset_server.deallocation_receiver.try_recv() {
        match message {
            DeallocationMessage::Mesh(vertex_alloc, index_alloc) => {
                asset_server.mesh_pool.vertex_allocator.free(vertex_alloc);
                asset_server.mesh_pool.index_allocator.free(index_alloc);
                println!("Deallocated a mesh from GPU pool.");
            }
        }
    }
}

pub fn generate_asset_reports_system(
    asset_server: Res<AssetServer>,
    mut reports: ResMut<AssetReports>,
) {
    // Only update reports every 5 seconds to avoid spamming
    if reports.last_generated.elapsed().as_secs() < 5 {
        return;
    }
    
    // Get offset allocator reports
    let vertex_report = asset_server.mesh_pool.vertex_allocator.storage_report();
    let index_report = asset_server.mesh_pool.index_allocator.storage_report();
    
    reports.vertex_total_free = vertex_report.total_free_space;
    reports.vertex_largest_free = vertex_report.largest_free_region;
    reports.index_total_free = index_report.total_free_space;
    reports.index_largest_free = index_report.largest_free_region;
    
    // Get database reports
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap();
    let db_path = workspace_root.join("assets/models.redb");
    
    if let Ok(db) = Database::open(&db_path) {
        if let Ok(read_txn) = db.begin_read() {
            if let Ok(model_table) = read_txn.open_table(MODEL_TABLE) {
                reports.model_count = model_table.len().unwrap_or(0);
            }
            
            if let Ok(texture_table) = read_txn.open_table(TEXTURE_TABLE) {
                reports.texture_count = texture_table.len().unwrap_or(0);
            }
        }
        
        if let Ok(metadata) = std::fs::metadata(&db_path) {
            reports.database_file_size = metadata.len();
        }
    }
    
    reports.last_generated = std::time::Instant::now();
    
    println!("\n=== ASSET SERVER REPORTS (Updated) ===");
    println!("GPU Memory Pools:");
    println!("  - Vertex Buffer: {} bytes free, largest: {} bytes", 
             reports.vertex_total_free, reports.vertex_largest_free);
    println!("  - Index Buffer: {} bytes free, largest: {} bytes", 
             reports.index_total_free, reports.index_largest_free);
    println!("Database:");
    println!("  - Models: {}, Textures: {}", reports.model_count, reports.texture_count);
    println!("  - Database file size: {} bytes", reports.database_file_size);
    println!("=== END REPORTS ===\n");
}
