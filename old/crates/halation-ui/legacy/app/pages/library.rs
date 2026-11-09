use super::super::*;
use iced::{widget::column, ContentFit};

pub(in crate::app) fn library_page(state: &State) -> Element<'_, Message> {
    const ROW_HEIGHT: f32 = 190.0;
    const HEADER_HEIGHT: f32 = 58.0;
    const GAP: f32 = 12.0;
    let columns = catalog_columns(state.catalog_view_width);
    let card_width = ((state.catalog_view_width - 40.0 - GAP * (columns - 1) as f32)
        / columns as f32)
        .max(160.0);
    let mut grid = column![row![
        column![
            text("PHOTO LIBRARY").size(13),
            text("Browse, rate, and organize your originals")
                .size(11)
                .color(Color::from_rgb8(150, 152, 156)),
        ]
        .spacing(5),
        Space::with_width(Length::Fill),
        text(if state.filter_culling {
            format!(
                "{} SELECTS / {} PHOTOS",
                visible_catalog_indices(state).len(),
                state.items.len()
            )
        } else {
            format!("{} PHOTOS", state.items.len())
        })
        .size(10)
        .color(Color::from_rgb8(150, 152, 156)),
        button(
            text(if state.filter_culling {
                "SHOW ALL"
            } else {
                "FILTER SELECTS"
            })
            .size(9)
        )
        .style(if state.filter_culling {
            button::secondary
        } else {
            button::text
        })
        .on_press(Message::ToggleCullingFilter),
    ]
    .align_y(Center),]
    .spacing(14);

    if state.items.is_empty() {
        grid = grid
            .push(Space::with_height(Length::Fixed(60.0)))
            .push(text("Catalog Empty").size(22))
            .push(
                text(
                    "Import raw files (DNG, ARW, CR2, NEF) or standard formats (JPEG, TIFF, PNG).",
                )
                .size(12),
            )
            .push(button(text("Import Master Files")).on_press(Message::Import));
    } else {
        let visible_items = visible_catalog_indices(state);
        if visible_items.is_empty() {
            grid = grid
                .push(Space::with_height(Length::Fixed(60.0)))
                .push(text("No photos meet the current culling thresholds").size(18))
                .push(button(text("Show All Photos")).on_press(Message::ToggleCullingFilter));
        }
        let row_count = visible_items.len().div_ceil(columns);
        let first_row = if row_count == 0 {
            0
        } else {
            ((((state.catalog_scroll_y - HEADER_HEIGHT).max(0.0) / ROW_HEIGHT) as usize)
                .saturating_sub(2))
            .min(row_count - 1)
        };
        let visible_rows = (state.catalog_view_height / ROW_HEIGHT).ceil() as usize + 4;
        let end_row = (first_row + visible_rows).min(row_count);
        if first_row > 0 {
            grid = grid.push(Space::with_height(Length::Fixed(
                first_row as f32 * ROW_HEIGHT,
            )));
        }
        for row_index in first_row..end_row {
            let mut cards = row![].spacing(GAP).height(Length::Fixed(ROW_HEIGHT));
            for idx in visible_items
                .iter()
                .skip(row_index * columns)
                .take(columns)
                .copied()
            {
                let item = &state.items[idx];
                let is_selected = state.selected == Some(idx);
                let preview: Element<'_, Message> = if let Some(handle) = &state.thumbnails[idx] {
                    image(handle.clone())
                        .content_fit(ContentFit::Contain)
                        .width(Length::Fill)
                        .height(Length::Fixed(126.0))
                        .into()
                } else {
                    container(
                        text("Loading preview")
                            .size(10)
                            .color(Color::from_rgb8(150, 152, 156)),
                    )
                    .width(Length::Fill)
                    .height(Length::Fixed(126.0))
                    .align_x(iced::alignment::Horizontal::Center)
                    .align_y(iced::alignment::Vertical::Center)
                    .into()
                };
                let flag_color = match item.flag.as_str() {
                    "Pick" => Color::from_rgb8(76, 215, 246),
                    "Reject" => Color::from_rgb8(255, 180, 171),
                    _ => Color::from_rgb8(150, 152, 156),
                };
                let format = item
                    .path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or("IMG")
                    .to_ascii_uppercase();
                let format = match format.as_str() {
                    "JPG" | "JPEG" => "JPG",
                    "TIF" | "TIFF" => "TIFF",
                    "DNG" => "DNG",
                    "ARW" | "CR2" | "CR3" | "NEF" | "RAF" | "RW2" | "ORF" => "RAW",
                    _ => "IMG",
                };
                let culling_label = state.culling_metrics[idx].map(|metrics| {
                    if metrics.sharpness < state.sharpness_threshold
                        || metrics.clipped_highlights > state.clipping_threshold
                    {
                        ("REVIEW", Color::from_rgb8(255, 180, 171))
                    } else {
                        ("SELECT", Color::from_rgb8(76, 215, 246))
                    }
                });
                let card = button(
                    column![
                        preview,
                        text(&item.filename).size(11).font(Font::MONOSPACE),
                        row![
                            text(format!("* {}", item.rating)).size(10),
                            text(format).size(9).color(Color::from_rgb8(76, 215, 246)),
                            text(&item.flag).size(10).color(flag_color),
                            text(&item.color_label).size(10),
                            if let Some((label, color)) = culling_label {
                                text(label).size(9).color(color)
                            } else {
                                text("").size(9)
                            },
                            Space::with_width(Length::Fill),
                            text(format!("{:.1} MB", item.file_size as f32 / 1_000_000.0))
                                .size(9)
                                .color(Color::from_rgb8(150, 152, 156)),
                        ]
                        .spacing(8)
                        .align_y(Center),
                    ]
                    .spacing(7)
                    .padding(8)
                    .width(Length::Fill),
                )
                .style(if is_selected {
                    button::secondary
                } else {
                    button::text
                })
                .width(Length::Fixed(card_width))
                .height(Length::Fill)
                .on_press(Message::Select(idx));
                cards = cards.push(card);
            }
            grid = grid.push(cards);
        }
        if end_row < row_count {
            grid = grid.push(Space::with_height(Length::Fixed(
                (row_count - end_row) as f32 * ROW_HEIGHT,
            )));
        }
    }

    container(
        scrollable(grid.spacing(0))
            .id(state.catalog_scroll_id.clone())
            .on_scroll(|viewport| {
                let offset = viewport.absolute_offset();
                let bounds = viewport.bounds();
                Message::CatalogScrolled(offset.y, bounds.height, bounds.width)
            }),
    )
    .padding(18)
    .width(Length::Fill)
    .height(Length::Fill)
    .style(container::rounded_box)
    .into()
}
