//! Purpose-built darkroom controls (Sprint 4), built as canvas programs so they need no `advanced` API:
//!
//! * [`Scrubby`] — label · track · editable mono readout. Drag the track or the number; `Shift` makes
//!   drags 10× finer; double-click label / thumb / number resets to the calibrated default. Temperature
//!   and Tint render live gradient tracks.
//! * [`HistogramView`] — logarithmic RGB + luminance histogram with five interactive tonal zones
//!   (Blacks · Shadows · Exposure · Highlights · Whites): hover highlights a zone, horizontal dragging
//!   drives the matching slider; the corner triangles toggle the clipping overlay.
//! * [`CurveEditor`] — Fritsch–Carlson tone curve per channel: click to add a point, drag to move,
//!   double-click or right-click to delete.
//! * [`WheelView`] — polar hue/saturation wheel with an auto-snap anchor and double-click reset.
//!
//! Needs iced feature `canvas`.

use crate::Message;
use halation_core::{curve::MAX_CURVE_POINTS, CurveChannel, HistogramData, ParamId, ToneCurve, Wheel};
use iced::{
    alignment::{Horizontal, Vertical},
    event, keyboard, mouse,
    widget::{canvas, canvas::{path::Arc, Fill, Frame, Geometry, Gradient, Path, Stroke, Style, Text}},
    Color, Element, Font, Length, Point, Radians, Rectangle, Renderer, Size, Theme,
};
use std::time::{Duration, Instant};

const DOUBLE_CLICK: Duration = Duration::from_millis(350);

const fn hex(v: u32) -> Color {
    Color { r: ((v >> 16) & 0xFF) as f32 / 255.0, g: ((v >> 8) & 0xFF) as f32 / 255.0, b: (v & 0xFF) as f32 / 255.0, a: 1.0 }
}
const RECESS: Color = hex(0x282828);
const RAIL: Color = hex(0x4A4A4A);
const ACCENT: Color = hex(0x2680EB);
const TEXT_DIM: Color = hex(0xA0A0A0);
const TEXT: Color = hex(0xE0E0E0);

fn gradient_fill(x0: f32, x1: f32, a: Color, b: Color) -> Fill {
    let linear = canvas::gradient::Linear::new(Point::new(x0, 0.0), Point::new(x1, 0.0)).add_stop(0.0, a).add_stop(1.0, b);
    Fill { style: Style::Gradient(Gradient::Linear(linear)), ..Fill::default() }
}

fn text(frame: &mut Frame, s: String, at: Point, size: f32, color: Color, h: Horizontal, mono: bool) {
    frame.fill_text(Text {
        content: s,
        position: at,
        color,
        size: size.into(),
        font: if mono { Font::MONOSPACE } else { Font::DEFAULT },
        horizontal_alignment: h,
        vertical_alignment: Vertical::Center,
        ..Text::default()
    });
}

// ------------------------------------------------------------------------------------------------
// Scrubby slider
// ------------------------------------------------------------------------------------------------

pub struct Scrubby {
    pub id: ParamId,
    pub value: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Zone {
    Label,
    Track,
    Value,
}

enum Drag {
    Track,
    Value { acc: f32, last_x: f32 },
}

#[derive(Default)]
pub struct ScrubState {
    drag: Option<Drag>,
    last_click: Option<(Instant, Zone)>,
    shift: bool,
}

const LABEL_W: f32 = 74.0;
const VALUE_W: f32 = 54.0;
const GAP: f32 = 8.0;

impl Scrubby {
    pub fn view(id: ParamId, value: f32) -> Element<'static, Message> {
        canvas(Self { id, value }).width(Length::Fill).height(Length::Fixed(22.0)).into()
    }

    fn track(width: f32) -> (f32, f32) {
        (LABEL_W + GAP, (width - VALUE_W - GAP).max(LABEL_W + GAP + 20.0))
    }

    fn zone(&self, x: f32, width: f32) -> Zone {
        let (t0, t1) = Self::track(width);
        if x < t0 - 2.0 {
            Zone::Label
        } else if x > t1 + 2.0 {
            Zone::Value
        } else {
            Zone::Track
        }
    }

    fn slider_at(&self, x: f32, width: f32) -> f32 {
        let (t0, t1) = Self::track(width);
        let (lo, hi) = self.id.slider_range();
        lo + ((x - t0) / (t1 - t0)).clamp(0.0, 1.0) * (hi - lo)
    }
}

impl canvas::Program<Message> for Scrubby {
    type State = ScrubState;

    fn update(&self, state: &mut ScrubState, event: canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> (event::Status, Option<Message>) {
        let width = bounds.width;
        let (lo, hi) = self.id.slider_range();
        match event {
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => state.shift = m.shift(),
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(p) = cursor.position_in(bounds) else { return (event::Status::Ignored, None) };
                let zone = self.zone(p.x, width);
                let double = state.last_click.is_some_and(|(at, z)| z == zone && at.elapsed() < DOUBLE_CLICK);
                state.last_click = Some((Instant::now(), zone));
                if double {
                    state.last_click = None;
                    return (event::Status::Captured, Some(Message::ResetParam(self.id)));
                }
                return match zone {
                    Zone::Track => {
                        state.drag = Some(Drag::Track);
                        (event::Status::Captured, Some(Message::Param(self.id, self.slider_at(p.x, width))))
                    }
                    Zone::Value | Zone::Label => {
                        state.drag = Some(Drag::Value { acc: self.id.to_slider(self.value), last_x: p.x });
                        (event::Status::Captured, None)
                    }
                };
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let Some(p) = cursor.position_in(bounds).or_else(|| cursor.position().map(|c| Point::new(c.x - bounds.x, c.y - bounds.y))) else {
                    return (event::Status::Ignored, None);
                };
                match &mut state.drag {
                    Some(Drag::Track) => return (event::Status::Captured, Some(Message::Param(self.id, self.slider_at(p.x, width)))),
                    Some(Drag::Value { acc, last_x }) => {
                        let per_px = (hi - lo) / 300.0 * if state.shift { 0.1 } else { 1.0 };
                        *acc = (*acc + (p.x - *last_x) * per_px).clamp(lo, hi);
                        *last_x = p.x;
                        return (event::Status::Captured, Some(Message::Param(self.id, *acc)));
                    }
                    None => {}
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.drag.take().is_some() {
                    return (event::Status::Captured, Some(Message::CommitEdit));
                }
            }
            _ => {}
        }
        (event::Status::Ignored, None)
    }

    fn draw(&self, state: &ScrubState, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, cursor: mouse::Cursor) -> Vec<Geometry> {
        let (w, h) = (bounds.width, bounds.height);
        let mut frame = Frame::new(renderer, bounds.size());
        let modified = (self.value - self.id.default_value()).abs() > 1.0e-4;
        let (t0, t1) = Self::track(w);
        let cy = h * 0.5;
        let (lo, hi) = self.id.slider_range();
        let t = ((self.id.to_slider(self.value) - lo) / (hi - lo)).clamp(0.0, 1.0);
        let tx = t0 + t * (t1 - t0);

        text(&mut frame, self.id.label().to_owned(), Point::new(0.0, cy), 11.0, if modified { TEXT } else { TEXT_DIM }, Horizontal::Left, false);

        match self.id {
            ParamId::Temperature => frame.fill_rectangle_gradient(t0, t1, cy, hex(0x5EB5FF), hex(0xFF9326)),
            ParamId::Tint => frame.fill_rectangle_gradient(t0, t1, cy, hex(0x30D158), hex(0xBF5AF2)),
            _ => {
                frame.fill_rectangle(Point::new(t0, cy - 1.0), Size::new(t1 - t0, 2.0), RECESS);
                let base = ((self.id.to_slider(self.id.default_value()) - lo) / (hi - lo)).clamp(0.0, 1.0);
                let (a, b) = (t0 + base.min(t) * (t1 - t0), t0 + base.max(t) * (t1 - t0));
                frame.fill_rectangle(Point::new(a, cy - 1.0), Size::new((b - a).max(0.0), 2.0), if modified { ACCENT } else { RAIL });
            }
        }
        let hovered = cursor.position_in(bounds).is_some_and(|p| (p.x - tx).abs() < 8.0);
        let thumb = if state.drag.is_some() { hex(0x5AA2F5) } else if hovered || modified { TEXT } else { TEXT_DIM };
        frame.fill(&Path::circle(Point::new(tx, cy), 6.0), thumb);
        text(&mut frame, self.id.format_value(self.value), Point::new(w, cy), 11.0, if modified { TEXT } else { TEXT_DIM }, Horizontal::Right, true);
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, state: &ScrubState, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        match (&state.drag, cursor.position_in(bounds).map(|p| self.zone(p.x, bounds.width))) {
            (Some(_), _) => mouse::Interaction::ResizingHorizontally,
            (None, Some(Zone::Value)) => mouse::Interaction::ResizingHorizontally,
            (None, Some(Zone::Track)) => mouse::Interaction::Pointer,
            _ => mouse::Interaction::default(),
        }
    }
}

trait FrameExt {
    fn fill_rectangle_gradient(&mut self, x0: f32, x1: f32, cy: f32, a: Color, b: Color);
}

impl FrameExt for Frame {
    fn fill_rectangle_gradient(&mut self, x0: f32, x1: f32, cy: f32, a: Color, b: Color) {
        self.fill(&Path::rectangle(Point::new(x0, cy - 2.0), Size::new(x1 - x0, 4.0)), gradient_fill(x0, x1, a, b));
    }
}

// ------------------------------------------------------------------------------------------------
// 5-zone histogram
// ------------------------------------------------------------------------------------------------

/// Zone boundaries as fractions of the 0..255 axis, and the slider each zone drives.
const ZONES: [(f32, f32, ParamId, &str); 5] = [
    (0.00, 0.10, ParamId::Blacks, "Blacks"),
    (0.10, 0.35, ParamId::Shadows, "Shadows"),
    (0.35, 0.65, ParamId::Exposure, "Exposure"),
    (0.65, 0.90, ParamId::Highlights, "Highlights"),
    (0.90, 1.00, ParamId::Whites, "Whites"),
];

pub struct HistogramView {
    pub data: Option<HistogramData>,
    /// Current slider values in zone order (Blacks, Shadows, Exposure, Highlights, Whites).
    pub values: [f32; 5],
    pub clip_overlay: bool,
}

#[derive(Default)]
pub struct HistogramState {
    drag: Option<(usize, f32, f32)>, // zone, accumulated value, last x
    shift: bool,
}

impl HistogramView {
    pub fn view(self, height: f32) -> Element<'static, Message> {
        canvas(self).width(Length::Fill).height(Length::Fixed(height)).into()
    }

    fn zone_at(x: f32, width: f32) -> usize {
        let f = (x / width.max(1.0)).clamp(0.0, 0.9999);
        ZONES.iter().position(|z| f >= z.0 && f < z.1).unwrap_or(2)
    }
}

impl canvas::Program<Message> for HistogramView {
    type State = HistogramState;

    fn update(&self, state: &mut HistogramState, event: canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> (event::Status, Option<Message>) {
        match event {
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => state.shift = m.shift(),
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(p) = cursor.position_in(bounds) else { return (event::Status::Ignored, None) };
                if p.y < 20.0 && (p.x < 22.0 || p.x > bounds.width - 22.0) {
                    return (event::Status::Captured, Some(Message::ToggleClipping));
                }
                let zone = Self::zone_at(p.x, bounds.width);
                state.drag = Some((zone, self.values[zone], p.x));
                return (event::Status::Captured, None);
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let Some((zone, acc, last_x)) = state.drag else { return (event::Status::Ignored, None) };
                let Some(p) = cursor.position_in(bounds).or_else(|| cursor.position().map(|c| Point::new(c.x - bounds.x, c.y - bounds.y))) else {
                    return (event::Status::Ignored, None);
                };
                let id = ZONES[zone].2;
                let per_px = if id == ParamId::Exposure { 1.0 / 60.0 } else { 100.0 / 120.0 } * if state.shift { 0.1 } else { 1.0 };
                let next = (acc + (p.x - last_x) * per_px).clamp(id.min(), id.max());
                state.drag = Some((zone, next, p.x));
                return (event::Status::Captured, Some(Message::Param(id, id.to_slider(next))));
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.drag.take().is_some() {
                    return (event::Status::Captured, Some(Message::CommitEdit));
                }
            }
            _ => {}
        }
        (event::Status::Ignored, None)
    }

    fn draw(&self, state: &HistogramState, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, cursor: mouse::Cursor) -> Vec<Geometry> {
        let (w, h) = (bounds.width, bounds.height);
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), hex(0x1C1C1C));
        for z in &ZONES[1..] {
            frame.fill_rectangle(Point::new(z.0 * w, 0.0), Size::new(1.0, h), hex(0x2A2A2A));
        }
        if let Some(d) = &self.data {
            let peak = (d.peak() as f32).ln_1p();
            let curve = |counts: &[u32; 256], close: bool| {
                Path::new(|b| {
                    b.move_to(Point::new(0.0, h));
                    for (i, &c) in counts.iter().enumerate() {
                        b.line_to(Point::new(i as f32 / 255.0 * w, h - (c as f32).ln_1p() / peak * (h - 4.0)));
                    }
                    if close {
                        b.line_to(Point::new(w, h));
                        b.close();
                    }
                })
            };
            for (counts, color) in [(&d.r, Color::from_rgba8(255, 60, 60, 0.45)), (&d.g, Color::from_rgba8(60, 220, 90, 0.45)), (&d.b, Color::from_rgba8(70, 110, 255, 0.45))] {
                frame.fill(&curve(counts, true), color);
            }
            frame.stroke(&curve(&d.luma, false), Stroke::default().with_width(1.2).with_color(Color::from_rgba8(235, 235, 235, 0.9)));
        }
        let hovered = state.drag.map(|d| d.0).or_else(|| cursor.position_in(bounds).map(|p| Self::zone_at(p.x, w)));
        if let Some(z) = hovered {
            let (a, b, _, name) = ZONES[z];
            frame.fill_rectangle(Point::new(a * w, 0.0), Size::new((b - a) * w, h), Color::from_rgba(1.0, 1.0, 1.0, 0.08));
            text(&mut frame, format!("{name} {}", ZONES[z].2.format_value(self.values[z])), Point::new(w * 0.5, h - 9.0), 10.0, TEXT, Horizontal::Center, true);
        }
        let (lo, hi) = self.data.as_ref().map_or((false, false), |d| (d.clipped_shadows() > 0.001, d.clipped_highlights() > 0.001));
        let tri = |x: f32, flip: f32, on: bool, c: Color| {
            let c = if on { c } else { Color { a: 0.25, ..c } };
            (Path::new(move |b| {
                b.move_to(Point::new(x, 3.0));
                b.line_to(Point::new(x + 12.0 * flip, 3.0));
                b.line_to(Point::new(x, 15.0));
                b.close();
            }), c)
        };
        let (p1, c1) = tri(3.0, 1.0, lo || self.clip_overlay, hex(0x2255FF));
        let (p2, c2) = tri(w - 3.0, -1.0, hi || self.clip_overlay, hex(0xFF2222));
        frame.fill(&p1, c1);
        frame.fill(&p2, c2);
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, state: &HistogramState, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        match cursor.position_in(bounds) {
            Some(p) if state.drag.is_none() && p.y < 20.0 && (p.x < 22.0 || p.x > bounds.width - 22.0) => mouse::Interaction::Pointer,
            Some(_) => mouse::Interaction::ResizingHorizontally,
            None if state.drag.is_some() => mouse::Interaction::ResizingHorizontally,
            None => mouse::Interaction::default(),
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Tone curve editor
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CurveEdit {
    Insert { channel: CurveChannel, x: f32, y: f32 },
    Move { channel: CurveChannel, index: usize, x: f32, y: f32 },
    Remove { channel: CurveChannel, index: usize },
}

pub struct CurveEditor {
    pub curve: ToneCurve,
    pub channel: CurveChannel,
}

#[derive(Default)]
pub struct CurveState {
    dragging: Option<usize>,
    last_click: Option<(Instant, usize)>,
}

const PAD: f32 = 8.0;

impl CurveEditor {
    pub fn view(self, side: f32) -> Element<'static, Message> {
        canvas(self).width(Length::Fixed(side)).height(Length::Fixed(side)).into()
    }

    fn to_unit(p: Point, size: Size) -> (f32, f32) {
        (((p.x - PAD) / (size.width - 2.0 * PAD)).clamp(0.0, 1.0), (1.0 - (p.y - PAD) / (size.height - 2.0 * PAD)).clamp(0.0, 1.0))
    }

    fn to_screen(x: f32, y: f32, size: Size) -> Point {
        Point::new(PAD + x * (size.width - 2.0 * PAD), PAD + (1.0 - y) * (size.height - 2.0 * PAD))
    }

    fn color(&self) -> Color {
        match self.channel {
            CurveChannel::Rgb => Color::WHITE,
            CurveChannel::Red => hex(0xFF5555),
            CurveChannel::Green => hex(0x55E070),
            CurveChannel::Blue => hex(0x6699FF),
        }
    }
}

impl canvas::Program<Message> for CurveEditor {
    type State = CurveState;

    fn update(&self, state: &mut CurveState, event: canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> (event::Status, Option<Message>) {
        let size = bounds.size();
        let edit = |e: CurveEdit| (event::Status::Captured, Some(Message::Curve(e)));
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(button)) => {
                let Some(p) = cursor.position_in(bounds) else { return (event::Status::Ignored, None) };
                let (x, y) = Self::to_unit(p, size);
                let hit = self.curve.hit_test(self.channel, x, y, 0.045);
                match (button, hit) {
                    (mouse::Button::Right, Some(index)) | (mouse::Button::Left, Some(index))
                        if button == mouse::Button::Right || state.last_click.is_some_and(|(at, i)| i == index && at.elapsed() < DOUBLE_CLICK) =>
                    {
                        state.last_click = None;
                        return edit(CurveEdit::Remove { channel: self.channel, index });
                    }
                    (mouse::Button::Left, Some(index)) => {
                        state.dragging = Some(index);
                        state.last_click = Some((Instant::now(), index));
                        return (event::Status::Captured, None);
                    }
                    (mouse::Button::Left, None) => {
                        let mut probe = self.curve.clone();
                        if let Some(index) = probe.insert_point(self.channel, x, y) {
                            state.dragging = Some(index);
                            state.last_click = Some((Instant::now(), index));
                            return edit(CurveEdit::Insert { channel: self.channel, x, y });
                        }
                    }
                    _ => {}
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(index) = state.dragging {
                    let p = cursor.position().map(|c| Point::new(c.x - bounds.x, c.y - bounds.y));
                    if let Some(p) = p {
                        let (x, y) = Self::to_unit(p, size);
                        return edit(CurveEdit::Move { channel: self.channel, index, x, y });
                    }
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.dragging.take().is_some() {
                    return (event::Status::Captured, Some(Message::CommitEdit));
                }
            }
            _ => {}
        }
        (event::Status::Ignored, None)
    }

    fn draw(&self, state: &CurveState, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, cursor: mouse::Cursor) -> Vec<Geometry> {
        let size = bounds.size();
        let mut frame = Frame::new(renderer, size);
        frame.fill_rectangle(Point::ORIGIN, size, hex(0x1C1C1C));
        let grid = Stroke::default().with_width(1.0).with_color(hex(0x2E2E2E));
        for k in 1..4 {
            let f = k as f32 / 4.0;
            frame.stroke(&Path::line(Self::to_screen(f, 0.0, size), Self::to_screen(f, 1.0, size)), grid);
            frame.stroke(&Path::line(Self::to_screen(0.0, f, size), Self::to_screen(1.0, f, size)), grid);
        }
        frame.stroke(&Path::line(Self::to_screen(0.0, 0.0, size), Self::to_screen(1.0, 1.0, size)), Stroke::default().with_width(1.0).with_color(hex(0x3A3A3A)));
        let line = Path::new(|b| {
            for i in 0..=160 {
                let x = i as f32 / 160.0;
                let p = Self::to_screen(x, self.curve.evaluate(self.channel, x).clamp(0.0, 1.0), size);
                if i == 0 {
                    b.move_to(p);
                } else {
                    b.line_to(p);
                }
            }
        });
        frame.stroke(&line, Stroke::default().with_width(1.8).with_color(self.color()));
        let hover = cursor.position_in(bounds).and_then(|p| {
            let (x, y) = Self::to_unit(p, size);
            self.curve.hit_test(self.channel, x, y, 0.045)
        });
        for (i, pt) in self.curve.points(self.channel).iter().enumerate() {
            let at = Self::to_screen(pt[0], pt[1], size);
            let active = state.dragging == Some(i) || hover == Some(i);
            frame.fill(&Path::circle(at, if active { 5.5 } else { 4.0 }), if active { self.color() } else { hex(0x1C1C1C) });
            frame.stroke(&Path::circle(at, if active { 5.5 } else { 4.0 }), Stroke::default().with_width(1.5).with_color(self.color()));
        }
        let count = self.curve.points(self.channel).len();
        text(&mut frame, format!("{count}/{MAX_CURVE_POINTS}"), Point::new(size.width - 10.0, size.height - 10.0), 9.0, hex(0x6E6E6E), Horizontal::Right, true);
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, state: &CurveState, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        match cursor.position_in(bounds) {
            Some(_) if state.dragging.is_some() => mouse::Interaction::Grabbing,
            Some(p) => {
                let (x, y) = Self::to_unit(p, bounds.size());
                if self.curve.hit_test(self.channel, x, y, 0.045).is_some() { mouse::Interaction::Grab } else { mouse::Interaction::Crosshair }
            }
            None => mouse::Interaction::default(),
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Colour wheel
// ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelId {
    Shadows,
    Midtones,
    Highlights,
    Global,
}

impl WheelId {
    pub const ALL: [WheelId; 4] = [WheelId::Shadows, WheelId::Midtones, WheelId::Highlights, WheelId::Global];

    pub fn label(self) -> &'static str {
        match self {
            WheelId::Shadows => "Shadows",
            WheelId::Midtones => "Midtones",
            WheelId::Highlights => "Highlights",
            WheelId::Global => "Global",
        }
    }
}

pub struct WheelView {
    pub id: WheelId,
    pub wheel: Wheel,
}

#[derive(Default)]
pub struct WheelState {
    dragging: bool,
    last_click: Option<Instant>,
}

impl WheelView {
    pub fn view(id: WheelId, wheel: Wheel, side: f32) -> Element<'static, Message> {
        canvas(Self { id, wheel }).width(Length::Fixed(side)).height(Length::Fixed(side)).into()
    }

    fn puck_from(p: Point, size: Size) -> (f32, f32) {
        let r = size.width.min(size.height) * 0.5 - 6.0;
        ((p.x - size.width * 0.5) / r, -(p.y - size.height * 0.5) / r)
    }
}

impl canvas::Program<Message> for WheelView {
    type State = WheelState;

    fn update(&self, state: &mut WheelState, event: canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> (event::Status, Option<Message>) {
        let apply = |p: Point| {
            let (x, y) = Self::puck_from(p, bounds.size());
            let mut w = self.wheel;
            w.set_from_puck(x, y);
            Some(Message::Wheel(self.id, w))
        };
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(p) = cursor.position_in(bounds) else { return (event::Status::Ignored, None) };
                if state.last_click.is_some_and(|at| at.elapsed() < DOUBLE_CLICK) {
                    state.last_click = None;
                    return (event::Status::Captured, Some(Message::Ext(crate::ext::ExtMessage::WheelReset(self.id))));
                }
                state.last_click = Some(Instant::now());
                state.dragging = true;
                return (event::Status::Captured, apply(p));
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if state.dragging => {
                if let Some(c) = cursor.position() {
                    return (event::Status::Captured, apply(Point::new(c.x - bounds.x, c.y - bounds.y)));
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if std::mem::take(&mut state.dragging) {
                    return (event::Status::Captured, Some(Message::CommitEdit));
                }
            }
            _ => {}
        }
        (event::Status::Ignored, None)
    }

    fn draw(&self, _state: &WheelState, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let size = bounds.size();
        let mut frame = Frame::new(renderer, size);
        let (c, r) = (Point::new(size.width * 0.5, size.height * 0.5), size.width.min(size.height) * 0.5 - 6.0);

        // Hue/saturation gamut: concentric bands fade from white at the centre to fully saturated
        // hue at the rim. Draw the bands as arcs (like the legacy wheel) instead of filling the
        // disc with flat wedges, and never cover it with opaque rings.
        const RINGS: usize = 10;
        const SEGMENTS: usize = 72;
        let band = r / RINGS as f32;
        for ring in 0..RINGS {
            let sat = (ring as f32 + 0.5) / RINGS as f32;
            let radius = r * sat;
            for i in 0..SEGMENTS {
                let (a0, a1) = (
                    i as f32 / SEGMENTS as f32 * std::f32::consts::TAU,
                    (i as f32 + 1.0) / SEGMENTS as f32 * std::f32::consts::TAU,
                );
                // screen y points down, so negate the angles to keep hue increasing counter-clockwise
                let [hr, hg, hb] = halation_core::grading::hue_to_rgb(a0.to_degrees());
                let color = Color::from_rgb(1.0 + (hr - 1.0) * sat, 1.0 + (hg - 1.0) * sat, 1.0 + (hb - 1.0) * sat);
                let arc = Path::new(|b| {
                    b.arc(Arc { center: c, radius, start_angle: Radians(-a1), end_angle: Radians(-a0 + 0.03) });
                });
                frame.stroke(&arc, Stroke::default().with_color(color).with_width(band + 0.7));
            }
        }

        // Faint reference rings: strokes only, so the gamut stays visible underneath.
        for k in (1..RINGS).rev() {
            frame.stroke(
                &Path::circle(c, r * k as f32 / RINGS as f32),
                Stroke::default().with_width(1.0).with_color(Color::from_rgba(1.0, 1.0, 1.0, 0.06)),
            );
        }
        frame.stroke(&Path::circle(c, r), Stroke::default().with_width(1.5).with_color(hex(0x3A3A3A)));
        frame.fill(&Path::circle(c, 2.0), hex(0x1C1C1C));
        let [px, py] = self.wheel.puck();
        let puck = Point::new(c.x + px * r, c.y - py * r);
        let [pr, pg, pb] = halation_core::grading::hue_to_rgb(self.wheel.hue);
        frame.fill(&Path::circle(puck, 6.5), Color::from_rgb(pr, pg, pb));
        frame.stroke(&Path::circle(puck, 6.5), Stroke::default().with_width(2.0).with_color(Color::WHITE));
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, state: &WheelState, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if state.dragging {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(bounds) {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_zones_partition_the_axis() {
        assert_eq!(HistogramView::zone_at(0.0, 300.0), 0);
        assert_eq!(HistogramView::zone_at(299.0, 300.0), 4);
        assert_eq!(HistogramView::zone_at(150.0, 300.0), 2);
        assert_eq!(HistogramView::zone_at(60.0, 300.0), 1);
        assert_eq!(HistogramView::zone_at(240.0, 300.0), 3);
    }

    #[test]
    fn curve_coordinates_roundtrip() {
        let size = Size::new(296.0, 296.0);
        let p = CurveEditor::to_screen(0.25, 0.75, size);
        let (x, y) = CurveEditor::to_unit(p, size);
        assert!((x - 0.25).abs() < 1.0e-5 && (y - 0.75).abs() < 1.0e-5);
    }
}
