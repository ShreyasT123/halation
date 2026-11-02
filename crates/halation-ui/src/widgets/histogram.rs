use halation_core::HistogramData;
use iced::{
    mouse,
    widget::canvas::{self, Frame, Geometry, Program},
    Color, Length, Point, Rectangle, Size, Theme,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Blacks,
    Shadows,
    Exposure,
    Highlights,
    Whites,
}

#[derive(Debug, Clone, Copy)]
pub enum Output {
    Adjust(Zone, f32),
    ToggleClipping(usize),
}

#[derive(Default)]
struct Interaction {
    dragging: Option<Zone>,
    last_x: f32,
}

pub fn view<'a>(data: Option<HistogramData>, clipping: [bool; 2]) -> iced::Element<'a, Output> {
    canvas::Canvas::new(Histogram { data, clipping })
        .width(Length::Fill)
        .height(Length::Fixed(92.0))
        .into()
}

struct Histogram {
    data: Option<HistogramData>,
    clipping: [bool; 2],
}

fn zone_at(x: f32, width: f32) -> Zone {
    let t = (x / width).clamp(0.0, 0.999);
    match t {
        t if t < 0.10 => Zone::Blacks,
        t if t < 0.35 => Zone::Shadows,
        t if t < 0.65 => Zone::Exposure,
        t if t < 0.90 => Zone::Highlights,
        _ => Zone::Whites,
    }
}

impl Program<Output> for Histogram {
    type State = Interaction;

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Output>) {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(p) = cursor.position_in(bounds) else {
                    return (canvas::event::Status::Ignored, None);
                };
                if p.y < 18.0 && p.x < 22.0 {
                    return (
                        canvas::event::Status::Captured,
                        Some(Output::ToggleClipping(0)),
                    );
                }
                if p.y < 18.0 && p.x > bounds.width - 22.0 {
                    return (
                        canvas::event::Status::Captured,
                        Some(Output::ToggleClipping(1)),
                    );
                }
                let zone = zone_at(p.x, bounds.width);
                state.dragging = Some(zone);
                state.last_x = p.x;
                (canvas::event::Status::Captured, None)
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position })
                if state.dragging.is_some() =>
            {
                let x = position.x - bounds.x;
                let delta = x - state.last_x;
                state.last_x = x;
                let zone = state.dragging.unwrap();
                let amount = if zone == Zone::Exposure {
                    delta * 0.02
                } else {
                    delta * 0.5
                };
                (
                    canvas::event::Status::Captured,
                    Some(Output::Adjust(zone, amount)),
                )
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging = None;
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
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            Color::from_rgb(0.055, 0.06, 0.07),
        );
        let columns = 5;
        let zone_width = bounds.width / columns as f32;
        if let Some(p) = cursor.position_in(bounds) {
            let zone = zone_at(p.x, bounds.width) as usize;
            frame.fill_rectangle(
                Point::new(zone as f32 * zone_width, 0.0),
                Size::new(zone_width, bounds.height),
                Color::from_rgba(1.0, 1.0, 1.0, 0.07),
            );
        }
        for i in 1..columns {
            let x = i as f32 * zone_width;
            frame.fill_rectangle(
                Point::new(x, 0.0),
                Size::new(1.0, bounds.height),
                Color::from_rgba(0.6, 0.62, 0.65, 0.28),
            );
        }
        if let Some(data) = &self.data {
            let luminance: [u32; 256] = std::array::from_fn(|i| {
                (data.r[i] as u64 + data.g[i] as u64 + data.b[i] as u64).div_ceil(3) as u32
            });
            let max = data
                .r
                .iter()
                .chain(data.g.iter())
                .chain(data.b.iter())
                .chain(luminance.iter())
                .copied()
                .max()
                .unwrap_or(1)
                .max(1) as f32;
            for (bins, color) in [
                (&data.r, Color::from_rgba(1.0, 0.25, 0.22, 0.56)),
                (&data.g, Color::from_rgba(0.25, 0.9, 0.42, 0.52)),
                (&data.b, Color::from_rgba(0.28, 0.55, 1.0, 0.52)),
                (&luminance, Color::from_rgba(0.92, 0.93, 0.95, 0.72)),
            ] {
                let path = canvas::Path::new(|builder| {
                    builder.move_to(Point::new(0.0, bounds.height));
                    for (i, count) in bins.iter().enumerate() {
                        let x = i as f32 / 255.0 * bounds.width;
                        let h = (*count as f32 + 1.0).ln() / (max + 1.0).ln() * bounds.height;
                        builder.line_to(Point::new(x, bounds.height - h));
                    }
                    builder.line_to(Point::new(bounds.width, bounds.height));
                    builder.close();
                });
                frame.fill(&path, color);
            }
        }
        for (i, name) in ["BLK", "SHD", "EXP", "HLT", "WHT"].iter().enumerate() {
            frame.fill_text(canvas::Text {
                content: (*name).into(),
                position: Point::new(
                    i as f32 * zone_width + zone_width / 2.0,
                    bounds.height - 3.0,
                ),
                color: Color::from_rgba(0.82, 0.83, 0.85, 0.75),
                size: 9.0.into(),
                horizontal_alignment: iced::alignment::Horizontal::Center,
                ..canvas::Text::default()
            });
        }
        for (index, color) in [
            Color::from_rgb(0.25, 0.55, 1.0),
            Color::from_rgb(1.0, 0.25, 0.18),
        ]
        .iter()
        .enumerate()
        {
            let x = if index == 0 { 3.0 } else { bounds.width - 13.0 };
            let triangle = canvas::Path::new(|p| {
                p.move_to(Point::new(x, 13.0));
                p.line_to(Point::new(x + 7.0, 1.0));
                p.line_to(Point::new(x + 14.0, 13.0));
                p.close();
            });
            let fill = if self.clipping[index] {
                *color
            } else {
                Color::from_rgba(color.r, color.g, color.b, 0.35)
            };
            frame.fill(&triangle, fill);
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging.is_some() {
            mouse::Interaction::Grabbing
        } else {
            mouse::Interaction::Grab
        }
    }
}
