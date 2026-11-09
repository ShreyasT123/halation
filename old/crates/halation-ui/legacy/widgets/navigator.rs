use super::CanvasInteraction;
use crate::app::{Message, ZoomMode};
use iced::{mouse, widget::canvas, Color, Point, Rectangle, Size, Theme};

pub(crate) struct Navigator {
    pub(crate) handle: iced::widget::image::Handle,
    pub(crate) zoom: ZoomMode,
    pub(crate) custom_zoom: f32,
    pub(crate) pan: [f32; 2],
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
