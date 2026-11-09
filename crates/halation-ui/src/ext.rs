//! Session glue for Sprints 3–8. Lives next to `State` (a child module, so it may touch its private
//! fields) and keeps `lib.rs` readable:
//!
//! * Sprint 3 — view transform, native-resolution detail layer for true 1:1.
//! * Sprint 5 — monitor profile discovery → display LUT, re-probed when the window moves.
//! * Sprint 7 — persistent branching history (SQLite) and debounced XMP sidecar writes / import.
//! * Sprint 4 — tone-curve and colour-wheel edits.
//! * Sprint 8 — AI + gradient masks, batch export, external editor hand-off.

use super::{request_render, visible_indices, Message, State};
use crate::{
    viewport::{Compare, Detail, ViewEvent, ViewTransform},
    widgets::{CurveEdit, WheelId},
};
use halation_catalog::xmp::{self, XmpState};
use halation_color::{DisplayProfile, OutputSpace};
use halation_core::{AiMaskKind, CurveChannel, DevelopParameters, HistoryGraph, HistoryNode, ImageBufferF32, LocalAdjustment, MaskBuffer, MaskSource, Wheel};
use halation_gpu::GpuDevelopEngine;
use halation_ml::{
    has_models,
    mask::{rasterize, Prompt},
    Embedding, VisionEngine,
};
use halation_pipeline::{
    crop_region,
    export::{self, ExportFormat, ExportJob, ExportProgress as Progress, ExportSettings, Watermark, WatermarkKind},
    external::{self, ExternalEditor},
    process_buffer, process_with_context, DisplayTransform, RenderContext,
};
use iced::{Point, Size, Task};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

// ------------------------------------------------------------------------------------------------
// Small enums
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Edit,
    Curve,
    Color,
    Mask,
    Export,
}

impl Tool {
    pub const ALL: [Tool; 5] = [Tool::Edit, Tool::Curve, Tool::Color, Tool::Mask, Tool::Export];
    pub fn label(self) -> &'static str {
        match self {
            Tool::Edit => "Edit",
            Tool::Curve => "Curve",
            Tool::Color => "Color",
            Tool::Mask => "Mask",
            Tool::Export => "Export",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Tool::Edit => "sliders",
            Tool::Curve => "chart-line",
            Tool::Color => "palette",
            Tool::Mask => "mask",
            Tool::Export => "file-export",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientKind {
    Linear,
    Radial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalField {
    Exposure,
    Contrast,
    Highlights,
    Shadows,
    Saturation,
    Temperature,
}

impl LocalField {
    pub const ALL: [LocalField; 6] =
        [LocalField::Exposure, LocalField::Contrast, LocalField::Highlights, LocalField::Shadows, LocalField::Saturation, LocalField::Temperature];
    pub fn label(self) -> &'static str {
        match self {
            LocalField::Exposure => "Exposure",
            LocalField::Contrast => "Contrast",
            LocalField::Highlights => "Highlights",
            LocalField::Shadows => "Shadows",
            LocalField::Saturation => "Saturation",
            LocalField::Temperature => "Warmth",
        }
    }
    pub fn range(self) -> (f32, f32) {
        if self == LocalField::Exposure { (-5.0, 5.0) } else { (-100.0, 100.0) }
    }
    pub fn get(self, l: &LocalAdjustment) -> f32 {
        match self {
            LocalField::Exposure => l.exposure,
            LocalField::Contrast => l.contrast,
            LocalField::Highlights => l.highlights,
            LocalField::Shadows => l.shadows,
            LocalField::Saturation => l.saturation,
            LocalField::Temperature => l.temperature,
        }
    }
    pub fn set(self, l: &mut LocalAdjustment, v: f32) {
        match self {
            LocalField::Exposure => l.exposure = v,
            LocalField::Contrast => l.contrast = v,
            LocalField::Highlights => l.highlights = v,
            LocalField::Shadows => l.shadows = v,
            LocalField::Saturation => l.saturation = v,
            LocalField::Temperature => l.temperature = v,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatChoice {
    Jpeg,
    Png8,
    Png16,
    Tiff8,
    Tiff16,
    WebP,
    Dng,
}

impl FormatChoice {
    pub const ALL: [FormatChoice; 7] =
        [FormatChoice::Jpeg, FormatChoice::Png8, FormatChoice::Png16, FormatChoice::Tiff8, FormatChoice::Tiff16, FormatChoice::WebP, FormatChoice::Dng];
    pub fn label(self) -> &'static str {
        match self {
            FormatChoice::Jpeg => "JPEG",
            FormatChoice::Png8 => "PNG 8",
            FormatChoice::Png16 => "PNG 16",
            FormatChoice::Tiff8 => "TIFF 8",
            FormatChoice::Tiff16 => "TIFF 16",
            FormatChoice::WebP => "WebP",
            FormatChoice::Dng => "Linear DNG",
        }
    }
    pub fn to_format(self, quality: u8) -> ExportFormat {
        match self {
            FormatChoice::Jpeg => ExportFormat::Jpeg { quality },
            FormatChoice::Png8 => ExportFormat::Png { sixteen_bit: false },
            FormatChoice::Png16 => ExportFormat::Png { sixteen_bit: true },
            FormatChoice::Tiff8 => ExportFormat::Tiff { sixteen_bit: false },
            FormatChoice::Tiff16 => ExportFormat::Tiff { sixteen_bit: true },
            FormatChoice::WebP => ExportFormat::WebP,
            FormatChoice::Dng => ExportFormat::LinearDng,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportScope {
    Current,
    Visible,
}

// Wrappers so foreign types can travel inside `Debug` messages.
#[derive(Clone)]
pub struct VisionHandle(pub Arc<VisionEngine>);
impl fmt::Debug for VisionHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VisionEngine({})", self.0.provider())
    }
}

#[derive(Clone)]
pub struct EmbeddingHandle(pub Embedding);
impl fmt::Debug for EmbeddingHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Embedding(image {})", self.0.image_id)
    }
}

#[derive(Debug)]
pub struct DetailResult {
    doc_id: u64,
    rect: [f32; 4],
    handle: iced::widget::image::Handle,
}

// ------------------------------------------------------------------------------------------------
// State
// ------------------------------------------------------------------------------------------------

pub enum VisionState {
    Idle,
    Loading,
    Ready(Arc<VisionEngine>),
    Unavailable(String),
}

pub struct ExportUi {
    pub format: FormatChoice,
    pub quality: u8,
    pub space: OutputSpace,
    pub max_edge: Option<u32>,
    pub scope: ExportScope,
    pub logo: Option<PathBuf>,
    pub running: bool,
    pub progress: Option<Progress>,
    pub message: String,
}

pub struct ExternalUi {
    pub editor: Option<ExternalEditor>,
    pub stop: Arc<AtomicBool>,
    pub watching: bool,
}

pub struct ExtState {
    pub view: ViewTransform,
    pub compare: Compare,
    pub peek: bool,
    pub detail: Option<(u64, Detail)>,
    detail_busy: bool,
    detail_dirty: bool,
    full: Option<(u64, Arc<ImageBufferF32>)>,
    full_loading: Option<u64>,
    pub tool: Tool,
    pub curve_channel: CurveChannel,
    pub clip_overlay: bool,
    pub display: Option<Arc<DisplayTransform>>,
    pub display_name: String,
    display_gen: u64,
    pub window: Size,
    window_pos: Point,
    history_db: Vec<i64>,
    xmp_gen: u64,
    /// Set whenever catalog state worth mirroring to the sidecar changed; `update` then debounces a write.
    pub xmp_pending: bool,
    pub vision: VisionState,
    embedding: Option<(u64, Embedding)>,
    embedding_busy: bool,
    pending_ai: Option<AiMaskKind>,
    pub mask: Option<Arc<MaskBuffer>>,
    pub mask_busy: bool,
    pub export: ExportUi,
    pub external: ExternalUi,
    pub grid: crate::grid::GridState,
}

impl ExtState {
    pub fn new() -> Self {
        Self {
            view: ViewTransform::default(),
            compare: Compare::Off,
            peek: false,
            detail: None,
            detail_busy: false,
            detail_dirty: false,
            full: None,
            full_loading: None,
            tool: Tool::Edit,
            curve_channel: CurveChannel::Rgb,
            clip_overlay: false,
            display: None,
            display_name: "sRGB (built-in)".into(),
            display_gen: 0,
            window: Size::new(1600.0, 960.0),
            window_pos: Point::ORIGIN,
            history_db: Vec::new(),
            xmp_gen: 0,
            xmp_pending: false,
            vision: VisionState::Idle,
            embedding: None,
            embedding_busy: false,
            pending_ai: None,
            mask: None,
            mask_busy: false,
            export: ExportUi {
                format: FormatChoice::Jpeg,
                quality: 92,
                space: OutputSpace::Srgb,
                max_edge: None,
                scope: ExportScope::Current,
                logo: None,
                running: false,
                progress: None,
                message: String::new(),
            },
            external: ExternalUi { editor: None, stop: Arc::new(AtomicBool::new(false)), watching: false },
            grid: crate::grid::GridState::new(),
        }
    }

    /// Forget everything tied to the previously opened photo.
    pub fn reset_for_new_photo(&mut self) {
        self.view = ViewTransform::default();
        self.detail = None;
        self.full = None;
        self.full_loading = None;
        self.embedding = None;
        self.pending_ai = None;
        self.mask = None;
        self.history_db.clear();
    }
}

// ------------------------------------------------------------------------------------------------
// Messages
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ExtMessage {
    FullLoaded(u64, Result<Arc<ImageBufferF32>, String>),
    DetailRendered(Arc<DetailResult>),
    WindowOpened(Option<Point>, Size),
    WindowMoved(Point),
    WindowResized(Size),
    ProbeDisplay(u64),
    DisplayReady(Option<Arc<DisplayTransform>>, String),
    XmpDue(u64),
    XmpWritten(Result<PathBuf, String>),
    SetTool(Tool),
    CurveChannel(CurveChannel),
    CurveReset,
    WheelLum(WheelId, f32),
    WheelReset(WheelId),
    VisionLoaded(Result<VisionHandle, String>),
    Embedded(u64, Result<EmbeddingHandle, String>),
    AiMask(AiMaskKind),
    GradientMask(GradientKind),
    MaskReady(Result<Arc<MaskBuffer>, String>),
    LocalParam(LocalField, f32),
    LocalInvert,
    LocalClear,
    ExportFormat(FormatChoice),
    ExportQuality(f32),
    ExportSpace(OutputSpace),
    ExportSize(Option<u32>),
    ExportScope(ExportScope),
    ExportPickLogo,
    ExportLogo(Option<PathBuf>),
    ExportRun,
    ExportRunTo(Option<PathBuf>),
    ExportProgress(Progress),
    ExportDone(usize, usize, String),
    ExternalPick,
    ExternalPicked(Option<PathBuf>),
    ExternalOpen,
    ExternalStatus(String),
    ExternalSaved(PathBuf),
    Grid(crate::grid::GridMessage),
}

// ------------------------------------------------------------------------------------------------
// Helpers shared with lib.rs
// ------------------------------------------------------------------------------------------------

/// Approximate size of the loupe pane from the window size and the visible chrome.
pub fn pane_size(state: &State) -> Size {
    let side = (if state.left_open { super::LEFT_W + 1.0 } else { 0.0 }) + (if state.right_open { super::RIGHT_W + 1.0 } else { 0.0 });
    let tall = 48.0 + 40.0 + 30.0 + 26.0 + if state.filmstrip_open { super::FILMSTRIP_H + 1.0 } else { 0.0 };
    Size::new((state.ext.window.width - side).max(200.0), (state.ext.window.height - tall).max(150.0))
}

pub fn render_context(state: &State) -> RenderContext {
    RenderContext { display: state.ext.display.clone(), mask: state.ext.mask.clone(), dither: true, clip_overlay: state.ext.clip_overlay }
}

fn delayed<T: Send + 'static>(ms: u64, value: T) -> impl std::future::Future<Output = T> {
    async move {
        std::thread::sleep(Duration::from_millis(ms));
        value
    }
}

// ------------------------------------------------------------------------------------------------
// Detail layer (true 1:1)
// ------------------------------------------------------------------------------------------------

async fn load_full(id: u64, path: PathBuf) -> (u64, Result<Arc<ImageBufferF32>, String>) {
    (id, halation_raw::decode_file(&path).map(|a| Arc::new(a.buffer)).map_err(|e| e.to_string()))
}

async fn detail_job(
    doc_id: u64,
    full: Arc<ImageBufferF32>,
    rect: [f32; 4],
    params: DevelopParameters,
    gpu: Option<Arc<GpuDevelopEngine>>,
    ctx: RenderContext,
) -> Arc<DetailResult> {
    let (crop, covered) = crop_region(&full, rect, 4096);
    let out = process_with_context(&crop, &params, gpu.as_deref(), &ctx);
    Arc::new(DetailResult { doc_id, rect: covered, handle: iced::widget::image::Handle::from_rgba(out.width, out.height, out.display_rgba) })
}

/// Called after any change of view, parameters or display: (re)render the native-resolution window
/// when the loupe is magnified past the preview's own resolution, drop it otherwise.
pub fn refresh_detail(state: &mut State) -> Task<Message> {
    let Some(doc) = state.doc.clone() else { return Task::none() };
    let pane = pane_size(state);
    let scale = state.ext.view.effective_scale(doc.full_size, pane);
    let preview_scale = doc.preview.width as f32 / doc.full_size.0.max(1) as f32;
    if scale <= preview_scale * 1.02 {
        state.ext.detail = None;
        return Task::none();
    }
    let Some(full) = state.ext.full.as_ref().filter(|(id, _)| *id == doc.id).map(|(_, f)| f.clone()) else {
        if state.ext.full_loading != Some(doc.id) {
            if let Some(item) = state.selected.and_then(|i| state.items.get(i)) {
                state.ext.full_loading = Some(doc.id);
                state.status = "Loading full resolution…".into();
                return Task::perform(load_full(doc.id, item.path.clone()), |(id, r)| Message::Ext(ExtMessage::FullLoaded(id, r)));
            }
        }
        return Task::none();
    };
    if state.ext.detail_busy {
        state.ext.detail_dirty = true;
        return Task::none();
    }
    state.ext.detail_busy = true;
    state.ext.detail_dirty = false;
    let mut rect = state.ext.view.visible_rect(doc.full_size, pane);
    let margin = 0.04;
    rect = [(rect[0] - rect[2] * margin).max(0.0), (rect[1] - rect[3] * margin).max(0.0), (rect[2] * (1.0 + 2.0 * margin)).min(1.0), (rect[3] * (1.0 + 2.0 * margin)).min(1.0)];
    let job = detail_job(doc.id, full, rect, state.params.clone(), state.gpu_engine.clone(), render_context(state));
    Task::perform(job, |r| Message::Ext(ExtMessage::DetailRendered(r)))
}

pub fn on_view_event(state: &mut State, event: ViewEvent) -> Task<Message> {
    match event {
        ViewEvent::Set(t) => {
            state.ext.view = t;
            state.zoom = t.mode;
            refresh_detail(state)
        }
        ViewEvent::Divider(d) => {
            if matches!(state.ext.compare, Compare::Wipe(_)) {
                state.ext.compare = Compare::Wipe(d);
            }
            Task::none()
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Persistent history + XMP
// ------------------------------------------------------------------------------------------------

/// Load the stored history of `asset_id` (or start a fresh graph from `saved`) and set `state.params`.
pub fn history_open(state: &mut State, asset_id: u64, saved: &DevelopParameters) {
    state.ext.history_db.clear();
    if let Some((nodes, active_db)) = state.catalog.history_load(asset_id) {
        let index: HashMap<i64, usize> = nodes.iter().enumerate().map(|(i, n)| (n.db_id, i)).collect();
        let graph_nodes: Vec<HistoryNode> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| HistoryNode {
                id: i,
                parent: n.parent_db_id.and_then(|p| index.get(&p).copied()),
                label: n.label.clone(),
                detail: n.detail.clone(),
                params: n.params.clone().sanitized(),
            })
            .collect();
        if let Some(graph) = index.get(&active_db).and_then(|&a| HistoryGraph::from_parts(graph_nodes, a)) {
            state.params = graph.active().params.clone();
            state.ext.history_db = nodes.iter().map(|n| n.db_id).collect();
            state.history = graph;
            return;
        }
    }
    state.params = saved.clone();
    state.history = HistoryGraph::new(saved.clone(), if saved.is_default() { "Imported" } else { "Saved settings" });
}

/// Commit `params` to the in-memory DAG and SQLite. Returns whether anything changed.
pub fn history_commit(state: &mut State, params: DevelopParameters, label: Option<String>, detail: String) -> bool {
    let Some(index) = state.selected else { return false };
    let asset_id = state.items[index].id;
    let parent = state.history.active_id();
    let created = match label {
        Some(l) => state.history.commit_labeled(params.clone(), l, detail),
        None => state.history.commit(params.clone()),
    };
    let Some(new_idx) = created else { return false };
    if state.ext.history_db.is_empty() {
        let base = state.history.nodes()[0].clone();
        match state.catalog.history_append(asset_id, None, &base.label, &base.detail, &base.params) {
            Ok(id) => state.ext.history_db.push(id),
            Err(e) => {
                state.status = format!("History not saved: {e}");
                return true;
            }
        }
    }
    let node = state.history.nodes()[new_idx].clone();
    let parent_db = state.ext.history_db.get(parent).copied();
    match state.catalog.history_append(asset_id, parent_db, &node.label, &node.detail, &params) {
        Ok(id) => state.ext.history_db.push(id),
        Err(e) => state.status = format!("History not saved: {e}"),
    }
    state.items[index].params = params;
    state.ext.xmp_pending = true;
    true
}

/// After undo / redo / checkout: move the persisted head and store the parameters.
pub fn history_activate(state: &mut State) {
    let Some(index) = state.selected else { return };
    let params = state.history.active().params.clone();
    if let Some(&db) = state.ext.history_db.get(state.history.active_id()) {
        let _ = state.catalog.history_set_active(state.items[index].id, db, &params);
    } else {
        state.catalog.save_parameters(state.items[index].id, &params);
    }
    state.items[index].params = params;
    state.ext.xmp_pending = true;
}

pub fn schedule_xmp(state: &mut State) -> Task<Message> {
    state.ext.xmp_gen += 1;
    Task::perform(delayed(500, state.ext.xmp_gen), |g| Message::Ext(ExtMessage::XmpDue(g)))
}

/// On open: if another program rewrote the sidecar, adopt its values as a new history step.
pub fn import_sidecar_if_external(state: &mut State) {
    let Some(item) = state.selected.and_then(|i| state.items.get(i)) else { return };
    let Ok(text) = std::fs::read_to_string(xmp::sidecar_path(&item.path)) else { return };
    if !xmp::externally_modified(&text) {
        return;
    }
    if let Some(external) = xmp::parse(&text) {
        if external.params != state.params {
            let params = external.params;
            state.params = params.clone();
            history_commit(state, params, Some("Sidecar edited externally".into()), String::new());
            state.status = "Applied changes found in the XMP sidecar".into();
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Display profile
// ------------------------------------------------------------------------------------------------

async fn probe_display(center: (i32, i32)) -> (Option<Arc<DisplayTransform>>, String) {
    match DisplayProfile::discover(Some(center)) {
        Some(profile) => (Some(Arc::new(DisplayTransform::from_profile(&profile))), profile.description.clone()),
        None => (None, "sRGB (no monitor profile found)".to_owned()),
    }
}

fn schedule_probe(state: &mut State) -> Task<Message> {
    state.ext.display_gen += 1;
    Task::perform(delayed(300, state.ext.display_gen), |g| Message::Ext(ExtMessage::ProbeDisplay(g)))
}

// ------------------------------------------------------------------------------------------------
// AI masks
// ------------------------------------------------------------------------------------------------

fn model_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push = |p: PathBuf| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(|p| p.to_path_buf());
        while let Some(d) = dir {
            push(d.join("assets").join("models"));
            push(d.join("models"));
            dir = d.parent().map(|p| p.to_path_buf());
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        push(cwd.join("assets").join("models"));
        push(cwd.join("models"));
    }
    // In development the executable lives under `target/`, so also try the checkout root.
    if cfg!(debug_assertions) {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        if let Some(root) = manifest.parent().and_then(|p| p.parent()) {
            push(root.join("assets").join("models"));
            push(root.join("models"));
        }
    }
    out
}

pub fn model_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("HALATION_MODELS") {
        return PathBuf::from(p);
    }
    let candidates = model_candidates();
    candidates
        .iter()
        .find(|p| has_models(p))
        .or_else(|| candidates.iter().find(|p| p.is_dir()))
        .cloned()
        .unwrap_or_else(|| PathBuf::from("assets/models"))
}

pub fn load_vision_task(state: &mut State) -> Task<Message> {
    if !matches!(state.ext.vision, VisionState::Idle) {
        return Task::none();
    }
    state.ext.vision = VisionState::Loading;
    let dir = model_dir();
    Task::perform(
        async move { VisionEngine::new(&dir).map(|e| VisionHandle(Arc::new(e))).map_err(|e| e.to_string()) },
        |r| Message::Ext(ExtMessage::VisionLoaded(r)),
    )
}

fn embed_task(state: &mut State) -> Task<Message> {
    let VisionState::Ready(engine) = &state.ext.vision else { return Task::none() };
    let (engine, Some(doc)) = (engine.clone(), state.doc.clone()) else { return Task::none() };
    if state.ext.embedding_busy || state.ext.embedding.as_ref().is_some_and(|(id, _)| *id == doc.id) {
        return Task::none();
    }
    state.ext.embedding_busy = true;
    Task::perform(
        async move {
            let out = process_buffer(&doc.preview, &DevelopParameters::default(), None);
            let rgb: Vec<u8> = out.display_rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            (doc.id, engine.embed(doc.id, &rgb, out.width, out.height).map(EmbeddingHandle).map_err(|e| e.to_string()))
        },
        |(id, r)| Message::Ext(ExtMessage::Embedded(id, r)),
    )
}

fn decode_task(state: &mut State, kind: AiMaskKind, extra: Vec<Prompt>) -> Task<Message> {
    let VisionState::Ready(engine) = &state.ext.vision else { return Task::none() };
    let Some((_, emb)) = state.ext.embedding.clone() else { return Task::none() };
    let engine = engine.clone();
    state.ext.mask_busy = true;
    Task::perform(
        async move { engine.select(&emb, kind, &extra).map(Arc::new).map_err(|e| e.to_string()) },
        |r| Message::Ext(ExtMessage::MaskReady(r)),
    )
}

fn set_local_source(state: &mut State, source: MaskSource) {
    let local = state.params.local.get_or_insert_with(LocalAdjustment::default);
    local.source = Some(source);
    local.enabled = true;
}

/// Regenerate the mask raster of the stored local adjustment (after opening a photo).
pub fn restore_mask(state: &mut State) -> Task<Message> {
    let Some(source) = state.params.local.as_ref().and_then(|l| l.source.clone()) else { return Task::none() };
    let aspect = state.doc.as_ref().map_or(1.5, |d| d.full_size.0 as f32 / d.full_size.1.max(1) as f32);
    match &source {
        MaskSource::Ai { .. } => load_vision_task(state),
        other => {
            state.ext.mask = rasterize(other, aspect, 512).map(Arc::new);
            Task::none()
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Export / external editor
// ------------------------------------------------------------------------------------------------

async fn pick_dir() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new().pick_folder().await.map(|h| h.path().to_path_buf())
}

async fn pick_file(filter: &'static [&'static str]) -> Option<PathBuf> {
    rfd::AsyncFileDialog::new().add_filter("File", filter).pick_file().await.map(|h| h.path().to_path_buf())
}

fn export_stream(jobs: Vec<ExportJob>, settings: ExportSettings) -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(64, move |output| async move {
        let tx = output.clone();
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let results = export::export_batch(&jobs, &settings, &move |p| {
                let _ = progress_tx.clone().try_send(Message::Ext(ExtMessage::ExportProgress(p)));
            });
            let failed: Vec<&(PathBuf, Result<(), String>)> = results.iter().filter(|(_, r)| r.is_err()).collect();
            let first = failed.first().and_then(|(p, r)| r.as_ref().err().map(|e| format!("{}: {e}", p.display()))).unwrap_or_default();
            let _ = tx.clone().try_send(Message::Ext(ExtMessage::ExportDone(results.len() - failed.len(), failed.len(), first)));
        });
    })
}

fn external_stream(editor: ExternalEditor, source: PathBuf, handoff: PathBuf, params: DevelopParameters, mask: Option<Arc<MaskBuffer>>, stop: Arc<AtomicBool>) -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(16, move |output| async move {
        let tx = output.clone();
        std::thread::spawn(move || {
            let say = |m: String| {
                let _ = tx.clone().try_send(Message::Ext(ExtMessage::ExternalStatus(m)));
            };
            if let Some(parent) = handoff.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = external::write_handoff_tiff(&source, &handoff, &params, mask) {
                return say(format!("External editor: could not render hand-off: {e}"));
            }
            if let Err(e) = external::launch(&editor, &handoff) {
                return say(format!("External editor: could not start {}: {e}", editor.name));
            }
            say(format!("Opened in {} — save the file to bring the result back", editor.name));
            let saved_tx = tx.clone();
            let handle = external::watch_for_saves(handoff.clone(), Duration::from_millis(700), stop, move |p| {
                let _ = saved_tx.clone().try_send(Message::Ext(ExtMessage::ExternalSaved(p)));
            });
            let _ = handle.join();
        });
    })
}

fn unique_dest(taken: &mut HashSet<PathBuf>, dest: PathBuf) -> PathBuf {
    if taken.insert(dest.clone()) {
        return dest;
    }
    let (stem, ext) = (dest.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), dest.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    (2..)
        .map(|n| dest.with_file_name(format!("{stem}-{n}.{ext}")))
        .find(|p| taken.insert(p.clone()))
        .unwrap_or(dest)
}

// ------------------------------------------------------------------------------------------------
// Update
// ------------------------------------------------------------------------------------------------

pub fn update(state: &mut State, message: ExtMessage) -> Task<Message> {
    match message {
        ExtMessage::FullLoaded(id, result) => {
            state.ext.full_loading = None;
            match result {
                Ok(full) if state.doc.as_ref().map(|d| d.id) == Some(id) => {
                    state.ext.full = Some((id, full));
                    state.status = "Full resolution ready".into();
                    refresh_detail(state)
                }
                Err(e) => {
                    state.status = format!("Could not load full resolution: {e}");
                    Task::none()
                }
                _ => Task::none(),
            }
        }
        ExtMessage::DetailRendered(r) => {
            state.ext.detail_busy = false;
            if state.doc.as_ref().map(|d| d.id) == Some(r.doc_id) {
                state.ext.detail = Some((r.doc_id, Detail { rect: r.rect, handle: r.handle.clone() }));
            }
            if state.ext.detail_dirty { refresh_detail(state) } else { Task::none() }
        }
        ExtMessage::WindowOpened(pos, size) => {
            state.ext.window = size;
            if let Some(p) = pos {
                state.ext.window_pos = p;
            }
            schedule_probe(state)
        }
        ExtMessage::WindowMoved(p) => {
            state.ext.window_pos = p;
            schedule_probe(state)
        }
        ExtMessage::WindowResized(s) => {
            state.ext.window = s;
            Task::none()
        }
        ExtMessage::ProbeDisplay(g) => {
            if g != state.ext.display_gen {
                return Task::none();
            }
            let center = (state.ext.window_pos.x as i32 + state.ext.window.width as i32 / 2, state.ext.window_pos.y as i32 + state.ext.window.height as i32 / 2);
            Task::perform(probe_display(center), |(t, name)| Message::Ext(ExtMessage::DisplayReady(t, name)))
        }
        ExtMessage::DisplayReady(transform, name) => {
            if name == state.ext.display_name {
                return Task::none();
            }
            state.ext.display = transform;
            state.ext.display_name = name.clone();
            state.original_for = None;
            state.status = format!("Display profile: {name}");
            Task::batch([request_render(state), refresh_detail(state)])
        }
        ExtMessage::XmpDue(g) => {
            if g != state.ext.xmp_gen {
                return Task::none();
            }
            let Some(item) = state.selected.and_then(|i| state.items.get(i)) else { return Task::none() };
            let (path, snapshot) = (item.path.clone(), XmpState { params: state.params.clone(), rating: item.rating, flag: item.flag.clone() });
            Task::perform(async move { xmp::write_sidecar(&path, &snapshot).map_err(|e| e.to_string()) }, |r| Message::Ext(ExtMessage::XmpWritten(r)))
        }
        ExtMessage::XmpWritten(r) => {
            if let Err(e) = r {
                state.status = format!("XMP sidecar not written: {e}");
            }
            Task::none()
        }
        ExtMessage::SetTool(tool) => {
            state.ext.tool = tool;
            if tool == Tool::Mask {
                return load_vision_task(state);
            }
            Task::none()
        }
        ExtMessage::CurveChannel(ch) => {
            state.ext.curve_channel = ch;
            Task::none()
        }
        ExtMessage::CurveReset => {
            let ch = state.ext.curve_channel;
            state.params.curve.reset_channel(ch);
            let params = state.params.clone();
            history_commit(state, params, Some(format!("Reset curve {}", ch.label())), String::new());
            request_render(state)
        }
        ExtMessage::WheelLum(id, v) => {
            wheel_mut(&mut state.params, id).lum = v.clamp(-1.0, 1.0);
            request_render(state)
        }
        ExtMessage::WheelReset(id) => {
            *wheel_mut(&mut state.params, id) = Wheel::default();
            let params = state.params.clone();
            history_commit(state, params, Some(format!("Reset {} wheel", id.label())), String::new());
            request_render(state)
        }
        ExtMessage::VisionLoaded(result) => {
            match result {
                Ok(h) => {
                    state.ext.vision = VisionState::Ready(h.0);
                    return embed_task(state);
                }
                Err(e) => state.ext.vision = VisionState::Unavailable(e),
            }
            Task::none()
        }
        ExtMessage::Grid(m) => crate::grid::update(state, m),
        ExtMessage::Embedded(id, result) => {
            state.ext.embedding_busy = false;
            match result {
                Ok(h) if state.doc.as_ref().map(|d| d.id) == Some(id) => {
                    state.ext.embedding = Some((id, h.0));
                    let kind = state.ext.pending_ai.take().or_else(|| match state.params.local.as_ref().and_then(|l| l.source.as_ref()) {
                        Some(MaskSource::Ai { kind, .. }) if state.ext.mask.is_none() => Some(*kind),
                        _ => None,
                    });
                    match kind {
                        Some(kind) => decode_task(state, kind, Vec::new()),
                        None => Task::none(),
                    }
                }
                Err(e) => {
                    state.status = format!("AI preparation failed: {e}");
                    Task::none()
                }
                _ => Task::none(),
            }
        }
        ExtMessage::AiMask(kind) => {
            match &state.ext.vision {
                VisionState::Unavailable(e) => {
                    state.status = format!("AI masks unavailable: {e}");
                    return Task::none();
                }
                VisionState::Idle => return load_vision_task(state),
                _ => {}
            }
            set_local_source(state, MaskSource::Ai { kind, points: Vec::new() });
            if state.ext.embedding.as_ref().is_some_and(|(id, _)| Some(*id) == state.doc.as_ref().map(|d| d.id)) {
                decode_task(state, kind, Vec::new())
            } else {
                state.ext.pending_ai = Some(kind);
                state.status = "Analysing the photo…".into();
                embed_task(state)
            }
        }
        ExtMessage::GradientMask(kind) => {
            let source = match kind {
                GradientKind::Linear => MaskSource::Linear { start: [0.5, 0.0], end: [0.5, 0.6] },
                GradientKind::Radial => MaskSource::Radial { center: [0.5, 0.5], radius: [0.3, 0.3], feather: 0.4 },
            };
            let aspect = state.doc.as_ref().map_or(1.5, |d| d.full_size.0 as f32 / d.full_size.1.max(1) as f32);
            state.ext.mask = rasterize(&source, aspect, 512).map(Arc::new);
            set_local_source(state, source);
            let params = state.params.clone();
            history_commit(state, params, Some(if kind == GradientKind::Linear { "Linear gradient mask" } else { "Radial gradient mask" }.into()), String::new());
            request_render(state)
        }
        ExtMessage::MaskReady(result) => {
            state.ext.mask_busy = false;
            match result {
                Ok(mask) => {
                    state.ext.mask = Some(mask);
                    let params = state.params.clone();
                    let label = match params.local.as_ref().and_then(|l| l.source.as_ref()) {
                        Some(MaskSource::Ai { kind: AiMaskKind::Sky, .. }) => "Select Sky",
                        _ => "Select Subject",
                    };
                    history_commit(state, params, Some(label.into()), String::new());
                    state.status = format!("{label} ready");
                    Task::batch([request_render(state), refresh_detail(state)])
                }
                Err(e) => {
                    state.status = format!("Mask failed: {e}");
                    Task::none()
                }
            }
        }
        ExtMessage::LocalParam(field, v) => {
            let local = state.params.local.get_or_insert_with(LocalAdjustment::default);
            let (lo, hi) = field.range();
            field.set(local, v.clamp(lo, hi));
            request_render(state)
        }
        ExtMessage::LocalInvert => {
            if let Some(l) = state.params.local.as_mut() {
                l.invert = !l.invert;
            }
            let params = state.params.clone();
            history_commit(state, params, Some("Invert mask".into()), String::new());
            request_render(state)
        }
        ExtMessage::LocalClear => {
            state.params.local = None;
            state.ext.mask = None;
            let params = state.params.clone();
            history_commit(state, params, Some("Remove local adjustment".into()), String::new());
            request_render(state)
        }
        ExtMessage::ExportFormat(f) => {
            state.ext.export.format = f;
            Task::none()
        }
        ExtMessage::ExportQuality(q) => {
            state.ext.export.quality = q.round().clamp(1.0, 100.0) as u8;
            Task::none()
        }
        ExtMessage::ExportSpace(s) => {
            state.ext.export.space = s;
            Task::none()
        }
        ExtMessage::ExportSize(s) => {
            state.ext.export.max_edge = s;
            Task::none()
        }
        ExtMessage::ExportScope(s) => {
            state.ext.export.scope = s;
            Task::none()
        }
        ExtMessage::ExportPickLogo => Task::perform(pick_file(&["png", "webp", "jpg", "jpeg"]), |p| Message::Ext(ExtMessage::ExportLogo(p))),
        ExtMessage::ExportLogo(p) => {
            state.ext.export.logo = p;
            Task::none()
        }
        ExtMessage::ExportRun => {
            if state.ext.export.running {
                return Task::none();
            }
            Task::perform(pick_dir(), |d| Message::Ext(ExtMessage::ExportRunTo(d)))
        }
        ExtMessage::ExportRunTo(None) => Task::none(),
        ExtMessage::ExportRunTo(Some(dir)) => {
            let ui = &state.ext.export;
            let indices: Vec<usize> = match ui.scope {
                ExportScope::Current => state.selected.into_iter().collect(),
                ExportScope::Visible => visible_indices(state),
            };
            if indices.is_empty() {
                state.status = "Nothing to export".into();
                return Task::none();
            }
            let format = ui.format.to_format(ui.quality);
            let mut taken = HashSet::new();
            let jobs: Vec<ExportJob> = indices
                .iter()
                .map(|&i| {
                    let item = &state.items[i];
                    let current = state.selected == Some(i);
                    ExportJob {
                        src: item.path.clone(),
                        dest: unique_dest(&mut taken, export::default_dest(&item.path, &dir, format)),
                        params: if current { state.params.clone() } else { item.params.clone() },
                        mask: if current { state.ext.mask.clone() } else { None },
                    }
                })
                .collect();
            let settings = ExportSettings {
                format,
                space: ui.space,
                max_long_edge: ui.max_edge,
                embed_profile: true,
                watermark: ui.logo.clone().map(|p| Watermark {
                    kind: WatermarkKind::Logo(p),
                    anchor: export::Anchor::BottomRight,
                    margin: 0.03,
                    scale: 0.15,
                    opacity: 0.6,
                    offset: [0.0, 0.0],
                }),
            };
            state.ext.export.running = true;
            state.ext.export.progress = None;
            state.ext.export.message = format!("Exporting {} photo(s)…", jobs.len());
            Task::run(export_stream(jobs, settings), |m| m)
        }
        ExtMessage::ExportProgress(p) => {
            state.ext.export.progress = Some(p);
            Task::none()
        }
        ExtMessage::ExportDone(ok, failed, first) => {
            state.ext.export.running = false;
            state.ext.export.message = if failed == 0 { format!("Exported {ok} photo(s)") } else { format!("Exported {ok}, {failed} failed — {first}") };
            state.status = state.ext.export.message.clone();
            Task::none()
        }
        ExtMessage::ExternalPick => Task::perform(pick_file(&["exe", "app", ""]), |p| Message::Ext(ExtMessage::ExternalPicked(p))),
        ExtMessage::ExternalPicked(path) => {
            if let Some(executable) = path {
                let name = executable.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "editor".into());
                state.ext.external.editor = Some(ExternalEditor { name, executable, extra_args: Vec::new() });
            }
            Task::none()
        }
        ExtMessage::ExternalOpen => {
            let (Some(editor), Some(item)) = (state.ext.external.editor.clone(), state.selected.and_then(|i| state.items.get(i))) else {
                state.status = "Choose an external editor first (Export tab)".into();
                return Task::none();
            };
            state.ext.external.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            let stop = Arc::new(AtomicBool::new(false));
            state.ext.external.stop = stop.clone();
            state.ext.external.watching = true;
            let handoff = external::handoff_path(&std::env::temp_dir(), &item.path, item.id);
            Task::run(external_stream(editor, item.path.clone(), handoff, state.params.clone(), state.ext.mask.clone(), stop), |m| m)
        }
        ExtMessage::ExternalStatus(s) => {
            state.status = s;
            Task::none()
        }
        ExtMessage::ExternalSaved(path) => {
            let Some(item) = state.selected.and_then(|i| state.items.get(i)) else { return Task::none() };
            let dest = item.path.with_file_name(format!("{}-edit.tif", item.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()));
            match std::fs::copy(&path, &dest) {
                Ok(_) => Task::done(Message::Picked(vec![dest])),
                Err(e) => {
                    state.status = format!("Could not bring the edited file back: {e}");
                    Task::none()
                }
            }
        }
    }
}

fn wheel_mut(params: &mut DevelopParameters, id: WheelId) -> &mut Wheel {
    match id {
        WheelId::Shadows => &mut params.grading.shadows,
        WheelId::Midtones => &mut params.grading.midtones,
        WheelId::Highlights => &mut params.grading.highlights,
        WheelId::Global => &mut params.grading.global,
    }
}

/// Direct wheel edit from the canvas widget.
pub fn set_wheel(state: &mut State, id: WheelId, wheel: Wheel) -> Task<Message> {
    *wheel_mut(&mut state.params, id) = wheel;
    request_render(state)
}

/// Direct curve edit from the canvas widget.
pub fn edit_curve(state: &mut State, edit: CurveEdit) -> Task<Message> {
    match edit {
        CurveEdit::Insert { channel, x, y } => {
            state.params.curve.insert_point(channel, x, y);
        }
        CurveEdit::Move { channel, index, x, y } => state.params.curve.move_point(channel, index, x, y),
        CurveEdit::Remove { channel, index } => {
            if state.params.curve.remove_point(channel, index) {
                let params = state.params.clone();
                history_commit(state, params, Some(format!("Remove curve point ({})", channel.label())), String::new());
            }
        }
    }
    request_render(state)
}
