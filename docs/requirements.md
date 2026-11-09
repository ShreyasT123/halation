# Halation Desktop: Software Requirements Specification (SRS)
**Document Version:** 2.0.0 (Corrected Canonical)  
**Target Surface:** Offline Desktop (Windows 10/11, macOS 14+, Linux x86_64)  
**Runtime Engine:** Rust 2021 Workspace (`wgpu`, `iced`, `rusqlite`, `libraw-sys`, DirectML/CoreML)

---

## 1. System Vision & Core Constraints

### 1.1 Scope & Offline Guarantee
Halation is a professional, standalone, non-destructive desktop photo-editing application. 
- **100% Air-Gapped Operation:** The software **must** function completely without an internet connection. There are no user accounts, activation servers, cloud sync daemons, remote telemetry collectors, or credit-metered APIs.
- **In-Place File System Authority:** The local file system is the single source of truth. Halation opens, reads, and edits files directly in place on local drives, network-attached storage (NAS), and removable media.
- **Sidecar Synchronization:** Edits, ratings, color labels, and crop parameters **must** be serialized to industry-standard Adobe-compatible `.xmp` companion files alongside the source media. Moving or renaming files within Halation moves both the primary asset and its sidecar.

---

## 2. System Architecture & Data Model

### 2.1 Concurrency & Threading Topology
Halation enforces a 4-tier execution model to prevent UI thread contention and ensure a sustained 120 FPS interface:

```
┌────────────────────────────────────────────────────────────────────────┐
│ 1. MAIN UI THREAD (Iced / Elm Architecture)                            │
│    • Handles user input, layout (Taffy flexbox), drag deltas           │
│    • Manages widget states (Scrubby Sliders, Curve controls)           │
│    • Dispatches transient GPU uniforms directly (< 1 ms latency)       │
└───────────────────┬─────────────────────────────────┬──────────────────┘
                    │ (Zero-copy GPU Ring Buffer)     │ (Lock-free MPSC Queue)
                    ▼                                 ▼
┌───────────────────────────────────────┐ ┌──────────────────────────────┐
│ 2. GPU COMPUTE QUEUE (Dedicated)      │ │ 3. CPU WORKER POOL (Rayon)   │
│    • 120 FPS Viewport Develop Pipeline│ │    • Full-sensor RAW Demosaic│
│    • 32-bit Floating-Point WGSL       │ │    • Tile Pyramid Generation │
│    • Display Gamut 3D Tetrahedral LUT │ │    • Multi-core Batch Export │
└───────────────────────────────────────┘ └──────────────────────────────┘
                    ▲                                 ▲
                    │                                 │
┌───────────────────┴─────────────────────────────────┴──────────────────┐
│ 4. ASYNC BACKGROUND RUNTIME (Tokio)                                    │
│    • Disk I/O & directory watcher events                               │
│    • SQLite WAL transactions (debounced catalog commits)               │
│    • Local thumbnail generation and disk cache writes                  │
└────────────────────────────────────────────────────────────────────────┘
```

### 2.2 Volume Mobility & Catalog Schema
To prevent asset breakage when external drives change drive letters on Windows or mount points on macOS/Linux, file identity **shall not** rely on absolute file paths.

#### Database Configuration
- SQLite running in **Write-Ahead Logging mode** (`PRAGMA journal_mode = WAL;`).
- Dedicated single-writer connection with pooled multi-threaded reader connections (`PRAGMA synchronous = NORMAL;`).

#### Schema Definition
```sql
-- 1. Ingested Photo Assets
CREATE TABLE IF NOT EXISTS assets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    volume_uuid TEXT NOT NULL,         -- OS volume GUID / fsid
    relative_path TEXT NOT NULL,       -- Path relative to volume root
    file_hash BLOB NOT NULL,           -- 32-byte BLAKE3 checksum
    file_size INTEGER NOT NULL,
    capture_time INTEGER,              -- Unix epoch (UTC)
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
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    UNIQUE(volume_uuid, relative_path)
);

CREATE INDEX IF NOT EXISTS idx_assets_lookup ON assets(volume_uuid, relative_path);
CREATE INDEX IF NOT EXISTS idx_assets_rating ON assets(rating);
CREATE INDEX IF NOT EXISTS idx_assets_flag ON assets(flag);

-- 2. Non-Destructive Active Parameter States
CREATE TABLE IF NOT EXISTS develop_states (
    asset_id INTEGER PRIMARY KEY,
    active_history_id INTEGER NOT NULL, -- Points to the active head of the DAG
    params_json TEXT NOT NULL,          -- Canonical JSON of DevelopParameters
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    FOREIGN KEY(asset_id) REFERENCES assets(id) ON DELETE CASCADE,
    FOREIGN KEY(active_history_id) REFERENCES develop_history(id)
);

-- 3. Branching History DAG
CREATE TABLE IF NOT EXISTS develop_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id INTEGER NOT NULL,
    parent_history_id INTEGER,          -- Self-reference for branching
    action_label TEXT NOT NULL,          -- e.g., "Exposure +0.50", "Point Curve"
    params_json TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    FOREIGN KEY(asset_id) REFERENCES assets(id) ON DELETE CASCADE,
    FOREIGN KEY(parent_history_id) REFERENCES develop_history(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_history_asset ON develop_history(asset_id);
```

### 2.3 Interaction Coalescing & History DAG
1. **Transient State (Dragging):** Slider scrubbing updates the UI and streams raw parameter structs to the GPU uniform buffer at display refresh rate. **No database transactions or undo nodes shall be generated during active dragging.**
2. **Coalesced Commit:** Upon mouse release (`ButtonReleased`) or keyboard entry confirmation, the state is hashed via `BLAKE3`. If the hash differs from the active history node:
   - A new child node is appended to `develop_history` with `parent_history_id = active_history_id`.
   - `develop_states.active_history_id` is advanced to this new node.
   - An asynchronous debounced write ($500\text{ ms}$) serializes the parameters to the companion `.xmp` file.
3. **Branching:** If a user navigates back $N$ steps in history and alters a slider, a new branch node is spawned. Prior history nodes **must not** be truncated or overwritten.

---

## 3. Imaging Pipeline & Color Science Requirements

### 3.1 RAW Ingestion Pipeline
- **Engine:** Decoupled architecture using `libraw-sys` (LibRaw 0.21+) as the baseline camera RAW demosaicing engine for proprietary sensor formats (`.arw`, `.cr2`, `.cr3`, `.nef`, `.orf`, `.rw2`). Pure-Rust codecs (`image`, `zune-image`) decode uncompressed DNGs, standard TIFFs, and JPEGs.
- **Linearization:** RAW data must be extracted into unclipped 32-bit floating-point linear sensor space, retaining original black levels and white saturation points.
- **Metadata Extraction:** `kamadak-exif` parses standard EXIF and MakerNote tags (Camera Make/Model, Lens, Focal Length, Exposure Time, F-Number, ISO, Capture Time, Orientation).

### 3.2 Internal Working Color Space
- **Primary Working Space:** **Radiometrically Linear ProPhoto RGB** (ROMM RGB) with standard CIE D65 alignment.
- **Linear ProPhoto Luminance Weights:** All luminance, thresholding, and contrast calculations in WGSL shaders **must** use the normalized ProPhoto RGB matrix luminance coefficients:
  $$\mathbf{Y}_{\text{ProPhoto}} = [0.28804,\ 0.71187,\ 0.00009]$$
  *(The use of Rec.709 or sRGB weights $[0.2126, 0.7152, 0.0722]$ is strictly prohibited).*

### 3.3 GPU Memory Layouts (WGSL std140 Compliance)
Uniform structures transferred to the GPU must be 16-byte aligned.

```rust
#[repr(C, align(16))]
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
    pub _padding: [f32; 2],  // Pads struct to 48 bytes (multiple of 16)
}

/// Padded 3x3 matrix for WGSL uniform buffers where each column occupies 16 bytes.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuMat3x3 {
    pub col0: [f32; 4], // 3 floats + 1 float padding
    pub col1: [f32; 4],
    pub col2: [f32; 4],
}
```

### 3.4 WGSL Compute Shader Specification
The real-time develop pipeline **must** execute in a single compute pass on $16 \times 16$ threadgroups over linear floating-point textures:

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

struct WhiteBalanceMatrix {
    col0: vec4<f32>,
    col1: vec4<f32>,
    col2: vec4<f32>,
};

const PROPHOTO_LUMA: vec3<f32> = vec3<f32>(0.28804, 0.71187, 0.00009);

@group(0) @binding(0) var input_tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var display_lut_3d: texture_3d<f32>;

@group(1) @binding(0) var<uniform> u: DevelopUniforms;
@group(1) @binding(1) var<uniform> wb: WhiteBalanceMatrix;
@group(1) @binding(2) var output_tex: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(16, 16)
fn cs_develop_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(input_tex);
    if (id.x >= dims.x || id.y >= dims.y) { return; }
    let coord = vec2<i32>(id.xy);

    var color = textureLoad(input_tex, coord, 0).rgb;

    // 1. Chromatic Adaptation / White Balance (Bradford Matrix Multiply)
    let wb_mat = mat3x3<f32>(wb.col0.xyz, wb.col1.xyz, wb.col2.xyz);
    color = wb_mat * color;

    // 2. Exposure Scaling (Linear Radiometric Energy)
    color = color * exp2(u.exposure);

    // 3. High Dynamic Range Tone Reconstruction
    let luma = dot(color, PROPHOTO_LUMA);
    if (luma > 0.00001) {
        let log_luma = log2(luma);
        let shadow_weight = clamp(1.0 - (log_luma + 4.0) / 4.0, 0.0, 1.0);
        let highlight_weight = clamp((log_luma + 1.0) / 4.0, 0.0, 1.0);
        
        let luma_delta = (u.shadows * 0.01 * shadow_weight) + (u.highlights * 0.01 * highlight_weight);
        color *= exp2(luma_delta);
    }

    // 4. Whites & Blacks Linear Scaling
    let white_scale = 1.0 + (u.whites * 0.005);
    let black_offset = u.blacks * 0.002;
    color = max((color * white_scale) + vec3<f32>(black_offset), vec3<f32>(0.0));

    // 5. Radiometric Contrast (Log-Domain Sigmoid about 18% Gray)
    let contrast_gamma = 1.0 + (u.contrast * 0.005);
    color = pow(color / 0.18, vec3<f32>(contrast_gamma)) * 0.18;

    // 6. Vibrance & Saturation
    let post_luma = dot(color, PROPHOTO_LUMA);
    let max_c = max(color.r, max(color.g, color.b));
    let min_c = min(color.r, min(color.g, color.b));
    let current_sat = (max_c - min_c) / (max_c + 0.001);
    
    // Vibrance selectively boosts less-saturated colors
    let vib_mult = 1.0 + (u.vibrance * 0.01 * (1.0 - current_sat));
    let sat_mult = 1.0 + (u.saturation * 0.01);
    color = post_luma + (color - post_luma) * (vib_mult * sat_mult);

    // 7. Display Gamut Mapping & Output OETF via 3D Tetrahedral Display LUT
    // The 3D LUT encapsulates: Linear ProPhoto RGB -> Monitor Gamut (ICC) + Tone Response Curve
    let display_color = textureSampleLevel(display_lut_3d, samp, clamp(color, vec3(0.0), vec3(1.0)), 0.0).rgb;

    textureStore(output_tex, coord, vec4<f32>(display_color, 1.0));
}
```

### 3.5 Display Gamut Management (Monitor ICC Adaptation)
- **Profile Detection:** Query the active display ICC profile across platforms:
  - **Windows:** `GetICMProfileW` on the current window `HDC`.
  - **macOS:** `CGDisplayCopyColorSpace` via CoreGraphics/AppKit bindings.
  - **Linux:** X11 `_ICC_PROFILE` atom or Wayland `color-management-v1` protocol.
- **3D Tetrahedral LUT:** `moxcms` parses the monitor ICC profile and generates a $33 \times 33 \times 33$ 3D LUT mapping linear ProPhoto RGB directly to the calibrated monitor color space.
- **Dynamic Window Migration:** Window movement across monitors with differing color gamuts (e.g., sRGB to Display P3) **must** recompute and upload a new 3D LUT texture in $< 50\text{ ms}$.

---

## 4. Functional Specifications

### 4.1 Ingestion & Library Browser
- **Direct Directory Access:** The left pane renders an OS folder tree. Selecting a directory scans images in place; no files are copied or ingested into a hidden proprietary library.
- **Format Filter:** Filters support `.arw`, `.cr2`, `.cr3`, `.nef`, `.dng`, `.tif`, `.tiff`, `.jpg`, `.jpeg`, and `.webp`.
- **Metadata Badges:** Square grid cells display Rating (0–5 stars), Pick/Reject flags, and Color Labels.
- **Virtualized Grid:** Viewport virtualization allows smooth, hitch-free scrolling through directories with 100,000+ assets, allocating rendering overhead exclusively to visible items plus a two-row buffer.

### 4.2 Assisted Culling (100% Local Inference)
- **Local Face & Saliency Analysis:** Analysis is processed entirely on the local CPU/NPU thread pool without remote network requests.
- **Quality Metrics:**
  - *Subject Sharpness:* Normalized gradient variance across salient feature boundaries.
  - *Eye State & Sharpness:* Pupil boundary detection and Eye Aspect Ratio (EAR) to detect closed eyes or blinks.
  - *Technical Rejection:* Automatic flagging for motion blur and clipped exposure extremes.
- **Interface:** Dual-state `Select` and `Reject` badges with real-time threshold adjusters that filter the visible grid without re-running inference passes.

### 4.3 Viewport & Loupe Navigation
- **View Modes:**
  - `Fit`: Scales the full frame to the viewport boundary.
  - `Fill`: Crops frame to completely fill the canvas.
  - `1:1` (100%): Maps 1 image pixel to 1 display pixel.
  - `2:1`, `4:1`: Magnification modes for fine noise and focus inspection.
- **Navigation Shortcuts:**
  - `Space` (Hold) + Mouse Drag: Engages pan canvas.
  - `Z` or Single Click: Toggles between `Fit` and `1:1`.
  - `\` (Backslash): Toggles Before/After split view (`SplitSideBySide`, `SplitTopBottom`, or full frame toggle).
- **Navigator Widget:** Persistent mini-map rendering an overview thumbnail with an interactive viewport indicator rectangle.

### 4.4 Parameter Sliders & Specialized Darkroom Widgets
- **Scrubby Sliders:**
  - Horizontal drag on numeric labels streams relative motion deltas.
  - `Shift` key modifier drops slider sensitivity by $10\times$.
  - Double-clicking label or thumb snaps parameter back to default.
  - Temperature track renders a calibrated Kelvin blackbody gradient ($2000\text{ K} \to 50000\text{ K}$); Tint track renders Green-to-Magenta.
- **Interactive 5-Zone Histogram (`J` toggles clipping):**
  - Zones: Blacks ($0–10\%$), Shadows ($10–35\%$), Exposure ($35–65\%$), Highlights ($65–90\%$), Whites ($90–100\%$).
  - Hovering a zone highlights the tonal range; click-dragging inside a zone modulates the associated slider directly.
- **Tone Curve:**
  - Monotone cubic spline interpolation (Fritsch-Carlson) evaluated without overshoot artifacts.
  - Channel selector: Composite RGB, Red, Green, Blue.
  - Points evaluated analytically in shaders or mapped via high-precision log-domain transfer functions to avoid shadow posterization.
- **3-Way Color Wheels:**
  - Individual polar wheels for Shadows, Midtones, Highlights, and Global.
  - Puck angle dictates Hue ($0^\circ–360^\circ$); radial distance dictates Saturation ($0.0–1.0$).
  - Dedicated baseline Luminance slider for each tonal wheel.

### 4.5 Local Retouching & AI Masking
- **Two-Stage On-Device Neural Masking:**
  - **Stage 1 (Asynchronous Background Embedding):** When an asset is selected in the Loupe, an on-device quantized model (MobileSAM / SAM via DirectML on Windows, CoreML on macOS) computes the image embedding tensor in the background.
  - **Stage 2 (Interactive Prompt Decoding):** User clicks "Select Subject" or "Select Sky". The prompt decoder runs against the cached embedding tensor, outputting an 8-bit precision mask in $< 20\text{ ms}$.
- **Geometric Masks:** Brush (Size, Feather, Flow, Density, Auto-Mask), Linear Gradient, Radial Gradient.
- **Range Masks:** Color Range (sampled up to 5 points), Luminance Range (with falloff map), Depth Range (utilizing EXIF embedded depth textures).
- **Local Poisson Healing:**
  - Heals blemishes and sensor dust via gradient domain blending (solving Poisson's equation over the destination boundary).
  - "Visualize Spots" (`A`) renders an inverted high-contrast laplacian edge overlay to reveal faint sensor dust.

### 4.6 Batch Processing & Export Engine
- **Decoupled Render Queue:** Multi-threaded Rayon export pipeline decoupled from the active UI loop.
- **Export Options:**
  - **Formats:** JPEG (selectable quality 1–100%), TIFF (8-bit or 16-bit uncompressed/LZW), PNG, WebP, Linear Archival DNG.
  - **Scaling:** High-quality Lanczos3 resampling to arbitrary bounding dimensions.
  - **Output Color Spaces:** sRGB, Adobe RGB (1998), Display P3, ProPhoto RGB (with embedded ICC profile chunks).
  - **Watermarking:** Vector text layout or user-supplied transparent PNG logo with precise sub-pixel anchoring.
- **External Editor Handoff:** Configurable path to external system executables (e.g., Affinity Photo, GIMP). Emits a 16-bit ProPhoto TIFF, spawns the process, and watches the file system for saved modifications.

---

## 5. Technical Stack & Dependencies

| Subsystem | Crates / Libraries | Minimum Version | Architectural Purpose |
| :--- | :--- | :--- | :--- |
| **UI Framework** | `iced`, `iced_wgpu` | `0.13` | Elm architecture GUI; custom `shader::Program` integration sharing `wgpu::Device`. |
| **GPU Hardware API** | `wgpu` | `23.0` | Portable abstraction over DirectX 12, Metal, and Vulkan. |
| **RAW Ingestion** | `libraw-sys` | `0.21` | Native CFA Bayer/X-Trans demosaicing and sensor unpacking. |
| **Color Science** | `moxcms` | `0.5` | ICC profile parsing, tetrahedral 3D display LUT generation. |
| **Embedded Database**| `rusqlite` | `0.32` (`bundled`) | Local ACID catalog storage running in WAL mode. |
| **Hashing Engine** | `blake3` | `1.5` | File integrity, deduplication, and parameter state hashing ($> 3\text{ GB/s}$). |
| **CPU Concurrency** | `rayon` | `1.10` | Work-stealing thread pool for tile rendering and export queues. |
| **Async Runtime** | `tokio` | `1.40` (`rt-multi-thread`) | Non-blocking file I/O, directory monitoring, catalog commits. |
| **Linear Algebra** | `glam` | `0.29` (`bytemuck`) | SIMD math; zero-copy uniform memory transmutation. |
| **Local ML Engine** | `ort` | `2.0` (`directml`, `coreml`)| On-device neural inference for vision segmentation masks. |

---

## 6. Hardware Baselines & Verification Standards

### 6.1 Hardware Support Tiers

| Component | Minimum Specification | Recommended Specification |
| :--- | :--- | :--- |
| **Operating System** | Windows 10 64-bit (22H2+), macOS 14+, Linux 6.x | Windows 11, macOS 15 Sonoma/Sequoia |
| **Processor** | 4-Core Intel Core i5 / AMD Ryzen 5 (2.5 GHz) | 8-Core Intel Core i7/i9, AMD Ryzen 7/9, Apple M2/M3 |
| **Memory (RAM)** | 8 GB System RAM | 16 GB to 32 GB System / Unified RAM |
| **Graphics Adapter** | DirectX 12 / Metal / Vulkan capable with 2 GB VRAM | Dedicated GPU with $\ge 8\text{ GB}$ VRAM (RTX 3060+, Apple M-Series) |
| **Storage** | 500 MB for app binary; NVMe SSD for cache | PCIe 4.0 NVMe SSD for fast RAW preview caching |

### 6.2 Acceptance & Compliance Tests

1. **Air-Gap Verification:**
   - Execute Halation inside an isolated network sandbox (all inbound and outbound network interfaces blocked).
   - Verify all subsystems remain operational: RAW decoding, parametric editing, local AI segmentation, SQLite catalog queries, and batch export.
   - Run packet inspection (e.g., Wireshark) over a 24-hour cycle; verify that **zero network frames are emitted**.

2. **Sidecar Interoperability:**
   - Modify exposure, contrast, temperature, and crop parameters on a `.NEF` file in Halation.
   - Verify the written companion `.xmp` file conforms to the Adobe XMP Core specification.
   - Open the same image directory in Adobe Lightroom Classic or Capture One; confirm that basic parameters match without error.

3. **Performance Floor:**
   - Continuous scrubbing of the Exposure slider on a 45 MP uncompressed RAW file **must** maintain $\ge 60\text{ FPS}$ on minimum spec hardware, and $\ge 120\text{ FPS}$ on recommended hardware.
   - CPU utilization **must not exceed $2\%$** during active GPU slider scrubbing.
   - Navigating a virtualized grid of 100,000 indexed records **must** maintain a locked $60\text{ FPS}$ scroll rate.