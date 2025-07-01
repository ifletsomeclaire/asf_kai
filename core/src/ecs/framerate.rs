use bevy_ecs::prelude::*;
use std::time::Instant;

#[derive(Resource)]
pub struct FrameRate {
    pub fps: f32,
    pub last_update: Instant,
    pub frame_count: u32,
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

impl FrameRate {
    pub fn update(&mut self) {
        self.frame_count += 1;
        let now = Instant::now();
        let elapsed = (now - self.last_update).as_secs_f32();

        if elapsed >= 1.0 {
            self.fps = self.frame_count as f32 / elapsed;
            self.last_update = now;
            self.frame_count = 0;
        }
    }
}

pub fn frame_rate_system(mut frame_rate: ResMut<FrameRate>) {
    frame_rate.update();
}
