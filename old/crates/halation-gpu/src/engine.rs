//! Develop compute engine (wgpu 23). Replaces the Sprint-1 engine in `lib.rs`.
//!
//! Bind group 0: 0 `DevelopUniforms` · 1 `GpuDevelopMatrices` · 2 input `Rgba32Float` image ·
//! 3 output `Rgba8Unorm` storage · 4 `GpuExtraUniforms` · 5 spline segment storage buffer ·
//! 6 display LUT (3D `Rgba32Float`) · 7 mask (`R8Unorm`) · 8 mask sampler.
//!
//! Everything expensive is cached: the input texture per image id, the LUT per LUT id, the mask per
//! mask id, plus the readback buffer per frame size. A slider drag therefore costs four
//! `write_buffer` calls, one dispatch and one readback.
//!
//! iced 0.13 renders through its own pinned wgpu, so a `Device` cannot be shared with the UI yet; the
//! frame is read back and handed to iced as an RGBA image.

use halation_core::{GpuFrameInputs, ImageBufferF32, MAX_SEGMENTS};
use std::sync::{mpsc, Mutex};

const SEGMENT_BYTES: u64 = 32;
const CURVE_BUFFER_BYTES: u64 = 4 * MAX_SEGMENTS as u64 * SEGMENT_BYTES;

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
                    Self {
                        backend_name: format!("{:?}", info.backend),
                        adapter_name: info.name,
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
        }
    }
}

struct FrameResources {
    image_id: u64,
    width: u32,
    height: u32,
    input_texture: wgpu::Texture,
    input_view: wgpu::TextureView,
    output_texture: wgpu::Texture,
    output_view: wgpu::TextureView,
    readback: wgpu::Buffer,
    padded_bytes_per_row: u32,
}

struct CachedTexture {
    id: u64,
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

#[derive(Default)]
struct EngineState {
    frame: Option<FrameResources>,
    lut: Option<CachedTexture>,
    mask: Option<CachedTexture>,
}

pub struct GpuDevelopEngine {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    matrix_buffer: wgpu::Buffer,
    extra_buffer: wgpu::Buffer,
    curve_buffer: wgpu::Buffer,
    dummy_lut: CachedTexture,
    dummy_mask: CachedTexture,
    state: Mutex<EngineState>,
    info: GpuEngineInfo,
}

fn buffer_entry(binding: u32, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(
    binding: u32,
    dim: wgpu::TextureViewDimension,
    filterable: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: dim,
            multisampled: false,
        },
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

        let shader = device.create_shader_module(wgpu::include_wgsl!(
            "../../../assets/shaders/develop_pipeline.wgsl"
        ));
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Develop compute bindings"),
            entries: &[
                buffer_entry(0, wgpu::BufferBindingType::Uniform),
                buffer_entry(1, wgpu::BufferBindingType::Uniform),
                texture_entry(2, wgpu::TextureViewDimension::D2, false),
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
                buffer_entry(4, wgpu::BufferBindingType::Uniform),
                buffer_entry(5, wgpu::BufferBindingType::Storage { read_only: true }),
                texture_entry(6, wgpu::TextureViewDimension::D3, false),
                texture_entry(7, wgpu::TextureViewDimension::D2, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Mask sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let make_buffer = |label: &str, size: u64, usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let uniform_usage = wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST;
        let uniform_buffer = make_buffer(
            "Develop controls",
            std::mem::size_of::<halation_core::GpuDevelopUniforms>() as u64,
            uniform_usage,
        );
        let matrix_buffer = make_buffer(
            "Develop matrices",
            std::mem::size_of::<halation_core::GpuDevelopMatrices>() as u64,
            uniform_usage,
        );
        let extra_buffer = make_buffer(
            "Develop extras",
            std::mem::size_of::<halation_core::GpuExtraUniforms>() as u64,
            uniform_usage,
        );
        let curve_buffer = make_buffer(
            "Tone curve segments",
            CURVE_BUFFER_BYTES,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );

        let dummy =
            |label: &str, dim: wgpu::TextureDimension, size: u32, format: wgpu::TextureFormat| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size,
                        height: size,
                        depth_or_array_layers: if dim == wgpu::TextureDimension::D3 {
                            size
                        } else {
                            1
                        },
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: dim,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = texture.create_view(&Default::default());
                CachedTexture {
                    id: 0,
                    _texture: texture,
                    view,
                }
            };
        let dummy_lut = dummy(
            "Dummy LUT",
            wgpu::TextureDimension::D3,
            2,
            wgpu::TextureFormat::Rgba32Float,
        );
        let dummy_mask = dummy(
            "Dummy mask",
            wgpu::TextureDimension::D2,
            1,
            wgpu::TextureFormat::R8Unorm,
        );

        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
            sampler,
            uniform_buffer,
            matrix_buffer,
            extra_buffer,
            curve_buffer,
            dummy_lut,
            dummy_mask,
            state: Mutex::new(EngineState::default()),
            info: GpuEngineInfo {
                backend_name: format!("{:?}", adapter_info.backend),
                adapter_name: adapter_info.name,
            },
        })
    }

    pub fn info(&self) -> &GpuEngineInfo {
        &self.info
    }

    fn create_frame(&self, width: u32, height: u32) -> FrameResources {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let make = |label: &str, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
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
        let input_texture = make(
            "Develop input image",
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let output_texture = make(
            "Develop output image",
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        let padded_bytes_per_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        FrameResources {
            image_id: 0,
            width,
            height,
            input_view: input_texture.create_view(&Default::default()),
            output_view: output_texture.create_view(&Default::default()),
            input_texture,
            output_texture,
            readback: self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Develop output readback"),
                size: padded_bytes_per_row as u64 * height as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            padded_bytes_per_row,
        }
    }

    fn upload_image(&self, res: &FrameResources, input: &ImageBufferF32) {
        let rgba: Vec<f32> = input
            .pixels
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 1.0])
            .collect();
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &res.input_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&rgba),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(input.width * 16),
                rows_per_image: Some(input.height),
            },
            wgpu::Extent3d {
                width: input.width,
                height: input.height,
                depth_or_array_layers: 1,
            },
        );
    }

    fn upload_lut(&self, lut: &halation_core::LutBuffer) -> CachedTexture {
        let n = lut.size;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Display LUT"),
            size: wgpu::Extent3d {
                width: n,
                height: n,
                depth_or_array_layers: n,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&lut.rgba32f),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(n * 16),
                rows_per_image: Some(n),
            },
            wgpu::Extent3d {
                width: n,
                height: n,
                depth_or_array_layers: n,
            },
        );
        let view = texture.create_view(&Default::default());
        CachedTexture {
            id: lut.id,
            _texture: texture,
            view,
        }
    }

    fn upload_mask(&self, mask: &halation_core::MaskBuffer) -> CachedTexture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Local adjustment mask"),
            size: wgpu::Extent3d {
                width: mask.width,
                height: mask.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &mask.data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(mask.width),
                rows_per_image: Some(mask.height),
            },
            wgpu::Extent3d {
                width: mask.width,
                height: mask.height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&Default::default());
        CachedTexture {
            id: mask.id,
            _texture: texture,
            view,
        }
    }

    /// Develop `input` (linear ProPhoto) into display-encoded 8-bit RGBA (`width * height * 4` bytes).
    pub fn process(
        &self,
        input: &ImageBufferF32,
        frame: &GpuFrameInputs,
    ) -> Result<Vec<u8>, String> {
        if !input.is_valid() {
            return Err("Invalid image buffer dimensions".into());
        }
        let (width, height) = (input.width, input.height);
        let max_dim = self.device.limits().max_texture_dimension_2d;
        if width > max_dim || height > max_dim {
            return Err(format!(
                "Image exceeds the GPU texture limit of {max_dim}px"
            ));
        }

        let mut guard = self
            .state
            .lock()
            .map_err(|_| "GPU cache poisoned".to_owned())?;
        let state = &mut *guard;

        if state
            .frame
            .as_ref()
            .map_or(true, |f| f.width != width || f.height != height)
        {
            state.frame = Some(self.create_frame(width, height));
        }
        let res = state
            .frame
            .as_mut()
            .ok_or_else(|| "GPU resources unavailable".to_owned())?;
        if res.image_id != input.id {
            self.upload_image(res, input);
            res.image_id = input.id;
        }

        let lut = frame.lut.filter(|l| l.is_valid());
        if let Some(lut) = lut {
            if state.lut.as_ref().map_or(true, |c| c.id != lut.id) {
                state.lut = Some(self.upload_lut(lut));
            }
        }
        let mask = frame.mask.filter(|m| m.is_valid());
        if let Some(mask) = mask {
            if state.mask.as_ref().map_or(true, |c| c.id != mask.id) {
                state.mask = Some(self.upload_mask(mask));
            }
        }

        let mut extra = frame.extra;
        extra.flags[0] = u32::from(lut.is_some());
        if mask.is_none() {
            extra.local_b[2] = 0.0;
        }
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&frame.uniforms));
        self.queue
            .write_buffer(&self.matrix_buffer, 0, bytemuck::bytes_of(&frame.matrices));
        self.queue
            .write_buffer(&self.extra_buffer, 0, bytemuck::bytes_of(&extra));
        let segments = &frame.segments[..frame.segments.len().min(4 * MAX_SEGMENTS)];
        if !segments.is_empty() {
            self.queue
                .write_buffer(&self.curve_buffer, 0, bytemuck::cast_slice(segments));
        }

        let lut_view = if lut.is_some() {
            state.lut.as_ref().map(|c| &c.view)
        } else {
            None
        }
        .unwrap_or(&self.dummy_lut.view);
        let mask_view = if mask.is_some() {
            state.mask.as_ref().map(|c| &c.view)
        } else {
            None
        }
        .unwrap_or(&self.dummy_mask.view);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Develop compute bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.matrix_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&res.input_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&res.output_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.extra_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.curve_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(lut_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(mask_view),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
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
                texture: &res.output_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &res.readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(res.padded_bytes_per_row),
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

        let slice = res.readback.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("GPU readback failed: {e}"))?;

        let mapped = slice.get_mapped_range();
        let row_bytes = (width * 4) as usize;
        let padded = res.padded_bytes_per_row as usize;
        let mut output = Vec::with_capacity(row_bytes * height as usize);
        for y in 0..height as usize {
            output.extend_from_slice(&mapped[y * padded..y * padded + row_bytes]);
        }
        drop(mapped);
        res.readback.unmap();
        Ok(output)
    }
}
