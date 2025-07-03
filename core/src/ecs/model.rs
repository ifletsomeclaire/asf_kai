use crate::{
    ecs::commands::SpawnGpuInstance,
    renderer::assets::{AssetServer, MeshHandle, TextureHandle},
    renderer::{
        core::WgpuQueue,
    },
};
use bevy_ecs::prelude::*;
use bevy_transform::components::GlobalTransform;
use bytemuck::{Pod, Zeroable};
use image;
use redb::{Database, TableDefinition, ReadableTableMetadata};
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use types::Model as TypesModel;

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
    pub _padding: u32,
}

#[repr(C)]
#[derive(Component, Clone, Copy, Pod, Zeroable)]
pub struct GpuInstance {
    pub model_matrix: [[f32; 4]; 4],
    // The stable u64 ID from the AssetServer, cast to u32 for the shader.
    // This is used to look up the MeshDescription.
    pub mesh_id: u32,
    // The stable u64 ID from the AssetServer for the texture.
    pub texture_id: u32,
    // The volatile u32 index into the texture array.
    pub texture_array_index: u32,
    // The total number of indices in this instance's mesh.
    // This is crucial for calculating the total vertex count for the draw call.
    pub index_count: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct InstanceLookup {
    pub instance_id: u32,
    pub local_vertex_index: u32,
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

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap();
    let db_path = workspace_root.join("assets/models.redb");

    let db = Database::open(&db_path)?;
    let read_txn = db.begin_read()?;

    // --- Load ALL textures from the database ---
    let mut texture_handles = HashMap::new();
    let texture_table = read_txn.open_table(TEXTURE_TABLE)?;
    for result in texture_table.range::<&str>(..)? {
        let (key, value) = result?;
        let texture_name = key.value();
        let texture_bytes = value.value();
        let texture_image = image::load_from_memory(texture_bytes)?;
        let texture_handle = asset_server
            .load_texture(&texture_image, &queue.0, texture_name)
            .unwrap();
        texture_handles.insert(texture_name.to_string(), texture_handle);
        println!("[CORE] Loaded texture '{}' from DB.", texture_name);
    }

    // --- Get a default texture if one isn't found for a mesh ---
    let default_texture_handle = texture_handles
        .values()
        .next()
        .expect("Failed to load any textures, cannot provide a default.");

    // Load all models and spawn an instance for each mesh
    let model_table = read_txn.open_table(MODEL_TABLE)?;
    for result in model_table.range::<&str>(..)? {
        let (key, value) = result?;
        let filename = key.value().to_string();
        let model: TypesModel = bincode::deserialize(value.value())?;

        // A model can have multiple meshes
        for cpu_mesh in &model.meshes {
            let (mesh_handle, _) = asset_server.load_mesh(cpu_mesh, &queue.0).unwrap();

            let texture_handle = cpu_mesh
                .texture_name
                .as_ref()
                .and_then(|name| texture_handles.get(name))
                .unwrap_or(default_texture_handle);

            println!(
                "[CORE] Spawning instance for mesh '{}' from model '{}'",
                cpu_mesh.name, filename
            );

            commands.spawn(SpawnGpuInstance {
                transform: GlobalTransform::default(),
                mesh_handle,
                texture_handle: *texture_handle,
            });
        }
    }

    println!("--- Finished load_models_from_db_system ---");

    Ok(())
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
