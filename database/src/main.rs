use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use log::{info, warn};
use redb::{Database, Error, TableDefinition};
use russimp::material::TextureType;
use russimp::scene::{PostProcess, Scene};
use types::{Mesh, Model};
use walkdir::WalkDir;

const MODEL_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("models");
const TEXTURE_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("textures");

pub struct ModelDatabase {
    db: Database,
}

impl ModelDatabase {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let db = Database::create(path)?;
        Ok(Self { db })
    }

    pub fn populate_from_assets<P: AsRef<Path>>(&self, assets_dir: P) -> Result<(), anyhow::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut model_table = write_txn.open_table(MODEL_TABLE)?;
            let mut texture_table = write_txn.open_table(TEXTURE_TABLE)?;
            for entry in WalkDir::new(assets_dir)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                let extension = path.extension().and_then(|s| s.to_str());
                let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or_default();

                match extension {
                    Some("gltf") | Some("glb") => {
                        let model_name = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("unknown_model");

                        info!("Processing model: {}", model_name);

                        let scene = Scene::from_file(
                            path.to_str().unwrap(),
                            vec![
                                PostProcess::Triangulate,
                                PostProcess::JoinIdenticalVertices,
                            ],
                        )?;

                        let meshes = scene
                            .meshes
                            .into_iter()
                            .enumerate()
                            .map(|(i, mesh)| {
                                let material = &scene.materials[mesh.material_index as usize];
                                let mut texture_name = None;
                                for prop in &material.properties {
                                    if prop.key == "$tex.file" && prop.semantic == TextureType::Diffuse {
                                        if let russimp::material::PropertyTypeInfo::String(path) = &prop.data {
                                            texture_name = Path::new(path).file_name().and_then(|s| s.to_str()).map(String::from);
                                        }
                                    }
                                }

                                let unique_mesh_name = format!("{}-mesh-{}", model_name, i);
                                let vertices: Vec<types::Vertex> = mesh
                                    .vertices
                                    .into_iter()
                                    .map(|v| types::Vertex {
                                        position: glam::vec3(v.x, v.y, v.z),
                                        normal: glam::Vec3::ZERO, // Placeholder
                                        uv: glam::Vec2::ZERO,     // Placeholder
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

                        let model = Model {
                            name: model_name.to_string(),
                            meshes,
                        };

                        let encoded = bincode::serialize(&model)?;
                        model_table.insert(model_name, encoded.as_slice())?;
                    }
                    Some("png") => {
                        info!("Processing texture: {}", file_name);
                        let texture_bytes = fs::read(path)?;
                        texture_table.insert(file_name, texture_bytes.as_slice())?;
                    }
                    _ => {
                        // Skip other file types
                    }
                }
            }
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn get_model(&self, name: &str) -> Result<Option<Model>, anyhow::Error> {
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

fn main() -> Result<(), anyhow::Error> {
    env_logger::init();
    info!("Starting database populator");

    let mut workspace_root = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    workspace_root.pop(); // Go up to the workspace root from the crate root

    let db_path = workspace_root.join("assets/models.redb");
    let assets_path = workspace_root.join("assets/models");

    let db = ModelDatabase::new(&db_path)?;
    db.populate_from_assets(&assets_path)?;
    info!("Database populated successfully from {:?}", assets_path);

    // Example of retrieving a model
    if let Some(model) = db.get_model("cube")? {
        info!("Successfully retrieved model 'cube' with {} meshes.", model.meshes.len());
        for mesh in &model.meshes {
            info!("    - Mesh: {}", mesh.name);
            info!("      - Vertices: {}", mesh.vertices.len());
            info!("      - Indices: {}", mesh.indices.len());
            // Print first 3 vertices for inspection
            for (j, v) in mesh.vertices.iter().take(3).enumerate() {
                info!(
                    "      - Vertex {}: [{}, {}, {}]",
                    j, v.position.x, v.position.y, v.position.z
                );
            }
        }
    } else {
        warn!("Could not retrieve model 'cube'");
    }

    Ok(())
}
