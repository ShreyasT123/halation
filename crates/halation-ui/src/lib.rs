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
    keyboard, mouse,
    theme::Palette,
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

const PREVIEW_MAX: u32 = 1200;

pub fn run() -> iced::Result {
    iced::application("Halation | Pro Color grading", update, view)
        .theme(|_| {
            Theme::custom(
                "Halation Studio Dark".into(),
                Palette {
                    background: Color::from_rgb8(35, 36, 39),       // #232427
                    text: Color::from_rgb8(227, 226, 228),          // #e3e2e4
                    primary: Color::from_rgb8(255, 193, 116),       // Amber #ffc174
                    success: Color::from_rgb8(76, 215, 246),       // Cyan #4cd7f6
                    danger: Color::from_rgb8(255, 180, 171),        // Salmon #ffb4ab
                },
            )
        })
        .default_font(Font::with_name("Segoe UI"))
        .window_size(Size::new(1000.0, 600.0))
        .centered()
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
    ColorLabel(&'static str),
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
            let first_row = (((y - HEADER_HEIGHT).max(0.0) / ROW_HEIGHT) as usize).saturating_sub(2);
            let row_count = (height / ROW_HEIGHT).ceil() as usize + 4;
            let first = first_row * columns;
            let visible_items = visible_catalog_indices(state);
            let end = ((first_row + row_count) * columns).min(visible_items.len());
            let mut tasks = Vec::new();
            for index in visible_items.into_iter().skip(first).take(end.saturating_sub(first)) {
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
                        Err(error) => state.status = format!("Mask overlay generation failed: {error}"),
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
        },
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
        Message::ColorLabel(label) => {
            if let Some(i) = state.selected {
                state.catalog.update_color_label(state.items[i].id, label);
                state.items[i].color_label = label.into();
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
                } else if row_y + ROW_HEIGHT
                    > state.catalog_scroll_y + state.catalog_view_height
                {
                    (row_y + ROW_HEIGHT - state.catalog_view_height).max(0.0)
                } else {
                    state.catalog_scroll_y
                };
                state.catalog_scroll_y = target_y;
                return scrollable::scroll_to(
                    state.catalog_scroll_id.clone(),
                    scrollable::AbsoluteOffset { x: 0.0, y: target_y },
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

pub fn view(state: &State) -> Element<'_, Message> {
    let brand = row![
        text("HALATION").size(16).font(Font::with_name("Segoe UI")),
        container(text("STUDIO").size(10).color(Color::from_rgb8(76, 215, 246)))
            .padding([2, 5])
            .style(container::rounded_box),
    ]
    .spacing(6)
    .align_y(Center);

    let nav_module = row![
        button(text("Library").size(12))
            .style(if state.page == Page::Library {
                button::primary
            } else {
                button::text
            })
            .on_press(Message::Navigate(Page::Library)),
        button(text("Develop").size(12))
            .style(if state.page == Page::Develop {
                button::primary
            } else {
                button::text
            })
            .on_press(Message::Navigate(Page::Develop)),
    ]
    .spacing(4)
    .align_y(Center);

    let loupe_quick = row![
        button(text("FIT").size(10))
            .style(if state.zoom == ZoomMode::Fit {
                button::secondary
            } else {
                button::text
            })
            .on_press(Message::SetZoom(ZoomMode::Fit)),
        button(text("100%").size(10))
            .style(if state.zoom == ZoomMode::Actual {
                button::secondary
            } else {
                button::text
            })
            .on_press(Message::SetZoom(ZoomMode::Actual)),
        button(text("200%").size(10))
            .style(if state.zoom == ZoomMode::Two {
                button::secondary
            } else {
                button::text
            })
            .on_press(Message::SetZoom(ZoomMode::Two)),
    ]
    .spacing(2)
    .align_y(Center);

    let history_actions = row![
        button(text("Undo").size(11))
            .style(button::text)
            .on_press(Message::HistoryStep(false)),
        button(text("Redo").size(11))
            .style(button::text)
            .on_press(Message::HistoryStep(true)),
        button(text(if state.view_mode == ViewMode::Single {
            "Compare"
        } else {
            "Loupe"
        }).size(11))
        .style(if state.view_mode != ViewMode::Single {
            button::secondary
        } else {
            button::text
        })
        .on_press(Message::ToggleComparison),
    ]
    .spacing(4)
    .align_y(Center);

    let export_actions = row![
        button(text("JPEG").size(10))
            .style(button::text)
            .on_press(Message::ExportAll(ExportFormat::Jpeg)),
        button(text("PNG").size(10))
            .style(button::text)
            .on_press(Message::ExportAll(ExportFormat::Png)),
        button(text("WebP").size(10))
            .style(button::text)
            .on_press(Message::ExportAll(ExportFormat::WebP)),
        button(text("TIFF").size(10))
            .style(button::text)
            .on_press(Message::ExportAll(ExportFormat::Tiff)),
    ]
    .spacing(3)
    .align_y(Center);

    let header = container(
        row![
            brand,
            Space::with_width(Length::Fixed(16.0)),
            nav_module,
            Space::with_width(Length::Fixed(16.0)),
            loupe_quick,
            Space::with_width(Length::Fixed(12.0)),
            history_actions,
            Space::with_width(Length::Fill),
            export_actions,
            Space::with_width(Length::Fixed(12.0)),
            button(text("Import Photos").size(11))
                .style(button::primary)
                .on_press(Message::Import),
        ]
        .align_y(Center)
    )
    .padding([6, 14])
    .width(Length::Fill)
    .style(container::rounded_box);

    let content: Element<'_, Message> = match state.page {
        Page::Library => library_page(state),
        Page::Develop => develop_page(state),
    };

    let footer = container(
        row![
            text(format!("{} photos indexed", state.items.len()))
                .size(10)
                .font(Font::MONOSPACE),
            Space::with_width(Length::Fixed(12.0)),
            text(&state.status)
                .size(10)
                .font(Font::MONOSPACE)
                .color(Color::from_rgb8(76, 215, 246)),
            Space::with_width(Length::Fill),
            text(format!(
                "{}  ·  {}",
                state.gpu.adapter_name, state.gpu.display_profile
            ))
            .size(10)
            .color(Color::from_rgb8(150, 152, 156)),
        ]
        .align_y(Center)
    )
    .padding([5, 14])
    .width(Length::Fill)
    .style(container::rounded_box);

    column![header, content, footer].spacing(4).into()
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
                        "z", "Z", "j", "J", "\\", "1", "2", "3", "4", "5", "6", "7", "8", "9",
                        "x", "X", "u", "U",
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
        iced::time::every(std::time::Duration::from_secs(3)).map(|_| Message::RefreshDisplay),
    ])
}

fn library_page(state: &State) -> Element<'_, Message> {
    const ROW_HEIGHT: f32 = 190.0;
    const HEADER_HEIGHT: f32 = 58.0;
    const GAP: f32 = 12.0;
    let columns = catalog_columns(state.catalog_view_width);
    let card_width = ((state.catalog_view_width - 40.0 - GAP * (columns - 1) as f32)
        / columns as f32)
        .max(160.0);
    let mut grid = column![
        row![
            column![
                text("PHOTO LIBRARY").size(13),
                text("Browse, rate, and organize your originals")
                    .size(11)
                    .color(Color::from_rgb8(150, 152, 156)),
            ]
            .spacing(5),
            Space::with_width(Length::Fill),
            text(if state.filter_culling {
                format!(
                    "{} SELECTS / {} PHOTOS",
                    visible_catalog_indices(state).len(),
                    state.items.len()
                )
            } else {
                format!("{} PHOTOS", state.items.len())
            })
                .size(10)
                .color(Color::from_rgb8(150, 152, 156)),
            button(text(if state.filter_culling {
                "SHOW ALL"
            } else {
                "FILTER SELECTS"
            })
            .size(9))
            .style(if state.filter_culling {
                button::secondary
            } else {
                button::text
            })
            .on_press(Message::ToggleCullingFilter),
        ]
        .align_y(Center),
    ]
    .spacing(14);

    if state.items.is_empty() {
        grid = grid
            .push(Space::with_height(Length::Fixed(60.0)))
            .push(text("Catalog Empty").size(22))
            .push(
                text("Import raw files (DNG, ARW, CR2, NEF) or standard formats (JPEG, TIFF, PNG).")
                    .size(12),
            )
            .push(button(text("Import Master Files")).on_press(Message::Import));
    } else {
        let visible_items = visible_catalog_indices(state);
        if visible_items.is_empty() {
            grid = grid
                .push(Space::with_height(Length::Fixed(60.0)))
                .push(text("No photos meet the current culling thresholds").size(18))
                .push(
                    button(text("Show All Photos"))
                        .on_press(Message::ToggleCullingFilter),
                );
        }
        let row_count = visible_items.len().div_ceil(columns);
        let first_row = if row_count == 0 {
            0
        } else {
            ((((state.catalog_scroll_y - HEADER_HEIGHT).max(0.0) / ROW_HEIGHT) as usize)
                .saturating_sub(2))
            .min(row_count - 1)
        };
        let visible_rows = (state.catalog_view_height / ROW_HEIGHT).ceil() as usize + 4;
        let end_row = (first_row + visible_rows).min(row_count);
        if first_row > 0 {
            grid = grid.push(Space::with_height(Length::Fixed(first_row as f32 * ROW_HEIGHT)));
        }
        for row_index in first_row..end_row {
            let mut cards = row![].spacing(GAP).height(Length::Fixed(ROW_HEIGHT));
            for idx in visible_items
                .iter()
                .skip(row_index * columns)
                .take(columns)
                .copied()
            {
                let item = &state.items[idx];
                let is_selected = state.selected == Some(idx);
                let preview: Element<'_, Message> = if let Some(handle) = &state.thumbnails[idx] {
                    image(handle.clone())
                        .content_fit(ContentFit::Contain)
                        .width(Length::Fill)
                        .height(Length::Fixed(126.0))
                        .into()
                } else {
                    container(text("Loading preview").size(10).color(Color::from_rgb8(150, 152, 156)))
                        .width(Length::Fill)
                        .height(Length::Fixed(126.0))
                        .align_x(iced::alignment::Horizontal::Center)
                        .align_y(iced::alignment::Vertical::Center)
                        .into()
                };
                let flag_color = match item.flag.as_str() {
                    "Pick" => Color::from_rgb8(76, 215, 246),
                    "Reject" => Color::from_rgb8(255, 180, 171),
                    _ => Color::from_rgb8(150, 152, 156),
                };
                let format = item
                    .path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or("IMG")
                    .to_ascii_uppercase();
                let format = match format.as_str() {
                    "JPG" | "JPEG" => "JPG",
                    "TIF" | "TIFF" => "TIFF",
                    "DNG" => "DNG",
                    "ARW" | "CR2" | "CR3" | "NEF" | "RAF" | "RW2" | "ORF" => "RAW",
                    _ => "IMG",
                };
                let culling_label = state.culling_metrics[idx].map(|metrics| {
                    if metrics.sharpness < state.sharpness_threshold
                        || metrics.clipped_highlights > state.clipping_threshold
                    {
                        ("REVIEW", Color::from_rgb8(255, 180, 171))
                    } else {
                        ("SELECT", Color::from_rgb8(76, 215, 246))
                    }
                });
                let card = button(
                    column![
                        preview,
                        text(&item.filename).size(11).font(Font::MONOSPACE),
                        row![
                            text(format!("* {}", item.rating)).size(10),
                            text(format).size(9).color(Color::from_rgb8(76, 215, 246)),
                            text(&item.flag).size(10).color(flag_color),
                            text(&item.color_label).size(10),
                            if let Some((label, color)) = culling_label {
                                text(label).size(9).color(color)
                            } else {
                                text("").size(9)
                            },
                            Space::with_width(Length::Fill),
                            text(format!("{:.1} MB", item.file_size as f32 / 1_000_000.0))
                                .size(9)
                                .color(Color::from_rgb8(150, 152, 156)),
                        ]
                        .spacing(8)
                        .align_y(Center),
                    ]
                    .spacing(7)
                    .padding(8)
                    .width(Length::Fill),
                )
                .style(if is_selected { button::secondary } else { button::text })
                .width(Length::Fixed(card_width))
                .height(Length::Fill)
                .on_press(Message::Select(idx));
                cards = cards.push(card);
            }
            grid = grid.push(cards);
        }
        if end_row < row_count {
            grid = grid.push(Space::with_height(Length::Fixed(
                (row_count - end_row) as f32 * ROW_HEIGHT,
            )));
        }
    }

    container(
        scrollable(grid.spacing(0))
            .id(state.catalog_scroll_id.clone())
            .on_scroll(|viewport| {
                let offset = viewport.absolute_offset();
                let bounds = viewport.bounds();
                Message::CatalogScrolled(offset.y, bounds.height, bounds.width)
            }),
    )
    .padding(18)
    .width(Length::Fill)
    .height(Length::Fill)
    .style(container::rounded_box)
    .into()
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

fn develop_page(state: &State) -> Element<'_, Message> {
    let Some(index) = state.selected else {
        return library_page(state);
    };
    let item = &state.items[index];

    if state.loading_photo {
        return container(
            column![
                text("LOADING ASSET").size(12).color(Color::from_rgb8(76, 215, 246)),
                text(&item.filename).size(20),
                text("initializing…").size(12),
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
    let exif_stamp = meta
        .map(|m| {
            format!(
                "{} · {} · ISO {} · {} · {}",
                empty_dash(&m.camera),
                empty_dash(&m.lens),
                m.iso.map(|v| v.to_string()).unwrap_or_else(|| "—".into()),
                empty_dash(&m.shutter),
                empty_dash(&m.aperture)
            )
        })
        .unwrap_or_default();

    // ==========================================
    // COLUMN 1: LEFT UTILITY DECK (260px)
    // ==========================================
    let navigator: Element<'_, Message> = if let Some(handle) = &state.viewport {
        column![
            row![
                text("NAVIGATOR").size(11).color(Color::from_rgb8(76, 215, 246)),
                Space::with_width(Length::Fill),
                button(text("FIT").size(9)).style(button::text).on_press(Message::SetZoom(ZoomMode::Fit)),
                button(text("1:1").size(9)).style(button::text).on_press(Message::SetZoom(ZoomMode::Actual)),
                button(text("2:1").size(9)).style(button::text).on_press(Message::SetZoom(ZoomMode::Two)),
            ]
            .align_y(Center),
            canvas::Canvas::new(Navigator {
                handle: handle.clone(),
                zoom: state.zoom,
                custom_zoom: state.custom_zoom,
                pan: state.pan
            })
            .width(Length::Fixed(236.0))
            .height(Length::Fixed(140.0)),
            text(format!("{} × {} px", state.preview_dimensions.0, state.preview_dimensions.1))
                .size(9)
                .color(Color::from_rgb8(150, 152, 156)),
        ]
        .spacing(6)
        .into()
    } else {
        column![text("NAVIGATOR").size(11)].into()
    };

    let history_panel: Element<'_, Message> = {
        let entries = state.catalog.history(item.id);
        let active = state.catalog.active_history_id(item.id);
        let mut list = column![
            row![
                text("HISTORY & BRANCHES").size(11).color(Color::from_rgb8(76, 215, 246)),
                Space::with_width(Length::Fill),
                text(format!("{} steps", entries.len())).size(9).color(Color::from_rgb8(150, 152, 156)),
            ]
            .align_y(Center)
        ]
        .spacing(4);

        for entry in entries.into_iter().rev() {
            let is_active = Some(entry.id) == active;
            list = list.push(
                button(
                    row![
                        text(if is_active { "●" } else { "○" })
                            .size(10)
                            .color(if is_active {
                                Color::from_rgb8(76, 215, 246)
                            } else {
                                Color::from_rgb8(150, 152, 156)
                            }),
                        text(entry.label).size(10), // <-- Pass owned String by value
                    ]
                    .spacing(6)
                    .align_y(Center),
                )
                .style(if is_active {
                    button::secondary
                } else {
                    button::text
                })
                .width(Length::Fill)
                .on_press(Message::HistorySelect(entry.id)),
            );
        }
        list.into()
    };

    let culling_panel: Element<'_, Message> = {
        let metrics = state.culling_metrics[index];
        let status_str = metrics
            .map(|m| {
                if m.sharpness < state.sharpness_threshold || m.clipped_highlights > state.clipping_threshold {
                    ("REVIEW REQUIRED", Color::from_rgb8(255, 180, 171))
                } else {
                    ("IN FOCUS · BALANCED", Color::from_rgb8(76, 215, 246))
                }
            })
            .unwrap_or(("PENDING METRICS", Color::from_rgb8(150, 152, 156)));

        column![
            text("CULLING ANALYSIS").size(11).color(Color::from_rgb8(76, 215, 246)),
            text(status_str.0).size(10).color(status_str.1),
            text(metrics.map(|m| format!("Laplacian Focus: {:.4}", m.sharpness)).unwrap_or_default())
                .size(9)
                .font(Font::MONOSPACE),
            text(metrics.map(|m| format!("Clipped Highlights: {:.2}%", m.clipped_highlights * 100.0)).unwrap_or_default())
                .size(9)
                .font(Font::MONOSPACE),
            text("Min Sharpness").size(9).color(Color::from_rgb8(150, 152, 156)),
            slider(0.0001_f32..=0.015_f32, state.sharpness_threshold, Message::SharpnessThreshold)
                .step(0.0001_f32),
            text("Max Highlight Clipping").size(9).color(Color::from_rgb8(150, 152, 156)),
            slider(0.0_f32..=0.20_f32, state.clipping_threshold, Message::ClippingThreshold)
                .step(0.005_f32),
        ]
        .spacing(4)
        .into()
    };

    let left_deck = container(
        scrollable(
            column![
                navigator,
                container(history_panel).padding(8).style(container::rounded_box),
                container(culling_panel).padding(8).style(container::rounded_box),
            ]
            .spacing(12),
        )
    )
    .width(Length::Fixed(260.0))
    .padding(10)
    .style(container::rounded_box);

    // ==========================================
    // COLUMN 2: CENTER STAGE & FILMSTRIP (FILL)
    // ==========================================
    let center_toolbar = row![
        button(text("Loupe").size(10))
            .style(if state.view_mode == ViewMode::Single { button::secondary } else { button::text })
            .on_press(Message::SetViewMode(ViewMode::Single)),
        button(text("Side Split").size(10))
            .style(if state.view_mode == ViewMode::SplitSideBySide { button::secondary } else { button::text })
            .on_press(Message::SetViewMode(ViewMode::SplitSideBySide)),
        button(text("Top Split").size(10))
            .style(if state.view_mode == ViewMode::SplitTopBottom { button::secondary } else { button::text })
            .on_press(Message::SetViewMode(ViewMode::SplitTopBottom)),
        Space::with_width(Length::Fixed(12.0)),
        button(text(if state.clipping[0] { "▲ Black Clip ON" } else { "▲ Black Clip" }).size(10))
            .style(if state.clipping[0] { button::secondary } else { button::text })
            .on_press(Message::ToggleClipping(0)),
        button(text(if state.clipping[1] { "▼ White Clip ON" } else { "▼ White Clip" }).size(10))
            .style(if state.clipping[1] { button::danger } else { button::text })
            .on_press(Message::ToggleClipping(1)),
        Space::with_width(Length::Fill),
        if state.view_mode != ViewMode::Single {
            row![
                text("Split").size(10),
                slider(0.1_f32..=0.9_f32, state.divider, Message::Divider).width(Length::Fixed(90.0)),
            ]
            .spacing(6)
            .align_y(Center)
        } else {
            row![]
        },
    ]
    .spacing(4)
    .align_y(Center);

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
                    .width(Length::FillPortion(((1.0 - state.divider) * 100.0) as u16)),
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
                    .height(Length::FillPortion(((1.0 - state.divider) * 100.0) as u16)),
                ]
                .width(Length::Fill)
                .into()
            }
        }
    } else {
        container(text("Select photo to begin develop").size(12))
            .center(Length::Fill)
            .into()
    };

    let filmstrip: Element<'_, Message> = {
        let mut row_items = row![].spacing(6).align_y(Center);
        for (i, it) in state.items.iter().enumerate() {
            let is_sel = state.selected == Some(i);
            let thumb: Element<'_, Message> = if let Some(h) = &state.thumbnails[i] {
                image(h.clone())
                    .content_fit(ContentFit::Cover)
                    .width(Length::Fixed(84.0))
                    .height(Length::Fixed(50.0))
                    .into()
            } else {
                container(
                    text("No Preview")
                        .size(8)
                        .color(Color::from_rgb8(150, 152, 156)),
                )
                .width(Length::Fixed(84.0))
                .height(Length::Fixed(50.0))
                .align_x(iced::alignment::Horizontal::Center)
                .align_y(iced::alignment::Vertical::Center)
                .into()
            };

            row_items = row_items.push(
                button(
                    column![
                        thumb,
                        row![
                            text(format!("#{}", i + 1))
                                .size(8)
                                .color(Color::from_rgb8(150, 152, 156)),
                            Space::with_width(Length::Fixed(8.0)),
                            text(format!("★{}", it.rating))
                                .size(8)
                                .color(Color::from_rgb8(255, 193, 116)),
                        ]
                        .width(Length::Fixed(84.0))
                        .align_y(Center),
                    ]
                    .spacing(2)
                    .align_x(Center),
                )
                .width(Length::Fixed(92.0))
                .height(Length::Fixed(72.0))
                .padding(3)
                .style(if is_sel {
                    button::secondary
                } else {
                    button::text
                })
                .on_press(Message::Select(i)),
            );
        }

        container(
            column![
                row![
                    text("FILMSTRIP DOCK").size(10).color(Color::from_rgb8(76, 215, 246)),
                    Space::with_width(Length::Fill),
                    text(format!("{} of {}", index + 1, state.items.len()))
                        .size(9)
                        .color(Color::from_rgb8(150, 152, 156)),
                ]
                .align_y(Center),
                scrollable::Scrollable::with_direction(
                    row_items.height(Length::Fixed(74.0)),
                    scrollable::Direction::Horizontal(scrollable::Scrollbar::default()),
                ),
            ]
            .spacing(4),
        )
        .padding(6)
        .width(Length::Fill)
        .height(Length::Fixed(106.0))
        .style(container::rounded_box)
        .into()
    };

    let center_canvas = column![
        center_toolbar,
        container(viewport)
            .width(Length::Fill)
            .height(Length::Fill)
            .center(Length::Fill)
            .style(container::rounded_box),
        text(exif_stamp).size(10).font(Font::MONOSPACE).color(Color::from_rgb8(170, 172, 176)),
        filmstrip,
    ]
    .spacing(6);

    // ==========================================
    // COLUMN 3: RIGHT DEVELOP INSPECTOR (350px)
    // ==========================================
    let mut controls = column![
        row![
            text("DEVELOP INSPECTOR").size(11).color(Color::from_rgb8(76, 215, 246)),
            Space::with_width(Length::Fill),
            text(&item.filename).size(10).font(Font::MONOSPACE),
        ]
        .align_y(Center),
    ]
    .spacing(8);

    controls = controls.push(
        histogram::view(state.histogram_data.clone(), state.clipping).map(|output| match output {
            histogram::Output::Adjust(zone, delta) => Message::HistogramAdjust(zone, delta),
            histogram::Output::ToggleClipping(idx) => Message::ToggleClipping(idx),
        }),
    );

    if let Some(m) = meta {
        controls = controls.push(
            row![
                container(text(format!("ISO {}", m.iso.map(|v| v.to_string()).unwrap_or_else(|| "—".into()))).size(9))
                    .padding([2, 6])
                    .style(container::rounded_box),
                container(text(empty_dash(&m.shutter)).size(9))
                    .padding([2, 6])
                    .style(container::rounded_box),
                container(text(empty_dash(&m.aperture)).size(9))
                    .padding([2, 6])
                    .style(container::rounded_box),
                container(text(empty_dash(&m.lens)).size(9))
                    .padding([2, 6])
                    .style(container::rounded_box),
            ]
            .spacing(4),
        );
    }

    controls = controls.push(
        column![
            text("AI MASKING (MOBILESAM)").size(10).color(Color::from_rgb8(76, 215, 246)),
            row![
                button(text(if state.mask_tool { "Cancel Point" } else { "Point Mask" }).size(10))
                    .style(if state.mask_tool { button::secondary } else { button::text })
                    .on_press(Message::MaskTool(!state.mask_tool)),
                button(text("Sky").size(10)).style(button::text).on_press(Message::MaskPoint([0.5, 0.1])),
                button(text("Subject").size(10)).style(button::text).on_press(Message::MaskPoint([0.5, 0.5])),
                button(text("Clear").size(10)).style(button::text).on_press(Message::ClearMask),
            ]
            .spacing(4),
        ]
        .spacing(4),
    );

    controls = controls.push(
        column![
            row![
                text("TONE CURVE").size(10).color(Color::from_rgb8(76, 215, 246)),
                Space::with_width(Length::Fill),
                button(text("RGB").size(9)).style(if state.curve_channel == 0 { button::secondary } else { button::text }).on_press(Message::CurveChannel(0)),
                button(text("R").size(9)).style(if state.curve_channel == 1 { button::secondary } else { button::text }).on_press(Message::CurveChannel(1)),
                button(text("G").size(9)).style(if state.curve_channel == 2 { button::secondary } else { button::text }).on_press(Message::CurveChannel(2)),
                button(text("B").size(9)).style(if state.curve_channel == 3 { button::secondary } else { button::text }).on_press(Message::CurveChannel(3)),
            ]
            .spacing(2)
            .align_y(Center),
            widgets::tone_curve::view(state.curve_points[state.curve_channel].clone())
                .map(|p| Message::CurvePoint(p.0, p.1)),
        ]
        .spacing(4),
    );

    controls = controls.push(
        column![
            text("3-WAY COLOR GRADING").size(10).color(Color::from_rgb8(76, 215, 246)),
            row![
                column![
                    text("Shadows").size(9),
                    widgets::color_wheel::view(state.wheel_values[0].0, state.wheel_values[0].1)
                        .map(|v| Message::Wheel(0, v.0, v.1)),
                    slider(-1.0..=1.0, state.wheel_values[0].2, |v| Message::WheelLuma(0, v)).step(0.02_f32),
                ]
                .align_x(Center)
                .spacing(3),
                column![
                    text("Midtones").size(9),
                    widgets::color_wheel::view(state.wheel_values[1].0, state.wheel_values[1].1)
                        .map(|v| Message::Wheel(1, v.0, v.1)),
                    slider(-1.0..=1.0, state.wheel_values[1].2, |v| Message::WheelLuma(1, v)).step(0.02_f32),
                ]
                .align_x(Center)
                .spacing(3),
                column![
                    text("Highlights").size(9),
                    widgets::color_wheel::view(state.wheel_values[2].0, state.wheel_values[2].1)
                        .map(|v| Message::Wheel(2, v.0, v.1)),
                    slider(-1.0..=1.0, state.wheel_values[2].2, |v| Message::WheelLuma(2, v)).step(0.02_f32),
                ]
                .align_x(Center)
                .spacing(3),
            ]
            .spacing(8),
        ]
        .spacing(4),
    );

    controls = controls.push(
        column![
            text("BASIC CALIBRATION").size(10).color(Color::from_rgb8(76, 215, 246)),
            control("Temperature", state.params.temperature, 2000.0..=50000.0, 25.0, Message::Temperature),
            control("Tint", state.params.tint, -150.0..=150.0, 1.0, Message::Tint),
        ]
        .spacing(2),
    );

    controls = controls.push(
        column![
            text("TONE DISTRIBUTION").size(10).color(Color::from_rgb8(76, 215, 246)),
            control("Exposure", state.params.exposure, -5.0..=5.0, 0.05, Message::Exposure),
            control("Contrast", state.params.contrast, -100.0..=100.0, 1.0, Message::Contrast),
            control("Highlights", state.params.highlights, -100.0..=100.0, 1.0, Message::Highlights),
            control("Shadows", state.params.shadows, -100.0..=100.0, 1.0, Message::Shadows),
            control("Whites", state.params.whites, -100.0..=100.0, 1.0, Message::Whites),
            control("Blacks", state.params.blacks, -100.0..=100.0, 1.0, Message::Blacks),
        ]
        .spacing(2),
    );

    controls = controls.push(
        row![
            text("Rating:").size(10),
            button(text("1").size(9)).style(button::text).on_press(Message::Rating(1)),
            button(text("2").size(9)).style(button::text).on_press(Message::Rating(2)),
            button(text("3").size(9)).style(button::text).on_press(Message::Rating(3)),
            button(text("4").size(9)).style(button::text).on_press(Message::Rating(4)),
            button(text("5").size(9)).style(button::text).on_press(Message::Rating(5)),
            Space::with_width(Length::Fill),
            button(text("Pick").size(9)).style(button::text).on_press(Message::Flag("Pick")),
            button(text("Reject").size(9)).style(button::text).on_press(Message::Flag("Reject")),
        ]
        .spacing(2)
        .align_y(Center),
    );

    controls = controls.push(
        row![
            button(text("Reset").size(11))
                .style(button::text)
                .on_press(Message::Reset),
            Space::with_width(Length::Fill),
            button(text("Save Edits").size(11))
                .style(button::primary)
                .on_press(Message::Save),
        ]
        .align_y(Center),
    );

    let right_inspector = container(scrollable(controls.spacing(10)))
        .width(Length::Fixed(350.0))
        .padding(12)
        .style(container::rounded_box);

    row![left_deck, center_canvas, right_inspector]
        .spacing(6)
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
                state.last = cursor.position();
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
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            Color::from_rgb8(35, 36, 39),
        );

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
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging {
            mouse::Interaction::Grabbing
        } else if self.mask_tool {
            mouse::Interaction::Crosshair
        } else if {
            let size = self.display_size(bounds.size());
            size.width > bounds.width || size.height > bounds.height
        } {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::Idle
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
                if let Some(p) = cursor.position_in(bounds) {
                    (
                        canvas::event::Status::Captured,
                        Some(Message::PanTo(
                            (p.x / bounds.width).clamp(0.0, 1.0),
                            (p.y / bounds.height).clamp(0.0, 1.0),
                        )),
                    )
                } else {
                    (canvas::event::Status::Ignored, None)
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) if state.dragging => {
                let x = (position.x - bounds.x).clamp(0.0, bounds.width);
                let y = (position.y - bounds.y).clamp(0.0, bounds.height);
                (
                    canvas::event::Status::Captured,
                    Some(Message::PanTo(
                        (x / bounds.width).clamp(0.0, 1.0),
                        (y / bounds.height).clamp(0.0, 1.0),
                    )),
                )
            }
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
        let width = (bounds.width * visible).max(20.0).min(bounds.width);
        let height = (bounds.height * visible).max(14.0).min(bounds.height);
        let x = (bounds.width - width) * self.pan[0];
        let y = (bounds.height - height) * self.pan[1];
        frame.stroke_rectangle(
            Point::new(x, y),
            Size::new(width, height),
            canvas::Stroke::default()
                .with_color(Color::from_rgb8(76, 215, 246))
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
