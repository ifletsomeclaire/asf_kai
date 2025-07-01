use bevy_derive::Deref;
use bevy_ecs::prelude::*;
use eframe::egui::{InputState, Key};

use crate::ecs::ui::EguiCtx;

#[derive(Resource, Deref)]
pub struct Input(pub InputState);

pub fn keyboard_input_system(input: Res<Input>, egui_ctx: Res<EguiCtx>) {
    if egui_ctx.wants_keyboard_input() || egui_ctx.wants_pointer_input() {
        return;
    }

    if input.key_pressed(Key::Space) {
        println!("Space was pressed!");
    }
}
