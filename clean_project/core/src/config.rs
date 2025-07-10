use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CameraConfig {
    pub orbit_sensitivity: f32,
    pub pan_sensitivity: f32,
    pub zoom_sensitivity: f32,
    pub keyboard_pan_sensitivity: f32,
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            orbit_sensitivity: 0.005,
            pan_sensitivity: 0.01,
            zoom_sensitivity: 0.1,
            keyboard_pan_sensitivity: 0.1,
        }
    }
}

#[derive(Resource, Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub vsync: bool,
    #[serde(default)]
    pub camera: CameraConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            vsync: true,
            camera: CameraConfig::default(),
        }
    }
}

impl Config {
    const APP_NAME: &'static str = "AsfKai";
    const CONFIG_FILE: &'static str = "config.ron";

    fn config_path() -> Option<PathBuf> {
        if let Ok(home) = std::env::var("HOME") {
            let config_dir = PathBuf::from(home).join(".config").join(Self::APP_NAME);
            Some(config_dir.join(Self::CONFIG_FILE))
        } else if let Ok(home) = std::env::var("USERPROFILE") {
            // Windows fallback
            let config_dir = PathBuf::from(home)
                .join("AppData")
                .join("Roaming")
                .join(Self::APP_NAME);
            Some(config_dir.join(Self::CONFIG_FILE))
        } else {
            // Final fallback - use current directory
            Some(PathBuf::from(Self::CONFIG_FILE))
        }
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
