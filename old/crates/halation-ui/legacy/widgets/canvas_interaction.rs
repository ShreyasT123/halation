use iced::Point;

#[derive(Default)]
pub(crate) struct CanvasInteraction {
    pub(crate) dragging: bool,
    pub(crate) last: Option<Point>,
}
