



use bevy_ecs::{
    prelude::Resource,
    world::{FromWorld, World},
};
use glam::{Mat4, Vec3};
use redb::ReadOnlyTable;
use types::{AABB, MODEL_TABLE, TEXTURE_TABLE, ANIMATED_MODEL_TABLE, ANIMATION_TABLE};

use crate::{
    renderer::core::{WgpuDevice, WgpuQueue},
};

use self::{
    animated_meshlet::AnimatedMeshletManager, static_meshlet::MeshletManager,
    texture::TextureManager,
};

pub mod animated_meshlet;
pub mod static_meshlet;
pub mod texture;

#[derive(Resource)]
pub struct AssetServer {
    pub meshlet_manager: MeshletManager,
    pub animated_meshlet_manager: AnimatedMeshletManager,
    pub textures: TextureManager,
    pub texture_bind_group_layout: Option<wgpu::BindGroupLayout>,
    pub texture_bind_group: Option<wgpu::BindGroup>,
}

impl FromWorld for AssetServer {
    fn from_world(world: &mut World) -> Self {
        new(world)
    }
}

pub fn new(world: &mut World) -> AssetServer {
    let db = redb::Database::open("assets/models.redb").unwrap();
    let read_txn = db.begin_read().unwrap();
    let model_table: ReadOnlyTable<&str, &[u8]> =
        read_txn.open_table(MODEL_TABLE).unwrap();
    let animated_model_table: ReadOnlyTable<&str, &[u8]> =
        read_txn.open_table(ANIMATED_MODEL_TABLE).unwrap();
    let animation_table: ReadOnlyTable<&str, &[u8]> =
        read_txn.open_table(ANIMATION_TABLE).unwrap();
    let texture_table = read_txn.open_table(TEXTURE_TABLE).unwrap();

    let (texture_cpu_data, texture_map) = texture::load_textures_from_db(&texture_table);
    
    let device = world.resource::<WgpuDevice>();
    let meshlet_manager = MeshletManager::new(device, &model_table, &texture_map);
    let animated_meshlet_manager =
        AnimatedMeshletManager::new(device, &animated_model_table, &animation_table, &texture_map);

    let mut asset_server = AssetServer {
        meshlet_manager,
        animated_meshlet_manager,
        textures: texture::TextureManager {
            texture_cpu_data,
            texture_array: None,
            texture_sampler: None,
        },
        texture_bind_group_layout: None,
        texture_bind_group: None,
    };
    let queue = world.resource::<WgpuQueue>();
    create_texture_gpu_resources(&mut asset_server, device, queue);
    asset_server
}

fn create_texture_gpu_resources(
    asset_server: &mut AssetServer,
    device: &wgpu::Device,
    queue: &WgpuQueue,
) {
    let (texture_array, texture_sampler) =
        texture::create_texture_gpu_resources(device, queue, &asset_server.textures.texture_cpu_data);
    asset_server.textures.texture_array = Some(texture_array);
    asset_server.textures.texture_sampler = Some(texture_sampler);

    let texture_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Texture Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

    let texture_view = asset_server
        .textures
        .texture_array
        .as_ref()
        .unwrap()
        .create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

    let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Texture Bind Group"),
        layout: &texture_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(
                    asset_server.textures.texture_sampler.as_ref().unwrap(),
                ),
            },
        ],
    });

    asset_server.texture_bind_group_layout = Some(texture_bind_group_layout);
    asset_server.texture_bind_group = Some(texture_bind_group);
}

pub fn layout_models_in_a_row(aabbs: &[AABB]) -> Vec<Mat4> {
    let mut transforms = Vec::new();
    let mut current_x_offset = 0.0;
    let spacing = 1.0;

    for aabb in aabbs {
        let size = aabb.max - aabb.min;

        let transform = Mat4::from_translation(Vec3::new(
            current_x_offset - aabb.min.x,
            0.0,
            0.0,
        ));
        transforms.push(transform);

        current_x_offset += size.x + spacing;
    }

    transforms
}
