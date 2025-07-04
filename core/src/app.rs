use bevy_ecs::prelude::*;
use bevy_ecs::{event::Events, schedule::ScheduleLabel};
use bevy_transform::{
    components::{GlobalTransform, Transform},
    systems::{mark_dirty_trees, propagate_parent_transforms, sync_simple_transforms},
};
use eframe::egui::{self};
use std::sync::Arc;

use crate::{
    config::Config,
    ecs::{
        camera::{Camera, OrbitCamera, camera_control_system, update_camera_transform_system},
        framerate::{FrameRate, frame_rate_system},
        input::{Input, keyboard_input_system},
        model::{initialize_asset_db_system, SpawnedEntities},
        ui::{EguiCtx, LastSize, UiState, ui_system},
    },
    renderer::{
        core::{WgpuDevice, WgpuQueue, WgpuRenderState, initialize_renderer},
        d3_pipeline::{
            render_d3_pipeline_system, setup_d3_pipeline_system, setup_depth_texture_system,
            update_camera_buffer_system,
        },
        events::ResizeEvent,
        scene::prepare_and_copy_scene_data_system,
        tonemapping_pass::{
            TonemappingBindGroup, TonemappingPass, resize_hdr_texture_system,
            setup_tonemapping_pass_system,
        },
        triangle_pass::{
            clear_hdr_texture_system, render_triangle_system, setup_triangle_pass_system,
        },
    },
};

#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct Update;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct RendererInitialization;

#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct Startup;

#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct Shutdown;

#[derive(Resource)]
pub struct InitialSize(pub wgpu::Extent3d);

pub struct Custom3d {
    pub world: World,
    schedule: Schedule,
}

impl Custom3d {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        let wgpu_render_state = cc.wgpu_render_state.as_ref()?;
        let config: Config = Default::default();

        let mut world = World::default();

        let device = Arc::new(wgpu_render_state.device.clone());
        let queue = Arc::new(wgpu_render_state.queue.clone());
        world.insert_resource(WgpuDevice(device.clone()));
        world.insert_resource(WgpuQueue(queue.clone()));
        world.insert_resource(WgpuRenderState(wgpu_render_state.clone()));
        initialize_renderer(&mut world, &device);

        world.insert_resource(config);
        world.init_resource::<Events<ResizeEvent>>();
        world.insert_resource(InitialSize(wgpu::Extent3d { width: 1280, height: 720, depth_or_array_layers: 1 }));
        world.init_resource::<OrbitCamera>();
        world.insert_resource(EguiCtx(cc.egui_ctx.clone()));
        
        // --- Startup Schedule ---
        let mut startup_schedule = Schedule::new(Startup);
        startup_schedule.add_systems(
            (
                setup_d3_pipeline_system,
                setup_tonemapping_pass_system,
                initialize_asset_db_system,
                (
                    setup_triangle_pass_system,
                    update_camera_transform_system,
                    setup_depth_texture_system,
                    resize_hdr_texture_system.after(setup_depth_texture_system),
                )
                    .after(setup_d3_pipeline_system)
                    .after(setup_tonemapping_pass_system),
            )
                .chain(),
        );
        startup_schedule.run(&mut world);
        world.remove_resource::<InitialSize>();

        let tonemapping_pass = world.resource::<TonemappingPass>().clone();
        let tonemapping_bind_group = world.remove_resource::<TonemappingBindGroup>().unwrap();

        let wgpu_render_state = world.get_resource_mut::<WgpuRenderState>().unwrap();

        wgpu_render_state.0.renderer.write().callback_resources.insert(tonemapping_pass);
        wgpu_render_state.0.renderer.write().callback_resources.insert(tonemapping_bind_group);

        world.init_resource::<FrameRate>();
        world.init_resource::<UiState>();
        world.init_resource::<LastSize>();
        world.init_resource::<SpawnedEntities>();

        world.spawn((Camera::default(), Transform::default(), GlobalTransform::default()));

        // --- Main Update Schedule ---
        let mut update_schedule = Schedule::new(Update);
        update_schedule.add_systems(
            (
                keyboard_input_system,
                camera_control_system,
                update_camera_transform_system.after(camera_control_system),
                frame_rate_system,
                ui_system,
                update_camera_buffer_system.after(ui_system),
                // Note: process_spawn_requests_system is removed. Spawning is handled by commands.
                prepare_and_copy_scene_data_system,
            ).chain(),
        );
        update_schedule.add_systems((sync_simple_transforms, mark_dirty_trees, propagate_parent_transforms).chain());
        update_schedule.add_systems(
            (
                clear_hdr_texture_system,
                render_triangle_system.run_if(|ui_state: Res<UiState>| ui_state.render_triangle),
                render_d3_pipeline_system.run_if(|ui_state: Res<UiState>| ui_state.render_model),
            ).chain(),
        );

        Some(Self { world, schedule: update_schedule })
    }
}

impl eframe::App for Custom3d {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.world.insert_resource(Input(ctx.input(|i| i.clone())));
        self.schedule.run(&mut self.world);
        ctx.request_repaint();
    }
}
