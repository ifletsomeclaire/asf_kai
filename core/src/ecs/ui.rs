use bevy_derive::Deref;
use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;
use eframe::egui;

use crate::{
    config::Config,
    ecs::{
        framerate::FrameRate,
        model::{AvailableModels, Model},
    },
    renderer::events::ResizeEvent,
};
use bevy_transform::components::{Transform, GlobalTransform};
use glam::{Quat, Vec3};

#[derive(Resource, Deref)]
pub struct EguiCtx(pub egui::Context);

#[derive(Resource, Default)]
pub struct LastSize(pub egui::Vec2);

#[derive(Resource)]
pub struct UiState {
    pub render_triangle: bool,
    pub render_model: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            render_triangle: false,
            render_model: true,
        }
    }
}

#[derive(Resource)]
pub struct SpawnerState {
    pub position: Vec3,
}

impl Default for SpawnerState {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
        }
    }
}

#[derive(SystemParam)]
pub struct UiSystemParams<'w, 's> {
    egui_ctx: Res<'w, EguiCtx>,
    last_size: ResMut<'w, LastSize>,
    ui_state: ResMut<'w, UiState>,
    config: ResMut<'w, Config>,
    frame_rate: Res<'w, FrameRate>,
    events: EventWriter<'w, ResizeEvent>,
    commands: Commands<'w, 's>,
    available_models: Res<'w, AvailableModels>,
    spawner_state: ResMut<'w, SpawnerState>,
}

pub fn ui_system(mut ui_params: UiSystemParams) {
    let ctx = &ui_params.egui_ctx;
    let new_size = ctx.screen_rect().size() * ctx.pixels_per_point();

    egui::CentralPanel::default().show(ctx, |ui| {
        let rect = ui.max_rect();
        if ui_params.last_size.0 != new_size {
            ui_params.last_size.0 = new_size;
            ui_params.events.write(ResizeEvent(wgpu::Extent3d {
                width: new_size.x.round() as u32,
                height: new_size.y.round() as u32,
                depth_or_array_layers: 1,
            }));
        }

        ui.interact(rect, ui.id().with("3d_view"), egui::Sense::drag());

        let callback = eframe::egui_wgpu::Callback::new_paint_callback(
            rect,
            crate::renderer::tonemapping_pass::FinalBlitCallback {},
        );
        ui.painter().add(callback);
    });

    egui::Window::new("Overlay").show(ctx, |ui| {
        ui.label("You can put any egui widget here.");
        if ui.button("A button").clicked() {
            // take some action
        }

        ui.label(format!("FPS: {:.1}", ui_params.frame_rate.fps));

        ui.separator();

        ui.checkbox(&mut ui_params.ui_state.render_triangle, "Render Triangle");
        ui.checkbox(&mut ui_params.ui_state.render_model, "Render Model");

        if ui.checkbox(&mut ui_params.config.vsync, "V-Sync").changed() {
            ui_params.config.save();
            ui.label("(Requires restart)");
        }
    });

    egui::Window::new("Spawner").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Position:");
            ui.add(egui::DragValue::new(&mut ui_params.spawner_state.position.x).speed(0.1));
            ui.add(egui::DragValue::new(&mut ui_params.spawner_state.position.y).speed(0.1));
            ui.add(egui::DragValue::new(&mut ui_params.spawner_state.position.z).speed(0.1));
        });

        ui.separator();

        for model_info in &ui_params.available_models.models {
            if ui.button(format!("Spawn {}", model_info.name)).clicked() {
                let translation = ui_params.spawner_state.position;
                ui_params.commands.spawn((
                    Model {
                        mesh_name: model_info.name.clone(),
                    },
                    Transform::from_translation(translation),
                    GlobalTransform::default(),
                ));
            }
        }
    });
}
