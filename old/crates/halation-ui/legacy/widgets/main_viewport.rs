use super::CanvasInteraction;
use crate::app::{Message, ZoomMode};
use iced::{mouse, widget::canvas, Color, Point, Rectangle, Size, Theme};

pub(crate) struct MainViewport {
    pub(crate) handle: iced::widget::image::Handle,
    pub(crate) dimensions: (u32, u32),
    pub(crate) zoom: ZoomMode,
    pub(crate) custom_zoom: f32,
    pub(crate) pan: [f32; 2],
    pub(crate) mask_overlay: Option<iced::widget::image::Handle>,
    pub(crate) mask_tool: bool,
}

impl MainViewport {
    fn display_size(&self, bounds: Size) -> Size {
        let (width, height) = self.dimensions;
        if width == 0 || height == 0 {
            return bounds;
        }
        let fit = (bounds.width / width as f32).min(bounds.height / height as f32);
        let fill = (bounds.width / width as f32).max(bounds.height / height as f32);
        let scale = match self.zoom {
            ZoomMode::Fit => fit,
            ZoomMode::Fill => fill,
            ZoomMode::Actual => 1.0,
            ZoomMode::Two => 2.0,
            ZoomMode::Four => 4.0,
            ZoomMode::Custom => fit * self.custom_zoom,
        };
        Size::new(width as f32 * scale, height as f32 * scale)
    }
}

impl canvas::Program<Message> for MainViewport {
    type State = CanvasInteraction;

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(
                button @ (mouse::Button::Left | mouse::Button::Middle),
            )) => {
                if self.mask_tool {
                    if button == mouse::Button::Left {
                        if let Some(position) = cursor.position_in(bounds) {
                            let size = self.display_size(bounds.size());
                            let x = if size.width > bounds.width {
                                -(size.width - bounds.width) * self.pan[0]
                            } else {
                                (bounds.width - size.width) / 2.0
                            };
                            let y = if size.height > bounds.height {
                                -(size.height - bounds.height) * self.pan[1]
                            } else {
                                (bounds.height - size.height) / 2.0
                            };
                            if position.x >= x
                                && position.x <= x + size.width
                                && position.y >= y
                                && position.y <= y + size.height
                            {
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::MaskPoint([
                                        (position.x - x) / size.width.max(1.0),
                                        (position.y - y) / size.height.max(1.0),
                                    ])),
                                );
                            }
                        }
                    }
                    return (canvas::event::Status::Ignored, None);
                }
                state.dragging = true;
                state.last = cursor.position();
                (canvas::event::Status::Captured, None)
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(
                mouse::Button::Left | mouse::Button::Middle,
            )) => {
                state.dragging = false;
                state.last = None;
                (canvas::event::Status::Captured, None)
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) if state.dragging => {
                let dimensions = self.display_size(bounds.size());
                let hidden_x = (dimensions.width - bounds.width).max(1.0);
                let hidden_y = (dimensions.height - bounds.height).max(1.0);
                let previous = state.last.replace(position).unwrap_or(position);
                let delta = position - previous;
                (
                    canvas::event::Status::Captured,
                    Some(Message::PanBy(delta.x / hidden_x, delta.y / hidden_y)),
                )
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta })
                if cursor.position_in(bounds).is_some() =>
            {
                let y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y,
                    mouse::ScrollDelta::Pixels { y, .. } => y / 100.0,
                };
                (
                    canvas::event::Status::Captured,
                    Some(Message::WheelZoom(y.signum())),
                )
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
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), Color::from_rgb8(35, 36, 39));

        let size = self.display_size(bounds.size());
        let x = if size.width > bounds.width {
            -(size.width - bounds.width) * self.pan[0]
        } else {
            (bounds.width - size.width) / 2.0
        };
        let y = if size.height > bounds.height {
            -(size.height - bounds.height) * self.pan[1]
        } else {
            (bounds.height - size.height) / 2.0
        };
        let image_bounds = Rectangle {
            x,
            y,
            width: size.width,
            height: size.height,
        };
        frame.with_clip(
            Rectangle {
                x: 0.0,
                y: 0.0,
                width: bounds.width,
                height: bounds.height,
            },
            |clipped| {
                clipped.draw_image(image_bounds, &self.handle);
                if let Some(mask) = &self.mask_overlay {
                    clipped.draw_image(image_bounds, mask);
                }
            },
        );
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging {
            mouse::Interaction::Grabbing
        } else if self.mask_tool {
            mouse::Interaction::Crosshair
        } else if {
            let size = self.display_size(bounds.size());
            size.width > bounds.width || size.height > bounds.height
        } {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::Idle
        }
    }
}
