// core/src/ecs/time.rs
use bevy_ecs::prelude::Resource;
use std::time::{Duration, Instant};
use bevy_ecs::prelude::ResMut;

#[derive(Resource)]
pub struct Time {
    delta: Duration,
    last_update: Instant,
    elapsed: Duration,
}

impl Default for Time {
    fn default() -> Self {
        Self {
            delta: Duration::from_secs(0),
            last_update: Instant::now(),
            elapsed: Duration::from_secs(0),
        }
    }
}

impl Time {
    pub fn update(&mut self) {
        let now = Instant::now();
        self.delta = now - self.last_update;
        self.last_update = now;
        self.elapsed += self.delta;
    }

    pub fn delta_seconds(&self) -> f32 {
        self.delta.as_secs_f32()
    }

    pub fn elapsed_seconds(&self) -> f32 {
        self.elapsed.as_secs_f32()
    }
}

pub fn time_system(mut time: ResMut<Time>) {
    time.update();
} 