//! This module is responsible for preparing all scene data for a single-pass, vertex-pulling render pipeline.
use bevy_ecs::prelude::*;
use wgpu::util::DeviceExt;

use crate::{
    ecs::model::{InstanceGpuData, InstanceLookup, MeshDescription},
    renderer::{
        assets::AssetServer,
        core::{WgpuDevice, WgpuQueue},
        d3_pipeline::D3Pipeline,
    },
};
use std::collections::HashMap;

/// A resource to hold the final data needed for the single main render pass.
#[derive(Resource, Default)]
pub struct FrameRenderData {
    pub total_indices_to_draw: u32,
}

/// A resource to hold the bind group containing all mesh and instance data for the frame.
#[derive(Resource)]
pub struct MeshBindGroup(pub wgpu::BindGroup);

/// The single system that prepares all data for one draw call, respecting the vertex pulling model.
/// This replaces all previous "prepare" and "copy" logic.
pub fn prepare_and_copy_scene_data_system(
    mut commands: Commands,
    instance_query: Query<&InstanceGpuData>,
    asset_server: Res<AssetServer>,
    pipeline: Res<D3Pipeline>,
    device: Res<WgpuDevice>,
    queue: Res<WgpuQueue>,
) {
    let instances: Vec<InstanceGpuData> = instance_query
        .iter()
        .filter(|inst| inst.index_count != u32::MAX)
        .copied()
        .collect();
        
    if instances.is_empty() {
        commands.remove_resource::<MeshBindGroup>();
        commands.remove_resource::<FrameRenderData>();
        return;
    }
    
    // --- Part A: Mesh Description Data ---
    // The shader needs a buffer of MeshDescription structs. We sort the active meshes by their
    // ID to ensure the buffer we create has a predictable order. The shader will use the mesh_id
    // as an index into this sorted buffer.
    let mut all_meshes: Vec<_> = asset_server.meshes.iter().collect();
    all_meshes.sort_by_key(|(id, _)| **id);

    // Create a map from the original (sparse) mesh ID to its new (dense) index in the sorted list.
    let mesh_id_to_dense_index: HashMap<u64, u32> = all_meshes
        .iter()
        .enumerate()
        .map(|(dense_index, (sparse_id, _))| (**sparse_id, dense_index as u32))
        .collect();

    let mesh_descriptions: Vec<MeshDescription> = all_meshes
        .iter()
        .map(|(_, gpu_mesh)| MeshDescription {
            index_count: gpu_mesh.index_count,
            first_index: (gpu_mesh.index_buffer_offset / std::mem::size_of::<u32>() as u64) as u32,
            base_vertex: (gpu_mesh.vertex_buffer_offset / std::mem::size_of::<crate::ecs::model::Vertex>() as u64) as i32,
            _padding: 0,
        })
        .collect();
        
    let mesh_description_buffer =
        device.0.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh_description_buffer"),
            contents: bytemuck::cast_slice(&mesh_descriptions),
            usage: wgpu::BufferUsages::STORAGE,
        });

    // --- Part B: Instance Data (with remapped IDs) ---
    // We must update the `mesh_id` in each instance to be the *dense index* from our map.
    let remapped_instances: Vec<InstanceGpuData> = instances
        .iter()
        .filter_map(|original_instance| {
            if let Some(dense_index) = mesh_id_to_dense_index.get(&(original_instance.mesh_id as u64)) {
                let mut updated_instance = *original_instance;
                updated_instance.mesh_id = *dense_index;
                Some(updated_instance)
            } else {
                None // This instance's mesh is no longer loaded, so we skip it.
            }
        })
        .collect();
        
    if remapped_instances.is_empty() {
         commands.remove_resource::<MeshBindGroup>();
         commands.remove_resource::<FrameRenderData>();
         return;
    }

    let instance_buffer = device.0.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("instance_buffer"),
        contents: bytemuck::cast_slice(&remapped_instances),
        usage: wgpu::BufferUsages::STORAGE,
    });

    // --- Part C: The "Uber" Lookup Buffer ---
    // This buffer maps a global vertex index to the specific instance it belongs to.
    let mut instance_lookups: Vec<InstanceLookup> = Vec::new();
    let mut total_indices = 0;
    for (instance_id, instance) in remapped_instances.iter().enumerate() {
        for i in 0..instance.index_count {
            instance_lookups.push(InstanceLookup {
                instance_id: instance_id as u32,
                local_vertex_index: i,
            });
        }
        total_indices += instance.index_count;
    }
    let instance_lookup_buffer =
        device.0.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("instance_lookup_buffer"),
            contents: bytemuck::cast_slice(&instance_lookups),
            usage: wgpu::BufferUsages::STORAGE,
        });

    // --- Part D: Create the Bind Group ---
    let mesh_bind_group = device.0.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("mesh_bind_group"),
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
                resource: instance_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: mesh_description_buffer.as_entire_binding(),
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

    commands.insert_resource(MeshBindGroup(mesh_bind_group));
    commands.insert_resource(FrameRenderData {
        total_indices_to_draw: total_indices,
    });
} 