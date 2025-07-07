pub mod meshlets;



use bevy_ecs::{
    prelude::Resource,
    world::{FromWorld, World},
};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use redb::{ReadableTable, ReadableTableMetadata};
use types::{AABB, Model, Vertex, MODEL_TABLE, TEXTURE_TABLE};
use wgpu::util::DeviceExt;

use crate::renderer::core::WgpuDevice;

// Static asset data describing a slice of the geometry buffers.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct MeshletDescription {
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
    pub texture_id: u32,
    pub _padding: u32,
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
    pub texture_cpu_data: Vec<image::DynamicImage>,


    // GPU resources
    pub vertex_buffer: Option<wgpu::Buffer>,
    pub meshlet_vertex_index_buffer: Option<wgpu::Buffer>,
    pub meshlet_triangle_index_buffer: Option<wgpu::Buffer>,
    pub meshlet_description_buffer: Option<wgpu::Buffer>,
    pub transform_buffer: Option<wgpu::Buffer>,
    pub indirection_buffer: Option<wgpu::Buffer>,
    pub texture_array: Option<wgpu::Texture>,
    pub texture_sampler: Option<wgpu::Sampler>,

    pub mesh_bind_group_layout: Option<wgpu::BindGroupLayout>,
    pub mesh_bind_group: Option<wgpu::BindGroup>,
}

pub fn layout_models_in_a_row(aabbs: &[AABB]) -> Vec<Mat4> {
    let mut transforms = Vec::new();
    let mut current_x_offset = 0.0;
    let spacing = 1.0; // 1 unit of space between models

    for aabb in aabbs {
        let size = aabb.max - aabb.min;

        // The transform should position the model's starting edge (min.x) at the current offset.
        let transform = Mat4::from_translation(Vec3::new(
            current_x_offset - aabb.min.x,
            0.0,
            0.0,
        ));
        transforms.push(transform);

        // Update the offset for the next model.
        current_x_offset += size.x + spacing;
    }

    transforms
}

impl FromWorld for AssetServer {
    fn from_world(world: &mut bevy_ecs::world::World) -> Self {
        new(world)
    }
}

pub fn new(world: &mut World) -> AssetServer {
    let db = redb::Database::open("/Users/mewosmith/rust/asf_kai/assets/models.redb").unwrap();
    let read_txn = db.begin_read().unwrap();
    let model_table = read_txn.open_table(MODEL_TABLE).unwrap();
    let texture_table = read_txn.open_table(TEXTURE_TABLE).unwrap();
    println!("model table length: {:?}", model_table.len());
    println!("texture table length: {:?}", texture_table.len());

    let mut texture_map = std::collections::HashMap::new();
    let mut texture_cpu_data = Vec::new();

    // Create a fallback texture
    let fallback_texture = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 255, 255])));
    texture_cpu_data.push(fallback_texture);
    
    for result in texture_table.iter().unwrap() {
        let (name_bytes, texture_data) = result.unwrap();
        let name = name_bytes.value();
        println!("[Asset Loading] Loading texture: {}", name);
        if let Ok(image) = image::load_from_memory(texture_data.value()) {
            texture_map.insert(name.to_string(), texture_cpu_data.len() as u32);
            texture_cpu_data.push(image);
        }
    }


    let mut all_vertices = Vec::new();
    let mut all_meshlet_vertex_indices = Vec::<u32>::new();
    let mut all_meshlet_triangle_indices = Vec::new();
    let mut all_meshlets = Vec::new();
    let mut draw_commands: Vec<DrawCommand> = Vec::new();

    println!("[Asset Loading] Starting model processing...");

    let models: Vec<Model> = model_table
        .iter()
        .unwrap()
        .filter_map(|result| {
            result
                .ok()
                .and_then(|(_, model_data)| bincode::deserialize::<Model>(model_data.value()).ok())
        })
        .collect();

    let aabbs: Vec<AABB> = models.iter().map(|model| model.aabb).collect();
    let transforms = layout_models_in_a_row(&aabbs);

    let mut meshes_with_meshlets = 0;
    for (transform_id, model) in models.iter().enumerate() {
        println!("[Asset Loading] Processing model: {}", model.name);
        for mesh in &model.meshes {
            if let Some(mesh_meshlets) = &mesh.meshlets {
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

                let texture_id = mesh
                    .texture_name
                    .as_ref()
                    .and_then(|name| texture_map.get(name).copied())
                    .unwrap_or(0);

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

                    let draw_command = DrawCommand {
                        meshlet_id: (all_meshlets.len() - 1) as u32,
                        transform_id: transform_id as u32,
                        texture_id,
                        _padding: 0,
                    };
                    draw_commands.push(draw_command);
                }
            } else {
                println!(
                    "  -> Mesh '{}' does NOT have meshlets. Skipping.",
                    mesh.name
                );
            }
        }
    }

    println!(
        "[Asset Loading] Summary: {} models processed, {} meshes with meshlets found.",
        models.len(),
        meshes_with_meshlets
    );
    println!("[Asset Loading] Generated {} transforms.", transforms.len());
    println!("[Asset Loading] Generated {} draw commands.", draw_commands.len());

    let mut asset_server = AssetServer {
        vertices: all_vertices,
        meshlet_vertex_indices: all_meshlet_vertex_indices,
        meshlet_triangle_indices: all_meshlet_triangle_indices,
        meshlets: all_meshlets,
        transforms,
        draw_commands,
 
        texture_cpu_data,
        vertex_buffer: None,
        meshlet_vertex_index_buffer: None,
        meshlet_triangle_index_buffer: None,
        meshlet_description_buffer: None,
        transform_buffer: None,
        indirection_buffer: None,
        texture_array: None,
        texture_sampler: None,
        mesh_bind_group_layout: None,
        mesh_bind_group: None,
    };
    let device = world.resource::<WgpuDevice>();
    let queue = world.resource::<crate::renderer::core::WgpuQueue>();
    create_gpu_resources(&mut asset_server, &device, &queue);
    asset_server
}

fn create_gpu_resources(
    asset_server: &mut AssetServer,
    device: &wgpu::Device,
    queue: &crate::renderer::core::WgpuQueue,
) {
    // If no vertices were loaded (e.g., no models with meshlets were found),
    // we cannot create valid GPU buffers. We will return early, and the render
    // system will check for this case and skip drawing.
    if asset_server.vertices.is_empty() {
        return;
    }

    // Create texture array and sampler
    let (array_width, array_height) = (2048, 2048);
    let texture_size = wgpu::Extent3d {
        width: array_width,
        height: array_height,
        depth_or_array_layers: asset_server.texture_cpu_data.len() as u32,
    };
    let texture_array = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Texture Array"),
        size: texture_size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    for (i, image) in asset_server.texture_cpu_data.iter().enumerate() {
        let rgba_image = image
            .resize_to_fill(
                array_width,
                array_height,
                image::imageops::FilterType::Triangle,
            )
            .to_rgba8();
        let (width, height) = rgba_image.dimensions();
        queue.0.write_texture(
            wgpu::TexelCopyTextureInfo{
                texture: &texture_array,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: i as u32,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &rgba_image,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    asset_server.texture_array = Some(texture_array);
    asset_server.texture_sampler = Some(device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Texture Sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    }));


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
                // texture_array @binding(6)
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                // texture_sampler @binding(7)
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(
                        &asset_server
                            .texture_array
                            .as_ref()
                            .unwrap()
                            .create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(
                        asset_server.texture_sampler.as_ref().unwrap(),
                    ),
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
