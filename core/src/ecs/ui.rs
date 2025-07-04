use bevy_derive::Deref;
use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;
use eframe::egui;

use crate::{
    config::Config,
    ecs::{
        commands::{DespawnInstance, SpawnInstance},
        framerate::FrameRate,
        model::SpawnedEntities,
    },
    renderer::{assets::AssetServer, events::ResizeEvent},
};

#[derive(Resource, Deref)]
pub struct EguiCtx(pub egui::Context);

#[derive(Resource, Default)]
pub struct LastSize(pub egui::Vec2);

#[derive(Resource)]
pub struct UiState {
    pub render_triangle: bool,
    pub render_model: bool,
    // --- Spawner UI State ---
    pub spawner_selected_mesh: String,
    pub spawner_selected_texture: String,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            render_triangle: false,
            render_model: true,
            spawner_selected_mesh: String::new(),
            spawner_selected_texture: String::new(),
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
    // --- For Spawner ---
    commands: Commands<'w, 's>,
    asset_server: Res<'w, AssetServer>,
    spawned_entities: ResMut<'w, SpawnedEntities>,
}

pub fn ui_system(mut p: UiSystemParams) {
    let ctx = &p.egui_ctx;

    egui::Window::new("Settings").show(ctx, |ui| {
        ui.label(format!("FPS: {:.1}", p.frame_rate.fps));
        ui.separator();
        ui.checkbox(&mut p.ui_state.render_triangle, "Render Triangle");
        ui.checkbox(&mut p.ui_state.render_model, "Render Model");
        if ui.checkbox(&mut p.config.vsync, "V-Sync").changed() {
            p.config.save();
            ui.label("(Requires restart)");
        }
    });

    egui::Window::new("Spawner").show(ctx, |ui| {
        // --- Mesh Selection ---
        let mesh_names = p.asset_server.get_mesh_names();
        if p.ui_state.spawner_selected_mesh.is_empty() {
            p.ui_state.spawner_selected_mesh = mesh_names.first().cloned().unwrap_or_default();
        }
        egui::ComboBox::from_label("Mesh")
            .selected_text(p.ui_state.spawner_selected_mesh.clone())
            .show_ui(ui, |ui| {
                for name in mesh_names {
                    ui.selectable_value(&mut p.ui_state.spawner_selected_mesh, name.clone(), name);
                }
            });

        // --- Texture Selection ---
        let texture_names = p.asset_server.get_texture_names();
        if p.ui_state.spawner_selected_texture.is_empty() {
            p.ui_state.spawner_selected_texture = texture_names.first().cloned().unwrap_or_default();
        }
        egui::ComboBox::from_label("Texture")
            .selected_text(p.ui_state.spawner_selected_texture.clone())
            .show_ui(ui, |ui| {
                for name in texture_names {
                    ui.selectable_value(&mut p.ui_state.spawner_selected_texture, name.clone(), name);
                }
            });

        ui.separator();

        if ui.button("Spawn").clicked() {
            if !p.ui_state.spawner_selected_mesh.is_empty() && !p.ui_state.spawner_selected_texture.is_empty() {
                p.commands.queue(SpawnInstance {
                    transform: Default::default(),
                    mesh_name: p.ui_state.spawner_selected_mesh.clone(),
                    texture_name: p.ui_state.spawner_selected_texture.clone(),
                });
            }
        }

        if ui.button("Despawn Last").clicked() {
            if let Some(entity) = p.spawned_entities.0.pop() {
                p.commands.queue(DespawnInstance { entity });
            }
        }
    });

    egui::CentralPanel::default().show(ctx, |ui| {
        let rect = ui.max_rect();
        let new_size = ui.ctx().screen_rect().size() * ui.ctx().pixels_per_point();
        if p.last_size.0 != new_size {
            p.last_size.0 = new_size;
            p.events.write(ResizeEvent(wgpu::Extent3d {
                width: new_size.x.round() as u32,
                height: new_size.y.round() as u32,
                depth_or_array_layers: 1,
            }));
        }

        // ui.interact(rect, ui.id().with("3d_view"), egui::Sense::drag().union(egui::Sense::hover()));

        let callback = eframe::egui_wgpu::Callback::new_paint_callback(
            rect,
            crate::renderer::tonemapping_pass::FinalBlitCallback {},
        );
        ui.painter().add(callback);
    });
}
