use glam::{Vec2, Vec4};
use redb::TableDefinition;
use serde::{Deserialize, Serialize};

pub const MODEL_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("models");
pub const TEXTURE_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("textures");



#[derive(Debug, Clone, Copy, Serialize, Deserialize, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Vertex {
    pub position: Vec4,
    pub normal: Vec4,
    pub uv: Vec2,
    pub _padding: [f32; 2],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mesh {
    pub name: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub texture_name: Option<String>,
    pub meshlets: Option<Meshlets>,
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
}
