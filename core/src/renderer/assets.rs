use bevy_ecs::prelude::{Component, Resource};
use image::{DynamicImage};
use offset_allocator::{Allocation, Allocator};
use redb::{Database, ReadableTable, TableDefinition};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use types::{Model as TypesModel, Mesh as CpuMesh};

use crate::ecs::model::Vertex;

// A unique ticket to validate a specific load request
pub type LoadToken = u64;

// The message containing data from the background task
pub enum AssetLoadResult {
    Mesh {
        name: String,
        cpu_mesh: CpuMesh,
        token: LoadToken, // The ticket to prevent race conditions
    },
    Texture {
        name: String,
        image: DynamicImage,
        token: LoadToken,
    },
}

// The state of a given asset name
pub enum LoadingStatus {
    Loading { id: u64, token: LoadToken },
    Loaded(u64),
}

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
    // --- Lock-Free Communication ---
    asset_load_sender: crossbeam_channel::Sender<AssetLoadResult>,
    pub asset_load_receiver: crossbeam_channel::Receiver<AssetLoadResult>,

    // --- State Tracking ---
    pub mesh_load_state: HashMap<String, LoadingStatus>,
    pub texture_load_state: HashMap<String, LoadingStatus>,

    // --- GPU Resource Pools ---
    pub mesh_pool: GpuMeshPool,
    texture_pool: GpuTexturePool,

    // --- Other fields ---
    db: Option<Arc<Database>>,
    pub fallback_gpu_mesh: Option<GpuMesh>,
    pub fallback_gpu_texture: Option<GpuTexture>,

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
    next_load_token: AtomicU64,
    
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

        let (asset_load_sender, asset_load_receiver) = crossbeam_channel::unbounded();

        Self {
            mesh_pool,
            texture_pool,
            db: None,
            fallback_gpu_mesh: None,
            fallback_gpu_texture: None,
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
            next_load_token: AtomicU64::new(0),
            all_mesh_names: Vec::new(),
            all_texture_names: Vec::new(),
            asset_load_sender,
            asset_load_receiver,
            mesh_load_state: HashMap::new(),
            texture_load_state: HashMap::new(),
        }
    }

    pub fn create_and_upload_fallback_assets(&mut self, queue: &wgpu::Queue) {
        // --- Fallback Mesh (a simple cube) ---
        let fallback_cpu_mesh = CpuMesh {
            name: "fallback_cube".to_string(),
            vertices: vec![
                // -Z
                types::Vertex { position: [-0.5, -0.5, -0.5].into(), normal: [0.0, 0.0, -1.0].into(), uv: [0.0, 0.0].into() },
                types::Vertex { position: [0.5, -0.5, -0.5].into(), normal: [0.0, 0.0, -1.0].into(), uv: [1.0, 0.0].into() },
                types::Vertex { position: [0.5, 0.5, -0.5].into(), normal: [0.0, 0.0, -1.0].into(), uv: [1.0, 1.0].into() },
                types::Vertex { position: [-0.5, 0.5, -0.5].into(), normal: [0.0, 0.0, -1.0].into(), uv: [0.0, 1.0].into() },
                // +Z
                types::Vertex { position: [-0.5, -0.5, 0.5].into(), normal: [0.0, 0.0, 1.0].into(), uv: [0.0, 0.0].into() },
                types::Vertex { position: [0.5, -0.5, 0.5].into(), normal: [0.0, 0.0, 1.0].into(), uv: [1.0, 0.0].into() },
                types::Vertex { position: [0.5, 0.5, 0.5].into(), normal: [0.0, 0.0, 1.0].into(), uv: [1.0, 1.0].into() },
                types::Vertex { position: [-0.5, 0.5, 0.5].into(), normal: [0.0, 0.0, 1.0].into(), uv: [0.0, 1.0].into() },
                // -X
                types::Vertex { position: [-0.5, -0.5, -0.5].into(), normal: [-1.0, 0.0, 0.0].into(), uv: [0.0, 0.0].into() },
                types::Vertex { position: [-0.5, 0.5, -0.5].into(), normal: [-1.0, 0.0, 0.0].into(), uv: [1.0, 0.0].into() },
                types::Vertex { position: [-0.5, 0.5, 0.5].into(), normal: [-1.0, 0.0, 0.0].into(), uv: [1.0, 1.0].into() },
                types::Vertex { position: [-0.5, -0.5, 0.5].into(), normal: [-1.0, 0.0, 0.0].into(), uv: [0.0, 1.0].into() },
                // +X
                types::Vertex { position: [0.5, -0.5, -0.5].into(), normal: [1.0, 0.0, 0.0].into(), uv: [0.0, 0.0].into() },
                types::Vertex { position: [0.5, 0.5, -0.5].into(), normal: [1.0, 0.0, 0.0].into(), uv: [1.0, 0.0].into() },
                types::Vertex { position: [0.5, 0.5, 0.5].into(), normal: [1.0, 0.0, 0.0].into(), uv: [1.0, 1.0].into() },
                types::Vertex { position: [0.5, -0.5, 0.5].into(), normal: [1.0, 0.0, 0.0].into(), uv: [0.0, 1.0].into() },
                // -Y
                types::Vertex { position: [-0.5, -0.5, -0.5].into(), normal: [0.0, -1.0, 0.0].into(), uv: [0.0, 0.0].into() },
                types::Vertex { position: [0.5, -0.5, -0.5].into(), normal: [0.0, -1.0, 0.0].into(), uv: [1.0, 0.0].into() },
                types::Vertex { position: [0.5, -0.5, 0.5].into(), normal: [0.0, -1.0, 0.0].into(), uv: [1.0, 1.0].into() },
                types::Vertex { position: [-0.5, -0.5, 0.5].into(), normal: [0.0, -1.0, 0.0].into(), uv: [0.0, 1.0].into() },
                // +Y
                types::Vertex { position: [-0.5, 0.5, -0.5].into(), normal: [0.0, 1.0, 0.0].into(), uv: [0.0, 0.0].into() },
                types::Vertex { position: [0.5, 0.5, -0.5].into(), normal: [0.0, 1.0, 0.0].into(), uv: [1.0, 0.0].into() },
                types::Vertex { position: [0.5, 0.5, 0.5].into(), normal: [0.0, 1.0, 0.0].into(), uv: [1.0, 1.0].into() },
                types::Vertex { position: [-0.5, 0.5, 0.5].into(), normal: [0.0, 1.0, 0.0].into(), uv: [0.0, 1.0].into() },
            ],
            indices: vec![
                0, 1, 2, 0, 2, 3, // -Z
                4, 5, 6, 4, 6, 7, // +Z
                8, 9, 10, 8, 10, 11, // -X
                12, 13, 14, 12, 14, 15, // +X
                16, 17, 18, 16, 18, 19, // -Y
                20, 21, 22, 20, 22, 23, // +Y
            ],
            texture_name: Some("fallback_texture".to_string()),
        };
        let fallback_mesh_id = self.next_mesh_id.fetch_add(1, Ordering::Relaxed);
        if self.upload_mesh_to_gpu(fallback_mesh_id, &fallback_cpu_mesh, queue) {
            self.fallback_gpu_mesh = self.meshes.get(&fallback_mesh_id).cloned();
        }

        // --- Fallback Texture (a 1x1 white pixel) ---
        let fallback_image = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255])));
        let fallback_texture_id = self.next_texture_id.fetch_add(1, Ordering::Relaxed);
        if self.upload_texture_to_gpu(fallback_texture_id, &fallback_image, queue, "fallback_texture") {
            self.fallback_gpu_texture = self.textures.get(&fallback_texture_id).cloned();
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
        self.db = Some(Arc::new(db));
    }

    pub fn get_mesh_handle(&mut self, name: &str) -> MeshHandle {
        if let Some(status) = self.mesh_load_state.get(name) {
            let id = match status {
                LoadingStatus::Loading { id, .. } => *id,
                LoadingStatus::Loaded(id) => *id,
            };
            return MeshHandle(id);
        }

        // It's a new request.
        let id = self
            .mesh_id_free_list
            .pop()
            .unwrap_or_else(|| self.next_mesh_id.fetch_add(1, Ordering::Relaxed));
        let token = self.next_load_token.fetch_add(1, Ordering::Relaxed);
        let handle = MeshHandle(id);

        self.mesh_load_state
            .insert(name.to_string(), LoadingStatus::Loading { id, token });

        // --- Spawn Background Task ---
        let sender = self.asset_load_sender.clone();
        let db = self.db.as_ref().unwrap().clone();
        let name_owned = name.to_string();

        std::thread::spawn(move || {
            let read_txn = db.begin_read().unwrap();
            let table = read_txn.open_table(MODEL_TABLE).unwrap();

            for item in table.iter().unwrap().flatten() {
                if let Ok(model) = bincode::deserialize::<TypesModel>(item.1.value()) {
                    for mesh in &model.meshes {
                        if mesh.name == name_owned {
                            sender
                                .send(AssetLoadResult::Mesh {
                                    name: name_owned,
                                    cpu_mesh: mesh.clone(),
                                    token,
                                })
                                .unwrap();
                            return;
                        }
                    }
                }
            }
        });

        handle
    }

    pub fn get_texture_handle(&mut self, name: &str) -> TextureHandle {
        if let Some(status) = self.texture_load_state.get(name) {
            let id = match status {
                LoadingStatus::Loading { id, .. } => *id,
                LoadingStatus::Loaded(id) => *id,
            };
            return TextureHandle(id);
        }

        let id = self
            .texture_id_free_list
            .pop()
            .unwrap_or_else(|| self.next_texture_id.fetch_add(1, Ordering::Relaxed));
        let token = self.next_load_token.fetch_add(1, Ordering::Relaxed);
        let handle = TextureHandle(id);

        self.texture_load_state
            .insert(name.to_string(), LoadingStatus::Loading { id, token });

        // --- Spawn Background Task ---
        let sender = self.asset_load_sender.clone();
        let db = self.db.as_ref().unwrap().clone();
        let name_owned = name.to_string();

        std::thread::spawn(move || {
            let read_txn = db.begin_read().unwrap();
            let table = read_txn.open_table(TEXTURE_TABLE).unwrap();

            if let Ok(Some(data)) = table.get(name_owned.as_str()) {
                if let Ok(image) = image::load_from_memory(data.value()) {
                    sender
                        .send(AssetLoadResult::Texture {
                            name: name_owned,
                            image,
                            token,
                        })
                        .unwrap();
                }
            }
        });

        handle
    }

    fn upload_mesh_to_gpu(&mut self, id: u64, cpu_mesh: &CpuMesh, queue: &wgpu::Queue) -> bool {
        let vertices: Vec<Vertex> = cpu_mesh
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

        let vertex_data = bytemuck::cast_slice(&vertices);
        let index_data = bytemuck::cast_slice(&cpu_mesh.indices);

        let Some(vertex_alloc) = self.mesh_pool.vertex_allocator.allocate(vertex_data.len() as u32) else { return false; };
        let Some(index_alloc) = self.mesh_pool.index_allocator.allocate(index_data.len() as u32) else { return false; };

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

        let handle = MeshHandle(id);
        let gpu_mesh = GpuMesh {
            name: cpu_mesh.name.clone(),
            vertex_buffer_offset: vertex_alloc.offset.into(),
            vertex_count: cpu_mesh.vertices.len() as u32,
            index_buffer_offset: index_alloc.offset.into(),
            index_count: cpu_mesh.indices.len() as u32,
        };

        self.meshes.insert(id, gpu_mesh);
        self.mesh_name_to_handle
            .insert(cpu_mesh.name.clone(), handle);
        self.mesh_allocations.insert(id, (vertex_alloc, index_alloc));
        self.mesh_ref_counts.insert(id, 0);

        true
    }

    fn upload_texture_to_gpu(
        &mut self,
        id: u64,
        image: &DynamicImage,
        queue: &wgpu::Queue,
        name: &str,
    ) -> bool {
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
        let Some(texture_array_index) = self.texture_pool.free_slots.pop() else { return false; };

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

        let handle = TextureHandle(id);
        self.textures
            .insert(id, GpuTexture { texture_array_index });
        self.texture_name_to_handle
            .insert(name.to_string(), handle);
        self.texture_ref_counts.insert(id, 0);

        true
    }

    pub fn finish_loading_mesh(
        &mut self,
        name: String,
        cpu_mesh: CpuMesh,
        token: LoadToken,
        queue: &wgpu::Queue,
    ) {
        if let Some(LoadingStatus::Loading { id, token: stored_token }) = self.mesh_load_state.get(&name) {
            if *stored_token == token {
                let id = *id;
                if self.upload_mesh_to_gpu(id, &cpu_mesh, queue) {
                    self.mesh_load_state
                        .insert(name, LoadingStatus::Loaded(id));
                } else {
                    // How to handle allocation failure? For now, just remove it.
                    self.mesh_load_state.remove(&name);
                    self.mesh_id_free_list.push(id);
                }
            }
        }
    }

    pub fn finish_loading_texture(
        &mut self,
        name: String,
        image: DynamicImage,
        token: LoadToken,
        queue: &wgpu::Queue,
    ) {
        if let Some(LoadingStatus::Loading { id, token: stored_token }) = self.texture_load_state.get(&name) {
            if *stored_token == token {
                let id = *id;
                let name_str = name.as_str();
                if self.upload_texture_to_gpu(id, &image, queue, name_str) {
                    self.texture_load_state
                        .insert(name, LoadingStatus::Loaded(id));
                } else {
                    self.texture_load_state.remove(&name);
                    self.texture_id_free_list.push(id);
                }
            }
        }
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
