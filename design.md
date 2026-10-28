# Halation: Software Architecture & Technical Design Document (SDD)
**Document Version:** 1.0.0  
**Status:** Canonical Implementation Blueprint  
**Target Architecture:** Windows 64-bit (DirectX 12 / Vulkan), macOS (Metal), Linux (Vulkan)  
**Rust Edition:** 2021 (Workspace Resolver 2)

---

## 1. Technical Stack & Library Rationale

| Subsystem | Primary Crate | Features / Flags | Architectural Rationale & Tradeoffs |
| :--- | :--- | :--- | :--- |
| **UI Framework** | `iced` | `wgpu`, `canvas`, `image`, `svg` | Pure Rust, Elm architecture (unidirectional state flow), natively backed by `wgpu`. Unlike GPUI, it allows direct `wgpu` render pass integration. Unlike `egui`, it avoids CPU mesh re-tessellation on 100k catalog grids. |
| **GPU Hardware API** | `wgpu` | `default` | Portable abstraction over DX12, Vulkan, and Metal. Shares the same device/queue across both UI presentation and compute image processing. |
| **Linear Algebra** | `glam` | `bytemuck` | SIMD-accelerated math ($3 \times 3$, $4 \times 4$). The `bytemuck` feature allows zero-copy transmutation of matrices directly into raw bytes for GPU uniform buffers. |
| **Scientific Math** | `faer` | `default` | High-performance linear algebra for polynomial regression (lens distortion calibration) and chromatic adaptation solves. Outperforms OpenBLAS without C linkage. |
| **Color Management** | `moxcms` | `lut` | Pure-Rust LittleCMS alternative. Parses ICC profiles, builds 3D tetrahedral LUTs, and runs color transforms without linking external C dependencies. |
| **CPU Parallelism** | `rayon` | `default` | Work-stealing thread pool for tile-based CPU demosaicing, draft rasterization, and batch exports. |
| **Async I/O Runtime** | `tokio` | `rt-multi-thread`, `macros`, `fs`, `io-util` | Asynchronous file ingestion, disk thumbnail reads, and database persistence decoupled from the UI thread. |
| **Embedded Database** | `rusqlite` | `bundled` | Embedded ACID storage for catalog assets, tags, ratings, and develop histories. Compiles SQLite from source to prevent Windows DLL mismatch. |
| **Fast Hashing** | `blake3` | `default` | High-throughput hashing (> 3 GB/s) for RAW file deduplication, tile cache keys, and develop state change detection. |
| **Image Codecs** | `image`, `zune-image` | `png`, `jpeg`, `tiff`, `webp` | Fast image encoding/decoding. Pure-Rust codecs avoid C security vulnerabilities during thumbnail parsing. |
| **Metadata Parser** | `kamadak-exif` | `default` | Zero-allocation, high-speed EXIF, TIFF, and MakerNote extraction from RAW headers. |
| **File Dialogs** | `rfd` | `default` | Native Windows Win32/COM Open and Save file dialogs. |
| **Platform Hooks** | `windows` | `Win32_Graphics_Gdi`, `Win32_Storage_FileSystem` | Windows-specific queries for active monitor ICC profiles (`GetICMProfileW`) and volume serial numbers. |

---

## 2. Multi-Tier Concurrency & Threading Architecture

Halation uses four distinct execution tiers to guarantee the main event loop never drops below 120 FPS:

```
┌────────────────────────────────────────────────────────────────────────┐
│ 1. MAIN UI THREAD (Iced Event Loop)                                    │
│    • Input events (Mouse, Keyboard, Scrubby Slider deltas)             │
│    • Layout calculations (Taffy flexbox)                               │
│    • Immediate parameter updates (< 1ms)                               │
└───────────────────┬─────────────────────────────────┬──────────────────┘
                    │ (Non-blocking Channel)          │ (Shared Arc)
                    ▼                                 ▼
┌───────────────────────────────────────┐ ┌──────────────────────────────┐
│ 2. GPU COMPUTE QUEUE (Dedicated)      │ │ 3. CPU WORKER POOL (Rayon)   │
│    • 120 FPS Viewport Draft Compute   │ │    • RAW Demosaicing (AMAZE) │
│    • 32-bit Float Shaders (WGSL)      │ │    • High-res 1:1 Tile Gen   │
│    • 256-bin Histogram Generation     │ │    • CPU Draft Fallback      │
│    • Display Gamut 3D LUT Blit        │ │    • Multi-core Export Jobs  │
└───────────────────────────────────────┘ └──────────────────────────────┘
                    ▲                                 ▲
                    │                                 │
┌───────────────────┴─────────────────────────────────┴──────────────────┐
│ 4. ASYNC BACKGROUND RUNTIME (Tokio)                                    │
│    • Asynchronous RAW file ingestion from disk / SD card               │
│    • SQLite database commits and vacuuming                             │
│    • Disk thumbnail cache read/write (.halation-cache/)                │
└────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Data Structures & Memory Layouts

### 3.1 Develop Parameters (AST Model)
Located in `crates/halation-core`. Must be strictly `Pod` (Plain Old Data) compatible for GPU uniform serialization:

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable, serde::Serialize, serde::Deserialize)]
pub struct GpuDevelopUniforms {
    pub exposure: f32,       // -5.0 to +5.0 EV
    pub contrast: f32,       // -100.0 to +100.0
    pub highlights: f32,     // -100.0 to +100.0
    pub shadows: f32,        // -100.0 to +100.0
    pub whites: f32,         // -100.0 to +100.0
    pub blacks: f32,         // -100.0 to +100.0
    pub temperature: f32,    // 2000.0 to 50000.0 K
    pub tint: f32,           // -150.0 to +150.0
    pub vibrance: f32,       // -100.0 to +100.0
    pub saturation: f32,     // -100.0 to +100.0
    pub _padding: [f32; 2],  // Aligns to 16-byte boundary for WGSL std140
}
```

### 3.2 Spline Tone Curve Representation
```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SplineCurve {
    /// Ordered control points in normalized Cartesian coordinates [0.0, 1.0]
    pub points: Vec<glam::Vec2>,
}

impl SplineCurve {
    /// Evaluates the curve at luminance input x using Monotone Cubic Spline (Fritsch-Carlson)
    pub fn evaluate(&self, x: f32) -> f32;
    /// Generates a 256-entry 1D lookup table for GPU upload
    pub fn generate_lut_256(&self) -> [f32; 256];
}
```

### 3.3 Tile Pyramid & Cache Key
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileCoord {
    pub asset_id: u64,
    pub level: u8,   // 0 = 100% full-res, 1 = 50%, 2 = 25%, 3 = 12.5%
    pub x: u32,      // Tile coordinate x (512px stride)
    pub y: u32,      // Tile coordinate y (512px stride)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub coord: TileCoord,
    pub params_hash: [u8; 32], // blake3 hash of active DevelopParameters
}
```

---

## 4. Internal API Trait Definitions

To ensure strict module decoupling, crates interact exclusively through traits:

### 4.1 Image Pipeline Engine (`halation-pipeline`)
```rust
pub trait ImagePipelineEngine: Send + Sync {
    /// Dispatches an immediate preview render for the viewport at current display resolution
    fn render_viewport_preview(
        &self,
        asset: &RawAsset,
        params: &DevelopParameters,
        target_size: (u32, u32),
    ) -> Result<ViewportFrame, PipelineError>;

    /// Requests a specific high-resolution 1:1 tile from the compute queue
    fn request_tile(
        &self,
        coord: TileCoord,
        params: &DevelopParameters,
    ) -> Result<wgpu::TextureView, PipelineError>;

    /// Calculates a 256-bin RGB + Luminance histogram
    fn compute_histogram(
        &self,
        frame: &ViewportFrame,
    ) -> HistogramData;
}
```

### 4.2 Color Engine (`halation-color`)
```rust
pub trait ColorManagementEngine: Send + Sync {
    /// Queries the OS for the active display's ICC profile (Win32 GDI / AppKit)
    fn get_active_display_profile(&self) -> Result<DisplayProfile, ColorError>;

    /// Builds a 3D tetrahedral RGB LUT converting ProPhoto RGB linear into target display space
    fn build_display_lut(
        &self,
        source_profile: &DisplayProfile,
        target_profile: &DisplayProfile,
        lut_size: u32, // Standard: 33x33x33 or 65x65x65
    ) -> Result<Vec<f32>, ColorError>;

    /// Generates chromatic adaptation matrix (Bradford) for White Balance shift
    fn calculate_white_balance_matrix(
        &self,
        temp_kelvin: f32,
        tint: f32,
    ) -> glam::Mat3;
}
```

### 4.3 Catalog Repository (`halation-catalog`)
```rust
pub trait CatalogRepository: Send + Sync {
    fn import_asset(&self, path: &std::path::Path) -> Result<u64, CatalogError>;
    fn query_assets(&self, filter: &CatalogFilter) -> Result<Vec<CatalogItem>, CatalogError>;
    fn update_rating(&self, asset_id: u64, rating: u8) -> Result<(), CatalogError>;
    fn update_flag(&self, asset_id: u64, flag: FlagState) -> Result<(), CatalogError>;
    fn commit_develop_state(&self, asset_id: u64, params: &DevelopParameters) -> Result<(), CatalogError>;
    fn get_undo_history(&self, asset_id: u64) -> Result<Vec<HistoryNode>, CatalogError>;
}
```

---

## 5. GPU Shaders & Pipeline Layouts (`halation-gpu`)

### 5.1 Pipeline Bind Groups
The core Develop compute pipeline uses two primary bind groups to separate static assets from high-frequency parameters:

* **Bind Group 0 (Static Scene):**
  * Binding 0: `input_texture` (`texture_2d<f32>`, linear sensor data or source tile).
  * Binding 1: `linear_sampler` (`sampler`).
  * Binding 2: `display_lut_3d` (`texture_3d<f32>`, 33x33x33 tetrahedral monitor color transform).
* **Bind Group 1 (Dynamic Parameters - Updated per slider drag):**
  * Binding 0: `uniforms` (`buffer<uniform>, GpuDevelopUniforms`).
  * Binding 1: `tone_curve_lut` (`texture_1d<f32>`, 256-entry R32Float).
  * Binding 2: `output_surface` (`texture_storage_2d<rgba16float, write>`).

### 5.2 Compute Shader Specification (`assets/shaders/develop_pipeline.wgsl`)
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
    _padding: vec2<f32>,
};

@group(0) @binding(0) var input_tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var display_lut_3d: texture_3d<f32>;

@group(1) @binding(0) var<uniform> u: DevelopUniforms;
@group(1) @binding(1) var curve_lut: texture_1d<f32>;
@group(1) @binding(2) var output_tex: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(16, 16)
fn cs_develop_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(input_tex);
    if (id.x >= dims.x || id.y >= dims.y) {
        return;
    }
    let coord = vec2<i32>(id.xy);
    var color = textureLoad(input_tex, coord, 0).rgb;

    // 1. Exposure Compensation (Stops)
    color = color * pow(2.0, u.exposure);

    // 2. Highlights & Shadows Decomposition
    let luma = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    let shadow_mask = pow(1.0 - clamp(luma / 0.35, 0.0, 1.0), 2.0);
    let highlight_mask = pow(clamp((luma - 0.65) / 0.35, 0.0, 1.0), 2.0);
    color += color * ((u.shadows * 0.005 * shadow_mask) + (u.highlights * 0.005 * highlight_mask));

    // 3. Contrast (S-Curve about 18% neutral gray)
    let c_factor = (259.0 * (u.contrast + 255.0)) / (255.0 * (259.0 - u.contrast));
    color = (color - 0.18) * c_factor + 0.18;

    // 4. Tone Curve Sampling (1D Texture LUT)
    color.r = textureSampleLevel(curve_lut, samp, color.r, 0.0).r;
    color.g = textureSampleLevel(curve_lut, samp, color.g, 0.0).r;
    color.b = textureSampleLevel(curve_lut, samp, color.b, 0.0).r;

    // 5. Display Gamut Mapping (3D Tetrahedral LUT)
    let display_color = textureSampleLevel(display_lut_3d, samp, clamp(color, vec3(0.0), vec3(1.0)), 0.0).rgb;

    textureStore(output_tex, coord, vec4<f32>(display_color, 1.0));
}
```

---

## 6. SQLite Catalog Schema Specification

The catalog uses write-ahead logging (WAL) mode for concurrent read/write transactions:

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;

-- 1. Ingested Photo Assets
CREATE TABLE IF NOT EXISTS assets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL UNIQUE,
    volume_serial TEXT NOT NULL,
    file_hash BLOB NOT NULL, -- 32-byte BLAKE3 checksum
    file_size INTEGER NOT NULL,
    capture_time INTEGER,    -- Unix epoch (UTC)
    camera_make TEXT,
    camera_model TEXT,
    lens_model TEXT,
    focal_length REAL,
    aperture REAL,
    shutter_speed TEXT,
    iso INTEGER,
    rating INTEGER NOT NULL DEFAULT 0 CHECK (rating BETWEEN 0 AND 5),
    flag TEXT NOT NULL DEFAULT 'None' CHECK (flag IN ('Pick', 'Reject', 'None')),
    color_label TEXT NOT NULL DEFAULT 'None' CHECK (color_label IN ('Red', 'Yellow', 'Green', 'Blue', 'Purple', 'None')),
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_assets_hash ON assets(file_hash);
CREATE INDEX IF NOT EXISTS idx_assets_rating ON assets(rating);
CREATE INDEX IF NOT EXISTS idx_assets_flag ON assets(flag);

-- 2. Non-Destructive Parameter States (Current Active State)
CREATE TABLE IF NOT EXISTS develop_states (
    asset_id INTEGER PRIMARY KEY,
    params_json TEXT NOT NULL, -- Canonical JSON matching DevelopParameters
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    FOREIGN KEY(asset_id) REFERENCES assets(id) ON DELETE CASCADE
);

-- 3. Undo/Redo Directed Acyclic Graph (DAG) & Snapshots
CREATE TABLE IF NOT EXISTS develop_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id INTEGER NOT NULL,
    parent_history_id INTEGER,
    action_label TEXT NOT NULL, -- e.g., "Change Exposure", "Apply Preset: Moody"
    params_json TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    FOREIGN KEY(asset_id) REFERENCES assets(id) ON DELETE CASCADE,
    FOREIGN KEY(parent_history_id) REFERENCES develop_history(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_history_asset ON develop_history(asset_id);
```

---

## 7. State Synchronization & Undo/Redo DAG

To prevent slider interactions from spamming thousands of redundant undo nodes:

1. **Continuous Scrubbing (Transient State):**
   * Dragging a slider dispatches `Message::ParamExposure(val)`.
   * The UI updates synchronously; frames are rendered at 120 FPS.
   * **No undo record is created during active dragging.**
2. **Coalesced Commit (Persistent State):**
   * On mouse-button release (`Event::Mouse(mouse::Event::ButtonReleased)`), a debounced commit fires.
   * Halation hashes the new `DevelopParameters` struct via `blake3`.
   * If the hash differs from the parent state, a new node is appended to the `develop_history` table in SQLite.
3. **Branching History DAG:**
   * Undoing 5 steps and moving a new slider creates a branch node rather than overwriting history, preserving every user edit across sessions.