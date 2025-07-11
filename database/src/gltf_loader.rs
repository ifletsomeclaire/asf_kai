// database/src/gltf_loader.rs

use gltf;
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use image::ImageEncoder;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::io::Write;
use types::{
    AnimatedMesh, AnimatedModel, Animation, AnimationChannel, Bone, Mesh, Meshlet, Meshlets,
    Model, PositionKey, RotationKey, ScaleKey, Skeleton, SkinnedVertex, Vertex, AABB,
};

pub fn load_gltf_model<P: AsRef<Path>>(
    path: P,
    model_name: &str,
    skip_validation: bool,
) -> Result<(Option<Model>, Option<AnimatedModel>, Vec<Animation>, Vec<(String, Vec<u8>)>), Box<dyn std::error::Error>> {
    println!("[GLTF] Loading model: {} from {:?}", model_name, path.as_ref());

    let (document, buffers, images) = gltf::import(path)?;

    // Debug information
    println!("[GLTF] Document info:");
    println!("  - Scenes: {}", document.scenes().count());
    println!("  - Nodes: {}", document.nodes().count());
    println!("  - Meshes: {}", document.meshes().count());
    println!("  - Animations: {}", document.animations().count());
    println!("  - Skins: {}", document.skins().count());
    println!("  - Images: {}", images.len());

    // Check if this is an animated model
    let has_animations = !document.animations().collect::<Vec<_>>().is_empty();
    let has_skins = !document.skins().collect::<Vec<_>>().is_empty();

    let mut textures_to_add = Vec::new();

    // Process textures from the imported images
    for (idx, image) in images.iter().enumerate() {
        let texture_name = format!("{}_texture_{}.png", model_name, idx);
        println!("[GLTF] Processing texture {}: {}x{}, format: {:?}",
            idx, image.width, image.height, image.format);

        // Convert to PNG using the image crate
        let png_data = match image.format {
            gltf::image::Format::R8G8B8A8 => {
                // Already in RGBA format, encode as PNG
                let mut png_data = Vec::new();
                {
                    let encoder = image::codecs::png::PngEncoder::new(&mut png_data);
                    let img = image::RgbaImage::from_raw(
                        image.width,
                        image.height,
                        image.pixels.clone(),
                    ).ok_or("Failed to create image from pixels")?;
                    encoder.write_image(
                        &img,
                        image.width,
                        image.height,
                        image::ColorType::Rgba8.into(),
                    )?;
                }
                png_data
            },
            gltf::image::Format::R8G8B8 => {
                // Convert RGB to RGBA
                let mut rgba_pixels = Vec::with_capacity((image.width * image.height * 4) as usize);
                for chunk in image.pixels.chunks(3) {
                    rgba_pixels.extend_from_slice(chunk);
                    rgba_pixels.push(255); // Alpha = 1.0
                }

                let mut png_data = Vec::new();
                {
                    let encoder = image::codecs::png::PngEncoder::new(&mut png_data);
                    let img = image::RgbaImage::from_raw(
                        image.width,
                        image.height,
                        rgba_pixels,
                    ).ok_or("Failed to create image from RGBA pixels")?;
                    encoder.write_image(
                        &img,
                        image.width,
                        image.height,
                        image::ColorType::Rgba8.into(),
                    )?;
                }
                png_data
            },
            _ => {
                println!("[GLTF] Skipping unsupported image format: {:?}", image.format);
                continue;
            }
        };

        textures_to_add.push((texture_name, png_data));
    }

    if has_animations || has_skins {
        println!("[GLTF] Processing as animated model");
        // Process as animated model
        let (animated_model, animations) = process_animated_gltf(
            &document,
            &buffers,
            &images,
            model_name,
            &mut textures_to_add,
            skip_validation,
        )?;
        Ok((None, Some(animated_model), animations, textures_to_add))
    } else {
        println!("[GLTF] Processing as static model");
        // Process as static model
        let model = process_static_gltf(
            &document,
            &buffers,
            &images,
            model_name,
            &mut textures_to_add,
        )?;
        Ok((Some(model), None, Vec::new(), textures_to_add))
    }
}

fn process_static_gltf(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
    model_name: &str,
    textures_to_add: &mut Vec<(String, Vec<u8>)>,
) -> Result<Model, Box<dyn std::error::Error>> {
    let mut meshes = Vec::new();
    let mut mesh_counter = 0;

    // If there's a default scene, use it. Otherwise use the first scene, or process all nodes
    if let Some(scene) = document.default_scene().or_else(|| document.scenes().next()) {
        println!("[GLTF] Processing scene: {:?}", scene.name());
        for node in scene.nodes() {
            process_static_node(
                &node,
                &Mat4::IDENTITY,
                &mut meshes,
                &mut mesh_counter,
                model_name,
                buffers,
                images,
                textures_to_add,
            )?;
        }
    } else {
        // No scenes, process all root nodes
        println!("[GLTF] No scenes found, processing all root nodes");
        for node in document.nodes() {
            // Only process nodes that don't have parents (root nodes)
            let has_parent = document.nodes().any(|n| n.children().any(|c| c.index() == node.index()));
            if !has_parent {
                process_static_node(
                    &node,
                    &Mat4::IDENTITY,
                    &mut meshes,
                    &mut mesh_counter,
                    model_name,
                    buffers,
                    images,
                    textures_to_add,
                )?;
            }
        }
    }

    // If no meshes were found through node traversal, try processing meshes directly
    if meshes.is_empty() {
        println!("[GLTF] No meshes found through node traversal, processing meshes directly");
        for mesh in document.meshes() {
            for primitive in mesh.primitives() {
                let unique_mesh_name = format!("{}-mesh-{}", model_name, mesh_counter);
                mesh_counter += 1;

                if let Ok(processed_mesh) = process_primitive(
                    &primitive,
                    &unique_mesh_name,
                    &Mat4::IDENTITY,
                    buffers,
                    model_name,
                ) {
                    meshes.push(processed_mesh);
                }
            }
        }
    }

    println!("[GLTF] Processed {} meshes", meshes.len());

    // Calculate model AABB
    let mut model_aabb = AABB::default();
    if let Some(first_mesh) = meshes.first() {
        model_aabb = first_mesh.aabb;
        for mesh in meshes.iter().skip(1) {
            model_aabb.min = model_aabb.min.min(mesh.aabb.min);
            model_aabb.max = model_aabb.max.max(mesh.aabb.max);
        }
    }

    Ok(Model {
        name: model_name.to_string(),
        meshes,
        aabb: model_aabb,
    })
}

fn process_static_node(
    node: &gltf::Node,
    parent_transform: &Mat4,
    meshes: &mut Vec<Mesh>,
    mesh_counter: &mut usize,
    model_name: &str,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
    textures_to_add: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), Box<dyn std::error::Error>> {
    let node_transform = Mat4::from_cols_array_2d(&node.transform().matrix());
    let accumulated_transform = *parent_transform * node_transform;

    if let Some(mesh) = node.mesh() {
        println!("[GLTF] Processing mesh at node: {:?}", node.name());
        for primitive in mesh.primitives() {
            let unique_mesh_name = format!("{}-mesh-{}", model_name, *mesh_counter);
            *mesh_counter += 1;

            if let Ok(processed_mesh) = process_primitive(
                &primitive,
                &unique_mesh_name,
                &accumulated_transform,
                buffers,
                model_name,
            ) {
                meshes.push(processed_mesh);
            }
        }
    }

    // Process children
    for child in node.children() {
        process_static_node(
            &child,
            &accumulated_transform,
            meshes,
            mesh_counter,
            model_name,
            buffers,
            images,
            textures_to_add,
        )?;
    }

    Ok(())
}

fn process_primitive(
    primitive: &gltf::Primitive,
    mesh_name: &str,
    transform: &Mat4,
    buffers: &[gltf::buffer::Data],
    model_name: &str,
) -> Result<Mesh, Box<dyn std::error::Error>> {
    // Get texture name if available
    let texture_name = primitive.material().pbr_metallic_roughness()
        .base_color_texture()
        .map(|tex| format!("{}_texture_{}.png", model_name, tex.texture().source().index()));

    // Extract vertices
    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

    // Extract attribute arrays
    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or("Mesh has no positions")?
        .collect();
    let normals: Vec<[f32; 3]> = if let Some(normals_iter) = reader.read_normals() {
        normals_iter.collect()
    } else {
        println!("[GLTF]   - No normals found, generating default normals");
        vec![[0.0, 0.0, 1.0]; positions.len()]
    };
    // Check available texture coordinate sets
    let mut available_tex_coords = Vec::new();
    for i in 0..4 {
        if let Some(tex_coords) = reader.read_tex_coords(i) {
            available_tex_coords.push(i);
        }
    }
    println!("[GLTF]   - Available texture coordinate sets: {:?}", available_tex_coords);
    let uvs: Vec<[f32; 2]> = if let Some(tex_coords) = reader.read_tex_coords(0) {
        let coords: Vec<[f32; 2]> = tex_coords.into_f32().collect();
        println!("[GLTF]   - Using texture coordinate set 0 with {} coordinates", coords.len());
        coords
    } else {
        println!("[GLTF]   - No texture coordinates found, using defaults");
        vec![[0.0, 0.0]; positions.len()]
    };

    // Extract indices
    let indices: Vec<u32> = if let Some(indices_reader) = reader.read_indices() {
        indices_reader.into_u32().collect()
    } else {
        println!("[GLTF]   - No indices found, generating triangle list");
        (0..positions.len() as u32).collect()
    };

    println!("[GLTF]   - Primitive has {} indices ({} triangles)",
        indices.len(), indices.len() / 3);

    // Build deduplicated vertex buffer using indices
    use std::collections::HashMap;
    let mut vertex_map: HashMap<(u32, u32, u32), u32> = HashMap::new();
    let mut dedup_vertices: Vec<Vertex> = Vec::new();
    let mut remapped_indices: Vec<u32> = Vec::with_capacity(indices.len());

    for &idx in &indices {
        let pos_idx = idx as usize;
        let norm_idx = idx as usize;
        let uv_idx = idx as usize;
        let key = (pos_idx as u32, norm_idx as u32, uv_idx as u32);
        let entry = vertex_map.entry(key).or_insert_with(|| {
            let position = Vec3::from(positions[pos_idx]);
            let normal = Vec3::from(normals[norm_idx]).normalize_or_zero();
            let uv = uvs.get(uv_idx).copied().unwrap_or([0.0, 0.0]);
            let transformed_pos = transform.transform_point3(position);
            let transformed_normal = transform.transform_vector3(normal).normalize_or_zero();
            let v = Vertex {
                position: transformed_pos.extend(1.0),
                normal: transformed_normal.extend(0.0),
                uv: Vec2::new(uv[0], uv[1]),
                _padding: [0.0; 2],
            };
            dedup_vertices.push(v);
            (dedup_vertices.len() - 1) as u32
        });
        remapped_indices.push(*entry);
    }

    // Build meshlets
    let meshlets = build_meshlets_for_vertices(&dedup_vertices, &remapped_indices)?;

    // Calculate AABB
    let mut aabb = AABB::default();
    if let Some(first_vtx) = dedup_vertices.first() {
        aabb.min = first_vtx.position;
        aabb.max = first_vtx.position;
        for v in dedup_vertices.iter().skip(1) {
            aabb.min = aabb.min.min(v.position);
            aabb.max = aabb.max.max(v.position);
        }
    }

    println!("[GLTF]   - AABB: min=[{:.2}, {:.2}, {:.2}], max=[{:.2}, {:.2}, {:.2}]",
        aabb.min.x, aabb.min.y, aabb.min.z,
        aabb.max.x, aabb.max.y, aabb.max.z);

    Ok(Mesh {
        name: mesh_name.to_string(),
        vertices: dedup_vertices,
        indices: remapped_indices,
        texture_name,
        meshlets,
        aabb,
    })
}

fn process_animated_gltf(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
    model_name: &str,
    textures_to_add: &mut Vec<(String, Vec<u8>)>,
    skip_validation: bool,
) -> Result<(AnimatedModel, Vec<Animation>), Box<dyn std::error::Error>> {
    // Build skeleton from the first skin (most GLTF files have one skin)
    let skin = document.skins().next()
        .ok_or("Animated model has no skin")?;

    println!("[GLTF] Building skeleton from skin: {:?}", skin.name());

    let mut bones = Vec::new();
    let mut bone_map = HashMap::new();
    let mut node_to_bone = HashMap::new();

    // Get inverse bind matrices
    let reader = skin.reader(|buffer| Some(&buffers[buffer.index()]));
    let inverse_bind_matrices: Vec<Mat4> = reader
        .read_inverse_bind_matrices()
        .map(|matrices| {
            matrices.map(|m| Mat4::from_cols_array_2d(&m)).collect()
        })
        .unwrap_or_else(|| vec![Mat4::IDENTITY; skin.joints().count()]);

    // First pass: collect all joints and create bone entries
    let joints: Vec<_> = skin.joints().collect();
    println!("[GLTF] Skeleton has {} joints", joints.len());

    for (idx, joint) in joints.iter().enumerate() {
        let bone_name = joint.name().unwrap_or(&format!("Bone_{}", idx)).to_string();
        bone_map.insert(joint.index(), idx);
        node_to_bone.insert(joint.index(), idx);

        let transform = Mat4::from_cols_array_2d(&joint.transform().matrix());

        bones.push(Bone {
            name: bone_name.clone(),
            parent_index: None, // Will be filled in second pass
            transform,
            inverse_bind_pose: inverse_bind_matrices.get(idx).copied()
                .unwrap_or(Mat4::IDENTITY),
        });

        println!("[GLTF]   - Joint {}: {}", idx, bone_name);
    }

    // Second pass: establish parent relationships
    for (idx, joint) in joints.iter().enumerate() {
        // Find parent by checking all nodes to see which one has this joint as a child
        for (potential_parent_idx, potential_parent) in joints.iter().enumerate() {
            if potential_parent.children().any(|child| child.index() == joint.index()) {
                bones[idx].parent_index = Some(potential_parent_idx);
                println!("[GLTF]     - Bone {} parent is {}", idx, potential_parent_idx);
                break;
            }
        }
    }

    let skeleton = Skeleton { bones };

    // Process meshes
    let mut animated_meshes = Vec::new();
    let mut mesh_counter = 0;

    // Process meshes from scenes
    if let Some(scene) = document.default_scene().or_else(|| document.scenes().next()) {
        println!("[GLTF] Processing animated meshes from scene: {:?}", scene.name());
        for node in scene.nodes() {
            process_animated_node(
                &node,
                &mut animated_meshes,
                &mut mesh_counter,
                model_name,
                buffers,
                &skeleton,
                &node_to_bone,
                textures_to_add,
            )?;
        }
    }

    // If no meshes found, try processing all skinned meshes directly
    if animated_meshes.is_empty() {
        println!("[GLTF] No meshes found through scene traversal, checking all nodes with meshes");
        for node in document.nodes() {
            if node.mesh().is_some() {
                process_animated_node(
                    &node,
                    &mut animated_meshes,
                    &mut mesh_counter,
                    model_name,
                    buffers,
                    &skeleton,
                    &node_to_bone,
                    textures_to_add,
                )?;
            }
        }
    }

    println!("[GLTF] Processed {} animated meshes", animated_meshes.len());

    // Process animations
    let mut animations = Vec::new();
    for (anim_idx, anim) in document.animations().enumerate() {
        println!("[GLTF] Processing animation {}: {:?}", anim_idx, anim.name());
        let animation = process_gltf_animation(&anim, buffers, &skeleton, &node_to_bone)?;
        animations.push(animation);
    }

    // Validate animations (if not skipped)
    if !skip_validation {
        for animation in &mut animations {
            validate_and_fix_animation_data(animation, &skeleton, model_name)?;
        }
    }

    // Calculate model AABB
    let mut model_aabb = AABB::default();
    if let Some(first_mesh) = animated_meshes.first() {
        model_aabb = first_mesh.aabb;
        for mesh in animated_meshes.iter().skip(1) {
            model_aabb.min = model_aabb.min.min(mesh.aabb.min);
            model_aabb.max = model_aabb.max.max(mesh.aabb.max);
        }
    }

    Ok((
        AnimatedModel {
            name: model_name.to_string(),
            meshes: animated_meshes,
            skeleton,
            aabb: model_aabb,
        },
        animations,
    ))
}

fn process_animated_node(
    node: &gltf::Node,
    meshes: &mut Vec<AnimatedMesh>,
    mesh_counter: &mut usize,
    model_name: &str,
    buffers: &[gltf::buffer::Data],
    skeleton: &Skeleton,
    node_to_bone: &HashMap<usize, usize>,
    textures_to_add: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(mesh) = node.mesh() {
        println!("[GLTF] Processing animated mesh at node: {:?}", node.name());
        for primitive in mesh.primitives() {
            let unique_mesh_name = format!("{}-mesh-{}", model_name, *mesh_counter);
            *mesh_counter += 1;

            // Get texture name
            let texture_name = primitive.material().pbr_metallic_roughness()
                .base_color_texture()
                .map(|tex| format!("{}_texture_{}.png", model_name, tex.texture().source().index()));

            // Extract vertex data
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

            let positions: Vec<[f32; 3]> = reader
                .read_positions()
                .ok_or("Mesh has no positions")?
                .collect();

            println!("[GLTF]   - Primitive has {} vertices", positions.len());

            let normals: Vec<[f32; 3]> = if let Some(normals_iter) = reader.read_normals() {
                normals_iter.collect()
            } else {
                println!("[GLTF]   - No normals found, generating defaults");
                vec![[0.0, 0.0, 1.0]; positions.len()]
            };

            // Check available texture coordinate sets for animated mesh
            let mut available_tex_coords = Vec::new();
            for i in 0..4 { // Check first 4 texture coordinate sets
                if let Some(tex_coords) = reader.read_tex_coords(i) {
                    available_tex_coords.push(i);
                }
            }
            println!("[GLTF]   - Available texture coordinate sets: {:?}", available_tex_coords);

            let uvs: Vec<[f32; 2]> = if let Some(tex_coords) = reader.read_tex_coords(0) {
                let coords: Vec<[f32; 2]> = tex_coords.into_f32().collect();
                println!("[GLTF]   - Using texture coordinate set 0 with {} coordinates", coords.len());
                coords
            } else {
                println!("[GLTF]   - No UVs found, using defaults");
                vec![[0.0, 0.0]; positions.len()]
            };

            // Read bone weights and indices
            let joints: Vec<[u16; 4]> = if let Some(joints_iter) = reader.read_joints(0) {
                joints_iter.into_u16().collect()
            } else {
                println!("[GLTF]   - No joint data found, using defaults");
                vec![[0, 0, 0, 0]; positions.len()]
            };

            let weights: Vec<[f32; 4]> = if let Some(weights_iter) = reader.read_weights(0) {
                weights_iter.into_f32().collect()
            } else {
                println!("[GLTF]   - No weight data found, using defaults");
                vec![[1.0, 0.0, 0.0, 0.0]; positions.len()]
            };

            // Extract indices
            let indices: Vec<u32> = if let Some(indices_reader) = reader.read_indices() {
                indices_reader.into_u32().collect()
            } else {
                (0..positions.len() as u32).collect()
            };
            println!("[GLTF]   - Primitive has {} indices", indices.len());

            // Build deduplicated vertex buffer using indices
            use std::collections::HashMap;
            // Quantize weights to [u16; 4] for hashing
            fn quantize_weights(w: [f32; 4]) -> [u16; 4] {
                [
                    (w[0] * 65535.0).round() as u16,
                    (w[1] * 65535.0).round() as u16,
                    (w[2] * 65535.0).round() as u16,
                    (w[3] * 65535.0).round() as u16,
                ]
            }
            let mut vertex_map: HashMap<(u32, u32, u32, [u16; 4], [u16; 4]), u32> = HashMap::new();
            let mut dedup_vertices: Vec<SkinnedVertex> = Vec::new();
            let mut remapped_indices: Vec<u32> = Vec::with_capacity(indices.len());

            for &idx in &indices {
                let pos_idx = idx as usize;
                let norm_idx = idx as usize;
                let uv_idx = idx as usize;
                let joint = joints.get(idx as usize).copied().unwrap_or([0, 0, 0, 0]);
                let weight = weights.get(idx as usize).copied().unwrap_or([1.0, 0.0, 0.0, 0.0]);
                let quant_weight = quantize_weights(weight);
                let key = (pos_idx as u32, norm_idx as u32, uv_idx as u32, joint, quant_weight);
                let entry = vertex_map.entry(key).or_insert_with(|| {
                    // Normalize weights
                    let weight_sum: f32 = weight.iter().sum();
                    let normalized_weights = if weight_sum > 0.0 {
                        [
                            weight[0] / weight_sum,
                            weight[1] / weight_sum,
                            weight[2] / weight_sum,
                            weight[3] / weight_sum,
                        ]
                    } else {
                        [1.0, 0.0, 0.0, 0.0]
                    };
                    let pos = positions[pos_idx];
                    let norm = normals[norm_idx];
                    let uv = uvs.get(uv_idx).copied().unwrap_or([0.0, 0.0]);
                    dedup_vertices.push(SkinnedVertex {
                        position: Vec4::new(pos[0], pos[1], pos[2], 1.0),
                        normal: Vec4::new(norm[0], norm[1], norm[2], 0.0).normalize(),
                        uv: Vec2::new(uv[0], uv[1]),
                        bone_indices: [
                            joint[0] as u32,
                            joint[1] as u32,
                            joint[2] as u32,
                            joint[3] as u32,
                        ],
                        bone_weights: normalized_weights,
                        _padding: [0.0; 2],
                    });
                    (dedup_vertices.len() - 1) as u32
                });
                remapped_indices.push(*entry);
            }

            // Build meshlets
            let meshlets = build_meshlets_for_skinned_vertices(&dedup_vertices, &remapped_indices)?;

            // Calculate AABB
            let mut aabb = AABB::default();
            if let Some(first_vtx) = dedup_vertices.first() {
                aabb.min = first_vtx.position;
                aabb.max = first_vtx.position;
                for v in dedup_vertices.iter().skip(1) {
                    aabb.min = aabb.min.min(v.position);
                    aabb.max = aabb.max.max(v.position);
                }
            }

            meshes.push(AnimatedMesh {
                name: unique_mesh_name,
                vertices: dedup_vertices,
                indices: remapped_indices,
                texture_name,
                meshlets,
                aabb,
            });
        }
    }

    // Process children
    for child in node.children() {
        process_animated_node(
            &child,
            meshes,
            mesh_counter,
            model_name,
            buffers,
            skeleton,
            node_to_bone,
            textures_to_add,
        )?;
    }

    Ok(())
}

fn process_gltf_animation(
    animation: &gltf::Animation,
    buffers: &[gltf::buffer::Data],
    skeleton: &Skeleton,
    node_to_bone: &HashMap<usize, usize>,
) -> Result<Animation, Box<dyn std::error::Error>> {
    let name = animation.name().unwrap_or("Unnamed Animation").to_string();
    let mut channels = Vec::new();
    let mut max_time: f64 = 0.0;

    // Group channels by target node
    let mut channel_map: HashMap<String, AnimationChannel> = HashMap::new();

    println!("[GLTF]   - Animation has {} channels", animation.channels().count());

    for channel in animation.channels() {
        let target_node = channel.target().node();
        let node_index = target_node.index();

        // Check if this node is a bone in our skeleton
        let bone_name = if let Some(&bone_idx) = node_to_bone.get(&node_index) {
            skeleton.bones[bone_idx].name.clone()
        } else {
            // Try to find by name
            let node_name_owned = match target_node.name() {
                Some(name) => name.to_string(),
                None => format!("Node_{}", node_index),
            };
            if skeleton.bones.iter().any(|b| b.name == node_name_owned) {
                node_name_owned
            } else {
                println!("[GLTF]     - Skipping channel for non-bone node: {}", node_name_owned);
                continue;
            }
        };

        let reader = channel.reader(|buffer| Some(&buffers[buffer.index()]));

        let entry = channel_map.entry(bone_name.clone())
            .or_insert_with(|| AnimationChannel {
                bone_name,
                position_keys: Vec::new(),
                rotation_keys: Vec::new(),
                scale_keys: Vec::new(),
            });

        // Read keyframe times
        let times: Vec<f32> = reader.read_inputs()
            .ok_or("Animation channel has no input times")?
            .collect();

        match reader.read_outputs() {
            Some(gltf::animation::util::ReadOutputs::Translations(translations)) => {
                let positions: Vec<[f32; 3]> = translations.collect();
                println!("[GLTF]     - Translation channel: {} keyframes", positions.len());
                for (time, pos) in times.iter().zip(positions.iter()) {
                    entry.position_keys.push(PositionKey {
                        time: *time as f64,
                        position: Vec3::from(*pos),
                    });
                    max_time = max_time.max(*time as f64);
                }
            }
            Some(gltf::animation::util::ReadOutputs::Rotations(rotations)) => {
                let quats: Vec<[f32; 4]> = rotations.into_f32().collect();
                println!("[GLTF]     - Rotation channel: {} keyframes", quats.len());
                for (time, quat) in times.iter().zip(quats.iter()) {
                    // GLTF quaternions are [x, y, z, w]
                    let rotation = Quat::from_xyzw(quat[0], quat[1], quat[2], quat[3]).normalize();
                    entry.rotation_keys.push(RotationKey {
                        time: *time as f64,
                        rotation,
                    });
                    max_time = max_time.max(*time as f64);
                }
            }
            Some(gltf::animation::util::ReadOutputs::Scales(scales)) => {
                let scales: Vec<[f32; 3]> = scales.collect();
                println!("[GLTF]     - Scale channel: {} keyframes", scales.len());
                for (time, scale) in times.iter().zip(scales.iter()) {
                    entry.scale_keys.push(ScaleKey {
                        time: *time as f64,
                        scale: Vec3::from(*scale),
                    });
                    max_time = max_time.max(*time as f64);
                }
            }
            _ => {
                println!("[GLTF]     - Unknown channel type");
            }
        }
    }

    channels = channel_map.into_values().collect();
    println!("[GLTF]   - Processed {} bone channels, duration: {:.2}s", channels.len(), max_time);

    Ok(Animation {
        name,
        duration_in_ticks: max_time,
        ticks_per_second: 1.0, // GLTF uses seconds directly
        channels,
    })
}

// Keep all the existing helper functions unchanged...
fn build_meshlets_for_vertices(
    vertices: &[Vertex],
    indices: &[u32],
) -> Result<Option<Meshlets>, Box<dyn std::error::Error>> {
    use meshopt::{build_meshlets, VertexDataAdapter};

    const MAX_VERTICES: usize = 64;
    const MAX_TRIANGLES: usize = 128;

    let vertex_stride = std::mem::size_of::<Vertex>();
    let vertex_data_bytes = bytemuck::cast_slice(vertices);

    let adapter = VertexDataAdapter::new(vertex_data_bytes, vertex_stride, 0).unwrap();
    let meshlets_result = build_meshlets(indices, &adapter, MAX_VERTICES, MAX_TRIANGLES, 0.0);

    if meshlets_result.meshlets.is_empty() {
        println!("[GLTF]   - No meshlets generated for static mesh");
        return Ok(None);
    }

    println!("[GLTF]   - Generated {} meshlets for static mesh", meshlets_result.meshlets.len());

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

    Ok(Some(Meshlets {
        meshlets: converted_meshlets,
        vertices: meshlets_result.vertices,
        triangles: meshlets_result.triangles,
    }))
}

fn build_meshlets_for_skinned_vertices(
    vertices: &[SkinnedVertex],
    indices: &[u32],
) -> Result<Option<Meshlets>, Box<dyn std::error::Error>> {
    use meshopt::{build_meshlets, VertexDataAdapter};

    const MAX_VERTICES: usize = 64;
    const MAX_TRIANGLES: usize = 128;

    let vertex_stride = std::mem::size_of::<SkinnedVertex>();
    let vertex_data_bytes = bytemuck::cast_slice(vertices);

    let adapter = VertexDataAdapter::new(vertex_data_bytes, vertex_stride, 0).unwrap();
    let meshlets_result = build_meshlets(indices, &adapter, MAX_VERTICES, MAX_TRIANGLES, 0.0);

    println!("[GLTF]   - Generated {} meshlets for animated mesh", meshlets_result.meshlets.len());

    // Always create meshlets for animated meshes (like the old code)
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

    Ok(Some(Meshlets {
        meshlets: converted_meshlets,
        vertices: meshlets_result.vertices,
        triangles: meshlets_result.triangles,
    }))
}

// Copy your validate_and_fix_animation_data function here but with println! instead of print!
fn validate_and_fix_animation_data(
    animation: &mut Animation,
    skeleton: &Skeleton,
    model_name: &str,
) -> Result<(), String> {
    const VELOCITY_SPIKE_THRESHOLD: f32 = 10.0;
    const EPSILON: f64 = 1e-4;

    // --- 1. Validate Animation Timings and Duration ---
    if animation.duration_in_ticks <= 0.0 {
        return Err(format!(
            "Animation '{}' in model '{}' has a non-positive duration: {}",
            animation.name, model_name, animation.duration_in_ticks
        ));
    }
    if animation.ticks_per_second <= 0.0 {
        return Err(format!(
            "Animation '{}' in model '{}' has non-positive ticks_per_second: {}",
            animation.name, model_name, animation.ticks_per_second
        ));
    }

    for channel in &mut animation.channels {
        // --- 2. Validate Bone and Channel Integrity ---
        if !skeleton.bones.iter().any(|b| b.name == channel.bone_name) {
            return Err(format!(
                "Animation '{}' in model '{}' targets a non-existent bone: '{}'",
                animation.name, model_name, channel.bone_name
            ));
        }

        // --- 3. Sort Keyframes ---
        channel.position_keys.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
        channel.rotation_keys.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
        channel.scale_keys.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));

        // --- 4. Fix Consecutive Quaternion Flips and Detect Issues ---
        if !channel.rotation_keys.is_empty() {
            // First pass: fix quaternion flips and detect issues
            for i in 1..channel.rotation_keys.len() {
                let (left, right) = channel.rotation_keys.split_at_mut(i);
                let prev_quat = left[i-1].rotation;
                let curr_quat = &mut right[0].rotation;
                let key_time = right[0].time;

                // Fix quaternion hemisphere flips
                if prev_quat.dot(*curr_quat) < 0.0 {
                    *curr_quat = -*curr_quat;
                    println!("[DB] -> Fixed quaternion flip in animation '{}' for bone '{}' at keyframe {}",
                        animation.name, channel.bone_name, i);
                }

                // Detect and fix identity quaternions
                if curr_quat.length_squared() < 0.1 {
                    println!("[DB] -> FIXING: Identity quaternion detected in animation '{}' for bone '{}' at keyframe {} (t={:.2})",
                        animation.name, channel.bone_name, i, key_time);
                    *curr_quat = prev_quat;
                    println!("[DB] -> -> Replaced with previous keyframe rotation");
                }
            }

            // Additional quaternion validation
            for i in 0..channel.rotation_keys.len() {
                let quat = channel.rotation_keys[i].rotation;

                if (quat.length_squared() - 1.0).abs() > 1e-4 {
                    channel.rotation_keys[i].rotation = quat.normalize();
                }

                if quat.length_squared() < 0.01 {
                    println!("[DB] -> FIXING: Very small quaternion detected in animation '{}' for bone '{}' at keyframe {} (t={:.2})",
                        animation.name, channel.bone_name, i, channel.rotation_keys[i].time);
                    channel.rotation_keys[i].rotation = Quat::IDENTITY;
                    println!("[DB] -> -> Replaced with identity quaternion");
                }
            }

            // Second pass: detect and fix large rotation differences
            let mut i = 1;
            while i < channel.rotation_keys.len() {
                let prev_quat = channel.rotation_keys[i - 1].rotation;
                let curr_quat = channel.rotation_keys[i].rotation;
                let prev_time = channel.rotation_keys[i-1].time;
                let curr_time = channel.rotation_keys[i].time;

                let angle_diff = prev_quat.angle_between(curr_quat);
                if angle_diff > 2.0 {
                    println!("[DB] -> FIXING: Large rotation difference ({:.1}°) in animation '{}' for bone '{}' between keyframes {} and {} (t={:.2} and t={:.2})",
                        angle_diff.to_degrees(), animation.name, channel.bone_name,
                        i-1, i, prev_time, curr_time);

                    let mid_time = (prev_time + curr_time) / 2.0;
                    let mid_quat = prev_quat.slerp(curr_quat, 0.5);

                    channel.rotation_keys.insert(i, RotationKey {
                        time: mid_time,
                        rotation: mid_quat
                    });

                    println!("[DB] -> Inserted intermediate keyframe at t={:.2} to smooth large rotation", mid_time);
                    i += 1;
                }
                i += 1;
            }
        }

        // --- 5. Reconstruct Loop Point for Perfect Looping ---
        let has_keyframes = !channel.position_keys.is_empty() || !channel.rotation_keys.is_empty() || !channel.scale_keys.is_empty();

        if has_keyframes {
            // Ensure keyframe at t=0.0
            if channel.position_keys.first().map_or(true, |k| k.time > EPSILON) {
                let first_pos = channel.position_keys.first().map_or(Vec3::ZERO, |k| k.position);
                channel.position_keys.insert(0, PositionKey { time: 0.0, position: first_pos });
            }
            if channel.rotation_keys.first().map_or(true, |k| k.time > EPSILON) {
                let first_rot = channel.rotation_keys.first().map_or(Quat::IDENTITY, |k| k.rotation);
                channel.rotation_keys.insert(0, RotationKey { time: 0.0, rotation: first_rot });
            }
            if channel.scale_keys.first().map_or(true, |k| k.time > EPSILON) {
                let first_scale = channel.scale_keys.first().map_or(Vec3::ONE, |k| k.scale);
                channel.scale_keys.insert(0, ScaleKey { time: 0.0, scale: first_scale });
            }

            // Remove keyframes at animation duration
            channel.position_keys.retain(|k| (k.time - animation.duration_in_ticks).abs() > EPSILON);
            channel.rotation_keys.retain(|k| (k.time - animation.duration_in_ticks).abs() > EPSILON);
            channel.scale_keys.retain(|k| (k.time - animation.duration_in_ticks).abs() > EPSILON);

            // Create perfect end keyframe
            if let Some(first_pos) = channel.position_keys.first().map(|k| k.position) {
                channel.position_keys.push(PositionKey { time: animation.duration_in_ticks, position: first_pos });
            }
            if let Some(first_scale) = channel.scale_keys.first().map(|k| k.scale) {
                channel.scale_keys.push(ScaleKey { time: animation.duration_in_ticks, scale: first_scale });
            }
            if let Some(first_rot) = channel.rotation_keys.first().map(|k| k.rotation) {
                channel.rotation_keys.push(RotationKey { time: animation.duration_in_ticks, rotation: first_rot });
            }
        }

        // --- 6. Post-Loop Validation ---
        if !channel.rotation_keys.is_empty() && channel.rotation_keys.len() > 1 {
            let first_key = &channel.rotation_keys[0];
            let last_key = &channel.rotation_keys[channel.rotation_keys.len() - 1];

            let loop_angle_diff = first_key.rotation.angle_between(last_key.rotation);
            if loop_angle_diff > 1.5 {
                println!("[DB] -> FIXING: Large loop rotation difference ({:.1}°) in animation '{}' for bone '{}'",
                    loop_angle_diff.to_degrees(), animation.name, channel.bone_name);

                let smoother_last = first_key.rotation.slerp(last_key.rotation, 0.8);
                let last_index = channel.rotation_keys.len() - 1;
                channel.rotation_keys[last_index].rotation = smoother_last;

                println!("[DB] -> Adjusted last keyframe to smooth loop transition");
            }

            // Remove clearly broken keyframes
            let mut i = 0;
            while i < channel.rotation_keys.len() {
                let key = &channel.rotation_keys[i];

                if key.rotation.length_squared() < 0.01 {
                    println!("[DB] -> REMOVING: Identity quaternion keyframe in animation '{}' for bone '{}' at t={:.2}",
                        animation.name, channel.bone_name, key.time);
                    channel.rotation_keys.remove(i);
                    println!("[DB] -> Removed identity quaternion keyframe");
                } else {
                    i += 1;
                }
            }
        }

        // --- 7. Final Validation ---
        for key in channel.rotation_keys.iter_mut() {
            if (key.rotation.length_squared() - 1.0).abs() > 1e-4 {
                key.rotation = key.rotation.normalize();
            }
        }

        for (i, key) in channel.scale_keys.iter().enumerate() {
            if key.scale.x <= 0.0 || key.scale.y <= 0.0 || key.scale.z <= 0.0 {
                return Err(format!(
                    "Animation '{}', bone '{}': Scale keyframe {} has zero or negative components: {:?}",
                    animation.name, channel.bone_name, i, key.scale
                ));
            }

            if key.scale.length_squared() < 0.01 {
                println!("[DB] -> WARNING: Near-zero scale detected in animation '{}' for bone '{}' at keyframe {} (t={:.2}): {:?}",
                    animation.name, channel.bone_name, i, key.time, key.scale);
            }
        }

        // Check for velocity spikes
        if channel.rotation_keys.len() > 2 {
            let mut velocities = Vec::new();
            for i in 0..channel.rotation_keys.len() - 1 {
                let key1 = &channel.rotation_keys[i];
                let key2 = &channel.rotation_keys[i+1];
                let dt = key2.time - key1.time;
                if dt > EPSILON {
                    let angle = key1.rotation.angle_between(key2.rotation);
                    velocities.push(angle / dt as f32);
                }
            }
            if !velocities.is_empty() {
                let avg_velocity: f32 = velocities.iter().sum::<f32>() / velocities.len() as f32;
                for (i, &v) in velocities.iter().enumerate() {
                    if avg_velocity > 0.0 && v > avg_velocity * VELOCITY_SPIKE_THRESHOLD {
                        println!("[DB] ->   WARNING: Potential velocity spike in animation '{}' for bone '{}' ({}x avg)",
                            animation.name, channel.bone_name, v / avg_velocity);
                    }
                }
            }
        }
    }

    Ok(())
}