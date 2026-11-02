use halation_catalog::{Catalog, CatalogItem};
use halation_core::HistogramData;
use halation_core::{DevelopParameters, ImageBufferF32};
use halation_gpu::{GpuDevelopEngine, GpuEngineInfo};
use halation_ml::{Mask, MobileSam};
use halation_pipeline::{
    analyze_for_culling, export_batch, process_frame_with_gpu, CullingMetrics, ExportFormat,
    ExportJob, ThumbnailCache,
};
use halation_raw::{decode_file, RawAsset};
use iced::{
    event, keyboard, mouse,
    widget::{button, canvas, column, container, image, row, scrollable, slider, text, Space},
    Center, Color, ContentFit, Element, Event, Length, Point, Rectangle, Size, Subscription, Task,
    Theme,
};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
};
mod widgets;
use widgets::{
    histogram,
    scrubby_slider::{self, Gradient, Output as SliderOutput},
};

const PREVIEW_MAX: u32 = 1000;

pub fn run() -> iced::Result {
    iced::application("Halation | Photo Library", update, view)
        .theme(|_| Theme::Dark)
        .subscription(subscription)
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Library,
    Develop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomMode {
    Fit,
    Fill,
    Actual,
    Two,
    Four,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Single,
    SplitSideBySide,
    SplitTopBottom,
}

pub struct State {
    catalog: Catalog,
    items: Vec<CatalogItem>,
    thumbnail_cache: Arc<ThumbnailCache>,
    thumbnails: Vec<Option<image::Handle>>,
    thumbnail_requested: HashSet<usize>,
    culling_metrics: Vec<Option<CullingMetrics>>,
    exporting: bool,
    sharpness_threshold: f32,
    clipping_threshold: f32,
    ml: Arc<Mutex<Option<MobileSam>>>,
    mask_overlay: Option<image::Handle>,
    mask_tool: bool,
    mask_generation: u64,
    raw_assets: Vec<Option<RawAsset>>,
    selected: Option<usize>,
    page: Page,
    params: DevelopParameters,
    curve_points: [Vec<[f32; 2]>; 4],
    curve_channel: usize,
    wheel_values: [(f32, f32, f32); 3],
    clipping: [bool; 2],
    preview_dirty: bool,
    preview_scheduled: bool,
    loading_photo: bool,
    load_generation: u64,
    xmp_generation: u64,
    catalog_scroll_y: f32,
    catalog_view_height: f32,
    viewport: Option<image::Handle>,
    histogram: Option<image::Handle>,
    histogram_data: Option<HistogramData>,
    gpu: GpuEngineInfo,
    gpu_engine: Option<GpuDevelopEngine>,
    original_viewport: Option<image::Handle>,
    original_asset_index: Option<usize>,
    zoom: ZoomMode,
    view_mode: ViewMode,
    divider: f32,
    pan: [f32; 2],
    custom_zoom: f32,
    preview_dimensions: (u32, u32),
    status: String,
}

impl Default for State {
    fn default() -> Self {
        let catalog_path = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Halation")
            .join("catalog.halation-cat");
        if let Some(parent) = catalog_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let catalog = Catalog::open(&catalog_path)
            .or_else(|_| Catalog::open_in_memory())
            .expect("catalog initialization failed");
        let items = catalog.load_items();
        let count = items.len();
        let cache_root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Halation")
            .join("cache");
        let gpu_engine = GpuDevelopEngine::new().ok();
        let gpu = gpu_engine
            .as_ref()
            .map(|engine| engine.info().clone())
            .unwrap_or_else(GpuEngineInfo::fallback);
        Self {
            catalog,
            raw_assets: vec![None; count],
            items,
            thumbnail_cache: Arc::new(ThumbnailCache::new(cache_root)),
            thumbnails: vec![None; count],
            thumbnail_requested: HashSet::new(),
            culling_metrics: vec![None; count],
            exporting: false,
            sharpness_threshold: 0.001,
            clipping_threshold: 0.03,
            ml: Arc::new(Mutex::new(None)),
            mask_overlay: None,
            mask_tool: false,
            mask_generation: 0,
            selected: None,
            page: Page::Library,
            params: DevelopParameters::default(),
            curve_points: std::array::from_fn(|_| {
                vec![
                    [0.0, 0.0],
                    [0.25, 0.25],
                    [0.5, 0.5],
                    [0.75, 0.75],
                    [1.0, 1.0],
                ]
            }),
            curve_channel: 0,
            wheel_values: [(0.58, 0.0, 0.0); 3],
            clipping: [false; 2],
            preview_dirty: false,
            preview_scheduled: false,
            loading_photo: false,
            load_generation: 0,
            xmp_generation: 0,
            catalog_scroll_y: 0.0,
            catalog_view_height: 700.0,
            viewport: None,
            histogram: None,
            histogram_data: None,
            gpu,
            gpu_engine,
            original_viewport: None,
            original_asset_index: None,
            zoom: ZoomMode::Fit,
            view_mode: ViewMode::Single,
            divider: 0.5,
            pan: [0.5, 0.5],
            custom_zoom: 1.0,
            preview_dimensions: (0, 0),
            status: format!("{} photos in catalog", count),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Import,
    Imported(Vec<PathBuf>),
    ImportFinished(
        Vec<CatalogItem>,
        Vec<(u64, RawAsset, CullingMetrics)>,
        Vec<String>,
    ),
    Select(usize),
    Navigate(Page),
    Exposure(f32),
    Contrast(f32),
    Highlights(f32),
    Shadows(f32),
    Whites(f32),
    Blacks(f32),
    Temperature(f32),
    Tint(f32),
    Rating(u8),
    Flag(&'static str),
    Reset,
    Save,
    SetZoom(ZoomMode),
    ToggleZoom,
    SetViewMode(ViewMode),
    ToggleComparison,
    Divider(f32),
    KeyPressed(String),
    PanTo(f32, f32),
    PanBy(f32, f32),
    WheelZoom(f32),
    HistogramAdjust(histogram::Zone, f32),
    CurvePoint(usize, [f32; 2]),
    CurveChannel(usize),
    Wheel(usize, f32, f32),
    WheelLuma(usize, f32),
    ToggleClipping(usize),
    RefreshDisplay,
    RefreshPreview,
    PhotoDecoded(usize, u64, Result<(RawAsset, CullingMetrics), String>),
    CatalogScrolled(f32, f32),
    HistoryStep(bool),
    HistorySelect(i64),
    SyncXmp(u64),
    ThumbnailReady(usize, Result<Vec<u8>, String>),
    ExportAll(ExportFormat),
    ExportFinished(Result<Vec<Result<PathBuf, String>>, String>),
    SharpnessThreshold(f32),
    ClippingThreshold(f32),
    MaskTool(bool),
    MaskPoint([f32; 2]),
    MaskReady(usize, u64, u64, Result<Mask, String>),
    ClearMask,
}

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    let changed = matches!(
        &message,
        Message::Exposure(_)
            | Message::Contrast(_)
            | Message::Highlights(_)
            | Message::Shadows(_)
            | Message::Whites(_)
            | Message::Blacks(_)
            | Message::Temperature(_)
            | Message::Tint(_)
            | Message::CurvePoint(_, _)
            | Message::Wheel(_, _, _)
            | Message::WheelLuma(_, _)
            | Message::ToggleClipping(_)
            | Message::Reset
            | Message::HistogramAdjust(_, _)
    );
    match message {
        Message::Import => {
            let paths = rfd::FileDialog::new()
                .add_filter(
                    "Photos",
                    &[
                        "arw", "cr2", "cr3", "nef", "dng", "tif", "tiff", "jpg", "jpeg", "png",
                        "webp",
                    ],
                )
                .pick_files()
                .unwrap_or_default();
            return Task::done(Message::Imported(paths));
        }
        Message::Imported(paths) => {
            let catalog = state.catalog.clone();
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        let mut imported = Vec::new();
                        let mut errors = Vec::new();
                        for path in paths {
                            match decode_file(&path) {
                                Ok(mut raw) => {
                                    let metrics = analyze_for_culling(&raw);
                                    match catalog.import_asset(&path, &raw.metadata) {
                                        Ok(id) => {
                                            raw.id = id;
                                            imported.push((id, raw, metrics));
                                        }
                                        Err(error) => {
                                            errors.push(format!("{}: {error}", path.display()))
                                        }
                                    }
                                }
                                Err(error) => errors.push(format!("{}: {error}", path.display())),
                            }
                        }
                        (catalog.load_items(), imported, errors)
                    })
                    .await
                    .unwrap_or_else(|error| (Vec::new(), Vec::new(), vec![error.to_string()]))
                },
                |(items, imported, errors)| Message::ImportFinished(items, imported, errors),
            );
        }
        Message::ImportFinished(items, imported, errors) => {
            let imported_count = imported.len();
            state.items = items;
            state.raw_assets = vec![None; state.items.len()];
            state.thumbnails = vec![None; state.items.len()];
            state.culling_metrics = vec![None; state.items.len()];
            state.thumbnail_requested.clear();
            let mut latest = None;
            for (id, raw, metrics) in imported {
                if let Some(index) = state.items.iter().position(|item| item.id == id) {
                    state.raw_assets[index] = Some(raw);
                    state.culling_metrics[index] = Some(metrics);
                    latest = Some(index);
                }
            }
            state.status = if errors.is_empty() {
                format!("Imported {imported_count} photo(s)")
            } else {
                format!("Import completed; {} failed: {}", errors.len(), errors[0])
            };
            if let Some(index) = latest {
                select_photo(state, index);
            }
        }
        Message::Select(index) => return open_photo(state, index),
        Message::Navigate(page) => state.page = page,
        Message::CatalogScrolled(y, height) => {
            state.catalog_scroll_y = y;
            state.catalog_view_height = height;
            const ROW_HEIGHT: f32 = 78.0;
            const HEADER_HEIGHT: f32 = 48.0;
            let first = (((y - HEADER_HEIGHT).max(0.0) / ROW_HEIGHT) as usize).saturating_sub(2);
            let end = (first + (height / ROW_HEIGHT).ceil() as usize + 4).min(state.items.len());
            let mut tasks = Vec::new();
            for index in first..end {
                if state.thumbnails[index].is_none() && state.thumbnail_requested.insert(index) {
                    let path = state.items[index].path.clone();
                    let cache = state.thumbnail_cache.clone();
                    tasks.push(Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                cache.get_or_create(&path).map(|bytes| (*bytes).clone())
                            })
                            .await
                            .map_err(|error| error.to_string())
                            .and_then(|result| result)
                        },
                        move |result| Message::ThumbnailReady(index, result),
                    ));
                }
            }
            return Task::batch(tasks);
        }
        Message::HistoryStep(redo) => {
            if let Some(index) = state.selected {
                let id = state.items[index].id;
                let history = state.catalog.history(id);
                let active = state.catalog.active_history_id(id);
                let target = if redo {
                    history
                        .iter()
                        .find(|entry| entry.parent_id == active)
                        .map(|entry| entry.id)
                } else {
                    active.and_then(|active_id| {
                        history
                            .iter()
                            .find(|entry| entry.id == active_id)
                            .and_then(|entry| entry.parent_id)
                    })
                };
                if let Some(target) = target {
                    return Task::done(Message::HistorySelect(target));
                }
            }
        }
        Message::HistorySelect(history_id) => {
            if let Some(index) = state.selected {
                if let Some(params) = state
                    .catalog
                    .activate_history(state.items[index].id, history_id)
                {
                    state.items[index].params = params.clone();
                    sync_controls_from_params(state, params);
                    let _ = state
                        .catalog
                        .write_xmp_sidecar(state.items[index].id, &state.params);
                    state.preview_dirty = true;
                }
            }
        }
        Message::SyncXmp(generation) => {
            if generation == state.xmp_generation {
                if let Some(index) = state.selected {
                    let id = state.items[index].id;
                    state.catalog.save_parameters(id, &state.params);
                    let _ = state.catalog.write_xmp_sidecar(id, &state.params);
                    state.items[index].params = state.params.clone();
                }
            }
        }
        Message::ThumbnailReady(index, result) => {
            if let Some(slot) = state.thumbnails.get_mut(index) {
                if let Ok(bytes) = result {
                    *slot = Some(image::Handle::from_bytes(bytes));
                }
            }
        }
        Message::ExportAll(format) => {
            if state.exporting {
                return Task::none();
            }
            let Some(directory) = rfd::FileDialog::new().pick_folder() else {
                return Task::none();
            };
            let jobs: Vec<ExportJob> = state
                .items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let mut destination = directory.join(&item.filename);
                    destination.set_extension("jpg");
                    ExportJob {
                        source: item.path.clone(),
                        destination,
                        params: if state.selected == Some(index) {
                            state.params.clone()
                        } else {
                            item.params.clone()
                        },
                    }
                })
                .collect();
            state.exporting = true;
            state.status = format!("Exporting {} photos...", jobs.len());
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || export_batch(&jobs, format))
                        .await
                        .map_err(|error| error.to_string())
                },
                Message::ExportFinished,
            );
        }
        Message::ExportFinished(result) => {
            state.exporting = false;
            state.status = match result {
                Ok(results) => {
                    let succeeded = results.iter().filter(|result| result.is_ok()).count();
                    let failed = results.len() - succeeded;
                    if failed == 0 {
                        format!("Exported {succeeded} photos")
                    } else {
                        format!("Exported {succeeded}; {failed} failed")
                    }
                }
                Err(error) => format!("Export failed: {error}"),
            };
        }
        Message::SharpnessThreshold(value) => state.sharpness_threshold = value,
        Message::ClippingThreshold(value) => state.clipping_threshold = value,
        Message::MaskTool(enabled) => state.mask_tool = enabled,
        Message::MaskPoint(point) => {
            let Some(index) = state.selected else {
                return Task::none();
            };
            let path = state.items[index].path.clone();
            let ml = state.ml.clone();
            let model_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../models/mobilesam/mobilesam-onnx-float");
            state.status = "Preparing MobileSAM selection...".into();
            let generation = state.load_generation;
            state.mask_generation = state.mask_generation.wrapping_add(1);
            let mask_generation = state.mask_generation;
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        let mut slot = ml.lock().map_err(|e| e.to_string())?;
                        if slot.is_none() {
                            *slot = Some(MobileSam::load(&model_dir)?);
                        }
                        slot.as_mut()
                            .ok_or("MobileSAM did not initialize")?
                            .select_point(&path, point)
                    })
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|result| result)
                },
                move |result| Message::MaskReady(index, generation, mask_generation, result),
            );
        }
        Message::MaskReady(index, generation, mask_generation, result) => {
            if state.selected == Some(index)
                && state.load_generation == generation
                && state.mask_generation == mask_generation
            {
                match result {
                    Ok(mask) => match mask.overlay_rgba() {
                        Ok(rgba) => {
                            state.mask_overlay =
                                Some(image::Handle::from_rgba(mask.width, mask.height, rgba));
                            state.mask_tool = false;
                            state.status = "MobileSAM selection overlay ready".into();
                        }
                        Err(error) => state.status = format!("Could not display mask: {error}"),
                    },
                    Err(error) => state.status = format!("MobileSAM failed: {error}"),
                }
            }
        }
        Message::ClearMask => state.mask_overlay = None,
        Message::Exposure(v) => state.params.exposure = v,
        Message::Contrast(v) => state.params.contrast = v,
        Message::Highlights(v) => state.params.highlights = v,
        Message::Shadows(v) => state.params.shadows = v,
        Message::Whites(v) => state.params.whites = v,
        Message::Blacks(v) => state.params.blacks = v,
        Message::Temperature(v) => state.params.temperature = v,
        Message::Tint(v) => state.params.tint = v,
        Message::CurvePoint(index, point) => {
            let curve = &mut state.curve_points[state.curve_channel];
            if index > 0 && index + 1 < curve.len() {
                let left = curve[index - 1][0] + 0.005;
                let right = curve[index + 1][0] - 0.005;
                curve[index] = [point[0].clamp(left, right), point[1]];
                let p = curve[index];
                let offset = state.curve_channel * 5 + index;
                state.params.tone_curve[offset] = p;
            }
        }
        Message::CurveChannel(channel) => state.curve_channel = channel.min(3),
        Message::Wheel(index, hue, saturation) => {
            if let Some(value) = state.wheel_values.get_mut(index) {
                value.0 = hue;
                value.1 = saturation;
                state.params.color_grading[index][0] = hue;
                state.params.color_grading[index][1] = saturation;
            }
        }
        Message::WheelLuma(index, value) => {
            if let Some(wheel) = state.wheel_values.get_mut(index) {
                wheel.2 = value;
                state.params.color_grading[index][2] = value;
            }
        }
        Message::ToggleClipping(index) => {
            if let Some(mask) = state.clipping.get_mut(index) {
                *mask = !*mask;
            }
        }
        Message::RefreshDisplay => {
            refresh_display_profile(state);
        }
        Message::RefreshPreview => {
            state.preview_scheduled = false;
            if state.preview_dirty {
                state.preview_dirty = false;
                refresh(state);
            }
        }
        Message::PhotoDecoded(index, generation, result) => match result {
            Ok((raw, metrics)) => {
                state.raw_assets[index] = Some(raw);
                state.culling_metrics[index] = Some(metrics);
                if state.selected == Some(index) && state.load_generation == generation {
                    state.loading_photo = false;
                    state.status = format!("Opened {}", state.items[index].filename);
                    refresh(state);
                }
            }
            Err(error) => {
                if state.selected == Some(index) && state.load_generation == generation {
                    state.loading_photo = false;
                    state.status =
                        format!("Could not open {}: {error}", state.items[index].filename);
                }
            }
        },
        Message::Rating(rating) => {
            if let Some(i) = state.selected {
                state.catalog.update_rating(state.items[i].id, rating);
                state.items[i].rating = rating;
            }
        }
        Message::Flag(flag) => {
            if let Some(i) = state.selected {
                state.catalog.update_flag(state.items[i].id, flag);
                state.items[i].flag = flag.into();
            }
        }
        Message::Reset => {
            state.params = DevelopParameters::default();
            state.curve_points = std::array::from_fn(|_| {
                vec![
                    [0.0, 0.0],
                    [0.25, 0.25],
                    [0.5, 0.5],
                    [0.75, 0.75],
                    [1.0, 1.0],
                ]
            });
            state.wheel_values = [(0.58, 0.0, 0.0); 3];
            state.clipping = [false; 2];
            state.preview_dirty = true;
        }
        Message::Save => {
            if let Some(i) = state.selected {
                let id = state.items[i].id;
                if let Err(error) =
                    state
                        .catalog
                        .commit_history(id, "Develop settings", &state.params)
                {
                    state.status = format!("Could not save history: {error}");
                }
                if let Err(error) = state.catalog.write_xmp_sidecar(id, &state.params) {
                    state.status = format!("Saved settings; XMP sidecar failed: {error}");
                }
                state.items[i].params = state.params.clone();
                if !state.status.starts_with("Could not save")
                    && !state.status.starts_with("Saved settings;")
                {
                    state.status = "Develop settings saved to history and XMP".into();
                }
            }
        }
        Message::SetZoom(zoom) => {
            state.zoom = zoom;
            state.custom_zoom = 1.0;
            state.pan = [0.5, 0.5];
        }
        Message::ToggleZoom => {
            state.zoom = if state.zoom == ZoomMode::Fit {
                ZoomMode::Actual
            } else {
                ZoomMode::Fit
            };
        }
        Message::SetViewMode(mode) => state.view_mode = mode,
        Message::ToggleComparison => {
            state.view_mode = if state.view_mode == ViewMode::Single {
                ViewMode::SplitSideBySide
            } else {
                ViewMode::Single
            };
        }
        Message::Divider(value) => state.divider = value,
        Message::PanTo(x, y) => state.pan = [x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)],
        Message::PanBy(dx, dy) => {
            state.pan[0] = (state.pan[0] - dx).clamp(0.0, 1.0);
            state.pan[1] = (state.pan[1] - dy).clamp(0.0, 1.0);
        }
        Message::WheelZoom(delta) => {
            state.custom_zoom = (state.custom_zoom * (1.0 + delta * 0.12)).clamp(0.25, 4.0);
            state.zoom = ZoomMode::Custom;
        }
        Message::HistogramAdjust(zone, delta) => {
            match zone {
                histogram::Zone::Blacks => {
                    state.params.blacks = (state.params.blacks + delta).clamp(-100.0, 100.0)
                }
                histogram::Zone::Shadows => {
                    state.params.shadows = (state.params.shadows + delta).clamp(-100.0, 100.0)
                }
                histogram::Zone::Exposure => {
                    state.params.exposure = (state.params.exposure + delta).clamp(-5.0, 5.0)
                }
                histogram::Zone::Highlights => {
                    state.params.highlights = (state.params.highlights + delta).clamp(-100.0, 100.0)
                }
                histogram::Zone::Whites => {
                    state.params.whites = (state.params.whites + delta).clamp(-100.0, 100.0)
                }
            }
            state.preview_dirty = true;
        }
        Message::KeyPressed(key) if state.page == Page::Library => {
            if let Some(index) = state.selected {
                match key.as_str() {
                    "ArrowDown" | "ArrowRight" => {
                        state.selected = Some((index + 1).min(state.items.len().saturating_sub(1)))
                    }
                    "ArrowUp" | "ArrowLeft" => state.selected = Some(index.saturating_sub(1)),
                    "1" | "2" | "3" | "4" | "5" => {
                        let rating = key.parse::<u8>().unwrap_or(0);
                        state.catalog.update_rating(state.items[index].id, rating);
                        state.items[index].rating = rating;
                    }
                    "6" | "7" | "8" | "9" => {
                        let label = match key.as_str() {
                            "6" => "Red",
                            "7" => "Yellow",
                            "8" => "Green",
                            _ => "Blue",
                        };
                        state
                            .catalog
                            .update_color_label(state.items[index].id, label);
                        state.items[index].color_label = label.into();
                    }
                    "z" | "Z" => state.items[index].flag = "Pick".into(),
                    "x" | "X" => state.items[index].flag = "Reject".into(),
                    "u" | "U" => state.items[index].flag = "None".into(),
                    _ => {}
                }
                if matches!(key.as_str(), "z" | "Z" | "x" | "X" | "u" | "U") {
                    let flag = state.items[index].flag.clone();
                    state.catalog.update_flag(state.items[index].id, &flag);
                }
            }
        }
        Message::KeyPressed(key) => match key.as_str() {
            "z" | "Z" => {
                state.zoom = if state.zoom == ZoomMode::Fit {
                    ZoomMode::Actual
                } else {
                    ZoomMode::Fit
                }
            }
            "\\" => {
                state.view_mode = if state.view_mode == ViewMode::Single {
                    ViewMode::SplitSideBySide
                } else {
                    ViewMode::Single
                }
            }
            "1" => state.zoom = ZoomMode::Fit,
            "2" => state.zoom = ZoomMode::Fill,
            "3" => state.zoom = ZoomMode::Actual,
            "4" => state.zoom = ZoomMode::Two,
            "5" => state.zoom = ZoomMode::Four,
            "j" | "J" => {
                state.clipping[0] = !state.clipping[0];
                state.clipping[1] = !state.clipping[1];
                state.preview_dirty = true;
            }
            _ => {}
        },
    }
    if changed {
        state.preview_dirty = true;
    }
    let mut pending = Vec::new();
    if changed {
        state.xmp_generation = state.xmp_generation.wrapping_add(1);
        let generation = state.xmp_generation;
        pending.push(Task::perform(
            async move {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                generation
            },
            Message::SyncXmp,
        ));
    }
    if state.preview_dirty && !state.preview_scheduled {
        state.preview_scheduled = true;
        pending.push(Task::perform(
            async { tokio::time::sleep(std::time::Duration::from_millis(90)).await },
            |_| Message::RefreshPreview,
        ));
    }
    Task::batch(pending)
}

fn select_photo(state: &mut State, index: usize) {
    if index >= state.items.len() {
        return;
    }
    state.selected = Some(index);
    state.loading_photo = false;
    state.load_generation = state.load_generation.wrapping_add(1);
    state.pan = [0.5, 0.5];
    state.custom_zoom = 1.0;
    if state.original_asset_index != Some(index) {
        state.original_viewport = None;
        state.original_asset_index = None;
    }
    state.params = state.items[index].params.clone();
    if state.params.tone_curve.len() == 20 {
        for channel in 0..4 {
            state.curve_points[channel] =
                state.params.tone_curve[channel * 5..channel * 5 + 5].to_vec();
        }
    }
    for channel in 0..3 {
        let grade = state.params.color_grading[channel];
        state.wheel_values[channel] = (grade[0], grade[1], grade[2]);
    }
    if state.raw_assets[index].is_none() {
        match decode_file(&state.items[index].path) {
            Ok(raw) => state.raw_assets[index] = Some(raw),
            Err(e) => {
                state.status = format!("Could not open {}: {e}", state.items[index].filename);
                return;
            }
        }
    }
    refresh(state);
    state.page = Page::Develop;
}

fn sync_controls_from_params(state: &mut State, params: DevelopParameters) {
    state.params = params;
    if state.params.tone_curve.len() == 20 {
        for channel in 0..4 {
            state.curve_points[channel] =
                state.params.tone_curve[channel * 5..channel * 5 + 5].to_vec();
        }
    }
    for channel in 0..3 {
        let grade = state.params.color_grading[channel];
        state.wheel_values[channel] = (grade[0], grade[1], grade[2]);
    }
}

fn open_photo(state: &mut State, index: usize) -> Task<Message> {
    if index >= state.items.len() {
        return Task::none();
    }
    prepare_selected_photo(state, index);
    if state.raw_assets[index].is_some() {
        state.loading_photo = false;
        refresh(state);
        state.page = Page::Develop;
        return Task::none();
    }
    state.loading_photo = true;
    state.viewport = None;
    state.original_viewport = None;
    state.original_asset_index = None;
    state.histogram = None;
    state.histogram_data = None;
    state.status = format!("Opening {}...", state.items[index].filename);
    let generation = state.load_generation;
    let path = state.items[index].path.clone();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let raw = decode_file(&path).map_err(|e| e.to_string())?;
                let metrics = analyze_for_culling(&raw);
                Ok::<_, String>((raw, metrics))
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|result| result)
        },
        move |result| Message::PhotoDecoded(index, generation, result),
    )
}

fn prepare_selected_photo(state: &mut State, index: usize) {
    state.selected = Some(index);
    state.mask_overlay = None;
    state.load_generation = state.load_generation.wrapping_add(1);
    state.pan = [0.5, 0.5];
    state.custom_zoom = 1.0;
    if state.original_asset_index != Some(index) {
        state.original_viewport = None;
        state.original_asset_index = None;
    }
    state.params = state.items[index].params.clone();
    if let Ok(external) = Catalog::read_xmp_sidecar(&state.items[index].path) {
        state.params.exposure = external.exposure;
        state.params.contrast = external.contrast;
        state.params.highlights = external.highlights;
        state.params.shadows = external.shadows;
        state.params.whites = external.whites;
        state.params.blacks = external.blacks;
        state.params.temperature = external.temperature;
        state.params.tint = external.tint;
    }
    if state.params.tone_curve.len() == 20 {
        for channel in 0..4 {
            state.curve_points[channel] =
                state.params.tone_curve[channel * 5..channel * 5 + 5].to_vec();
        }
    }
    for channel in 0..3 {
        let grade = state.params.color_grading[channel];
        state.wheel_values[channel] = (grade[0], grade[1], grade[2]);
    }
    state.page = Page::Develop;
}

fn refresh_display_profile(state: &mut State) {
    let updated = state
        .gpu_engine
        .as_mut()
        .is_some_and(GpuDevelopEngine::refresh_display_profile);
    if updated {
        state.gpu = state.gpu_engine.as_ref().unwrap().info().clone();
        refresh(state);
    }
}

fn refresh(state: &mut State) {
    let Some(index) = state.selected else { return };
    let Some(raw) = state.raw_assets[index].as_ref() else {
        return;
    };
    let scale = (PREVIEW_MAX as f32 / raw.buffer.width.max(raw.buffer.height) as f32).min(1.0);
    let w = (raw.buffer.width as f32 * scale).round().max(1.0) as u32;
    let h = (raw.buffer.height as f32 * scale).round().max(1.0) as u32;
    state.preview_dimensions = (w, h);
    // Decode at source resolution, then sample a bounded viewport buffer for responsive first-pass editing.
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let sx = x * raw.buffer.width / w;
            let sy = y * raw.buffer.height / h;
            pixels.push(raw.buffer.pixels[(sy * raw.buffer.width + sx) as usize]);
        }
    }
    let preview = RawAsset {
        id: raw.id,
        filename: raw.filename.clone(),
        metadata: raw.metadata.clone(),
        buffer: ImageBufferF32 {
            width: w,
            height: h,
            pixels,
        },
    };
    if state.original_asset_index != Some(index) {
        let original = process_frame_with_gpu(
            &preview,
            &DevelopParameters::default(),
            state.gpu_engine.as_ref(),
        );
        state.original_viewport = Some(image::Handle::from_rgba(w, h, original.display_rgba));
        state.original_asset_index = Some(index);
    }
    let output = process_frame_with_gpu(&preview, &state.params, state.gpu_engine.as_ref());
    let gpu_used = output.gpu_used;
    let mut developed = output.display_rgba;
    if gpu_used {
        apply_clipping_mask(&mut developed, state.clipping);
    } else {
        apply_curve_and_grading(
            &mut developed,
            &state.curve_points,
            &state.wheel_values,
            state.clipping,
        );
    }
    state.viewport = Some(image::Handle::from_rgba(w, h, developed));
    state.histogram = Some(image::Handle::from_rgba(256, 64, output.histogram_rgba));
    state.histogram_data = Some(output.histogram);
}

fn apply_curve_and_grading(
    rgba: &mut [u8],
    curves: &[Vec<[f32; 2]>; 4],
    wheels: &[(f32, f32, f32); 3],
    clipping: [bool; 2],
) {
    let prepared: [_; 4] = std::array::from_fn(|i| PreparedCurve::new(&curves[i]));
    let wheel_tints: [[f32; 3]; 3] = std::array::from_fn(|i| {
        let a = wheels[i].0 * std::f32::consts::TAU;
        [a.cos(), (a - 2.094).cos(), (a + 2.094).cos()]
    });
    for p in rgba.chunks_exact_mut(4) {
        let rgb = [
            p[0] as f32 / 255.0,
            p[1] as f32 / 255.0,
            p[2] as f32 / 255.0,
        ];
        let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        let weights = [
            (1.0 - luma / 0.42).clamp(0.0, 1.0),
            (1.0 - ((luma - 0.5).abs() / 0.38)).clamp(0.0, 1.0),
            ((luma - 0.58) / 0.42).clamp(0.0, 1.0),
        ];
        let mut out = rgb;
        for c in 0..3 {
            out[c] = prepared[c + 1].sample(rgb[c]);
        }
        for c in 0..3 {
            out[c] = prepared[0].sample(out[c]);
        }
        for (wi, (_, sat, lum)) in wheels.iter().enumerate() {
            for c in 0..3 {
                out[c] += wheel_tints[wi][c] * sat * weights[wi] * 0.11 + lum * weights[wi] * 0.12;
            }
        }
        let final_luma = 0.2126 * out[0] + 0.7152 * out[1] + 0.0722 * out[2];
        for c in 0..3 {
            p[c] = (out[c].clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        if (clipping[0] && final_luma < 0.015) || (clipping[1] && final_luma > 0.985) {
            p[0] = if final_luma < 0.015 { 40 } else { 255 };
            p[1] = if final_luma < 0.015 { 120 } else { 35 };
            p[2] = if final_luma < 0.015 { 255 } else { 25 };
        }
    }
}

fn apply_clipping_mask(rgba: &mut [u8], clipping: [bool; 2]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let luma = 0.2126 * pixel[0] as f32 + 0.7152 * pixel[1] as f32 + 0.0722 * pixel[2] as f32;
        let low = clipping[0] && luma < 4.0;
        let high = clipping[1] && luma > 251.0;
        if low || high {
            pixel[0] = if low { 40 } else { 255 };
            pixel[1] = if low { 120 } else { 35 };
            pixel[2] = if low { 255 } else { 25 };
        }
    }
}

struct PreparedCurve<'a> {
    points: &'a [[f32; 2]],
    tangent: Vec<f32>,
}
impl<'a> PreparedCurve<'a> {
    fn new(points: &'a [[f32; 2]]) -> Self {
        if points.len() < 2 {
            return Self {
                points,
                tangent: vec![],
            };
        }
        let n = points.len();
        let slopes: Vec<f32> = points
            .windows(2)
            .map(|p| (p[1][1] - p[0][1]) / (p[1][0] - p[0][0]).max(1e-5))
            .collect();
        let mut tangent = vec![0.0; n];
        tangent[0] = slopes[0];
        tangent[n - 1] = slopes[n - 2];
        for i in 1..n - 1 {
            tangent[i] = if slopes[i - 1] * slopes[i] <= 0.0 {
                0.0
            } else {
                2.0 * slopes[i - 1] * slopes[i] / (slopes[i - 1] + slopes[i])
            };
        }
        Self { points, tangent }
    }
    fn sample(&self, x: f32) -> f32 {
        if self.points.len() < 2 {
            return x;
        }
        let n = self.points.len();
        let i = self
            .points
            .windows(2)
            .position(|p| x <= p[1][0])
            .unwrap_or(n - 2);
        let (x0, y0) = (self.points[i][0], self.points[i][1]);
        let (x1, y1) = (self.points[i + 1][0], self.points[i + 1][1]);
        let h = (x1 - x0).max(1e-5);
        let t = ((x - x0) / h).clamp(0.0, 1.0);
        let t2 = t * t;
        let t3 = t2 * t;
        ((2.0 * t3 - 3.0 * t2 + 1.0) * y0
            + (t3 - 2.0 * t2 + t) * h * self.tangent[i]
            + (-2.0 * t3 + 3.0 * t2) * y1
            + (t3 - t2) * h * self.tangent[i + 1])
            .clamp(0.0, 1.0)
    }
}

pub fn view(state: &State) -> Element<'_, Message> {
    let nav = row![
        text("HALATION").size(17),
        Space::with_width(Length::Fixed(28.0)),
        button(text("Library")).on_press(Message::Navigate(Page::Library)),
        button(text("Develop")).on_press(Message::Navigate(Page::Develop)),
        button(text("Undo")).on_press(Message::HistoryStep(false)),
        button(text("Redo")).on_press(Message::HistoryStep(true)),
        button(text(if state.exporting {
            "Exporting..."
        } else {
            "JPEG"
        }))
        .on_press(Message::ExportAll(ExportFormat::Jpeg)),
        button(text("PNG")).on_press(Message::ExportAll(ExportFormat::Png)),
        button(text("WebP")).on_press(Message::ExportAll(ExportFormat::WebP)),
        button(text("TIFF")).on_press(Message::ExportAll(ExportFormat::Tiff)),
        Space::with_width(Length::Fill),
        text(format!(
            "{} photos  ·  {}  ·  {}",
            state.items.len(),
            state.gpu.adapter_name,
            state.gpu.display_profile
        ))
        .size(11),
        button(text("Import Photos...")).on_press(Message::Import)
    ]
    .spacing(12)
    .align_y(Center);
    let content = match state.page {
        Page::Library => library_page(state),
        Page::Develop => develop_page(state),
    };
    column![
        container(nav).padding([12, 18]).width(Length::Fill),
        content,
        container(text(&state.status).size(11))
            .padding([7, 18])
            .width(Length::Fill)
    ]
    .into()
}

fn subscription(_state: &State) -> Subscription<Message> {
    let events = event::listen_with(|event, status, _window| {
        if status == event::Status::Captured {
            return None;
        }
        match event {
            Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => match key.as_ref() {
                keyboard::Key::Named(keyboard::key::Named::ArrowUp) => {
                    Some(Message::KeyPressed("ArrowUp".into()))
                }
                keyboard::Key::Named(keyboard::key::Named::ArrowDown) => {
                    Some(Message::KeyPressed("ArrowDown".into()))
                }
                keyboard::Key::Named(keyboard::key::Named::ArrowLeft) => {
                    Some(Message::KeyPressed("ArrowLeft".into()))
                }
                keyboard::Key::Named(keyboard::key::Named::ArrowRight) => {
                    Some(Message::KeyPressed("ArrowRight".into()))
                }
                keyboard::Key::Character(value)
                    if [
                        "z",
                        "Z",
                        "j",
                        "J",
                        "\\",
                        "1",
                        "2",
                        "3",
                        "4",
                        "5",
                        "6",
                        "7",
                        "8",
                        "9",
                        "x",
                        "X",
                        "u",
                        "U",
                        "ArrowUp",
                        "ArrowDown",
                        "ArrowLeft",
                        "ArrowRight",
                    ]
                    .contains(&value) =>
                {
                    Some(Message::KeyPressed(value.to_owned()))
                }
                _ => None,
            },
            Event::Window(iced::window::Event::Moved(_)) => Some(Message::RefreshDisplay),
            _ => None,
        }
    });
    Subscription::batch([
        events,
        iced::time::every(std::time::Duration::from_secs(2)).map(|_| Message::RefreshDisplay),
    ])
}

fn library_page(state: &State) -> Element<'_, Message> {
    let mut grid = column![
        text("PHOTO LIBRARY").size(13),
        text("Your catalog is stored locally and keeps original files untouched.").size(11)
    ]
    .spacing(10);
    if state.items.is_empty() {
        grid = grid
            .push(Space::with_height(Length::Fixed(60.0)))
            .push(text("No photos yet").size(24))
            .push(text("Import JPEG, PNG, TIFF, WebP, or DNG photos to begin.").size(13))
            .push(button(text("Import Photos")).on_press(Message::Import));
    } else {
        const ROW_HEIGHT: f32 = 82.0;
        const HEADER_HEIGHT: f32 = 48.0;
        let first = (((state.catalog_scroll_y - HEADER_HEIGHT).max(0.0) / ROW_HEIGHT) as usize)
            .saturating_sub(2);
        let visible = (state.catalog_view_height / ROW_HEIGHT).ceil() as usize + 4;
        let end = (first + visible).min(state.items.len());
        if first > 0 {
            grid = grid.push(Space::with_height(Length::Fixed(first as f32 * ROW_HEIGHT)));
        }
        for idx in first..end {
            let item = &state.items[idx];
            let preview: Element<'_, Message> = if let Some(handle) = &state.thumbnails[idx] {
                image(handle.clone())
                    .content_fit(ContentFit::Contain)
                    .width(Length::Fixed(116.0))
                    .height(Length::Fixed(72.0))
                    .into()
            } else {
                container(text("Preview").size(10))
                    .width(Length::Fixed(116.0))
                    .height(Length::Fixed(72.0))
                    .center(Length::Fill)
                    .into()
            };
            grid = grid.push(
                row![
                    preview,
                    button(
                        text(format!(
                            "{}    {}  ·  {} MB  ·  {}/5  {}  {}",
                            if state.selected == Some(idx) {
                                "●"
                            } else {
                                "○"
                            },
                            item.filename,
                            item.file_size / 1_000_000,
                            item.rating,
                            item.flag,
                            item.color_label
                        ))
                        .size(13),
                    )
                    .width(Length::Fill)
                    .height(Length::Fixed(72.0))
                    .on_press(Message::Select(idx))
                ]
                .spacing(12)
                .height(Length::Fixed(78.0)),
            );
        }
        if end < state.items.len() {
            grid = grid.push(Space::with_height(Length::Fixed(
                (state.items.len() - end) as f32 * ROW_HEIGHT,
            )));
        }
    }
    container(scrollable(grid.spacing(2)).on_scroll(|viewport| {
        let offset = viewport.absolute_offset();
        Message::CatalogScrolled(offset.y, viewport.bounds().height)
    }))
    .padding(24)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn develop_page(state: &State) -> Element<'_, Message> {
    let Some(index) = state.selected else {
        return library_page(state);
    };
    let item = &state.items[index];
    if state.loading_photo {
        return container(
            column![
                text("OPENING PHOTO").size(12),
                text(&item.filename).size(21),
                text("Decoding the image and preparing the develop preview…").size(13),
            ]
            .spacing(16)
            .align_x(Center),
        )
        .center(Length::Fill)
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    }
    if state.viewport.is_none() {
        return container(
            column![
                text("PHOTO COULD NOT BE OPENED").size(12),
                text(&item.filename).size(19),
                text(&state.status).size(13),
                button(text("Back to Library")).on_press(Message::Navigate(Page::Library)),
            ]
            .spacing(12)
            .align_x(Center),
        )
        .center(Length::Fill)
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    }
    let raw = state.raw_assets[index].as_ref();
    let meta = raw.map(|r| &r.metadata);
    let metadata = meta
        .map(|m| {
            format!(
                "{}   {}   ISO {}   {}   {}",
                empty_dash(&m.camera),
                empty_dash(&m.lens),
                m.iso.map(|v| v.to_string()).unwrap_or_else(|| "—".into()),
                empty_dash(&m.shutter),
                empty_dash(&m.aperture)
            )
        })
        .unwrap_or_default();
    let culling = state.culling_metrics[index]
        .map(|metrics| {
            format!(
                "Sharpness {:.3}  ·  clipped {:.2}%",
                metrics.sharpness,
                metrics.clipped_highlights * 100.0
            )
        })
        .unwrap_or_else(|| "Culling analysis pending".into());
    let review_status = state.culling_metrics[index]
        .map(|metrics| {
            if metrics.sharpness < state.sharpness_threshold
                || metrics.clipped_highlights > state.clipping_threshold
            {
                "REVIEW"
            } else {
                "TECHNICALLY OK"
            }
        })
        .unwrap_or("ANALYSIS PENDING");
    let mut controls = column![
        text("DEVELOP").size(13),
        text(&item.filename).size(12),
        text(review_status).size(11),
        text(culling).size(10),
        text("CULLING THRESHOLDS").size(10),
        text("Minimum sharpness").size(10),
        slider(
            0.0_f32..=0.02_f32,
            state.sharpness_threshold,
            Message::SharpnessThreshold
        )
        .step(0.0001_f32),
        text("Maximum clipped highlights").size(10),
        slider(
            0.0_f32..=0.25_f32,
            state.clipping_threshold,
            Message::ClippingThreshold
        )
        .step(0.005_f32),
        row![
            button(text(if state.mask_tool {
                "Cancel point"
            } else {
                "Point mask"
            }))
            .on_press(Message::MaskTool(!state.mask_tool)),
            button(text("Sky")).on_press(Message::MaskPoint([0.5, 0.1])),
            button(text("Subject")).on_press(Message::MaskPoint([0.5, 0.5]))
        ]
        .spacing(4),
        button(text("Clear mask")).on_press(Message::ClearMask),
        text("RGB HISTOGRAM").size(11)
    ];
    controls = controls.push(
        histogram::view(state.histogram_data.clone(), state.clipping).map(|output| match output {
            histogram::Output::Adjust(zone, delta) => Message::HistogramAdjust(zone, delta),
            histogram::Output::ToggleClipping(index) => Message::ToggleClipping(index),
        }),
    );
    controls = controls
        .push(text("TONE CURVE · RGB").size(11))
        .push(
            row![
                button(text("RGB")).on_press(Message::CurveChannel(0)),
                button(text("R")).on_press(Message::CurveChannel(1)),
                button(text("G")).on_press(Message::CurveChannel(2)),
                button(text("B")).on_press(Message::CurveChannel(3)),
            ]
            .spacing(4),
        )
        .push(
            widgets::tone_curve::view(state.curve_points[state.curve_channel].clone())
                .map(|p| Message::CurvePoint(p.0, p.1)),
        );
    controls = controls.push(text("COLOR GRADING").size(11));
    for (index, name) in ["Shadows", "Midtones", "Highlights"].iter().enumerate() {
        let (hue, saturation, luminance) = state.wheel_values[index];
        controls = controls.push(
            row![
                text(*name).width(Length::Fixed(76.0)).size(11),
                widgets::color_wheel::view(hue, saturation)
                    .map(move |v| Message::Wheel(index, v.0, v.1)),
                slider(-1.0..=1.0, luminance, move |v| Message::WheelLuma(index, v))
                    .step(0.01_f32)
                    .width(Length::Fixed(90.0)),
            ]
            .align_y(Center)
            .spacing(8),
        );
    }
    controls = controls
        .push(control(
            "Exposure",
            state.params.exposure,
            -5.0..=5.0,
            0.05,
            Message::Exposure,
        ))
        .push(control(
            "Contrast",
            state.params.contrast,
            -100.0..=100.0,
            1.0,
            Message::Contrast,
        ))
        .push(control(
            "Highlights",
            state.params.highlights,
            -100.0..=100.0,
            1.0,
            Message::Highlights,
        ))
        .push(control(
            "Shadows",
            state.params.shadows,
            -100.0..=100.0,
            1.0,
            Message::Shadows,
        ))
        .push(control(
            "Whites",
            state.params.whites,
            -100.0..=100.0,
            1.0,
            Message::Whites,
        ))
        .push(control(
            "Blacks",
            state.params.blacks,
            -100.0..=100.0,
            1.0,
            Message::Blacks,
        ))
        .push(text("WHITE BALANCE").size(11))
        .push(control(
            "Temperature",
            state.params.temperature,
            2000.0..=50000.0,
            25.0,
            Message::Temperature,
        ))
        .push(control(
            "Tint",
            state.params.tint,
            -150.0..=150.0,
            1.0,
            Message::Tint,
        ))
        .push(
            row![
                button(text("1")).on_press(Message::Rating(1)),
                button(text("2")).on_press(Message::Rating(2)),
                button(text("3")).on_press(Message::Rating(3)),
                button(text("4")).on_press(Message::Rating(4)),
                button(text("5")).on_press(Message::Rating(5))
            ]
            .spacing(3),
        )
        .push(
            row![
                button(text("Pick")).on_press(Message::Flag("Pick")),
                button(text("Reject")).on_press(Message::Flag("Reject")),
                button(text("Clear flag")).on_press(Message::Flag("None"))
            ]
            .spacing(3),
        )
        .push(
            row![
                button(text("Reset")).on_press(Message::Reset),
                button(text("Save edits")).on_press(Message::Save)
            ]
            .spacing(8),
        );
    let viewport: Element<'_, Message> = if let Some(handle) = &state.viewport {
        match state.view_mode {
            ViewMode::Single => canvas::Canvas::new(MainViewport {
                handle: handle.clone(),
                dimensions: state.preview_dimensions,
                zoom: state.zoom,
                custom_zoom: state.custom_zoom,
                pan: state.pan,
                mask_overlay: state.mask_overlay.clone(),
                mask_tool: state.mask_tool,
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
            ViewMode::SplitSideBySide => {
                let original = state.original_viewport.as_ref().unwrap_or(handle);
                row![
                    container(
                        image::Viewer::new(original.clone())
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .content_fit(ContentFit::Contain)
                    )
                    .width(Length::FillPortion((state.divider * 100.0) as u16)),
                    container(
                        image::Viewer::new(handle.clone())
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .content_fit(ContentFit::Contain)
                    )
                    .width(Length::FillPortion(((1.0 - state.divider) * 100.0) as u16))
                ]
                .height(Length::Fill)
                .into()
            }
            ViewMode::SplitTopBottom => {
                let original = state.original_viewport.as_ref().unwrap_or(handle);
                column![
                    container(
                        image::Viewer::new(original.clone())
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .content_fit(ContentFit::Contain)
                    )
                    .height(Length::FillPortion((state.divider * 100.0) as u16)),
                    container(
                        image::Viewer::new(handle.clone())
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .content_fit(ContentFit::Contain)
                    )
                    .height(Length::FillPortion(((1.0 - state.divider) * 100.0) as u16))
                ]
                .width(Length::Fill)
                .into()
            }
        }
    } else {
        text("Select a photo from Library").into()
    };
    let zoom_controls = row![
        button(text("Fit")).on_press(Message::SetZoom(ZoomMode::Fit)),
        button(text("Fill")).on_press(Message::SetZoom(ZoomMode::Fill)),
        button(text("1:1")).on_press(Message::SetZoom(ZoomMode::Actual)),
        button(text("2:1")).on_press(Message::SetZoom(ZoomMode::Two)),
        button(text("4:1")).on_press(Message::SetZoom(ZoomMode::Four)),
        Space::with_width(Length::Fill),
        button(text("Single")).on_press(Message::SetViewMode(ViewMode::Single)),
        button(text("Before | After")).on_press(Message::SetViewMode(ViewMode::SplitSideBySide)),
        button(text("Top / Bottom")).on_press(Message::SetViewMode(ViewMode::SplitTopBottom)),
        button(text("Z: toggle zoom  |  \\: compare")).on_press(Message::ToggleComparison)
    ]
    .spacing(5)
    .align_y(Center);
    let divider = row![
        text("Comparison split").size(10),
        slider(0.2_f32..=0.8_f32, state.divider, Message::Divider).step(0.01_f32)
    ]
    .spacing(8)
    .align_y(Center);
    let navigator: Element<'_, Message> = if let Some(handle) = &state.viewport {
        column![
            text("NAVIGATOR").size(11),
            canvas::Canvas::new(Navigator {
                handle: handle.clone(),
                zoom: state.zoom,
                custom_zoom: state.custom_zoom,
                pan: state.pan
            })
            .width(Length::Fixed(180.0))
            .height(Length::Fixed(120.0)),
            text("Drag the red frame to pan").size(10)
        ]
        .spacing(8)
        .into()
    } else {
        column![text("NAVIGATOR").size(11)].into()
    };
    let history_panel: Element<'_, Message> = if let Some(index) = state.selected {
        let entries = state.catalog.history(state.items[index].id);
        let active = state.catalog.active_history_id(state.items[index].id);
        let mut panel = column![text("HISTORY").size(11)].spacing(4);
        for entry in entries {
            panel = panel.push(
                button(
                    text(format!(
                        "{}{}",
                        if Some(entry.id) == active {
                            "● "
                        } else {
                            "○ "
                        },
                        entry.label
                    ))
                    .size(10),
                )
                .width(Length::Fill)
                .on_press(Message::HistorySelect(entry.id)),
            );
        }
        panel.into()
    } else {
        column![text("HISTORY").size(11)].into()
    };
    let canvas = column![
        zoom_controls,
        divider,
        container(viewport)
            .width(Length::Fill)
            .height(Length::Fill)
            .center(Length::Fill),
        text(metadata).size(11)
    ]
    .spacing(8)
    .align_x(Center);
    row![
        container(scrollable(column![navigator, history_panel].spacing(18)))
            .width(Length::Fixed(205.0))
            .padding([18, 12]),
        container(canvas)
            .padding(18)
            .width(Length::Fill)
            .height(Length::Fill),
        container(scrollable(controls.spacing(12)))
            .padding(16)
            .width(Length::Fixed(310.0))
            .height(Length::Fill)
    ]
    .height(Length::Fill)
    .into()
}

fn control(
    label: &'static str,
    value: f32,
    range: std::ops::RangeInclusive<f32>,
    step: f32,
    on_change: fn(f32) -> Message,
) -> Element<'static, Message> {
    let min = *range.start();
    let max = *range.end();
    let default = if label == "Temperature" { 5500.0 } else { 0.0 };
    let gradient = match label {
        "Temperature" => Gradient::Temperature,
        "Tint" => Gradient::Tint,
        _ => Gradient::Neutral,
    };
    scrubby_slider::view(label, value, min, max, default, step, gradient)
        .map(move |SliderOutput::Changed(value)| on_change(value))
}
fn empty_dash(s: &str) -> &str {
    if s.is_empty() {
        "—"
    } else {
        s
    }
}

#[derive(Default)]
struct CanvasInteraction {
    dragging: bool,
    last: Option<Point>,
}

struct MainViewport {
    handle: image::Handle,
    dimensions: (u32, u32),
    zoom: ZoomMode,
    custom_zoom: f32,
    pan: [f32; 2],
    mask_overlay: Option<image::Handle>,
    mask_tool: bool,
}

impl MainViewport {
    fn display_size(&self, bounds: Size) -> Size {
        let (width, height) = self.dimensions;
        if width == 0 || height == 0 {
            return bounds;
        }
        let fit = (bounds.width / width as f32).min(bounds.height / height as f32);
        let fill = (bounds.width / width as f32).max(bounds.height / height as f32);
        let scale = match self.zoom {
            ZoomMode::Fit => fit,
            ZoomMode::Fill => fill,
            ZoomMode::Actual => 1.0,
            ZoomMode::Two => 2.0,
            ZoomMode::Four => 4.0,
            ZoomMode::Custom => fit * self.custom_zoom,
        };
        Size::new(width as f32 * scale, height as f32 * scale)
    }
}

impl canvas::Program<Message> for MainViewport {
    type State = CanvasInteraction;

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(
                button @ (mouse::Button::Left | mouse::Button::Middle),
            )) => {
                if self.mask_tool {
                    if button == mouse::Button::Left {
                        if let Some(position) = cursor.position_in(bounds) {
                            let size = self.display_size(bounds.size());
                            let x = if size.width > bounds.width {
                                -(size.width - bounds.width) * self.pan[0]
                            } else {
                                (bounds.width - size.width) / 2.0
                            };
                            let y = if size.height > bounds.height {
                                -(size.height - bounds.height) * self.pan[1]
                            } else {
                                (bounds.height - size.height) / 2.0
                            };
                            if position.x >= x
                                && position.x <= x + size.width
                                && position.y >= y
                                && position.y <= y + size.height
                            {
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::MaskPoint([
                                        (position.x - x) / size.width.max(1.0),
                                        (position.y - y) / size.height.max(1.0),
                                    ])),
                                );
                            }
                        }
                    }
                    return (canvas::event::Status::Ignored, None);
                }
                state.dragging = true;
                state.last = cursor.position_in(bounds);
                (canvas::event::Status::Captured, None)
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(
                mouse::Button::Left | mouse::Button::Middle,
            )) => {
                state.dragging = false;
                state.last = None;
                (canvas::event::Status::Captured, None)
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) if state.dragging => {
                let dimensions = self.display_size(bounds.size());
                let hidden_x = (dimensions.width - bounds.width).max(1.0);
                let hidden_y = (dimensions.height - bounds.height).max(1.0);
                let previous = state.last.replace(position).unwrap_or(position);
                let delta = position - previous;
                (
                    canvas::event::Status::Captured,
                    Some(Message::PanBy(delta.x / hidden_x, delta.y / hidden_y)),
                )
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta })
                if cursor.position_in(bounds).is_some() =>
            {
                let y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y,
                    mouse::ScrollDelta::Pixels { y, .. } => y / 100.0,
                };
                (
                    canvas::event::Status::Captured,
                    Some(Message::WheelZoom(y.signum())),
                )
            }
            _ => (canvas::event::Status::Ignored, None),
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let size = self.display_size(bounds.size());
        let x = if size.width > bounds.width {
            -(size.width - bounds.width) * self.pan[0]
        } else {
            (bounds.width - size.width) / 2.0
        };
        let y = if size.height > bounds.height {
            -(size.height - bounds.height) * self.pan[1]
        } else {
            (bounds.height - size.height) / 2.0
        };
        let image_bounds = Rectangle {
            x,
            y,
            width: size.width,
            height: size.height,
        };
        frame.with_clip(
            Rectangle {
                x: 0.0,
                y: 0.0,
                width: bounds.width,
                height: bounds.height,
            },
            |clipped| {
                clipped.draw_image(image_bounds, &self.handle);
                if let Some(mask) = &self.mask_overlay {
                    clipped.draw_image(image_bounds, mask);
                }
            },
        );
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging {
            mouse::Interaction::Grabbing
        } else if self.mask_tool {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::Grab
        }
    }
}

struct Navigator {
    handle: image::Handle,
    zoom: ZoomMode,
    custom_zoom: f32,
    pan: [f32; 2],
}

impl canvas::Program<Message> for Navigator {
    type State = CanvasInteraction;

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                state.dragging = true;
                let p = cursor
                    .position_in(bounds)
                    .unwrap_or(Point::new(bounds.width / 2.0, bounds.height / 2.0));
                (
                    canvas::event::Status::Captured,
                    Some(Message::PanTo(p.x / bounds.width, p.y / bounds.height)),
                )
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) if state.dragging => (
                canvas::event::Status::Captured,
                Some(Message::PanTo(
                    position.x / bounds.width,
                    position.y / bounds.height,
                )),
            ),
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging = false;
                (canvas::event::Status::Captured, None)
            }
            _ => (canvas::event::Status::Ignored, None),
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let image_rect = Rectangle {
            x: 0.0,
            y: 0.0,
            width: bounds.width,
            height: bounds.height,
        };
        frame.draw_image(image_rect, &self.handle);
        let visible = match self.zoom {
            ZoomMode::Fit => 1.0,
            ZoomMode::Fill => 0.78,
            ZoomMode::Actual => 0.36,
            ZoomMode::Two => 0.18,
            ZoomMode::Four => 0.09,
            ZoomMode::Custom => (1.0 / self.custom_zoom.max(1.0)).max(0.09),
        };
        let width = (bounds.width * visible).max(24.0).min(bounds.width);
        let height = (bounds.height * visible).max(18.0).min(bounds.height);
        let x = (bounds.width - width) * self.pan[0];
        let y = (bounds.height - height) * self.pan[1];
        frame.stroke_rectangle(
            Point::new(x, y),
            Size::new(width, height),
            canvas::Stroke::default()
                .with_color(Color::from_rgb(1.0, 0.18, 0.12))
                .with_width(2.0),
        );
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging {
            mouse::Interaction::Grabbing
        } else {
            mouse::Interaction::Grab
        }
    }
}
