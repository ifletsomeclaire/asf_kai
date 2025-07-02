use bevy_ecs::prelude::{Component, Resource};
use offset_allocator::{Allocation, Allocator};
use std::collections::HashMap;
use types::{Mesh as CpuMesh};
use uuid::Uuid;
use wgpu::util::DeviceExt;

use crate::ecs::model::Vertex;

// --- Handle IDs ---
// This is the public API for referencing an asset. It's a lightweight, copyable handle.

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Handle<T> {
    pub id: Uuid,
    _phantom: std::marker::PhantomData<T>,
}

impl<T> Handle<T> {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            _phantom: std::marker::PhantomData,
        }
    }
}

// --- GPU Asset Data Structs ---
// These are the internal representations of the assets as they exist on the GPU.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GpuTexture {
    /// The layer in the GPU texture array where this texture is stored.
    pub texture_array_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GpuMesh {
    pub name: String,
    /// The offset in bytes into the global GPU vertex buffer.
    pub vertex_buffer_offset: u64,
    pub vertex_count: u32,
    /// The offset in bytes into the global GPU index buffer.
    pub index_buffer_offset: u64,
    pub index_count: u32,
}

// --- Core GPU Resource Pools ---

struct GpuMeshPool {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    vertex_allocator: Allocator,
    index_allocator: Allocator,
}

struct GpuTexturePool {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    // A simple way to track free slots. A bitset would be more efficient.
    free_slots: Vec<u32>,
}

// --- The Central Asset Manager ---

#[derive(Resource)]
pub struct AssetServer {
    mesh_pool: GpuMeshPool,
    texture_pool: GpuTexturePool,

    // Central storage for all asset metadata.
    pub meshes: HashMap<Handle<GpuMesh>, (GpuMesh, Allocation, Allocation)>,
    pub textures: HashMap<Handle<GpuTexture>, GpuTexture>,

    // Reference counting to track how many entities use a given asset.
    mesh_ref_counts: HashMap<Handle<GpuMesh>, u32>,
    texture_ref_counts: HashMap<Handle<GpuTexture>, u32>,

    mesh_name_to_handle: HashMap<String, Handle<GpuMesh>>,
}

impl AssetServer {
    pub fn new(device: &wgpu::Device) -> Self {
        // Init Mesh Pool
        const VERTEX_BUFFER_SIZE: u32 = 1024 * 1024 * 128; // 128 MB
        const INDEX_BUFFER_SIZE: u32 = 1024 * 1024 * 64; // 64 MB
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("global_vertex_buffer"),
            size: VERTEX_BUFFER_SIZE as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("global_index_buffer"),
            size: INDEX_BUFFER_SIZE as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let mesh_pool = GpuMeshPool {
            vertex_buffer,
            index_buffer,
            vertex_allocator: Allocator::new(VERTEX_BUFFER_SIZE),
            index_allocator: Allocator::new(INDEX_BUFFER_SIZE),
        };

        // Init Texture Pool
        const TEXTURE_ARRAY_SIZE: u32 = 256;
        const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
        const TEXTURE_DIMENSION: u32 = 1024;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("global_texture_array"),
            size: wgpu::Extent3d {
                width: TEXTURE_DIMENSION,
                height: TEXTURE_DIMENSION,
                depth_or_array_layers: TEXTURE_ARRAY_SIZE,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEXTURE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        let texture_pool = GpuTexturePool {
            texture,
            view,
            sampler,
            free_slots: (0..TEXTURE_ARRAY_SIZE).rev().collect(),
        };

        Self {
            mesh_pool,
            texture_pool,
            meshes: HashMap::new(),
            textures: HashMap::new(),
            mesh_ref_counts: HashMap::new(),
            texture_ref_counts: HashMap::new(),
            mesh_name_to_handle: HashMap::new(),
        }
    }

    pub fn load_mesh(&mut self, cpu_mesh: &CpuMesh, queue: &wgpu::Queue) -> Option<Handle<GpuMesh>> {
        let vertices_with_padding: Vec<Vertex> = cpu_mesh.vertices.iter().map(|v| {
            Vertex {
                position: [v.position.x, v.position.y, v.position.z, 1.0],
            }
        }).collect();

        let vertex_data = bytemuck::cast_slice(&vertices_with_padding);
        let index_data = bytemuck::cast_slice(&cpu_mesh.indices);

        let vertex_alloc = self.mesh_pool.vertex_allocator.allocate(vertex_data.len() as u32)?;
        let index_alloc = self.mesh_pool.index_allocator.allocate(index_data.len() as u32)?;

        queue.write_buffer(&self.mesh_pool.vertex_buffer, vertex_alloc.offset.into(), vertex_data);
        queue.write_buffer(&self.mesh_pool.index_buffer, index_alloc.offset.into(), index_data);

        let handle = Handle::<GpuMesh>::new();
        let gpu_mesh = GpuMesh {
            name: cpu_mesh.name.clone(),
            vertex_buffer_offset: vertex_alloc.offset.into(),
            vertex_count: cpu_mesh.vertices.len() as u32,
            index_buffer_offset: index_alloc.offset.into(),
            index_count: cpu_mesh.indices.len() as u32,
        };

        self.meshes.insert(handle.clone(), (gpu_mesh, vertex_alloc, index_alloc));
        self.mesh_ref_counts.insert(handle.clone(), 1);

        Some(handle)
    }

    /// Called by a system when an entity with a mesh handle is despawned.
    pub fn unload_mesh(&mut self, handle: &Handle<GpuMesh>) {
        if let Some(count) = self.mesh_ref_counts.get_mut(handle) {
            *count -= 1;
            if *count == 0 {
                if let Some((_gpu_mesh, vertex_alloc, index_alloc)) = self.meshes.remove(handle) {
                    self.mesh_pool.vertex_allocator.free(vertex_alloc);
                    self.mesh_pool.index_allocator.free(index_alloc);
                    self.mesh_ref_counts.remove(handle);
                    println!("Unloaded mesh and freed GPU memory for handle {:?}", handle.id);
                }
            }
        }
    }

    pub fn get_vertex_buffer(&self) -> &wgpu::Buffer {
        &self.mesh_pool.vertex_buffer
    }

    pub fn get_index_buffer(&self) -> &wgpu::Buffer {
        &self.mesh_pool.index_buffer
    }

    pub fn get_mesh_handle(&self, mesh_name: &str) -> Option<&Handle<GpuMesh>> {
        self.mesh_name_to_handle.get(mesh_name)
    }

    pub fn register_mesh_handle(&mut self, mesh_name: &str, handle: Handle<GpuMesh>) {
        self.mesh_name_to_handle.insert(mesh_name.to_string(), handle);
    }

    pub fn get_gpu_mesh(&self, handle: &Handle<GpuMesh>) -> Option<&GpuMesh> {
        self.meshes.get(handle).map(|(gpu_mesh, _, _)| gpu_mesh)
    }
} 