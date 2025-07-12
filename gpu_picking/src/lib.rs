use bevy_ecs::prelude::Resource;
use futures_channel::oneshot;

const WORKGROUP_SIZE: u32 = 8;

/// GPU-based picking system that supports a parallelized selection box.
#[derive(Resource)]
pub struct GPUPicking {
    // --- Internal State & WGPU Resources ---
    pending_result_future: Option<oneshot::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    compute_pipeline: wgpu::ComputePipeline,
    pick_bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    results_buffer: wgpu::Buffer,
    staging_buffer: wgpu::Buffer,
    
    // --- Public State ---
    pub last_result: Option<Vec<u32>>,
}

impl GPUPicking {
    pub fn new(device: &wgpu::Device, id_texture_view: &wgpu::TextureView) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("GPU Picking Shader (Parallel)"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "picking.wgsl"
            ))),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("GPU Picking Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        // Uniforms
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        // ID Texture
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Uint,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        // Results Buffer
                        binding: 2,
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

        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("GPU Picking Pipeline"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("GPU Picking Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            })),
            module: &shader,
            entry_point: "main".into(),
            compilation_options: Default::default(),
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU Picking Uniform Buffer"),
            size: std::mem::size_of::<[u32; 4]>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let buffer_size = (256 + 1) * std::mem::size_of::<u32>() as u64;
        let results_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU Picking Results Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU Picking Staging Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

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
                    resource: results_buffer.as_entire_binding(),
                },
            ],
        });

        Self {
            pending_result_future: None,
            compute_pipeline,
            pick_bind_group,
            uniform_buffer,
            results_buffer,
            staging_buffer,
            last_result: None,
        }
    }

    /// Initiates a pick operation at the given screen coordinates.
    /// This function handles encoding and submitting GPU commands, and starting the async readback.
    pub fn pick(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, position: (u32, u32), pick_box_size: u32) {
        // Guard against starting a new pick while one is already in progress.
        if self.is_picking_in_progress() {
            return;
        }

        // --- 1. Encode Commands ---
        let pick_x = position.0.saturating_sub(pick_box_size / 2);
        let pick_y = position.1.saturating_sub(pick_box_size / 2);
        let selection_box = [pick_x, pick_y, pick_box_size, pick_box_size];
        
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("GPU Picking Encoder"),
        });

        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&selection_box));
        encoder.clear_buffer(&self.results_buffer, 0, None);

        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("GPU Picking Compute Pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.compute_pipeline);
            cpass.set_bind_group(0, &self.pick_bind_group, &[]);
            let dispatch_x = (selection_box[2] + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;
            let dispatch_y = (selection_box[3] + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;
            cpass.dispatch_workgroups(dispatch_x, dispatch_y, 1);
        }

        encoder.copy_buffer_to_buffer(
            &self.results_buffer,
            0,
            &self.staging_buffer,
            0,
            self.staging_buffer.size(),
        );
        
        let command_buffer = encoder.finish();

        // --- 2. Submit Commands to GPU ---
        println!("[GPU Picking] Submitting pick commands.");
        queue.submit(std::iter::once(command_buffer));

        // --- 3. Start Asynchronous Readback ---
        println!("[GPU Picking] Initiating readback.");
        let (sender, receiver) = oneshot::channel();
        self.staging_buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |v| {
                let _ = sender.send(v);
            });
        
        self.pending_result_future = Some(receiver);
    }

    /// Checks for a picking result. Returns `true` if an operation completed this frame.
    pub fn check_and_update_result(&mut self) -> bool {
        if let Some(mut receiver) = self.pending_result_future.take() {
            match receiver.try_recv() {
                // Case 1: Future is not ready yet. Put it back and report not-complete.
                Err(futures_channel::oneshot::Canceled) => {
                    self.pending_result_future = Some(receiver);
                    return false;
                }
                // Case 2: Future is ready. Process it.
                Ok(Some(result)) => {
                    if let Err(e) = result {
                        println!("[GPU Picking] Buffer mapping failed: {:?}", e);
                        self.last_result = None;
                    } else {
                        // Scope to ensure the mapped view is dropped before we unmap.
                        {
                            let mapped_range = self.staging_buffer.slice(..).get_mapped_range();
                            let result_slice: &[u32] = bytemuck::cast_slice(&mapped_range);
                            let count = result_slice[0] as usize;
                            if count > 0 {
                                let ids: Vec<u32> = result_slice[1..=count.min(256)].to_vec();
                                self.last_result = Some(ids);
                            } else {
                                self.last_result = None;
                            }
                        } // `mapped_range` is dropped here.
                    }
                    self.staging_buffer.unmap();
                    return true; // Operation IS complete.
                }
                // Case 3: Sender was dropped. Operation is over.
                Ok(None) => {
                    println!("[GPU Picking] Future channel was closed. Resetting state.");
                    self.last_result = None;
                    return true; // Operation IS complete (by failure).
                }
            }
        }
        // No future was pending.
        false
    }

    /// Returns a reference to the last picking result, if any.
    pub fn get_last_result(&self) -> Option<&Vec<u32>> {
        self.last_result.as_ref()
    }

    /// Clears the last picking result.
    pub fn clear_result(&mut self) {
        self.last_result = None;
    }

    /// Returns true if a picking operation is currently in flight.
    pub fn is_picking_in_progress(&self) -> bool {
        self.pending_result_future.is_some()
    }
}