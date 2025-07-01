use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Resource, Serialize, Deserialize, Default)]
pub struct Config {
    pub vsync: bool,
}

impl Config {
    const APP_NAME: &'static str = "AsfKai";
    const CONFIG_FILE: &'static str = "config.ron";

    fn config_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("com", "AsfKai", Self::APP_NAME)
            .map(|dirs| dirs.config_dir().join(Self::CONFIG_FILE))
    }

    pub fn save(&self) {
        if let Some(path) = Self::config_path() {
            if let Some(parent) = path.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    eprintln!("Failed to create config directory: {e}");
                    return;
                }
            }
            match ron::to_string(self) {
                Ok(ron_string) => {
                    if let Err(e) = fs::write(path, ron_string) {
                        eprintln!("Failed to write config file: {e}");
                    }
                }
                Err(e) => {
                    eprintln!("Failed to serialize config: {e}");
                }
            }
        }
    }

    pub fn load() -> Self {
        if let Some(path) = Self::config_path() {
            if let Ok(ron_string) = fs::read_to_string(path) {
                if let Ok(config) = ron::from_str(&ron_string) {
                    return config;
                }
            }
        }
        Self::default()
    }
}
