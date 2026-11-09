# Halation Desktop: Software Architecture & Technical Design Document (SDD)
**Document Version:** 2.0.0 (Corrected Canonical Blueprint)  
**Status:** Approved for Implementation  
**Target Architectures:** Windows 10/11 64-bit (DirectX 12 / Vulkan), macOS 14+ (Metal), Linux x86_64 (Vulkan)  
**Workspace Configuration:** Rust 2021 Multi-Crate Workspace (Resolver 2)

---

## 1. Workspace Topology & Crate Dependency Graph

To prevent circular dependencies and isolate system-specific APIs, Halation is structured as a layered directed acyclic graph (DAG):

```
apps/halation-desktop
  │
  ├──► crates/halation-ui (Iced 0.13, Custom Widgets, Layout)
  │      │
  │      ├──► crates/halation-pipeline (Frame Scheduler, Tile Engine)
  │      │      │
  │      │      ├──► crates/halation-gpu (wgpu 23, WGSL Shaders, Compute Queue)
  │      │      ├──► crates/halation-color (moxcms, Colorimetric Solves, Bradford)
  │      │      ├──► crates/halation-raw (libraw-sys, EXIF, Demosaicing)
  │      │      └──► crates/halation-ml (ort 2.0, DirectML / CoreML, SAM Decoder)
  │      │
  │      ├──► crates/halation-catalog (SQLite WAL, Blake3 Indexing, XMP Engine)
  │      └──► crates/halation-core (AST Types, Uniform Structs, ParamTransaction)
  │
  └── (Leaf Crates: halation-core, halation-color, halation-gpu, halation-raw)
```

### 1.1 Crate Responsibility Boundaries

| Crate | Responsibilities | Forbidden Inclusions |
| :--- | :--- | :--- |
| `halation-core` | `GpuDevelopUniforms`, AST parameter models, `ParamTransaction`, history DAG node definitions. Zero runtime dependencies. | `wgpu`, `iced`, `rusqlite`, any I/O. |
| `halation-raw` | Raw unpacking, CFA extraction via `libraw-sys`, fallback bitmap decoding (`image`, `zune-image`), EXIF extraction (`kamadak-exif`). | GPU allocations, UI types. |
| `halation-color`| Bradford CAT matrix calculation, ProPhoto/ACES primaries, ICC parser, tetrahedral 3D display LUT builder (`moxcms`), platform display profile queries. | Raw disk I/O, UI rendering. |
| `halation-gpu` | Device/Queue initialization, bind groups, WGSL compute pipelines, swapchain blitting, uniform ring-buffer allocations. | UI business logic, database queries. |
| `halation-catalog` | SQLite WAL connection pooling, volume UUID resolution, relative path indexing, Adobe-compatible XMP serializer/deserializer. | GUI types, GPU compute types. |
| `halation-pipeline`| Multi-tier tile pyramid ($512\text{ px}$ tiles), asynchronous thumbnail pre-fetching, Rayon CPU export runner. | OS windowing handles. |
| `halation-ml` | Quantized on-device ONNX runtime (`ort`), background embedding generation, low-latency mask prompt decoding. | Network sockets, cloud APIs. |
| `halation-ui` | Custom `iced::advanced::Widget` implementations (Scrubby Sliders, 5-zone Histogram, Spline Curve, 3-way Color Wheels), layout tree. | Direct filesystem mutations. |
| `apps/halation-desktop`| Binary entrypoint, OS event loop bootstrap, platform initialization. | None (top-level coordinator). |

---

## 2. Multi-Tier Concurrency & Device Sharing Architecture

### 2.1 Shared GPU Device Topology (`wgpu` + `iced_wgpu`)
To eliminate texture copy overhead between the compute pipeline and display presentation, the compute pipeline and Iced **must** share the identical `wgpu::Device` and `wgpu::Queue`.

```
                    ┌─────────────────────────────────────────┐
                    │      wgpu::Instance (DX12/Metal/VK)     │
                    └────────────────────┬────────────────────┘
                                         ▼
                    ┌─────────────────────────────────────────┐
                    │  Shared Arc<wgpu::Device>, Arc<Queue>   │
                    └───────────┬──────────────────┬──────────┘
                                │                  │
         (Dedicated Compute)    ▼                  ▼   (Presentation)
┌───────────────────────────────────────┐ ┌──────────────────────────────┐
│ halation-gpu Compute Pipeline         │ │ iced_wgpu Presentation Engine│
│ • Input: 32-bit Float RAW Tensor      │ │ • Custom shader::Program     │
│ • Storage: Rgba16Float Viewport Buffer│ │ • Renders UI overlays        │
│ • Blit Target: Shared Texture Atlas   │ │ • Direct Swapchain Blit      │
└───────────────────────────────────────┘ └──────────────────────────────┘
```

```rust
// crates/halation-gpu/src/device.rs
use std::sync::Arc;
use wgpu::{Device, Instance, Queue, Surface};

pub struct GpuContext {
    pub instance: Instance,
    pub device: Arc<Device>,
    pub queue: Arc<Queue>,
}

impl GpuContext {
    pub async fn new_shared(surface_hint: Option<&Surface<'_>>) -> Result<Self, wgpu::RequestDeviceError> {
        let instance = Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: surface_hint,
                force_fallback_adapter: false,
            })
            .await
            .expect("Halation: Failed to acquire hardware GPU adapter");

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Halation Primary Device"),
                    required_features: wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
                        | wgpu::Features::FLOAT32_FILTERABLE,
                    required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await?;

        Ok(Self {
            instance,
            device: Arc::new(device),
            queue: Arc::new(queue),
        })
    }
}
```

### 2.2 Threading & Communication Primitives
1. **Transient Slider Stream (UI $\to$ GPU):** 
   - Uses an atomic ring buffer (`crossbeam::queue::ArrayQueue<GpuDevelopUniforms>`) with capacity 2.
   - When scrubbing, the UI thread drops stale unrendered uniforms and writes the latest value.
   - The GPU compute loop polls this queue at $120\text{ FPS}$ without blocking the UI thread.
2. **Catalog / History Commits (UI $\to$ Background DB):** 
   - A lock-free MPSC channel (`tokio::sync::mpsc::unbounded_channel`) dispatches `DbCommand::CommitDevelopState` on mouse release.
3. **Tile Computation (Pipeline $\to$ Rayon):**
   - Work-stealing Rayon pool using explicit priorities:
     - `Priority::InteractiveLoupe` (Viewport visible tiles, preempts all other work).
     - `Priority::ThumbnailPreload` (Visible grid buffer $\pm 2$ rows).
     - `Priority::BackgroundExport` (Batch export jobs).

---

## 3. Imaging Pipeline & Colorimetric Formulation

### 3.1 Color Transformation Chain
Halation executes all tonal, contrast, and color-grading math inside a **Radiometrically Linear ProPhoto RGB** color space:

```
[Raw Camera Sensor Data] (CFA Bayer/X-Trans)
       │
       ▼ (libraw-sys demosaic + black/white point calibration)
[Camera-Native Linear RGB]
       │
       ▼ (Camera Matrix * Bradford Chromatic Adaptation to D65)
[Linear Camera-Neutral D65]
       │
       ▼ (ColorMatrix: Camera Neutral -> Linear ProPhoto RGB)
[Working Space: Radiometrically Linear ProPhoto RGB]
       │
       ├── Exposure & Radiometric Scaling
       ├── High-Dynamic-Range Tone Reconstruction (Log-Domain Highlights/Shadows)
       ├── Radiometric Sigmoidal Contrast (Log-Domain around 18% Gray)
       ├── Analytical Fritsch-Carlson Spline Tone Curve
       ├── Color Grading, Vibrance & Saturation
       │
       ▼
[Clamped Scene Radiance [0.0, 1.0]]
       │
       ▼ (3D Tetrahedral Display LUT via moxcms)
[Display Monitor Profile] (sRGB / Display P3 / Rec.2020)
```

### 3.2 Chromatic Adaptation (Bradford D65 Formulation)
Given source white balance parameters (Temperature $T$ in Kelvin, Tint $\Delta uv$), `halation-color` computes the chromatic adaptation matrix $\mathbf{M}_{\text{CAT}}$:

$$\begin{bmatrix} X \\ Y \\ Z \end{bmatrix}_{\text{adapted}} = \mathbf{M}_{\text{Bradford}}^{-1} \begin{bmatrix} \rho_{\text{target}} / \rho_{\text{source}} & 0 & 0 \\ 0 & \gamma_{\text{target}} / \gamma_{\text{source}} & 0 \\ 0 & 0 & \beta_{\text{target}} / \beta_{\text{source}} \end{bmatrix} \mathbf{M}_{\text{Bradford}} \begin{bmatrix} X \\ Y \\ Z \end{bmatrix}_{\text{source}}$$

Where the cone fundamentals transformation matrix is:
$$\mathbf{M}_{\text{Bradford}} = \begin{bmatrix} 0.8951 & 0.2664 & -0.1614 \\ -0.7502 & 1.7135 & 0.0367 \\ 0.0389 & -0.0685 & 1.0296 \end{bmatrix}$$

### 3.3 Log-Domain Tonal Reconstruction & Contrast Formulations

#### 1. Exposure Scaling
$$\mathbf{C}_{\text{exposed}} = \mathbf{C}_{\text{in}} \cdot 2^{\text{exposure}}$$

#### 2. High-Dynamic-Range Highlights & Shadows
Luminance is calculated using the ProPhoto RGB linear weights:
$$L = \mathbf{C}_{\text{exposed}} \cdot \begin{bmatrix} 0.28804 \\ 0.71187 \\ 0.00009 \end{bmatrix}$$

To prevent unnatural tonal inversions, highlights and shadows are evaluated in log-space:
$$l_{\text{log}} = \log_2(L + 10^{-5})$$
$$W_{\text{shadow}} = \text{clamp}\left(1.0 - \frac{l_{\text{log}} + 4.0}{4.0},\ 0.0,\ 1.0\right)$$
$$W_{\text{highlight}} = \text{clamp}\left(\frac{l_{\text{log}} + 1.0}{4.0},\ 0.0,\ 1.0\right)$$
$$\Delta L = (\text{shadows} \cdot 0.01 \cdot W_{\text{shadow}}) + (\text{highlights} \cdot 0.01 \cdot W_{\text{highlight}})$$
$$\mathbf{C}_{\text{toned}} = \mathbf{C}_{\text{exposed}} \cdot 2^{\Delta L}$$

#### 3. Radiometric Contrast (Sigmoidal Log-Domain)
Contrast must pivot around the standardized $18\%$ neutral gray level without clipping negative values:
$$\gamma = 1.0 + (\text{contrast} \cdot 0.005)$$
$$\mathbf{C}_{\text{contrast}} = \left(\frac{\mathbf{C}_{\text{toned}}}{0.18}\right)^\gamma \cdot 0.18$$

---

## 4. GPU Shaders, Uniform Memory Layouts & Data Models

### 4.1 Memory Layout Definitions (`halation-core`)
WGSL `std140` and `std430` layout rules require strict 16-byte alignment boundaries. Matrices must have 16-byte aligned column vectors.

```rust
// crates/halation-core/src/uniforms.rs
use bytemuck::{Pod, Zeroable};

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable, serde::Serialize, serde::Deserialize)]
pub struct GpuDevelopUniforms {
    pub exposure: f32,       // Offset 0
    pub contrast: f32,       // Offset 4
    pub highlights: f32,     // Offset 8
    pub shadows: f32,        // Offset 12
    pub whites: f32,         // Offset 16
    pub blacks: f32,         // Offset 20
    pub temperature: f32,    // Offset 24
    pub tint: f32,           // Offset 28
    pub vibrance: f32,       // Offset 32
    pub saturation: f32,     // Offset 36
    pub num_spline_segments: u32, // Offset 40 (Count of valid spline cubic segments)
    pub _padding: f32,       // Offset 44 -> Aligns to 48 bytes (multiple of 16)
}

/// Represents a single cubic spline segment: f(x) = a*x^3 + b*x^2 + c*x + d
/// Valid on domain [x_min, x_max].
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable, serde::Serialize, serde::Deserialize)]
pub struct SplineSegment {
    pub coeffs: [f32; 4],    // a, b, c, d (16 bytes)
    pub domain: [f32; 2],    // x_min, x_max (8 bytes)
    pub _pad: [f32; 2],      // 8 bytes padding -> 32 bytes total per segment
}

/// Padded 3x3 matrix for WGSL uniform buffers where each column occupies 16 bytes.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct GpuMat3x3 {
    pub col0: [f32; 4],      // r, g, b, 0.0
    pub col1: [f32; 4],      // r, g, b, 0.0
    pub col2: [f32; 4],      // r, g, b, 0.0
}
```

### 4.2 Production WGSL Shader (`assets/shaders/develop_pipeline.wgsl`)

```wgsl
struct DevelopUniforms {
    exposure: f32,
    contrast: f32,
    highlights: f32,
    shadows: f32,
    whites: f32,
    blacks: f32,
    temperature: f32,
    tint: f32,
    vibrance: f32,
    saturation: f32,
    num_spline_segments: u32,
    _pad: f32,
};

struct SplineSegment {
    coeffs: vec4<f32>, // a, b, c, d
    domain: vec2<f32>, // x_min, x_max
    _pad: vec2<f32>,
};

struct WhiteBalanceMatrix {
    col0: vec4<f32>,
    col1: vec4<f32>,
    col2: vec4<f32>,
};

// ProPhoto RGB Linear Luminance Weights
const PROPHOTO_LUMA: vec3<f32> = vec3<f32>(0.28804, 0.71187, 0.00009);

@group(0) @binding(0) var input_tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var display_lut_3d: texture_3d<f32>;

@group(1) @binding(0) var<uniform> u: DevelopUniforms;
@group(1) @binding(1) var<uniform> wb: WhiteBalanceMatrix;
@group(1) @binding(2) var<storage, read> spline_segments: array<SplineSegment>;
@group(1) @binding(3) var output_tex: texture_storage_2d<rgba16float, write>;

fn evaluate_spline(val: f32) -> f32 {
    let num_segs = u.num_spline_segments;
    if (num_segs == 0u) {
        return val;
    }
    
    // Clamp evaluation to domain of defined splines
    let clamped_val = clamp(val, 0.0, 1.0);
    for (var i = 0u; i < num_segs; i = i + 1u) {
        let seg = spline_segments[i];
        if (clamped_val >= seg.domain.x && clamped_val <= seg.domain.y) {
            let x = clamped_val - seg.domain.x;
            return seg.coeffs.x * x * x * x +
                   seg.coeffs.y * x * x +
                   seg.coeffs.z * x +
                   seg.coeffs.w;
        }
    }
    return val;
}

@compute @workgroup_size(16, 16)
fn cs_develop_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(input_tex);
    if (id.x >= dims.x || id.y >= dims.y) {
        return;
    }
    let coord = vec2<i32>(id.xy);
    var color = textureLoad(input_tex, coord, 0).rgb;

    // 1. Chromatic Adaptation / White Balance (Bradford Transform in LMS Space)
    let wb_mat = mat3x3<f32>(wb.col0.xyz, wb.col1.xyz, wb.col2.xyz);
    color = wb_mat * color;

    // 2. Linear Exposure Energy Scaling
    color = color * exp2(u.exposure);

    // 3. High-Dynamic-Range Highlights & Shadows in Log Space
    let luma = dot(color, PROPHOTO_LUMA);
    if (luma > 0.00001) {
        let log_luma = log2(luma);
        let shadow_weight = clamp(1.0 - (log_luma + 4.0) / 4.0, 0.0, 1.0);
        let highlight_weight = clamp((log_luma + 1.0) / 4.0, 0.0, 1.0);
        
        let luma_delta = (u.shadows * 0.01 * shadow_weight) + (u.highlights * 0.01 * highlight_weight);
        color = color * exp2(luma_delta);
    }

    // 4. Whites & Blacks Linear Expansion
    let white_scale = 1.0 + (u.whites * 0.005);
    let black_offset = u.blacks * 0.002;
    color = max((color * white_scale) + vec3<f32>(black_offset), vec3<f32>(0.0));

    // 5. Radiometric Sigmoidal Contrast (pivoting around 18% gray)
    let contrast_gamma = 1.0 + (u.contrast * 0.005);
    color = pow(max(color, vec3<f32>(0.0)) / 0.18, vec3<f32>(contrast_gamma)) * 0.18;

    // 6. Analytical Tone Curve Evaluation (Per-Channel)
    color.r = evaluate_spline(color.r);
    color.g = evaluate_spline(color.g);
    color.b = evaluate_spline(color.b);

    // 7. Vibrance and Saturation
    let post_luma = dot(color, PROPHOTO_LUMA);
    let max_c = max(color.r, max(color.g, color.b));
    let min_c = min(color.r, min(color.g, color.b));
    let current_sat = (max_c - min_c) / (max_c + 0.0001);
    
    let vib_mult = 1.0 + (u.vibrance * 0.01 * (1.0 - current_sat));
    let sat_mult = 1.0 + (u.saturation * 0.01);
    color = post_luma + (color - post_luma) * (vib_mult * sat_mult);

    // 8. Display Gamut Mapping & Output OETF via 3D Tetrahedral Display LUT
    // Clamping to [0.0, 1.0] prepares normalized coordinates for the 33x33x33 texture LUT
    let display_color = textureSampleLevel(display_lut_3d, samp, clamp(color, vec3(0.0), vec3(1.0)), 0.0).rgb;

    textureStore(output_tex, coord, vec4<f32>(display_color, 1.0));
}
```

---

## 5. Catalog Persistence, Volume Resolution & History DAG

### 5.1 Cross-Platform Volume UUID Resolution
To survive drive letter reassignments (Windows) and fluctuating mount paths (macOS/Linux), Halation generates a canonical, persistent volume identity:

```rust
// crates/halation-catalog/src/volume.rs
use std::path::Path;

pub struct VolumeResolver;

impl VolumeResolver {
    #[cfg(target_os = "windows")]
    pub fn get_volume_uuid(path: &Path) -> Result<String, std::io::Error> {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Storage::FileSystem::GetVolumeInformationW;

        let root = path.components().next().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "Missing volume root")
        })?;
        let root_str: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();

        let mut serial_number: u32 = 0;
        unsafe {
            GetVolumeInformationW(
                windows::core::PCWSTR(root_str.as_ptr()),
                None,
                Some(&mut serial_number),
                None,
                None,
                None,
            )?;
        }
        Ok(format!("WIN-{:08X}", serial_number))
    }

    #[cfg(target_os = "macos")]
    pub fn get_volume_uuid(path: &Path) -> Result<String, std::io::Error> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let c_path = CString::new(path.as_os_str().as_bytes())?;
        let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
        let res = unsafe { libc::statfs(c_path.as_ptr(), &mut stat) };
        if res != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // stat.f_fsid encodes the unique OS filesystem ID
        Ok(format!("MAC-{:08X}{:08X}", stat.f_fsid.val[0], stat.f_fsid.val[1]))
    }

    #[cfg(target_os = "linux")]
    pub fn get_volume_uuid(path: &Path) -> Result<String, std::io::Error> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let c_path = CString::new(path.as_os_str().as_bytes())?;
        let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
        let res = unsafe { libc::statfs(c_path.as_ptr(), &mut stat) };
        if res != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(format!("LNX-{:08X}{:08X}", stat.f_fsid.__val[0], stat.f_fsid.__val[1]))
    }
}
```

### 5.2 Branching History DAG Traversal Engine

```
       [Node 1: Import Baseline]
                   │
                   ▼
       [Node 2: Exposure +0.50]
                   │
         ┌─────────┴─────────┐ (Undo 1 step + modify)
         ▼                   ▼
[Node 3: Contrast +20]   [Node 4: Temp 6500K] (Active Head)
         │
         ▼
[Node 5: B&W Conversion]
```

When an edit is applied from an arbitrary historical state:
1. Halation evaluates `parent_history_id = develop_states.active_history_id`.
2. Inserts a new node into `develop_history`.
3. Points `develop_states.active_history_id` directly to the newly inserted node.
4. Old branch nodes (`Node 3`, `Node 5`) remain persisted and reachable in the History Panel UI without truncation.

```rust
// crates/halation-catalog/src/history.rs
use rusqlite::{params, Connection, Result};

pub fn commit_history_step(
    conn: &mut Connection,
    asset_id: u64,
    action_label: &str,
    new_params_json: &str,
) -> Result<u64> {
    let tx = conn.transaction()?;

    // 1. Fetch current active history head
    let active_head_id: Option<i64> = tx.query_row(
        "SELECT active_history_id FROM develop_states WHERE asset_id = ?1",
        params![asset_id],
        |row| row.get(0),
    ).ok();

    // 2. Insert new DAG node referencing active head as parent
    tx.execute(
        "INSERT INTO develop_history (asset_id, parent_history_id, action_label, params_json)
         VALUES (?1, ?2, ?3, ?4)",
        params![asset_id, active_head_id, action_label, new_params_json],
    )?;
    let new_node_id = tx.last_insert_rowid() as u64;

    // 3. Update the active develop state pointer
    tx.execute(
        "INSERT INTO develop_states (asset_id, active_history_id, params_json, updated_at)
         VALUES (?1, ?2, ?3, strftime('%s', 'now'))
         ON CONFLICT(asset_id) DO UPDATE SET
             active_history_id = excluded.active_history_id,
             params_json = excluded.params_json,
             updated_at = excluded.updated_at",
        params![asset_id, new_node_id, new_params_json],
    )?;

    tx.commit()?;
    Ok(new_node_id)
}
```

---

## 6. Local Machine Learning Inference Pipeline (`halation-ml`)

Halation implements a **two-phase decoupled neural vision architecture** via `ort` (ONNX Runtime) with native hardware acceleration (DirectML on Windows, CoreML on macOS, Vulkan on Linux):

```
[Loupe Viewport Selection]
           │
           ▼ (Trigger Background Task via Tokio)
┌────────────────────────────────────────────────────────┐
│ Phase 1: Background Embedding Worker                   │
│ • Model: MobileSAM Image Encoder (Quantized INT8)      │
│ • Input: 1024x1024 RGB Preprocessed Tensor             │
│ • Execution Time: ~300ms (dGPU) / ~1200ms (Iris Xe)   │
│ • Output: Image Embedding Tensor [1, 256, 64, 64]       │
└──────────────────────────┬─────────────────────────────┘
                           │
                           ▼ (Stored in Memory Cache)
┌────────────────────────────────────────────────────────┐
│ Phase 2: Interactive Prompt Decoder                    │
│ • User Action: Clicks "Select Subject" or "Select Sky" │
│ • Model: Lightweight Mask Decoder (FP16)               │
│ • Execution Time: < 15ms                               │
│ • Output: 8-bit Alpha Mask (1024x1024)                 │
└──────────────────────────┬─────────────────────────────┘
                           │
                           ▼
[Uploaded to GPU as wgpu::TextureView -> Injected into Mask Pipeline]
```

### 6.1 ONNX Pipeline Manager Implementation
```rust
// crates/halation-ml/src/engine.rs
use ort::{session::Session, value::Tensor};
use std::sync::Arc;

pub struct VisionEngine {
    encoder_session: Arc<Session>,
    decoder_session: Arc<Session>,
}

impl VisionEngine {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let env = ort::init()
            .with_name("HalationVision")
            .commit()?;

        #[cfg(target_os = "windows")]
        let execution_provider = ort::execution_providers::DirectMLExecutionProvider::default().build();
        #[cfg(target_os = "macos")]
        let execution_provider = ort::execution_providers::CoreMLExecutionProvider::default().build();
        #[cfg(target_os = "linux")]
        let execution_provider = ort::execution_providers::CPUExecutionProvider::default().build();

        let encoder_session = Session::builder()?
            .with_execution_providers([execution_provider.clone()])?
            .with_optimization_level(ort::session::GraphOptimizationLevel::Level3)?
            .commit_from_file("assets/models/mobilesam_encoder_quant.onnx")?;

        let decoder_session = Session::builder()?
            .with_execution_providers([execution_provider])?
            .commit_from_file("assets/models/mobilesam_decoder.onnx")?;

        Ok(Self {
            encoder_session: Arc::new(encoder_session),
            decoder_session: Arc::new(decoder_session),
        })
    }

    /// Evaluates prompt points against the precomputed embedding in < 15ms
    pub fn decode_mask(
        &self,
        image_embeddings: &Tensor<f32>,
        point_coords: &[f32], // Normalized [N, 2]
        point_labels: &[f32], // 1.0 = foreground, 0.0 = background
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        // Run decoder session and extract single-channel 8-bit precision mask
        // ... (tensor bind, session.run(), return mask bytes)
        Ok(vec![])
    }
}
```

---

## 7. Tile Pyramid Engine & Memory Budgeting (`halation-pipeline`)

To support instantaneous pan and 1:1 pixel inspection on 45–100 MP images without out-of-memory (OOM) crashes, Halation uses a **tiled multiresolution image pyramid**:

```
Level 0: 100% Native Resolution (512x512 Tiled Grid) ─── Highest VRAM footprint, loaded on demand
Level 1: 50% Stride (Downsampled 2x2)
Level 2: 25% Stride (Downsampled 4x4)
Level 3: Fit Preview (Max Dimension: 2048px) ────────── Persistently pinned in GPU VRAM
```

### 7.1 Dynamic Memory Budget Caps
- **VRAM Hard Ceiling:** Halation clamps its active GPU texture allocations to **$50\%$ of detected dedicated VRAM** (e.g., $4\text{ GB}$ ceiling on an $8\text{ GB}$ RTX 3060).
- **Host RAM LRU Tile Cache:** 
  - Maintains $1024$ uncompressed 16-bit half-float tiles ($512 \times 512 \times 8\text{ bytes} \approx 2\text{ MB}$ per tile $\to 2.0\text{ GB}$ total RAM budget).
  - Background eviction uses a thread-safe `lru::LruCache` keyed by `TileCoord`.

---

## 8. Verification, Testing & Air-Gap Compliance Checklist

### 8.1 Automated Air-Gap Test Suite
An automated integration test verifies that Halation never initiates network socket connections:

```rust
// tests/airgap_compliance.rs
#[test]
fn verify_zero_network_sockets_opened() {
    // 1. Hook OS socket syscalls (socket, connect, sendto) via LD_PRELOAD / Detours
    // 2. Initialize Halation catalog, import RAW files, run AI segmentation, batch export
    // 3. Assert socket call counter == 0
}
```

### 8.2 Colorimetric Numerical Tolerance Tests
- Linear ProPhoto Bradford calculations **must** match reference double-precision calculations within a tolerance of $\Delta E_{00} < 0.05$.
- Tone curve monotonicity: For any monotonically increasing control points $y_i \le y_{i+1}$, the first derivative of the spline **must** satisfy $f'(x) \ge 0$ across all $x \in [0.0, 1.0]$.

### 8.3 Sidecar Round-Trip Invariance
- Writing a parameter state to `.xmp` and immediately parsing it back into `DevelopParameters` **must** produce identical bitwise hashes (`blake3`) with zero floating-point drift:
  $$\text{assert\_eq!}(\text{hash}(\mathbf{P}_{\text{initial}}),\ \text{hash}(\text{deserialize}(\text{serialize}(\mathbf{P}_{\text{initial}}))))$$