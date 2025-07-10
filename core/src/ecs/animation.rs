use bevy_ecs::prelude::*;
use bevy_transform::components::GlobalTransform;
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
    mut query: Query<(&mut AnimationPlayer, &mut BoneMatrices, &AnimatedInstance, &GlobalTransform)>,
) {
    for (mut player, mut bone_matrices, instance, transform) in query.iter_mut() {
        if !player.playing {
            continue;
        }
        println!("[Animation] Processing: {} transform: {:?}", player.animation_name, transform.translation());

        if let Some(animation) = asset_server.animated_meshlet_manager.animations.get(&player.animation_name) {
            // Update time
            let old_time = player.current_time;
            player.current_time += time.delta_seconds() * player.speed;
            let duration_in_seconds = animation.duration_in_ticks as f32 / animation.ticks_per_second as f32;

            if player.looping {
                player.current_time %= duration_in_seconds;
            } else if player.current_time > duration_in_seconds {
                player.current_time = duration_in_seconds;
                player.playing = false;
            }
            
            let animation_time_in_ticks = player.current_time as f64 * animation.ticks_per_second;
            let model_matrix = transform.compute_matrix(); // Get the entity's world transform matrix

            println!("[Animation] -> Time: {:.3}s -> {:.3}s (duration: {:.3}s)", 
                old_time, player.current_time, duration_in_seconds);
            println!("[Animation] -> Animation time in ticks: {:.3}", animation_time_in_ticks);

            if let Some(skeleton) = &asset_server.animated_meshlet_manager.skeletons.get(&instance.model_name) {
                println!("[Animation] -> Skeleton has {} bones", skeleton.bones.len());
                
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
                    
                    // Log bone transformation details for first few bones
                    if i < 3 {
                        let local_pos = local_poses[i].transform_point3(glam::Vec3::ZERO);
                        let global_pos = global_poses[i].transform_point3(glam::Vec3::ZERO);
                        println!("[Animation] -> Bone {} '{}': local_pos=[{:.3}, {:.3}, {:.3}], global_pos=[{:.3}, {:.3}, {:.3}]", 
                            i, bone.name, local_pos.x, local_pos.y, local_pos.z, global_pos.x, global_pos.y, global_pos.z);
                    }
                }

                // Calculate final skinning matrices with world transform applied
                bone_matrices.matrices.resize(256, Mat4::IDENTITY);
                for (i, bone) in skeleton.bones.iter().enumerate() {
                    // The correct order for skinning matrices is:
                    // world_transform * global_pose * inverse_bind_pose
                    // This gives us the final bone matrix that transforms from bind pose to world space
                    bone_matrices.matrices[i] = model_matrix * global_poses[i] * bone.inverse_bind_pose;
                    
                    // Log final bone matrix details for first few bones
                    if i < 3 {
                        let final_pos = bone_matrices.matrices[i].transform_point3(glam::Vec3::ZERO);
                        println!("[Animation] -> Final bone {} '{}': world_pos=[{:.3}, {:.3}, {:.3}]", 
                            i, bone.name, final_pos.x, final_pos.y, final_pos.z);
                    }
                }
                
                println!("[Animation] -> Updated {} bone matrices", skeleton.bones.len());
            } else {
                println!("[Animation] -> WARNING: No skeleton found for model '{}'", instance.model_name);
            }
        } else {
            println!("[Animation] -> WARNING: No animation found for '{}'", player.animation_name);
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

        let transform = Mat4::from_scale_rotation_translation(scale, rotation, position);
        
        // Log interpolation details for debugging
        if bone_name.contains("Armature") || bone_name.contains("Bone") {
            println!("[Interpolation] Bone '{}': pos=[{:.3}, {:.3}, {:.3}], rot=[{:.3}, {:.3}, {:.3}, {:.3}], scale=[{:.3}, {:.3}, {:.3}]", 
                bone_name, position.x, position.y, position.z,
                rotation.x, rotation.y, rotation.z, rotation.w,
                scale.x, scale.y, scale.z);
        }
        
        transform
    } else {
        // If no animation channel affects this bone, return the bone's bind pose transform
        if bone_name.contains("Armature") || bone_name.contains("Bone") {
            println!("[Interpolation] Bone '{}': using default transform (no animation channel)", bone_name);
        }
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

    let result = prev_key.position.lerp(next_key.position, interpolation_factor as f32);
    
    // Log interpolation details for debugging
    if keys.len() > 1 && (prev_key.position - next_key.position).length() > 0.1 {
        println!("[Interpolation] Position: t={:.3}, factor={:.3}, prev=[{:.3}, {:.3}, {:.3}], next=[{:.3}, {:.3}, {:.3}], result=[{:.3}, {:.3}, {:.3}]", 
            time_in_ticks, interpolation_factor,
            prev_key.position.x, prev_key.position.y, prev_key.position.z,
            next_key.position.x, next_key.position.y, next_key.position.z,
            result.x, result.y, result.z);
    }

    Some(result)
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

    let result = prev_key.rotation.slerp(next_key.rotation, interpolation_factor as f32);
    
    // Log interpolation details for debugging
    if keys.len() > 1 && (prev_key.rotation.xyz() - next_key.rotation.xyz()).length() > 0.1 {
        println!("[Interpolation] Rotation: t={:.3}, factor={:.3}, prev=[{:.3}, {:.3}, {:.3}, {:.3}], next=[{:.3}, {:.3}, {:.3}, {:.3}], result=[{:.3}, {:.3}, {:.3}, {:.3}]", 
            time_in_ticks, interpolation_factor,
            prev_key.rotation.x, prev_key.rotation.y, prev_key.rotation.z, prev_key.rotation.w,
            next_key.rotation.x, next_key.rotation.y, next_key.rotation.z, next_key.rotation.w,
            result.x, result.y, result.z, result.w);
    }

    Some(result)
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

    let result = prev_key.scale.lerp(next_key.scale, interpolation_factor as f32);
    
    // Log interpolation details for debugging
    if keys.len() > 1 && (prev_key.scale - next_key.scale).length() > 0.1 {
        println!("[Interpolation] Scale: t={:.3}, factor={:.3}, prev=[{:.3}, {:.3}, {:.3}], next=[{:.3}, {:.3}, {:.3}], result=[{:.3}, {:.3}, {:.3}]", 
            time_in_ticks, interpolation_factor,
            prev_key.scale.x, prev_key.scale.y, prev_key.scale.z,
            next_key.scale.x, next_key.scale.y, next_key.scale.z,
            result.x, result.y, result.z);
    }

    Some(result)
}