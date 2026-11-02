use bytemuck::{Pod, Zeroable};
use halation_core::{DevelopParameters, ImageBufferF32};
use std::sync::mpsc;
use wgpu::util::DeviceExt;

#[derive(Clone)]
pub struct GpuEngineInfo {
    pub backend_name: String,
    pub adapter_name: String,
    pub display_profile: String,
}

pub struct GpuDevelopEngine {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    display_lut_texture: wgpu::Texture,
    display_lut_view: wgpu::TextureView,
    display_sampler: wgpu::Sampler,
    blue_noise_view: wgpu::TextureView,
    display_profile_key: u64,
    info: GpuEngineInfo,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DevelopUniforms {
    exposure: f32,
    contrast: f32,
    highlights: f32,
    shadows: f32,
    whites: f32,
    blacks: f32,
    temperature: f32,
    tint: f32,
    white_balance: [[f32; 4]; 3],
    tone_points: [[f32; 4]; 20],
    grading: [[f32; 4]; 3],
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
                    Self {
                        backend_name: format!("{:?}", info.backend),
                        adapter_name: info.name,
                        display_profile: halation_color::display::discover_display_profile().name,
                    }
                }
                None => Self::fallback(),
            }
        })
    }

    pub fn fallback() -> Self {
        Self {
            backend_name: "CPU fallback".into(),
            adapter_name: "No compatible GPU".into(),
            display_profile: "sRGB fallback".into(),
        }
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
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Halation develop device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    ..Default::default()
                },
                None,
            )
            .await
            .map_err(|e| format!("GPU device unavailable: {e}"))?;

        let shader = device.create_shader_module(wgpu::include_wgsl!(
            "../../../assets/shaders/develop_pipeline.wgsl"
        ));
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Develop compute bindings"),
            entries: &[
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
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

        let profile = halation_color::display::discover_display_profile();
        let lut = halation_color::display::build_display_lut(&profile, 33)
            .or_else(|_| {
                halation_color::display::build_display_lut(
                    &halation_color::display::DisplayProfile {
                        name: "sRGB fallback".into(),
                        icc_data: None,
                    },
                    33,
                )
            })
            .map_err(|e| format!("Could not build display color LUT: {e}"))?;
        let lut_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Display profile 3D LUT"),
            size: wgpu::Extent3d {
                width: lut.size,
                height: lut.size,
                depth_or_array_layers: lut.size,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &lut_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&lut.rgba),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(lut.size * 8),
                rows_per_image: Some(lut.size),
            },
            wgpu::Extent3d {
                width: lut.size,
                height: lut.size,
                depth_or_array_layers: lut.size,
            },
        );
        let display_lut_view = lut_texture.create_view(&Default::default());
        let display_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Display LUT sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let noise_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("64x64 blue noise dither tile"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &noise_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &blue_noise_tile(),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(64),
                rows_per_image: Some(64),
            },
            wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
        );
        let blue_noise_view = noise_texture.create_view(&Default::default());
        let display_profile_key = profile_key(&profile);

        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
            display_lut_texture: lut_texture,
            display_lut_view,
            display_sampler,
            blue_noise_view,
            display_profile_key,
            info: GpuEngineInfo {
                backend_name: format!("{:?}", info.backend),
                adapter_name: info.name,
                display_profile: profile.name,
            },
        })
    }

    pub fn info(&self) -> &GpuEngineInfo {
        &self.info
    }

    /// Requery the active display and replace the texture in place when its ICC profile changes.
    pub fn refresh_display_profile(&mut self) -> bool {
        let profile = halation_color::display::discover_display_profile();
        let key = profile_key(&profile);
        if key == self.display_profile_key {
            return false;
        }
        let Ok(lut) = halation_color::display::build_display_lut(&profile, 33) else {
            return false;
        };
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.display_lut_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&lut.rgba),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(lut.size * 8),
                rows_per_image: Some(lut.size),
            },
            wgpu::Extent3d {
                width: lut.size,
                height: lut.size,
                depth_or_array_layers: lut.size,
            },
        );
        self.info.display_profile = profile.name;
        self.display_profile_key = key;
        true
    }

    pub fn process(
        &self,
        input: &ImageBufferF32,
        params: &DevelopParameters,
    ) -> Result<Vec<u8>, String> {
        let width = input.width;
        let height = input.height;
        if width == 0 || height == 0 || input.pixels.len() != (width * height) as usize {
            return Err("Invalid image buffer dimensions".into());
        }

        let rgba: Vec<f32> = input
            .pixels
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 1.0])
            .collect();
        let input_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Develop input image"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &input_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&rgba),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width * 16),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let output_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Develop output image"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let uniforms = DevelopUniforms {
            exposure: params.exposure,
            contrast: params.contrast,
            highlights: params.highlights,
            shadows: params.shadows,
            whites: params.whites,
            blacks: params.blacks,
            temperature: params.temperature,
            tint: params.tint,
            white_balance: {
                let m = halation_color::white_balance_matrix(params.temperature, params.tint);
                [
                    [m[0][0], m[0][1], m[0][2], 0.0],
                    [m[1][0], m[1][1], m[1][2], 0.0],
                    [m[2][0], m[2][1], m[2][2], 0.0],
                ]
            },
            tone_points: {
                let mut points = [[0.0; 4]; 20];
                let defaults;
                let tone_curve = if params.tone_curve.len() == 20 {
                    params.tone_curve.as_slice()
                } else {
                    defaults = DevelopParameters::default();
                    defaults.tone_curve.as_slice()
                };
                for (index, point) in tone_curve.iter().take(20).enumerate() {
                    points[index] = [point[0], point[1], 0.0, 0.0];
                }
                points
            },
            grading: std::array::from_fn(|i| {
                [
                    params.color_grading[i][0],
                    params.color_grading[i][1],
                    params.color_grading[i][2],
                    0.0,
                ]
            }),
        };
        let uniform_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Develop controls"),
                contents: bytemuck::bytes_of(&uniforms),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let input_view = input_texture.create_view(&Default::default());
        let output_view = output_texture.create_view(&Default::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Develop compute bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&input_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&output_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.display_lut_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.display_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&self.blue_noise_view),
                },
            ],
        });

        let bytes_per_row = width * 4;
        let padded_bytes_per_row = bytes_per_row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Develop output readback"),
            size: (padded_bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Develop compute commands"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Develop compute pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(width.div_ceil(16), height.div_ceil(16), 1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &output_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = readback.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("GPU readback failed: {e}"))?;
        let mapped = slice.get_mapped_range();
        let mut output = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            let row = &mapped[(y * padded_bytes_per_row) as usize
                ..(y * padded_bytes_per_row + bytes_per_row) as usize];
            for pixel in row.chunks_exact(4) {
                output.extend_from_slice(pixel);
            }
        }
        drop(mapped);
        readback.unmap();
        Ok(output)
    }
}

fn profile_key(profile: &halation_color::display::DisplayProfile) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    profile.name.hash(&mut hash);
    if let Some(bytes) = &profile.icc_data {
        bytes.hash(&mut hash);
    }
    hash.finish()
}

/// Toroidal farthest-point ranking creates a tile whose low thresholds are spatially
/// well distributed, avoiding the visible clumps and diagonal structure of white noise.
fn blue_noise_tile() -> Vec<u8> {
    const SIDE: usize = 64;
    const COUNT: usize = SIDE * SIDE;
    let mut nearest = vec![u16::MAX; COUNT];
    let mut ranks = vec![0_u8; COUNT];
    let mut chosen = vec![false; COUNT];
    let mut last = 0_usize;
    for rank in 0..COUNT {
        chosen[last] = true;
        ranks[last] = ((rank * 255) / (COUNT - 1)) as u8;
        let lx = last % SIDE;
        let ly = last / SIDE;
        let mut next = 0;
        let mut farthest = 0_u16;
        for y in 0..SIDE {
            for x in 0..SIDE {
                let idx = y * SIDE + x;
                if chosen[idx] {
                    continue;
                }
                let dx = x.abs_diff(lx).min(SIDE - x.abs_diff(lx));
                let dy = y.abs_diff(ly).min(SIDE - y.abs_diff(ly));
                let distance = (dx * dx + dy * dy) as u16;
                nearest[idx] = nearest[idx].min(distance);
                if nearest[idx] > farthest {
                    farthest = nearest[idx];
                    next = idx;
                }
            }
        }
        last = next;
    }
    ranks
}
