

use futures_channel::oneshot;
use bevy_ecs::prelude::Resource;

/// GPU-based picking system that manages its own asynchronous state.
/// Provides simple synchronous polling interface for integration with game engines.
#[derive(Resource)]
pub struct GPUPicking {
    // --- Public State ---
    pub selection_origin: Option<[u32; 2]>,
    pub last_result: Option<Vec<u32>>,

    // --- Internal State & WGPU Resources ---
    // Tracks the in-flight async mapping operation.
    pending_result_future: Option<oneshot::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    // Track if we need to submit a pick command
    needs_pick_submit: bool,

    compute_pipeline: wgpu::ComputePipeline,
    pick_bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    seen_ids_buffer: wgpu::Buffer,
    results_buffer: wgpu::Buffer,
    staging_buffer: wgpu::Buffer,
}

impl GPUPicking {
    /// Creates a new GPU picking system.
    /// 
    /// # Arguments
    /// * `device` - The WGPU device
    /// * `id_texture_view` - The texture view containing entity IDs for picking
    pub fn new(device: &wgpu::Device, id_texture_view: &wgpu::TextureView) -> Self {
        // Create compute shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("GPU Picking Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!("picking.wgsl"))),
        });

        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("GPU Picking Bind Group Layout"),
            entries: &[
                // Uniform buffer for pick coordinates
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // ID texture
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Seen IDs buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Results buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Create compute pipeline
        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("GPU Picking Pipeline"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("GPU Picking Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            })),
            module: &shader,
            entry_point: "main".into(),
            cache: None,
            compilation_options: Default::default(),
        });

        // Create buffers
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU Picking Uniform Buffer"),
            size: std::mem::size_of::<[u32; 2]>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let seen_ids_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU Picking Seen IDs Buffer"),
            size: 1024 * std::mem::size_of::<u32>() as u64, // 1024 unique IDs max
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let results_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU Picking Results Buffer"),
            size: (256 + 1) * std::mem::size_of::<u32>() as u64, // 256 results + count
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU Picking Staging Buffer"),
            size: (256 + 1) * std::mem::size_of::<u32>() as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create bind group
        let pick_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GPU Picking Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(id_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: seen_ids_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: results_buffer.as_entire_binding(),
                },
            ],
        });

        Self {
            selection_origin: None,
            last_result: None,
            pending_result_future: None,
            needs_pick_submit: false,
            compute_pipeline,
            pick_bind_group,
            uniform_buffer,
            seen_ids_buffer,
            results_buffer,
            staging_buffer,
        }
    }

    /// Sets the pick coordinates for the next pick operation.
    pub fn set_pick_coordinates(&mut self, x: u32, y: u32) {
        self.selection_origin = Some([x, y]);
    }

    /// Encodes GPU commands for picking WITHOUT initiating the async readback.
    /// Returns true if commands were encoded.
    pub fn encode_pick_commands(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> Option<wgpu::CommandBuffer> {
        // Only run if a pick is triggered and no other readback is in flight.
        if self.selection_origin.is_some() && self.pending_result_future.is_none() {
            if let Some(origin) = self.selection_origin.take() {
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("GPU Picking Encoder"),
                });

                // Update uniform buffer with pick coordinates
                queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&origin));

                // Clear buffers
                encoder.clear_buffer(&self.seen_ids_buffer, 0, None);
                encoder.clear_buffer(&self.results_buffer, 0, None);

                // Dispatch compute pass
                let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("GPU Picking Compute Pass"),
                    timestamp_writes: None,
                });
                compute_pass.set_pipeline(&self.compute_pipeline);
                compute_pass.set_bind_group(0, &self.pick_bind_group, &[]);
                compute_pass.dispatch_workgroups(1, 1, 1);
                drop(compute_pass);

                // Copy results to the staging buffer.
                encoder.copy_buffer_to_buffer(
                    &self.results_buffer, 0,
                    &self.staging_buffer, 0,
                    self.staging_buffer.size(),
                );

                self.needs_pick_submit = true;
                return Some(encoder.finish());
            }
        }
        None
    }

    /// Initiates the async map operation after commands have been submitted.
    pub fn start_async_readback(&mut self) {
        if self.needs_pick_submit && self.pending_result_future.is_none() {
            self.needs_pick_submit = false;
            
            // Start the async map operation and store the future.
            let (sender, receiver) = oneshot::channel();
            self.staging_buffer.slice(..).map_async(wgpu::MapMode::Read, move |v| {
                let _ = sender.send(v);
            });
            self.pending_result_future = Some(receiver);
        }
    }

    /// Synchronously checks if a pending result is ready and updates `last_result`.
    /// Call this once per frame in your main application loop.
    pub fn check_and_update_result(&mut self) {
        if let Some(receiver) = self.pending_result_future.as_mut() {
            // Check if the future is ready without blocking.
            match receiver.try_recv() {
                Ok(Some(Ok(()))) => {
                    // Success! Data is ready on the staging buffer.
                    let data = self.staging_buffer.slice(..).get_mapped_range();
                    let result_slice: &[u32] = bytemuck::cast_slice(&data);
                    let count = result_slice[0] as usize;

                    if count > 0 && count <= 256 {
                        self.last_result = Some(result_slice[1..=count].to_vec());
                    } else {
                        self.last_result = None;
                    }

                    drop(data);
                    self.staging_buffer.unmap();
                    self.pending_result_future = None; // Reset the future.
                },
                Ok(Some(Err(_))) => {
                    // An error occurred (mapping failed).
                    self.staging_buffer.unmap();
                    self.last_result = None;
                    self.pending_result_future = None; // Reset on error.
                },
                Ok(None) | Err(_) => {
                    // Channel closed or not ready yet, do nothing.
                }
            }
        }
    }

    /// Returns the last picking result, if any.
    pub fn get_last_result(&self) -> Option<&Vec<u32>> {
        self.last_result.as_ref()
    }

    /// Clears the last result.
    pub fn clear_result(&mut self) {
        self.last_result = None;
    }

    /// Returns true if a picking operation is currently in progress.
    pub fn is_picking_in_progress(&self) -> bool {
        self.pending_result_future.is_some()
    }
}
