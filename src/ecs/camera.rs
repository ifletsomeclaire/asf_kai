use crate::ecs::input::Input;
use bevy_ecs::prelude::*;
use glam::{Mat4, Vec3, Vec4, Vec4Swizzles};

#[derive(Resource)]
pub struct Camera {
    pub position: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub aspect: f32,
    pub fovy: f32,
    pub znear: f32,
    pub zfar: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 1.0, 5.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            aspect: 16.0 / 9.0,
            fovy: 45.0f32.to_radians(),
            znear: 0.1,
            zfar: 10000.0,
        }
    }
}

impl Camera {
    pub fn build_view_projection_matrix(&self) -> Mat4 {
        let view = Mat4::look_at_rh(self.position, self.target, self.up);
        let proj = Mat4::perspective_rh(self.fovy, self.aspect, self.znear, self.zfar);
        proj * view
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
    mut camera: ResMut<Camera>,
    mut orbit_camera: ResMut<OrbitCamera>,
    input: Res<Input>,
) {
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

    let rotation = Mat4::from_rotation_y(orbit_camera.yaw) * Mat4::from_rotation_x(orbit_camera.pitch);
    camera.position = orbit_camera.target + (rotation * Vec4::new(0.0, 0.0, orbit_camera.distance, 1.0)).xyz();
} 