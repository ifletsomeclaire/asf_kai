use crate::{
    ecs::model::{InstanceGpuData, InstanceMaterial, SpawnedEntities},
    renderer::assets::{AssetRequest, AssetServer, AssetType},
};
use bevy_ecs::prelude::*;
use bevy_transform::prelude::GlobalTransform;

/// A command to spawn a new instance of a model.
/// This command handles all the logic of interacting with the AssetServer.
#[derive(Debug)]
pub struct SpawnInstance {
    pub transform: GlobalTransform,
    pub mesh_name: String,
    pub texture_name: String,
}

impl Command for SpawnInstance {
    fn apply(self, world: &mut World) {
        world.resource_scope(|world, mut asset_server: Mut<AssetServer>| {
            // 1. Create a batch request for the assets.
            let requests = [
                AssetRequest {
                    name: self.mesh_name.clone(),
                    kind: AssetType::Mesh,
                },
                AssetRequest {
                    name: self.texture_name.clone(),
                    kind: AssetType::Texture,
                },
            ];
            asset_server.load_batch(&requests);

            // 2. Get handles for the named assets. They will exist now because load_batch creates them.
            let mesh_handle = *asset_server
                .mesh_name_to_handle
                .get(&self.mesh_name)
                .unwrap();
            let texture_handle = *asset_server
                .texture_name_to_handle
                .get(&self.texture_name)
                .unwrap();

            // 3. Get or create a material from the handles.
            let material_handle = asset_server.get_or_create_material(mesh_handle, texture_handle);

            // 4. Increment the reference count for the material.
            asset_server.increment_material_ref(material_handle.0);

            // 5. Create the GpuInstance component, using fallback data until loaded.
            let fallback_mesh = asset_server
                .fallback_gpu_mesh
                .as_ref()
                .expect("Fallback mesh not initialized");
            let fallback_texture = asset_server
                .fallback_gpu_texture
                .as_ref()
                .expect("Fallback texture not initialized");

            let instance_gpu_data = InstanceGpuData {
                model_matrix: self.transform.compute_matrix().to_cols_array_2d(),
                mesh_id: mesh_handle.0 as u32,
                texture_id: texture_handle.0 as u32,
                texture_array_index: fallback_texture.texture_array_index,
                index_count: fallback_mesh.index_count,
            };

            let instance_material = InstanceMaterial {
                material_handle: material_handle.0,
            };

            // 6. Spawn the entity with the component containing placeholder data.
            let entity_id = world.spawn((instance_gpu_data, instance_material)).id();

            // 7. Add the new entity to our tracking list for the UI.
            if let Some(mut spawned_entities) = world.get_resource_mut::<SpawnedEntities>() {
                spawned_entities.0.push(entity_id);
            }
        });
    }
}

/// A command to despawn an entity and correctly decrement the ref count of its material.
pub struct DespawnInstance {
    pub entity: Entity,
}

impl Command for DespawnInstance {
    fn apply(self, world: &mut World) {
        let material_handle = world
            .get::<InstanceMaterial>(self.entity)
            .map(|mat| mat.material_handle);

        if let Some(handle) = material_handle {
            if let Some(mut asset_server) = world.get_resource_mut::<AssetServer>() {
                asset_server.decrement_material_ref(handle);
            }
        }

        if let Ok(entity_commands) = world.get_entity_mut(self.entity) {
            entity_commands.despawn();
        }

        if let Some(mut spawned_entities) = world.get_resource_mut::<SpawnedEntities>() {
            spawned_entities.0.retain(|&e| e != self.entity);
        }
    }
}
