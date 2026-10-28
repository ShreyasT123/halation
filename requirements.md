# Halation: System & Product Requirements Specification (SRS)
**Document Version:** 1.0.0  
**Status:** Approved Architecture Draft  
**Target Platform:** Windows 10/11 (64-bit, x86-64 / ARM64), with architectural parity for macOS (Apple Silicon) and Linux (X11/Wayland).  
**Core Framework:** Pure Rust Workspace (`iced`, `wgpu`, `rayon`, `tokio`, `rusqlite`, `moxcms`).

---

## 1. Executive Summary & Product Mission

Halation is an uncompromising, desktop-class, non-destructive photo editing and digital asset management (DAM) application inspired by Adobe Lightroom Classic. 

### Core Design Tenets
1. **Engine-First Architecture:** The core image processing pipeline, catalog database, and color management systems are strictly decoupled from the UI layer. The UI is a presentation skin and command dispatcher.
2. **Absolute Non-Destructive Integrity:** Source RAW and raster assets are never altered. Every adjustment is stored as parametric metadata within a SQLite catalog and mirrored to industry-standard XMP sidecars.
3. **Floating-Point Wide-Gamut Pipeline:** All pixel manipulations take place in unbounded 32-bit floating-point linear color space using the **ProPhoto RGB (ROMM RGB)** or **ACEScg** color primaries to prevent clipping, banding, or color shifts.
4. **Hardware-Accelerated Responsiveness:** High-frequency interactions (sliders, tone curves, cropping) target **120 FPS** continuous viewport feedback via GPU compute shaders and multi-resolution proxy pyramids.
5. **Desktop Ergonomics:** Keyboard-driven navigation, mouse-scrubbing parameters, high-density controls, and zero decorative UI bloat.

---

## 2. System Architecture & Crate Topology

The workspace is organized into discrete domain crates with strict dependency boundaries:

```
┌────────────────────────────────────────────────────────────────────────┐
│                          apps/halation-desktop                         │
│             (OS Entry Point, Win32 Lifecycle, Native Menus)            │
└───────────────────────────────────┬────────────────────────────────────┘
                                    │
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│                           crates/halation-ui                           │
│        (Iced 0.13 Shell, Custom Widgets, Layout, Theme, Event Loop)     │
└───────┬──────────────┬─────────────┬─────────────┬─────────────┬───────┘
        │              │             │             │             │
        ▼              ▼             ▼             ▼             ▼
┌──────────────┐┌──────────────┐┌───────────┐┌───────────┐┌──────────────┐
│halation-core ││halation-color││halation-  ││halation-  ││halation-     │
│(Models, AST, ││(ICC, CMS,    ││pipeline   ││gpu        ││catalog       │
│ Undo DAG)    ││ Gamuts, Math)││(Scheduler,││(WGPU, WGSL││(SQLite, DAM, │
└───────┬──────┘└──────┬───────┘│ Proxies)  ││ Pipelines)││ Search, XMP) │
        │              │        └─────┬─────┘└─────┬─────┘└──────┬───────┘
        │              │              │            │             │
        └──────────────┴──────────────┼────────────┴─────────────┘
                                      ▼
                        ┌───────────────────────────┐
                        │       halation-raw        │
                        │(LibRaw / Demosaic / Codecs│
                        │     kamadak-exif)         │
                        └───────────────────────────┘
```

---

## 3. Module 1: Library & Digital Asset Management (DAM)

### 3.1 Catalog Database Specifications
* **Engine:** Embedded SQLite (`rusqlite` with `bundled` feature).
* **Storage Model:** Single-file catalog (`.halation-cat`) storing:
  * Asset metadata (path, volume serial, file hash, capture time, camera/lens UID).
  * Relational tags, collections, hierarchical keywords.
  * Star ratings (0–5), Pick/Reject flags (`Pick`, `Reject`, `Unflagged`), color labels (Red, Yellow, Green, Blue, Purple).
  * Non-destructive develop parameter histories and snapshot trees.
* **Volume Persistence (Windows First):** Asset volume serial numbers are captured via `GetVolumeInformationW`. If an external drive letter changes from `D:\` to `E:\`, Halation matches the volume GUID/serial without breaking the catalog link.

### 3.2 RAW & Image Ingestion Pipeline
* **Supported Formats:**
  * **RAW:** Sony (`.ARW`), Canon (`.CR2`, `.CR3`), Nikon (`.NEF`), Fujifilm (`.RAF`), Panasonic (`.RW2`), Adobe Digital Negative (`.DNG`).
  * **Raster:** TIFF (8/16-bit integer, 32-bit float), JPEG, PNG, WebP, JPEG XL (`.jxl`), AVIF.
* **Ingestion Modes:**
  * *Add in Place:* Index assets at their current disk location without moving.
  * *Copy to Destination:* Copy and structure into date-based folder hierarchies (`YYYY/YYYY-MM-DD/`).
* **Checksum Verification:** Every ingested file is hashed via `blake3`. Duplicate imports are detected in $\mathcal{O}(1)$ time.

### 3.3 Cache Architecture & Proxy Pyramids
To allow instantaneous scrolling through 100,000+ images without reading 50MB RAW files from disk:
* **Micro-Thumbnail Cache:** $256 \times 256$ px compressed JPEG/WebP stored in SQLite blob storage for the virtualized grid.
* **Standard Preview Cache:** $2048 \times 2048$ px 8-bit sRGB cached on NVMe disk in `.halation-cache/`.
* **1:1 Float Tile Cache:** $512 \times 512$ px half-float (`RGBA16F`) pyramid tiles generated on demand for pixel-peeping.

---

## 4. Module 2: The Develop Engine (Image Processing Pipeline)

### 4.1 Color Pipeline & Mathematical Order of Operations
Processing follows an invariant, linear, non-destructive sequence. Operations are strictly split between **scene-referred** (linear physical light) and **display-referred** (gamma/tonemapped):

```
[RAW Sensor Data (14/16-bit CFA)]
               │
               ▼
   1. Black Level Subtraction & White Level Normalization
               │
               ▼
   2. Demosaicing (AHD / RCD / Neural Demosaic) ──► Linear Sensor RGB
               │
               ▼
   3. White Balance & Chromatic Adaptation (Bradford Transform in LMS)
               │
               ▼
   4. Camera Calibration (Camera Input Matrix ──► CIE XYZ ──► Linear ProPhoto RGB)
               │
    ┌──────────┴───────────────────────────────────────────────────────┐
    │              SCENE-REFERRED LINEAR DOMAIN                        │
    │  • Exposure Compensation: C_out = C_in * 2^EV                    │
    │  • Defringe & Chromatic Aberration Correction                    │
    │  • Highlights & Shadows Recovery (Luminance Decomposition)       │
    │  • Linear Spatial Denoising & Deconvolution Sharpening          │
    │  • Local Adjustment Masks (Parametric & AI Bitmasks)             │
    └──────────┬───────────────────────────────────────────────────────┘
               │
               ▼
   5. Tone Curves & Contrast (Monotone Splines in Perceptual Domain)
               │
   6. HSL Adjustments & 3-Way Color Grading (Polar Hue/Chroma/Luma)
               │
    ┌──────────┴───────────────────────────────────────────────────────┐
    │              DISPLAY-REFERRED DOMAIN                             │
    │  • 3D LUT Color Profiles & Creative Look Transforms              │
    │  • Display Gamut Mapping (ProPhoto RGB ──► Target Monitor ICC)   │
    │  • Output Dithering (Temporal or 8/10-bit spatial Blue Noise)    │
    └──────────┬───────────────────────────────────────────────────────┘
               │
               ▼
[WGPU Viewport Swapchain (SDR sRGB / Display P3 / HDR10 PQ)]
```

### 4.2 Develop Parameter Specifications

#### A. Basic Tone Controls
* **Exposure:** Floating-point scalar range $[-5.00, +5.00]$ EV (stops). Step: $0.01$ EV.
* **Contrast:** Non-linear S-curve expansion $[-100, +100]$. Midpoint pinned at $18\%$ neutral gray ($0.18$ linear).
* **Highlights & Whites:**
  * *Highlights:* Recovery/compression $[-100, +100]$ operating on upper $30\%$ of the dynamic range curve.
  * *Whites:* Clipping point threshold $[-100, +100]$ anchoring the white point ($1.0$).
* **Shadows & Blacks:**
  * *Shadows:* Lift/crush $[-100, +100]$ operating on lower $30\%$ of dynamic range.
  * *Blacks:* Black point offset $[-100, +100]$ preventing zero-clipping while controlling toe depth.

#### B. Color & White Balance
* **Temperature:** Blackbody correlated color temperature (CCT) range $[2000\text{ K}, 50000\text{ K}]$. Neutral calibrated at $5500\text{ K}$.
* **Tint:** Green-Magenta balance $[-150, +150]$. Adjusts along the Planckian locus perpendicular.
* **Vibrance & Saturation:**
  * *Saturation:* Uniform radial chroma scaling across all gamuts $[-100, +100]$.
  * *Vibrance:* Non-linear chroma scaling with skin-tone protection; prioritizes under-saturated hues and dampens warm orange/red skin tones.

#### C. Parametric & Point Tone Curves
* **Interpolation:** Strict **monotone cubic spline** (Fritsch-Carlson algorithm) to prevent overshoot and local extrema inversion.
* **Channels:** Master Composite (RGB), Red, Green, Blue.
* **Split Points:** Parametric mode provides 3 movable zone thresholds dividing Shadows, Darks, Lights, and Highlights.

#### D. 3-Way Color Grading (Split Toning)
* **Target Bands:** Shadows, Midtones, Highlights, and Global.
* **Coordinate Space:** Polar coordinates $(\theta, r)$ where $\theta \in [0^\circ, 360^\circ]$ is Hue and $r \in [0.0, 1.0]$ is Saturation.
* **Luminance Offset:** Dedicated offset $[-100, +100]$ per band.
* **Balance & Blending:**
  * *Blending:* Overlap width between Shadows/Midtones and Midtones/Highlights $[0, 100]$.
  * *Balance:* Shifts the midpoint influence toward either shadows or highlights $[-100, +100]$.

#### E. Detail, Optics & Geometry
* **Sharpening:** Deconvolution algorithm: Amount, Radius ($0.5$–$3.0$ px), Detail (high-frequency suppression), Masking (Sobel edge-detection threshold).
* **Noise Reduction:**
  * *Luminance:* Bilateral / non-local means spatial filtering.
  * *Chrominance:* Median filtering in Lab color space ($a^*, b^*$ channels).
* **Optics:** Lens calibration database matching (Lensfun format):
  * Radial distortion polynomials: $r_d = r(1 + k_1 r^2 + k_2 r^4 + k_3 r^6)$.
  * Transverse Chromatic Aberration (TCA) red/blue scaling.
  * Cosine-fourth law vignetting falloff correction.
* **Geometry & Perspective:** Horizontal, Vertical, Aspect, Rotation ($\pm 45.0^\circ$), Keystone Upright auto-detection.

---

## 5. Module 3: Specialized Photography Widgets

The interface must implement custom, desktop-grade widgets designed for muscle-memory interaction:

### 5.1 The Scrubby Slider Primitive (`ScrubbySlider`)
* **Layout:**
  ```
  [ Label: 80px ] ─── [ Slider Track: Flex Fill ] ─── [ Editable Text: 60px ]
  ```
* **Interactions:**
  * **Direct Track Drag:** Linear parameter mapping.
  * **Relative Scrubbing:** Click-dragging the numeric readout initiates OS pointer capture (hidden cursor). Horizontal displacement adjusts value by relative delta $\Delta x$.
  * **Precision Mode:** Holding `Shift` reduces step size by $10\times$ (e.g., Exposure steps by $0.005$ EV instead of $0.05$ EV).
  * **Reset Action:** Double-clicking either the slider thumb, the label, or the numeric readout instantly snaps value to its calibrated `default_value`.
  * **Dynamic Track Shader:** The track is backed by a WGSL fragment shader rendering real-time gradients (e.g., Temperature renders Kelvin spectrum, Tint renders Green $\to$ Magenta, HSL channels render target hue band).

### 5.2 Direct-Manipulation Interactive Histogram (`HistogramZoneWidget`)
* **Display:** 256-bin additive overlay of Red, Green, Blue, and Luminance paths, normalized via peak square-root scaling to prevent high-frequency spikes from dominating the graph.
* **5 Interactive Direct-Manipulation Zones:**
  * Zone 1 (0–10%): **Blacks**
  * Zone 2 (10–35%): **Shadows**
  * Zone 3 (35–65%): **Exposure**
  * Zone 4 (65–90%): **Highlights**
  * Zone 5 (90–100%): **Whites**
* **Interactivity:**
  * Hovering over any zone highlights that region with an alpha-blended polygon.
  * Horizontal dragging inside a zone directly adjusts that parameter’s scalar, streaming updates to the slider and pipeline simultaneously.
* **Clipping Indicators:**
  * Left corner triangle: Toggles blue viewport overlay highlighting pixels where any channel $\le 0.0$.
  * Right corner triangle: Toggles red viewport overlay highlighting pixels where any channel $\ge 1.0$.

### 5.3 On-Canvas Viewport Overlays
* **Crop & Straighten:**
  * Interactive 8-point bounding box with aspect-ratio locking presets ($1:1$, $4:5$, $16:9$, $2:3$, Original, Custom).
  * Overlay grid toggles: Rule of Thirds, Golden Ratio, Diagonal, Grid.
  * Spirit Level tool: Click-drag vector along a visible horizon automatically calculates $\theta = \arctan2(\Delta y, \Delta x)$ and counter-rotates the crop canvas.
* **Targeted Adjustment Tool (TAT):**
  * Eyedropper cursor that samples the underlying pixel's luminance or color coordinate on click-drag, directly pulling the corresponding Tone Curve node or HSL slider.

---

## 6. Module 4: Hardware Graphics, Color Management & Shaders

### 6.1 Display & OS Color Management
* **Target Detection:**
  * Windows: Query monitor profile path via `GetICMProfileW`. Query Advanced Color state via DXGI / WinRT Display Information to detect HDR10 / wide-gamut mode.
  * macOS: Query active screen color profile via `objc2-app-kit` (`NSScreen.colorSpace`).
* **Transform Execution:**
  * Read display ICC profiles via `moxcms`.
  * Convert ProPhoto RGB linear floats into the calibrated display space via tetrahedral 3D LUT interpolation inside the final blit shader (`display_transform.wgsl`).

### 6.2 WGPU Compute Architecture
The Develop pipeline relies on a chain of WGSL compute shaders:

```
Buffer: RAW Sensor F32 
  ──► [CS_Demosaic] 
  ──► Texture2D<RGBA16F> (Linear ProPhoto)
  ──► [CS_ToneMapping & Exposure]
  ──► [CS_Curves_And_Color]
  ──► [CS_LocalMasksComposite]
  ──► Texture2D<RGBA16F> (Final Linear Render)
  ──► [VS/FS_DisplayBlit] (Color Profile LUT + Dither ──► Surface Texture)
```

---

## 7. Module 5: AI & Machine Learning Acceleration

### 7.1 Vision Capabilities
1. **Subject Segmentation:** One-click automated isolation of the primary subject.
2. **Sky Replacement / Masking:** Precise horizon segmentation with foliage feathering.
3. **Background Isolation:** Inversion of subject mask with depth-guided falloff.
4. **AI Denoise:** Deep residual neural network removing high-ISO sensor noise while preserving edge micro-contrast.

### 7.2 Inference Engine Specification
* **Runtime:** `ort` (ONNX Runtime bindings) or `candle-core` (pure Rust fallback).
* **Hardware Execution Providers (EP):**
  * Windows: **DirectML** (accelerated on Intel, AMD, and NVIDIA GPUs).
  * macOS: **CoreML** (targeting Apple Neural Engine / Apple Silicon GPU).
  * Linux: **TensorRT / CUDA** or OpenVINO.
* **Zero-Copy Interop:** Inference output masks are uploaded directly into `wgpu::Texture` bitmasks, bypassing CPU round-trips.

---

## 8. Module 6: Export & Output Processing

### 8.1 Batch Rendering Pipeline
* **Threading:** Dedicated multi-threaded export queue decoupled from the interactive UI, driven by `tokio` and `rayon`.
* **Output Formats:**
  * JPEG: High-speed quantization via `jpeg-encoder` / `zune-jpeg`.
  * TIFF: 8-bit or 16-bit uncompressed or LZW-compressed.
  * PNG: Standard 8/16-bit.
  * AVIF / JPEG XL: Modern HDR-ready formats embedding ICC profiles and gain maps.

### 8.2 Color Space & Resampling Options
* **Target Gamuts:** sRGB (Web), Adobe RGB (Print), Display P3, Rec.2020, ProPhoto RGB.
* **Spatial Resampling Algorithms:**
  * Lanczos3 (High sharpness downsampling).
  * Catmull-Rom (Smooth interpolation).
  * Bicubic / Bilinear.
* **Output Sharpening:** Radius and strength tuned for target media (Screen, Matte Paper, Glossy Paper) at target DPI (72, 150, 300).
* **Watermarking:** Text and vector SVG watermark overlay with opacity, anchor alignment, and blend mode controls.

---

## 9. Performance, Memory & Reliability Targets

| Metric | Target | Failure Condition |
| :--- | :--- | :--- |
| **Slider Interactive Latency** | $< 8.33\text{ ms}$ ($120\text{ FPS}$) | $> 16.6\text{ ms}$ (Dropped frames) |
| **Full Resolution Tile Settle** | $< 120\text{ ms}$ (NVMe / Modern GPU) | $> 350\text{ ms}$ |
| **Idle CPU / GPU Usage** | $0.0\%$ CPU / $0.0\%$ GPU | $> 1.0\%$ when no user input occurs |
| **Catalog Memory Footprint** | $< 350\text{ MB}$ RAM (10,000 photos indexed) | $> 1.5\text{ GB}$ RAM |
| **Catalog Scroll Frame Rate** | Constant $60$–$120\text{ FPS}$ | Hitching during thumbnail paging |
| **Crash Recovery** | Zero catalog corruption on abrupt termination | Corrupted SQLite state / lost edits |

---

## 10. File & Metadata Interoperability (XMP Standards)

* **XMP Sidecar Serialization:** Every adjustment maps directly to standard Adobe XMP tags where applicable:
  * Exposure: `crs:Exposure2012`
  * Contrast: `crs:Contrast2012`
  * Highlights: `crs:Highlights2012`
  * Shadows: `crs:Shadows2012`
  * Temperature: `crs:Temperature`
  * Tint: `crs:Tint`
  * Parametric Curve: `crs:ToneCurvePV2012`
* **Sync Strategy:**
  * Instant transactional write to SQLite catalog.
  * Asynchronous debounced background write ($500\text{ ms}$ idle delay) to the companion `.xmp` file alongside the RAW image.

---

## 11. Verification & Compliance Requirements

1. **Automated Unit Tests:**
   * All color matrix conversions must preserve energy ($\Delta E_{00} < 0.05$ roundtrip error).
   * Monotone cubic spline must never produce negative derivatives for monotonically increasing control points.
   * Catalog migrations must be covered by automated forward/backward migration unit tests.
2. **Visual Snapshot Regression:**
   * Automated rendering tests comparing deterministic RAW inputs against golden reference bitmaps.
3. **Memory Safety & Leak Proving:**
   * CI checks with `cargo clippy --all-targets -- -D warnings`.
   * Validation under address sanitizer (`ASan`) and heap profiling via `dhat`.