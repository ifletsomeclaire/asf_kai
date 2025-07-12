use bevy_ecs::prelude::*;
use bevy_transform::components::GlobalTransform;
use glam::{Mat4, Quat, Vec3};
use crate::ecs::time::Time;
use crate::renderer::assets::AssetServer;
use types::{Animation, PositionKey, RotationKey, ScaleKey};
use log;

#[derive(Component)]
pub struct AnimationPlayer {
    pub animation_name: String,
    pub next_animation: Option<String>,
    pub blend_factor: f32, // 0.0 = all current, 1.0 = all next
    pub blend_duration: f32, // Duration of blend transition in seconds
    pub current_time: f64,
    pub next_time: f64, // Time for the next animation during blending
    pub speed: f64,
    pub looping: bool,
    pub playing: bool,
}

impl Default for AnimationPlayer {
    fn default() -> Self {
        Self {
            animation_name: String::new(),
            next_animation: None,
            blend_factor: 0.0,
            blend_duration: 0.3, // 300ms default blend duration
            current_time: 0.0,
            next_time: 0.0,
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
        log::debug!("[Animation] Processing: {} transform: {:?}", player.animation_name, transform.translation());

        // Handle animation blending
        let next_anim_name = player.next_animation.clone();
        if let Some(next_anim_name) = &next_anim_name {
            // Update blend factor
            player.blend_factor += time.delta_seconds() as f32 / player.blend_duration;
            
            // Update next animation time
            player.next_time += time.delta_seconds() * player.speed;
            
            // Check if blend is complete
            if player.blend_factor >= 1.0 {
                // Transition complete
                player.animation_name = next_anim_name.clone();
                player.current_time = player.next_time;
                player.next_animation = None;
                player.blend_factor = 0.0;
                player.next_time = 0.0;
                log::info!("[Animation] -> Blend complete, switched to '{}'", player.animation_name);
            }
        }

        // Get current animation
        let current_animation = if let Some(anim) = asset_server.animated_meshlet_manager.animations.get(&player.animation_name) {
            anim
        } else {
            log::warn!("[Animation] -> WARNING: No animation found for '{}'", player.animation_name);
            continue;
        };

        // Update current animation time
        let old_time = player.current_time;
        player.current_time += time.delta_seconds() * player.speed;
        let duration_in_seconds = current_animation.duration_in_ticks as f64 / current_animation.ticks_per_second as f64;

        if player.looping {
            player.current_time %= duration_in_seconds;
        } else if player.current_time > duration_in_seconds {
            player.current_time = duration_in_seconds;
            player.playing = false;
        }
        
        let current_animation_time_in_ticks = player.current_time as f64 * current_animation.ticks_per_second;
        let model_matrix = transform.compute_matrix();

        log::debug!("[Animation] -> Time: {:.3}s -> {:.3}s (duration: {:.3}s)", 
            old_time, player.current_time, duration_in_seconds);
        log::debug!("[Animation] -> Animation time in ticks: {:.3}", current_animation_time_in_ticks);

        if let Some(skeleton) = &asset_server.animated_meshlet_manager.skeletons.get(&instance.model_name) {
            log::debug!("[Animation] -> Skeleton has {} bones", skeleton.bones.len());
            
            // Calculate poses for current animation
            let current_local_poses: Vec<Mat4> = skeleton.bones.iter().map(|bone| {
                calculate_bone_transform(current_animation, &bone.name, current_animation_time_in_ticks, bone.transform)
            }).collect();

            // Calculate global poses for current animation
            let mut current_global_poses = vec![Mat4::IDENTITY; skeleton.bones.len()];
            for (i, bone) in skeleton.bones.iter().enumerate() {
                let parent_pose = bone.parent_index
                    .map(|idx| current_global_poses[idx])
                    .unwrap_or(Mat4::IDENTITY);
                current_global_poses[i] = parent_pose * current_local_poses[i];
            }

            // If blending, calculate poses for next animation
            let mut final_global_poses = current_global_poses.clone();
            let next_anim_name = player.next_animation.clone();
            if let Some(next_anim_name) = &next_anim_name {
                if let Some(next_animation) = asset_server.animated_meshlet_manager.animations.get(next_anim_name) {
                    let next_duration_in_seconds = next_animation.duration_in_ticks as f64 / next_animation.ticks_per_second as f64;
                    let next_animation_time_in_ticks = player.next_time as f64 * next_animation.ticks_per_second;

                    // Calculate poses for next animation
                    let next_local_poses: Vec<Mat4> = skeleton.bones.iter().map(|bone| {
                        calculate_bone_transform(next_animation, &bone.name, next_animation_time_in_ticks, bone.transform)
                    }).collect();

                    // Calculate global poses for next animation
                    let mut next_global_poses = vec![Mat4::IDENTITY; skeleton.bones.len()];
                    for (i, bone) in skeleton.bones.iter().enumerate() {
                        let parent_pose = bone.parent_index
                            .map(|idx| next_global_poses[idx])
                            .unwrap_or(Mat4::IDENTITY);
                        next_global_poses[i] = parent_pose * next_local_poses[i];
                    }

                    // Blend between current and next poses
                    for i in 0..skeleton.bones.len() {
                        final_global_poses[i] = blend_poses(current_global_poses[i], next_global_poses[i], player.blend_factor);
                    }

                    log::debug!("[Animation] -> Blending animations: factor={:.3}", player.blend_factor);
                }
            }

            // Calculate final skinning matrices with world transform applied
            bone_matrices.matrices.resize(256, Mat4::IDENTITY);
            for (i, bone) in skeleton.bones.iter().enumerate() {
                bone_matrices.matrices[i] = model_matrix * final_global_poses[i] * bone.inverse_bind_pose;
                
                // Log final bone matrix details for first few bones
                if i < 3 {
                    let final_pos = bone_matrices.matrices[i].transform_point3(glam::Vec3::ZERO);
                    log::debug!("[Animation] -> Final bone {} '{}': world_pos=[{:.3}, {:.3}, {:.3}]", 
                        i, bone.name, final_pos.x, final_pos.y, final_pos.z);
                }
            }
            
            log::debug!("[Animation] -> Updated {} bone matrices", skeleton.bones.len());
        } else {
            log::warn!("[Animation] -> WARNING: No skeleton found for model '{}'", instance.model_name);
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
            log::debug!("[Interpolation] Bone '{}': pos=[{:.3}, {:.3}, {:.3}], rot=[{:.3}, {:.3}, {:.3}, {:.3}], scale=[{:.3}, {:.3}, {:.3}]", 
                bone_name, position.x, position.y, position.z,
                rotation.x, rotation.y, rotation.z, rotation.w,
                scale.x, scale.y, scale.z);
        }
        
        transform
    } else {
        // If no animation channel affects this bone, return the bone's bind pose transform
        if bone_name.contains("Armature") || bone_name.contains("Bone") {
            log::debug!("[Interpolation] Bone '{}': using default transform (no animation channel)", bone_name);
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

    // Handle looping
    if time_in_ticks >= keys.last().unwrap().time {
        let last_key = keys.last().unwrap();
        let first_key = &keys[0];
        
        let time_past_end = time_in_ticks - last_key.time;
        let total_time = first_key.time + (keys.last().unwrap().time - last_key.time);
        let interpolation_factor = if total_time > 0.0 {
            time_past_end / total_time
        } else {
            0.0
        };
        
        return Some(last_key.position.lerp(first_key.position, interpolation_factor.min(1.0) as f32));
    }

    let Some(next_key_index) = keys.iter().position(|k| k.time >= time_in_ticks) else {
        return Some(keys.last().unwrap().position);
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
        log::debug!("[Interpolation] Position: t={:.3}, factor={:.3}, prev=[{:.3}, {:.3}, {:.3}], next=[{:.3}, {:.3}, {:.3}], result=[{:.3}, {:.3}, {:.3}]", 
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
    
    // Handle looping: if we're past the last keyframe, interpolate between last and first
    if time_in_ticks >= keys.last().unwrap().time {
        let last_key = keys.last().unwrap();
        let first_key = &keys[0];
        
        // Calculate how far past the last keyframe we are
        let time_past_end = time_in_ticks - last_key.time;
        
        // Assume the "virtual" next keyframe is at duration + first_key.time
        // This creates a smooth loop
        let total_time = first_key.time + (keys.last().unwrap().time - last_key.time);
        let interpolation_factor = if total_time > 0.0 {
            time_past_end / total_time
        } else {
            0.0
        };
        
        // IMPORTANT: Check for quaternion flip
        let mut target_rotation = first_key.rotation;
        let dot = last_key.rotation.dot(target_rotation);
        if dot < 0.0 {
            target_rotation = -target_rotation;
            log::warn!("[Quat Interp] WARNING: Negative dot product ({:.3}) between loop keyframes at t={:.2} and t={:.2}. Animation will flip!", dot, last_key.time, first_key.time);
        }
        
        return Some(last_key.rotation.slerp(target_rotation, interpolation_factor.min(1.0) as f32));
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

    // **FIX QUATERNION FLIPS**: Check the dot product. A negative value means the slerp will take the "long way around", causing a flip.
    // Instead of negating, use a smoother transition to avoid creating very small rotation values
    let mut next_quat_for_interp = next_key.rotation;
    let dot = prev_key.rotation.dot(next_quat_for_interp);
    if dot < 0.0 {
        // Instead of negating, use a smoother interpolation that avoids the flip
        log::warn!("[Quat Interp] DETECTED: Negative dot product ({:.3}) between keyframes at t={:.2} and t={:.2}. Using smooth transition!", dot, prev_key.time, next_key.time);
        
        // Use a step function to avoid the flip entirely
        if interpolation_factor < 0.5 {
            return Some(prev_key.rotation);
        } else {
            return Some(next_key.rotation);
        }
    }
    
    // **ADDITIONAL LOGGING**: Check for identity quaternions which can cause "crushing" effects
    let prev_is_identity = prev_key.rotation.length_squared() < 0.01;
    let next_is_identity = next_key.rotation.length_squared() < 0.01;
    if prev_is_identity || next_is_identity {
        log::warn!("[Quat Interp] WARNING: Identity quaternion detected! prev_is_identity={}, next_is_identity={} at t={:.2} and t={:.2}", 
            prev_is_identity, next_is_identity, prev_key.time, next_key.time);
    }
    
    // **DEFENSIVE MEASURES**: Handle edge cases that might cause issues
    let mut prev_quat = prev_key.rotation;
    let mut next_quat = next_quat_for_interp; // Use the corrected quaternion
    
    // Normalize quaternions to prevent issues
    if (prev_quat.length_squared() - 1.0).abs() > 1e-4 {
        prev_quat = prev_quat.normalize();
    }
    if (next_quat.length_squared() - 1.0).abs() > 1e-4 {
        next_quat = next_quat.normalize();
    }
    
    // Handle very small quaternions by replacing with identity
    if prev_quat.length_squared() < 0.01 {
        prev_quat = Quat::IDENTITY;
    }
    if next_quat.length_squared() < 0.01 {
        next_quat = Quat::IDENTITY;
    }

    let result = prev_quat.slerp(next_quat, interpolation_factor as f32);
    
    // Log interpolation details for debugging
    if keys.len() > 1 && (prev_key.rotation.xyz() - next_key.rotation.xyz()).length() > 0.1 {
        log::debug!("[Interpolation] Rotation: t={:.3}, factor={:.3}, prev=[{:.3}, {:.3}, {:.3}, {:.3}], next=[{:.3}, {:.3}, {:.3}, {:.3}], result=[{:.3}, {:.3}, {:.3}, {:.3}]", 
            time_in_ticks, interpolation_factor,
            prev_key.rotation.x, prev_key.rotation.y, prev_key.rotation.z, prev_key.rotation.w,
            next_key.rotation.x, next_key.rotation.y, next_key.rotation.z, next_key.rotation.w,
            result.x, result.y, result.z, result.w);
    }
    
    // **DEFENSIVE INTERPOLATION**: Handle large rotation differences gracefully
    let angle_diff = prev_key.rotation.angle_between(next_key.rotation);
    if angle_diff > 2.0 { // More than ~115 degrees
        log::warn!("[Quat Interp] WARNING: Large rotation difference ({:.1}°) between keyframes at t={:.2} and t={:.2}. Using defensive interpolation!", 
            angle_diff.to_degrees(), prev_key.time, next_key.time);
        
        // For large rotations, use a step function to avoid sudden flips
        // This is a common technique in animation systems
        if interpolation_factor < 0.5 {
            // Use the previous keyframe for the first half
            return Some(prev_quat);
        } else {
            // Use the next keyframe for the second half
            return Some(next_quat);
        }
    } else if angle_diff > 1.0 { // More than ~57 degrees
        // For medium rotations, use a smoother transition but still avoid slerp
        // Use a cubic interpolation on the rotation axis
        let t = interpolation_factor as f32;
        let smooth_t = t * t * (3.0 - 2.0 * t); // Smoothstep
        return Some(prev_quat.slerp(next_quat, smooth_t));
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

    // Handle looping
    if time_in_ticks >= keys.last().unwrap().time {
        let last_key = keys.last().unwrap();
        let first_key = &keys[0];
        
        let time_past_end = time_in_ticks - last_key.time;
        let total_time = first_key.time + (keys.last().unwrap().time - last_key.time);
        let interpolation_factor = if total_time > 0.0 {
            time_past_end / total_time
        } else {
            0.0
        };
        
        return Some(last_key.scale.lerp(first_key.scale, interpolation_factor.min(1.0) as f32));
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
    
    // **LOGGING**: Check for near-zero scale values, which cause the "crushed into a ball" effect.
    if result.length_squared() < 0.01 { // 0.1 * 0.1
        log::warn!("[Scale Interp] WARNING: Scale is near zero ({:.3}, {:.3}, {:.3}) at t={:.2}. Mesh will be crushed!", result.x, result.y, result.z, time_in_ticks);
    }
    
    // **DEBUGGING**: Log all scale interpolations to see what's happening
    if keys.len() > 1 {
        log::debug!("[Scale Interp] t={:.3}, factor={:.3}, prev=[{:.3}, {:.3}, {:.3}], next=[{:.3}, {:.3}, {:.3}], result=[{:.3}, {:.3}, {:.3}]", 
            time_in_ticks, interpolation_factor,
            prev_key.scale.x, prev_key.scale.y, prev_key.scale.z,
            next_key.scale.x, next_key.scale.y, next_key.scale.z,
            result.x, result.y, result.z);
    }
    
    // Log interpolation details for debugging
    if keys.len() > 1 && (prev_key.scale - next_key.scale).length() > 0.1 {
        log::debug!("[Interpolation] Scale: t={:.3}, factor={:.3}, prev=[{:.3}, {:.3}, {:.3}], next=[{:.3}, {:.3}, {:.3}], result=[{:.3}, {:.3}, {:.3}]", 
            time_in_ticks, interpolation_factor,
            prev_key.scale.x, prev_key.scale.y, prev_key.scale.z,
            next_key.scale.x, next_key.scale.y, next_key.scale.z,
            result.x, result.y, result.z);
    }

    Some(result)
}

/// Blend between two bone poses using linear interpolation for position/scale and spherical linear interpolation for rotation
fn blend_poses(pose1: Mat4, pose2: Mat4, factor: f32) -> Mat4 {
    // Decompose matrices into translation, rotation, and scale
    let (pos1, rot1, scale1) = decompose_matrix(pose1);
    let (pos2, rot2, scale2) = decompose_matrix(pose2);
    
    // Interpolate components
    let blended_pos = pos1.lerp(pos2, factor);
    let blended_rot = rot1.slerp(rot2, factor);
    let blended_scale = scale1.lerp(scale2, factor);
    
    // Reconstruct matrix
    Mat4::from_scale_rotation_translation(blended_scale, blended_rot, blended_pos)
}

/// Decompose a transformation matrix into translation, rotation, and scale
fn decompose_matrix(matrix: Mat4) -> (Vec3, Quat, Vec3) {
    // Extract translation
    let translation = matrix.transform_point3(Vec3::ZERO);
    
    // Extract scale and rotation (simplified - assumes uniform scaling)
    let scale_x = matrix.x_axis.length();
    let scale_y = matrix.y_axis.length();
    let scale_z = matrix.z_axis.length();
    let scale = Vec3::new(scale_x, scale_y, scale_z);
    
    // Extract rotation (normalize the basis vectors)
    let rotation_matrix = Mat4::from_cols(
        matrix.x_axis.normalize(),
        matrix.y_axis.normalize(),
        matrix.z_axis.normalize(),
        glam::Vec4::new(0.0, 0.0, 0.0, 1.0),
    );
    let rotation = Quat::from_mat4(&rotation_matrix);
    
    (translation, rotation, scale)
}