use bevy_ecs::prelude::*;
use glam::{Mat4, Quat, Vec3};
use crate::ecs::time::Time;
use crate::renderer::assets::AssetServer;
use types::{Animation, PositionKey, RotationKey, ScaleKey};

#[derive(Component)]
pub struct AnimationPlayer {
    pub animation_name: String,
    pub current_time: f32,
    pub speed: f32,
    pub looping: bool,
    pub playing: bool,
}

impl Default for AnimationPlayer {
    fn default() -> Self {
        Self {
            animation_name: String::new(),
            current_time: 0.0,
            speed: 1.0,
            looping: true,
            playing: true,
        }
    }
}

#[derive(Component)]
pub struct BoneMatrices {
    pub matrices: Vec<Mat4>,
}

#[derive(Component)]
pub struct AnimatedInstance {
    pub model_name: String,
}

pub fn animation_system(
    time: Res<Time>,
    asset_server: Res<AssetServer>,
    mut query: Query<(&mut AnimationPlayer, &mut BoneMatrices, &AnimatedInstance)>,
) {
    for (mut player, mut bone_matrices, instance) in query.iter_mut() {
        if !player.playing {
            continue;
        }

        if let Some(animation) = asset_server.animated_meshlet_manager.animations.get(&player.animation_name) {
            // Update time
            player.current_time += time.delta_seconds() * player.speed;
            let duration_in_seconds = animation.duration_in_ticks as f32 / animation.ticks_per_second as f32;

            if player.looping {
                player.current_time %= duration_in_seconds;
            } else if player.current_time > duration_in_seconds {
                player.current_time = duration_in_seconds;
                player.playing = false;
            }
            
            let animation_time_in_ticks = player.current_time as f64 * animation.ticks_per_second;

            if let Some(skeleton) = &asset_server.animated_meshlet_manager.skeletons.get(&instance.model_name) {
                // Calculate local pose for each bone
                let local_poses: Vec<Mat4> = skeleton.bones.iter().map(|bone| {
                    calculate_bone_transform(animation, &bone.name, animation_time_in_ticks, bone.transform)
                }).collect();

                // Calculate global pose for each bone
                let mut global_poses = vec![Mat4::IDENTITY; skeleton.bones.len()];
                for (i, bone) in skeleton.bones.iter().enumerate() {
                    let parent_pose = bone.parent_index
                        .map(|idx| global_poses[idx])
                        .unwrap_or(Mat4::IDENTITY);
                    global_poses[i] = parent_pose * local_poses[i];
                }

                // Calculate final skinning matrices
                bone_matrices.matrices.resize(256, Mat4::IDENTITY);
                for (i, bone) in skeleton.bones.iter().enumerate() {
                    bone_matrices.matrices[i] = global_poses[i] * bone.inverse_bind_pose;
                }
            }
        }
    }
}

fn calculate_bone_transform(animation: &Animation, bone_name: &str, time_in_ticks: f64, default_transform: Mat4) -> Mat4 {
    // Find the channel for the given bone
    if let Some(channel) = animation.channels.iter().find(|c| c.bone_name == bone_name) {
        // Interpolate position, rotation, and scale
        let position = find_interpolated_position(time_in_ticks, &channel.position_keys).unwrap_or(Vec3::ZERO);
        let rotation = find_interpolated_rotation(time_in_ticks, &channel.rotation_keys).unwrap_or(Quat::IDENTITY);
        let scale = find_interpolated_scale(time_in_ticks, &channel.scale_keys).unwrap_or(Vec3::ONE);

        Mat4::from_scale_rotation_translation(scale, rotation, position)
    } else {
        // If no animation channel affects this bone, return the bone's bind pose transform
        default_transform
    }
}

fn find_interpolated_position(time_in_ticks: f64, keys: &[PositionKey]) -> Option<Vec3> {
    if keys.is_empty() {
        return None;
    }
    if keys.len() == 1 {
        return Some(keys[0].position);
    }

    // Find the two keyframes to interpolate between
    let Some(next_key_index) = keys.iter().position(|k| k.time >= time_in_ticks) else {
        return Some(keys.last().unwrap().position); // Past the last keyframe
    };

    if next_key_index == 0 {
        return Some(keys[0].position);
    }

    let prev_key = &keys[next_key_index - 1];
    let next_key = &keys[next_key_index];

    let total_time = next_key.time - prev_key.time;
    let interpolation_factor = if total_time > 0.0 {
        (time_in_ticks - prev_key.time) / total_time
    } else {
        0.0
    };

    Some(prev_key.position.lerp(next_key.position, interpolation_factor as f32))
}

fn find_interpolated_rotation(time_in_ticks: f64, keys: &[RotationKey]) -> Option<Quat> {
    if keys.is_empty() {
        return None;
    }
    if keys.len() == 1 {
        return Some(keys[0].rotation);
    }
    
    let Some(next_key_index) = keys.iter().position(|k| k.time >= time_in_ticks) else {
        return Some(keys.last().unwrap().rotation);
    };

    if next_key_index == 0 {
        return Some(keys[0].rotation);
    }

    let prev_key = &keys[next_key_index - 1];
    let next_key = &keys[next_key_index];

    let total_time = next_key.time - prev_key.time;
    let interpolation_factor = if total_time > 0.0 {
        (time_in_ticks - prev_key.time) / total_time
    } else {
        0.0
    };

    Some(prev_key.rotation.slerp(next_key.rotation, interpolation_factor as f32))
}

fn find_interpolated_scale(time_in_ticks: f64, keys: &[ScaleKey]) -> Option<Vec3> {
    if keys.is_empty() {
        return None;
    }
    if keys.len() == 1 {
        return Some(keys[0].scale);
    }

    let Some(next_key_index) = keys.iter().position(|k| k.time >= time_in_ticks) else {
        return Some(keys.last().unwrap().scale);
    };
    
    if next_key_index == 0 {
        return Some(keys[0].scale);
    }

    let prev_key = &keys[next_key_index - 1];
    let next_key = &keys[next_key_index];
    
    let total_time = next_key.time - prev_key.time;
    let interpolation_factor = if total_time > 0.0 {
        (time_in_ticks - prev_key.time) / total_time
    } else {
        0.0
    };

    Some(prev_key.scale.lerp(next_key.scale, interpolation_factor as f32))
}