use bevy_ecs::prelude::{Component, Resource};
use image::{DynamicImage};
use offset_allocator::{Allocation, Allocator};
use redb::{Database, ReadableTable, TableDefinition};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use types::{Model as TypesModel, Mesh as CpuMesh};

use crate::ecs::model::Vertex;

const MODEL_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("models");
const TEXTURE_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("textures");

// --- Handle IDs ---
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureHandle(pub u64);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MeshHandle(pub u64);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MaterialHandle(pub u64);

// --- GPU & CPU Asset Data Structs ---

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GpuTexture {
    pub texture_array_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GpuMesh {
    pub name: String,
    pub vertex_buffer_offset: u64,
    pub vertex_count: u32,
    pub index_buffer_offset: u64,
    pub index_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Material {
    pub mesh_handle: MeshHandle,
    pub texture_handle: TextureHandle,
}

// --- Core GPU Resource Pools ---

pub struct GpuMeshPool {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub vertex_allocator: Allocator,
    pub index_allocator: Allocator,
}

struct GpuTexturePool {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    free_slots: Vec<u32>,
}

// --- The Central Asset Manager ---

#[derive(Resource)]
pub struct AssetServer {
    pub mesh_pool: GpuMeshPool,
    texture_pool: GpuTexturePool,
    db: Option<Database>,

    // --- Reference Counting ---
    mesh_ref_counts: HashMap<u64, usize>,
    texture_ref_counts: HashMap<u64, usize>,
    material_ref_counts: HashMap<u64, usize>,
    
    // --- GPU Buffer Allocations ---
    mesh_allocations: HashMap<u64, (Allocation, Allocation)>,

    // --- Free Lists for ID reuse ---
    mesh_id_free_list: Vec<u64>,
    texture_id_free_list: Vec<u64>,

    // --- ID Generation ---
    next_mesh_id: AtomicU64,
    next_texture_id: AtomicU64,
    next_material_id: AtomicU64,
    
    // --- Asset Storage (now with Options to allow for "holes") ---
    pub meshes: HashMap<u64, GpuMesh>,
    pub textures: HashMap<u64, GpuTexture>,
    pub materials: HashMap<u64, Material>,

    // --- Name to Handle Mapping ---
    mesh_name_to_handle: HashMap<String, MeshHandle>,
    texture_name_to_handle: HashMap<String, TextureHandle>,
    material_lookup: HashMap<(u64, u64), MaterialHandle>,
    
    // --- For UI ---
    all_mesh_names: Vec<String>,
    all_texture_names: Vec<String>,
}

impl AssetServer {
    pub fn new(device: &wgpu::Device) -> Self {
        const VERTEX_BUFFER_SIZE: u32 = 1024 * 1024 * 128;
        const INDEX_BUFFER_SIZE: u32 = 1024 * 1024 * 64;
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
            db: None,
            mesh_ref_counts: HashMap::new(),
            texture_ref_counts: HashMap::new(),
            material_ref_counts: HashMap::new(),
            mesh_allocations: HashMap::new(),
            // Initialize empty free lists
            mesh_id_free_list: Vec::new(),
            texture_id_free_list: Vec::new(),
            meshes: HashMap::new(),
            textures: HashMap::new(),
            materials: HashMap::new(),
            mesh_name_to_handle: HashMap::new(),
            texture_name_to_handle: HashMap::new(),
            material_lookup: HashMap::new(),
            next_mesh_id: AtomicU64::new(0),
            next_texture_id: AtomicU64::new(0),
            next_material_id: AtomicU64::new(0),
            all_mesh_names: Vec::new(),
            all_texture_names: Vec::new(),
        }
    }

    pub fn set_db(&mut self, db: Database) {
        let read_txn = db.begin_read().unwrap();
        if let Ok(table) = read_txn.open_table(MODEL_TABLE) {
            self.all_mesh_names = table
                .iter()
                .unwrap()
                .flatten()
                .filter_map(|item| bincode::deserialize::<TypesModel>(item.1.value()).ok())
                .flat_map(|model| model.meshes.into_iter().map(|mesh| mesh.name))
                .collect();
        }
        if let Ok(table) = read_txn.open_table(TEXTURE_TABLE) {
            self.all_texture_names = table
                .iter()
                .unwrap()
                .flatten()
                .map(|item| item.0.value().to_string())
                .collect();
        }
        self.db = Some(db);
    }

    // --- Allocation now uses the free list ---
    fn load_mesh(&mut self, cpu_mesh: &CpuMesh, queue: &wgpu::Queue) -> Option<MeshHandle> {
        let vertices: Vec<Vertex> = cpu_mesh.vertices.iter().map(|v| Vertex {
            position: [v.position.x, v.position.y, v.position.z, 1.0],
            normal: [v.normal.x, v.normal.y, v.normal.z],
            _padding1: 0,
            tex_coords: [v.uv.x, v.uv.y],
            _padding2: [0; 2],
        }).collect();

        let vertex_data = bytemuck::cast_slice(&vertices);
        let index_data = bytemuck::cast_slice(&cpu_mesh.indices);

        let vertex_alloc = self.mesh_pool.vertex_allocator.allocate(vertex_data.len() as u32)?;
        let index_alloc = self.mesh_pool.index_allocator.allocate(index_data.len() as u32)?;

        queue.write_buffer(&self.mesh_pool.vertex_buffer, vertex_alloc.offset.into(), vertex_data);
        queue.write_buffer(&self.mesh_pool.index_buffer, index_alloc.offset.into(), index_data);

        // Get an ID: either from the free list or by creating a new one.
        let id = self.mesh_id_free_list.pop().unwrap_or_else(|| self.next_mesh_id.fetch_add(1, Ordering::Relaxed));
        let handle = MeshHandle(id);

        let gpu_mesh = GpuMesh {
            name: cpu_mesh.name.clone(),
            vertex_buffer_offset: vertex_alloc.offset.into(),
            vertex_count: cpu_mesh.vertices.len() as u32,
            index_buffer_offset: index_alloc.offset.into(),
            index_count: cpu_mesh.indices.len() as u32,
        };

        self.meshes.insert(id, gpu_mesh);
        self.mesh_name_to_handle.insert(cpu_mesh.name.clone(), handle);
        self.mesh_allocations.insert(id, (vertex_alloc, index_alloc));
        self.mesh_ref_counts.insert(id, 0);

        Some(handle)
    }

    fn load_texture(&mut self, image: &DynamicImage, queue: &wgpu::Queue, name: &str) -> Option<TextureHandle> {
        let dimensions = self.texture_pool.texture.size();
        let resized_image = image.resize_exact(dimensions.width, dimensions.height, image::imageops::FilterType::Lanczos3);
        let rgba = resized_image.to_rgba8();
        let size = wgpu::Extent3d {
            width: dimensions.width,
            height: dimensions.height,
            depth_or_array_layers: 1,
        };
        let texture_array_index = self.texture_pool.free_slots.pop()?;
        
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture_pool.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: 0, z: texture_array_index },
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * dimensions.width),
                rows_per_image: Some(dimensions.height),
            },
            size,
        );
        
        // Get an ID from the free list or create a new one.
        let id = self.texture_id_free_list.pop().unwrap_or_else(|| self.next_texture_id.fetch_add(1, Ordering::Relaxed));
        let handle = TextureHandle(id);

        self.textures.insert(id, GpuTexture { texture_array_index });
        self.texture_name_to_handle.insert(name.to_string(), handle);
        self.texture_ref_counts.insert(id, 0);

        Some(handle)
    }

    pub fn get_mesh_handle(&mut self, name: &str, queue: &wgpu::Queue) -> Option<MeshHandle> {
        if let Some(handle) = self.mesh_name_to_handle.get(name) {
            return Some(*handle);
        }

        let db = self.db.as_ref()?;
        let read_txn = db.begin_read().ok()?;
        let table = read_txn.open_table(MODEL_TABLE).ok()?;

        for item in table.iter().ok()?.flatten() {
            if let Ok(model) = bincode::deserialize::<TypesModel>(item.1.value()) {
                for mesh in &model.meshes {
                    if mesh.name == name {
                        println!("[CORE] Lazily loaded mesh '{}'", mesh.name);
                        return self.load_mesh(mesh, queue);
                    }
                }
            }
        }
        None
    }

    pub fn get_texture_handle(&mut self, name: &str, queue: &wgpu::Queue) -> Option<TextureHandle> {
        if let Some(handle) = self.texture_name_to_handle.get(name) {
            return Some(*handle);
        }

        let db = self.db.as_ref()?;
        let read_txn = db.begin_read().ok()?;
        let table = read_txn.open_table(TEXTURE_TABLE).ok()?;

        if let Ok(Some(data)) = table.get(name) {
             if let Ok(image) = image::load_from_memory(data.value()) {
                println!("[CORE] Lazily loaded texture '{}'", name);
                return self.load_texture(&image, queue, name);
             }
        }
        None
    }

    pub fn get_or_create_material(&mut self, mesh_handle: MeshHandle, texture_handle: TextureHandle) -> MaterialHandle {
        let key = (mesh_handle.0, texture_handle.0);
        if let Some(handle) = self.material_lookup.get(&key) {
            return *handle;
        }

        let material_id = self.next_material_id.fetch_add(1, Ordering::Relaxed);
        let material_handle = MaterialHandle(material_id);
        let material = Material { mesh_handle, texture_handle };

        self.materials.insert(material_id, material);
        self.material_ref_counts.insert(material_id, 0);
        self.material_lookup.insert(key, material_handle);

        material_handle
    }

    pub fn increment_material_ref(&mut self, material_id: u64) {
        let count = self.material_ref_counts.entry(material_id).or_insert(0);
        *count += 1;
        
        if *count == 1 {
            if let Some(material) = self.materials.get(&material_id) {
                *self.mesh_ref_counts.entry(material.mesh_handle.0).or_insert(0) += 1;
                *self.texture_ref_counts.entry(material.texture_handle.0).or_insert(0) += 1;
            }
        }
    }

    pub fn decrement_material_ref(&mut self, material_id: u64) {
        if let Some(count) = self.material_ref_counts.get_mut(&material_id) {
            *count -= 1;
            if *count == 0 {
                if let Some(material) = self.materials.remove(&material_id) {
                    self.decrement_mesh_ref(material.mesh_handle.0);
                    self.decrement_texture_ref(material.texture_handle.0);
                    self.material_lookup.remove(&(material.mesh_handle.0, material.texture_handle.0));
                }
                self.material_ref_counts.remove(&material_id);
            }
        }
    }

    // --- De-allocation now uses the free list ---
    
    fn decrement_mesh_ref(&mut self, mesh_id: u64) {
        if let Some(count) = self.mesh_ref_counts.get_mut(&mesh_id) {
            *count -= 1;
            if *count == 0 {
                // Free the buffer space
                if let Some((vertex_alloc, index_alloc)) = self.mesh_allocations.remove(&mesh_id) {
                    self.mesh_pool.vertex_allocator.free(vertex_alloc);
                    self.mesh_pool.index_allocator.free(index_alloc);
                }
                // Remove the GPU mesh data
                if let Some(gpu_mesh) = self.meshes.remove(&mesh_id) {
                    self.mesh_name_to_handle.remove(&gpu_mesh.name);
                }
                self.mesh_ref_counts.remove(&mesh_id);
                
                // **Add the ID to the free list for reuse!**
                self.mesh_id_free_list.push(mesh_id);
            }
        }
    }

    fn decrement_texture_ref(&mut self, texture_id: u64) {
        if let Some(count) = self.texture_ref_counts.get_mut(&texture_id) {
            *count -= 1;
            if *count == 0 {
                // Free the texture array slot
                if let Some(gpu_texture) = self.textures.remove(&texture_id) {
                    self.texture_pool.free_slots.push(gpu_texture.texture_array_index);
                }
                // ... (removing from texture_name_to_handle is the same)
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
                
                self.texture_ref_counts.remove(&texture_id);

                // **Add the ID to the free list for reuse!**
                self.texture_id_free_list.push(texture_id);
            }
        }
    }

    pub fn get_vertex_buffer(&self) -> &wgpu::Buffer { &self.mesh_pool.vertex_buffer }
    pub fn get_index_buffer(&self) -> &wgpu::Buffer { &self.mesh_pool.index_buffer }
    pub fn get_texture_view(&self) -> &wgpu::TextureView { &self.texture_pool.view }
    pub fn get_texture_sampler(&self) -> &wgpu::Sampler { &self.texture_pool.sampler }
    
    pub fn get_mesh_names(&self) -> Vec<String> { self.all_mesh_names.clone() }
    pub fn get_texture_names(&self) -> Vec<String> { self.all_texture_names.clone() }
    
    pub fn get_gpu_mesh(&self, handle: MeshHandle) -> Option<&GpuMesh> { self.meshes.get(&handle.0) }
    pub fn get_gpu_texture(&self, handle: TextureHandle) -> Option<&GpuTexture> { self.textures.get(&handle.0) }
}
