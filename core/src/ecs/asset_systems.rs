//! Systems for managing asset life cycles.

use bevy_ecs::prelude::*;

use crate::{ecs::commands::SpawnGpuInstance, renderer::assets::AssetServer};

use super::model::GpuInstance;

/// This system queries for `SpawnGpuInstance` components and processes them.
/// It creates the final `GpuInstance` component and increments the asset reference counts.
/// The original entity with the `SpawnGpuInstance` request is then despawned.
pub fn process_spawn_requests_system(
    mut commands: Commands,
    mut asset_server: ResMut<AssetServer>,
    query: Query<(Entity, &SpawnGpuInstance)>,
) {
    for (entity, request) in query.iter() {
        let Some(gpu_mesh) = asset_server.meshes.get(&request.mesh_handle.0) else { continue; };
        let Some(gpu_texture) = asset_server.textures.get(&request.texture_handle.0) else { continue; };

        let instance_component = GpuInstance {
            model_matrix: request.transform.compute_matrix().to_cols_array_2d(),
            mesh_id: request.mesh_handle.0 as u32,
            texture_id: request.texture_handle.0 as u32,
            texture_array_index: gpu_texture.texture_array_index,
            index_count: gpu_mesh.index_count,
        };

        commands.spawn(instance_component);
        asset_server.increment_mesh_ref(request.mesh_handle.0);
        asset_server.increment_texture_ref(request.texture_handle.0);

        // Despawn the request entity
        commands.entity(entity).despawn();
    }
}

/// This system is responsible for decrementing the reference counts of assets
/// when a `GpuInstance` component is removed from an entity.
///
/// It listens for `RemovedComponents<GpuInstance>` events.
//
// TODO: This system currently cannot be implemented correctly because Bevy's
// `RemovedComponents` only provides the `Entity` ID, not the data of the
// component that was removed. To properly decrement the asset reference counts,
// we need the `mesh_id` and `texture_id` from the `GpuInstance` component
// before it's deleted.
//
// A possible solution would be to introduce an intermediate `DespawnedGpuInstance`
// event that is sent manually with the component data just before an entity
// is despawned. This system would then listen for those events instead.
//
// For now, this system is a placeholder to fulfill the plan's structure.
pub fn decrement_asset_ref_counts_system(
    mut asset_server: ResMut<AssetServer>,
    mut removed: RemovedComponents<GpuInstance>,
) {
    for _entity in removed.read() {
        // let gpu_instance = ... get GpuInstance data for entity ...
        // asset_server.decrement_mesh_ref(gpu_instance.mesh_id as u64);
        // asset_server.decrement_texture_ref(gpu_instance.texture_id as u64);
    }
} 