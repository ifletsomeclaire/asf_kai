use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use redb::{Database, TableDefinition};
use russimp::material::TextureType;
use russimp::scene::{PostProcess, Scene};
use types::{Mesh, Model};

const MODEL_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("models");
const TEXTURE_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("textures");

pub struct ModelDatabase {
    db: Database,
}

impl ModelDatabase {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let db = Database::create(path)?;
        Ok(Self { db })
    }

    pub fn populate_from_assets<P: AsRef<Path>>(&self, assets_dir: P) -> Result<(), Box<dyn std::error::Error>> {
        let write_txn = self.db.begin_write()?;
        {
            let mut model_table = write_txn.open_table(MODEL_TABLE)?;
            let mut texture_table = write_txn.open_table(TEXTURE_TABLE)?;
            
            fn visit_dir(dir: &Path, model_table: &mut redb::Table<&str, &[u8]>, texture_table: &mut redb::Table<&str, &[u8]>) -> Result<(), Box<dyn std::error::Error>> {
                if dir.is_dir() {
                    for entry in fs::read_dir(dir)? {
                        let entry = entry?;
                        let path = entry.path();
                        if path.is_dir() {
                            visit_dir(&path, model_table, texture_table)?;
                        } else {
                            process_file(&path, model_table, texture_table)?;
                        }
                    }
                }
                Ok(())
            }

            fn process_file(path: &Path, model_table: &mut redb::Table<&str, &[u8]>, texture_table: &mut redb::Table<&str, &[u8]>) -> Result<(), Box<dyn std::error::Error>> {
                let extension = path.extension().and_then(|s| s.to_str());
                let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or_default();

                match extension {
                    Some("gltf") | Some("glb") => {
                        let model_name = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("unknown_model");

                        println!("Processing model: {}", model_name);

                        let scene = Scene::from_file(
                            path.to_str().unwrap(),
                            vec![
                                PostProcess::Triangulate,
                                PostProcess::JoinIdenticalVertices,
                                PostProcess::GenerateSmoothNormals,
                            ],
                        )?;

                        let mut new_textures_to_add = Vec::new();

                        let meshes = scene
                            .meshes
                            .into_iter()
                            .enumerate()
                            .map(|(i, mesh)| {
                                let material = &scene.materials[mesh.material_index as usize];
                                let mut texture_name = None;
                                let unique_mesh_name = format!("{}-mesh-{}", model_name, i);

                                // Correctly look for textures in the material's texture map.
                                // Prioritize Diffuse, fallback to Emissive.
                                let texture_to_use = material.textures.get(&TextureType::Diffuse)
                                    .or_else(|| material.textures.get(&TextureType::Emissive));

                                if let Some(texture_ref) = texture_to_use {
                                    let texture = texture_ref.borrow();
                                    // The texture.data field is an enum. We need to handle its variants.
                                    match &texture.data {
                                        russimp::material::DataContent::Bytes(bytes) => {
                                            // This is the compressed data (e.g., a full PNG file in memory).
                                            let texture_type_str = if material.textures.contains_key(&TextureType::Diffuse) { "diffuse" } else { "emissive" };
                                            let new_name = format!("{}_{}.png", unique_mesh_name, texture_type_str);

                                            new_textures_to_add.push((new_name.clone(), bytes.clone()));
                                            texture_name = Some(new_name);
                                        },
                                        russimp::material::DataContent::Texel(_) => {
                                            eprintln!("Found uncompressed texel data for model '{}'. This is not currently supported for embedded textures.", model_name);
                                        }
                                    }
                                } else {
                                    // If no embedded texture, check properties for a file path. This handles older formats.
                                    for prop in &material.properties {
                                        if prop.key.contains("$tex.file") {
                                            if let russimp::material::PropertyTypeInfo::String(path) = &prop.data {
                                                texture_name = Path::new(path).file_name().and_then(|s| s.to_str()).map(String::from);
                                                if texture_name.is_some() {
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }

                                println!("[DB] Mesh: '{}' -> Found texture: {:?}", unique_mesh_name, texture_name);

                                let vertices: Vec<types::Vertex> = mesh
                                    .vertices
                                    .into_iter()
                                    .zip(mesh.normals.into_iter())
                                    .zip(
                                        mesh.texture_coords[0]
                                            .clone()
                                            .unwrap_or_default()
                                            .into_iter(),
                                    )
                                    .map(|((v, n), uv)| types::Vertex {
                                        position: glam::vec3(v.x, v.y, v.z),
                                        normal: glam::vec3(n.x, n.y, n.z),
                                        uv: glam::vec2(uv.x, 1.0 - uv.y),
                                    })
                                    .collect();
                                let indices: Vec<u32> =
                                    mesh.faces.into_iter().flat_map(|f| f.0).collect();
                                Mesh {
                                    name: unique_mesh_name,
                                    vertices,
                                    indices,
                                    texture_name,
                                }
                            })
                            .collect();

                        for (name, data) in new_textures_to_add {
                            texture_table.insert(name.as_str(), data.as_slice())?;
                        }

                        let model = Model {
                            name: model_name.to_string(),
                            meshes,
                        };

                        let encoded = bincode::serialize(&model)?;
                        model_table.insert(model_name, encoded.as_slice())?;
                    }
                    Some("png") => {
                        println!("Processing texture: {}", file_name);
                        let texture_bytes = fs::read(path)?;
                        texture_table.insert(file_name, texture_bytes.as_slice())?;
                    }
                    _ => {
                        // Skip other file types
                    }
                }
                Ok(())
            }

            visit_dir(assets_dir.as_ref(), &mut model_table, &mut texture_table)?;
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
    println!("Database populated successfully from {:?}", assets_path);

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
