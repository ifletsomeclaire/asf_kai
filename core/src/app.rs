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
        camera::{Camera, OrbitCamera, camera_control_system, setup_camera_transform_system},
        framerate::{FrameRate, frame_rate_system},
        input::{Input, keyboard_input_system},
        model::{
            load_models_from_db_system, prepare_scene_data_system,
            process_asset_deallocations_system, generate_asset_reports_system, AssetReports,
        },
        ui::{EguiCtx, LastSize, SpawnerState, UiState, ui_system},
    },
    renderer::{
        assets::AssetServer,
        core::{WgpuDevice, WgpuQueue, WgpuRenderState, initialize_renderer},
        d3_pipeline::{
            render_d3_pipeline_system, setup_d3_pipeline_system, setup_depth_texture_system,
            update_camera_buffer_system,
        },
        events::ResizeEvent,
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
struct Startup;

#[derive(Resource)]
pub struct InitialSize(pub wgpu::Extent3d);

fn infallible_load_models_from_db_system(
    commands: Commands,
    asset_server: ResMut<AssetServer>,
    queue: Res<WgpuQueue>,
) {
    load_models_from_db_system(commands, asset_server, queue).unwrap();
}

pub struct Custom3d {
    pub world: World,
    schedule: Schedule,
}

impl Custom3d {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        let wgpu_render_state = cc.wgpu_render_state.as_ref()?;
        let config: Config = Default::default();

        let mut world = World::default();

        // --- Core Resource Initialization ---
        let device = Arc::new(wgpu_render_state.device.clone());
        let queue = Arc::new(wgpu_render_state.queue.clone());
        world.insert_resource(WgpuDevice(device.clone()));
        world.insert_resource(WgpuQueue(queue));
        world.insert_resource(WgpuRenderState(wgpu_render_state.clone()));
        initialize_renderer(&mut world, &device); // Initialize AssetServer here
        // --- End Core Resource Initialization ---

        world.insert_resource(config);
        world.init_resource::<Events<ResizeEvent>>();

        let initial_size_pixels = [1280, 720]; // Default value
        world.insert_resource(InitialSize(wgpu::Extent3d {
            width: initial_size_pixels[0],
            height: initial_size_pixels[1],
            depth_or_array_layers: 1,
        }));

        world.init_resource::<OrbitCamera>();
        world.insert_resource(EguiCtx(cc.egui_ctx.clone()));

        let mut startup_schedule = Schedule::new(Startup);
        startup_schedule.add_systems(
            (
                setup_triangle_pass_system,
                setup_tonemapping_pass_system,
                setup_depth_texture_system,
                setup_d3_pipeline_system,
                infallible_load_models_from_db_system,
                setup_camera_transform_system,
            )
                .chain(),
        );
        startup_schedule.run(&mut world);
        world.remove_resource::<InitialSize>();

        wgpu_render_state
            .renderer
            .write()
            .callback_resources
            .insert(world.resource::<TonemappingPass>().clone());
        wgpu_render_state
            .renderer
            .write()
            .callback_resources
            .insert(world.remove_resource::<TonemappingBindGroup>().unwrap());

        world.init_resource::<FrameRate>();
        world.init_resource::<UiState>();
        world.init_resource::<SpawnerState>();
        world.init_resource::<LastSize>();
        world.init_resource::<AssetReports>();

        world.spawn((
            Camera::default(),
            Transform::default(),
            GlobalTransform::default(),
        ));

        let mut schedule = Schedule::default();
        schedule.add_systems((keyboard_input_system, ui_system, frame_rate_system).chain());
        schedule.add_systems(
            (
                sync_simple_transforms,
                mark_dirty_trees,
                propagate_parent_transforms,
            )
                .chain(),
        );
        schedule.add_systems(
            (
                camera_control_system,
                update_camera_buffer_system,
                prepare_scene_data_system,
            )
                .chain(),
        );
        schedule
            .add_systems((resize_hdr_texture_system, update_camera_aspect_ratio_system).chain());
        schedule.add_systems(
            (
                clear_hdr_texture_system,
                render_triangle_system.run_if(|ui_state: Res<UiState>| ui_state.render_triangle),
                render_d3_pipeline_system.run_if(|ui_state: Res<UiState>| ui_state.render_model),
                process_asset_deallocations_system,
                generate_asset_reports_system,
            )
                .chain(),
        );

        Some(Self { world, schedule })
    }
}

pub fn update_camera_aspect_ratio_system(
    mut events: EventReader<ResizeEvent>,
    mut query: Query<&mut Camera>,
) {
    if let Ok(mut camera) = query.single_mut() {
        for event in events.read() {
            camera.aspect = event.0.width as f32 / event.0.height as f32;
        }
    }
}

impl eframe::App for Custom3d {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.world.insert_resource(Input(ctx.input(|i| i.clone())));
        self.schedule.run(&mut self.world);
        ctx.request_repaint();
    }
}
