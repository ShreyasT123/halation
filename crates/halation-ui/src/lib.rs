//! halation-ui — Halation darkroom shell (Iced 0.13).
//!
//! ```text
//! ┌ top bar ────────────────────────────────────────────────────────────┐
//! │ left 260  │ loupe canvas · toolbar · EXIF HUD            │ right 320 │
//! │ navigator │                                              │ histogram │
//! │ history   │                                              │ tool rail │
//! │ snapshots │                                              │ accordion │
//! │ folders   │                                              │           │
//! ├───────────┴──────────────────────────────────────────────┴───────────┤
//! │ filmstrip 110                                                        │
//! └ status bar ─────────────────────────────────────────────────────────┘
//! ```
//!
//! Decode, thumbnails and develop renders all run off the UI thread. Renders are coalesced
//! (latest-wins, at most one in flight), so dragging a slider never queues frames behind itself.
//!
//! Needs iced features `image` plus an executor (`tokio` or `thread-pool`).
//!
//! Keymap: Tab / Shift+Tab / `/` toggle left / right / filmstrip · Z zoom · `\` before/after ·
//! ←/→ previous/next · 0–5 rating · P/X/U pick/reject/clear · Ctrl/Cmd+Z, +Shift+Z undo/redo ·
//! Ctrl/Cmd+O import.

mod ext;
mod grid;
mod tools;
mod viewport;
mod widgets;

use ext::{ExtMessage, ExtState};
use grid::{GridMessage, HandleCache};
use halation_catalog::{Catalog, CatalogItem};
use halation_core::{DevelopParameters, HistogramData, HistoryGraph, ImageBufferF32, ParamId, Wheel};
use halation_gpu::{GpuDevelopEngine, GpuEngineInfo};
use halation_pipeline::{make_preview, process_with_context, RenderContext, HISTOGRAM_HEIGHT, HISTOGRAM_WIDTH};
use halation_raw::{decode_file, decode_thumbnail, read_metadata, RawMetadata, SUPPORTED_EXTENSIONS};
use viewport::{Compare, Navigator, ViewEvent, Viewport};
use widgets::{CurveEdit, HistogramView, Scrubby, WheelId};
use iced::{
    alignment::{Horizontal, Vertical},
    keyboard::{self, key::Named, Key},
    widget::{button, column, container, image, mouse_area, row, scrollable, slider, stack, text, Column, Row, Space, Text},
    Background, Border, Color, ContentFit, Element, Font, Length, Padding, Shadow, Size, Subscription, Task, Theme,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

const PREVIEW_MAX: u32 = 2048;
const THUMB_MAX: u32 = 192;
const LEFT_W: f32 = 260.0;
const RIGHT_W: f32 = 320.0;
const FILMSTRIP_H: f32 = 110.0;
const RADIUS: f32 = 3.0;

// ------------------------------------------------------------------------------------------------
// Palette & styles
// ------------------------------------------------------------------------------------------------

const fn hex(v: u32) -> Color {
    Color {
        r: ((v >> 16) & 0xFF) as f32 / 255.0,
        g: ((v >> 8) & 0xFF) as f32 / 255.0,
        b: (v & 0xFF) as f32 / 255.0,
        a: 1.0,
    }
}

const BG_APP: Color = hex(0x121212);
const BG_PANEL: Color = hex(0x181818);
const BG_HEADER: Color = hex(0x222222);
const BG_INPUT: Color = hex(0x282828);
const BG_CANVAS: Color = hex(0x161616);
const BG_HOVER: Color = hex(0x2A2A2A);
const BG_SELECTED: Color = hex(0x1F2E44);
const BORDER: Color = hex(0x2C2C2C);
const TEXT_PRIMARY: Color = hex(0xE0E0E0);
const TEXT_HEADING: Color = hex(0xD4D4D4);
const TEXT_SECONDARY: Color = hex(0xA0A0A0);
const TEXT_DIM: Color = hex(0x6E6E6E);
const ACCENT: Color = hex(0x2680EB);
const PICK: Color = hex(0x46A758);
const REJECT: Color = hex(0xE5484D);
const STAR: Color = hex(0xF2C14E);
const CLIP_SHADOW: Color = hex(0x2255FF);
const CLIP_HIGHLIGHT: Color = hex(0xFF2222);

fn darkroom_theme() -> Theme {
    Theme::custom(
        "Halation".to_string(),
        iced::theme::Palette { background: BG_APP, text: TEXT_PRIMARY, primary: ACCENT, success: PICK, danger: REJECT },
    )
}

fn solid(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style { background: Some(Background::Color(color)), ..container::Style::default() }
}

fn framed(color: Color, border: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(color)),
        border: Border { color: border, width: 1.0, radius: RADIUS.into() },
        ..container::Style::default()
    }
}

fn button_style(active: bool, accent: Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let (bg, fg, border) = match status {
            button::Status::Active => (
                active.then_some(Color { a: 0.20, ..accent }),
                if active { TEXT_PRIMARY } else { TEXT_SECONDARY },
                if active { accent } else { BORDER },
            ),
            button::Status::Hovered => (Some(BG_HOVER), TEXT_PRIMARY, if active { accent } else { TEXT_DIM }),
            button::Status::Pressed => (Some(BG_INPUT), TEXT_PRIMARY, accent),
            button::Status::Disabled => (None, TEXT_DIM, BORDER),
        };
        button::Style {
            background: bg.map(Background::Color),
            text_color: fg,
            border: Border { color: border, width: 1.0, radius: RADIUS.into() },
            shadow: Shadow::default(),
        }
    }
}

fn row_style(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: match (active, status) {
            (true, _) => Some(Background::Color(BG_SELECTED)),
            (false, button::Status::Hovered) => Some(Background::Color(BG_HOVER)),
            _ => None,
        },
        text_color: if active { TEXT_PRIMARY } else { TEXT_SECONDARY },
        border: Border { color: Color::TRANSPARENT, width: 0.0, radius: RADIUS.into() },
        shadow: Shadow::default(),
    }
}

fn tile_style(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: Some(Background::Color(if status == button::Status::Hovered { BG_HOVER } else { BG_PANEL })),
        text_color: TEXT_SECONDARY,
        border: Border { color: if selected { ACCENT } else { BORDER }, width: if selected { 2.0 } else { 1.0 }, radius: RADIUS.into() },
        shadow: Shadow::default(),
    }
}

fn slider_style(modified: bool) -> impl Fn(&Theme, slider::Status) -> slider::Style {
    move |_, status| {
        let thumb = match status {
            slider::Status::Active => if modified { ACCENT } else { TEXT_SECONDARY },
            slider::Status::Hovered => TEXT_PRIMARY,
            slider::Status::Dragged => hex(0x5AA2F5),
        };
        slider::Style {
            rail: slider::Rail {
                backgrounds: (Background::Color(if modified { ACCENT } else { hex(0x4A4A4A) }), Background::Color(BG_INPUT)),
                width: 2.0,
                border: Border { radius: 1.0.into(), ..Border::default() },
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Circle { radius: 6.0 },
                background: Background::Color(thumb),
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
            },
        }
    }
}

fn label(s: impl Into<String>, size: u16, color: Color) -> Text<'static> {
    text(s.into()).size(size).color(color)
}

fn mono(s: impl Into<String>, size: u16, color: Color) -> Text<'static> {
    label(s, size, color).font(Font::MONOSPACE)
}

fn fa(name: &str, size: f32, color: Color) -> Element<'static, Message> {
    iced_font_awesome::fa_icon_solid::<Theme>(name).size(size).color(color).into()
}

fn fa_regular(name: &str, size: f32, color: Color) -> Element<'static, Message> {
    iced_font_awesome::fa_icon::<Theme>(name).size(size).color(color).into()
}

fn v_rule() -> Element<'static, Message> {
    container(Space::new(Length::Fixed(1.0), Length::Fill)).height(Length::Fill).style(solid(BORDER)).into()
}

fn h_rule() -> Element<'static, Message> {
    container(Space::new(Length::Fill, Length::Fixed(1.0))).width(Length::Fill).style(solid(BORDER)).into()
}

fn chip(s: &'static str, color: Color) -> Element<'static, Message> {
    container(label(s, 9, hex(0x101010))).padding([0, 4]).style(solid(color)).into()
}

// ------------------------------------------------------------------------------------------------
// Types
// ------------------------------------------------------------------------------------------------

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
    /// Wheel / pinch zoom: the exact scale lives in the view transform.
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Single,
    SplitSideBySide,
    SplitTopBottom,
    Wipe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Left,
    Right,
    Filmstrip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    WhiteBalance,
    Tone,
    Presence,
}

impl Section {
    const ALL: [Section; 3] = [Section::WhiteBalance, Section::Tone, Section::Presence];

    fn index(self) -> usize {
        self as usize
    }
    fn title(self) -> &'static str {
        match self {
            Section::WhiteBalance => "White Balance",
            Section::Tone => "Tone",
            Section::Presence => "Presence",
        }
    }
    fn ids(self) -> &'static [ParamId] {
        match self {
            Section::WhiteBalance => &ParamId::WHITE_BALANCE,
            Section::Tone => &ParamId::TONE,
            Section::Presence => &ParamId::PRESENCE,
        }
    }
}

/// A photo opened in Develop: metadata plus the working-space preview every render starts from.
#[derive(Debug)]
pub struct Document {
    id: u64,
    filename: String,
    metadata: RawMetadata,
    full_size: (u32, u32),
    preview: Arc<ImageBufferF32>,
}

#[derive(Debug, Clone)]
pub struct LoadError {
    id: u64,
    message: String,
}

#[derive(Debug)]
pub struct RenderResult {
    doc_id: u64,
    display: image::Handle,
    histogram: HistogramData,
    histogram_bitmap: image::Handle,
    original: Option<image::Handle>,
    used_gpu: bool,
    millis: f32,
}

#[derive(Debug, Clone, Default)]
pub struct ImportReport {
    first_id: Option<u64>,
    imported: usize,
    errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Noop,
    Navigate(Page),
    ImportDialog,
    FolderDialog,
    Picked(Vec<PathBuf>),
    Imported(ImportReport),
    Select(usize),
    Step(i32),
    Opened(Result<Arc<Document>, LoadError>),
    Rendered(Arc<RenderResult>),
    Thumbnail(u64, Option<image::Handle>),
    Param(ParamId, f32),
    CommitEdit,
    ResetParam(ParamId),
    ResetSection(Section),
    ResetAll,
    Undo,
    Redo,
    HistoryCheckout(usize),
    SnapshotAdd,
    SnapshotApply(usize),
    ToggleSection(Section),
    TogglePanel(Panel),
    FolderFilter(Option<PathBuf>),
    Rating(u8),
    Flag(&'static str),
    SetZoom(ZoomMode),
    ToggleZoom,
    SetViewMode(ViewMode),
    ToggleComparison,
    Divider(f32),
    /// Hold `\` to peek at the baseline.
    Peek(bool),
    View(ViewEvent),
    Curve(CurveEdit),
    Wheel(WheelId, Wheel),
    ToggleClipping,
    Ext(ExtMessage),
}

// ------------------------------------------------------------------------------------------------
// State
// ------------------------------------------------------------------------------------------------

pub struct State {
    catalog: Catalog,
    items: Vec<CatalogItem>,
    thumbs: HandleCache,
    thumb_failed: HashSet<u64>,
    thumb_pending: bool,
    selected: Option<usize>,
    loading: Option<u64>,
    doc: Option<Arc<Document>>,
    page: Page,
    params: DevelopParameters,
    history: HistoryGraph,
    snapshots: Vec<(String, DevelopParameters)>,
    viewport: Option<image::Handle>,
    original: Option<image::Handle>,
    original_for: Option<u64>,
    histogram: Option<HistogramData>,
    histogram_bitmap: Option<image::Handle>,
    render_in_flight: bool,
    render_dirty: bool,
    render_ms: f32,
    used_gpu: bool,
    gpu: GpuEngineInfo,
    gpu_engine: Option<Arc<GpuDevelopEngine>>,
    zoom: ZoomMode,
    view_mode: ViewMode,
    divider: f32,
    left_open: bool,
    right_open: bool,
    filmstrip_open: bool,
    sections: [bool; 3],
    folder_filter: Option<PathBuf>,
    theme: Theme,
    status: String,
    ext: ExtState,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    fn new() -> Self {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
            .unwrap_or_else(std::env::temp_dir);
        let catalog_path = base.join("Halation").join("catalog.halation-cat");
        if let Some(parent) = catalog_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let catalog = Catalog::open(&catalog_path)
            .or_else(|_| Catalog::open_in_memory())
            .expect("catalog initialization failed");
        let items = catalog.load_items();
        let gpu_engine = GpuDevelopEngine::new().ok().map(Arc::new);
        let gpu = gpu_engine.as_ref().map(|e| e.info().clone()).unwrap_or_else(GpuEngineInfo::fallback);
        let status = format!("{} photos in catalog", items.len());
        Self {
            catalog,
            items,
            thumbs: HandleCache::new(800),
            thumb_failed: HashSet::new(),
            thumb_pending: false,
            selected: None,
            loading: None,
            doc: None,
            page: Page::Library,
            params: DevelopParameters::default(),
            history: HistoryGraph::new(DevelopParameters::default(), "Imported"),
            snapshots: Vec::new(),
            viewport: None,
            original: None,
            original_for: None,
            histogram: None,
            histogram_bitmap: None,
            render_in_flight: false,
            render_dirty: false,
            render_ms: 0.0,
            used_gpu: false,
            gpu,
            gpu_engine,
            zoom: ZoomMode::Fit,
            view_mode: ViewMode::Single,
            divider: 0.5,
            left_open: true,
            right_open: true,
            filmstrip_open: true,
            sections: [true; 3],
            folder_filter: None,
            theme: darkroom_theme(),
            status,
            ext: ExtState::new(),
        }
    }

    /// Initial state plus the first thumbnail job.
    pub fn boot() -> (Self, Task<Message>) {
        let mut state = Self::new();
        let task = next_thumbnail(&mut state);
        (state, task)
    }
}

pub fn run() -> iced::Result {
    let icon = iced::window::icon::from_file_data(include_bytes!("../../../assets/favicon.png"), None)
        .expect("failed to load window icon");

    iced::application(title, update, view)
        .theme(|state: &State| state.theme.clone())
        .subscription(subscription)
        .window(iced::window::Settings { icon: Some(icon), ..Default::default() })
        .window_size(Size::new(1200.0, 600.0))
        .centered()
        .run_with(State::boot)
}

fn title(state: &State) -> String {
    match &state.doc {
        Some(doc) => format!("{} — Halation", doc.filename),
        None => "Halation".to_owned(),
    }
}

// ------------------------------------------------------------------------------------------------
// Async jobs
// ------------------------------------------------------------------------------------------------

async fn pick_files() -> Vec<PathBuf> {
    rfd::AsyncFileDialog::new()
        .add_filter("Photos", SUPPORTED_EXTENSIONS)
        .pick_files()
        .await
        .map(|files| files.into_iter().map(|f| f.path().to_path_buf()).collect())
        .unwrap_or_default()
}

async fn pick_folder() -> Vec<PathBuf> {
    match rfd::AsyncFileDialog::new().pick_folder().await {
        Some(dir) => scan_folder(dir.path()),
        None => Vec::new(),
    }
}

fn scan_folder(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|e| SUPPORTED_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    files
}

async fn import_paths(catalog: Catalog, paths: Vec<PathBuf>) -> ImportReport {
    let mut report = ImportReport::default();
    for path in paths {
        let metadata = read_metadata(&path);
        match catalog.import_asset(&path, &metadata) {
            Ok(id) => {
                report.imported += 1;
                report.first_id.get_or_insert(id);
            }
            Err(e) => report.errors.push(format!("{}: {e}", path.display())),
        }
    }
    report
}

async fn load_document(id: u64, path: PathBuf) -> Result<Arc<Document>, LoadError> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let asset = decode_file(&path).map_err(|e| LoadError { id, message: format!("Could not open {name}: {e}") })?;
    let full_size = (asset.buffer.width, asset.buffer.height);
    let preview = Arc::new(make_preview(&asset.buffer, PREVIEW_MAX));
    Ok(Arc::new(Document { id, filename: asset.filename, metadata: asset.metadata, full_size, preview }))
}

async fn make_thumbnail(id: u64, path: PathBuf) -> (u64, Option<image::Handle>) {
    match decode_thumbnail(&path, THUMB_MAX) {
        Ok(t) => (id, Some(image::Handle::from_rgba(t.width, t.height, t.rgba))),
        Err(_) => (id, None),
    }
}

async fn render_job(
    doc: Arc<Document>,
    params: DevelopParameters,
    gpu: Option<Arc<GpuDevelopEngine>>,
    with_original: bool,
    ctx: RenderContext,
) -> Arc<RenderResult> {
    let started = std::time::Instant::now();
    let out = process_with_context(&doc.preview, &params, gpu.as_deref(), &ctx);
    let display = image::Handle::from_rgba(out.width, out.height, out.display_rgba);
    let histogram_bitmap = image::Handle::from_rgba(HISTOGRAM_WIDTH, HISTOGRAM_HEIGHT, out.histogram_rgba);
    let original = with_original.then(|| {
        let baseline_ctx = RenderContext { display: ctx.display.clone(), mask: None, dither: true, clip_overlay: false };
        let o = process_with_context(&doc.preview, &DevelopParameters::default(), gpu.as_deref(), &baseline_ctx);
        image::Handle::from_rgba(o.width, o.height, o.display_rgba)
    });
    Arc::new(RenderResult {
        doc_id: doc.id,
        display,
        histogram: out.histogram,
        histogram_bitmap,
        original,
        used_gpu: out.used_gpu,
        millis: started.elapsed().as_secs_f32() * 1000.0,
    })
}

// ------------------------------------------------------------------------------------------------
// Update
// ------------------------------------------------------------------------------------------------

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    let task = update_inner(state, message);
    if std::mem::take(&mut state.ext.xmp_pending) {
        Task::batch([task, ext::schedule_xmp(state)])
    } else {
        task
    }
}

fn update_inner(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Noop => Task::none(),
        Message::Navigate(page) => {
            state.page = page;
            Task::none()
        }
        Message::ImportDialog => Task::perform(pick_files(), Message::Picked),
        Message::FolderDialog => Task::perform(pick_folder(), Message::Picked),
        Message::Picked(paths) => {
            if paths.is_empty() {
                return Task::none();
            }
            state.status = format!("Importing {} file(s)…", paths.len());
            Task::perform(import_paths(state.catalog.clone(), paths), Message::Imported)
        }
        Message::Imported(report) => {
            let selected_id = state.selected.and_then(|i| state.items.get(i)).map(|i| i.id);
            state.items = state.catalog.load_items();
            state.selected = selected_id.and_then(|id| state.items.iter().position(|i| i.id == id));
            state.status = if report.errors.is_empty() {
                format!("Imported {} photo(s)", report.imported)
            } else {
                format!("Imported {}; {} failed: {}", report.imported, report.errors.len(), report.errors[0])
            };
            let mut tasks = Vec::new();
            if let Some(index) = report.first_id.and_then(|id| state.items.iter().position(|i| i.id == id)) {
                tasks.push(open_photo(state, index));
            }
            tasks.push(next_thumbnail(state));
            Task::batch(tasks)
        }
        Message::Select(index) => open_photo(state, index),
        Message::Step(delta) => {
            let visible = visible_indices(state);
            if visible.is_empty() {
                return Task::none();
            }
            let pos = state.selected.and_then(|s| visible.iter().position(|&i| i == s));
            let next = match pos {
                Some(p) => (p as i32 + delta).clamp(0, visible.len() as i32 - 1) as usize,
                None => 0,
            };
            if Some(visible[next]) == state.selected {
                return Task::none();
            }
            open_photo(state, visible[next])
        }
        Message::Opened(Ok(doc)) => {
            if state.loading != Some(doc.id) {
                return Task::none();
            }
            state.loading = None;
            state.status = format!("{} · {}×{}", doc.filename, doc.full_size.0, doc.full_size.1);
            state.doc = Some(doc);
            ext::import_sidecar_if_external(state);
            let mask = ext::restore_mask(state);
            Task::batch([request_render(state), mask, ext::refresh_detail(state)])
        }
        Message::Opened(Err(e)) => {
            if state.loading == Some(e.id) {
                state.loading = None;
                state.status = e.message;
            }
            Task::none()
        }
        Message::Rendered(result) => {
            state.render_in_flight = false;
            if state.doc.as_ref().map(|d| d.id) == Some(result.doc_id) {
                state.viewport = Some(result.display.clone());
                state.histogram = Some(result.histogram.clone());
                state.histogram_bitmap = Some(result.histogram_bitmap.clone());
                if let Some(original) = &result.original {
                    state.original = Some(original.clone());
                    state.original_for = Some(result.doc_id);
                }
                state.render_ms = result.millis;
                state.used_gpu = result.used_gpu;
            }
            let again = if state.render_dirty { request_render(state) } else { Task::none() };
            Task::batch([again, ext::refresh_detail(state)])
        }
        Message::Thumbnail(id, handle) => {
            state.thumb_pending = false;
            match handle {
                Some(h) => {
                    state.thumbs.insert(id, h);
                }
                None => {
                    state.thumb_failed.insert(id);
                }
            }
            next_thumbnail(state)
        }
        Message::Param(id, slider_value) => {
            if state.doc.is_none() {
                return Task::none();
            }
            let value = id.from_slider(slider_value);
            if (id.get(&state.params) - value).abs() < 1.0e-6 {
                return Task::none();
            }
            id.set(&mut state.params, value);
            request_render(state)
        }
        Message::CommitEdit => {
            let (params, label, detail) = (state.params.clone(), None::<String>, String::new());
            commit(state, params, label, detail);
            Task::none()
        }
        Message::ResetParam(id) => {
            state.params.reset_param(id);
            let params = state.params.clone();
            commit(state, params, Some(format!("Reset {}", id.label())), String::new());
            request_render(state)
        }
        Message::ResetSection(section) => {
            for id in section.ids() {
                state.params.reset_param(*id);
            }
            let params = state.params.clone();
            commit(state, params, Some(format!("Reset {}", section.title())), String::new());
            request_render(state)
        }
        Message::ResetAll => {
            state.params = DevelopParameters::default();
            let params = state.params.clone();
            commit(state, params, Some("Reset all".into()), String::new());
            request_render(state)
        }
        Message::Undo => match state.history.undo().cloned() {
            Some(p) => apply_params(state, p),
            None => Task::none(),
        },
        Message::Redo => match state.history.redo().cloned() {
            Some(p) => apply_params(state, p),
            None => Task::none(),
        },
        Message::HistoryCheckout(id) => match state.history.checkout(id).cloned() {
            Some(p) => apply_params(state, p),
            None => Task::none(),
        },
        Message::SnapshotAdd => {
            if state.doc.is_some() {
                let name = format!("Snapshot {}", state.snapshots.len() + 1);
                state.snapshots.push((name, state.params.clone()));
            }
            Task::none()
        }
        Message::SnapshotApply(index) => {
            let Some((name, params)) = state.snapshots.get(index).cloned() else { return Task::none() };
            state.params = params.clone();
            commit(state, params, Some(format!("Snapshot: {name}")), String::new());
            request_render(state)
        }
        Message::ToggleSection(section) => {
            state.sections[section.index()] ^= true;
            Task::none()
        }
        Message::TogglePanel(panel) => {
            match panel {
                Panel::Left => state.left_open ^= true,
                Panel::Right => state.right_open ^= true,
                Panel::Filmstrip => state.filmstrip_open ^= true,
            }
            Task::none()
        }
        Message::FolderFilter(filter) => {
            state.folder_filter = filter;
            Task::none()
        }
        Message::Rating(rating) => {
            if let Some(i) = state.selected {
                let rating = rating.min(5);
                state.catalog.update_rating(state.items[i].id, rating);
                state.items[i].rating = rating;
                state.ext.xmp_pending = true;
            }
            Task::none()
        }
        Message::Flag(flag) => {
            if let Some(i) = state.selected {
                state.catalog.update_flag(state.items[i].id, flag);
                state.items[i].flag = flag.into();
                state.ext.xmp_pending = true;
            }
            Task::none()
        }
        Message::SetZoom(zoom) => {
            state.zoom = zoom;
            state.ext.view = viewport::ViewTransform { mode: zoom, scale: 1.0, center: [0.5, 0.5] };
            ext::refresh_detail(state)
        }
        Message::ToggleZoom => {
            let Some(doc) = state.doc.clone() else { return Task::none() };
            let pane = ext::pane_size(state);
            let anchor = iced::Point::new(pane.width * 0.5, pane.height * 0.5);
            state.ext.view = state.ext.view.toggled_at(doc.full_size, pane, anchor);
            state.zoom = state.ext.view.mode;
            ext::refresh_detail(state)
        }
        Message::SetViewMode(mode) => {
            state.view_mode = mode;
            state.ext.compare = match mode {
                ViewMode::Single => Compare::Off,
                ViewMode::SplitSideBySide => Compare::SideBySide,
                ViewMode::SplitTopBottom => Compare::TopBottom,
                ViewMode::Wipe => Compare::Wipe(state.divider),
            };
            Task::none()
        }
        Message::ToggleComparison => {
            let next = if state.view_mode == ViewMode::Single { ViewMode::Wipe } else { ViewMode::Single };
            update_inner(state, Message::SetViewMode(next))
        }
        Message::Divider(value) => {
            state.divider = value;
            if matches!(state.ext.compare, Compare::Wipe(_)) {
                state.ext.compare = Compare::Wipe(value);
            }
            Task::none()
        }
        Message::Peek(on) => {
            state.ext.peek = on && state.doc.is_some();
            Task::none()
        }
        Message::View(event) => ext::on_view_event(state, event),
        Message::Curve(edit) => ext::edit_curve(state, edit),
        Message::Wheel(id, wheel) => ext::set_wheel(state, id, wheel),
        Message::ToggleClipping => {
            state.ext.clip_overlay ^= true;
            request_render(state)
        }
        Message::Ext(m) => ext::update(state, m),
    }
}

fn open_photo(state: &mut State, index: usize) -> Task<Message> {
    let Some(item) = state.items.get(index) else { return Task::none() };
    let (id, path, filename, saved) = (item.id, item.path.clone(), item.filename.clone(), item.params.clone());
    state.ext.reset_for_new_photo();
    ext::history_open(state, id, &saved.sanitized());
    state.selected = Some(index);
    state.page = Page::Develop;
    state.doc = None;
    state.viewport = None;
    state.original = None;
    state.original_for = None;
    state.histogram = None;
    state.histogram_bitmap = None;
    state.loading = Some(id);
    state.status = format!("Opening {filename}…");
    Task::perform(load_document(id, path), Message::Opened)
}

fn request_render(state: &mut State) -> Task<Message> {
    let Some(doc) = state.doc.clone() else { return Task::none() };
    if state.render_in_flight {
        state.render_dirty = true;
        return Task::none();
    }
    state.render_in_flight = true;
    state.render_dirty = false;
    let with_original = state.original_for != Some(doc.id);
    let ctx = ext::render_context(state);
    Task::perform(render_job(doc, state.params.clone(), state.gpu_engine.clone(), with_original, ctx), Message::Rendered)
}

fn next_thumbnail(state: &mut State) -> Task<Message> {
    grid::request_visible(state)
}

/// Record `params` in the history DAG (auto-labelled unless `label` is given) and persist it.
fn commit(state: &mut State, params: DevelopParameters, label: Option<String>, detail: String) {
    ext::history_commit(state, params, label, detail);
}

/// Undo / redo / history click: the graph head already moved, persist it and re-render.
fn apply_params(state: &mut State, params: DevelopParameters) -> Task<Message> {
    state.params = params;
    ext::history_activate(state);
    Task::batch([request_render(state), ext::refresh_detail(state)])
}

fn visible_indices(state: &State) -> Vec<usize> {
    state
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| state.folder_filter.as_deref().map_or(true, |f| item.path.parent() == Some(f)) && grid::passes_cull(state, item))
        .map(|(i, _)| i)
        .collect()
}

// ------------------------------------------------------------------------------------------------
// Keyboard
// ------------------------------------------------------------------------------------------------

fn subscription(_state: &State) -> Subscription<Message> {
    Subscription::batch([
        keyboard::on_key_press(on_key),
        keyboard::on_key_release(on_key_release),
        iced::event::listen_with(on_window_event),
    ])
}

fn on_key_release(key: Key, _modifiers: keyboard::Modifiers) -> Option<Message> {
    match key.as_ref() {
        Key::Character("\\") => Some(Message::Peek(false)),
        _ => None,
    }
}

fn on_window_event(event: iced::Event, _status: iced::event::Status, _id: iced::window::Id) -> Option<Message> {
    match event {
        iced::Event::Window(iced::window::Event::Opened { position, size }) => Some(Message::Ext(ExtMessage::WindowOpened(position, size))),
        iced::Event::Window(iced::window::Event::Moved(p)) => Some(Message::Ext(ExtMessage::WindowMoved(p))),
        iced::Event::Window(iced::window::Event::Resized(s)) => Some(Message::Ext(ExtMessage::WindowResized(s))),
        _ => None,
    }
}

fn on_key(key: Key, modifiers: keyboard::Modifiers) -> Option<Message> {
    let command = modifiers.command();
    match key.as_ref() {
        Key::Named(Named::Tab) => Some(Message::TogglePanel(if modifiers.shift() { Panel::Right } else { Panel::Left })),
        Key::Named(Named::ArrowLeft) => Some(Message::Step(-1)),
        Key::Named(Named::ArrowRight) => Some(Message::Step(1)),
        Key::Named(Named::ArrowUp) => Some(Message::Ext(ExtMessage::Grid(GridMessage::Row(-1)))),
        Key::Named(Named::ArrowDown) => Some(Message::Ext(ExtMessage::Grid(GridMessage::Row(1)))),
        Key::Character(c) => match (c, command) {
            ("e" | "E", true) if modifiers.alt() => Some(Message::Ext(ExtMessage::ExternalOpen)),
            ("z" | "Z", true) => Some(if modifiers.shift() { Message::Redo } else { Message::Undo }),
            ("o" | "O", true) => Some(Message::ImportDialog),
            ("z" | "Z", false) => Some(Message::ToggleZoom),
            ("j" | "J", false) => Some(Message::ToggleClipping),
            ("6", false) => Some(Message::Ext(ExtMessage::Grid(GridMessage::Label("Red")))),
            ("7", false) => Some(Message::Ext(ExtMessage::Grid(GridMessage::Label("Yellow")))),
            ("8", false) => Some(Message::Ext(ExtMessage::Grid(GridMessage::Label("Green")))),
            ("9", false) => Some(Message::Ext(ExtMessage::Grid(GridMessage::Label("Blue")))),
            ("\\", false) => Some(Message::Peek(true)),
            ("/", false) => Some(Message::TogglePanel(Panel::Filmstrip)),
            ("p" | "P", false) => Some(Message::Flag("Pick")),
            ("x" | "X", false) => Some(Message::Flag("Reject")),
            ("u" | "U", false) => Some(Message::Flag("None")),
            (d @ ("0" | "1" | "2" | "3" | "4" | "5"), false) => d.parse().ok().map(Message::Rating),
            _ => None,
        },
        _ => None,
    }
}

// ------------------------------------------------------------------------------------------------
// View — shell
// ------------------------------------------------------------------------------------------------

pub fn view(state: &State) -> Element<'_, Message> {
    let body = match state.page {
        Page::Library => library_page(state),
        Page::Develop => develop_page(state),
    };
    container(column![top_bar(state), h_rule(), body, h_rule(), status_bar(state)])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(solid(BG_APP))
        .into()
}

fn action_button(icon_name: &'static str, text_label: &'static str, msg: Option<Message>) -> Element<'static, Message> {
    button(
        row![fa(icon_name, 11.0, TEXT_SECONDARY), label(text_label, 11, TEXT_SECONDARY)]
            .spacing(5)
            .align_y(Vertical::Center),
    )
    .padding([4, 10])
    .style(button_style(false, ACCENT))
    .on_press_maybe(msg)
    .into()
}

fn top_bar(state: &State) -> Element<'static, Message> {
    let tab = |name: &'static str, icon_name: &'static str, page: Page| -> Element<'static, Message> {
        button(
            row![fa(icon_name, 11.0, TEXT_PRIMARY), label(name, 12, TEXT_PRIMARY)]
                .spacing(5)
                .align_y(Vertical::Center),
        )
        .padding([5, 14])
        .style(button_style(state.page == page, ACCENT))
        .on_press(Message::Navigate(page))
        .into()
    };
    let can_undo = state.history.active().parent.is_some();
    let can_redo = !state.history.children(state.history.active_id()).is_empty();
    let bar = row![
        label("HALATION", 14, TEXT_PRIMARY),
        Space::with_width(Length::Fixed(18.0)),
        tab("Library", "images", Page::Library),
        tab("Develop", "sliders", Page::Develop),
        Space::with_width(Length::Fill),
        mono(format!("{} photos", state.items.len()), 10, TEXT_DIM),
        action_button("arrow-rotate-left", "Undo", can_undo.then_some(Message::Undo)),
        action_button("arrow-rotate-right", "Redo", can_redo.then_some(Message::Redo)),
        action_button("folder-open", "Open Folder…", Some(Message::FolderDialog)),
        action_button("file-import", "Import…", Some(Message::ImportDialog)),
    ]
    .spacing(8)
    .align_y(Vertical::Center);
    container(bar).padding([8, 14]).width(Length::Fill).style(solid(BG_PANEL)).into()
}

fn status_bar(state: &State) -> Element<'static, Message> {
    let right = if state.doc.is_some() {
        format!("{} · {:.1} ms · {}", if state.used_gpu { "GPU" } else { "CPU" }, state.render_ms, state.gpu.adapter_name)
    } else {
        state.gpu.adapter_name.clone()
    };
    container(row![label(state.status.clone(), 11, TEXT_SECONDARY), Space::with_width(Length::Fill), mono(right, 10, TEXT_DIM)])
        .padding([4, 14])
        .width(Length::Fill)
        .style(solid(BG_PANEL))
        .into()
}

fn develop_page(state: &State) -> Element<'static, Message> {
    let mut main = Row::new().height(Length::Fill);
    if state.left_open {
        main = main.push(left_panel(state)).push(v_rule());
    }
    main = main.push(center_panel(state));
    if state.right_open {
        main = main.push(v_rule()).push(right_panel(state));
    }
    let mut page = Column::new().push(main);
    if state.filmstrip_open {
        page = page.push(h_rule()).push(filmstrip(state));
    }
    page.height(Length::Fill).into()
}

// ------------------------------------------------------------------------------------------------
// View — left panel
// ------------------------------------------------------------------------------------------------

fn section_block(title: &str, action: Option<Element<'static, Message>>, body: Element<'static, Message>) -> Element<'static, Message> {
    let mut head = Row::new().align_y(Vertical::Center).push(label(title, 10, TEXT_DIM)).push(Space::with_width(Length::Fill));
    if let Some(action) = action {
        head = head.push(action);
    }
    column![
        container(head).padding([6, 12]).width(Length::Fill).style(solid(BG_HEADER)),
        container(body).padding([8, 10]).width(Length::Fill)
    ]
    .into()
}

fn left_panel(state: &State) -> Element<'static, Message> {
    let zoom_name = match state.zoom {
        ZoomMode::Fit => "Fit",
        ZoomMode::Fill => "Fill",
        ZoomMode::Actual => "1:1",
        ZoomMode::Custom => "Zoom",
    };
    let nav_body: Element<'static, Message> = match (&state.viewport, state.doc.as_ref()) {
        (Some(handle), Some(doc)) => container(
            Navigator { thumbnail: Some(handle.clone()), image_size: doc.full_size, view: state.ext.view, pane: ext::pane_size(state) }.view(LEFT_W - 28.0),
        )
        .padding(2)
        .style(framed(BG_CANVAS, BORDER))
        .into(),
        _ => container(label("No image", 11, TEXT_DIM)).padding(14).into(),
    };
    let navigator = section_block("NAVIGATOR", Some(mono(zoom_name, 10, TEXT_SECONDARY).into()), nav_body);
    let content = column![navigator, history_section(state), snapshots_section(state), folders_section(state)];
    container(scrollable(content).height(Length::Fill))
        .width(Length::Fixed(LEFT_W))
        .height(Length::Fill)
        .style(solid(BG_PANEL))
        .into()
}

fn history_section(state: &State) -> Element<'static, Message> {
    let active = state.history.active_id();
    let mut list = Column::new().spacing(1);
    for node in state.history.nodes().iter().rev().take(200) {
        let current = node.id == active;
        let color = if current {
            TEXT_PRIMARY
        } else if state.history.on_active_path(node.id) {
            TEXT_SECONDARY
        } else {
            TEXT_DIM
        };
        let line = row![
            Space::with_width(Length::Fixed(state.history.lane(node.id) as f32 * 10.0)),
            label(if current { "●" } else { "○" }, 9, if current { ACCENT } else { TEXT_DIM }),
            label(node.label.clone(), 11, color),
            Space::with_width(Length::Fill),
            mono(node.detail.clone(), 9, TEXT_DIM),
        ]
        .spacing(6)
        .align_y(Vertical::Center);
        list = list.push(
            button(line).width(Length::Fill).padding([4, 8]).style(row_style(current)).on_press(Message::HistoryCheckout(node.id)),
        );
    }
    section_block("HISTORY", None, list.into())
}

fn snapshots_section(state: &State) -> Element<'static, Message> {
    let add = button(label("+", 12, TEXT_SECONDARY))
        .padding([0, 7])
        .style(button_style(false, ACCENT))
        .on_press_maybe(state.doc.is_some().then_some(Message::SnapshotAdd));
    let mut list = Column::new().spacing(1);
    if state.snapshots.is_empty() {
        list = list.push(label("No snapshots", 11, TEXT_DIM));
    }
    for (i, (name, _)) in state.snapshots.iter().enumerate() {
        list = list.push(
            button(label(name.clone(), 11, TEXT_SECONDARY))
                .width(Length::Fill)
                .padding([4, 8])
                .style(row_style(false))
                .on_press(Message::SnapshotApply(i)),
        );
    }
    section_block("SNAPSHOTS", Some(add.into()), list.into())
}

fn folders_section(state: &State) -> Element<'static, Message> {
    let mut counts: BTreeMap<PathBuf, usize> = BTreeMap::new();
    for item in &state.items {
        if let Some(parent) = item.path.parent() {
            *counts.entry(parent.to_path_buf()).or_default() += 1;
        }
    }
    let entry = |name: String, count: usize, filter: Option<PathBuf>| -> Element<'static, Message> {
        let active = state.folder_filter == filter;
        button(row![label(name, 11, TEXT_SECONDARY), Space::with_width(Length::Fill), mono(count.to_string(), 10, TEXT_DIM)])
            .width(Length::Fill)
            .padding([4, 8])
            .style(row_style(active))
            .on_press(Message::FolderFilter(filter))
            .into()
    };
    let mut list = Column::new().spacing(1).push(entry("All photographs".into(), state.items.len(), None));
    for (dir, count) in counts {
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string());
        let parent = dir.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
        let display = parent.map_or(name.clone(), |p| format!("{p}/{name}"));
        list = list.push(entry(display, count, Some(dir)));
    }
    section_block("FOLDERS", None, list.into())
}

// ------------------------------------------------------------------------------------------------
// View — center (loupe)
// ------------------------------------------------------------------------------------------------

fn center_panel(state: &State) -> Element<'static, Message> {
    column![
        container(viewport_view(state)).width(Length::Fill).height(Length::Fill).style(solid(BG_CANVAS)),
        h_rule(),
        viewport_toolbar(state),
        h_rule(),
        exif_hud(state)
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn note(s: &'static str) -> Element<'static, Message> {
    container(label(s, 13, TEXT_DIM)).width(Length::Fill).height(Length::Fill).align_x(Horizontal::Center).align_y(Vertical::Center).into()
}

fn empty_state() -> Element<'static, Message> {
    let body = column![
        label("No photos yet", 24, TEXT_PRIMARY),
        label("Import JPEG, PNG, TIFF, WebP or DNG files — originals stay untouched.", 12, TEXT_DIM),
        row![
            action_button("file-import", "Import Photos…", Some(Message::ImportDialog)),
            action_button("folder-open", "Open Folder…", Some(Message::FolderDialog))
        ]
        .spacing(8),
    ]
    .spacing(12)
    .align_x(Horizontal::Center);
    container(body).width(Length::Fill).height(Length::Fill).align_x(Horizontal::Center).align_y(Vertical::Center).into()
}

fn viewer(handle: image::Handle, fit: ContentFit) -> Element<'static, Message> {
    image::Viewer::new(handle)
        .width(Length::Fill)
        .height(Length::Fill)
        .content_fit(fit)
        .min_scale(1.0)
        .max_scale(8.0)
        .scale_step(0.25)
        .into()
}

fn pane(content: Element<'static, Message>, caption: &'static str, portion: u16, vertical: bool) -> Element<'static, Message> {
    let tag = container(label(caption, 9, TEXT_PRIMARY)).padding([2, 6]).style(solid(Color { a: 0.7, ..BG_APP }));
    let layered = container(stack![content, container(tag).padding(8)]);
    if vertical {
        layered.width(Length::Fill).height(Length::FillPortion(portion)).into()
    } else {
        layered.width(Length::FillPortion(portion)).height(Length::Fill).into()
    }
}

fn canvas_view(state: &State) -> Element<'static, Message> {
    let Some(edited) = state.viewport.clone() else {
        return if state.loading.is_some() {
            note("Loading…")
        } else if state.items.is_empty() {
            empty_state()
        } else {
            note("Select a photo from the filmstrip")
        };
    };
    let fit = match state.zoom {
        ZoomMode::Fit | ZoomMode::Custom => ContentFit::Contain,
        ZoomMode::Fill => ContentFit::Cover,
        ZoomMode::Actual => ContentFit::None,
    };
    let before = state.original.clone().unwrap_or_else(|| edited.clone());
    let left = (state.divider * 100.0) as u16;
    let right = ((1.0 - state.divider) * 100.0) as u16;
    match state.view_mode {
        ViewMode::Single => viewer(edited, fit),
        ViewMode::SplitSideBySide => row![
            pane(viewer(before, ContentFit::Contain), "BEFORE", left, false),
            v_rule(),
            pane(viewer(edited, ContentFit::Contain), "AFTER", right, false)
        ]
        .height(Length::Fill)
        .into(),
        ViewMode::SplitTopBottom => column![
            pane(viewer(before, ContentFit::Contain), "BEFORE", left, true),
            h_rule(),
            pane(viewer(edited, ContentFit::Contain), "AFTER", right, true)
        ]
        .width(Length::Fill)
        .into(),
        ViewMode::Wipe => row![
            pane(viewer(before, ContentFit::Contain), "BEFORE", left, false),
            v_rule(),
            pane(viewer(edited, ContentFit::Contain), "AFTER", right, false)
        ]
        .height(Length::Fill)
        .into(),
    }
}

/// The loupe: canvas viewport once an image is rendered, otherwise the loading / empty-state notes.
fn viewport_view(state: &State) -> Element<'static, Message> {
    let (Some(edited), Some(doc)) = (state.viewport.clone(), state.doc.as_ref()) else {
        return canvas_view(state);
    };
    Viewport {
        edited,
        before: state.original.clone(),
        detail: state.ext.detail.as_ref().filter(|(id, _)| *id == doc.id).map(|(_, d)| d.clone()),
        image_size: doc.full_size,
        view: state.ext.view,
        compare: if state.ext.peek { Compare::Peek } else { state.ext.compare },
        background: BG_CANVAS,
    }
    .view()
}

fn section_header_plain(title: &str) -> Element<'static, Message> {
    label(title, 10, TEXT_DIM).into()
}

fn viewport_toolbar(state: &State) -> Element<'static, Message> {
    let zoom_btn = |name: &'static str, mode: ZoomMode| -> Element<'static, Message> {
        button(label(name, 11, TEXT_PRIMARY)).padding([3, 9]).style(button_style(state.zoom == mode, ACCENT)).on_press(Message::SetZoom(mode)).into()
    };
    let view_btn = |name: &'static str, mode: ViewMode| -> Element<'static, Message> {
        button(label(name, 11, TEXT_PRIMARY)).padding([3, 9]).style(button_style(state.view_mode == mode, ACCENT)).on_press(Message::SetViewMode(mode)).into()
    };
    let mut bar = Row::new()
        .spacing(5)
        .align_y(Vertical::Center)
        .push(zoom_btn("Fit", ZoomMode::Fit))
        .push(zoom_btn("Fill", ZoomMode::Fill))
        .push(zoom_btn("1:1", ZoomMode::Actual))
        .push(Space::with_width(Length::Fixed(14.0)))
        .push(view_btn("Single", ViewMode::Single))
        .push(view_btn("Before | After", ViewMode::SplitSideBySide))
        .push(view_btn("Before / After", ViewMode::SplitTopBottom))
        .push(view_btn("Wipe", ViewMode::Wipe));
    if state.view_mode != ViewMode::Single {
        bar = bar.push(slider(0.2_f32..=0.8_f32, state.divider, Message::Divider).step(0.01_f32).width(Length::Fixed(120.0)));
    }
    bar = bar.push(Space::with_width(Length::Fill)).push(mono("Z zoom  ·  \\ compare  ·  scroll to zoom", 10, TEXT_DIM));
    container(bar).padding([6, 12]).width(Length::Fill).style(solid(BG_PANEL)).into()
}

fn exif_line(m: &RawMetadata) -> String {
    let mut parts = Vec::new();
    if let Some(iso) = m.iso {
        parts.push(format!("ISO {iso}"));
    }
    let lens = if m.lens.is_empty() { m.focal_length.clone() } else { m.lens.clone() };
    for part in [lens, m.aperture.clone(), m.shutter.clone(), m.camera.clone()] {
        if !part.is_empty() {
            parts.push(part);
        }
    }
    if parts.is_empty() {
        "No EXIF metadata".to_owned()
    } else {
        parts.join("  |  ")
    }
}

fn exif_hud(state: &State) -> Element<'static, Message> {
    let line = state.doc.as_ref().map(|d| exif_line(&d.metadata)).unwrap_or_else(|| "—".to_owned());
    container(mono(line, 11, TEXT_SECONDARY)).padding([6, 12]).width(Length::Fill).style(solid(BG_PANEL)).into()
}

// ------------------------------------------------------------------------------------------------
// View — right panel
// ------------------------------------------------------------------------------------------------

fn clip_badge(on: bool, color: Color) -> Element<'static, Message> {
    label("▲", 10, if on { color } else { hex(0x3A3A3A) }).into()
}

fn histogram_block(state: &State) -> Element<'static, Message> {
    let (shadows, highlights) = state.histogram.as_ref().map(|h| (h.clipped_shadows(), h.clipped_highlights())).unwrap_or((0.0, 0.0));
    let head = row![
        label("HISTOGRAM", 10, TEXT_DIM),
        Space::with_width(Length::Fill),
        clip_badge(shadows > 0.001, CLIP_SHADOW),
        clip_badge(highlights > 0.001, CLIP_HIGHLIGHT)
    ]
    .spacing(6);
    let value = |id: ParamId| id.get(&state.params);
    let graph: Element<'static, Message> = HistogramView {
        data: state.histogram.clone(),
        values: [value(ParamId::Blacks), value(ParamId::Shadows), value(ParamId::Exposure), value(ParamId::Highlights), value(ParamId::Whites)],
        clip_overlay: state.ext.clip_overlay,
    }
    .view(96.0);
    let mut info = Column::new().spacing(2);
    if let Some(doc) = &state.doc {
        let m = &doc.metadata;
        info = info
            .push(label(if m.camera.is_empty() { doc.filename.clone() } else { m.camera.clone() }, 12, TEXT_HEADING))
            .push(label(if m.lens.is_empty() { "—".to_owned() } else { m.lens.clone() }, 10, TEXT_DIM))
            .push(mono(
                format!(
                    "ISO {}  ·  {}  ·  {}  ·  {}×{}",
                    m.iso.map(|v| v.to_string()).unwrap_or_else(|| "—".into()),
                    if m.aperture.is_empty() { "—" } else { &m.aperture },
                    if m.shutter.is_empty() { "—" } else { &m.shutter },
                    doc.full_size.0,
                    doc.full_size.1
                ),
                10,
                TEXT_SECONDARY,
            ));
    }
    container(column![head, graph, info].spacing(8)).padding(12).width(Length::Fill).into()
}

fn tool_rail() -> Element<'static, Message> {
    let tool = |name: &'static str, msg: Option<Message>| -> Element<'static, Message> {
        button(label(name, 11, TEXT_PRIMARY))
            .padding([4, 10])
            .style(button_style(msg.is_some(), ACCENT))
            .on_press_maybe(msg)
            .into()
    };
    container(
        row![tool("Edit", Some(Message::Noop)), tool("Crop", None), tool("Heal", None), tool("Mask", None), tool("Presets", None)].spacing(4),
    )
    .padding([8, 12])
    .width(Length::Fill)
    .into()
}

fn param_row(state: &State, id: ParamId) -> Element<'static, Message> {
    Scrubby::view(id, id.get(&state.params))
}

fn section_header(state: &State, section: Section) -> Element<'static, Message> {
    let open = state.sections[section.index()];
    let modified = section.ids().iter().any(|id| !state.params.is_default_param(*id));
    let title = row![
        fa(if open { "chevron-down" } else { "chevron-right" }, 10.0, TEXT_DIM),
        label(section.title(), 12, TEXT_HEADING),
        label(if modified { "•" } else { "" }, 12, ACCENT)
    ]
    .spacing(8);
    let head = button(container(title).width(Length::Fill))
        .width(Length::Fill)
        .padding([7, 12])
        .style(row_style(false))
        .on_press(Message::ToggleSection(section));
    let reset = button(fa("rotate-left", 11.0, TEXT_SECONDARY))
        .padding([6, 8])
        .style(row_style(false))
        .on_press_maybe(modified.then_some(Message::ResetSection(section)));
    container(row![head, reset].align_y(Vertical::Center)).width(Length::Fill).style(solid(BG_HEADER)).into()
}

fn right_panel(state: &State) -> Element<'static, Message> {
    let mut controls = Column::new().push(container(label("BASIC", 10, TEXT_DIM)).padding([8, 12]));
    for section in Section::ALL {
        controls = controls.push(section_header(state, section));
        if state.sections[section.index()] {
            let mut rows = Column::new().spacing(10);
            for id in section.ids() {
                rows = rows.push(param_row(state, *id));
            }
            controls = controls.push(container(rows).padding(Padding { top: 10.0, right: 12.0, bottom: 14.0, left: 12.0 }));
        }
    }
    controls = controls.push(
        container(
            row![
                action_button("rotate-left", "Reset all", (!state.params.is_default() && state.doc.is_some()).then_some(Message::ResetAll)),
                action_button("code-compare", "Before / After", state.doc.is_some().then_some(Message::ToggleComparison))
            ]
            .spacing(8),
        )
        .padding(12),
    );
    let body: Element<'static, Message> = match tools::tool_content(state) {
        Some(tool) => scrollable(tool).height(Length::Fill).into(),
        None => scrollable(controls).height(Length::Fill).into(),
    };
    container(column![histogram_block(state), h_rule(), tools::tool_tabs(state), h_rule(), body])
        .width(Length::Fixed(RIGHT_W))
        .height(Length::Fill)
        .style(solid(BG_PANEL))
        .into()
}

// ------------------------------------------------------------------------------------------------
// View — filmstrip & library
// ------------------------------------------------------------------------------------------------

fn flag_button(name: &'static str, flag: &'static str, current: &str, color: Color, enabled: bool) -> Element<'static, Message> {
    button(label(name, 10, TEXT_PRIMARY))
        .padding([2, 8])
        .style(button_style(current == flag && flag != "None", color))
        .on_press_maybe(enabled.then_some(Message::Flag(flag)))
        .into()
}

fn filmstrip(state: &State) -> Element<'static, Message> {
    let visible = visible_indices(state);
    let current = state.selected.and_then(|i| state.items.get(i));
    let rating = current.map(|i| i.rating).unwrap_or(0);
    let flag = current.map(|i| i.flag.clone()).unwrap_or_default();
    let enabled = current.is_some();

    let mut stars = Row::new().spacing(1);
    for n in 1..=5u8 {
        let filled = n <= rating;
        stars = stars.push(
            button(if filled {
                fa("star", 13.0, STAR)
            } else {
                fa_regular("star", 13.0, TEXT_DIM)
            })
            .padding([0, 2])
            .style(row_style(false))
            .on_press_maybe(enabled.then_some(Message::Rating(if rating == n { 0 } else { n }))),
        );
    }
    let position = state.selected.and_then(|s| visible.iter().position(|&i| i == s)).map(|p| p + 1).unwrap_or(0);
    let header = row![
        stars,
        Space::with_width(Length::Fixed(10.0)),
        flag_button("Pick", "Pick", &flag, PICK, enabled),
        flag_button("Reject", "Reject", &flag, REJECT, enabled),
        flag_button("Clear", "None", &flag, TEXT_DIM, enabled),
        Space::with_width(Length::Fill),
        mono(format!("{position}/{}", visible.len()), 10, TEXT_DIM)
    ]
    .spacing(6)
    .align_y(Vertical::Center);

    let mut strip = Row::new().spacing(6).align_y(Vertical::Center);
    for idx in visible {
        strip = strip.push(thumb_tile(state, idx));
    }
    let scroller = scrollable(strip)
        .direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::new().width(5).scroller_width(5)))
        .width(Length::Fill);

    container(column![container(header).padding([4, 12]), container(scroller).padding([0, 12])].spacing(2))
        .width(Length::Fill)
        .height(Length::Fixed(FILMSTRIP_H))
        .style(solid(BG_PANEL))
        .into()
}

fn thumb_picture(state: &State, item: &CatalogItem, width: f32, height: f32) -> Element<'static, Message> {
    match state.thumbs.get(&item.id) {
        Some(handle) => image(handle.clone())
            .width(Length::Fixed(width))
            .height(Length::Fixed(height))
            .content_fit(ContentFit::Contain)
            .into(),
        None => container(label(if state.thumb_failed.contains(&item.id) { "No preview" } else { "…" }, 9, TEXT_DIM))
            .width(Length::Fixed(width))
            .height(Length::Fixed(height))
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .style(solid(BG_INPUT))
            .into(),
    }
}

fn flag_chip(flag: &str) -> Option<Element<'static, Message>> {
    match flag {
        "Pick" => Some(chip("P", PICK)),
        "Reject" => Some(chip("R", REJECT)),
        _ => None,
    }
}

fn thumb_tile(state: &State, idx: usize) -> Element<'static, Message> {
    let item = &state.items[idx];
    let mut caption = Row::new().align_y(Vertical::Center).width(Length::Fixed(96.0));
    caption = caption.push(label("★".repeat(item.rating as usize), 9, STAR)).push(Space::with_width(Length::Fill));
    if let Some(c) = flag_chip(&item.flag) {
        caption = caption.push(c);
    }
    button(column![thumb_picture(state, item, 96.0, 58.0), caption].spacing(3))
        .padding(4)
        .style(tile_style(state.selected == Some(idx)))
        .on_press(Message::Select(idx))
        .into()
}

fn library_page(state: &State) -> Element<'static, Message> {
    if state.items.is_empty() {
        return empty_state();
    }
    grid::grid_page(state)
}

#[allow(dead_code)]
fn library_list(state: &State) -> Element<'static, Message> {
    let mut list = Column::new().spacing(4);
    for idx in visible_indices(state) {
        let item = &state.items[idx];
        let selected = state.selected == Some(idx);
        let mut line = Row::new()
            .spacing(12)
            .align_y(Vertical::Center)
            .push(thumb_picture(state, item, 72.0, 48.0))
            .push(
                column![
                    label(item.filename.clone(), 13, TEXT_PRIMARY),
                    mono(
                        format!(
                            "{:.1} MB  ·  {}",
                            item.file_size as f32 / 1_000_000.0,
                            item.path.parent().map(|p| p.display().to_string()).unwrap_or_default()
                        ),
                        10,
                        TEXT_DIM
                    )
                ]
                .spacing(3),
            )
            .push(Space::with_width(Length::Fill))
            .push(label("★".repeat(item.rating as usize), 12, STAR));
        if let Some(c) = flag_chip(&item.flag) {
            line = line.push(c);
        }
        list = list.push(button(line).width(Length::Fill).padding(6).style(row_style(selected)).on_press(Message::Select(idx)));
    }
    container(scrollable(column![label("PHOTO LIBRARY", 11, TEXT_DIM), list].spacing(10)))
        .padding(24)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
