use bevy_ecs::prelude::*;
use bevy_ecs::{event::Events, schedule::ScheduleLabel};
use bevy_transform::{
    components::{GlobalTransform, Transform},
    systems::{mark_dirty_trees, propagate_parent_transforms, sync_simple_transforms},
};
use eframe::egui::{self};
use glam::Mat4;
use std::sync::Arc;

use crate::{
    config::Config,
    ecs::{
        animation::{animation_system, AnimationPlayer, BoneMatrices, AnimatedInstance},
        camera::{Camera, OrbitCamera, camera_control_system, update_camera_transform_system},
        time::{Time, time_system},
        input::{Input, keyboard_input_system},
        ui::{EguiCtx, LastSize, UiState, ui_system},
    },
    renderer::{
        assets::AssetServer,
        core::{WgpuDevice, WgpuQueue, WgpuRenderState},
        events::ResizeEvent,
        pipelines::{
            d3_animated_pipeline::{render_d3_animated_pipeline_system, D3AnimatedPipeline, CameraUniformBuffer},
            d3_pipeline::render_d3_pipeline_system,
            tonemapping::{
                resize_hdr_texture_system, setup_tonemapping_pass_system, TonemappingBindGroup,
                TonemappingPass,
            },
            triangle::{
                clear_hdr_texture_system, render_triangle_system, setup_triangle_pass_system,
            },
        },
    },
};

#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct Update;

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
        println!("--- Creating Custom3d ---");
        let wgpu_render_state = cc.wgpu_render_state.as_ref()?;
        let config: Config = Default::default();

        let mut world = World::default();

        let device = Arc::new(wgpu_render_state.device.clone());
        let queue = Arc::new(wgpu_render_state.queue.clone());
        world.insert_resource(WgpuDevice(device.clone()));
        world.insert_resource(WgpuQueue(queue.clone()));
        world.insert_resource(WgpuRenderState(wgpu_render_state.clone()));
 

        world.insert_resource(config);
        world.init_resource::<Events<ResizeEvent>>();
        world.insert_resource(InitialSize(wgpu::Extent3d {
            width: 1280,
            height: 720,
            depth_or_array_layers: 1,
        }));
        world.init_resource::<OrbitCamera>();
        world.init_resource::<AssetServer>();
        world.insert_resource(EguiCtx(cc.egui_ctx.clone()));

        // --- Create persistent resources ---
        let asset_server = world.resource::<AssetServer>();
        let d3_animated_pipeline = D3AnimatedPipeline::new(
            &device,
            &asset_server,
            wgpu::TextureFormat::Rgba16Float,
        );

        let camera_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera_uniform_buffer"),
            size: std::mem::size_of::<Mat4>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        
        world.insert_resource(d3_animated_pipeline);
        world.insert_resource(CameraUniformBuffer(camera_uniform_buffer));
        
        // --- Startup Schedule ---
        let mut startup_schedule = Schedule::new(Startup);
        startup_schedule.add_systems(
            (
                setup_tonemapping_pass_system,
                (
                    setup_triangle_pass_system,
                    update_camera_transform_system,
                    resize_hdr_texture_system,
                )
                    .after(setup_tonemapping_pass_system),
            )
                .chain(),
        );
        startup_schedule.run(&mut world);
        world.remove_resource::<InitialSize>();

        let tonemapping_pass = world.resource::<TonemappingPass>().clone();
        let tonemapping_bind_group = world.remove_resource::<TonemappingBindGroup>().unwrap();

        let wgpu_render_state = world.get_resource_mut::<WgpuRenderState>().unwrap();

        wgpu_render_state
            .0
            .renderer
            .write()
            .callback_resources
            .insert(tonemapping_pass);
        wgpu_render_state
            .0
            .renderer
            .write()
            .callback_resources
            .insert(tonemapping_bind_group);

        world.init_resource::<Time>();
        world.init_resource::<UiState>();
        world.init_resource::<LastSize>();
        
        world.spawn((
            Camera::default(),
            Transform::default(),
            GlobalTransform::default(),
        ));

        // Spawn a test animated entity
        let model_names = [
            "Animation_Running_withSkin",
            // "Animation_Walking_withSkin", 
            // "Animation_RunFast_withSkin",
            // "Animation_Axe_Spin_Attack_withSkin",
        ];
        
  

        let animations = [
            "Armature|running|baselayer",
            // "Armature|walking_man|baselayer", 
            // "Armature|RunFast|baselayer",
            // "Armature|Axe_Spin_Attack|baselayer",
        ];

        // Debug: Print available animated models
        let asset_server = world.resource::<AssetServer>();
        println!("[App] Available animated models:");
        for model_name in asset_server.animated_meshlet_manager.model_meshlets.keys() {
            println!("  - {}", model_name);
        }

        // Debug: Print available animations
        println!("[App] Available animations:");
        for anim_name in asset_server.animated_meshlet_manager.animations.keys() {
            println!("  - {}", anim_name);
        }

        // Debug: Print skeleton information
        println!("[App] Skeleton information:");
        for (model_name, skeleton) in &asset_server.animated_meshlet_manager.skeletons {
            println!("  Model '{}': {} bones", model_name, skeleton.bones.len());
            for (i, bone) in skeleton.bones.iter().take(3).enumerate() {
                println!("    Bone {}: '{}' (parent: {})", 
                    i, bone.name, 
                    bone.parent_index.map(|p| p.to_string()).unwrap_or_else(|| "None".to_string()));
            }
            if skeleton.bones.len() > 3 {
                println!("    ... and {} more bones", skeleton.bones.len() - 3);
            }
        }

        // Spawn one instance for each model type
        for (i, (model_name, anim_name)) in model_names.iter().zip(animations.iter()).enumerate() {
             // Create a slight offset for each model so they don't overlap
            let mut transform = Transform::from_xyz(i as f32 * 2.0, 0.0, 0.0);
            transform.scale = glam::Vec3::splat(1.0); // Changed from 0.01 to isolate scale issue

            println!("[App] Spawning instance {}: model='{}', animation='{}', transform={:?}", 
                i, model_name, anim_name, transform.translation);

            world.spawn((
                AnimatedInstance {
                    model_name: model_name.to_string(),
                },
                AnimationPlayer {
                    animation_name: anim_name.to_string(),
                    ..Default::default()
                },
                BoneMatrices {
                    matrices: vec![Mat4::IDENTITY; 256],
                },
                transform,
                GlobalTransform::default(),
            ));
        }

        println!("[App] Spawned {} animated test instances.", animations.len());


        // --- Main Update Schedule ---
        let mut update_schedule = Schedule::new(Update);
        update_schedule.add_systems(
            (
                keyboard_input_system,
                camera_control_system,
                update_camera_transform_system,
                time_system,
                animation_system,
                ui_system,
            )
                .chain(),
        );
        update_schedule.add_systems(
            (
                sync_simple_transforms,
                mark_dirty_trees,
                propagate_parent_transforms,
            )
                .chain(),
        );
        update_schedule.add_systems(
            (
                clear_hdr_texture_system,
                render_triangle_system.run_if(|ui_state: Res<UiState>| ui_state.render_triangle),
                render_d3_pipeline_system
                    .run_if(|ui_state: Res<UiState>| ui_state.render_static_meshlets),
                render_d3_animated_pipeline_system
                    .run_if(|ui_state: Res<UiState>| ui_state.render_animated_meshlets),
            )
                .chain(),
        );

        Some(Self {
            world,
            schedule: update_schedule,
        })
    }
}

impl eframe::App for Custom3d {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.world.insert_resource(Input(ctx.input(|i| i.clone())));
        self.schedule.run(&mut self.world);
        ctx.request_repaint();
    }
}
