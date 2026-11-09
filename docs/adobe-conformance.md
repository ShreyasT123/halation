# Halation vs Adobe Lightroom Desktop v9.6 — Conformance Report

**Source specification:** `docs/Adobe Lightroom Desktop v9.6 Product Teardown and Functional Specification.md`
**Audit date:** 2026-11-09
**Build status:** `cargo check --workspace` passes (exit 0); 58 test functions defined.
**Method:** code inspection of the current workspace plus the feature matrix and panel sections of the Adobe document. No running-app visual comparison was performed.

## Verdict

Halation is a coherent **local-first desktop develop app**, but it is **not a full implementation of the Adobe teardown**.

Of the 79 feature-matrix rows in the Adobe document:

| Scope | Satisfied | Partial | Missing | N/A | Total |
|---|---:|---:|---:|---:|---:|
| Desktop / local core | 8 | 16 | 30 | 0 | 54 |
| Cloud, sharing, and AI/generative | 0 | 0 | 24 | 1 | 25 |
| **Overall** | **8** | **16** | **54** | **1** | **79** |

All eight fully satisfied rows and all sixteen partial rows belong to the local desktop core. The cloud/sharing/generative section is almost entirely absent, which is consistent with Halation’s local-first scope.

> The counts above follow the 79 rows of the Adobe feature matrix. The detailed tables below split some rows (for example, sharing flows and AI-credit policy) into finer sub-items, so their row totals are higher than the matrix counts.

**Status legend**

| Mark | Meaning |
|---|---|
| Satisfied | Implemented and reachable in the current UI |
| Partial | Implemented in some form, but materially narrower than the spec |
| Missing | Not implemented |
| N/A | Not applicable to a local-first product |

---

## 1. Desktop and local core

This section covers everything that is not cloud, sharing, or AI/generative. That includes the local library, develop tools, masking and retouching, presets/versions, export, platform behavior, and file formats.

### 1.1 Application shell and navigation

| Spec item | Status | Evidence / gap |
|---|---|---|
| Left/right panel workspace with filmstrip | Satisfied | `State` tracks `left_open`, `right_open`, `filmstrip_open`; toggled by `Tab`, `Shift+Tab`, and `/` in `halation-ui/src/lib.rs`. |
| Library / Develop separation | Satisfied | `Page::Library` and `Page::Develop`; the right Edit panel is only built for Develop (`right_panel`, `develop_page`). |
| Photo Grid, Square Grid, Detail | Partial | Virtualized photo grid and Detail/Develop view exist. There is no Square Grid and no grid/detail view switcher. |
| Filmstrip | Satisfied | `filmstrip` in `lib.rs`; slash key toggles it. |
| Full-screen viewing | Missing | No `F` binding and no window full-screen action. |
| Before/after comparison | Satisfied | Single, side-by-side, top/bottom, movable wipe, and hold-`\` peek in `viewport.rs` and `ViewMode`. |
| Zoom controls | Partial | Fit, Fill, 1:1, wheel zoom, and `Z` toggle exist. The documented `Cmd/Ctrl+=` and `Cmd/Ctrl+-` bindings are not implemented. |
| Window and Help menus | Missing | The app has a custom header bar, not an OS menu bar with File/Edit/Photo/View/Window/Help menus. |
| Preferences dialog | Missing | No preferences surface; catalog path, GPU fallback, and cache behavior are implicit. |

### 1.2 Library and organization

| Spec item | Status | Evidence / gap |
|---|---|---|
| Local library, in place | Satisfied | Import adds a catalog reference; originals are not copied. Edits live in the catalog and an XMP sidecar (`halation-catalog`, `xmp.rs`). |
| Open without import | Satisfied | “Open Folder…” scans a folder and adds references; originals stay in place. |
| Photo Grid | Satisfied | `grid.rs` virtualizes rows and loads thumbnails only for visible cells. |
| Filmstrip | Satisfied | Shared filmstrip used by Library and Develop. |
| Ratings and flags | Satisfied | `0–5`, `P` pick, `X` reject, `U` unflag; persisted in the `assets` table. |
| Color labels | Satisfied | `6–9` set red/yellow/green/blue; persisted as `color_label`. |
| Folders | Partial | A flat folder filter is built from asset parent directories. No album tree, no folder creation/rename/move, no “include subfolders.” |
| Albums and smart albums | Missing | No album model or album UI. Adobe’s desktop albums are cloud features, but no local equivalent exists. |
| Stacks | Missing | No stack model. Adobe documents stacks as Cloud-only. |
| Keywords | Missing | No keyword model, keyword panel, or `K` shortcut. |
| Facet search | Missing | No search bar and no facet syntax. |
| Filter and sort | Partial | Culling filters work (All/Select/Neutral/Reject). There is no sort menu and no rating/flag/label filter. `load_items` always orders by `id DESC`. |
| Capture-time editing | Missing | EXIF capture time is parsed into `RawMetadata` but cannot be edited. |
| Batch rename templates | Missing | No rename flow. |
| Assisted Culling | Partial | `culling.rs` computes focus, motion, and clipping metrics; threshold sliders and Select/Neutral/Reject filters work. No faces panel and no AI subject/face culling. |

### 1.3 Import and local storage

| Spec item | Status | Evidence / gap |
|---|---|---|
| Local import (files / folders) | Satisfied | File picker and folder scan feed `import_asset`; progress is reported through `ImportReport`. |
| Camera or card-reader import | Partial | A folder can be scanned, but there is no device/card detection and no review-for-import screen. |
| Duplicate detection | Satisfied | BLAKE3 content hash; an existing hash returns the existing asset id and follows moved files when the old path disappears. |
| Review-for-import screen | Missing | Import is immediate; there is no selection grid, “Previously Added” overlay, or Add to Album step. |
| Cache and storage preferences | Partial | A thumbnail cache exists; there is no cache-size control, smart-preview setting, storage-location picker, or NAS handling. |

### 1.4 Global editing

| Spec item | Status | Evidence / gap |
|---|---|---|
| Histogram and clipping overlay | Satisfied | Histogram bitmap and shadow/highlight clipping badges; `J` toggles the clipping overlay. |
| Drag-to-edit tone on the histogram | Missing | The histogram is display-only. |
| White balance | Partial | Temperature and tint sliders exist. No white-balance eyedropper and no `W` sampler. |
| Light: six tone sliders | Satisfied | Exposure, contrast, highlights, shadows, whites, blacks in `ParamId::TONE`. |
| Light: Auto Tone | Missing | No auto-tone analysis or `Shift+A`. |
| Light: parametric curve | Missing | Only point curves are implemented. |
| Light: point curve | Satisfied | RGB, Red, Green, and Blue channels with add/move/remove points and per-channel reset (`CurveEditor`, `ToneCurve`). |
| Light: curve presets / Refine Saturation | Missing | No Linear/Medium/Strong curve presets and no refine-saturation control. |
| Color: vibrance and saturation | Satisfied | Both are global sliders. |
| Color Mixer | Missing | No hue/saturation/luminance mixer and no targeted adjustment tool. |
| Point Color | Missing | No swatches, range controls, variance, or visualize-range mode. |
| Color Grading | Partial | Four wheels (shadows, midtones, highlights, global) with hue/saturation/luminance. No blending and no balance controls. |
| B&W conversion / channel mixer | Missing | No monochrome profile or B&W mixer. |
| Profiles | Missing | No profile browser, camera-matching profiles, creative profiles, amount slider, or profile import. |
| Effects | Missing | No texture, clarity, dehaze, vignette, grain, or lens blur controls. |
| Detail | Missing | No sharpening, luminance noise reduction, color noise reduction, or detail sliders. |
| Optics | Missing | No chromatic-aberration removal, lens corrections, lens-profile selection, distortion/vignetting correction, or defringe. |
| Geometry | Missing | No upright, transform sliders, or constrain-crop. |
| Crop and rotate | Missing | No crop, aspect ratio, straighten, overlay, flip, or rotate controls. The toolbar’s “Crop” button is disabled. |
| Presets | Missing | No preset model, preset browser, amount slider, or preset import/export. The toolbar’s “Presets” button is disabled. |
| Versions | Partial | In-memory snapshots can be added and applied, but they are not persisted and there are no Auto/Named version tabs. |
| Copy and paste edit settings | Missing | No copy/paste of parameter subsets. |
| Reset | Satisfied | Reset all, reset section, and reset individual parameter. |
| Show Original | Satisfied | Hold `\` to peek, plus split and wipe comparisons. |
| HDR and panorama merge | Missing | No merge workflow. |
| Edit in external editor | Partial | Any configured external editor can receive a 16-bit ProPhoto TIFF. There is no Photoshop-specific integration. |
| Content Credentials | Missing | No provenance or content-credential support. |

### 1.5 Masking and retouching (non-generative)

| Spec item | Status | Evidence / gap |
|---|---|---|
| Select Subject | Satisfied | MobileSAM point-prompt workflow with automatic foreground prompts and optional refinement clicks. |
| Select Sky | Satisfied | Sky prompt set in `halation-ml/src/mask.rs`, same MobileSAM path. |
| Select Background | Missing | No background mask. |
| Select Landscape | Missing | No landscape element segmentation. |
| Select Objects | Missing | No brush/rectangle object selection. |
| Select People | Missing | No person/body/face part masks. |
| Brush | Missing | No brush mask tool. |
| Linear gradient | Satisfied | Draggable linear mask with start/end points. |
| Radial gradient | Partial | Radial mask source exists, with a stored feather value, but the mask panel exposes no feather control. |
| Color / luminance / depth range | Missing | No range masks and no depth-map input. |
| Feather and Edge sliders | Missing | Radial stores `feather`, but no UI slider; no edge control. |
| Mask set operations | Partial | Invert is implemented. No add, subtract, intersect, duplicate-and-invert, or paste-to-selection. |
| Multiple masks per photo | Missing | `DevelopParameters.local` is a single `Option<LocalAdjustment>`, so only one mask can exist. |
| Mask overlay | Missing | No mask overlay color/opacity/unaffected-area modes or `O` cycling. |
| Local adjustment set | Partial | Exposure, contrast, highlights, shadows, saturation, and temperature. No whites, blacks, curves, tint, hue, colorize, texture, clarity, dehaze, grain, sharpness, noise reduction, moire, defringe, or refine saturation. |
| Clone / Heal | Missing | No clone or heal tool. |
| Remove tool (non-generative fill) | Missing | No remove tool, detect-objects, add/subtract area, size/overlay, or source-region movement. |
| Visualize Spots | Missing | No spot visualization or threshold control. |
| Remove People / Reflections / Dust | Missing | No distraction-removal tools. |

### 1.6 Presets, versions, and editing utilities

| Spec item | Status | Evidence / gap |
|---|---|---|
| Presets panel | Missing | No Recommended/Premium/Yours groups and no preset browser. |
| Preset amount, favorites, layout | Missing | No preset application model at all. |
| Create / update / export / delete preset | Missing | No preset CRUD or XMP preset export. |
| Versions | Partial | Snapshots are in-memory only; they are lost on restart. No Auto tab, no Named tab, no rename/delete/export-with-settings. |
| Undo / redo | Satisfied | `Ctrl+Z` / `Ctrl+Shift+Z`; persistent branching history in SQLite. |
| History panel | Satisfied (extra) | Adobe Desktop documents no History panel; Halation exposes one as a deliberate extra. |
| Copy Edit Settings | Missing | No `Ctrl+C` copy-settings flow. |
| Choose Edit Settings to Copy | Missing | No subset dialog. |
| Paste Edit Settings | Missing | No `Ctrl+V` paste-settings flow, and no paste-to-entire-selection. |
| Reset To Original / Reset To Open | Partial | Reset All returns to defaults. There is no distinction between “original” and “open” states. |
| Before/after modes | Satisfied | Show Original, side-by-side, top/bottom, and wipe. |

### 1.7 Output and export

| Spec item | Status | Evidence / gap |
|---|---|---|
| Custom export | Partial | JPEG, 8/16-bit PNG, 8/16-bit TIFF, WebP, and Linear DNG. No “Original” copy export, AVIF, or JPEG XL. |
| Export quality | Satisfied | JPEG quality slider 1–100. |
| Export resizing | Satisfied | Original, 4096, 2048, or 1080 long edge with Lanczos3 resampling. |
| Output color spaces | Satisfied | sRGB, Display P3, Adobe RGB, and ProPhoto RGB are selectable; profile embedding is supported by the export pipeline. |
| Export scope | Partial | Current photo or all photos in the current view. No selected-multiple or album scope. |
| Export progress | Satisfied | Progress bar, count, percentage, and ETA in the export panel. |
| Watermark | Partial | Logo/PNG watermark is exposed in the UI. Text watermark exists in `export.rs` (`WatermarkKind::Text`) but is not reachable from the export panel. |
| Watermark controls | Missing | No anchor, margin, scale, opacity, or offset controls in the UI. |
| Export presets | Missing | No JPG Small/Large/Original/Previous Settings shortcuts and no repeat-last export. |
| Export metadata controls | Missing | No PPI, metadata, or output-sharpening options. |
| External editor handoff | Satisfied | `ExternalEditor` receives a 16-bit ProPhoto TIFF; `Alt+E` opens it. |

### 1.8 Platform, performance, and preferences

| Spec item | Status | Evidence / gap |
|---|---|---|
| Windows desktop app | Satisfied | Windows-only; other operating systems are rejected. |
| GPU acceleration with CPU fallback | Satisfied | `wgpu` develop engine with `GpuDevelopEngine::new().ok()` fallback and `used_gpu` reporting. |
| GPU hardware gating | Missing | The app falls back silently; it does not detect or publish a VRAM floor or disable GPU-heavy features. |
| Cache and smart previews | Partial | Thumbnail cache exists. No user-facing cache size, smart preview, or storage-location controls. |
| Preferences | Missing | No Account/Cache/General/Import/Interface/Performance/Technology Preview dialog. |
| Display profile handling | Satisfied | Monitor profile discovery feeds a display LUT and is re-probed when the window moves. |
| Language support | Missing | UI is English-only; the Adobe doc lists 16 languages. |
| Minimum display / OS floor | Partial | Windows-only build. No enforced minimum Windows build, display size, or GPU requirement. |
| Accessibility | Missing | No documented focus order, screen-reader labels, or keyboard-only coverage beyond the shortcuts listed below. |

### 1.9 File formats and metadata

| Spec item | Status | Evidence / gap |
|---|---|---|
| JPEG / PNG / TIFF / WebP import | Satisfied | `halation-raw::decode_file` uses `image::open`. |
| Camera RAW import | Missing | `decode_file` uses `image::open`, so CR2/CR3/ARW/NEF/ORF/RW2 originals are not demosaiced. Thumbnails can fall back to the embedded JPEG preview, but Develop cannot open the RAW. The README explicitly states full RAW decoding is not implemented. |
| DNG import | Partial | TIFF-like/uncompressed DNG may open through `image`. Proprietary or compressed DNG is not guaranteed. |
| HEIC / AVIF import | Missing | Neither extension is in `SUPPORTED_EXTENSIONS`. |
| XMP sidecar read/write | Partial | Core develop values, point curves, four-way grading, rating, and flag are written. Local masks are stored only in `halation:ParamsJson`. No full Adobe XMP parity. |
| EXIF metadata display | Satisfied | Camera, lens, focal length, ISO, shutter, and aperture are read and shown. Capture date is parsed into `RawMetadata` but not shown in the HUD. |
| EXIF metadata editing | Missing | Read-only. |
| Catalog persistence | Satisfied | SQLite catalog at `%LOCALAPPDATA%\Halation\catalog.halation-cat`, with assets, develop states, and persistent history DAG. |
| Sidecar conflict handling | Partial | `xmp.rs` treats external `crs:` edits as authoritative when they disagree with `halation:ParamsJson`; unsupported local masks come from JSON. This is a deliberate subset, not full bidirectional parity. |

### 1.10 Keyboard shortcut coverage

| Shortcut | Status | Action |
|---|---|---|
| `Tab` / `Shift+Tab` | Satisfied | Toggle left / right panel |
| `/` | Satisfied | Toggle filmstrip |
| `←` / `→` | Satisfied | Previous / next photo |
| `↑` / `↓` | Satisfied | Move a grid row |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Satisfied | Undo / redo |
| `Ctrl+O` | Satisfied | Import dialog |
| `Z` | Satisfied | Toggle Fit / 1:1 |
| `J` | Satisfied | Toggle clipping overlay |
| `\` | Satisfied | Peek at original |
| `0–5` | Satisfied | Star rating |
| `6–9` | Satisfied | Color label |
| `P` / `X` / `U` | Satisfied | Pick / reject / unflag |
| `Alt+E` | Satisfied | Open in external editor |
| `F` / `Esc` | Missing | Full-screen |
| `C` | Missing | Crop |
| `B` / `L` / `R` | Missing | Brush / linear / radial gradient |
| `H` | Missing | Heal / remove |
| `A` | Missing | Visualize spots |
| `Shift+V` | Missing | Versions |
| `Shift+P` | Missing | Presets |
| `Ctrl+1–6`, `Ctrl+0` | Missing | Edit section / histogram focus |
| `Ctrl+=` / `Ctrl+-` | Missing | Zoom in / out |
| `Ctrl+[` / `Ctrl+]` | Missing | Rotate left / right |
| `Ctrl+C` / `Ctrl+V` | Missing | Copy / paste edit settings |
| `Shift+Ctrl+V` | Missing | Paste selected settings |
| `Shift+A` | Missing | Auto Tone |
| `Shift+E`, `Ctrl+E` | Missing | Export / export with previous |

---

## 2. Cloud, sharing, and AI/generative features

These items are separated because they depend on cloud services, sharing infrastructure, biometric analysis, or generative/partner AI. Their absence is consistent with Halation’s local-first design, but they are still part of the Adobe specification.

### 2.1 Cloud library and sync

| Spec item | Status | Evidence / gap |
|---|---|---|
| Cloud library views | Missing | No Cloud/All Photos/Recently Added/Recent Edits/By Date views. |
| Cloud Add Photos | Missing | No managed copy-in/upload workflow. |
| Copy to Cloud | Missing | No cloud copy or refresh-from-local flow. |
| Archive cloud photos to disk | Missing | No archive dialog or folder/date organization. |
| Offline albums | Missing | No offline album support. |
| Sync state display | Missing | No syncing/synced/originals-local statuses. |
| Sync conflicts, failures, quotas | Missing | No cloud transport at all. |
| Deleted view / restore | Missing | No deleted-items retention or restore flow. |
| People View | Missing | No opt-in People view, face clusters, names, merge, or cover photo. Adobe performs this server-side for cloud photos. |
| Improved Search (natural language) | Missing | No library indexing or natural-language search. |
| Connections (Portfolio, Blurb, WHCC) | Missing | No third-party publishing connections. |

### 2.2 Sharing and collaboration

| Spec item | Status | Evidence / gap |
|---|---|---|
| Share & Invite | Missing | No group albums, roles, links, or invitation flow. |
| Shared with You / Shared Albums / Shared Photos | Missing | No shared-library surfaces. |
| Album sharing / target album / contribute | Missing | No sharing model. |
| Share presets to mobile | Missing | No cross-client sync. |
| Subscription tiers and export limits | N/A | Halation has no subscription or metered export; all local features are available without an account. |

### 2.3 AI, enhancement, and generative

| Spec item | Status | Evidence / gap |
|---|---|---|
| Denoise (AI) | Missing | No neural denoise and no new-DNG output. |
| Raw Details | Missing | No Bayer/X-Trans detail enhancement. |
| Super Resolution | Missing | No 2× upscale. |
| Generative Upscale | Missing | No Topaz/partner upscale or credit flow. |
| AI Sharpen | Missing | No partner sharpen. |
| Generative Expand | Missing | No crop-based generative fill/expand. |
| Generate Video | Missing | No video generation. |
| Prompt to Edit | Missing | No prompt-driven editing. |
| Generative Remove | Missing | No remove tool or Firefly-billed variations. |
| Firefly / Firefly boards handoff | Missing | No Adobe ecosystem integration. |
| Metered AI credits / partner models | Missing | Halation’s only AI is on-device MobileSAM subject/sky selection, which has no metering. |
| Technology Preview switches | Missing | No preferences or preview-feature toggles. |

### 2.4 Note on on-device AI

Halation’s MobileSAM workflow is **not** the Adobe cloud AI stack. It is a local, on-demand segmentation model used for Select Subject and Select Sky. It is covered in section 1.5 because it provides a local masking primitive without cloud dependency. Adobe-style “AI is metered and partner-powered” assumptions from the teardown do not apply to it.

---

## 3. Where Halation diverges deliberately

These are not defects, but they explain why direct feature-count comparisons understate the app:

1. **Local-first storage instead of a managed cloud library.** Halation writes catalog state plus XMP sidecars and never uploads originals.
2. **Persistent branching history instead of Versions.** Adobe Desktop documents no History panel; Halation exposes one. The remaining gap is persistent named versions.
3. **On-device segmentation instead of cloud AI.** Subject/Sky work locally through MobileSAM; there is no credit system, partner model, or network dependency.
4. **Heuristic culling instead of cloud face analysis.** Focus, motion, and clipping thresholds are local and deterministic.
5. **A custom in-app shell instead of Adobe’s OS menu bar and Preferences dialog.** Functionality is reachable from the header and panels, but not through the same menu/shortcut paths.

---

## 4. Prioritized gaps for a core local Lightroom target

If the goal is parity with the **local desktop editing core** rather than the whole Adobe product, these are the highest-impact missing pieces:

1. **Crop, rotate, straighten, and geometry.** The most visible gap in a develop app; the toolbar currently exposes a disabled Crop button.
2. **Effects and Detail panels.** Texture, clarity, dehaze, vignette, grain, sharpening, and noise reduction.
3. **Optics.** Chromatic-aberration removal, lens profiles, distortion/vignetting correction, and defringe.
4. **Presets and persistent Versions.** A preset model, browser, amount slider, import/export, plus durable Auto/Named versions.
5. **Masking depth.** Brush, Select Background, range masks, multiple masks per photo, add/subtract/intersect, overlay, and feather/edge.
6. **Library organization.** Search, sort, albums, subfolders, keywords, and a view switcher (Grid / Square Grid / Detail).
7. **Export workflow.** Export presets, repeat-last, Original copy, AVIF/JPEG XL, text watermark, and watermark placement controls.
8. **Shell parity.** Full-screen, shortcut coverage, preferences, and a hardware/GPU information surface.
9. **File-format depth.** A LibRaw-style backend if camera RAW originals are a real target; otherwise keep the README’s current RAW warning prominent.

---

## 5. Bottom line

- The **local develop core is functional and internally coherent**: in-place catalog, XMP sidecars, global tone/color controls, point curves, four-way grading, subject/sky and gradient masks, persistent history, comparison modes, export, and GPU fallback.
- The app **does not satisfy “all” of the Adobe specification**. It satisfies 8 of 79 matrix rows fully and 16 partially; the remaining 55 are either absent (54) or not applicable (1), with 24 of the absent rows concentrated in cloud/sharing/AI.
- The RGB-curve corruption reported earlier did not reproduce in stored-curve, synthetic-patch, or direct CPU/GPU render tests, and the user has confirmed the current behavior is good. This report does not reopen that issue.
- The largest local gaps are **crop/rotate, Effects/Detail/Optics, presets/persistent versions, and masking depth**.
- The cloud/sharing/generative omissions are **expected for a local-first app** and should be treated as a separate product decision rather than regressions against the Adobe teardown.
