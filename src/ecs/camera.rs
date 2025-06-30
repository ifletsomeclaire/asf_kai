use crate::ecs::input::Input;
use bevy_ecs::prelude::*;
use glam::{Mat4, Quat, Vec3, Vec4, Vec4Swizzles};
use bevy_transform::components::Transform;

#[derive(Component)]
pub struct Camera {
    pub aspect: f32,
    pub fovy: f32,
    pub znear: f32,
    pub zfar: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            aspect: 16.0 / 9.0,
            fovy: 45.0f32.to_radians(),
            znear: 0.1,
            zfar: 10000.0,
        }
    }
}

impl Camera {
    pub fn projection_matrix(&self) -> Mat4 {
        Mat4::perspective_rh(self.fovy, self.aspect, self.znear, self.zfar)
    }
}

#[derive(Resource)]
pub struct OrbitCamera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: std::f32::consts::FRAC_PI_4,
            distance: 10.0,
        }
    }
}

pub fn camera_control_system(
    mut query: Query<&mut Transform, With<Camera>>,
    mut orbit_camera: ResMut<OrbitCamera>,
    input: Res<Input>,
) {
    let mut transform = if let Ok(t) = query.get_single_mut() {
        t
    } else {
        // No camera entity, so nothing to do.
        return;
    };

    if input.0.raw_scroll_delta.y != 0.0 {
        orbit_camera.distance -= input.0.raw_scroll_delta.y * orbit_camera.distance * 0.1;
        orbit_camera.distance = orbit_camera.distance.clamp(1.0, 10000.0);
    }

    if input.0.pointer.primary_down() {
        let delta = input.0.pointer.delta();
        orbit_camera.yaw -= delta.x * 0.005;
        orbit_camera.pitch -= delta.y * 0.005;

        orbit_camera.pitch = orbit_camera
            .pitch
            .clamp(-std::f32::consts::FRAC_PI_2 + 0.01, std::f32::consts::FRAC_PI_2 - 0.01);
    }

    let rotation = Quat::from_rotation_y(orbit_camera.yaw) * Quat::from_rotation_x(orbit_camera.pitch);
    transform.translation = orbit_camera.target + rotation * (Vec3::Z * orbit_camera.distance);
    transform.look_at(orbit_camera.target, Vec3::Y);
} 