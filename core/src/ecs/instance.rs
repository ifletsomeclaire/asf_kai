use bevy_ecs::prelude::*;
use bevy_transform::components::Transform;

use crate::renderer::{
    assets::{AssetServer, MeshHandle, TextureHandle},
    scene::InstanceGpuData,
};

// This system creates the GPU-specific instance data from the high-level components.
pub fn create_instance_gpu_data_system(
    mut commands: Commands,
    query: Query<(Entity, &Transform, &MeshHandle)>,
    mut asset_server: ResMut<AssetServer>,
) {
    let fallback_texture_handle = asset_server
        .fallback_gpu_texture
        .as_ref()
        .and_then(|tex| {
            asset_server
                .texture_name_to_handle
                .get("fallback_texture")
                .cloned()
        })
        .unwrap_or(TextureHandle(0)); // Should have a fallback

    for (entity, transform, mesh_handle) in query.iter() {
        let material =
            asset_server.get_or_create_material(*mesh_handle, fallback_texture_handle);
        commands.entity(entity).insert(InstanceGpuData {
            model_matrix: transform.compute_matrix().to_cols_array_2d(),
            material_id: material.0 as u32,
        });
    }
} 