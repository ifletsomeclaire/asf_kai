use bevy_ecs::prelude::*;
use std::time::Instant;

#[derive(Resource, Default)]
pub struct RotationAngle(pub f32);

#[derive(Resource, Default)]
pub struct DragDelta(pub egui::Vec2);

#[derive(Resource, Default)]
pub struct Counter(pub u32);

#[derive(Resource)]
pub struct FrameRate {
    pub fps: f32,
    last_update: Instant,
    frame_count: u32,
}

impl Default for FrameRate {
    fn default() -> Self {
        Self {
            fps: 0.0,
            last_update: Instant::now(),
            frame_count: 0,
        }
    }
}

pub fn update_angle_system(mut angle: ResMut<RotationAngle>, drag_delta: Res<DragDelta>) {
    angle.0 += drag_delta.0.x * 0.01;
}

pub fn increment_counter_system(mut counter: ResMut<Counter>) {
    counter.0 += 1;
}

pub fn frame_rate_system(mut frame_rate: ResMut<FrameRate>) {
    frame_rate.frame_count += 1;
    let now = Instant::now();
    let elapsed = (now - frame_rate.last_update).as_secs_f32();

    if elapsed >= 1.0 {
        frame_rate.fps = frame_rate.frame_count as f32 / elapsed;
        frame_rate.last_update = now;
        frame_rate.frame_count = 0;
    }
}
