use iced::{
    mouse,
    widget::canvas::{self, Frame, Geometry, Program},
    Color, Length, Point, Theme,
};

#[derive(Debug, Clone, Copy)]
pub struct Changed(pub f32, pub f32);
pub fn view<'a>(hue: f32, saturation: f32) -> iced::Element<'a, Changed> {
    canvas::Canvas::new(Wheel { hue, saturation })
        .width(Length::Fixed(112.0))
        .height(Length::Fixed(112.0))
        .into()
}
struct Wheel {
    hue: f32,
    saturation: f32,
}
impl Program<Changed> for Wheel {
    type State = bool;
    fn update(
        &self,
        s: &mut bool,
        e: canvas::Event,
        b: iced::Rectangle,
        c: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Changed>) {
        match e {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                *s = true;
                Self::get(b, c)
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) if *s => {
                let cur = mouse::Cursor::Available(position);
                Self::get(b, cur)
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                *s = false;
                (canvas::event::Status::Captured, None)
            }
            _ => (canvas::event::Status::Ignored, None),
        }
    }
    fn draw(
        &self,
        _: &bool,
        r: &iced::Renderer,
        _: &Theme,
        b: iced::Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut f = Frame::new(r, b.size());
        let c = Point::new(b.width / 2.0, b.height / 2.0);
        let radius = b.width.min(b.height) * 0.47;
        for ring in 0..6 {
            let saturation = (ring as f32 + 0.5) / 6.0;
            let rr = radius * saturation;
            for i in 0..32 {
                let a = i as f32 * std::f32::consts::TAU / 32.0;
                let hue = hue_color(a / std::f32::consts::TAU);
                let color = Color::from_rgb(
                    1.0 + (hue.r - 1.0) * saturation,
                    1.0 + (hue.g - 1.0) * saturation,
                    1.0 + (hue.b - 1.0) * saturation,
                );
                let p = canvas::Path::new(|x| {
                    x.arc(canvas::path::Arc {
                        center: c,
                        radius: rr,
                        start_angle: iced::Radians(a),
                        end_angle: iced::Radians(a + std::f32::consts::TAU / 32.0 + 0.03),
                    })
                });
                f.stroke(
                    &p,
                    canvas::Stroke::default()
                        .with_color(color)
                        .with_width(radius / 6.0 + 0.4),
                );
            }
        }
        let a = self.hue * std::f32::consts::TAU;
        let puck = Point::new(
            c.x + self.saturation * radius * a.cos(),
            c.y + self.saturation * radius * a.sin(),
        );
        f.fill(&canvas::Path::circle(puck, 5.0), Color::WHITE);
        vec![f.into_geometry()]
    }
}
impl Wheel {
    fn get(b: iced::Rectangle, c: mouse::Cursor) -> (canvas::event::Status, Option<Changed>) {
        let Some(p) = c.position_in(b) else {
            return (canvas::event::Status::Ignored, None);
        };
        let dx = p.x - b.width / 2.0;
        let dy = p.y - b.height / 2.0;
        let sat = (dx.hypot(dy) / (b.width.min(b.height) * 0.47)).clamp(0.0, 1.0);
        let hue = dy.atan2(dx).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
        (canvas::event::Status::Captured, Some(Changed(hue, sat)))
    }
}
fn hue_color(h: f32) -> Color {
    let x = (h * 6.0).fract();
    let sector = (h * 6.0).floor() as u8;
    let q = 1.0 - x;
    match sector {
        0 => Color::from_rgb(1.0, x, 0.0),
        1 => Color::from_rgb(q, 1.0, 0.0),
        2 => Color::from_rgb(0.0, 1.0, x),
        3 => Color::from_rgb(0.0, q, 1.0),
        4 => Color::from_rgb(x, 0.0, 1.0),
        _ => Color::from_rgb(1.0, 0.0, q),
    }
}
