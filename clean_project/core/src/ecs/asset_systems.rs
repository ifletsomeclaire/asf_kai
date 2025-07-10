use bevy_ecs::prelude::*;

use crate::{
    ecs::model::InstanceGpuData,
    renderer::{
        assets::{AssetServer, LoadedAssetData, LoadingStatus},
        core::WgpuQueue,
    },
};

pub fn process_asset_loads_system(mut asset_server: ResMut<AssetServer>, queue: Res<WgpuQueue>) {
    let messages: Vec<_> = asset_server.asset_load_receiver.try_iter().collect();
    for msg_batch in messages {
        let mut all_tokens_match = true;
        for loaded_asset in &msg_batch.assets {
            if let Some(LoadingStatus::Loading {
                token: stored_token,
                ..
            }) = asset_server.mesh_load_state.get(&loaded_asset.name)
            {
                if *stored_token != msg_batch.token {
                    all_tokens_match = false;
                    break;
                }
            } else if let Some(LoadingStatus::Loading {
                token: stored_token,
                ..
            }) = asset_server.texture_load_state.get(&loaded_asset.name)
            {
                if *stored_token != msg_batch.token {
                    all_tokens_match = false;
                    break;
                }
            }
        }

        if all_tokens_match {
            for loaded_asset in msg_batch.assets {
                let (id_to_upload, is_mesh) = {
                    let is_mesh = matches!(loaded_asset.data, LoadedAssetData::Mesh(_));
                    let state_map = if is_mesh {
                        &asset_server.mesh_load_state
                    } else {
                        &asset_server.texture_load_state
                    };

                    let id = if let Some(LoadingStatus::Loading { id, .. }) =
                        state_map.get(&loaded_asset.name)
                    {
                        Some(*id)
                    } else {
                        None
                    };
                    (id, is_mesh)
                };

                if let Some(id) = id_to_upload {
                    if is_mesh {
                        if let LoadedAssetData::Mesh(cpu_mesh) = loaded_asset.data {
                            if asset_server.upload_mesh_to_gpu(id, &cpu_mesh, &queue.0) {
                                asset_server
                                    .mesh_load_state
                                    .insert(loaded_asset.name, LoadingStatus::Loaded(id));
                            }
                        }
                    } else {
                        if let LoadedAssetData::Texture(image) = loaded_asset.data {
                            if asset_server.upload_texture_to_gpu(
                                id,
                                &image,
                                &queue.0,
                                &loaded_asset.name,
                            ) {
                                asset_server
                                    .texture_load_state
                                    .insert(loaded_asset.name, LoadingStatus::Loaded(id));
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn patch_instance_data_system(
    asset_server: Res<AssetServer>,
    mut query: Query<&mut InstanceGpuData>,
) {
    let fallback_mesh = match asset_server.fallback_gpu_mesh.as_ref() {
        Some(mesh) => mesh,
        None => return, // Fallback not loaded yet, can't patch.
    };

    for mut instance in query.iter_mut() {
        // If the instance is still using the fallback mesh's index count, it's a candidate for patching.
        if instance.index_count == fallback_mesh.index_count {
            let mesh_id = instance.mesh_id as u64;
            let texture_id = instance.texture_id as u64;

            // Check if both the real mesh and texture are loaded.
            if let (Some(gpu_mesh), Some(gpu_texture)) = (
                asset_server.meshes.get(&mesh_id),
                asset_server.textures.get(&texture_id),
            ) {
                // Both are loaded, so patch the instance data.
                instance.index_count = gpu_mesh.index_count;
                instance.texture_array_index = gpu_texture.texture_array_index;
            }
        }
    }
}
