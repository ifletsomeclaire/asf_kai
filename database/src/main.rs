// database/src/main.rs - Key changes for GLTF support

mod gltf_loader;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use redb::Database;
use russimp::scene::{Scene, PostProcess};
use types::{MODEL_TABLE, TEXTURE_TABLE, ANIMATED_MODEL_TABLE, ANIMATION_TABLE, Model};
use log;

pub struct ModelDatabase {
    db: Database,
    use_gltf: bool, // Add flag to choose loader
}

impl ModelDatabase {
    pub fn new<P: AsRef<Path>>(path: P, use_gltf: bool) -> Result<Self, Box<dyn std::error::Error>> {
        let db = Database::create(path)?;
        Ok(Self { db, use_gltf })
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
                use_gltf: bool,
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
                                use_gltf,
                            )?;
                        } else {
                            process_file(
                                &path,
                                model_table,
                                texture_table,
                                animated_model_table,
                                animation_table,
                                use_gltf,
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
                use_gltf: bool,
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

                        log::info!("[DB] Processing model: {model_name} (using {})", 
                            if use_gltf { "GLTF" } else { "russimp" });

                        if use_gltf {
                            // Use GLTF loader
                            let (static_model, animated_model, animations, textures) = 
                                crate::gltf_loader::load_gltf_model(path, model_name, false)?;
                            
                            // Save textures
                            for (texture_name, texture_data) in textures {
                                texture_table.insert(texture_name.as_str(), texture_data.as_slice())?;
                            }
                            
                            // Save model or animated model
                            if let Some(model) = static_model {
                                let encoded_model = bincode::serialize(&model)?;
                                model_table.insert(model_name, encoded_model.as_slice())?;
                            } else if let Some(animated_model) = animated_model {
                                let encoded_model = bincode::serialize(&animated_model)?;
                                animated_model_table.insert(model_name, encoded_model.as_slice())?;
                                
                                // Save animations
                                for animation in animations {
                                    let encoded_animation = bincode::serialize(&animation)?;
                                    animation_table.insert(animation.name.as_str(), encoded_animation.as_slice())?;
                                }
                            }
                        } else {
                            // Use existing russimp loader
                            let scene = Scene::from_file(
                                path.to_str().unwrap(),
                                vec![
                                    PostProcess::Triangulate,
                                    PostProcess::JoinIdenticalVertices,
                                    PostProcess::GenerateSmoothNormals,
                                ],
                            )?;

                            // ... (rest of existing russimp processing code)
                        }
                    }
                    Some("png") => {
                        log::info!("[DB] Processing texture: {file_name}");
                        let texture_bytes = fs::read(path)?;
                        texture_table.insert(file_name, texture_bytes.as_slice())?;
                    }
                    _ => {
                        // Skip other file types
                    }
                }
                Ok(())
            }

            visit_dir(assets_dir.as_ref(), &mut model_table, &mut texture_table, &mut animated_model_table, &mut animation_table, self.use_gltf)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn get_model(&self, model_name: &str) -> Result<Option<Model>, Box<dyn std::error::Error>> {
        let read_txn = self.db.begin_read()?;
        let model_table = read_txn.open_table(MODEL_TABLE)?;
        
        if let Some(model_data) = model_table.get(model_name)? {
            let model: Model = bincode::deserialize(model_data.value())?;
            Ok(Some(model))
        } else {
            Ok(None)
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logger
    env_logger::init();
    
    log::info!("Starting database populator");

    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    let use_gltf = args.iter().any(|arg| arg == "--gltf");
    
    if use_gltf {
        log::info!("Using GLTF loader");
    } else {
        log::info!("Using russimp loader (default)");
    }

    let mut workspace_root = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    workspace_root.pop(); // Go up to the workspace root from the crate root

    let db_path = workspace_root.join("assets/models.redb");
    let assets_path = workspace_root.join("assets/models");

    let db = ModelDatabase::new(&db_path, use_gltf)?;
    db.populate_from_assets(&assets_path)?;
    log::info!("Database populated successfully from {assets_path:?}");

    // Example of retrieving a model
    if let Some(model) = db.get_model("cube")? {
        log::info!(
            "Successfully retrieved model 'cube' with {} meshes.",
            model.meshes.len()
        );
        for mesh in &model.meshes {
            log::info!("    - Mesh: {}", mesh.name);
            log::info!("      - Vertices: {}", mesh.vertices.len());
            log::info!("      - Indices: {}", mesh.indices.len());
            if let Some(meshlets) = &mesh.meshlets {
                log::info!("      - Meshlets: {}", meshlets.meshlets.len());
            }
            // Print first 3 vertices for inspection
            for (j, v) in mesh.vertices.iter().take(3).enumerate() {
                log::info!(
                    "      - Vertex {}: [{}, {}, {}]",
                    j, v.position.x, v.position.y, v.position.z
                );
            }
        }
    } else {
        log::error!("Could not retrieve model 'cube'");
    }

    Ok(())
}