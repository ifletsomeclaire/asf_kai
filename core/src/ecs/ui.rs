use bevy_derive::Deref;
use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;
use eframe::egui;

use crate::{
    config::Config,
    ecs::{
        framerate::FrameRate,
        model::AssetReports,
    },
    renderer::events::ResizeEvent,
};
use bevy_transform::components::{GlobalTransform, Transform};
use glam::Vec3;

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

#[derive(SystemParam)]
pub struct UiSystemParams<'w> {
    egui_ctx: Res<'w, EguiCtx>,
    last_size: ResMut<'w, LastSize>,
    ui_state: ResMut<'w, UiState>,
    config: ResMut<'w, Config>,
    frame_rate: Res<'w, FrameRate>,
    events: EventWriter<'w, ResizeEvent>,
    asset_reports: Res<'w, AssetReports>,
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

        ui.interact(
            rect,
            ui.id().with("3d_view"),
            egui::Sense::drag().union(egui::Sense::hover()),
        );

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

        // Asset Reports Section
        egui::CollapsingHeader::new("🗄️ Asset Reports").show(ui, |ui| {
            ui.label(format!("GPU Memory Pools:"));
            ui.indent("gpu_memory", |ui| {
                ui.label(format!("Vertex Buffer: {:.1} MB free, largest: {:.1} MB", 
                    ui_params.asset_reports.vertex_total_free as f32 / 1_048_576.0,
                    ui_params.asset_reports.vertex_largest_free as f32 / 1_048_576.0));
                ui.label(format!("Index Buffer: {:.1} MB free, largest: {:.1} MB", 
                    ui_params.asset_reports.index_total_free as f32 / 1_048_576.0,
                    ui_params.asset_reports.index_largest_free as f32 / 1_048_576.0));
            });
            
            ui.separator();
            
            ui.label(format!("Database:"));
            ui.indent("database", |ui| {
                ui.label(format!("Models: {}", ui_params.asset_reports.model_count));
                ui.label(format!("Textures: {}", ui_params.asset_reports.texture_count));
                ui.label(format!("DB Size: {:.1} MB", 
                    ui_params.asset_reports.database_file_size as f32 / 1_048_576.0));
            });
            
            ui.separator();
            
            ui.small(format!("Last updated: {:.1}s ago", 
                ui_params.asset_reports.last_generated.elapsed().as_secs_f32()));
        });

        ui.separator();
    });
}
