use bevy_ecs::prelude::*;
use eframe::{
    egui::{self},
    egui_wgpu::Callback,
};
use std::sync::Arc;
use bevy_ecs::schedule::{ScheduleLabel};

use crate::{
    config::Config,
    ecs::{
        counter::{increment_counter_system, Counter},
        framerate::{frame_rate_system, FrameRate},
        rotation::{update_angle_system, DragDelta, RotationAngle},
    },
    renderer::{
        core::{WgpuDevice, WgpuQueue, WgpuRenderState},
        tonemapping_pass::{setup_tonemapping_pass_system, FinalBlitCallback, TonemappingResources},
        triangle_pass::{render_triangle_system, setup_triangle_pass_system},
    },
};

#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct Startup;

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

        let mut startup_schedule = Schedule::new(Startup);
        startup_schedule.add_systems((setup_triangle_pass_system, setup_tonemapping_pass_system).chain());
        startup_schedule.run(&mut world);

        wgpu_render_state
            .renderer
            .write()
            .callback_resources
            .insert(world.remove_resource::<TonemappingResources>().unwrap());

        world.init_resource::<FrameRate>();
        world.init_resource::<RotationAngle>();
        world.init_resource::<DragDelta>();
        world.init_resource::<Counter>();

        let mut schedule = Schedule::default();
        schedule.add_systems((
            update_angle_system,
            increment_counter_system,
            frame_rate_system,
            render_triangle_system,
        ));

        Some(Self { world, schedule })
    }
}

impl eframe::App for Custom3d {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let rect = ui.max_rect();
            let response = ui.interact(rect, ui.id().with("3d_view"), egui::Sense::drag());

            self.world
                .insert_resource(DragDelta(response.drag_delta()));
            self.schedule.run(&mut self.world);

            let callback = Callback::new_paint_callback(rect, FinalBlitCallback {});
            ui.painter().add(callback);
        });

        egui::Window::new("Overlay").show(ctx, |ui| {
            ui.label("You can put any egui widget here.");
            if ui.button("A button").clicked() {
                // take some action
            }
            let mut angle = self.world.resource_mut::<RotationAngle>();
            ui.add(egui::Slider::new(&mut angle.0, 0.0..=360.0).text("Angle"));
            
            ui.horizontal(|ui| {
                let counter = self.world.resource::<Counter>();
                ui.label(format!("Counter: {}", counter.0));
                let frame_rate = self.world.resource::<FrameRate>();
                ui.label(format!("FPS: {:.1}", frame_rate.fps));
            });

            ui.separator();

            let mut config = self.world.resource_mut::<Config>();
            if ui.checkbox(&mut config.vsync, "V-Sync").changed() {
                config.save();
            }
            ui.label("(Requires restart)");
        });

        ctx.request_repaint();
    }
}
