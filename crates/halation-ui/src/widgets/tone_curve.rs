use iced::{
    mouse,
    widget::canvas::{self, Frame, Geometry, Program},
    Color, Length, Point, Rectangle, Size, Theme,
};

#[derive(Debug, Clone, Copy)]
pub struct PointChanged(pub usize, pub [f32; 2]);

pub fn view<'a>(points: Vec<[f32; 2]>) -> iced::Element<'a, PointChanged> {
    canvas::Canvas::new(Curve { points })
        .width(Length::Fill)
        .height(Length::Fixed(150.0))
        .into()
}

struct Curve {
    points: Vec<[f32; 2]>,
}
impl Program<PointChanged> for Curve {
    type State = Option<usize>;
    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<PointChanged>) {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(p) = cursor.position_in(bounds) else {
                    return (canvas::event::Status::Ignored, None);
                };
                let nearest = self.points.iter().enumerate().min_by(|(_, a), (_, b)| {
                    let da = (p.x - a[0] * bounds.width).powi(2)
                        + (p.y - (1.0 - a[1]) * bounds.height).powi(2);
                    let db = (p.x - b[0] * bounds.width).powi(2)
                        + (p.y - (1.0 - b[1]) * bounds.height).powi(2);
                    da.total_cmp(&db)
                });
                let idx = nearest
                    .filter(|(_, pt)| {
                        ((p.x - pt[0] * bounds.width).powi(2)
                            + (p.y - (1.0 - pt[1]) * bounds.height).powi(2))
                            < 400.0
                    })
                    .map(|(i, _)| i);
                *state = idx;
                let idx = idx.unwrap_or_else(|| {
                    let x = p.x / bounds.width;
                    self.points
                        .iter()
                        .position(|pt| pt[0] > x)
                        .unwrap_or(self.points.len().saturating_sub(1))
                });
                (
                    canvas::event::Status::Captured,
                    Some(PointChanged(
                        idx,
                        [p.x / bounds.width, 1.0 - p.y / bounds.height],
                    )),
                )
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) if state.is_some() => {
                let i = state.unwrap();
                let x = ((position.x - bounds.x) / bounds.width).clamp(0.0, 1.0);
                let y = (1.0 - (position.y - bounds.y) / bounds.height).clamp(0.0, 1.0);
                (
                    canvas::event::Status::Captured,
                    Some(PointChanged(i, [x, y])),
                )
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                *state = None;
                (canvas::event::Status::Captured, None)
            }
            _ => (canvas::event::Status::Ignored, None),
        }
    }
    fn draw(
        &self,
        _: &Self::State,
        r: &iced::Renderer,
        _: &Theme,
        b: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut f = Frame::new(r, b.size());
        f.fill_rectangle(Point::ORIGIN, b.size(), Color::from_rgb(0.055, 0.06, 0.07));
        for n in 1..4 {
            let p = n as f32 / 4.0;
            f.fill_rectangle(
                Point::new(p * b.width, 0.0),
                Size::new(1.0, b.height),
                Color::from_rgba(0.7, 0.72, 0.75, 0.2),
            );
            f.fill_rectangle(
                Point::new(0.0, p * b.height),
                Size::new(b.width, 1.0),
                Color::from_rgba(0.7, 0.72, 0.75, 0.2),
            );
        }
        let mut pts = self.points.clone();
        pts.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let path = canvas::Path::new(|p| {
            if let Some(first) = pts.first() {
                p.move_to(Point::new(first[0] * b.width, (1.0 - first[1]) * b.height));
                for step in 1..=128 {
                    let x = step as f32 / 128.0;
                    p.line_to(Point::new(
                        x * b.width,
                        (1.0 - monotone_sample(x, &pts)) * b.height,
                    ));
                }
            }
        });
        f.stroke(
            &path,
            canvas::Stroke::default()
                .with_color(Color::from_rgb(0.95, 0.76, 0.42))
                .with_width(2.0),
        );
        for pt in pts {
            f.fill(
                &canvas::Path::circle(Point::new(pt[0] * b.width, (1.0 - pt[1]) * b.height), 4.0),
                Color::WHITE,
            );
        }
        vec![f.into_geometry()]
    }
}

fn monotone_sample(x: f32, points: &[[f32; 2]]) -> f32 {
    if points.len() < 2 {
        return x;
    }
    let n = points.len();
    let slopes: Vec<f32> = points
        .windows(2)
        .map(|p| (p[1][1] - p[0][1]) / (p[1][0] - p[0][0]).max(1e-5))
        .collect();
    let mut tangent = vec![0.0; n];
    tangent[0] = slopes[0];
    tangent[n - 1] = slopes[n - 2];
    for i in 1..n - 1 {
        tangent[i] = if slopes[i - 1] * slopes[i] <= 0.0 {
            0.0
        } else {
            2.0 * slopes[i - 1] * slopes[i] / (slopes[i - 1] + slopes[i])
        };
    }
    let i = points
        .windows(2)
        .position(|p| x <= p[1][0])
        .unwrap_or(n - 2);
    let (x0, y0) = (points[i][0], points[i][1]);
    let (x1, y1) = (points[i + 1][0], points[i + 1][1]);
    let h = (x1 - x0).max(1e-5);
    let t = ((x - x0) / h).clamp(0.0, 1.0);
    let (t2, t3) = (t * t, t * t * t);
    ((2.0 * t3 - 3.0 * t2 + 1.0) * y0
        + (t3 - 2.0 * t2 + t) * h * tangent[i]
        + (-2.0 * t3 + 3.0 * t2) * y1
        + (t3 - t2) * h * tangent[i + 1])
        .clamp(0.0, 1.0)
}
