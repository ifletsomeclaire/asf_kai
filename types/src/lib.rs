use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use redb::TableDefinition;
use serde::{Deserialize, Serialize};

pub const MODEL_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("models");
pub const TEXTURE_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("textures");
pub const ANIMATED_MODEL_TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new("animated_models");
pub const ANIMATION_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("animations");

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[repr(C)]
pub struct AABB {
    pub min: Vec4,
    pub max: Vec4,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Vertex {
    pub position: Vec4,
    pub normal: Vec4,
    pub uv: Vec2,
    pub _padding: [f32; 2],
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct SkinnedVertex {
    pub position: Vec4,
    pub normal: Vec4,
    pub uv: Vec2,
    pub _padding: [f32; 2],
    pub bone_indices: [u32; 4],
    pub bone_weights: [f32; 4],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mesh {
    pub name: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub texture_name: Option<String>,
    pub meshlets: Option<Meshlets>,
    pub aabb: AABB,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnimatedMesh {
    pub name: String,
    pub vertices: Vec<SkinnedVertex>,
    pub indices: Vec<u32>,
    pub texture_name: Option<String>,
    pub meshlets: Option<Meshlets>,
    pub aabb: AABB,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meshlet {
    pub vertex_offset: u32,
    pub triangle_offset: u32,
    pub vertex_count: u32,
    pub triangle_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meshlets {
    pub meshlets: Vec<Meshlet>,
    pub vertices: Vec<u32>, // indices into the original vertex buffer
    pub triangles: Vec<u8>, // 3 indices per triangle, into the meshlet's vertices
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Model {
    pub name: String,
    pub meshes: Vec<Mesh>,
    pub aabb: AABB,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Bone {
    pub name: String,
    pub parent_index: Option<usize>,
    pub transform: Mat4,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Keyframe {
    pub time: f32,
    pub rotations: Vec<Quat>,
    pub translations: Vec<Vec3>,
    pub scales: Vec<Vec3>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Animation {
    pub name: String,
    pub duration_in_ticks: f64,
    pub ticks_per_second: f64,
    pub keyframes: Vec<Keyframe>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AnimatedModel {
    pub name: String,
    pub meshes: Vec<AnimatedMesh>,
    pub skeleton: Skeleton,
    pub aabb: AABB,
}
