use bevy_ecs::prelude::*;
use bevy_ecs::{event::Events, schedule::ScheduleLabel};
use eframe::{
    egui::{self},
};
use std::sync::Arc;
use bevy_app::prelude::*;
use bevy_transform::TransformPlugin;
use bevy_transform::systems::{
    mark_dirty_trees, propagate_parent_transforms, sync_simple_transforms,
};

use crate::{
    config::Config,
    ecs::{
        camera::{camera_control_system, Camera, OrbitCamera},
        counter::{Counter, increment_counter_system},
        framerate::{FrameRate, frame_rate_system},
        input::{keyboard_input_system, Input},
        model::{load_static_models_system, prepare_scene_data_system},
        rotation::{DragDelta, RotationAngle, update_angle_system},
        ui::{EguiCtx, LastSize, UiState, ui_system},
    },
    renderer::{
        core::{WgpuDevice, WgpuQueue, WgpuRenderState},
        d3_pipeline::{
            render_d3_pipeline_system, setup_d3_pipeline_system, update_camera_buffer_system,
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

pub struct Custom3d {
    pub world: World,
    schedule: Schedule,
}

impl Custom3d {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        let wgpu_render_state = cc.wgpu_render_state.as_ref()?;
        let config: Config = Default::default();

        let mut world = World::default();
        world.insert_resource(WgpuDevice(Arc::new(wgpu_render_state.device.clone())));
        world.insert_resource(WgpuQueue(Arc::new(wgpu_render_state.queue.clone())));
        world.insert_resource(WgpuRenderState(wgpu_render_state.clone()));
        world.insert_resource(config);
        world.init_resource::<Events<ResizeEvent>>();

        let initial_size_pixels = [1280, 720]; // Default value
        world.insert_resource(InitialSize(wgpu::Extent3d {
            width: initial_size_pixels[0],
            height: initial_size_pixels[1],
            depth_or_array_layers: 1,
        }));

        let mut startup_schedule = Schedule::new(Startup);
        startup_schedule.add_systems(
            (
                setup_triangle_pass_system,
                setup_tonemapping_pass_system,
                (setup_d3_pipeline_system, load_static_models_system).chain(),
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
        world.init_resource::<RotationAngle>();
        world.init_resource::<DragDelta>();
        world.init_resource::<Counter>();
        world.init_resource::<UiState>();
        world.init_resource::<LastSize>();
        world.init_resource::<Camera>();
        world.init_resource::<OrbitCamera>();
        world.insert_resource(EguiCtx(cc.egui_ctx.clone()));

        let mut schedule = Schedule::default();
        schedule.add_systems(
            (
                keyboard_input_system,
                ui_system,
                update_angle_system,
                increment_counter_system,
                frame_rate_system,
                (
                    sync_simple_transforms,
                    mark_dirty_trees,
                    propagate_parent_transforms,
                    camera_control_system,
                    update_camera_buffer_system,
                    prepare_scene_data_system,
                    resize_hdr_texture_system,
                    update_camera_aspect_ratio_system,
                )
                    .chain(),
                clear_hdr_texture_system,
                render_triangle_system.run_if(|ui_state: Res<UiState>| ui_state.render_triangle),
                render_d3_pipeline_system.run_if(|ui_state: Res<UiState>| ui_state.render_model),
            )
                .chain(),
        );

        Some(Self { world, schedule })
    }
}

pub fn update_camera_aspect_ratio_system(
    mut events: EventReader<ResizeEvent>,
    mut camera: ResMut<Camera>,
) {
    for event in events.read() {
        camera.aspect = event.0.width as f32 / event.0.height as f32;
    }
}

impl eframe::App for Custom3d {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.world.insert_resource(Input(ctx.input(|i| i.clone())));
        self.schedule.run(&mut self.world);
        ctx.request_repaint();
    }
}
