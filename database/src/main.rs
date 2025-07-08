use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use meshopt::{build_meshlets, VertexDataAdapter};
use redb::Database;
use russimp::material::Texture;
use russimp::{
    material::TextureType,
    node::Node,
    scene::{PostProcess, Scene},
};
use std::cell::Ref;
use std::collections::HashMap;

use types::{
    AnimatedMesh, AnimatedModel, Animation, Bone, Mesh, Meshlet, Meshlets, Model,
    Skeleton, SkinnedVertex, AABB,
};
use types::{ANIMATED_MODEL_TABLE, ANIMATION_TABLE, MODEL_TABLE, TEXTURE_TABLE};

fn process_node(
    node_rc: &Node,
    scene: &Scene,
    parent_transform: &glam::Mat4,
    model_name: &str,
    new_textures_to_add: &mut Vec<(String, Vec<u8>)>,
) -> Vec<Mesh> {
    let node_transform = glam::Mat4::from_cols_array(&[
        node_rc.transformation.a1, node_rc.transformation.a2, node_rc.transformation.a3,
        node_rc.transformation.a4, node_rc.transformation.b1, node_rc.transformation.b2,
        node_rc.transformation.b3, node_rc.transformation.b4, node_rc.transformation.c1,
        node_rc.transformation.c2, node_rc.transformation.c3, node_rc.transformation.c4,
        node_rc.transformation.d1, node_rc.transformation.d2, node_rc.transformation.d3,
        node_rc.transformation.d4,
    ])
    .transpose();
    let accumulated_transform = *parent_transform * node_transform;

    let mut meshes = Vec::new();

    for &mesh_index in &node_rc.meshes {
        let mesh = &scene.meshes[mesh_index as usize];
        let material = &scene.materials[mesh.material_index as usize];
        let mut texture_name = None;
        let unique_mesh_name = format!("{model_name}-mesh-{mesh_index}");

        let texture_to_use = material
            .textures
            .get(&TextureType::Diffuse)
            .or_else(|| material.textures.get(&TextureType::Emissive));

        if let Some(texture_ref) = texture_to_use {
            let texture: Ref<Texture> = (**texture_ref).borrow();
            match &texture.data {
                russimp::material::DataContent::Bytes(bytes) => {
                    let texture_type_str = if material.textures.contains_key(&TextureType::Diffuse) {
                        "diffuse"
                    } else {
                        "emissive"
                    };
                    let new_name = format!("{unique_mesh_name}_{texture_type_str}.png");

                    new_textures_to_add.push((new_name.clone(), bytes.clone()));
                    texture_name = Some(new_name);
                }
                russimp::material::DataContent::Texel(_) => {
                    eprintln!("Found uncompressed texel data for model '{model_name}'. This is not currently supported for embedded textures.");
                }
            }
        } else {
            for prop in &material.properties {
                if prop.key.contains("$tex.file") {
                    if let russimp::material::PropertyTypeInfo::String(path) = &prop.data {
                        texture_name = Path::new(path)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .map(String::from);
                        if texture_name.is_some() {
                            break;
                        }
                    }
                }
            }
        }

        println!(
            "[DB] Mesh: '{unique_mesh_name}' -> Found texture: {texture_name:?}"
        );

        let vertices: Vec<types::Vertex> = mesh
            .vertices
            .iter()
            .zip(mesh.normals.iter())
            .zip(
                mesh.texture_coords[0]
                    .clone()
                    .unwrap_or_default()
                    .iter(),
            )
            .map(|((v, n), uv)| {
                let pos = glam::Vec3::new(v.x, v.y, v.z);
                let normal = glam::Vec3::new(n.x, n.y, n.z);
                let transformed_pos = accumulated_transform.transform_point3(pos);
                let transformed_normal = accumulated_transform.transform_vector3(normal);

                types::Vertex {
                    position: transformed_pos.extend(1.0),
                    normal: transformed_normal.extend(0.0),
                    uv: glam::vec2(uv.x, 1.0 - uv.y),
                    _padding: [0.0; 2],
                }
            })
            .collect();

        let indices: Vec<u32> = mesh.faces.iter().flat_map(|f| f.0.clone()).collect();

        const MAX_VERTICES: usize = 64;
        const MAX_TRIANGLES: usize = 128;

        let vertex_stride = std::mem::size_of::<types::Vertex>();
        let vertex_data_bytes = bytemuck::cast_slice(&vertices);

        let adapter = VertexDataAdapter::new(vertex_data_bytes, vertex_stride, 0).unwrap();
        let meshlets_result =
            build_meshlets(&indices, &adapter, MAX_VERTICES, MAX_TRIANGLES, 0.0);

        let meshlets = if !meshlets_result.meshlets.is_empty() {
            println!(
                "[DB] Mesh: '{}' -> Generated {} meshlets",
                unique_mesh_name,
                meshlets_result.meshlets.len()
            );

            let converted_meshlets = meshlets_result
                .meshlets
                .iter()
                .map(|m| Meshlet {
                    vertex_offset: m.vertex_offset,
                    triangle_offset: m.triangle_offset,
                    vertex_count: m.vertex_count,
                    triangle_count: m.triangle_count,
                })
                .collect();

            Some(Meshlets {
                meshlets: converted_meshlets,
                vertices: meshlets_result.vertices,
                triangles: meshlets_result.triangles,
            })
        } else {
            None
        };

        let mut mesh_aabb = AABB::default();
        if let Some(first_vtx) = vertices.first() {
            mesh_aabb.min = first_vtx.position;
            mesh_aabb.max = first_vtx.position;
            for v in vertices.iter().skip(1) {
                mesh_aabb.min = mesh_aabb.min.min(v.position);
                mesh_aabb.max = mesh_aabb.max.max(v.position);
            }
        }

        meshes.push(Mesh {
            name: unique_mesh_name,
            vertices,
            indices,
            texture_name,
            meshlets,
            aabb: mesh_aabb,
        });
    }

    for child_rc in node_rc.children.borrow().iter() {
        meshes.extend(process_node(
            child_rc,
            scene,
            &accumulated_transform,
            model_name,
            new_textures_to_add,
        ));
    }

    meshes
}

pub struct ModelDatabase {
    db: Database,
}

impl ModelDatabase {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let db = Database::create(path)?;
        Ok(Self { db })
    }

    pub fn populate_from_assets<P: AsRef<Path>>(
        &self,
        assets_dir: P,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let write_txn = self.db.begin_write()?;
        {
            let mut model_table = write_txn.open_table(MODEL_TABLE)?;
            let mut texture_table = write_txn.open_table(TEXTURE_TABLE)?;
            let mut animated_model_table = write_txn.open_table(ANIMATED_MODEL_TABLE)?;
            let mut animation_table = write_txn.open_table(ANIMATION_TABLE)?;

            fn visit_dir(
                dir: &Path,
                model_table: &mut redb::Table<&str, &[u8]>,
                texture_table: &mut redb::Table<&str, &[u8]>,
                animated_model_table: &mut redb::Table<&str, &[u8]>,
                animation_table: &mut redb::Table<&str, &[u8]>,
            ) -> Result<(), Box<dyn std::error::Error>> {
                if dir.is_dir() {
                    for entry in fs::read_dir(dir)? {
                        let entry = entry?;
                        let path = entry.path();
                        if path.is_dir() {
                            visit_dir(
                                &path,
                                model_table,
                                texture_table,
                                animated_model_table,
                                animation_table,
                            )?;
                        } else {
                            process_file(
                                &path,
                                model_table,
                                texture_table,
                                animated_model_table,
                                animation_table,
                            )?;
                        }
                    }
                }
                Ok(())
            }

            fn process_file(
                path: &Path,
                model_table: &mut redb::Table<&str, &[u8]>,
                texture_table: &mut redb::Table<&str, &[u8]>,
                animated_model_table: &mut redb::Table<&str, &[u8]>,
                animation_table: &mut redb::Table<&str, &[u8]>,
            ) -> Result<(), Box<dyn std::error::Error>> {
                let extension = path.extension().and_then(|s| s.to_str());
                let file_name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();

                match extension {
                    Some("gltf") | Some("glb") => {
                        let model_name = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("unknown_model");

                        println!("Processing model: {model_name}");

                        let scene = Scene::from_file(
                            path.to_str().unwrap(),
                            vec![
                                PostProcess::Triangulate,
                                PostProcess::JoinIdenticalVertices,
                                PostProcess::GenerateSmoothNormals,
                            ],
                        )?;

                        if scene.animations.is_empty() {
                            let mut new_textures_to_add = Vec::new();

                            let meshes = if let Some(root) = &scene.root {
                                process_node(
                                    root,
                                    &scene,
                                    &glam::Mat4::IDENTITY,
                                    model_name,
                                    &mut new_textures_to_add,
                                )
                            } else {
                                Vec::new()
                            };

                            let mut model_aabb = AABB::default();
                            if let Some(first_mesh) = meshes.first() {
                                model_aabb = first_mesh.aabb;
                                for mesh in meshes.iter().skip(1) {
                                    model_aabb.min = model_aabb.min.min(mesh.aabb.min);
                                    model_aabb.max = model_aabb.max.max(mesh.aabb.max);
                                }
                            }

                            let model = Model {
                                name: model_name.to_string(),
                                meshes,
                                aabb: model_aabb,
                            };
                            let encoded_model = bincode::serialize(&model)?;
                            model_table.insert(model_name, encoded_model.as_slice())?;

                            for (texture_name, texture_data) in new_textures_to_add {
                                texture_table.insert(texture_name.as_str(), texture_data.as_slice())?;
                            }
                        } else {
                            process_animated_scene(
                                &scene,
                                model_name,
                                animated_model_table,
                                animation_table,
                                texture_table,
                            )?;
                        }
                    }
                    Some("png") => {
                        println!("Processing texture: {file_name}");
                        let texture_bytes = fs::read(path)?;
                        texture_table.insert(file_name, texture_bytes.as_slice())?;
                    }
                    _ => {
                        // Skip other file types
                    }
                }
                Ok(())
            }

            visit_dir(assets_dir.as_ref(), &mut model_table, &mut texture_table, &mut animated_model_table, &mut animation_table)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn get_model(&self, name: &str) -> Result<Option<Model>, Box<dyn std::error::Error>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(MODEL_TABLE)?;
        match table.get(name)? {
            Some(guard) => {
                let bytes = guard.value();
                let model: Model = bincode::deserialize(bytes)?;
                Ok(Some(model))
            }
            None => Ok(None),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting database populator");

    let mut workspace_root = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    workspace_root.pop(); // Go up to the workspace root from the crate root

    let db_path = workspace_root.join("assets/models.redb");
    let assets_path = workspace_root.join("assets/models");

    let db = ModelDatabase::new(&db_path)?;
    db.populate_from_assets(&assets_path)?;
    println!("Database populated successfully from {assets_path:?}");

    // Example of retrieving a model
    if let Some(model) = db.get_model("cube")? {
        println!(
            "Successfully retrieved model 'cube' with {} meshes.",
            model.meshes.len()
        );
        for mesh in &model.meshes {
            println!("    - Mesh: {}", mesh.name);
            println!("      - Vertices: {}", mesh.vertices.len());
            println!("      - Indices: {}", mesh.indices.len());
            if let Some(meshlets) = &mesh.meshlets {
                println!("      - Meshlets: {}", meshlets.meshlets.len());
            }
            // Print first 3 vertices for inspection
            for (j, v) in mesh.vertices.iter().take(3).enumerate() {
                println!(
                    "      - Vertex {}: [{}, {}, {}]",
                    j, v.position.x, v.position.y, v.position.z
                );
            }
        }
    } else {
        eprintln!("Could not retrieve model 'cube'");
    }

    Ok(())
}

fn process_animated_scene(
    scene: &Scene,
    model_name: &str,
    animated_model_table: &mut redb::Table<&str, &[u8]>,
    animation_table: &mut redb::Table<&str, &[u8]>,
    texture_table: &mut redb::Table<&str, &[u8]>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("[DB] Processing animated model: {model_name}");

    // 1. Build skeleton and bone map
    let mut bones = Vec::new();
    let mut bone_map = HashMap::new();

    fn build_skeleton_recursive(
        node: &Node,
        parent_index: Option<usize>,
        bones: &mut Vec<Bone>,
        bone_map: &mut HashMap<String, usize>,
    ) {
        let bone_name = node.name.clone();
        let bone_index = bones.len();
        bone_map.insert(bone_name.clone(), bone_index);

        let transform = glam::Mat4::from_cols_array(&[
            node.transformation.a1,
            node.transformation.a2,
            node.transformation.a3,
            node.transformation.a4,
            node.transformation.b1,
            node.transformation.b2,
            node.transformation.b3,
            node.transformation.b4,
            node.transformation.c1,
            node.transformation.c2,
            node.transformation.c3,
            node.transformation.c4,
            node.transformation.d1,
            node.transformation.d2,
            node.transformation.d3,
            node.transformation.d4,
        ])
        .transpose();

        bones.push(Bone {
            name: bone_name,
            parent_index,
            transform,
            inverse_bind_pose: glam::Mat4::IDENTITY, // Will be filled in later
        });

        for child in &*node.children.borrow() {
            build_skeleton_recursive(child, Some(bone_index), bones, bone_map);
        }
    }

    if let Some(root) = &scene.root {
        build_skeleton_recursive(root, None, &mut bones, &mut bone_map);
    }

    // 1.5. Populate inverse bind poses
    for mesh in &scene.meshes {
        for r_bone in &mesh.bones {
            if let Some(&bone_index) = bone_map.get(&r_bone.name) {
                let inverse_bind_pose = glam::Mat4::from_cols_array(&[
                    r_bone.offset_matrix.a1,
                    r_bone.offset_matrix.a2,
                    r_bone.offset_matrix.a3,
                    r_bone.offset_matrix.a4,
                    r_bone.offset_matrix.b1,
                    r_bone.offset_matrix.b2,
                    r_bone.offset_matrix.b3,
                    r_bone.offset_matrix.b4,
                    r_bone.offset_matrix.c1,
                    r_bone.offset_matrix.c2,
                    r_bone.offset_matrix.c3,
                    r_bone.offset_matrix.c4,
                    r_bone.offset_matrix.d1,
                    r_bone.offset_matrix.d2,
                    r_bone.offset_matrix.d3,
                    r_bone.offset_matrix.d4,
                ])
                .transpose();
                bones[bone_index].inverse_bind_pose = inverse_bind_pose;
            }
        }
    }

    let skeleton = Skeleton { bones };

    // 2. Process animations
    for anim in &scene.animations {
        let mut channels = Vec::new();
        let ticks_per_second = if anim.ticks_per_second > 0.0 {
            anim.ticks_per_second
        } else {
            25.0 // Default to 25 FPS
        };

        for channel in &anim.channels {
            let position_keys = channel
                .position_keys
                .iter()
                .map(|pk| types::PositionKey {
                    time: pk.time,
                    position: glam::vec3(pk.value.x, pk.value.y, pk.value.z),
                })
                .collect();

            let rotation_keys = channel
                .rotation_keys
                .iter()
                .map(|rk| types::RotationKey {
                    time: rk.time,
                    rotation: glam::quat(rk.value.x, rk.value.y, rk.value.z, rk.value.w),
                })
                .collect();

            let scale_keys = channel
                .scaling_keys
                .iter()
                .map(|sk| types::ScaleKey {
                    time: sk.time,
                    scale: glam::vec3(sk.value.x, sk.value.y, sk.value.z),
                })
                .collect();

            channels.push(types::AnimationChannel {
                bone_name: channel.name.clone(),
                position_keys,
                rotation_keys,
                scale_keys,
            });
        }

        let animation = Animation {
            name: anim.name.clone(),
            duration_in_ticks: anim.duration,
            ticks_per_second,
            channels,
        };
        let encoded_animation = bincode::serialize(&animation)?;
        animation_table.insert(anim.name.as_str(), encoded_animation.as_slice())?;
    }

    // 3. Process meshes
    let mut animated_meshes = Vec::new();
    for (mesh_index, mesh) in scene.meshes.iter().enumerate() {
        let mut vertex_bone_data: Vec<Vec<(u32, f32)>> = vec![Vec::new(); mesh.vertices.len()];
        for r_bone in &mesh.bones {
            let bone_name = r_bone.name.clone();
            if let Some(&bone_index) = bone_map.get(&bone_name) {
                for weight in &r_bone.weights {
                    vertex_bone_data[weight.vertex_id as usize].push((bone_index as u32, weight.weight));
                }
            }
        }

        let vertices: Vec<SkinnedVertex> = mesh
            .vertices
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let mut bone_indices = [0u32; 4];
                let mut bone_weights = [0.0f32; 4];
                let bone_data = &vertex_bone_data[i];
                for (j, (index, weight)) in bone_data.iter().enumerate().take(4) {
                    bone_indices[j] = *index;
                    bone_weights[j] = *weight;
                }
                
                let normal = if let Some(n) = mesh.normals.get(i) {
                    glam::vec4(n.x, n.y, n.z, 0.0)
                } else {
                    glam::vec4(0.0, 0.0, 0.0, 0.0)
                };

                SkinnedVertex {
                    position: glam::vec4(v.x, v.y, v.z, 1.0),
                    normal,
                    uv: if let Some(uvs) = &mesh.texture_coords[0] {
                        glam::vec2(uvs[i].x, 1.0 - uvs[i].y)
                    } else {
                        glam::vec2(0.0, 0.0)
                    },
                    bone_indices,
                    bone_weights,
                    _padding: [0.0; 2],
                }
            })
            .collect();

        let indices: Vec<u32> = mesh.faces.iter().flat_map(|f| f.0.clone()).collect();

        let meshlets = {
            const MAX_VERTICES: usize = 64;
            const MAX_TRIANGLES: usize = 128;
            let vertex_stride = std::mem::size_of::<SkinnedVertex>();
            let vertex_data_bytes = bytemuck::cast_slice(&vertices);
            let adapter = VertexDataAdapter::new(vertex_data_bytes, vertex_stride, 0).unwrap();
            let meshlets_result = build_meshlets(&indices, &adapter, MAX_VERTICES, MAX_TRIANGLES, 0.0);
            let converted_meshlets = meshlets_result
                .meshlets
                .iter()
                .map(|m| Meshlet {
                    vertex_offset: m.vertex_offset,
                    triangle_offset: m.triangle_offset,
                    vertex_count: m.vertex_count,
                    triangle_count: m.triangle_count,
                })
                .collect();
            Some(Meshlets {
                meshlets: converted_meshlets,
                vertices: meshlets_result.vertices,
                triangles: meshlets_result.triangles,
            })
        };

        let mut aabb = AABB::default();
        if let Some(first_vtx) = vertices.first() {
            aabb.min = first_vtx.position;
            aabb.max = first_vtx.position;
            for v in vertices.iter().skip(1) {
                aabb.min = aabb.min.min(v.position);
                aabb.max = aabb.max.max(v.position);
            }
        }
        
        let material = &scene.materials[mesh.material_index as usize];
        let mut texture_name = None;

        if let Some(texture_ref) = material.textures.get(&TextureType::Diffuse) {
            let texture: Ref<Texture> = (**texture_ref).borrow();
            if let russimp::material::DataContent::Bytes(bytes) = &texture.data {
                let new_name = format!("{model_name}-mesh-{mesh_index}_diffuse.png");
                texture_table.insert(new_name.as_str(), bytes.as_slice())?;
                texture_name = Some(new_name);
            }
        }


        animated_meshes.push(AnimatedMesh {
            name: format!("{model_name}-mesh-{mesh_index}"),
            vertices,
            indices,
            texture_name,
            meshlets,
            aabb,
        });
    }

    let mut model_aabb = AABB::default();
    if let Some(first_mesh) = animated_meshes.first() {
        model_aabb = first_mesh.aabb;
        for mesh in animated_meshes.iter().skip(1) {
            model_aabb.min = model_aabb.min.min(mesh.aabb.min);
            model_aabb.max = model_aabb.max.max(mesh.aabb.max);
        }
    }

    let animated_model = AnimatedModel {
        name: model_name.to_string(),
        meshes: animated_meshes,
        skeleton,
        aabb: model_aabb,
    };

    let encoded_model = bincode::serialize(&animated_model)?;
    animated_model_table.insert(model_name, encoded_model.as_slice())?;

    println!("[DB] Successfully processed animated model: {model_name}");
    Ok(())
}
