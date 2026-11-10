//! Precision loupe navigation (Sprint 3): a canvas viewport and the navigator mini-map.
//!
//! * Scale is always *screen pixels per full-resolution image pixel*, so `1:1` really is 1:1 even
//!   though the interactive preview is smaller (the app paints a native-resolution detail layer over
//!   the preview when zoomed past the preview's own resolution).
//! * Wheel zoom keeps the image point under the cursor fixed; click toggles Fit ↔ 1:1 at the click
//!   point; `Space`-drag, middle-drag or plain drag (when zoomed) pans, with the panned centre clamped so
//!   the image can never leave the view.
//! * Compare modes: hold `\` to peek the baseline, movable wipe, side-by-side and top/bottom panes —
//!   all panes share one transform, so they pan and zoom in lockstep.
//!
//! Needs iced feature `canvas`.

use crate::{Message, ZoomMode};
use iced::{
    event, keyboard, mouse,
    widget::{canvas, canvas::{Frame, Geometry, Path, Stroke}, image},
    Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector,
};
use std::time::{Duration, Instant};

pub const MIN_SCALE: f32 = 0.02;
pub const MAX_SCALE: f32 = 8.0;
const CLICK_SLOP: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewTransform {
    pub mode: ZoomMode,
    /// Pixels per full-resolution pixel when `mode == Custom`.
    pub scale: f32,
    /// Normalised image coordinate shown at the pane centre.
    pub center: [f32; 2],
}

impl Default for ViewTransform {
    fn default() -> Self {
        Self { mode: ZoomMode::Fit, scale: 1.0, center: [0.5, 0.5] }
    }
}

impl ViewTransform {
    pub fn fit_scale(image: (u32, u32), pane: Size) -> f32 {
        (pane.width / image.0.max(1) as f32).min(pane.height / image.1.max(1) as f32).max(1.0e-4)
    }

    pub fn effective_scale(&self, image: (u32, u32), pane: Size) -> f32 {
        let (iw, ih) = (image.0.max(1) as f32, image.1.max(1) as f32);
        match self.mode {
            ZoomMode::Fit => Self::fit_scale(image, pane),
            ZoomMode::Fill => (pane.width / iw).max(pane.height / ih),
            ZoomMode::Actual => 1.0,
            ZoomMode::Custom => self.scale.clamp(MIN_SCALE, MAX_SCALE),
        }
    }

    /// Centre clamped so the image always covers the pane along any axis where it is larger.
    pub fn clamped_center(&self, image: (u32, u32), pane: Size) -> [f32; 2] {
        let s = self.effective_scale(image, pane);
        let size = [image.0 as f32 * s, image.1 as f32 * s];
        let dims = [pane.width, pane.height];
        let mut c = self.center;
        for k in 0..2 {
            let half = (dims[k] / size[k]) * 0.5;
            c[k] = if size[k] <= dims[k] { 0.5 } else { c[k].clamp(half, 1.0 - half) };
        }
        c
    }

    /// Screen-space rectangle of the full image inside a pane of size `pane`.
    pub fn image_rect(&self, image: (u32, u32), pane: Size) -> Rectangle {
        let s = self.effective_scale(image, pane);
        let c = self.clamped_center(image, pane);
        let (w, h) = (image.0 as f32 * s, image.1 as f32 * s);
        Rectangle::new(Point::new(pane.width * 0.5 - c[0] * w, pane.height * 0.5 - c[1] * h), Size::new(w, h))
    }

    /// Visible part of the image as a normalised rectangle `[x, y, w, h]`.
    pub fn visible_rect(&self, image: (u32, u32), pane: Size) -> [f32; 4] {
        let r = self.image_rect(image, pane);
        let x0 = (-r.x / r.width).clamp(0.0, 1.0);
        let y0 = (-r.y / r.height).clamp(0.0, 1.0);
        let x1 = ((pane.width - r.x) / r.width).clamp(0.0, 1.0);
        let y1 = ((pane.height - r.y) / r.height).clamp(0.0, 1.0);
        [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
    }

    /// Zoom to `scale` keeping the image point under `anchor` (pane coordinates) fixed.
    pub fn zoomed_at(&self, image: (u32, u32), pane: Size, anchor: Point, scale: f32) -> Self {
        let r = self.image_rect(image, pane);
        let u = [(anchor.x - r.x) / r.width, (anchor.y - r.y) / r.height];
        let scale = scale.clamp(MIN_SCALE, MAX_SCALE);
        let (w, h) = (image.0 as f32 * scale, image.1 as f32 * scale);
        let top_left = [anchor.x - u[0] * w, anchor.y - u[1] * h];
        let center = [(pane.width * 0.5 - top_left[0]) / w, (pane.height * 0.5 - top_left[1]) / h];
        let mut t = Self { mode: ZoomMode::Custom, scale, center };
        t.center = t.clamped_center(image, pane);
        if scale <= Self::fit_scale(image, pane) * 1.0001 {
            t = Self::default();
        }
        t
    }

    /// Toggle Fit ↔ 1:1 around `anchor`.
    pub fn toggled_at(&self, image: (u32, u32), pane: Size, anchor: Point) -> Self {
        if self.mode == ZoomMode::Fit {
            let mut t = self.zoomed_at(image, pane, anchor, 1.0);
            t.mode = ZoomMode::Actual;
            t
        } else {
            Self::default()
        }
    }

    pub fn panned(&self, image: (u32, u32), pane: Size, delta: Vector) -> Self {
        let s = self.effective_scale(image, pane);
        let c = self.clamped_center(image, pane);
        let mut t = Self {
            mode: if self.mode == ZoomMode::Fit || self.mode == ZoomMode::Fill { ZoomMode::Custom } else { self.mode },
            scale: s,
            center: [c[0] - delta.x / (image.0 as f32 * s), c[1] - delta.y / (image.1 as f32 * s)],
        };
        t.center = t.clamped_center(image, pane);
        t
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Compare {
    Off,
    /// Showing the baseline while `\` is held.
    Peek,
    Wipe(f32),
    SideBySide,
    TopBottom,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewEvent {
    Set(ViewTransform),
    Divider(f32),
    /// White-balance eyedropper sample in normalised image coordinates.
    Sample(f32, f32),
}

/// Native-resolution detail layer drawn over the preview (normalised rect `[x, y, w, h]`).
#[derive(Debug, Clone)]
pub struct Detail {
    pub rect: [f32; 4],
    pub handle: image::Handle,
}

pub struct Viewport {
    pub edited: image::Handle,
    pub before: Option<image::Handle>,
    pub detail: Option<Detail>,
    pub image_size: (u32, u32),
    pub view: ViewTransform,
    pub compare: Compare,
    pub background: Color,
    /// When armed, a click reports `ViewEvent::Sample` instead of toggling zoom.
    pub sampling: bool,
}

#[derive(Default)]
pub struct Interaction {
    space_down: bool,
    pan: Option<PanState>,
    press: Option<(Point, Instant)>,
    divider_drag: bool,
}

struct PanState {
    last: Point,
    travelled: f32,
    button: mouse::Button,
}

impl Viewport {
    pub fn view(self) -> Element<'static, Message> {
        canvas(self).width(Length::Fill).height(Length::Fill).into()
    }

    /// Panes `(rect, shows_before)` for the current compare mode, in canvas coordinates.
    fn panes(&self, size: Size) -> Vec<(Rectangle, bool)> {
        let full = Rectangle::new(Point::ORIGIN, size);
        match self.compare {
            Compare::Off | Compare::Wipe(_) => vec![(full, false)],
            Compare::Peek => vec![(full, true)],
            Compare::SideBySide => {
                let half = size.width * 0.5;
                vec![
                    (Rectangle::new(Point::ORIGIN, Size::new(half, size.height)), true),
                    (Rectangle::new(Point::new(half, 0.0), Size::new(half, size.height)), false),
                ]
            }
            Compare::TopBottom => {
                let half = size.height * 0.5;
                vec![
                    (Rectangle::new(Point::ORIGIN, Size::new(size.width, half)), true),
                    (Rectangle::new(Point::new(0.0, half), Size::new(size.width, half)), false),
                ]
            }
        }
    }

    fn pane_at(&self, size: Size, p: Point) -> Rectangle {
        self.panes(size).into_iter().map(|(r, _)| r).find(|r| r.contains(p)).unwrap_or(Rectangle::new(Point::ORIGIN, size))
    }

    fn draw_pane(&self, frame: &mut Frame, pane: Rectangle, before: bool) {
        frame.with_clip(pane, |f| {
            let size = pane.size();
            let r = self.view.image_rect(self.image_size, size);
            let handle = if before { self.before.as_ref().unwrap_or(&self.edited) } else { &self.edited };
            f.draw_image(r, handle);
            if !before {
                if let Some(d) = &self.detail {
                    let rect = Rectangle::new(
                        Point::new(r.x + d.rect[0] * r.width, r.y + d.rect[1] * r.height),
                        Size::new(d.rect[2] * r.width, d.rect[3] * r.height),
                    );
                    f.draw_image(rect, &d.handle);
                }
            }
        });
    }

    fn wipe_x(&self, size: Size) -> Option<f32> {
        match self.compare {
            Compare::Wipe(d) => Some(size.width * d.clamp(0.0, 1.0)),
            _ => None,
        }
    }
}

impl canvas::Program<Message> for Viewport {
    type State = Interaction;

    fn update(&self, state: &mut Interaction, event: canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> (event::Status, Option<Message>) {
        let size = bounds.size();
        let local = cursor.position_in(bounds);
        let emit = |t: ViewTransform| (event::Status::Captured, Some(Message::View(ViewEvent::Set(t))));
        match event {
            canvas::Event::Keyboard(keyboard::Event::KeyPressed { key: keyboard::Key::Named(keyboard::key::Named::Space), .. }) => {
                state.space_down = true;
            }
            canvas::Event::Keyboard(keyboard::Event::KeyReleased { key: keyboard::Key::Named(keyboard::key::Named::Space), .. }) => {
                state.space_down = false;
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(button)) => {
                let Some(p) = local else { return (event::Status::Ignored, None) };
                if let (Some(x), mouse::Button::Left) = (self.wipe_x(size), button) {
                    if (p.x - x).abs() < 10.0 {
                        state.divider_drag = true;
                        return (event::Status::Captured, None);
                    }
                }
                let pannable = state.space_down || button == mouse::Button::Middle || self.view.mode != ZoomMode::Fit;
                state.pan = (button == mouse::Button::Left || button == mouse::Button::Middle).then_some(PanState { last: p, travelled: 0.0, button });
                if button == mouse::Button::Left {
                    state.press = Some((p, Instant::now()));
                }
                if pannable {
                    return (event::Status::Captured, None);
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let Some(p) = local else { return (event::Status::Ignored, None) };
                if state.divider_drag {
                    return (event::Status::Captured, Some(Message::View(ViewEvent::Divider((p.x / size.width.max(1.0)).clamp(0.02, 0.98)))));
                }
                if let Some(pan) = &mut state.pan {
                    let delta = Vector::new(p.x - pan.last.x, p.y - pan.last.y);
                    pan.last = p;
                    pan.travelled += delta.x.abs() + delta.y.abs();
                    let allowed = state.space_down || pan.button == mouse::Button::Middle || self.view.mode != ZoomMode::Fit;
                    if allowed && pan.travelled > CLICK_SLOP {
                        let pane = self.pane_at(size, p).size();
                        return emit(self.view.panned(self.image_size, pane, delta));
                    }
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(button)) => {
                state.divider_drag = false;
                let travelled = state.pan.as_ref().map_or(0.0, |p| p.travelled);
                state.pan = None;
                if button == mouse::Button::Left && travelled <= CLICK_SLOP {
                    if let (Some((p, at)), Some(_)) = (state.press.take(), local) {
                        if at.elapsed() < Duration::from_millis(600) && !state.space_down {
                            let pane = self.pane_at(size, p);
                            if self.sampling {
                                let r = self.view.image_rect(self.image_size, pane.size());
                                let u = ((p.x - pane.x - r.x) / r.width.max(1.0)).clamp(0.0, 1.0);
                                let v = ((p.y - pane.y - r.y) / r.height.max(1.0)).clamp(0.0, 1.0);
                                return (event::Status::Captured, Some(Message::View(ViewEvent::Sample(u, v))));
                            }
                            let anchor = Point::new(p.x - pane.x, p.y - pane.y);
                            return emit(self.view.toggled_at(self.image_size, pane.size(), anchor));
                        }
                    }
                }
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let Some(p) = local else { return (event::Status::Ignored, None) };
                let dy = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y,
                    mouse::ScrollDelta::Pixels { y, .. } => y / 40.0,
                };
                let pane = self.pane_at(size, p);
                let anchor = Point::new(p.x - pane.x, p.y - pane.y);
                let current = self.view.effective_scale(self.image_size, pane.size());
                let next = (current * (dy * 0.15).exp()).clamp(MIN_SCALE, MAX_SCALE);
                return emit(self.view.zoomed_at(self.image_size, pane.size(), anchor, next));
            }
            _ => {}
        }
        (event::Status::Ignored, None)
    }

    fn draw(&self, _state: &Interaction, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let size = bounds.size();
        let mut frame = Frame::new(renderer, size);
        frame.fill_rectangle(Point::ORIGIN, size, self.background);
        for (pane, before) in self.panes(size) {
            self.draw_pane(&mut frame, pane, before);
        }
        if let (Some(x), Some(before)) = (self.wipe_x(size), &self.before) {
            frame.with_clip(Rectangle::new(Point::ORIGIN, Size::new(x, size.height)), |f| {
                let r = self.view.image_rect(self.image_size, size);
                f.draw_image(r, before);
            });
            frame.fill_rectangle(Point::new(x - 1.0, 0.0), Size::new(2.0, size.height), Color::WHITE);
            frame.fill(&Path::circle(Point::new(x, size.height * 0.5), 9.0), Color::WHITE);
            frame.stroke(&Path::line(Point::new(x - 3.0, size.height * 0.5), Point::new(x + 3.0, size.height * 0.5)), Stroke::default().with_width(1.5).with_color(Color::BLACK));
        }
        if matches!(self.compare, Compare::SideBySide) {
            frame.fill_rectangle(Point::new(size.width * 0.5 - 0.5, 0.0), Size::new(1.0, size.height), Color::from_rgb8(0x2C, 0x2C, 0x2C));
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, state: &Interaction, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        let Some(p) = cursor.position_in(bounds) else { return mouse::Interaction::default() };
        if self.sampling {
            return mouse::Interaction::Crosshair;
        }
        if self.wipe_x(bounds.size()).is_some_and(|x| (p.x - x).abs() < 10.0) || state.divider_drag {
            return mouse::Interaction::ResizingHorizontally;
        }
        match (&state.pan, state.space_down || self.view.mode != ZoomMode::Fit) {
            (Some(pan), true) if pan.travelled > CLICK_SLOP => mouse::Interaction::Grabbing,
            (_, true) => mouse::Interaction::Grab,
            _ => mouse::Interaction::default(),
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Navigator mini-map
// ------------------------------------------------------------------------------------------------

/// 180-px overview with a red frustum rectangle; click or drag recentres the loupe.
pub struct Navigator {
    pub thumbnail: Option<image::Handle>,
    pub image_size: (u32, u32),
    pub view: ViewTransform,
    /// Size of the loupe pane the frustum is computed for.
    pub pane: Size,
}

#[derive(Default)]
pub struct NavigatorState {
    dragging: bool,
}

impl Navigator {
    pub fn view(self, width: f32) -> Element<'static, Message> {
        let aspect = self.image_size.1.max(1) as f32 / self.image_size.0.max(1) as f32;
        canvas(self).width(Length::Fixed(width)).height(Length::Fixed((width * aspect).clamp(40.0, width * 1.5))).into()
    }

    fn fitted(&self, size: Size) -> Rectangle {
        let scale = ViewTransform::fit_scale(self.image_size, size);
        let (w, h) = (self.image_size.0 as f32 * scale, self.image_size.1 as f32 * scale);
        Rectangle::new(Point::new((size.width - w) * 0.5, (size.height - h) * 0.5), Size::new(w, h))
    }

    fn recentre(&self, size: Size, p: Point) -> Option<Message> {
        if self.view.mode == ZoomMode::Fit {
            return None;
        }
        let r = self.fitted(size);
        let c = [((p.x - r.x) / r.width).clamp(0.0, 1.0), ((p.y - r.y) / r.height).clamp(0.0, 1.0)];
        let mut t = ViewTransform { scale: self.view.effective_scale(self.image_size, self.pane), center: c, mode: ZoomMode::Custom };
        t.center = t.clamped_center(self.image_size, self.pane);
        Some(Message::View(ViewEvent::Set(t)))
    }
}

impl canvas::Program<Message> for Navigator {
    type State = NavigatorState;

    fn update(&self, state: &mut NavigatorState, event: canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> (event::Status, Option<Message>) {
        let local = cursor.position_in(bounds);
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(p) = local {
                    state.dragging = true;
                    return (event::Status::Captured, self.recentre(bounds.size(), p));
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if state.dragging => {
                if let Some(p) = local {
                    return (event::Status::Captured, self.recentre(bounds.size(), p));
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => state.dragging = false,
            _ => {}
        }
        (event::Status::Ignored, None)
    }

    fn draw(&self, _state: &NavigatorState, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let size = bounds.size();
        let mut frame = Frame::new(renderer, size);
        frame.fill_rectangle(Point::ORIGIN, size, Color::from_rgb8(0x16, 0x16, 0x16));
        let r = self.fitted(size);
        if let Some(t) = &self.thumbnail {
            frame.draw_image(r, t);
        }
        if self.view.mode != ZoomMode::Fit {
            let v = self.view.visible_rect(self.image_size, self.pane);
            let rect = Rectangle::new(Point::new(r.x + v[0] * r.width, r.y + v[1] * r.height), Size::new(v[2] * r.width, v[3] * r.height));
            frame.stroke(&Path::rectangle(rect.position(), rect.size()), Stroke::default().with_width(1.5).with_color(Color::from_rgb8(0xFF, 0x22, 0x22)));
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, _state: &NavigatorState, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if cursor.is_over(bounds) && self.view.mode != ZoomMode::Fit {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMG: (u32, u32) = (6000, 4000);
    const PANE: Size = Size { width: 1200.0, height: 800.0 };

    #[test]
    fn fit_fills_the_pane_and_centres() {
        let v = ViewTransform::default();
        assert!((v.effective_scale(IMG, PANE) - 0.2).abs() < 1.0e-6);
        let r = v.image_rect(IMG, PANE);
        assert!((r.x).abs() < 1.0e-3 && (r.width - 1200.0).abs() < 1.0e-3);
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor_fixed() {
        let v = ViewTransform::default();
        let anchor = Point::new(900.0, 300.0);
        let before = v.image_rect(IMG, PANE);
        let u = [(anchor.x - before.x) / before.width, (anchor.y - before.y) / before.height];
        let z = v.zoomed_at(IMG, PANE, anchor, 1.0);
        let after = z.image_rect(IMG, PANE);
        assert!((after.x + u[0] * after.width - anchor.x).abs() < 0.5);
        assert!((after.y + u[1] * after.height - anchor.y).abs() < 0.5);
    }

    #[test]
    fn panning_is_clamped_to_the_image() {
        let z = ViewTransform::default().zoomed_at(IMG, PANE, Point::new(600.0, 400.0), 1.0);
        let far = z.panned(IMG, PANE, Vector::new(1.0e6, 1.0e6));
        let r = far.image_rect(IMG, PANE);
        assert!(r.x <= 0.0 + 1.0e-3 && r.y <= 0.0 + 1.0e-3);
        let vis = far.visible_rect(IMG, PANE);
        assert!(vis[0] >= 0.0 && vis[1] >= 0.0);
    }

    #[test]
    fn click_toggles_between_fit_and_one_to_one() {
        let v = ViewTransform::default();
        let one = v.toggled_at(IMG, PANE, Point::new(300.0, 200.0));
        assert_eq!(one.mode, ZoomMode::Actual);
        assert!((one.effective_scale(IMG, PANE) - 1.0).abs() < 1.0e-6);
        assert_eq!(one.toggled_at(IMG, PANE, Point::ORIGIN).mode, ZoomMode::Fit);
    }
}
