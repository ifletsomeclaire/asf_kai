use crate::{
    config::Config,
    ecs::{input::Input, ui::EguiCtx},
};
use bevy_ecs::prelude::*;
use bevy_transform::components::Transform;
use glam::{Mat4, Quat, Vec3};
use eframe::egui::Key;

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
        // Standard right-handed perspective matrix.
        let projection = Mat4::perspective_rh(self.fovy, self.aspect, self.znear, self.zfar);

        // Correction matrix to flip the Y-axis for wgpu's coordinate system.
        let y_flip = Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0));

        // Apply the correction to the projection matrix.
        y_flip * projection
    }
}

#[derive(Resource)]
pub struct OrbitCamera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub pan: Vec3,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: std::f32::consts::FRAC_PI_6, // Reduced pitch for a more downward view
            distance: 15.0, // Increased distance to see more models
            pan: Vec3::ZERO,
        }
    }
}

pub fn update_camera_transform_system(
    mut query: Query<&mut Transform, With<Camera>>,
    orbit_camera: Res<OrbitCamera>,
) {
    if let Ok(mut transform) = query.single_mut() {
        let rotation =
            Quat::from_rotation_y(orbit_camera.yaw) * Quat::from_rotation_x(orbit_camera.pitch);
        let target = orbit_camera.target + orbit_camera.pan;
        transform.translation = target + rotation * (Vec3::Z * orbit_camera.distance);
        transform.look_at(target, Vec3::Y);
    }
}

pub fn camera_control_system(
    mut orbit_camera: ResMut<OrbitCamera>,
    input: Res<Input>,
    egui_ctx: Res<EguiCtx>,
    config: Res<Config>,
) {
    if egui_ctx.wants_pointer_input() || egui_ctx.wants_keyboard_input() {
        return;
    }

    let camera_config = &config.camera;
    let rotation =
        Quat::from_rotation_y(orbit_camera.yaw) * Quat::from_rotation_x(orbit_camera.pitch);

    // Keyboard pan
    let mut pan_delta = Vec3::ZERO;
    if input.0.key_down(Key::W) {
        pan_delta += rotation * Vec3::Z;
    }
    if input.0.key_down(Key::S) {
        pan_delta -= rotation * Vec3::Z;
    }
    if input.0.key_down(Key::A) {
        pan_delta += rotation * Vec3::X;
    }
    if input.0.key_down(Key::D) {
        pan_delta -= rotation * Vec3::X;
    }
    orbit_camera.pan += pan_delta.normalize_or_zero() * camera_config.keyboard_pan_sensitivity;

    if input.0.raw_scroll_delta.y != 0.0 {
        orbit_camera.distance -=
            input.0.raw_scroll_delta.y * orbit_camera.distance * camera_config.zoom_sensitivity;
        orbit_camera.distance = orbit_camera.distance.clamp(1.0, 10000.0);
    }

    if input.0.pointer.primary_down() {
        let delta = input.0.pointer.delta();
        orbit_camera.yaw -= delta.x * camera_config.orbit_sensitivity;
        orbit_camera.pitch -= delta.y * camera_config.orbit_sensitivity;

        orbit_camera.pitch = orbit_camera
            .pitch
            .clamp(-std::f32::consts::FRAC_PI_2 + 0.01, std::f32::consts::FRAC_PI_2 - 0.01);
    }

    if input.0.pointer.secondary_down() {
        let delta = input.0.pointer.delta();
        let right = rotation * -Vec3::X;
        let up = rotation * Vec3::Y;
        orbit_camera.pan += (right * delta.x - up * delta.y) * camera_config.pan_sensitivity;
    }
}