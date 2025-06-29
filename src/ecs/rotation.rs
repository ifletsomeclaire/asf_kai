use bevy_ecs::prelude::*;

#[derive(Resource, Default)]
pub struct RotationAngle(pub f32);

#[derive(Resource, Default)]
pub struct DragDelta(pub egui::Vec2);

pub fn update_angle_system(mut angle: ResMut<RotationAngle>, drag_delta: Res<DragDelta>) {
    angle.0 += drag_delta.0.x * 0.01;
}
