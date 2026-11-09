use crate::config;
use halation_catalog::{Catalog, CatalogItem};
use halation_core::{DevelopParameters, HistogramData, ImageBufferF32};
use halation_gpu::{GpuDevelopEngine, GpuEngineInfo};
use halation_ml::{Mask, MobileSam};
use halation_pipeline::{
    analyze_for_culling, export_batch, process_frame_with_gpu, CullingMetrics, ExportFormat,
    ExportJob, ThumbnailCache,
};
use halation_raw::{decode_file, RawAsset};
use iced::{
    event,
    font::Font,
    keyboard,
    theme::Palette,
    widget::{button, canvas, container, image, row, scrollable, slider, text, Space},
    Center, Color, Element, Event, Length, Size, Subscription, Task, Theme,
};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
};
mod pages;
mod shell;
use crate::widgets::{
    histogram,
    scrubby_slider::{self, Gradient, Output as SliderOutput},
};

const PREVIEW_MAX: u32 = 1200;

pub fn run() -> iced::Result {
    iced::application("Halation | Pro Color grading", update, shell::view)
        .theme(|_| {
            Theme::custom(
                "Halation Studio Dark".into(),
                Palette {
                    background: Color::from_rgb8(35, 36, 39),
                    text: Color::from_rgb8(227, 226, 228),
                    primary: Color::from_rgb8(255, 193, 116),
                    success: Color::from_rgb8(76, 215, 246),
                    danger: Color::from_rgb8(255, 180, 171),
                },
            )
        })
        .default_font(Font::with_name("Segoe UI"))
        .window_size(Size::new(1000.0, 600.0))
        .centered()
        .subscription(shell::subscription)
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Library,
    Develop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ZoomMode {
    Fit,
    Fill,
    Actual,
    Two,
    Four,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewMode {
    Single,
    SplitSideBySide,
    SplitTopBottom,
}

struct State {
    catalog: Catalog,
    items: Vec<CatalogItem>,
    thumbnail_cache: Arc<ThumbnailCache>,
    thumbnails: Vec<Option<image::Handle>>,
    thumbnail_requested: HashSet<usize>,
    culling_metrics: Vec<Option<CullingMetrics>>,
    exporting: bool,
    sharpness_threshold: f32,
    clipping_threshold: f32,
    filter_culling: bool,
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
    catalog_view_width: f32,
    catalog_scroll_id: scrollable::Id,
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
        let catalog_path = config::catalog_path();
        if let Some(parent) = catalog_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let catalog = Catalog::open(&catalog_path)
            .or_else(|_| Catalog::open_in_memory())
            .expect("catalog initialization failed");
        let items = catalog.load_items();
        let stored_culling = catalog.load_culling_metrics();
        let culling_metrics = items
            .iter()
            .map(|item| {
                stored_culling
                    .get(&item.id)
                    .map(|(sharpness, clipped_highlights)| CullingMetrics {
                        sharpness: *sharpness,
                        clipped_highlights: *clipped_highlights,
                    })
            })
            .collect();
        let count = items.len();
        let cache_root = config::cache_root();
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
            culling_metrics,
            exporting: false,
            sharpness_threshold: 0.001,
            clipping_threshold: 0.03,
            filter_culling: false,
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
            catalog_view_width: 960.0,
            catalog_scroll_id: scrollable::Id::unique(),
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
            status: format!("{} photos indexed in catalog", count),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Message {
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
    CatalogScrolled(f32, f32, f32),
    HistoryStep(bool),
    HistorySelect(i64),
    SyncXmp(u64),
    ThumbnailReady(usize, Result<Vec<u8>, String>),
    ExportAll(ExportFormat),
    ExportFinished(Result<Vec<Result<PathBuf, String>>, String>),
    SharpnessThreshold(f32),
    ClippingThreshold(f32),
    ToggleCullingFilter,
    MaskTool(bool),
    MaskPoint([f32; 2]),
    MaskReady(usize, u64, u64, Result<Mask, String>),
    ClearMask,
}

fn update(state: &mut State, message: Message) -> Task<Message> {
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
                                            catalog.save_culling_metrics(
                                                id,
                                                metrics.sharpness,
                                                metrics.clipped_highlights,
                                            );
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
            let count = imported.len();
            state.items = items;
            state.raw_assets = vec![None; state.items.len()];
            state.thumbnails = vec![None; state.items.len()];
            let stored_culling = state.catalog.load_culling_metrics();
            state.culling_metrics = state
                .items
                .iter()
                .map(|item| {
                    stored_culling
                        .get(&item.id)
                        .map(|(sharpness, clipped_highlights)| CullingMetrics {
                            sharpness: *sharpness,
                            clipped_highlights: *clipped_highlights,
                        })
                })
                .collect();
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
                format!("Imported {count} photo(s)")
            } else {
                format!("Import complete; {} failed: {}", errors.len(), errors[0])
            };
            if let Some(index) = latest {
                select_photo(state, index);
            }
        }
        Message::Select(index) => return open_photo(state, index),
        Message::Navigate(page) => state.page = page,
        Message::CatalogScrolled(y, height, width) => {
            state.catalog_scroll_y = y;
            state.catalog_view_height = height;
            state.catalog_view_width = width;
            const ROW_HEIGHT: f32 = 190.0;
            const HEADER_HEIGHT: f32 = 58.0;
            let columns = catalog_columns(width);
            let first_row =
                (((y - HEADER_HEIGHT).max(0.0) / ROW_HEIGHT) as usize).saturating_sub(2);
            let row_count = (height / ROW_HEIGHT).ceil() as usize + 4;
            let first = first_row * columns;
            let visible_items = visible_catalog_indices(state);
            let end = ((first_row + row_count) * columns).min(visible_items.len());
            let mut tasks = Vec::new();
            for index in visible_items
                .into_iter()
                .skip(first)
                .take(end.saturating_sub(first))
            {
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
                    let ext = match format {
                        ExportFormat::Jpeg => "jpg",
                        ExportFormat::Png => "png",
                        ExportFormat::WebP => "webp",
                        ExportFormat::Tiff => "tif",
                    };
                    destination.set_extension(ext);
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
            state.status = format!("Exporting {} master exposures...", jobs.len());
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
                        format!("Exported {succeeded} photos successfully")
                    } else {
                        format!("Exported {succeeded}; {failed} failed")
                    }
                }
                Err(error) => format!("Export failed: {error}"),
            };
        }
        Message::SharpnessThreshold(value) => {
            state.sharpness_threshold = value;
            if state.filter_culling {
                return Task::done(Message::CatalogScrolled(
                    state.catalog_scroll_y,
                    state.catalog_view_height,
                    state.catalog_view_width,
                ));
            }
        }
        Message::ClippingThreshold(value) => {
            state.clipping_threshold = value;
            if state.filter_culling {
                return Task::done(Message::CatalogScrolled(
                    state.catalog_scroll_y,
                    state.catalog_view_height,
                    state.catalog_view_width,
                ));
            }
        }
        Message::ToggleCullingFilter => {
            state.filter_culling = !state.filter_culling;
            state.catalog_scroll_y = 0.0;
            let scroll = scrollable::scroll_to(
                state.catalog_scroll_id.clone(),
                scrollable::AbsoluteOffset { x: 0.0, y: 0.0 },
            );
            let thumbnails = Task::done(Message::CatalogScrolled(
                0.0,
                state.catalog_view_height,
                state.catalog_view_width,
            ));
            return Task::batch([scroll, thumbnails]);
        }
        Message::MaskTool(enabled) => state.mask_tool = enabled,
        Message::MaskPoint(point) => {
            let Some(index) = state.selected else {
                return Task::none();
            };
            let path = state.items[index].path.clone();
            let ml = state.ml.clone();
            let model_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../models/mobilesam/mobilesam-onnx-float");
            state.status = "Inference: MobileSAM segmentation...".into();
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
                            .ok_or("MobileSAM failed to initialize")?
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
                            state.status = "MobileSAM target overlay active".into();
                        }
                        Err(error) => {
                            state.status = format!("Mask overlay generation failed: {error}")
                        }
                    },
                    Err(error) => state.status = format!("MobileSAM inference failed: {error}"),
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
            if state.thumbnails.iter().all(|t| t.is_none()) && !state.items.is_empty() {
                return Task::done(Message::CatalogScrolled(0.0, 800.0, 960.0));
            }
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
                    state.status = format!("Loaded {}", state.items[index].filename);
                    refresh(state);
                }
            }
            Err(error) => {
                if state.selected == Some(index) && state.load_generation == generation {
                    state.loading_photo = false;
                    state.status =
                        format!("Failed to load {}: {error}", state.items[index].filename);
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
                        .commit_history(id, "Manual Develop", &state.params)
                {
                    state.status = format!("History commit failed: {error}");
                }
                if let Err(error) = state.catalog.write_xmp_sidecar(id, &state.params) {
                    state.status = format!("XMP sidecar sync failed: {error}");
                }
                state.items[i].params = state.params.clone();
                if !state.status.starts_with("History commit failed")
                    && !state.status.starts_with("XMP sidecar sync failed")
                {
                    state.status = "Edits synced to catalog and XMP sidecar".into();
                }
            }
        }
        Message::SetZoom(zoom) => {
            state.zoom = zoom;
            state.custom_zoom = 1.0;
            state.pan = [0.5, 0.5];
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
            if matches!(
                key.as_str(),
                "ArrowDown" | "ArrowRight" | "ArrowUp" | "ArrowLeft"
            ) {
                let visible = visible_catalog_indices(state);
                if visible.is_empty() {
                    return Task::none();
                }
                let columns = catalog_columns(state.catalog_view_width);
                let current = state
                    .selected
                    .and_then(|selected| visible.iter().position(|index| *index == selected))
                    .unwrap_or(0);
                let next_position = match key.as_str() {
                    "ArrowDown" => (current + columns).min(visible.len() - 1),
                    "ArrowUp" => current.saturating_sub(columns),
                    "ArrowRight" => (current + 1).min(visible.len() - 1),
                    _ => current.saturating_sub(1),
                };
                let next = visible[next_position];
                state.selected = Some(next);
                const HEADER_HEIGHT: f32 = 58.0;
                const ROW_HEIGHT: f32 = 190.0;
                let row_y = HEADER_HEIGHT + (next_position / columns) as f32 * ROW_HEIGHT;
                let target_y = if row_y < state.catalog_scroll_y {
                    row_y
                } else if row_y + ROW_HEIGHT > state.catalog_scroll_y + state.catalog_view_height {
                    (row_y + ROW_HEIGHT - state.catalog_view_height).max(0.0)
                } else {
                    state.catalog_scroll_y
                };
                state.catalog_scroll_y = target_y;
                return scrollable::scroll_to(
                    state.catalog_scroll_id.clone(),
                    scrollable::AbsoluteOffset {
                        x: 0.0,
                        y: target_y,
                    },
                );
            }
            if let Some(index) = state.selected {
                match key.as_str() {
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
            async { tokio::time::sleep(std::time::Duration::from_millis(60)).await },
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
                state.status = format!("Could not decode {}: {e}", state.items[index].filename);
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

    // Eagerly request thumbnails for nearby filmstrip items
    let mut tasks = Vec::new();
    let start = index.saturating_sub(8);
    let end = (index + 9).min(state.items.len());
    for i in start..end {
        if state.thumbnails[i].is_none() && state.thumbnail_requested.insert(i) {
            let path = state.items[i].path.clone();
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
                move |result| Message::ThumbnailReady(i, result),
            ));
        }
    }

    if state.raw_assets[index].is_some() {
        state.loading_photo = false;
        refresh(state);
        state.page = Page::Develop;
        return Task::batch(tasks);
    }
    state.loading_photo = true;
    state.viewport = None;
    state.original_viewport = None;
    state.original_asset_index = None;
    state.histogram = None;
    state.histogram_data = None;
    state.status = format!("Decoding {}...", state.items[index].filename);
    let generation = state.load_generation;
    let path = state.items[index].path.clone();
    let asset_id = state.items[index].id;
    let catalog = state.catalog.clone();
    tasks.push(Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let raw = decode_file(&path).map_err(|e| e.to_string())?;
                let metrics = analyze_for_culling(&raw);
                catalog.save_culling_metrics(
                    asset_id,
                    metrics.sharpness,
                    metrics.clipped_highlights,
                );
                Ok::<_, String>((raw, metrics))
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|result| result)
        },
        move |result| Message::PhotoDecoded(index, generation, result),
    ));
    Task::batch(tasks)
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
    let mut developed = output.display_rgba;
    apply_clipping_mask(&mut developed, state.clipping);

    state.viewport = Some(image::Handle::from_rgba(w, h, developed));
    state.histogram = Some(image::Handle::from_rgba(256, 64, output.histogram_rgba));
    state.histogram_data = Some(output.histogram);
}

fn apply_clipping_mask(rgba: &mut [u8], clipping: [bool; 2]) {
    if !clipping[0] && !clipping[1] {
        return;
    }
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

fn catalog_columns(width: f32) -> usize {
    (((width - 40.0) / 230.0).floor() as usize).clamp(1, 6)
}

fn visible_catalog_indices(state: &State) -> Vec<usize> {
    state
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, _)| {
            if !state.filter_culling {
                return Some(index);
            }
            state.culling_metrics[index]
                .filter(|metrics| {
                    metrics.sharpness >= state.sharpness_threshold
                        && metrics.clipped_highlights <= state.clipping_threshold
                })
                .map(|_| index)
        })
        .collect()
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
