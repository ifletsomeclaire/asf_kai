use std::env;
use std::path::{Path, PathBuf};

use log::{info, warn};
use redb::{Database, Error, TableDefinition};
use russimp::scene::{PostProcess, Scene};
use types::{Mesh, Model};
use walkdir::WalkDir;

const MODEL_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("models");

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
            let mut table = write_txn.open_table(MODEL_TABLE)?;
            for entry in WalkDir::new(assets_dir)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                let extension = path.extension().and_then(|s| s.to_str());

                if !matches!(extension, Some("gltf") | Some("glb")) {
                    continue;
                }

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

                let num_meshes = scene.meshes.len();
                let meshes = scene
                    .meshes
                    .into_iter()
                    .enumerate()
                    .map(|(i, mesh)| {
                        let unique_mesh_name = format!("{}-mesh-{}", model_name, i);
                        let vertices = mesh
                            .vertices
                            .into_iter()
                            .map(|v| glam::vec3(v.x, v.y, v.z))
                            .collect();
                        let indices = mesh.faces.into_iter().flat_map(|f| f.0).collect();
                        Mesh { name: unique_mesh_name, vertices, indices }
                    })
                    .collect();

                let model = Model {
                    name: model_name.to_string(),
                    meshes,
                };

                let encoded = bincode::serialize(&model)?;
                table.insert(model_name, encoded.as_slice())?;
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
        for (i, mesh) in model.meshes.iter().enumerate() {
            info!("  Mesh {}: '{}'", i, mesh.name);
            info!("    - Vertices: {}", mesh.vertices.len());
            info!("    - Indices: {}", mesh.indices.len());
            // Print first 3 vertices for inspection
            for (j, v) in mesh.vertices.iter().take(3).enumerate() {
                info!("      - Vertex {}: [{}, {}, {}]", j, v.x, v.y, v.z);
            }
        }
    } else {
        warn!("Could not retrieve model 'cube'");
    }

    Ok(())
}
