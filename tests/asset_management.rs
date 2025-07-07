/*
 * ===============================================================
 * Part 1: The Library Code (Bevy ECS Compatible)
 * ===============================================================
 * This section defines the core components of our asset management library.
 * The main change from the original is deriving `Component` for our handle wrappers
 * and using `bevy_ecs` instead of the full `bevy` crate.
 */

use bevy_derive::{Deref, DerefMut};
use bevy_ecs::prelude::*;
use std::sync::Arc;

// --- 1. Define the individual asset types ---
#[derive(Debug)]
pub struct Texture {
    pub id: u32,
    pub path: String,
}

impl Drop for Texture {
    fn drop(&mut self) {
        println!(
            "✅ Deallocating Texture asset '{}' (ID: {})",
            self.path, self.id
        );
    }
}

#[derive(Debug)]
pub struct Mesh {
    pub id: u32,
    pub path: String,
}

impl Drop for Mesh {
    fn drop(&mut self) {
        println!(
            "✅ Deallocating Mesh asset '{}' (ID: {})",
            self.path, self.id
        );
    }
}

// --- 2. Define the main Asset Enum ---
#[derive(Debug)]
pub enum Asset {
    Texture(Texture),
    Mesh(Mesh),
}

// --- 3. Define the public AssetHandle and Component Wrappers ---
pub type AssetHandle = Arc<Asset>;

#[derive(Component, Deref, DerefMut)]
pub struct Handle<T: 'static>(#[deref] pub AssetHandle, std::marker::PhantomData<T>);

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), std::marker::PhantomData)
    }
}

// --- 4. Asset Server ---
// A simple struct to act as a central point for loading assets.
// In a real app, this would be a Resource with caching logic.
pub struct AssetServer;

impl Default for AssetServer {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetServer {
    pub fn new() -> Self {
        Self
    }

    pub fn load_texture(&self, id: u32, path: &str) -> Handle<Texture> {
        let handle = Arc::new(Asset::Texture(Texture {
            id,
            path: path.to_string(),
        }));
        Handle(handle, std::marker::PhantomData)
    }

    pub fn load_mesh(&self, id: u32, path: &str) -> Handle<Mesh> {
        let handle = Arc::new(Asset::Mesh(Mesh {
            id,
            path: path.to_string(),
        }));
        Handle(handle, std::marker::PhantomData)
    }
}

/*
 * ===============================================================
 * Part 2: Bevy ECS Test
 * ===============================================================
 * This file shows how to use the asset library within a Bevy ECS World.
 */

/// A marker component for our player entity so we can find it later.
#[derive(Component)]
struct Player;

#[test]
fn asset_handle_rc_should_drop_when_entity_is_despawned() {
    println!("--- Bevy ECS Asset Management Test ---");
    println!("NOTE: A player entity will be spawned and then despawned.");
    println!("Watch for the 'Deallocating' messages when the test is finished.\n");

    // 1. Setup World
    let mut world = World::new();

    // 2. Setup asset server and load assets
    let asset_server = AssetServer::new();

    println!("[SETUP] Loading assets...");
    let player_texture = asset_server.load_texture(101, "assets/player.png");
    let player_mesh = asset_server.load_mesh(201, "assets/player.obj");

    // The asset server created the "original" handles. Ref count is 1.
    assert_eq!(1, Arc::strong_count(&player_texture.0));
    assert_eq!(1, Arc::strong_count(&player_mesh.0));
    println!("[SETUP] Initial Ref Counts are correct (1).");

    // 3. Spawn a player entity and attach the handles as components.
    // This clones the handles, increasing their ref counts.
    println!("[SETUP] Spawning player entity...");
    let player_entity = world
        .spawn((Player, player_texture.clone(), player_mesh.clone()))
        .id();

    // The world now holds a clone, and we hold the original. Ref count is 2.
    assert_eq!(2, Arc::strong_count(&player_texture.0));
    assert_eq!(2, Arc::strong_count(&player_mesh.0));
    println!("[SETUP] Ref Counts after spawn are correct (2).");

    // 4. Despawn the player entity.
    println!("\n[ACTION] Despawning player...");
    let despawned = world.despawn(player_entity);
    assert!(despawned);
    println!("[ACTION] Player despawned.");

    // After despawning, the components are dropped, and the world's handle clones are released.
    // The ref count should go back to 1.
    assert_eq!(1, Arc::strong_count(&player_texture.0));
    assert_eq!(1, Arc::strong_count(&player_mesh.0));
    println!("[VERIFY] Ref Counts after despawn are correct (1).");

    println!("\n[CLEANUP] Test scope is ending. Original handles will be dropped now.");
    // The test function now drops `player_texture` and `player_mesh`.
    // The ref count will go to 0, and the `Drop` impl on `Texture` and `Mesh` will run.
}
