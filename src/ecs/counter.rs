use bevy_ecs::prelude::*;

#[derive(Resource, Default)]
pub struct Counter(pub u32);

pub fn increment_counter_system(mut counter: ResMut<Counter>) {
    counter.0 += 1;
}
