//! Commands for spawning entities with specific components.
use bevy_ecs::prelude::*;
use bevy_transform::components::GlobalTransform;

use crate::renderer::assets::{MeshHandle, TextureHandle};

/// A component that signals a request to spawn a `GpuInstance`.
/// This is the primary way that game logic should create new rendered objects.
#[derive(Component)]
pub struct SpawnGpuInstance {
    pub transform: GlobalTransform,
    pub mesh_handle: MeshHandle,
    pub texture_handle: TextureHandle,
} 