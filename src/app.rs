use std::num::NonZeroU64;

use bevy_ecs::prelude::*;
use eframe::{
    egui_wgpu::{
        wgpu::{util::DeviceExt, *},
        Callback,
    },
    egui,
};

use crate::{
    config::Config,
    ecs::{
        frame_rate_system, increment_counter_system, update_angle_system, Counter, DragDelta,
        FrameRate, RotationAngle,
    },
    render::{CustomTriangleCallback, TriangleRenderResources},
};

pub struct Custom3d {
    pub world: World,
    schedule: Schedule,
}

impl Custom3d {
    pub fn new<'a>(cc: &'a eframe::CreationContext<'a>) -> Option<Self> {
        let wgpu_render_state = cc.wgpu_render_state.as_ref()?;
        let device = &wgpu_render_state.device;

        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("custom3d"),
            source: ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("custom3d"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(4),
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("custom3d"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("custom3d"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu_render_state.target_format.into())],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let uniform_buffer = device.create_buffer_init(&util::BufferInitDescriptor {
            label: Some("custom3d"),
            contents: bytemuck::cast_slice(&[0.0_f32]),
            usage: BufferUsages::COPY_DST | BufferUsages::UNIFORM,
        });

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("custom3d"),
            layout: &bind_group_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        wgpu_render_state
            .renderer
            .write()
            .callback_resources
            .insert(TriangleRenderResources {
                pipeline,
                bind_group,
                uniform_buffer,
            });

        let mut world = World::default();
        world.init_resource::<RotationAngle>();
        world.init_resource::<DragDelta>();
        world.init_resource::<Counter>();
        world.init_resource::<FrameRate>();

        let mut schedule = Schedule::default();
        schedule.add_systems((
            update_angle_system,
            increment_counter_system,
            frame_rate_system,
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

            let angle = self.world.resource::<RotationAngle>().0;

            ui.painter()
                .add(Callback::new_paint_callback(rect, CustomTriangleCallback { angle }));

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
        });

        ctx.request_repaint();
    }
}
