use bevy_ecs::{world::{FromWorld, World}, prelude::Resource};
use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use redb::ReadableTable;
use types::{MODEL_TABLE, Model, Vertex};
use wgpu::util::DeviceExt;

use crate::renderer::core::WgpuDevice;

// Static asset data describing a slice of the geometry buffers.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct MeshletDescription {
    vertex_list_offset: u32,
    triangle_list_offset: u32,
    triangle_count: u32,
    _padding: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct DrawCommand {
    pub meshlet_id: u32,
    pub transform_id: u32,
}

#[derive(Resource)]
pub struct AssetServer {
    // CPU data
    pub vertices: Vec<Vertex>,
    pub meshlet_vertex_indices: Vec<u32>,
    pub meshlet_triangle_indices: Vec<u8>,
    pub meshlets: Vec<MeshletDescription>,
    pub transforms: Vec<Mat4>,
    pub draw_commands: Vec<DrawCommand>,

    // GPU resources
    pub vertex_buffer: Option<wgpu::Buffer>,
    pub meshlet_vertex_index_buffer: Option<wgpu::Buffer>,
    pub meshlet_triangle_index_buffer: Option<wgpu::Buffer>,
    pub meshlet_description_buffer: Option<wgpu::Buffer>,
    pub transform_buffer: Option<wgpu::Buffer>,
    pub indirection_buffer: Option<wgpu::Buffer>,

    pub mesh_bind_group_layout: Option<wgpu::BindGroupLayout>,
    pub mesh_bind_group: Option<wgpu::BindGroup>,
}

impl FromWorld for AssetServer {
    fn from_world(world: &mut bevy_ecs::world::World) -> Self {
        new(world)
    }
}

pub fn new(world: &mut World) -> AssetServer {
    let db = redb::Database::open("/Users/mewosmith/rust/asf_kai/database/models.redb").unwrap();
    let read_txn = db.begin_read().unwrap();
    let table = read_txn.open_table(MODEL_TABLE).unwrap();

    let mut all_vertices = Vec::new();
    let mut all_meshlet_vertex_indices = Vec::new();
    let mut all_meshlet_triangle_indices = Vec::new();
    let mut all_meshlets = Vec::new();

    for result in table.iter().unwrap() {
        let (_name, model_data) = result.unwrap();
        let model: Model = bincode::deserialize(model_data.value()).unwrap();

        for mesh in model.meshes {
            if let Some(mesh_meshlets) = mesh.meshlets {
                let vertex_base = all_vertices.len() as u32;
                let meshlet_vertex_index_base = all_meshlet_vertex_indices.len() as u32;
                let meshlet_triangle_index_base = all_meshlet_triangle_indices.len() as u32;

                // 1. Add this mesh's vertices to the global buffer.
                all_vertices.extend_from_slice(&mesh.vertices);

                // 2. Remap the meshlet's vertex indices to point into the global vertex
                //    buffer, then add them to the global meshlet vertex index buffer.
                let remapped_vertex_indices: Vec<u32> = mesh_meshlets
                    .vertices
                    .iter()
                    .map(|&i| vertex_base + i)
                    .collect();
                all_meshlet_vertex_indices.extend(remapped_vertex_indices);

                // 3. Add the meshlet's local triangle indices to the global buffer.
                all_meshlet_triangle_indices.extend(&mesh_meshlets.triangles);

                // 4. Create descriptions for each meshlet with offsets into the global buffers.
                for m in mesh_meshlets.meshlets {
                    all_meshlets.push(MeshletDescription {
                        // The offset to the list of vertex indices for this meshlet inside the
                        // global `meshlet_vertex_indices` buffer.
                        vertex_list_offset: meshlet_vertex_index_base + m.vertex_offset,

                        // The offset to the list of triangle indices for this meshlet.
                        // meshopt gives an offset in triangles, so we x3 for indices.
                        triangle_list_offset: meshlet_triangle_index_base + m.triangle_offset * 3,

                        triangle_count: m.triangle_count,
                        _padding: 0,
                    });
                }
            }
        }
    }

    let transforms = vec![glam::Mat4::IDENTITY];
    let draw_commands: Vec<DrawCommand> = (0..all_meshlets.len())
        .map(|i| DrawCommand {
            meshlet_id: i as u32,
            transform_id: 0, // All using the same identity transform for now
        })
        .collect();

    let mut asset_server = AssetServer {
        vertices: all_vertices,
        meshlet_vertex_indices: all_meshlet_vertex_indices,
        meshlet_triangle_indices: all_meshlet_triangle_indices,
        meshlets: all_meshlets,
        transforms,
        draw_commands,
        vertex_buffer: None,
        meshlet_vertex_index_buffer: None,
        meshlet_triangle_index_buffer: None,
        meshlet_description_buffer: None,
        transform_buffer: None,
        indirection_buffer: None,
        mesh_bind_group_layout: None,
        mesh_bind_group: None,
    };
    create_gpu_resources(&mut asset_server, &world.resource::<WgpuDevice>());
    asset_server
}

fn create_gpu_resources(asset_server: &mut AssetServer, device: &wgpu::Device) {
    // Create buffers for all the geometry data
    asset_server.vertex_buffer = Some(device.create_buffer_init(
        &wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(&asset_server.vertices),
            usage: wgpu::BufferUsages::STORAGE,
        },
    ));

    asset_server.meshlet_vertex_index_buffer = Some(device.create_buffer_init(
        &wgpu::util::BufferInitDescriptor {
            label: Some("Meshlet Vertex Index Buffer"),
            contents: bytemuck::cast_slice(&asset_server.meshlet_vertex_indices),
            usage: wgpu::BufferUsages::STORAGE,
        },
    ));

    // WGSL doesn't support u8 storage buffers, so we must pack the u8 triangle
    // indices into a u32 buffer. We pad the data to be a multiple of 4 bytes.
    let mut padded_triangle_indices = asset_server.meshlet_triangle_indices.clone();
    while padded_triangle_indices.len() % 4 != 0 {
        padded_triangle_indices.push(0);
    }
    let packed_triangle_indices: Vec<u32> = bytemuck::cast_slice(&padded_triangle_indices)
        .iter()
        .copied()
        .collect();

    asset_server.meshlet_triangle_index_buffer = Some(device.create_buffer_init(
        &wgpu::util::BufferInitDescriptor {
            label: Some("Meshlet Triangle Index Buffer (Packed)"),
            contents: bytemuck::cast_slice(&packed_triangle_indices),
            usage: wgpu::BufferUsages::STORAGE,
        },
    ));

    asset_server.meshlet_description_buffer = Some(device.create_buffer_init(
        &wgpu::util::BufferInitDescriptor {
            label: Some("Meshlet Description Buffer"),
            contents: bytemuck::cast_slice(&asset_server.meshlets),
            usage: wgpu::BufferUsages::STORAGE,
        },
    ));

    asset_server.transform_buffer = Some(device.create_buffer_init(
        &wgpu::util::BufferInitDescriptor {
            label: Some("Transform Buffer"),
            contents: bytemuck::cast_slice(&asset_server.transforms),
            usage: wgpu::BufferUsages::STORAGE,
        },
    ));

    asset_server.indirection_buffer = Some(device.create_buffer_init(
        &wgpu::util::BufferInitDescriptor {
            label: Some("Indirection Buffer"),
            contents: bytemuck::cast_slice(&asset_server.draw_commands),
            usage: wgpu::BufferUsages::STORAGE,
        },
    ));

    // Create bind group layout and bind group
    asset_server.mesh_bind_group_layout = Some(device.create_bind_group_layout(
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("Mesh Bind Group Layout"),
            entries: &[
                // global_vertices @binding(0)
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // meshlet_vertex_indices @binding(1)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // meshlet_triangle_indices (packed) @binding(2)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // meshlet_descriptions @binding(3)
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // indirection_buffer @binding(4)
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // transform_buffer @binding(5)
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        },
    ));

    asset_server.mesh_bind_group = Some(
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Mesh Bind Group"),
            layout: asset_server.mesh_bind_group_layout.as_ref().unwrap(),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: asset_server
                        .vertex_buffer
                        .as_ref()
                        .unwrap()
                        .as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: asset_server
                        .meshlet_vertex_index_buffer
                        .as_ref()
                        .unwrap()
                        .as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: asset_server
                        .meshlet_triangle_index_buffer
                        .as_ref()
                        .unwrap()
                        .as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: asset_server
                        .meshlet_description_buffer
                        .as_ref()
                        .unwrap()
                        .as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: asset_server
                        .indirection_buffer
                        .as_ref()
                        .unwrap()
                        .as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: asset_server
                        .transform_buffer
                        .as_ref()
                        .unwrap()
                        .as_entire_binding(),
                },
            ],
        }),
    );
}
