use bevy_ecs::prelude::*;

use crate::{
    ecs::model::InstanceGpuData,
    renderer::{
        assets::{AssetServer, AssetLoadResult, LoadingStatus},
        core::WgpuQueue,
    },
};

pub fn process_asset_loads_system(
    mut asset_server: ResMut<AssetServer>,
    queue: Res<WgpuQueue>,
) {
    let messages: Vec<_> = asset_server.asset_load_receiver.try_iter().collect();
    for msg in messages {
        match msg {
            AssetLoadResult::Mesh {
                name,
                cpu_mesh,
                token,
            } => {
                asset_server.finish_loading_mesh(name, cpu_mesh, token, &queue.0);
            }
            AssetLoadResult::Texture { name, image, token } => {
                asset_server.finish_loading_texture(name, image, token, &queue.0);
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