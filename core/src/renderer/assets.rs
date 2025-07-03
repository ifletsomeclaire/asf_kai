use bevy_ecs::prelude::{Component, Resource};
use image::{DynamicImage, GenericImageView};
use offset_allocator::{Allocation, Allocator};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use types::Mesh as CpuMesh;

use crate::ecs::model::Vertex;

// --- Handle IDs ---
// Plain, copyable handles. The u64 is a unique, stable ID.

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureHandle(pub u64);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MeshHandle(pub u64);

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

    // Manual reference counting and allocation tracking
    mesh_ref_counts: HashMap<u64, usize>,
    mesh_allocations: HashMap<u64, (Allocation, Allocation)>,
    texture_ref_counts: HashMap<u64, usize>,

    next_mesh_id: AtomicU64,
    next_texture_id: AtomicU64,

    // Central storage for all asset metadata.
    pub meshes: HashMap<u64, GpuMesh>,
    pub textures: HashMap<u64, GpuTexture>,

    mesh_name_to_handle: HashMap<String, MeshHandle>,
    texture_name_to_handle: HashMap<String, TextureHandle>,
}

impl AssetServer {
    pub fn new(device: &wgpu::Device) -> Self {
        // Init Mesh Pool
        const VERTEX_BUFFER_SIZE: u32 = 1024 * 1024 * 128; // 128 MB
        const INDEX_BUFFER_SIZE: u32 = 1024 * 1024 * 64; // 64 MB
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("global_vertex_buffer"),
            size: VERTEX_BUFFER_SIZE as u64,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("global_index_buffer"),
            size: INDEX_BUFFER_SIZE as u64,
            usage: wgpu::BufferUsages::INDEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::STORAGE,
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
        const TEXTURE_DIMENSION: u32 = 512;
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
            mesh_ref_counts: HashMap::new(),
            mesh_allocations: HashMap::new(),
            texture_ref_counts: HashMap::new(),
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
        let handle = MeshHandle(id);

        let gpu_mesh = GpuMesh {
            name: cpu_mesh.name.clone(),
            vertex_buffer_offset: vertex_alloc.offset.into(),
            vertex_count: cpu_mesh.vertices.len() as u32,
            index_buffer_offset: index_alloc.offset.into(),
            index_count: cpu_mesh.indices.len() as u32,
        };

        self.meshes.insert(id, gpu_mesh.clone());
        self.mesh_name_to_handle
            .insert(cpu_mesh.name.clone(), handle);
        self.mesh_allocations
            .insert(id, (vertex_alloc, index_alloc));
        self.mesh_ref_counts.insert(id, 0); // Start with zero refs

        Some((handle, gpu_mesh))
    }

    pub fn load_texture(
        &mut self,
        image: &DynamicImage,
        queue: &wgpu::Queue,
        name: &str,
    ) -> Option<TextureHandle> {
        // All textures must be the same size to fit into the texture array.
        // We resize them to the dimension specified during the texture pool's creation.
        let dimensions = self.texture_pool.texture.size();
        let resized_image = image.resize_exact(
            dimensions.width,
            dimensions.height,
            image::imageops::FilterType::Lanczos3,
        );
        let rgba = resized_image.to_rgba8();
        let size = wgpu::Extent3d {
            width: dimensions.width,
            height: dimensions.height,
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
            &rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * dimensions.width),
                rows_per_image: Some(dimensions.height),
            },
            size,
        );

        let id = self.next_texture_id.fetch_add(1, Ordering::Relaxed);
        let handle = TextureHandle(id);

        self.textures.insert(
            id,
            GpuTexture {
                texture_array_index,
            },
        );
        self.texture_name_to_handle
            .insert(name.to_string(), handle);
        self.texture_ref_counts.insert(id, 0); // Start with zero refs

        Some(handle)
    }

    // --- Ref Counting Methods ---

    pub fn increment_mesh_ref(&mut self, mesh_id: u64) {
        *self.mesh_ref_counts.entry(mesh_id).or_insert(0) += 1;
    }

    pub fn decrement_mesh_ref(&mut self, mesh_id: u64) {
        let count = self.mesh_ref_counts.entry(mesh_id).or_insert(1);
        *count -= 1;
        if *count == 0 {
            // Deallocate the mesh
            if let Some((vertex_alloc, index_alloc)) = self.mesh_allocations.remove(&mesh_id) {
                self.mesh_pool.vertex_allocator.free(vertex_alloc);
                self.mesh_pool.index_allocator.free(index_alloc);
            }
            if let Some(gpu_mesh) = self.meshes.remove(&mesh_id) {
                self.mesh_name_to_handle.remove(&gpu_mesh.name);
            }
            self.mesh_ref_counts.remove(&mesh_id);
        }
    }

    pub fn increment_texture_ref(&mut self, texture_id: u64) {
        *self.texture_ref_counts.entry(texture_id).or_insert(0) += 1;
    }

    pub fn decrement_texture_ref(&mut self, texture_id: u64) {
        let count = self.texture_ref_counts.entry(texture_id).or_insert(1);
        *count -= 1;
        if *count == 0 {
            // Deallocate the texture
            if let Some(gpu_texture) = self.textures.remove(&texture_id) {
                self.texture_pool
                    .free_slots
                    .push(gpu_texture.texture_array_index);

                // Find and remove the corresponding name mapping
                let mut name_to_remove = None;
                for (name, handle) in &self.texture_name_to_handle {
                    if handle.0 == texture_id {
                        name_to_remove = Some(name.clone());
                        break;
                    }
                }
                if let Some(name) = name_to_remove {
                    self.texture_name_to_handle.remove(&name);
                }
            }
            self.texture_ref_counts.remove(&texture_id);
        }
    }

    // --- Accessors ---

    pub fn get_vertex_buffer(&self) -> &wgpu::Buffer {
        &self.mesh_pool.vertex_buffer
    }

    pub fn get_index_buffer(&self) -> &wgpu::Buffer {
        &self.mesh_pool.index_buffer
    }

    pub fn get_mesh_handle(&self, mesh_name: &str) -> Option<MeshHandle> {
        self.mesh_name_to_handle.get(mesh_name).copied()
    }

    pub fn register_mesh_handle(&mut self, mesh_name: &str, handle: MeshHandle) {
        self.mesh_name_to_handle
            .insert(mesh_name.to_string(), handle);
    }

    pub fn get_texture_handle(&self, texture_name: &str) -> Option<TextureHandle> {
        self.texture_name_to_handle.get(texture_name).copied()
    }

    pub fn register_texture_handle(&mut self, texture_name: &str, handle: TextureHandle) {
        self.texture_name_to_handle
            .insert(texture_name.to_string(), handle);
    }

    pub fn get_gpu_mesh(&self, handle: MeshHandle) -> Option<&GpuMesh> {
        self.meshes.get(&handle.0)
    }

    pub fn get_gpu_texture(&self, handle: TextureHandle) -> Option<&GpuTexture> {
        self.textures.get(&handle.0)
    }

    pub fn get_texture_view(&self) -> &wgpu::TextureView {
        &self.texture_pool.view
    }

    pub fn get_texture_sampler(&self) -> &wgpu::Sampler {
        &self.texture_pool.sampler
    }
}
