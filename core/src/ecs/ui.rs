use bevy_derive::Deref;
use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;
use eframe::egui;
use log;

use crate::{
    config::Config,
    ecs::{
        animation::AnimationPlayer,
        camera::{Camera, OrbitCamera},
        // commands::{DespawnInstance, SpawnInstance},
        time::Time,
        // model::SpawnedEntities,
    },
    renderer::{assets::AssetServer, events::ResizeEvent},
};
use gpu_picking::GPUPicking;

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
    // --- For Animation Control ---
    animation_player_query: Query<'w, 's, &'static mut AnimationPlayer>,
    // --- For Spawner ---
    commands: Commands<'w, 's>,
    asset_server: Res<'w, AssetServer>,
    // spawned_entities: ResMut<'w, SpawnedEntities>,
    // --- For GPU Picking ---
    gpu_picking: Res<'w, GPUPicking>,
}

pub fn ui_system(mut p: UiSystemParams) {
    let ctx = &p.egui_ctx;

    egui::Window::new("Settings").show(ctx, |ui| {
        let fps = if p.time.delta_seconds() > 0.0 {
            1.0 / p.time.delta_seconds()
        } else {
            0.0
        };
        ui.label(format!("FPS: {fps:.1}"));
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

    // Add Animation Control Window
    egui::Window::new("Animation Control").show(ctx, |ui| {
        let mut animation_players: Vec<_> = p.animation_player_query.iter_mut().collect();
        let available_animations: Vec<String> = p.asset_server.animated_meshlet_manager.animations.keys().cloned().collect();
        
        if animation_players.is_empty() {
            ui.label("No animated entities found.");
        } else {
            ui.label(format!("Found {} animated entities:", animation_players.len()));
            
            for (i, player) in animation_players.iter_mut().enumerate() {
                ui.separator();
                ui.label(format!("Entity {}: {}", i + 1, player.animation_name));
                
                // Animation selection dropdown
                let current_anim_name = player.animation_name.clone();
                egui::ComboBox::from_label(format!("Entity {} Animation", i + 1))
                    .selected_text(&current_anim_name)
                    .show_ui(ui, |ui| {
                        for anim_name in &available_animations {
                            if ui.selectable_value(&mut player.animation_name, anim_name.clone(), anim_name).changed() {
                                player.current_time = 0.0; // Reset time on animation change
                                player.next_animation = None; // Cancel any ongoing blend
                                player.blend_factor = 0.0;
                                log::info!("[UI] Changed animation for entity {} to '{}'", i + 1, anim_name);
                            }
                        }
                    });
                
                // Blend duration slider
                let mut blend_duration = player.blend_duration;
                if ui.add(egui::Slider::new(&mut blend_duration, 0.1..=2.0).text("Blend Duration (s)")).changed() {
                    player.blend_duration = blend_duration;
                }
                
                // Animation speed slider
                let mut speed = player.speed;
                if ui.add(egui::Slider::new(&mut speed, 0.0..=5.0).text("Speed")).changed() {
                    player.speed = speed;
                }
                ui.label(format!("Current speed: {speed:.2}x"));
                
                // Play/Pause toggle
                if ui.checkbox(&mut player.playing, "Playing").changed() {
                    // The checkbox will automatically update the playing state
                }
                
                // Loop toggle
                if ui.checkbox(&mut player.looping, "Looping").changed() {
                    // The checkbox will automatically update the looping state
                }
                
                // Current time display
                ui.label(format!("Current time: {:.2}s", player.current_time));
                
                // Blend status display
                if player.next_animation.is_some() {
                    ui.label(format!("Blending to: {} (factor: {:.2})", 
                        player.next_animation.as_ref().unwrap(), player.blend_factor));
                }
                
                // Quick animation transition buttons
                ui.label("Quick Transitions:");
                ui.horizontal(|ui| {
                    for (j, anim_name) in available_animations.iter().take(3).enumerate() {
                        if ui.button(format!("{}", j + 1)).clicked()
                            && anim_name != &player.animation_name {
                                player.next_animation = Some(anim_name.clone());
                                player.blend_factor = 0.0;
                                player.next_time = 0.0;
                                log::info!("[UI] Triggered blend to '{}' for entity {}", anim_name, i + 1);
                            }
                    }
                });
            }
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

    // GPU Picking Window
    egui::Window::new("GPU Picking").show(ctx, |ui| {
        // Display current pick coordinates
        // if let Some(origin) = p.gpu_picking.selection_box {
        //     ui.label(format!("Pick Coordinates: ({}, {})", origin[0], origin[1]));
        // } else {
        //     ui.label("No pick coordinates set");
        // }
        
        ui.separator();
        
        // Display last picking results
        if let Some(results) = p.gpu_picking.get_last_result() {
            ui.label(format!("Found {} entities at pick location:", results.len()));
            
            for (i, &entity_id) in results.iter().enumerate() {
                ui.label(format!("  Entity {}: ID {}", i + 1, entity_id));
            }
        } else {
            ui.label("No entities found at pick location");
        }
        
        ui.separator();
        
        // Status information
        ui.label("Status:");
        if p.gpu_picking.is_picking_in_progress() {
            ui.label("• Picking operation in progress...");
        } else {
            ui.label("• Ready for picking");
        }
        
        ui.separator();
        
        // Instructions
        ui.label("Instructions:");
        ui.label("• Click in the 3D view to pick entities");
        ui.label("• Selected entities will be highlighted");
        ui.label("• Results are displayed above");
        ui.label("• Check console for detailed picking logs");
    });

    egui::Window::new("Spawner").show(ctx, |_ui| {
        // --- Mesh Selection ---
        // let mesh_names = p.asset_server.get_mesh_names();
        // if p.ui_state.spawner_selected_mesh.is_empty() {
        //     p.ui_state.spawner_selected_mesh = mesh_names.first().cloned().unwrap_or_default();
        // }
        // egui::ComboBox::from_label("Mesh")
        //     .selected_text(p.ui_state.spawner_selected_mesh.clone())
        //     .show_ui(ui, |ui| {
        //         for name in mesh_names {
        //             ui.selectable_value(&mut p.ui_state.spawner_selected_mesh, name.clone(), name);
        //         }
        //     });

        // --- Texture Selection ---
        // let texture_names = p.asset_server.get_texture_names();
        // if p.ui_state.spawner_selected_texture.is_empty() {
        //     p.ui_state.spawner_selected_texture =
        //         texture_names.first().cloned().unwrap_or_default();
        // }
        // egui::ComboBox::from_label("Texture")
        //     .selected_text(p.ui_state.spawner_selected_texture.clone())
        //     .show_ui(ui, |ui| {
        //         for name in texture_names {
        //             ui.selectable_value(
        //                 &mut p.ui_state.spawner_selected_texture,
        //                 name.clone(),
        //                 name,
        //             );
        //         }
        //     });

        // ui.separator();

        // if ui.button("Spawn").clicked()
        //     && !p.ui_state.spawner_selected_mesh.is_empty()
        //     && !p.ui_state.spawner_selected_texture.is_empty()
        // {
        //     p.commands.queue(SpawnInstance {
        //         transform: GlobalTransform::default(),
        //         mesh_name: p.ui_state.spawner_selected_mesh.clone(),
        //         texture_name: p.ui_state.spawner_selected_texture.clone(),
        //     });
        // }

        // if ui.button("Despawn Last").clicked() {
        //     if let Some(entity) = p.spawned_entities.0.pop() {
        //         p.commands.queue(DespawnInstance { entity });
        //     }
        // }
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
