use bevy_ecs::prelude::*;
use bevy_ecs::{event::Events, schedule::ScheduleLabel};
use bevy_transform::{
    components::{GlobalTransform, Transform},
    systems::{mark_dirty_trees, propagate_parent_transforms, sync_simple_transforms},
};
use eframe::egui::{self};
use glam::Mat4;
use gpu_picking::GPUPicking;
use wgpu::{PollStatus, PollType};
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
                TonemappingPass, clear_hdr_and_id_texture_system,
            },
            triangle::{
                render_triangle_system, setup_triangle_pass_system,
            },
        },
    },
};

// Component to mark selected entities
#[derive(Component)]
pub struct Selected;


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
        log::info!("--- Creating Custom3d ---");
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
            asset_server,
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

        // Create GPU picking system
        let id_texture = world.resource::<crate::renderer::pipelines::tonemapping::IdTexture>();
        let gpu_picking = gpu_picking::GPUPicking::new(&device, &id_texture.view);
        world.insert_resource(gpu_picking);

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
            "Animation_Walking_withSkin", 
            "Animation_RunFast_withSkin",
            "Animation_Axe_Spin_Attack_withSkin",
        ];
        
  

        let animations = [
            "Armature|running|baselayer",
            "Armature|walking_man|baselayer", 
            "Armature|RunFast|baselayer",
            "Armature|Axe_Spin_Attack|baselayer",
        ];

        // Debug: Print available animated models
        let asset_server = world.resource::<AssetServer>();
        log::info!("[App] Available animated models:");
        for model_name in asset_server.animated_meshlet_manager.model_meshlets.keys() {
            log::info!("  - {model_name}");
        }

        // Debug: Print available animations
        log::info!("[App] Available animations:");
        for anim_name in asset_server.animated_meshlet_manager.animations.keys() {
            log::info!("  - {anim_name}");
        }

        // Debug: Print skeleton information
        log::info!("[App] Skeleton information:");
        for (model_name, skeleton) in &asset_server.animated_meshlet_manager.skeletons {
            log::info!("  Model '{}': {} bones", model_name, skeleton.bones.len());
            for (i, bone) in skeleton.bones.iter().take(3).enumerate() {
                log::info!("    Bone {}: '{}' (parent: {})", 
                    i, bone.name, 
                    bone.parent_index.map(|p| p.to_string()).unwrap_or_else(|| "None".to_string()));
            }
            if skeleton.bones.len() > 3 {
                log::info!("    ... and {} more bones", skeleton.bones.len() - 3);
            }
        }

        // Spawn one instance for each model type
        for (i, (model_name, anim_name)) in model_names.iter().zip(animations.iter()).enumerate() {
             // Create a slight offset for each model so they don't overlap
            let mut transform = Transform::from_xyz(i as f32 * 3.0, 0.0, 0.0);
            transform.scale = glam::Vec3::splat(0.02); // Use smaller scale to prevent overlapping

            log::info!("[App] Spawning instance {}: model='{}', animation='{}', transform={:?}", 
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

        log::info!("[App] Spawned {} animated test instances.", animations.len());


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
                clear_hdr_and_id_texture_system,
                render_triangle_system.run_if(|ui_state: Res<UiState>| ui_state.render_triangle),
                render_d3_pipeline_system
                    .run_if(|ui_state: Res<UiState>| ui_state.render_static_meshlets),
                render_d3_animated_pipeline_system
                    .run_if(|ui_state: Res<UiState>| ui_state.render_animated_meshlets),
                gpu_picking_system,
              
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

pub fn gpu_picking_system(
    device: Res<WgpuDevice>,
    queue: Res<WgpuQueue>,
    mut gpu_picking: ResMut<GPUPicking>,
    input: Res<Input>,
    egui_ctx: Res<EguiCtx>,
    mut commands: Commands,
) {
    device.poll(PollType::Poll);

    // --- Part 1: Check for and process results from the previous frame ---
    let pick_op_completed_this_frame = gpu_picking.check_and_update_result();

    if let Some(entity_indices) = gpu_picking.get_last_result() {
        println!("[GPU Picking] Received result with {} entities.", entity_indices.len());
        for &index in entity_indices {
            if let Ok(entity_commands) = commands.get_entity(Entity::from_raw(index)) {
                 println!("[GPU Picking] Selecting entity {:?}", entity_commands.id());
            
            }
        }
        gpu_picking.clear_result();
    }

    // --- Part 2: Check for new input to start a new picking operation ---
    if egui_ctx.wants_pointer_input() || gpu_picking.is_picking_in_progress() || pick_op_completed_this_frame {
        return;
    }

    if input.0.pointer.primary_pressed() {
        if let Some(pos) = input.0.pointer.latest_pos() {
            let x = pos.x.round() as u32;
            let y = pos.y.round() as u32;
            
            // Call the single, merged pick function.
            gpu_picking.pick(&device, &queue, (x, y), 5);
        }
    }
}