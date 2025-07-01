use bevy_ecs::event::Event;
use wgpu::Extent3d;

#[derive(Event, Clone)]
pub struct ResizeEvent(pub Extent3d);
