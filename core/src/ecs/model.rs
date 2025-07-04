use bevy_ecs::prelude::*;
use bytemuck::{Pod, Zeroable};
use redb::{Database, ReadableTable};
use std::{env, path::PathBuf};

use crate::renderer::{
    assets::{
        AssetServer, AssetType, BatchAssetLoadRequest, LoadedAsset, LoadedAssetData, MODEL_TABLE,
        TEXTURE_TABLE,
    },
};
use crossbeam_channel::unbounded;
use types::Model as TypesModel;

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
#[derive(Component, Copy, Clone, Pod, Zeroable, Debug)]
pub struct InstanceGpuData {
    pub model_matrix: [[f32; 4]; 4],
    pub mesh_id: u32,
    pub texture_id: u32,
    pub texture_array_index: u32,
    pub index_count: u32,
}

#[derive(Component)]
pub struct InstanceMaterial {
    pub material_handle: u64,
}

#[derive(Component)]
pub struct Patched;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct InstanceLookup {
    pub instance_id: u32,
    pub local_vertex_index: u32,
}

#[derive(Resource, Default)]
pub struct SpawnedEntities(pub Vec<Entity>);

pub fn initialize_fallback_assets_system(
    mut asset_server: ResMut<AssetServer>,
    queue: Res<crate::renderer::core::WgpuQueue>,
) {
    asset_server.create_and_upload_fallback_assets(&queue.0);
}

pub fn spawn_asset_loader_task_system(mut asset_server: ResMut<AssetServer>) {
    let (s, r) = unbounded::<BatchAssetLoadRequest>();
    asset_server.asset_load_request_sender = Some(s);

    let result_sender = asset_server.asset_load_result_sender.clone();
    let db = asset_server.db.as_ref().unwrap().clone();

    std::thread::spawn(move || {
        for request_batch in r {
            let mut results = Vec::new();
            for request in request_batch.assets {
                match request.kind {
                    AssetType::Mesh => {
                        let read_txn = db.begin_read().unwrap();
                        let table = read_txn.open_table(MODEL_TABLE).unwrap();
                        for item in table.iter().unwrap().flatten() {
                            if let Ok(model) = bincode::deserialize::<TypesModel>(item.1.value())
                            {
                                for mesh in &model.meshes {
                                    if mesh.name == request.name {
                                        results.push(LoadedAsset {
                                            name: request.name.clone(),
                                            data: LoadedAssetData::Mesh(mesh.clone()),
                                        });
                                        break; // Found the mesh, move to next request
                                    }
                                }
                            }
                        }
                    }
                    AssetType::Texture => {
                        let read_txn = db.begin_read().unwrap();
                        let table = read_txn.open_table(TEXTURE_TABLE).unwrap();
                        if let Ok(Some(data)) = table.get(request.name.as_str()) {
                            if let Ok(image) = image::load_from_memory(data.value()) {
                                results.push(LoadedAsset {
                                    name: request.name.clone(),
                                    data: LoadedAssetData::Texture(image),
                                });
                            }
                        }
                    }
                }
            }

            if !results.is_empty() {
                let result_batch = crate::renderer::assets::BatchAssetLoadResult {
                    assets: results,
                    token: request_batch.token,
                };
                result_sender.send(result_batch).unwrap();
            }
        }
    });
}

/// This system loads all models and textures from the database at startup.
/// It populates the AssetServer but does not spawn any instances itself.
pub fn initialize_asset_db_system(
    mut asset_server: ResMut<AssetServer>,
) {
    println!("--- Initializing asset database ---");
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let db_path = PathBuf::from(manifest_dir)
        .parent()
        .unwrap()
        .join("assets/models.redb");

    let db = match Database::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Failed to open database at {:?}: {}", db_path, e);
            return;
        }
    };
    
    asset_server.set_db(db);
    println!("--- Asset database initialized ---");
}
