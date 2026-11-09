//! halation-gpu — single-pass WGSL develop engine (wgpu 23).
//!
//! Resources that used to be rebuilt per frame (input texture, output texture, readback buffer, bind
//! group, uniform buffers) now live in a cache keyed by image identity and size: dragging a slider
//! costs two `write_buffer` calls, one compute dispatch and one readback — the 16 MB RGBA32F upload
//! happens once per image.
//!
//! Note: iced 0.13 renders through its own pinned wgpu, which differs from this crate's wgpu 23, so a
//! single `Device` cannot be shared with the UI renderer yet; the frame is read back and handed to
//! iced as an RGBA handle. Zero-copy presentation lands with the iced upgrade.
//!
//! Binding layout (group 0): 0 = `DevelopUniforms`, 1 = `GpuDevelopMatrices`, 2 = input
//! `texture_2d<f32>` (Rgba32Float, linear ProPhoto), 3 = output storage texture (Rgba8Unorm, sRGB).

mod engine;

pub use engine::{GpuDevelopEngine, GpuEngineInfo};

// The Sprint-1 engine below is compiled out (`cfg(any())` is always false); it is superseded by
// `engine.rs`. Delete everything from here to the end of the file whenever convenient.
#[cfg(any())]
mod sprint1_engine {
use halation_core::{GpuDevelopMatrices, GpuDevelopUniforms, ImageBufferF32};
use std::sync::{mpsc, Mutex};

#[derive(Clone)]
pub struct GpuEngineInfo {
    pub backend_name: String,
    pub adapter_name: String,
}

impl GpuEngineInfo {
    pub fn probe() -> Self {
        pollster::block_on(async {
            let instance = wgpu::Instance::default();
            match instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                })
                .await
            {
                Some(adapter) => {
                    let info = adapter.get_info();
                    Self { backend_name: format!("{:?}", info.backend), adapter_name: info.name }
                }
                None => Self::fallback(),
            }
        })
    }

    pub fn fallback() -> Self {
        Self { backend_name: "CPU fallback".into(), adapter_name: "No compatible GPU".into() }
    }
}

struct FrameResources {
    image_id: u64,
    width: u32,
    height: u32,
    input_texture: wgpu::Texture,
    output_texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    readback: wgpu::Buffer,
    padded_bytes_per_row: u32,
}

pub struct GpuDevelopEngine {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    matrix_buffer: wgpu::Buffer,
    cache: Mutex<Option<FrameResources>>,
    info: GpuEngineInfo,
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

impl GpuDevelopEngine {
    pub fn new() -> Result<Self, String> {
        pollster::block_on(Self::new_async())
    }

    async fn new_async() -> Result<Self, String> {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| "GPU adapter unavailable".to_owned())?;
        let adapter_info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Halation develop device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
                    ..Default::default()
                },
                None,
            )
            .await
            .map_err(|e| format!("GPU device unavailable: {e}"))?;

        let shader = device.create_shader_module(wgpu::include_wgsl!("../../../assets/shaders/develop_pipeline.wgsl"));
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Develop compute bindings"),
            entries: &[
                uniform_entry(0),
                uniform_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Develop pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Halation develop compute pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_develop_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let make_uniform = |label: &str, size: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let uniform_buffer = make_uniform("Develop controls", std::mem::size_of::<GpuDevelopUniforms>());
        let matrix_buffer = make_uniform("Develop matrices", std::mem::size_of::<GpuDevelopMatrices>());

        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
            uniform_buffer,
            matrix_buffer,
            cache: Mutex::new(None),
            info: GpuEngineInfo { backend_name: format!("{:?}", adapter_info.backend), adapter_name: adapter_info.name },
        })
    }

    pub fn info(&self) -> &GpuEngineInfo {
        &self.info
    }

    fn create_resources(&self, width: u32, height: u32) -> FrameResources {
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let make_texture = |label: &str, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let input_texture = make_texture(
            "Develop input image",
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let output_texture = make_texture(
            "Develop output image",
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        let input_view = input_texture.create_view(&Default::default());
        let output_view = output_texture.create_view(&Default::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Develop compute bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.matrix_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&input_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&output_view) },
            ],
        });
        let padded_bytes_per_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Develop output readback"),
            size: padded_bytes_per_row as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        FrameResources { image_id: 0, width, height, input_texture, output_texture, bind_group, readback, padded_bytes_per_row }
    }

    fn upload(&self, res: &FrameResources, input: &ImageBufferF32) {
        let rgba: Vec<f32> = input.pixels.iter().flat_map(|p| [p[0], p[1], p[2], 1.0]).collect();
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &res.input_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&rgba),
            wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(input.width * 16), rows_per_image: Some(input.height) },
            wgpu::Extent3d { width: input.width, height: input.height, depth_or_array_layers: 1 },
        );
    }

    /// Develop `input` (linear ProPhoto) into sRGB-encoded 8-bit RGBA (`width * height * 4` bytes).
    pub fn process(
        &self,
        input: &ImageBufferF32,
        uniforms: &GpuDevelopUniforms,
        matrices: &GpuDevelopMatrices,
    ) -> Result<Vec<u8>, String> {
        if !input.is_valid() {
            return Err("Invalid image buffer dimensions".into());
        }
        let (width, height) = (input.width, input.height);
        let max_dim = self.device.limits().max_texture_dimension_2d;
        if width > max_dim || height > max_dim {
            return Err(format!("Image exceeds the GPU texture limit of {max_dim}px"));
        }

        let mut cache = self.cache.lock().map_err(|_| "GPU cache poisoned".to_owned())?;
        if cache.as_ref().map_or(true, |c| c.width != width || c.height != height) {
            *cache = Some(self.create_resources(width, height));
        }
        let res = cache.as_mut().ok_or_else(|| "GPU resources unavailable".to_owned())?;
        if res.image_id != input.id {
            self.upload(res, input);
            res.image_id = input.id;
        }

        self.queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));
        self.queue.write_buffer(&self.matrix_buffer, 0, bytemuck::bytes_of(matrices));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Develop compute commands") });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("Develop compute pass"), timestamp_writes: None });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &res.bind_group, &[]);
            pass.dispatch_workgroups(width.div_ceil(16), height.div_ceil(16), 1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &res.output_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &res.readback,
                layout: wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(res.padded_bytes_per_row), rows_per_image: Some(height) },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = res.readback.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().map_err(|e| e.to_string())?.map_err(|e| format!("GPU readback failed: {e}"))?;

        let mapped = slice.get_mapped_range();
        let bytes_per_row = (width * 4) as usize;
        let padded = res.padded_bytes_per_row as usize;
        let mut output = Vec::with_capacity(bytes_per_row * height as usize);
        for y in 0..height as usize {
            output.extend_from_slice(&mapped[y * padded..y * padded + bytes_per_row]);
        }
        drop(mapped);
        res.readback.unmap();
        Ok(output)
    }
}
}
