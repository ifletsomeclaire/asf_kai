use bevy_derive::Deref;
use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;
use eframe::egui;

use crate::{
    config::Config,
    ecs::{
        camera::{Camera, OrbitCamera},
        // commands::{DespawnInstance, SpawnInstance},
        time::Time,
        // model::SpawnedEntities,
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
    pub render_static_meshlets: bool,
    pub render_animated_meshlets: bool,
    // --- Spawner UI State ---
    pub spawner_selected_mesh: String,
    pub spawner_selected_texture: String,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            render_triangle: false,
            render_model: true,
            render_static_meshlets: true,
            render_animated_meshlets: true,
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
    time: Res<'w, Time>,
    events: EventWriter<'w, ResizeEvent>,
    orbit_camera: Res<'w, OrbitCamera>,
    camera_query: Query<'w, 's, &'static Camera>,
    // --- For Spawner ---
    commands: Commands<'w, 's>,
    asset_server: Res<'w, AssetServer>,
    // spawned_entities: ResMut<'w, SpawnedEntities>,
}

pub fn ui_system(mut p: UiSystemParams) {
    let ctx = &p.egui_ctx;

    egui::Window::new("Settings").show(ctx, |ui| {
        let fps = if p.time.delta_seconds() > 0.0 {
            1.0 / p.time.delta_seconds()
        } else {
            0.0
        };
        ui.label(format!("FPS: {:.1}", fps));
        ui.separator();
        ui.checkbox(&mut p.ui_state.render_triangle, "Render Triangle");
        ui.checkbox(&mut p.ui_state.render_model, "Render Model");
        ui.checkbox(
            &mut p.ui_state.render_static_meshlets,
            "Render Static Meshlets",
        );
        ui.checkbox(
            &mut p.ui_state.render_animated_meshlets,
            "Render Animated Meshlets",
        );
        if ui.checkbox(&mut p.config.vsync, "V-Sync").changed() {
            p.config.save();
            ui.label("(Requires restart)");
        }
    });

    egui::Window::new("Camera").show(ctx, |ui| {
        if let Ok(camera) = p.camera_query.single() {
            ui.label(format!("Distance: {:.2}", p.orbit_camera.distance));
            ui.label(format!("Yaw: {:.2}", p.orbit_camera.yaw.to_degrees()));
            ui.label(format!("Pitch: {:.2}", p.orbit_camera.pitch.to_degrees()));
            ui.label(format!("Target: {:.2?}", p.orbit_camera.target));
            ui.label(format!("Pan: {:.2?}", p.orbit_camera.pan));
            ui.separator();
            ui.label(format!("Near Plane (znear): {:.2}", camera.znear));
            ui.label(format!("Far Plane (zfar): {:.2}", camera.zfar));

            ui.separator();
            ui.label("Settings");

            let mut changed = false;
            changed |= ui
                .add(
                    egui::Slider::new(&mut p.config.camera.orbit_sensitivity, 0.001..=0.02)
                        .text("Orbit Sensitivity"),
                )
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(&mut p.config.camera.pan_sensitivity, 0.001..=0.05)
                        .text("Pan Sensitivity"),
                )
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(&mut p.config.camera.zoom_sensitivity, 0.01..=0.5)
                        .text("Zoom Sensitivity"),
                )
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(
                        &mut p.config.camera.keyboard_pan_sensitivity,
                        0.01..=1.0,
                    )
                    .text("Keyboard Pan Sensitivity"),
                )
                .changed();

            if changed {
                p.config.save();
            }
        } else {
            ui.label("Camera not found.");
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
            crate::renderer::pipelines::tonemapping::FinalBlitCallback {},
        );
        ui.painter().add(callback);
    });
}
