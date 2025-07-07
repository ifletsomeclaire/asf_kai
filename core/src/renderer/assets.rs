use bevy_ecs::{world::{FromWorld, World}, prelude::Resource};
use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use redb::{ReadableTable, ReadableTableMetadata};
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
    vertex_count: u32,
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
    let db = redb::Database::open("/Users/mewosmith/rust/asf_kai/assets/models.redb").unwrap();
    let read_txn = db.begin_read().unwrap();
    let table = read_txn.open_table(MODEL_TABLE).unwrap();
    println!("table length: {:?}", table.len());
    let mut all_vertices = Vec::new();
    let mut all_meshlet_vertex_indices = Vec::new();
    let mut all_meshlet_triangle_indices = Vec::new();
    let mut all_meshlets = Vec::new();
    let mut has_printed_debug_info = false;

    println!("[Asset Loading] Starting model processing...");
    let mut models_processed = 0;
    let mut meshes_with_meshlets = 0;

    for result in table.iter().unwrap() {
        let (name_bytes, model_data) = result.unwrap();
        let name = name_bytes.value();
        models_processed += 1;
        println!("[Asset Loading] Processing model: {}", name);

        let model: Model = bincode::deserialize(model_data.value()).unwrap();

        for mesh in model.meshes {
            if let Some(mesh_meshlets) = mesh.meshlets {
                meshes_with_meshlets += 1;
                println!(
                    "  -> Mesh '{}' HAS meshlets. Adding to buffers.",
                    mesh.name
                );
                let vertex_base = all_vertices.len() as u32;
                let meshlet_vertex_index_base = all_meshlet_vertex_indices.len() as u32;

                // 1. Add this mesh's vertices to the global buffer.
                all_vertices.extend_from_slice(&mesh.vertices);

                // 2. The meshlet's `vertices` are indices into the mesh's vertex buffer.
                //    We remap them to point into the global vertex buffer.
                let remapped_vertex_indices: Vec<u32> = mesh_meshlets
                    .vertices
                    .iter()
                    .map(|&i| vertex_base + i)
                    .collect();
                all_meshlet_vertex_indices.extend(remapped_vertex_indices);

                // 3. The meshlet's `triangles` are u8 indices into its *own* vertex list
                //    (the one we just remapped). We can append these directly.
                //    The `triangle_offset` in the meshlet description will be relative
                //    to the start of this mesh's triangle data.
                let triangle_base = all_meshlet_triangle_indices.len() as u32;
                all_meshlet_triangle_indices.extend(&mesh_meshlets.triangles);

                // 4. Create descriptions for each meshlet with offsets into the global buffers.
                for m in &mesh_meshlets.meshlets {
                    let desc = MeshletDescription {
                        // The offset to the list of vertex indices for this meshlet.
                        vertex_list_offset: meshlet_vertex_index_base + m.vertex_offset,

                        // The offset to the list of triangle indices for this meshlet.
                        // This is relative to the start of the mesh's triangle data.
                        triangle_list_offset: triangle_base + m.triangle_offset,

                        triangle_count: m.triangle_count,
                        vertex_count: m.vertex_count,
                    };
                    all_meshlets.push(desc);

                    if !has_printed_debug_info {
                        println!("\n--- MESHLET DEBUG DUMP (First Meshlet) ---");
                        println!("Mesh Name: {}", mesh.name);
                        println!("[Raw Meshlet Data from meshopt]");
                        println!("  - vertex_offset: {}", m.vertex_offset);
                        println!("  - triangle_offset: {}", m.triangle_offset);
                        println!("  - vertex_count: {}", m.vertex_count);
                        println!("  - triangle_count: {}", m.triangle_count);
                        println!("[Generated MeshletDescription for GPU]");
                        println!("  - vertex_list_offset: {}", desc.vertex_list_offset);
                        println!("  - triangle_list_offset: {}", desc.triangle_list_offset);
                        println!("  - triangle_count: {}", desc.triangle_count);
                        println!("  - vertex_count: {}", desc.vertex_count);

                        let tri_start = (triangle_base + m.triangle_offset) as usize;
                        let first_tri_indices =
                            &all_meshlet_triangle_indices[tri_start..tri_start + 3];
                        println!("[First Triangle's Local Indices (from GLOBAL triangles buffer)]");
                        println!(
                            "  - Raw u8 values: [{}, {}, {}]",
                            first_tri_indices[0], first_tri_indices[1], first_tri_indices[2]
                        );

                        let vert_start = m.vertex_offset as usize;
                        let local_vtx_idx_1 = first_tri_indices[0] as usize;
                        let local_vtx_idx_2 = first_tri_indices[1] as usize;
                        let local_vtx_idx_3 = first_tri_indices[2] as usize;

                        let global_vtx_ptr_1 = mesh_meshlets.vertices[vert_start + local_vtx_idx_1];
                        let global_vtx_ptr_2 = mesh_meshlets.vertices[vert_start + local_vtx_idx_2];
                        let global_vtx_ptr_3 = mesh_meshlets.vertices[vert_start + local_vtx_idx_3];

                        println!("[Vertex Remapping (from vertices buffer)]");
                        println!(
                            "  - Local idx {} -> Pointer to Global Vertex {}",
                            local_vtx_idx_1, global_vtx_ptr_1
                        );
                        println!(
                            "  - Local idx {} -> Pointer to Global Vertex {}",
                            local_vtx_idx_2, global_vtx_ptr_2
                        );
                        println!(
                            "  - Local idx {} -> Pointer to Global Vertex {}",
                            local_vtx_idx_3, global_vtx_ptr_3
                        );

                        let final_vtx_1 = mesh.vertices[global_vtx_ptr_1 as usize];
                        let final_vtx_2 = mesh.vertices[global_vtx_ptr_2 as usize];
                        let final_vtx_3 = mesh.vertices[global_vtx_ptr_3 as usize];
                        println!("[Final Vertex Positions (from mesh's vertex buffer)]");
                        println!("  - Vertex 1: {:?}", final_vtx_1.position);
                        println!("  - Vertex 2: {:?}", final_vtx_2.position);
                        println!("  - Vertex 3: {:?}", final_vtx_3.position);
                        println!("----------------------------------------\n");
                        has_printed_debug_info = true;
                    }
                }
            } else {
                println!(
                    "  -> Mesh '{}' does NOT have meshlets. Skipping.",
                    mesh.name
                );
            }
        }
    }

    println!("[Asset Loading] Summary: {} models processed, {} meshes with meshlets found.", models_processed, meshes_with_meshlets);

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
    // If no vertices were loaded (e.g., no models with meshlets were found),
    // we cannot create valid GPU buffers. We will return early, and the render
    // system will check for this case and skip drawing.
    if asset_server.vertices.is_empty() {
        return;
    }

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

    // The `bytemuck::cast_slice` can panic if the byte slice is not aligned to
    // 4 bytes. To fix this, we manually and safely construct each u32 from
    // its 4 constituent bytes, which guarantees correctness regardless of the
    // underlying memory alignment.
    let packed_triangle_indices: Vec<u32> = padded_triangle_indices
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()))
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

    println!("--- Asset Server GPU Resources Created ---");
    println!("Total Vertices: {}", asset_server.vertices.len());
    println!(
        "Total Meshlet Vertex Indices: {}",
        asset_server.meshlet_vertex_indices.len()
    );
    println!(
        "Total Meshlet Triangle Indices (u8): {}",
        asset_server.meshlet_triangle_indices.len()
    );
    println!("Total Meshlets: {}", asset_server.meshlets.len());
    println!("Total Transforms: {}", asset_server.transforms.len());
    println!(
        "Total Draw Commands: {}",
        asset_server.draw_commands.len()
    );
    println!("------------------------------------------");
}
