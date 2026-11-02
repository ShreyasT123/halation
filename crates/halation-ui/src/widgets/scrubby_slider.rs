use iced::{
    mouse,
    widget::canvas::{self, Frame, Geometry, Program},
    Color, Length, Point, Rectangle, Size, Theme,
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gradient {
    Neutral,
    Temperature,
    Tint,
}

#[derive(Debug, Clone, Copy)]
pub enum Output {
    Changed(f32),
}

#[derive(Default)]
struct Interaction {
    dragging: bool,
    relative: bool,
    last_x: f32,
    shift: bool,
    last_click: Option<Instant>,
}

pub fn view<'a>(
    label: &'static str,
    value: f32,
    min: f32,
    max: f32,
    default: f32,
    step: f32,
    gradient: Gradient,
) -> iced::Element<'a, Output> {
    canvas::Canvas::new(ScrubbySlider {
        label,
        value,
        min,
        max,
        default,
        step,
        gradient,
    })
    .width(Length::Fill)
    .height(Length::Fixed(28.0))
    .into()
}

struct ScrubbySlider {
    label: &'static str,
    value: f32,
    min: f32,
    max: f32,
    default: f32,
    step: f32,
    gradient: Gradient,
}

impl ScrubbySlider {
    fn track(&self, width: f32) -> (f32, f32) {
        (86.0, (width - 66.0).max(94.0))
    }
    fn value_at(&self, x: f32, width: f32) -> f32 {
        let (left, right) = self.track(width);
        let t = ((x - left) / (right - left)).clamp(0.0, 1.0);
        self.min + t * (self.max - self.min)
    }
}

impl Program<Output> for ScrubbySlider {
    type State = Interaction;

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Output>) {
        match event {
            canvas::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Shift),
                ..
            }) => {
                state.shift = true;
                (canvas::event::Status::Captured, None)
            }
            canvas::Event::Keyboard(iced::keyboard::Event::KeyReleased {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Shift),
                ..
            }) => {
                state.shift = false;
                (canvas::event::Status::Captured, None)
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(position) = cursor.position_in(bounds) else {
                    return (canvas::event::Status::Ignored, None);
                };
                let now = Instant::now();
                if state
                    .last_click
                    .is_some_and(|last| now.duration_since(last) < Duration::from_millis(360))
                {
                    state.last_click = None;
                    return (
                        canvas::event::Status::Captured,
                        Some(Output::Changed(self.default)),
                    );
                }
                state.last_click = Some(now);
                state.dragging = true;
                state.relative = position.x >= bounds.width - 62.0;
                state.last_x = position.x;
                if state.relative {
                    (canvas::event::Status::Captured, None)
                } else {
                    (
                        canvas::event::Status::Captured,
                        Some(Output::Changed(self.value_at(position.x, bounds.width))),
                    )
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) if state.dragging => {
                let position = Point::new(position.x - bounds.x, position.y - bounds.y);
                if state.relative {
                    let delta = position.x - state.last_x;
                    state.last_x = position.x;
                    let precision = if state.shift { 0.1 } else { 1.0 };
                    (
                        canvas::event::Status::Captured,
                        Some(Output::Changed(
                            (self.value + delta * self.step * precision).clamp(self.min, self.max),
                        )),
                    )
                } else {
                    (
                        canvas::event::Status::Captured,
                        Some(Output::Changed(self.value_at(position.x, bounds.width))),
                    )
                }
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
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_text(canvas::Text {
            content: self.label.into(),
            position: Point::new(0.0, 17.0),
            color: Color::from_rgb(0.70, 0.71, 0.74),
            size: 11.0.into(),
            ..canvas::Text::default()
        });
        let (left, right) = self.track(bounds.width);
        let track_width = right - left;
        let y = 15.0;
        if self.gradient == Gradient::Neutral {
            frame.fill_rectangle(
                Point::new(left, y),
                Size::new(track_width, 3.0),
                Color::from_rgb(0.17, 0.18, 0.20),
            );
        } else {
            let colors = match self.gradient {
                Gradient::Temperature => (
                    Color::from_rgb(1.0, 0.58, 0.15),
                    Color::from_rgb(0.37, 0.71, 1.0),
                ),
                Gradient::Tint => (
                    Color::from_rgb(0.19, 0.82, 0.33),
                    Color::from_rgb(0.75, 0.35, 0.95),
                ),
                Gradient::Neutral => unreachable!(),
            };
            for i in 0..48 {
                let t = i as f32 / 47.0;
                let color = Color::from_rgb(
                    colors.0.r + (colors.1.r - colors.0.r) * t,
                    colors.0.g + (colors.1.g - colors.0.g) * t,
                    colors.0.b + (colors.1.b - colors.0.b) * t,
                );
                frame.fill_rectangle(
                    Point::new(left + track_width * t, y),
                    Size::new(track_width / 48.0 + 1.0, 3.0),
                    color,
                );
            }
        }
        let t = ((self.value - self.min) / (self.max - self.min)).clamp(0.0, 1.0);
        let x = left + track_width * t;
        frame.fill(
            &canvas::Path::circle(Point::new(x, y + 1.5), 6.0),
            Color::from_rgb(0.91, 0.92, 0.94),
        );
        frame.fill_text(canvas::Text {
            content: format!("{:+.2}", self.value),
            position: Point::new(bounds.width - 2.0, 17.0),
            color: Color::from_rgb(0.86, 0.87, 0.89),
            size: 11.0.into(),
            horizontal_alignment: iced::alignment::Horizontal::Right,
            ..canvas::Text::default()
        });
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
