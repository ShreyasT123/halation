use super::*;
use iced::widget::column;

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let brand = row![
        text("HALATION").size(16).font(Font::with_name("Segoe UI")),
        container(
            text("STUDIO")
                .size(10)
                .color(Color::from_rgb8(76, 215, 246))
        )
        .padding([2, 5])
        .style(container::rounded_box),
    ]
    .spacing(6)
    .align_y(Center);

    let nav_module = row![
        button(text("Library").size(12))
            .style(if state.page == Page::Library {
                button::primary
            } else {
                button::text
            })
            .on_press(Message::Navigate(Page::Library)),
        button(text("Develop").size(12))
            .style(if state.page == Page::Develop {
                button::primary
            } else {
                button::text
            })
            .on_press(Message::Navigate(Page::Develop)),
    ]
    .spacing(4)
    .align_y(Center);

    let loupe_quick = row![
        button(text("FIT").size(10))
            .style(if state.zoom == ZoomMode::Fit {
                button::secondary
            } else {
                button::text
            })
            .on_press(Message::SetZoom(ZoomMode::Fit)),
        button(text("100%").size(10))
            .style(if state.zoom == ZoomMode::Actual {
                button::secondary
            } else {
                button::text
            })
            .on_press(Message::SetZoom(ZoomMode::Actual)),
        button(text("200%").size(10))
            .style(if state.zoom == ZoomMode::Two {
                button::secondary
            } else {
                button::text
            })
            .on_press(Message::SetZoom(ZoomMode::Two)),
    ]
    .spacing(2)
    .align_y(Center);

    let history_actions = row![
        button(text("Undo").size(11))
            .style(button::text)
            .on_press(Message::HistoryStep(false)),
        button(text("Redo").size(11))
            .style(button::text)
            .on_press(Message::HistoryStep(true)),
        button(
            text(if state.view_mode == ViewMode::Single {
                "Compare"
            } else {
                "Loupe"
            })
            .size(11)
        )
        .style(if state.view_mode != ViewMode::Single {
            button::secondary
        } else {
            button::text
        })
        .on_press(Message::ToggleComparison),
    ]
    .spacing(4)
    .align_y(Center);

    let export_actions = row![
        button(text("JPEG").size(10))
            .style(button::text)
            .on_press(Message::ExportAll(ExportFormat::Jpeg)),
        button(text("PNG").size(10))
            .style(button::text)
            .on_press(Message::ExportAll(ExportFormat::Png)),
        button(text("WebP").size(10))
            .style(button::text)
            .on_press(Message::ExportAll(ExportFormat::WebP)),
        button(text("TIFF").size(10))
            .style(button::text)
            .on_press(Message::ExportAll(ExportFormat::Tiff)),
    ]
    .spacing(3)
    .align_y(Center);

    let header = container(
        row![
            brand,
            Space::with_width(Length::Fixed(16.0)),
            nav_module,
            Space::with_width(Length::Fixed(16.0)),
            loupe_quick,
            Space::with_width(Length::Fixed(12.0)),
            history_actions,
            Space::with_width(Length::Fill),
            export_actions,
            Space::with_width(Length::Fixed(12.0)),
            button(text("Import Photos").size(11))
                .style(button::primary)
                .on_press(Message::Import),
        ]
        .align_y(Center),
    )
    .padding([6, 14])
    .width(Length::Fill)
    .style(container::rounded_box);

    let content: Element<'_, Message> = match state.page {
        Page::Library => pages::library::library_page(state),
        Page::Develop => pages::develop::develop_page(state),
    };

    let footer = container(
        row![
            text(format!("{} photos indexed", state.items.len()))
                .size(10)
                .font(Font::MONOSPACE),
            Space::with_width(Length::Fixed(12.0)),
            text(&state.status)
                .size(10)
                .font(Font::MONOSPACE)
                .color(Color::from_rgb8(76, 215, 246)),
            Space::with_width(Length::Fill),
            text(format!(
                "{}  ·  {}",
                state.gpu.adapter_name, state.gpu.display_profile
            ))
            .size(10)
            .color(Color::from_rgb8(150, 152, 156)),
        ]
        .align_y(Center),
    )
    .padding([5, 14])
    .width(Length::Fill)
    .style(container::rounded_box);

    column![header, content, footer].spacing(4).into()
}

pub(super) fn subscription(_state: &State) -> Subscription<Message> {
    let events = event::listen_with(|event, status, _window| {
        if status == event::Status::Captured {
            return None;
        }
        match event {
            Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => match key.as_ref() {
                keyboard::Key::Named(keyboard::key::Named::ArrowUp) => {
                    Some(Message::KeyPressed("ArrowUp".into()))
                }
                keyboard::Key::Named(keyboard::key::Named::ArrowDown) => {
                    Some(Message::KeyPressed("ArrowDown".into()))
                }
                keyboard::Key::Named(keyboard::key::Named::ArrowLeft) => {
                    Some(Message::KeyPressed("ArrowLeft".into()))
                }
                keyboard::Key::Named(keyboard::key::Named::ArrowRight) => {
                    Some(Message::KeyPressed("ArrowRight".into()))
                }
                keyboard::Key::Character(value)
                    if [
                        "z", "Z", "j", "J", "\\", "1", "2", "3", "4", "5", "6", "7", "8", "9", "x",
                        "X", "u", "U",
                    ]
                    .contains(&value) =>
                {
                    Some(Message::KeyPressed(value.to_owned()))
                }
                _ => None,
            },
            Event::Window(iced::window::Event::Moved(_)) => Some(Message::RefreshDisplay),
            _ => None,
        }
    });
    Subscription::batch([
        events,
        iced::time::every(std::time::Duration::from_secs(3)).map(|_| Message::RefreshDisplay),
    ])
}
