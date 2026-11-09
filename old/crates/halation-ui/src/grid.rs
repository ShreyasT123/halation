//! Library at scale (Sprint 6): a virtualized grid that renders only the visible rows (+2 buffer),
//! a bounded cache of GPU image handles in front of the two-tier thumbnail cache, background
//! thumbnail loading for exactly what is on screen (grid or filmstrip), colour-label keys, and the
//! assisted-culling bar (analysis runs once; threshold sliders re-filter instantly).

use super::{
    flag_chip, label, mono, solid, thumb_picture, tile_style, visible_indices, Message, Page,
    State, ACCENT, BG_PANEL, PICK, REJECT, STAR, TEXT_DIM, TEXT_SECONDARY,
};
use crate::ext::ExtMessage;
use halation_catalog::CatalogItem;
use halation_pipeline::{
    culling::{analyze_files, CullMetrics, CullThresholds, CullVerdict},
    thumbs::{Thumb, ThumbJob, ThumbnailCache},
};
use iced::{
    alignment::Vertical,
    widget::{button, column, container, image, row, scrollable, slider, Column, Row, Space},
    Color, Element, Length, Task,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::Arc,
};

pub const CELL: f32 = 160.0;
pub const ROW_H: f32 = 150.0;
const GAP: f32 = 10.0;
const BUFFER_ROWS: usize = 2;
const MAX_BATCH: usize = 48;

pub fn app_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .unwrap_or_else(std::env::temp_dir)
        .join("Halation")
}

// ------------------------------------------------------------------------------------------------
// Bounded handle cache (drop-in for the old `HashMap<u64, Handle>`)
// ------------------------------------------------------------------------------------------------

pub struct HandleCache {
    map: HashMap<u64, image::Handle>,
    order: VecDeque<u64>,
    cap: usize,
}

impl HandleCache {
    pub fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            cap: cap.max(16),
        }
    }
    pub fn get(&self, id: &u64) -> Option<&image::Handle> {
        self.map.get(id)
    }
    pub fn contains_key(&self, id: &u64) -> bool {
        self.map.contains_key(id)
    }
    pub fn insert(&mut self, id: u64, handle: image::Handle) {
        if self.map.insert(id, handle).is_none() {
            self.order.push_back(id);
            while self.order.len() > self.cap {
                if let Some(old) = self.order.pop_front() {
                    self.map.remove(&old);
                }
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------
// State & messages
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CullFilter {
    All,
    Select,
    Neutral,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CullField {
    MinFocus,
    MaxMotion,
    MaxClipped,
}

pub struct GridState {
    pub scroll_y: f32,
    pub viewport_h: f32,
    cache: Arc<ThumbnailCache>,
    pending: HashSet<u64>,
    pub metrics: HashMap<u64, CullMetrics>,
    pub thresholds: CullThresholds,
    pub cull_running: bool,
    pub filter: CullFilter,
}

impl GridState {
    pub fn new() -> Self {
        Self {
            scroll_y: 0.0,
            viewport_h: 700.0,
            cache: Arc::new(ThumbnailCache::new(app_dir().join("cache"), 192, 500)),
            pending: HashSet::new(),
            metrics: HashMap::new(),
            thresholds: CullThresholds::default(),
            cull_running: false,
            filter: CullFilter::All,
        }
    }
}

#[derive(Debug, Clone)]
pub enum GridMessage {
    Scrolled(f32, f32),
    ThumbsReady(Vec<(u64, Option<Arc<Thumb>>)>),
    Row(i32),
    Label(&'static str),
    CullRun,
    CullDone(Vec<(u64, Option<CullMetrics>)>),
    Threshold(CullField, f32),
    Filter(CullFilter),
}

fn grid_id() -> scrollable::Id {
    scrollable::Id::new("halation-grid")
}

pub fn columns(state: &State) -> usize {
    (((state.ext.window.width - 48.0) / (CELL + GAP)).floor() as usize).max(1)
}

fn key_for(item: &CatalogItem) -> [u8; 32] {
    if item.hash != [0u8; 32] {
        return item.hash;
    }
    let mut k = [0u8; 32];
    k[..8].copy_from_slice(&item.id.to_le_bytes());
    k
}

// ------------------------------------------------------------------------------------------------
// Thumbnails for exactly what is on screen
// ------------------------------------------------------------------------------------------------

/// Item indices currently needing thumbnails: visible grid rows (+buffer) or the filmstrip window.
fn wanted_indices(state: &State) -> Vec<usize> {
    let visible = visible_indices(state);
    if state.page == Page::Library {
        let cols = columns(state);
        let first = (state.ext.grid.scroll_y / ROW_H).floor().max(0.0) as usize;
        let last =
            ((state.ext.grid.scroll_y + state.ext.grid.viewport_h) / ROW_H).ceil() as usize + 1;
        let (a, b) = (
            first.saturating_sub(BUFFER_ROWS) * cols,
            ((last + BUFFER_ROWS) * cols).min(visible.len()),
        );
        visible
            .get(a..b.max(a))
            .map(|s| s.to_vec())
            .unwrap_or_default()
    } else {
        let pos = state
            .selected
            .and_then(|s| visible.iter().position(|&i| i == s))
            .unwrap_or(0);
        let (a, b) = (pos.saturating_sub(40), (pos + 41).min(visible.len()));
        visible
            .get(a..b.max(a))
            .map(|s| s.to_vec())
            .unwrap_or_default()
    }
}

/// Queue a batch of missing thumbnails (one Rayon-parallel batch at a time).
pub fn request_visible(state: &mut State) -> Task<Message> {
    let jobs: Vec<ThumbJob> = wanted_indices(state)
        .into_iter()
        .map(|i| &state.items[i])
        .filter(|it| {
            !state.thumbs.contains_key(&it.id)
                && !state.thumb_failed.contains(&it.id)
                && !state.ext.grid.pending.contains(&it.id)
        })
        .take(MAX_BATCH)
        .map(|it| ThumbJob {
            id: it.id,
            hash: key_for(it),
            path: it.path.clone(),
        })
        .collect();
    if jobs.is_empty() {
        return Task::none();
    }
    state.ext.grid.pending.extend(jobs.iter().map(|j| j.id));
    let cache = state.ext.grid.cache.clone();
    Task::perform(async move { cache.load_many(&jobs) }, |r| {
        Message::Ext(ExtMessage::Grid(GridMessage::ThumbsReady(r)))
    })
}

fn scroll_to_row(state: &State, row_index: usize) -> Task<Message> {
    let (top, bottom) = (row_index as f32 * ROW_H, (row_index + 1) as f32 * ROW_H);
    let g = &state.ext.grid;
    let target = if top < g.scroll_y {
        top
    } else if bottom > g.scroll_y + g.viewport_h {
        bottom - g.viewport_h
    } else {
        return Task::none();
    };
    scrollable::scroll_to(
        grid_id(),
        scrollable::AbsoluteOffset {
            x: 0.0,
            y: target.max(0.0),
        },
    )
}

pub fn update(state: &mut State, message: GridMessage) -> Task<Message> {
    match message {
        GridMessage::Scrolled(y, h) => {
            state.ext.grid.scroll_y = y;
            state.ext.grid.viewport_h = h;
            request_visible(state)
        }
        GridMessage::ThumbsReady(list) => {
            for (id, thumb) in list {
                state.ext.grid.pending.remove(&id);
                match thumb {
                    Some(t) => state.thumbs.insert(
                        id,
                        image::Handle::from_rgba(t.width, t.height, t.rgba.clone()),
                    ),
                    None => {
                        state.thumb_failed.insert(id);
                    }
                }
            }
            request_visible(state)
        }
        GridMessage::Row(delta) => {
            let visible = visible_indices(state);
            if visible.is_empty() {
                return Task::none();
            }
            let cols = columns(state) as i32;
            let pos = state
                .selected
                .and_then(|s| visible.iter().position(|&i| i == s))
                .map(|p| p as i32)
                .unwrap_or(-cols * delta.signum());
            let next = (pos + delta * cols).clamp(0, visible.len() as i32 - 1) as usize;
            let scroll = if state.page == Page::Library {
                scroll_to_row(state, next / cols as usize)
            } else {
                Task::none()
            };
            Task::batch([Task::done(Message::Select(visible[next])), scroll])
        }
        GridMessage::Label(l) => {
            if let Some(i) = state.selected {
                state.catalog.update_color_label(state.items[i].id, l);
                state.items[i].color_label = l.into();
            }
            Task::none()
        }
        GridMessage::CullRun => {
            if state.ext.grid.cull_running {
                return Task::none();
            }
            let targets: Vec<(u64, PathBuf)> = visible_indices(state)
                .into_iter()
                .map(|i| (state.items[i].id, state.items[i].path.clone()))
                .collect();
            if targets.is_empty() {
                return Task::none();
            }
            state.ext.grid.cull_running = true;
            state.status = format!("Analysing {} photos locally…", targets.len());
            Task::perform(
                async move {
                    let paths: Vec<PathBuf> = targets.iter().map(|t| t.1.clone()).collect();
                    let results = analyze_files(&paths, None);
                    targets
                        .into_iter()
                        .zip(results)
                        .map(|((id, _), (_, r))| (id, r.ok()))
                        .collect::<Vec<_>>()
                },
                |r| Message::Ext(ExtMessage::Grid(GridMessage::CullDone(r))),
            )
        }
        GridMessage::CullDone(list) => {
            let n = list.iter().filter(|(_, m)| m.is_some()).count();
            for (id, m) in list {
                if let Some(m) = m {
                    state.ext.grid.metrics.insert(id, m);
                }
            }
            state.ext.grid.cull_running = false;
            state.status = format!("Culling analysed {n} photos");
            Task::none()
        }
        GridMessage::Threshold(field, v) => {
            let t = &mut state.ext.grid.thresholds;
            match field {
                CullField::MinFocus => t.min_focus = v,
                CullField::MaxMotion => t.max_motion = v,
                CullField::MaxClipped => t.max_clipped = v,
            }
            Task::none()
        }
        GridMessage::Filter(f) => {
            state.ext.grid.filter = f;
            state.ext.grid.scroll_y = 0.0;
            request_visible(state)
        }
    }
}

/// Culling filter used by `visible_indices`.
pub fn passes_cull(state: &State, item: &CatalogItem) -> bool {
    let g = &state.ext.grid;
    if g.filter == CullFilter::All {
        return true;
    }
    match g.metrics.get(&item.id).map(|m| m.verdict(&g.thresholds)) {
        Some(CullVerdict::Select) => g.filter == CullFilter::Select,
        Some(CullVerdict::Neutral) => g.filter == CullFilter::Neutral,
        Some(CullVerdict::Reject(_)) => g.filter == CullFilter::Reject,
        None => false,
    }
}

// ------------------------------------------------------------------------------------------------
// View
// ------------------------------------------------------------------------------------------------

fn file_chip(item: &CatalogItem) -> String {
    let ext = item
        .path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_uppercase())
        .unwrap_or_default();
    match ext.as_str() {
        "ARW" | "CR2" | "CR3" | "NEF" | "ORF" | "RW2" => "RAW".into(),
        "JPEG" => "JPG".into(),
        other => other.to_owned(),
    }
}

fn label_color(name: &str) -> Option<Color> {
    Some(match name {
        "Red" => Color::from_rgb8(0xE5, 0x48, 0x4D),
        "Yellow" => Color::from_rgb8(0xF2, 0xC1, 0x4E),
        "Green" => Color::from_rgb8(0x46, 0xA7, 0x58),
        "Blue" => Color::from_rgb8(0x26, 0x80, 0xEB),
        "Purple" => Color::from_rgb8(0xBF, 0x5A, 0xF2),
        _ => return None,
    })
}

fn cell(state: &State, idx: usize) -> Element<'static, Message> {
    let item = &state.items[idx];
    let mut caption = Row::new()
        .spacing(4)
        .align_y(Vertical::Center)
        .width(Length::Fixed(CELL));
    caption = caption
        .push(label("★".repeat(item.rating as usize), 10, STAR))
        .push(Space::with_width(Length::Fill));
    if let Some(c) = label_color(&item.color_label) {
        caption = caption
            .push(container(Space::new(Length::Fixed(8.0), Length::Fixed(8.0))).style(solid(c)));
    }
    if let Some(m) = state.ext.grid.metrics.get(&item.id) {
        match m.verdict(&state.ext.grid.thresholds) {
            CullVerdict::Select => caption = caption.push(label("●", 10, PICK)),
            CullVerdict::Reject(_) => caption = caption.push(label("●", 10, REJECT)),
            CullVerdict::Neutral => {}
        }
    }
    if let Some(c) = flag_chip(&item.flag) {
        caption = caption.push(c);
    }
    caption = caption.push(mono(file_chip(item), 9, TEXT_DIM));
    button(column![thumb_picture(state, item, CELL, CELL * 0.66), caption].spacing(3))
        .padding(4)
        .style(tile_style(state.selected == Some(idx)))
        .on_press(Message::Select(idx))
        .into()
}

fn cull_bar(state: &State) -> Element<'static, Message> {
    let g = &state.ext.grid;
    let msg = |m: GridMessage| Message::Ext(ExtMessage::Grid(m));
    let filter_btn = |name: &'static str, f: CullFilter| -> Element<'static, Message> {
        button(label(name, 11, TEXT_SECONDARY))
            .padding([3, 9])
            .style(super::button_style(g.filter == f, ACCENT))
            .on_press(msg(GridMessage::Filter(f)))
            .into()
    };
    let slim = |name: &'static str, value: f32, field: CullField| -> Element<'static, Message> {
        row![
            label(name, 10, TEXT_DIM),
            slider(0.0..=1.0, value, move |v| msg(GridMessage::Threshold(
                field, v
            )))
            .step(0.01_f32)
            .width(Length::Fixed(90.0)),
        ]
        .spacing(6)
        .align_y(Vertical::Center)
        .into()
    };
    let (mut selects, mut rejects) = (0, 0);
    for m in g.metrics.values() {
        match m.verdict(&g.thresholds) {
            CullVerdict::Select => selects += 1,
            CullVerdict::Reject(_) => rejects += 1,
            CullVerdict::Neutral => {}
        }
    }
    container(
        row![
            button(label(
                if g.cull_running {
                    "Analysing…"
                } else {
                    "Analyse focus & exposure"
                },
                11,
                TEXT_SECONDARY
            ))
            .padding([3, 10])
            .style(super::button_style(false, ACCENT))
            .on_press_maybe((!g.cull_running).then(|| msg(GridMessage::CullRun))),
            filter_btn("All", CullFilter::All),
            filter_btn("Select", CullFilter::Select),
            filter_btn("Neutral", CullFilter::Neutral),
            filter_btn("Reject", CullFilter::Reject),
            Space::with_width(Length::Fixed(12.0)),
            slim("Focus", g.thresholds.min_focus, CullField::MinFocus),
            slim("Motion", g.thresholds.max_motion, CullField::MaxMotion),
            slim("Clipping", g.thresholds.max_clipped, CullField::MaxClipped),
            Space::with_width(Length::Fill),
            mono(format!("{selects} select · {rejects} reject"), 10, TEXT_DIM),
        ]
        .spacing(8)
        .align_y(Vertical::Center),
    )
    .padding([8, 16])
    .width(Length::Fill)
    .style(solid(BG_PANEL))
    .into()
}

pub fn grid_page(state: &State) -> Element<'static, Message> {
    let visible = visible_indices(state);
    let cols = columns(state);
    let rows = visible.len().div_ceil(cols);
    let g = &state.ext.grid;
    let first = ((g.scroll_y / ROW_H).floor().max(0.0) as usize).saturating_sub(BUFFER_ROWS);
    let last = (((g.scroll_y + g.viewport_h) / ROW_H).ceil() as usize + BUFFER_ROWS).min(rows);

    let mut content = Column::new().push(Space::with_height(Length::Fixed(first as f32 * ROW_H)));
    for r in first..last {
        let mut line = Row::new().spacing(GAP);
        for k in 0..cols {
            if let Some(&idx) = visible.get(r * cols + k) {
                line = line.push(cell(state, idx));
            }
        }
        content = content.push(container(line).height(Length::Fixed(ROW_H)));
    }
    content = content.push(Space::with_height(Length::Fixed(
        rows.saturating_sub(last) as f32 * ROW_H,
    )));
    let scroller = scrollable(container(content).padding([8, 24]))
        .id(grid_id())
        .on_scroll(|vp| {
            Message::Ext(ExtMessage::Grid(GridMessage::Scrolled(
                vp.absolute_offset().y,
                vp.bounds().height,
            )))
        })
        .height(Length::Fill);
    column![cull_bar(state), scroller].into()
}
