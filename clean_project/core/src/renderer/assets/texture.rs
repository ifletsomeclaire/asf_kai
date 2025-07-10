use redb::{ReadOnlyTable, ReadableTable};
use std::collections::HashMap;

pub struct TextureManager {
    pub texture_cpu_data: Vec<image::DynamicImage>,
    pub texture_array: Option<wgpu::Texture>,
    pub texture_sampler: Option<wgpu::Sampler>,
}

pub fn load_textures_from_db(
    texture_table: &ReadOnlyTable<&str, &[u8]>,
) -> (Vec<image::DynamicImage>, HashMap<String, u32>) {
    let mut texture_map = HashMap::new();
    let mut texture_cpu_data = Vec::new();

    // Create a fallback texture
    let fallback_texture = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        1,
        1,
        image::Rgba([255, 0, 255, 255]),
    ));
    texture_cpu_data.push(fallback_texture);

    for result in texture_table.iter().unwrap() {
        let (name_bytes, texture_data) = result.unwrap();
        let name = name_bytes.value();
        println!("[Asset Loading] Loading texture: {name}");
        if let Ok(image) = image::load_from_memory(texture_data.value()) {
            texture_map.insert(name.to_string(), texture_cpu_data.len() as u32);
            texture_cpu_data.push(image);
        }
    }

    (texture_cpu_data, texture_map)
}

pub fn create_texture_gpu_resources(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture_cpu_data: &[image::DynamicImage],
) -> (wgpu::Texture, wgpu::Sampler) {
    let (max_width, max_height) = texture_cpu_data
        .iter()
        .fold((0, 0), |(max_w, max_h), img| {
            (max_w.max(img.width()), max_h.max(img.height()))
        });

    let texture_array = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Texture Array"),
        size: wgpu::Extent3d {
            width: max_width,
            height: max_height,
            depth_or_array_layers: texture_cpu_data.len() as u32,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    for (i, image) in texture_cpu_data.iter().enumerate() {
        let rgba_image = image.to_rgba8();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture_array,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: i as u32,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &rgba_image,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * rgba_image.width()),
                rows_per_image: Some(rgba_image.height()),
            },
            wgpu::Extent3d {
                width: rgba_image.width(),
                height: rgba_image.height(),
                depth_or_array_layers: 1,
            },
        );
    }

    let texture_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Texture Sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });

    (texture_array, texture_sampler)
} 