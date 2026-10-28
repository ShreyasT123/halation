Here is the comprehensive, production-grade **`IMPLEMENTATION_PLAN.md`** for Halation. It translates our architectural design into phased engineering milestones, detailing the exact crate tasks, technical deliverables, and verification criteria for each phase.

---

# Halation: Engineering Implementation Plan & Roadmap
**Document Version:** 1.0.0  
**Status:** Canonical Implementation Roadmap  
**Target Milestone Schedule:** Sprints 1 through 8  
**Baseline Workspace:** Rust 2021 Workspace (`apps/halation-desktop`, `crates/halation-*`)

---

## Roadmap Overview

```
Sprint 1: Foundation Hardening & Real Image Ingestion
    │
    ▼
Sprint 2: GPU Compute Migration (WGSL Engine & Viewport Blit)
    │
    ▼
Sprint 3: Viewport Navigation (Pan, Zoom, 1:1 Peeping, Split Before/After)
    │
    ▼
Sprint 4: Specialized Control Widgets (Scrubby Sliders, Spline Curve, Color Wheels)
    │
    ▼
Sprint 5: Deep Color Engine & Display Gamut Mapping (MoxCMS & 3D LUTs)
    │
    ▼
Sprint 6: Catalog Scaling & Digital Asset Management (100k Virtualized Grid)
    │
    ▼
Sprint 7: Non-Destructive History DAG & XMP Sidecar Sync
    │
    ▼
Sprint 8: Batch Export Pipeline & AI/Neural Acceleration (DirectML / SAM)
```

---

## Sprint 1: Foundation Hardening & Real Asset Ingestion

**Primary Objective:** Transition from procedural/synthetic test patterns to importing and decoding real photographic assets (RAW, DNG, TIFF, JPEG) from the local filesystem on Windows.

### 1.1 Tasks & Deliverables

* [ ] **Fix Windows Fallback Glyph Artifacts (`halation-ui`):**
  * Replace the Unicode star codepoints (`★`) that failed to render on Windows with clean ASCII or bundled SVG star icons using `iced::widget::svg`.
* [ ] **Integrate Native File Dialog (`halation-ui` + `rfd`):**
  * Implement an **"Import Photos..."** action in the top bar.
  * Wire `rfd::FileDialog::new()` with file extension filters: `*.arw`, `*.cr2`, `*.cr3`, `*.nef`, `*.dng`, `*.tif`, `*.tiff`, `*.jpg`, `*.jpeg`.
* [ ] **RAW & Metadata Ingestion Engine (`halation-raw`):**
  * Integrate `kamadak-exif` to extract full EXIF blocks: Camera Make/Model, Lens Model, Focal Length, Exposure Time, F-Number, ISO, Capture Timestamp, and White Balance tags.
  * Implement a pure-Rust DNG / TIFF / JPEG decoding path using `image` and `zune-image`.
  * Define `RawDecoder` trait with a fallback strategy (pure-Rust decoders for uncompressed DNG/TIFF/JPEG; initial LibRaw FFI wrapper stubbed behind a `native-raw` feature flag).
* [ ] **Catalog Indexing on Ingest (`halation-catalog`):**
  * Compute 32-byte `blake3` file hashes upon file selection.
  * Query Windows volume serial numbers via Win32 `GetVolumeInformationW`.
  * Insert ingested records into SQLite `assets` table within a single transactional batch (`conn.transaction()`).

### 1.2 Definition of Done (DoD)
* User clicks "Import", selects an actual `.DNG` or `.JPEG` from their Windows drive, and the image renders in the center viewport.
* The EXIF HUD below the canvas displays the camera, lens, ISO, shutter, and aperture extracted from the actual file.
* Relaunching the application shows the imported file persisted in the SQLite catalog.

---

## Sprint 2: GPU Compute Pipeline Migration

**Primary Objective:** Move the pixel-processing pipeline off the CPU Rayon thread pool and onto the GPU using `wgpu` and WGSL compute shaders.

### 2.1 Tasks & Deliverables

* [ ] **WGPU Pipeline Initialization (`halation-gpu`):**
  * Initialize `wgpu::Device`, `wgpu::Queue`, and `wgpu::CommandEncoder`.
  * Compile and validate `assets/shaders/develop_pipeline.wgsl` via `wgpu::include_wgsl!`.
* [ ] **GPU Buffer & Texture Topology (`halation-gpu`):**
  * Create an input texture: `texture_2d<f32>` (`Rgba16Float` or `Rgba32Float` linear).
  * Create a ping-pong output compute target: `texture_storage_2d<rgba16float, write>`.
  * Allocate the 16-byte aligned `GpuDevelopUniforms` uniform buffer.
* [ ] **WGSL Compute Pipeline Implementation (`assets/shaders/develop_pipeline.wgsl`):**
  * Implement compute workgroup $(16 \times 16)$ kernel for:
    1. Linear exposure scaling: $C_{\text{out}} = C_{\text{in}} \cdot 2^{\text{EV}}$.
    2. White balance matrix multiply (Bradford transform).
    3. Contrast S-curve around $18\%$ neutral gray.
    4. Highlight recovery and shadow lifting.
* [ ] **GPU Readback / Surface Presentation (`halation-pipeline` + `halation-ui`):**
  * Implement zero-copy blit from `output_tex` to the Iced swapchain texture view.
  * Benchmark slider-to-frame dispatch latency; target $< 4\text{ ms}$ compute execution time.

### 2.2 Definition of Done (DoD)
* Sliders drive GPU compute passes directly on the active hardware adapter (e.g., Intel Iris Xe / NVIDIA RTX).
* CPU usage drops to $< 1\%$ during continuous slider dragging.
* Viewport runs locked at $120\text{ FPS}$ during parameter scrubbing.

---

## Sprint 3: Viewport Ergonomics & Navigation

**Primary Objective:** Deliver Lightroom-standard viewport navigation: panning, continuous scaling, pixel-peeping modes, and Before/After split screens.

### 3.1 Tasks & Deliverables

* [ ] **Zoom Mode State Machine (`halation-ui`):**
  * Implement discrete zoom states: `Fit` (scale to window), `Fill` (crop to fill), `1:1` (100% actual pixels), `2:1` (200%), and `4:1` (400%).
  * Keyboard shortcut `Z` or single-click toggles between `Fit` and `1:1`.
* [ ] **Pan & Drag Navigation:**
  * In `1:1`+ zoom modes, holding `Space` (or mouse middle-click) engages the grab cursor.
  * Moving the mouse updates the normalized viewport translation vector $(\Delta x, \Delta y)$ clamped to image boundaries.
  * Mouse wheel scroll zooms smoothly centered around the cursor position.
* [ ] **Navigator Thumbnail Widget (`halation-ui`):**
  * Render a persistent $180\text{ px}$ mini-preview in the left panel.
  * Draw a red rectangular viewport boundary indicator over the thumbnail showing the currently visible crop window.
  * Dragging inside the mini-preview repositions the main canvas in real time.
* [ ] **Before / After Comparison (`\` shortcut):**
  * Implement `ViewMode::Single`, `ViewMode::SplitSideBySide`, and `ViewMode::SplitTopBottom`.
  * Render a movable vertical divider bar across the canvas: Left half = RAW unedited baseline; Right half = active Develop parameter stack.

### 3.2 Definition of Done (DoD)
* Pressing `1:1` instantly magnifies to native pixel resolution; mouse dragging pans across a 45-megapixel image smoothly.
* Pressing `\` instantly reveals the unedited original frame with zero hitching.

---

## Sprint 4: Specialized Photography Widgets

**Primary Objective:** Replace basic Iced sliders with purpose-built darkroom widgets matching Lightroom Classic’s physical tactility.

### 4.1 Tasks & Deliverables

* [ ] **The Scrubby Slider Widget (`crates/halation-ui/src/widgets/scrubby_slider.rs`):**
  * Custom `iced::advanced::Widget` implementation combining Label, Track, Thumb, and Value Readout.
  * Relative mouse scrubbing: Click-dragging the numeric readout locks the OS cursor, emitting relative motion events ($\Delta x$).
  * Precision mod: `Shift` key reduces slider increment by $10\times$.
  * Reset default: Double-clicking label, thumb, or text snaps parameter to `default_value`.
  * Dynamic track gradients: Temperature slider renders Kelvin blackbody gradient; Tint slider renders Green-to-Magenta.
* [ ] **Interactive 5-Zone Histogram (`crates/halation-ui/src/widgets/histogram.rs`):**
  * Direct-manipulation zones: Blacks ($0$–$10\%$), Shadows ($10$–$35\%$), Exposure ($35$–$65\%$), Highlights ($65$–$90\%$), Whites ($90$–$100\%$).
  * Hovering a zone renders an alpha-blended polygon overlay.
  * Horizontal click-dragging inside a zone directly streams parameter deltas to the corresponding slider and compute pipeline.
* [ ] **Monotone Cubic Spline Tone Curve (`crates/halation-ui/src/widgets/tone_curve.rs`):**
  * Implement Fritsch-Carlson monotone cubic spline interpolation in `halation-color`.
  * Draw vector spline curve over the background histogram.
  * Support clicking anywhere on the curve to add control points; drag control points on $[0.0, 1.0]$; right-click / double-click to remove.
  * Channel selector: Composite RGB, Red, Green, Blue.
  * Generate 256-entry 1D LUT and upload to GPU `texture_1d<f32>` on modification.
* [ ] **3-Way Color Grading Wheels (`crates/halation-ui/src/widgets/color_wheel.rs`):**
  * Polar coordinate widget: Radius = Saturation $[0.0, 1.0]$, Angle = Hue $[0^\circ, 360^\circ]$.
  * Central puck with reset anchor dot for Shadows, Midtones, and Highlights.

### 4.2 Definition of Done (DoD)
* Double-clicking any slider snaps it back to neutral calibrated default.
* Dragging inside the Exposure zone of the histogram dynamically updates the Exposure slider.
* The Tone Curve allows placing control points without spline overshoot artifacts.

---

## Sprint 5: Deep Color Engine & Display Gamut Mapping

**Primary Objective:** Deliver rigorous color fidelity: wide-gamut working spaces (ProPhoto RGB / ACEScg), OS monitor profile detection, and 3D tetrahedral LUT color transformations.

### 5.1 Tasks & Deliverables

* [ ] **OS Display Profile Query (`halation-color`):**
  * Windows: Use `windows::Win32::Graphics::Gdi::GetICMProfileW` on the current window `HDC` to read the calibrated monitor ICC profile path from disk.
  * macOS: Query `NSScreen.colorSpace` via `objc2-app-kit`.
  * Watch for window migration across monitors with differing gamuts (e.g., dragging from sRGB laptop screen to a wide-gamut Display P3 monitor).
* [ ] **Tetrahedral 3D LUT Generator (`halation-color` + `moxcms`):**
  * Parse the monitor ICC profile via `moxcms`.
  * Construct a $33 \times 33 \times 33$ 3D LUT transforming linear ProPhoto RGB $\to$ calibrated monitor color space.
  * Upload the LUT to `wgpu::Texture` (`texture_3d<f32>`).
* [ ] **Display Blit Shader (`assets/shaders/display_blit.wgsl`):**
  * Sample the 3D LUT via tetrahedral or trilinear interpolation in the final swapchain render pass.
  * Implement spatial blue-noise dithering to eliminate 8-bit banding on standard SDR panels.

### 5.2 Definition of Done (DoD)
* Moving the application window from an sRGB display to a Display P3 / wide-gamut monitor dynamically recalibrates the display LUT.
* Deep saturated reds, greens, and blues match Adobe Lightroom Classic’s rendering of the same RAW file with zero gamut clipping.

---

## Sprint 6: Catalog Scaling & Digital Asset Management (DAM)

**Primary Objective:** Transform Halation into a production-grade digital darkroom capable of fluidly navigating 100,000+ RAW photos.

### 6.1 Tasks & Deliverables

* [ ] **High-Throughput SQLite Schema Optimization (`halation-catalog`):**
  * Enforce Write-Ahead Logging (`PRAGMA journal_mode = WAL;`) and `PRAGMA synchronous = NORMAL;`.
  * Build composite indexes on `(rating, flag, capture_time)`.
* [ ] **Virtualized Grid & Filmstrip Widget (`halation-ui`):**
  * Implement an infinite virtual scroll area in Iced that only renders cells visible in the viewport ($+2$ row buffer).
  * Support keyboard navigation: Arrow keys to move selection, `1`–`5` for star ratings, `6`–`9` for color labels, `P`/`X`/`U` for Pick/Reject/Unflag.
  * Marquee multi-selection and `Shift`-click range selection.
* [ ] **Multi-Tier Thumbnail Cache Engine (`halation-pipeline`):**
  * Tier 1 (Memory): 500-item LRU cache of decoded $256\text{ px}$ micro-thumbnails.
  * Tier 2 (Disk): Fast compressed WebP thumbnails stored in `.halation-cache/` named by `blake3` file hashes.
  * Background worker threads asynchronously decode and scale thumbnails without stuttering the scrolling grid.

### 6.2 Definition of Done (DoD)
* Ingest a catalog folder containing 5,000+ RAW photos.
* Scrolling the grid with a high-speed mouse wheel maintains a locked $60$–$120\text{ FPS}$ with zero UI freezes.
* Pressing `P` on an image flags it as "Pick" in memory and commits the change to SQLite in $< 2\text{ ms}$.

---

## Sprint 7: Non-Destructive History DAG & XMP Sync

**Primary Objective:** Implement an infallible, branching undo/redo history graph and Adobe-compatible `.xmp` sidecar synchronization.

### 7.1 Tasks & Deliverables

* [ ] **Interaction Coalescing & History DAG (`halation-core` + `halation-catalog`):**
  * Differentiate between continuous slider dragging (transient updates) and mouse release (history commit).
  * On mouse release, compute `blake3` hash of the new parameter state.
  * If different from the current parent node, insert a new record into `develop_history`.
  * Support branching: undoing 3 steps and making a new adjustment creates a branch node rather than truncating history.
* [ ] **History Panel UI (`halation-ui`):**
  * Render a vertical timeline in the left panel showing every action (e.g., *"Imported"*, *"Exposure +0.50"*, *"Tone Curve"*, *"Contrast -10"*).
  * Clicking any step reverts the canvas and develop sliders to that exact historical state.
* [ ] **XMP Sidecar Serialization & Deserialization (`halation-catalog`):**
  * Implement an Adobe-compatible XML serializer/parser for `.xmp` sidecars.
  * Read existing Adobe Lightroom develop settings (`crs:Exposure2012`, `crs:Temperature`, etc.) from companion `.xmp` files on disk.
  * Debounced asynchronous background write: after $500\text{ ms}$ of user idle time, write the updated parameter state back to the companion `.xmp` file.

### 7.2 Definition of Done (DoD)
* Adjusting an image, closing Halation, and opening the image in Adobe Lightroom Classic reveals identical slider values read from the companion `.xmp`.
* User can traverse 50 undo steps back and forth without state corruption.

---

## Sprint 8: Batch Export & Neural Vision Acceleration

**Primary Objective:** Deliver an industrial-grade batch export engine and hardware-accelerated AI segmentation masks.

### 8.1 Tasks & Deliverables

* [ ] **Batch Export Queue (`halation-pipeline` + `apps/halation-desktop`):**
  * Background multi-core export pipeline decoupled from the UI thread.
  * Options dialog: File format (JPEG, TIFF 8/16-bit, PNG, AVIF), color space (sRGB, Adobe RGB, ProPhoto), dimension scaling (Lanczos3 resampling), and output sharpening.
  * Progress HUD in the top bar showing active export progress and time remaining.
* [ ] **AI Neural Vision Engine (`crates/halation-ml`):**
  * Integrate `ort` (ONNX Runtime) with **DirectML** execution provider on Windows.
  * Load quantized vision weights (e.g., MobileSAM / SAM for subject and sky segmentation).
  * One-click **"Select Subject"** and **"Select Sky"** generating 8-bit precision bitmasks.
  * Upload masks directly into `wgpu::Texture` to apply localized exposure/color adjustments.

### 8.2 Definition of Done (DoD)
* Export 50 high-resolution RAW images to full-quality sRGB JPEGs in the background while continuing to edit a 51st image with zero slider stutter.
* Clicking "Select Sky" isolates the sky in $< 250\text{ ms}$ on the Intel Iris Xe GPU via DirectML, enabling localized exposure adjustments restricted to the clouds.

---

## Execution Matrix & Milestone Checkpoints

| Sprint | Target Subsystem | Primary Crate | Deliverable Milestone |
| :---: | :--- | :--- | :--- |
| **1** | Ingestion & EXIF | `halation-raw`, `halation-catalog` | Real RAW/DNG file open via native Windows dialog |
| **2** | GPU Compute | `halation-gpu`, `assets/shaders` | WGSL shader running sliders at 120 FPS on Vulkan/DX12 |
| **3** | Navigation | `halation-ui` | Fit, 1:1 pixel peeping, smooth pan, Before/After split |
| **4** | Custom Widgets | `halation-ui` (custom widgets) | Scrubby sliders, interactive 5-zone histogram, Tone Curve |
| **5** | Color Engine | `halation-color` | Monitor ICC detection via Win32 + tetrahedral 3D LUT blit |
| **6** | Catalog Scaling | `halation-catalog`, `halation-pipeline` | 100k photo virtualized grid with multi-tier LRU caching |
| **7** | History & XMP | `halation-core`, `halation-catalog` | Branching undo DAG + Adobe-compatible XMP sidecar sync |
| **8** | Export & AI | `halation-ml`, `halation-pipeline` | Multi-core batch export + DirectML subject/sky masking |