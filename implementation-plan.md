# Halation: Final Engineering Implementation Plan & UI Design System
**Document Version:** 2.0.0 (Corrected Canonical Roadmap)  
**Status:** Approved for Execution  
**Execution Horizon:** Sprints 1 through 8  
**Architecture:** 100% Standalone Offline Desktop (Windows 10/11, macOS 14+, Linux x86_64)

---

## 0. UI/UX Visual Architecture & Design System

To replace crude prototypes with a high-density, professional darkroom interface, Halation blends the **tactile depth of Lightroom Classic** (persistent Navigator mini-map, collapsible History DAG with delta labels, 5-zone histogram, and live EXIF HUD) with the **restrained, modern ergonomics of modern Lightroom Desktop** (vertical tool rails, high-contrast monospace readouts, fluid scrubby sliders with dynamic gradient tracks, and borderless canvas framing).

```
┌────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│ WINDOW TITLEBAR & APPLICATION MENU (File, Edit, Photo, View, Window, Help)                             │
├─────────┬───────────────────────────────────────────────────────────────┬──────────────────────────────┤
│ LEFT    │ CENTER VIEWPORT (The Loupe Canvas)                            │ RIGHT EDIT & TOOL RAIL       │
│ PANEL   │                                                               │                              │
│ (260px) │ ┌───────────────────────────────────────────────────────────┐ │ ┌──────────────────────────┐ │
│         │ │                                                           │ │ │ HISTOGRAM & EXIF HUD     │ │
│ • Nav   │ │                                                           │ │ │ [RGB Waveform + Metadata]│ │
│   Mini- │ │                                                           │ │ ├──────────────────────────┤ │
│   map   │ │                                                           │ │ │ TOOL STRIP (H, C, B, M)  │ │
│ • Snap- │ │                  IMAGE CANVAS                             │ │ ├──────────────────────────┤ │
│   shots │ │               (32-bit Float WGPU)                         │ │ │ ACCORDION PANELS:        │ │
│ • Histo-│ │                                                           │ │ │ • Basic / Light          │ │
│   ry    │ │                                                           │ │ │ • Color (HSL, Wheels)    │ │
│   DAG   │ │                                                           │ │ │ • Tone Curve (Spline)    │ │
│ • Local │ └───────────────────────────────────────────────────────────┘ │ │ • Detail (Sharpen/NR)    │ │
│   Tree  │ ┌───────────────────────────────────────────────────────────┐ │ │ • Optics & Geometry     │ │
│         │ │ VIEWPORT TOOLBAR (Fit, 1:1, Before/After Split, Zoom)     │ │ └──────────────────────────┘ │
├─────────┴─┴───────────────────────────────────────────────────────────┴─┴──────────────────────────────┤
│ BOTTOM FILMSTRIP DRAWER (110px)                                                                        │
│ [Rating ★★★☆☆] [Pick/Reject] [Thumb 1] [Thumb 2] [Active Thumb] [Thumb 4] ... [Counter: 1/542 RAWs]     │
└────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

### 0.1 Design Tokens & Surface Palette

*   **Window & Matte Neutral:**
    *   Application Shell Background: `#121212` (Ultra-deep neutral gray)
    *   Panel Surfaces (Left / Right): `#181818` (Dark charcoal)
    *   Section Headers & Accordion Cards: `#202020`
    *   Recessed Slider Tracks & Input Fields: `#282828`
    *   Canvas Viewport Matte: `#161616` (Neutral gray to prevent visual white-point drift)
*   **Dividers, Borders & Radii:**
    *   Subtle Panel Dividers: 1px solid `#2c2c2c`
    *   Component Corner Radii: Strict **3px to 4px** desktop standard (no rounded mobile cards)
*   **Semantic Accents & Indicators:**
    *   Interactive Accent / Selection Ring: Sky Blue `#2680eb`
    *   Pick Flag: Vibrant Emerald `#46a758`
    *   Reject Flag: Crimson `#e5484d`
    *   Shadow Clipping Overlay: Pure Blue `#2255ff` (Canvas HUD `J`)
    *   Highlight Clipping Overlay: Pure Red `#ff2222` (Canvas HUD `J`)
*   **Typography Hierarchy:**
    *   Interface Labels: `Inter`, `SF Pro Display`, or `Segoe UI Variable` (11px, Medium, `#a0a0a0`)
    *   Section Headings: 12px, Semi-Bold, `#d4d4d4`
    *   Numeric Slider Values: `JetBrains Mono` or `SF Mono` (11px, Regular, `#e0e0e0`) — **fixed-width tabular numbers prevent layout jitter during scrubbing.**

---

## Sprint 1: System Foundation, Native Ingestion & Professional Shell

**Primary Objective:** Replace crude placeholder boxes with the production tripartite darkroom shell; ingest real RAW/DNG files from disk via native dialogs; decode sensor data using LibRaw FFI.

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ SPRINT 1 ARCHITECTURE FLOW                                                   │
│ [Native File Dialog] ──► [libraw-sys FFI] ──► [Linear Float Texture Engine]  │
│                                           └──► [kamadak-exif HUD Display]    │
└──────────────────────────────────────────────────────────────────────────────┘
```

### 1.1 Tasks & Deliverables

*   [ ] **Tripartite Darkroom Window Shell (`halation-ui`):**
    *   Build the persistent desktop layout:
        *   Left Sidebar ($260\text{ px}$ fixed width, collapsible via `Tab`).
        *   Right Edit Inspector ($320\text{ px}$ fixed width, collapsible via `Shift+Tab`).
        *   Center Viewport with flexible layout sizing.
        *   Bottom Filmstrip ($110\text{ px}$ fixed height, collapsible via `/`).
    *   Implement sleek 1px panel borders (`#2c2c2c`) and collapse buttons with smooth transition state.
*   [ ] **Native File Dialog & Volume Discovery (`halation-ui` + `rfd`):**
    *   Add native Windows/macOS/Linux Open Folder and Open File actions (`Ctrl/Cmd+O`, `Ctrl/Cmd+Shift+I`).
    *   Filter for extensions: `.arw`, `.cr2`, `.cr3`, `.nef`, `.dng`, `.tif`, `.tiff`, `.jpg`, `.jpeg`.
    *   Compute volume UUIDs using platform hooks (`GetVolumeInformationW` on Windows, `statfs.f_fsid` on POSIX).
*   [ ] **RAW Decoding Pipeline Integration (`halation-raw`):**
    *   Bind `libraw-sys` (LibRaw 0.21+) as the baseline camera RAW demosaicing engine for proprietary CFA patterns.
    *   Extract unclipped 32-bit floating-point linear sensor pixels, active sensor area crop bounds, and camera matrix tags.
    *   Fallback pure-Rust decoding path via `image` and `zune-image` for uncompressed DNGs, TIFFs, and JPEGs.
*   [ ] **Live EXIF HUD Component (`halation-ui`):**
    *   Extract EXIF blocks via `kamadak-exif`: Camera Make/Model, Lens, Focal Length, Exposure Time, F-Number, ISO, and Capture Date.
    *   Render a high-density, professional HUD bar below the histogram:
        $$\text{\texttt{ISO 200 \textbar\ 50mm \textbar\ f/2.8 \textbar\ 1/500s \textbar\ Sony A7 IV}}$$
*   [ ] **SQLite WAL Storage Initialization (`halation-catalog`):**
    *   Initialize SQLite database with `PRAGMA journal_mode = WAL;` and `PRAGMA synchronous = NORMAL;`.
    *   Enforce schema: `assets (volume_uuid, relative_path, file_hash, EXIF)` with composite indices.

### 1.2 Definition of Done (DoD)
* Halation launches into a dark-themed (`#121212`), professional 3-panel shell.
* User clicks "Import", chooses a Sony `.ARW` or Nikon `.NEF` from an external drive, and the raw image renders directly in the center loupe.
* EXIF HUD populates with camera, lens, and exposure data.
* Relaunching the application shows the image cataloged in the SQLite database.

---

## Sprint 2: GPU Compute Pipeline & Shared Surface Presentation

**Primary Objective:** Migrate all image processing to a 32-bit floating-point WGSL compute pipeline running on a shared GPU device with Iced, achieving $< 4\text{ ms}$ frame execution.

### 2.1 Tasks & Deliverables

*   [ ] **Shared Device Architecture (`halation-gpu`):**
    *   Initialize a single `wgpu::Device` and `wgpu::Queue` shared across `halation-gpu` and `iced_wgpu`.
    *   Eliminate inter-device staging copies by using `iced::advanced::widget::shader::Program` to blit the compute texture view directly to the swapchain.
*   [ ] **Linear ProPhoto Working Space WGSL Pipeline (`assets/shaders/develop_pipeline.wgsl`):**
    *   Workgroup size $(16 \times 16)$ compute shader processing linear 32-bit float buffers.
    *   Implement Bradford matrix transform in LMS space for white balance.
    *   Implement linear exposure scaling: $C_{\text{exposed}} = C_{\text{in}} \cdot 2^{\text{exposure}}$.
    *   Implement log-domain highlight and shadow reconstruction using normalized ProPhoto RGB luminance weights:
        $$\mathbf{Y}_{\text{ProPhoto}} = [0.28804,\ 0.71187,\ 0.00009]$$
    *   Implement radiometric sigmoidal contrast pivoting around $18\%$ neutral gray in log space:
        $$C_{\text{contrast}} = \left(\frac{C_{\text{toned}}}{0.18}\right)^\gamma \cdot 0.18 \quad \text{where } \gamma = 1.0 + (\text{contrast} \cdot 0.005)$$
*   [ ] **Memory Layout Alignment (`halation-core`):**
    *   Align `GpuDevelopUniforms` to 48 bytes (multiple of 16 bytes for std140).
    *   Implement `GpuMat3x3` with 16-byte padded column vectors (`col0: [f32; 4]`, etc.).
*   [ ] **ParamTransaction State Management:**
    *   Implement atomic ring-buffer streaming (`crossbeam::queue::ArrayQueue`) from UI to GPU.
    *   Decouple ephemeral 120 FPS slider dragging from persistent catalog database commits.

### 2.2 Definition of Done (DoD)
* Dragging the Exposure slider updates the 45-megapixel viewport at 120 FPS on dedicated GPUs and $\ge 60\text{ FPS}$ on Intel Iris Xe / Apple M-series.
* CPU utilization remains $< 2\%$ during continuous slider dragging.
* Contrast adjustments do not invert deep shadows or crush highlights to black.

---

## Sprint 3: High-Performance Viewport & Precision Canvas Navigation

**Primary Objective:** Deliver precision darkroom navigation: buttery-smooth panning, continuous zoom, 1:1 pixel peeping, interactive Navigator mini-map, and split Before/After comparison.

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ VIEWPORT NAVIGATION STATE MACHINE                                            │
│ [Fit] ◄──(Click / 'Z')──► [1:1 100%] ◄──(Wheel)──► [Continuous 25%..800%]   │
│   │                              │                                           │
│   └── (Mini-Map Overview)        └── (Space + Drag / Middle Mouse Pan)       │
└──────────────────────────────────────────────────────────────────────────────┘
```

### 3.1 Tasks & Deliverables

*   [ ] **Viewport Canvas Engine (`halation-ui`):**
    *   Implement `Canvas` state machine: `ZoomMode::Fit`, `Fill`, `ActualSize (1:1)`, `Zoom(f32)`.
    *   `Z` key or single click toggles instantly between `Fit` and `1:1 (100%)`.
    *   Mouse wheel zooms continuously, centered mathematically on the cursor's normalized image coordinate.
*   [ ] **Pan & Kinetic Drag Navigation:**
    *   Holding `Space` or pressing middle-mouse activates the grab hand cursor (`CursorIcon::Grab` $\to$ `Grabbing`).
    *   Clamped translation vector $(\Delta x, \Delta y)$ prevents panning the image completely off-screen.
*   [ ] **Navigator Mini-Map Widget (`halation-ui`):**
    *   Render a persistent $180\text{ px}$ wide overview thumbnail in the top of the Left Panel.
    *   Draw an interactive red viewport boundary rectangle (`1.5px` border) showing the visible crop frustum.
    *   Clicking or dragging inside the mini-map repositions the main canvas loupe in real time.
*   [ ] **Before / After Comparison Engine (`\` shortcut):**
    *   Implement comparison modes:
        *   `Toggle`: Holding `\` shows unedited camera baseline; releasing restores edits.
        *   `SplitVertical`: Movable split-screen divider with a 2px high-contrast handle line.
        *   `SideBySide`: Two synchronized viewports panning and zooming in lockstep.

### 3.2 Definition of Done (DoD)
* Pressing `1:1` instantly magnifies a 45 MP image with zero hitching.
* Holding `Space` and dragging pans across native pixels at display refresh rate.
* Movable Before/After split bar shows active edits on the right and pristine RAW sensor data on the left.

---

## Sprint 4: Tactile Darkroom Widgets & Custom Iced Primitives

**Primary Objective:** Replace crude stock sliders with purpose-built darkroom controls: Scrubby Sliders with dynamic gradient tracks, an interactive 5-zone histogram, an analytical Tone Curve, and 3-way color wheels.

### 4.1 Tasks & Deliverables

```
SCRUBBY SLIDER SPECIFICATION:
┌─────────────────────────────────────────────────────────────────────────────┐
│ Exposure        ═══════════════════════●═══════════════════       +0.41 EV │
│ (11px Gray)     (2px Recessed Track, 12px Circular Thumb)   (Mono Readout)  │
└─────────────────────────────────────────────────────────────────────────────┘
```

*   [ ] **Scrubby Slider Implementation (`crates/halation-ui/src/widgets/scrubby_slider.rs`):**
    *   Custom `iced::advanced::Widget` combining Label (left), Track (center), and Editable Monospace Value (right).
    *   Click-dragging on the numeric readout locks the OS cursor and emits relative motion deltas ($\Delta x$).
    *   `Shift` modifier key drops drag sensitivity by $10\times$ for micro-adjustments.
    *   Double-clicking label, thumb, or text snaps the parameter to its calibrated default.
    *   **Dynamic Gradient Tracks:**
        *   Temperature slider: Linear gradient track from Deep Amber (`#ff9326`, $2000\text{ K}$) to Icy Blue (`#5eb5ff`, $50000\text{ K}$).
        *   Tint slider: Linear gradient from Green (`#30d158`) to Magenta (`#bf5af2`).
*   [ ] **Interactive 5-Zone Histogram (`crates/halation-ui/src/widgets/histogram.rs`):**
    *   Render real-time logarithmic 256-bin histogram: Red, Green, Blue, and combined Luminance curves.
    *   5 interactive zones: Blacks ($0–10\%$), Shadows ($10–35\%$), Exposure ($35–65\%$), Highlights ($65–90\%$), Whites ($90–100\%$).
    *   Hovering over a zone highlights it with an alpha-blended white overlay (`rgba(255, 255, 255, 0.08)`).
    *   Horizontal click-dragging inside a zone modulates the corresponding parameter directly.
    *   Clipping badges in top corners: Blue triangle (Shadows $< 0.0$), Red triangle (Highlights $> 1.0$); clicking toggles canvas clipping masks (`J`).
*   [ ] **Analytical Tone Curve Widget (`crates/halation-ui/src/widgets/tone_curve.rs`):**
    *   Monotone cubic spline interpolation (Fritsch-Carlson) evaluated without overshoot artifacts.
    *   Spline points uploaded as piecewise polynomial uniform structs ($a x^3 + b x^2 + c x + d$) to WGSL, completely eliminating the shadow-quantization bug of 256-entry linear LUTs.
    *   Channel selector tabs: RGB Composite (White), Red, Green, Blue.
    *   Visual grid lines at $25\%$, $50\%$, and $75\%$ tonal intersections.
*   [ ] **3-Way Color Wheels (`crates/halation-ui/src/widgets/color_wheel.rs`):**
    *   Polar color wheels for Shadows, Midtones, and Highlights.
    *   Angle maps to Hue ($0^\circ–360^\circ$); radial distance maps to Saturation ($0.0–1.0$).
    *   Central puck with auto-snap anchor dot; vertical slider beside each wheel to modulate Luminance.

### 4.2 Definition of Done (DoD)
* Double-clicking any slider thumb or label snaps it back to neutral calibrated default.
* Dragging inside the Exposure zone of the histogram dynamically updates the Exposure slider.
* The Tone Curve allows placing control points with zero spline ripple or shadow banding.

---

## Sprint 5: Deep Color Engine, Bradford CAT & Display Gamut Mapping

**Primary Objective:** Guarantee reference-grade color fidelity: automatic monitor ICC profile detection, Bradford chromatic adaptation, and 3D tetrahedral display LUT gamut mapping.

### 5.1 Tasks & Deliverables

*   [ ] **Platform Display Profile Discovery (`halation-color`):**
    *   **Windows:** Query `GetICMProfileW` on the current window `HDC`.
    *   **macOS:** Call `CGDisplayCopyColorSpace` via CoreGraphics/AppKit bindings.
    *   **Linux:** Parse X11 `_ICC_PROFILE` atom or query Wayland `color-management-v1`.
    *   Detect cross-monitor window migration; trigger dynamic recalibration.
*   [ ] **3D Tetrahedral Display LUT Generator (`halation-color` + `moxcms`):**
    *   Parse the monitor's ICC profile using `moxcms`.
    *   Generate a $33 \times 33 \times 33$ 3D LUT mapping Linear ProPhoto RGB directly to the calibrated monitor gamut and transfer function (sRGB / Display P3 / Rec.2020).
    *   Upload the table to a 3D float texture (`texture_3d<f32>`).
*   [ ] **Display Presentation Blit (`assets/shaders/display_blit.wgsl`):**
    *   Sample the 3D LUT using hardware trilinear/tetrahedral filtering in the final swapchain blit.
    *   Implement spatial blue-noise dithering to eliminate 8-bit banding on standard SDR panels.

### 5.2 Definition of Done (DoD)
* Moving the application window from an sRGB laptop screen to a wide-gamut Display P3 monitor dynamically updates the display LUT in $< 50\text{ ms}$.
* Deep saturated reds, greens, and skin tones match reference darkroom outputs with zero gamut clipping.

---

## Sprint 6: 100k Virtualized Catalog, SQLite WAL & Local Culling

**Primary Objective:** Build an industrial digital asset management system: virtualized infinite grid, multi-tier LRU thumbnail caching, and fast local assisted culling.

### 6.1 Tasks & Deliverables

*   [x] **Virtualized Grid & Filmstrip Widget (`halation-ui`):**
    *   Virtual scrolling grid rendering only visible items $+ 2$ row buffer.
    *   Keyboard navigation: Arrow keys to move selection, `1`–`5` for star ratings, `6`–`9` for color labels, `Z`/`X`/`U` for Pick/Reject/Unflag.
    *   Badges on thumbnails: Rating stars, pick flag icon, file format chip (`RAW`, `DNG`, `JPG`).
*   [ ] **Two-Tier LRU Thumbnail Cache Engine (`halation-pipeline`):**
    *   Tier 1 (Memory): 500-item LRU cache of decoded $256\text{ px}$ micro-thumbnails.
    *   Tier 2 (Disk): Fast WebP/LZ4 thumbnails stored in `.halation/cache/` keyed by `blake3` file hashes.
    *   Background Rayon workers asynchronously decode and downscale previews without dropping grid scroll frames.
    *   Current code uses a 500-entry memory LRU and WebP disk cache keyed by path, file size, and modification time; the full-content hash remains intentionally out of the hot thumbnail path.
*   [ ] **Local Assisted Culling Engine (`halation-pipeline`):**
    *   Local multi-threaded analysis pass (zero network requests):
        *   Subject Sharpness: High-frequency Laplacian gradient variance.
        *   Eye State / Blink Detection: Eye Aspect Ratio (EAR) metric identifying closed eyes.
        *   Technical Rejects: Detection of severe camera motion shake or blown highlights.
    *   UI badges: `Select` (Green) and `Reject` (Red) with real-time threshold filter sliders.
    *   Current code computes Laplacian sharpness and highlight clipping, persists those metrics in SQLite, and updates per-photo status badges from the live thresholds. Eye/blink and camera-motion detection remain unimplemented.
    *   The Library's `Filter Selects` toggle filters cards and keyboard navigation using the current sharpness and clipping thresholds.

### 6.2 Definition of Done (DoD)
* Smooth, locked 60–120 FPS scrolling across a catalog of 10,000+ RAW images.
* Pressing `Z` flags an image as "Pick" in memory and commits the change to SQLite in $< 2\text{ ms}$.
* Assisted Culling flags misfocused shots and blinks across 100 images locally in $< 5\text{ seconds}$.

---

## Sprint 7: Non-Destructive History DAG & Adobe-Compatible XMP Sync

**Primary Objective:** Deliver an infallible, branching undo/redo graph and bidirectional Adobe-compatible `.xmp` sidecar synchronization.

```
       [Node 1: Import Baseline]
                   │
                   ▼
       [Node 2: Exposure +0.50 EV]
                   │
         ┌─────────┴─────────┐ (Undo 1 step + modify parameter)
         ▼                   ▼
[Node 3: Contrast +20]   [Node 4: Temp 6500K] (Active Head Pointer)
         │
         ▼
[Node 5: B&W Conversion]
```

### 7.1 Tasks & Deliverables

*   [ ] **Branching History DAG Traversal (`halation-catalog`):**
    *   Implement `develop_history` with `parent_history_id` self-referencing hierarchy.
    *   Track active head via `develop_states.active_history_id`.
    *   Undoing 3 steps and moving a new slider spawns a new branch node, preserving all historical edits without truncation.
*   [ ] **History Panel UI Component (`halation-ui`):**
    *   Render an interactive vertical tree in the Left Panel.
    *   Format action labels with explicit parameter deltas:
        $$\text{\texttt{Exposure +0.50 EV \textbar\ Contrast -15 \textbar\ Tone Curve Red}}$$
    *   Clicking any historical node restores the viewport and slider controls to that exact parameter state.
*   [ ] **Bidirectional Adobe-Compatible XMP Sync Engine (`halation-catalog`):**
    *   Parse and serialize `.xmp` packets matching Adobe XMP Core (`crs:Exposure2012`, `crs:Temperature`, `crs:Tint`, `crs:Contrast2012`, etc.).
    *   Debounced asynchronous background write: $500\text{ ms}$ after the user stops moving sliders, write the parameter payload to the companion `.xmp` file on disk.

### 7.2 Definition of Done (DoD)
* Edits performed in Halation write an `.xmp` sidecar that opens with identical slider positions in Adobe Lightroom Classic.
* User can traverse 50 undo/redo steps across multiple branches with zero state corruption.

---

## Sprint 8: Decoupled On-Device AI Masking & High-Throughput Export

**Primary Objective:** Deliver hardware-accelerated local neural masking (Subject/Sky selection) via a decoupled two-stage inference model, along with a multi-threaded batch export queue.

### 8.1 Tasks & Deliverables

*   [ ] **Decoupled Two-Stage Neural Mask Engine (`halation-ml`):**
    *   Integrate `ort` (ONNX Runtime) with DirectML (Windows), CoreML (macOS), and CPU fallback (Linux).
    *   **Stage 1 (Background Embedding Worker):** Asynchronously runs MobileSAM quantized image encoder when an image is selected in the Loupe, producing an image embedding tensor $[1, 256, 64, 64]$.
    *   **Stage 2 (Interactive Prompt Decoder):** When user clicks "Select Subject" or "Select Sky", the prompt decoder evaluates the cached embedding in $< 15\text{ ms}$, outputting an 8-bit precision mask.
    *   Upload the mask directly to a `wgpu::TextureView` and bind it to the local adjustment shader pass.
*   [ ] **Multi-Threaded Batch Export Pipeline (`halation-pipeline`):**
    *   Background Rayon export engine decoupled from the UI thread.
    *   Export Dialog:
        *   Formats: JPEG (Quality 1–100%), TIFF (8/16-bit), PNG, WebP, Linear DNG.
        *   Resampling: High-quality Lanczos3 dimension scaling.
        *   Color Space Conversion: Embeds output ICC profiles (sRGB, Adobe RGB, Display P3, ProPhoto).
        *   Watermark: Vector text or transparent PNG graphic with sub-pixel anchoring.
    *   Header progress bar displaying export count, percentage, and estimated time remaining.
*   [ ] **External Editor Integration:**
    *   Configure external editor paths in Preferences (e.g., Affinity Photo, GIMP).
    *   "Open in External Editor" (`Ctrl/Cmd+Alt+E`) writes a 16-bit ProPhoto TIFF to disk, launches the external executable, and watches for file updates.

### 8.2 Definition of Done (DoD)
* Clicking "Select Sky" produces a pixel-accurate mask in $< 20\text{ ms}$ (using precomputed embeddings), allowing localized exposure adjustments to the clouds.
* Batch export of 50 high-resolution RAW files to sRGB JPEGs saturates CPU/GPU hardware without causing interface lag in the main window.
* Complete verification under an air-gapped environment confirms zero outbound network sockets.

---

## Master Sprint Milestone Matrix

| Sprint | Primary Focus | Key Subsystem | Deliverable Milestone |
| :---: | :--- | :--- | :--- |
| **1** | Ingestion & Shell | `halation-raw`, `halation-ui` | Professional 3-panel shell + real RAW decoding via LibRaw FFI |
| **2** | GPU Pipeline | `halation-gpu`, `assets/shaders` | 120 FPS WGSL compute pipeline with linear ProPhoto math |
| **3** | Navigation | `halation-ui` | Fit, 1:1 pixel peeping, pan hand, Before/After split screen |
| **4** | Custom Widgets | `halation-ui` (custom widgets) | Scrubby sliders, 5-zone histogram, analytical tone curve |
| **5** | Color Science | `halation-color` | OS display ICC detection + 3D tetrahedral display LUT |
| **6** | Catalog Scaling | `halation-catalog`, `halation-pipeline`| 100k photo virtualized grid + local assisted culling |
| **7** | History & XMP | `halation-core`, `halation-catalog` | Branching DAG undo/redo + Adobe-compatible XMP sidecar sync |
| **8** | AI & Batch Export | `halation-ml`, `halation-pipeline` | Decoupled on-device SAM masking + multi-threaded export |
