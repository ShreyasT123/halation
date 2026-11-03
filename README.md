# Halation

**A local-first photo editing desktop app, built with Rust for Windows.**

Halation brings a photo library, a non-destructive develop workflow, and a fast darkroom-inspired interface into one native application. It is an independent project in active development, with an emphasis on responsive controls, local files, and a design that stays out of the way of the image.

> **Project status: early development.** The application builds and runs, and its core library, editing, history, and export workflows are taking shape. Some planned features are incomplete; see [Current scope](#current-scope) before using it with important work.

## What works today

- **Photo library:** Import supported images into a local SQLite catalog. Browse a virtualized thumbnail grid and filmstrip, and organize images with star ratings, color labels, and pick/reject flags.
- **Develop workspace:** Adjust exposure, contrast, highlights, shadows, whites, blacks, white balance, tone curves, and color grading. View a histogram, compare the edit with the original, and navigate with fit and zoom controls.
- **Culling aids:** Review local sharpness and highlight-clipping metrics. Keep or filter to photos that meet the current thresholds.
- **Editing history:** Undo and redo edits, browse history entries, and branch from an earlier edit.
- **XMP sidecars:** Read and write a subset of common Adobe Camera Raw settings alongside an image.
- **Export:** Run background batch exports to JPEG, PNG, TIFF, or WebP.
- **Optional subject masking:** A MobileSAM point-prompt workflow can run locally with the matching ONNX model files installed. It is experimental and the model files are not included in the repository.
- **GPU with CPU fallback:** Halation can use its `wgpu` develop path when available and falls back to CPU processing.

## Current scope

Halation is currently **Windows-only**. The desktop binary explicitly rejects other operating systems.

The image decoder currently uses the `image` crate with JPEG, PNG, TIFF, and WebP support. Although the import dialog also lists camera RAW extensions, full RAW decoding and demosaicing are not implemented yet. Do not rely on the current build to open RAW originals.

XMP synchronization currently covers core develop values such as exposure, contrast, highlights, shadows, whites, blacks, temperature, and tint. It does not yet round-trip every Halation control or provide full Adobe XMP compatibility.

MobileSAM weights are not committed. The optional inference workflow expects `encoder.onnx` and `decoder.onnx` under `models/mobilesam/mobilesam-onnx-float/`; the model's companion `.data` files must remain beside their ONNX files. Model download and use are separate from the normal build.

The project is under active development. Treat it as a development build, keep backups of source photos, and verify exported files before using them in a production workflow.

## Build and run

### Requirements

- Windows 10 or 11, 64-bit
- Rust stable with the MSVC toolchain
- Git

Install Rust from [rustup.rs](https://rustup.rs/) and ensure the Visual Studio C++ build tools are installed.

### Run from source

```powershell
git clone https://github.com/ShreyasT123/halation.git
cd halation
cargo run --release -p halation-desktop
```

For a development compile check:

```powershell
cargo check --workspace
```

The optimized executable is written under:

```text
target\release\halation-desktop.exe
```

There is no packaged installer yet.

## Optional: MobileSAM model

The normal application build does not download model weights. To fetch and validate the model for the point-mask workflow, use the included script in a Python environment:

```powershell
py -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install requests onnx
python model.py
```

The script downloads the ONNX archive and extracts it beneath `models/mobilesam/`. The app expects the model files in `models/mobilesam/mobilesam-onnx-float/`. Model weights are ignored by Git and should not be committed.

## Local data

The catalog and thumbnail cache are stored in the current Windows user's local application data directory:

```text
%LOCALAPPDATA%\Halation\catalog.halation-cat
%LOCALAPPDATA%\Halation\cache\
```

Develop edits are also written to `.xmp` sidecars next to their source images. Keep the catalog and sidecars with your backups.

## Built with

- **Rust** workspace with separate crates for the desktop app, UI, catalog, image pipeline, color processing, GPU processing, and optional ML inference
- **Iced** for the native desktop interface
- **SQLite** for the local catalog and edit history
- **wgpu** for GPU processing where available
- **Rayon** and **Tokio** for parallel and background work
- **ONNX Runtime with DirectML** for the optional Windows MobileSAM path

## Repository layout

```text
apps/halation-desktop/   Windows desktop entry point
crates/halation-ui/      Application interface and interaction
crates/halation-catalog/ SQLite catalog, history, and XMP sidecars
crates/halation-core/    Shared edit parameters and image data
crates/halation-raw/     Image decoding and metadata
crates/halation-pipeline/ Develop processing, culling, thumbnails, and export
crates/halation-color/   Color transforms and grading
crates/halation-gpu/     GPU develop engine
crates/halation-ml/      Optional MobileSAM inference
assets/                  Shader and application assets
```

## Development roadmap

The project is being built in phases. The current work is focused on catalog performance and culling; broader RAW support, more complete XMP interoperability, and additional AI workflows remain in progress. The detailed design and implementation notes are available in [design.md](design.md), [requirements.md](requirements.md), and [implementation-plan.md](implementation-plan.md).

## Contributing

Bug reports and focused contributions are welcome. Please include your Windows version, GPU, steps to reproduce, and relevant logs when reporting a problem. For changes, run `cargo check --workspace` and describe the behavior you verified.

## License

No license file is included yet. Until a license is added, this repository is publicly viewable but reuse and redistribution are not granted.
