use bevy_ecs::prelude::*;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use russimp::{
    node::Node,
    scene::{PostProcess, Scene},
};
use walkdir::WalkDir;
use wgpu::util::DeviceExt;

use crate::renderer::{core::WgpuDevice, d3_pipeline::D3Pipeline};

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MeshInfo {
    pub transform: [Vec4; 4],
    pub index_count: u32,
    pub first_index: u32,
    pub base_vertex: u32,
    pub _padding: u32,
}

#[derive(Resource)]
pub struct SceneData {
    pub mesh_bind_group: wgpu::BindGroup,
    pub total_indices: u32,
}

fn get_model_transform(node: &Node) -> Mat4 {
    let t = node.transformation;
    Mat4::from_cols_array(&[
        t.a1, t.b1, t.c1, t.d1, t.a2, t.b2, t.c2, t.d2, t.a3, t.b3, t.c3, t.d3, t.a4, t.b4, t.c4, t.d4,
    ])
}

fn process_node_for_rendering_recursive(
    node: &Node,
    parent_transform: &Mat4,
    scene: &Scene,
    all_vertices: &mut Vec<Vertex>,
    all_indices: &mut Vec<u32>,
    mesh_infos: &mut Vec<MeshInfo>,
    draw_index_to_mesh_id: &mut Vec<u32>,
) {
    let current_transform = *parent_transform * get_model_transform(node);

    for &mesh_index in &node.meshes {
        let mesh = &scene.meshes[mesh_index as usize];
        println!(
            "Processing mesh: {}, num_vertices: {}, num_faces: {}",
            mesh.name,
            mesh.vertices.len(),
            mesh.faces.len()
        );
        let base_vertex = all_vertices.len() as u32;
        let first_index = all_indices.len() as u32;

        let mesh_vertices: Vec<Vertex> = mesh
            .vertices
            .iter()
            .map(|v| Vertex {
                position: [v.x, v.y, v.z, 1.0],
            })
            .collect();

        let mesh_indices: Vec<u32> = mesh.faces.iter().flat_map(|f| f.0.clone()).collect();
        let index_count = mesh_indices.len() as u32;

        all_vertices.extend(mesh_vertices);
        all_indices.extend(mesh_indices);

        let mesh_id = mesh_infos.len() as u32;
        for _ in 0..index_count {
            draw_index_to_mesh_id.push(mesh_id);
        }

        mesh_infos.push(MeshInfo {
            transform: current_transform.to_cols_array_2d().map(Vec4::from),
            index_count,
            first_index,
            base_vertex,
            _padding: 0,
        });
    }

    for child in node.children.borrow().iter() {
        process_node_for_rendering_recursive(
            child,
            &current_transform,
            scene,
            all_vertices,
            all_indices,
            mesh_infos,
            draw_index_to_mesh_id,
        );
    }
}

pub fn load_model_system(
    mut commands: Commands,
    device: Res<WgpuDevice>,
    pipeline: Res<D3Pipeline>,
) {
    let mut all_vertices: Vec<Vertex> = Vec::new();
    let mut all_indices: Vec<u32> = Vec::new();
    let mut mesh_infos: Vec<MeshInfo> = Vec::new();
    let mut draw_index_to_mesh_id: Vec<u32> = Vec::new();

    let model_paths = WalkDir::new("assets/models")
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension().map_or(false, |ext| {
                ext.to_str() == Some("gltf") || ext.to_str() == Some("glb")
            })
        })
        .map(|e| e.path().to_owned());

    let mut model_count = 0;
    for path in model_paths {
        log::info!("Attempting to load model from: {:?}", path);
        model_count += 1;
        let scene = match Scene::from_file(
            path.to_str().unwrap(),
            vec![
                PostProcess::Triangulate,
                PostProcess::JoinIdenticalVertices,
            ],
        ) {
            Ok(scene) => scene,
            Err(err) => {
                log::error!("Failed to load scene {:?}: {}", path, err);
                continue;
            }
        };

        let model_world_transform = Mat4::IDENTITY;
        
        if let Some(root) = &scene.root {
            process_node_for_rendering_recursive(
                root,
                &model_world_transform,
                &scene,
                &mut all_vertices,
                &mut all_indices,
                &mut mesh_infos,
                &mut draw_index_to_mesh_id,
            );
        }
    }

    let total_indices = all_indices.len() as u32;

    println!("Total vertices: {}", all_vertices.len());
    println!("Total indices: {}", total_indices);
    println!("Total meshes: {}", mesh_infos.len());
    if !mesh_infos.is_empty() {
        println!("First mesh info: {:?}", mesh_infos[0]);
    }
    if !all_vertices.is_empty() {
        println!(
            "First 5 vertices: {:?}",
            &all_vertices[..5.min(all_vertices.len())]
        );
    }
    if !all_indices.is_empty() {
        println!(
            "First 36 indices: {:?}",
            &all_indices[..36.min(all_indices.len())]
        );
    }
    if !draw_index_to_mesh_id.is_empty() {
        println!(
            "First 36 draw_index_to_mesh_id: {:?}",
            &draw_index_to_mesh_id[..36.min(draw_index_to_mesh_id.len())]
        );
    }

    if total_indices == 0 {
        log::warn!("No models loaded, no indices found.");
        return;
    }

    let vertex_buffer = device
        .0
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Global Vertex Buffer"),
            contents: bytemuck::cast_slice(&all_vertices),
            usage: wgpu::BufferUsages::STORAGE,
        });

    let index_buffer = device
        .0
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Global Index Buffer"),
            contents: bytemuck::cast_slice(&all_indices),
            usage: wgpu::BufferUsages::STORAGE,
        });

    let mesh_info_buffer =
        device
            .0
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Mesh Info Buffer"),
                contents: bytemuck::cast_slice(&mesh_infos),
                usage: wgpu::BufferUsages::STORAGE,
            });

    let draw_index_to_mesh_id_buffer =
        device
            .0
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Draw Index to Mesh ID Buffer"),
                contents: bytemuck::cast_slice(&draw_index_to_mesh_id),
                usage: wgpu::BufferUsages::STORAGE,
            });

    let mesh_bind_group = device.0.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("scene_data_bind_group"),
        layout: &pipeline.mesh_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: vertex_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: index_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: mesh_info_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: draw_index_to_mesh_id_buffer.as_entire_binding(),
            },
        ],
    });

    commands.insert_resource(SceneData {
        mesh_bind_group,
        total_indices,
    });
    log::info!(
        "Loaded {} meshes from {} models, with {} total vertices and {} total indices",
        mesh_infos.len(),
        model_count,
        all_vertices.len(),
        total_indices
    );
} 