use bevy_ecs::prelude::{Component, Resource};
use crossbeam_channel::{Receiver, Sender};
use image::{DynamicImage, GenericImageView};
use offset_allocator::{Allocation, Allocator};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use types::{Mesh as CpuMesh};

use crate::ecs::model::Vertex;

pub enum DeallocationMessage {
    Mesh(Allocation, Allocation), // Vertex and Index allocations
                                  // Texture(u32), // Texture array index
}

// --- Handle IDs ---

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureHandle(u64);

struct GpuMeshHandleInner {
    id: u64,
    vertex_alloc: Allocation,
    index_alloc: Allocation,
    deallocator: Sender<DeallocationMessage>,
}

impl Drop for GpuMeshHandleInner {
    fn drop(&mut self) {
        // This is it! When the last Arc is dropped, this code runs.
        // We send the allocations to the AssetServer for freeing.
        // The `send` might fail if the receiver is already dropped, which is fine.
        let _ = self.deallocator.send(DeallocationMessage::Mesh(
            self.vertex_alloc.clone(), // Allocation is cheap to clone
            self.index_alloc.clone(),
        ));
    }
}

#[derive(Component, Clone)]
pub struct MeshHandle {
    inner: Arc<GpuMeshHandleInner>,
}

impl MeshHandle {
    pub fn id(&self) -> u64 {
        self.inner.id
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

pub struct GpuMeshPool {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    pub vertex_allocator: Allocator,
    pub index_allocator: Allocator,
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
    pub mesh_pool: GpuMeshPool,
    texture_pool: GpuTexturePool,

    deallocation_sender: Sender<DeallocationMessage>,
    pub deallocation_receiver: Receiver<DeallocationMessage>,

    next_mesh_id: AtomicU64,
    next_texture_id: AtomicU64,

    // Central storage for all asset metadata.
    pub meshes: HashMap<u64, GpuMesh>,
    pub textures: HashMap<TextureHandle, GpuTexture>,

    mesh_name_to_handle: HashMap<String, MeshHandle>,
    texture_name_to_handle: HashMap<String, TextureHandle>,
}

impl AssetServer {
    pub fn new(device: &wgpu::Device) -> Self {
        let (sender, receiver) = crossbeam_channel::unbounded();

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
            deallocation_sender: sender,
            deallocation_receiver: receiver,
            meshes: HashMap::new(),
            textures: HashMap::new(),
            mesh_name_to_handle: HashMap::new(),
            texture_name_to_handle: HashMap::new(),
            next_mesh_id: AtomicU64::new(0),
            next_texture_id: AtomicU64::new(0),
        }
    }

    pub fn load_mesh(
        &mut self,
        cpu_mesh: &CpuMesh,
        queue: &wgpu::Queue,
    ) -> Option<(MeshHandle, GpuMesh)> {
        let vertices_with_padding: Vec<Vertex> = cpu_mesh
            .vertices
            .iter()
            .map(|v| Vertex {
                position: [v.position.x, v.position.y, v.position.z, 1.0],
                normal: [v.normal.x, v.normal.y, v.normal.z],
                _padding1: 0,
                tex_coords: [v.uv.x, v.uv.y],
                _padding2: [0; 2],
            })
            .collect();

        let vertex_data = bytemuck::cast_slice(&vertices_with_padding);
        let index_data = bytemuck::cast_slice(&cpu_mesh.indices);

        let vertex_alloc = self
            .mesh_pool
            .vertex_allocator
            .allocate(vertex_data.len() as u32)?;
        let index_alloc = self
            .mesh_pool
            .index_allocator
            .allocate(index_data.len() as u32)?;

        queue.write_buffer(
            &self.mesh_pool.vertex_buffer,
            vertex_alloc.offset.into(),
            vertex_data,
        );
        queue.write_buffer(
            &self.mesh_pool.index_buffer,
            index_alloc.offset.into(),
            index_data,
        );

        let id = self.next_mesh_id.fetch_add(1, Ordering::Relaxed);

        let handle_inner = GpuMeshHandleInner {
            id,
            vertex_alloc: vertex_alloc.clone(),
            index_alloc: index_alloc.clone(),
            deallocator: self.deallocation_sender.clone(),
        };

        let handle = MeshHandle {
            inner: Arc::new(handle_inner),
        };

        let gpu_mesh = GpuMesh {
            name: cpu_mesh.name.clone(),
            vertex_buffer_offset: vertex_alloc.offset.into(),
            vertex_count: cpu_mesh.vertices.len() as u32,
            index_buffer_offset: index_alloc.offset.into(),
            index_count: cpu_mesh.indices.len() as u32,
        };

        self.meshes.insert(id, gpu_mesh.clone());

        Some((handle, gpu_mesh))
    }

    pub fn load_texture(
        &mut self,
        image: &DynamicImage,
        queue: &wgpu::Queue,
    ) -> Option<TextureHandle> {
        let texture_data = image.to_rgba8();
        let dimensions = image.dimensions();

        let texture_extent = wgpu::Extent3d {
            width: dimensions.0,
            height: dimensions.1,
            depth_or_array_layers: 1,
        };

        let texture_array_index = self.texture_pool.free_slots.pop()?;

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.texture_pool.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: texture_array_index,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &texture_data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * dimensions.0),
                rows_per_image: Some(dimensions.1),
            },
            texture_extent,
        );

        let handle = TextureHandle(self.next_texture_id.fetch_add(1, Ordering::Relaxed));
        let gpu_texture = GpuTexture {
            texture_array_index,
        };

        self.textures.insert(handle, gpu_texture);

        Some(handle)
    }

    pub fn get_vertex_buffer(&self) -> &wgpu::Buffer {
        &self.mesh_pool.vertex_buffer
    }

    pub fn get_index_buffer(&self) -> &wgpu::Buffer {
        &self.mesh_pool.index_buffer
    }

    pub fn get_mesh_handle(&self, mesh_name: &str) -> Option<&MeshHandle> {
        self.mesh_name_to_handle.get(mesh_name)
    }

    pub fn register_mesh_handle(&mut self, mesh_name: &str, handle: MeshHandle) {
        self.mesh_name_to_handle.insert(mesh_name.to_string(), handle);
    }

    pub fn get_texture_handle(&self, texture_name: &str) -> Option<&TextureHandle> {
        self.texture_name_to_handle.get(texture_name)
    }

    pub fn register_texture_handle(&mut self, texture_name: &str, handle: TextureHandle) {
        self.texture_name_to_handle
            .insert(texture_name.to_string(), handle);
    }

    pub fn get_gpu_mesh(&self, handle: &MeshHandle) -> Option<&GpuMesh> {
        self.meshes.get(&handle.id())
    }

    pub fn get_gpu_texture(&self, handle: &TextureHandle) -> Option<&GpuTexture> {
        self.textures.get(handle)
    }

    pub fn get_texture_view(&self) -> &wgpu::TextureView {
        &self.texture_pool.view
    }

    pub fn get_texture_sampler(&self) -> &wgpu::Sampler {
        &self.texture_pool.sampler
    }
} 