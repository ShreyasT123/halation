use super::super::*;
use crate::widgets;
use crate::widgets::{MainViewport, Navigator};
use iced::{widget::column, ContentFit};

pub(in crate::app) fn develop_page(state: &State) -> Element<'_, Message> {
    let Some(index) = state.selected else {
        return super::library::library_page(state);
    };
    let item = &state.items[index];

    if state.loading_photo {
        return container(
            column![
                text("LOADING ASSET")
                    .size(12)
                    .color(Color::from_rgb8(76, 215, 246)),
                text(&item.filename).size(20),
                text("initializing…").size(12),
            ]
            .spacing(12)
            .align_x(Center),
        )
        .center(Length::Fill)
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    }

    let raw = state.raw_assets[index].as_ref();
    let meta = raw.map(|r| &r.metadata);
    let exif_stamp = meta
        .map(|m| {
            format!(
                "{} · {} · ISO {} · {} · {}",
                empty_dash(&m.camera),
                empty_dash(&m.lens),
                m.iso.map(|v| v.to_string()).unwrap_or_else(|| "—".into()),
                empty_dash(&m.shutter),
                empty_dash(&m.aperture)
            )
        })
        .unwrap_or_default();

    // ==========================================
    // COLUMN 1: LEFT UTILITY DECK (260px)
    // ==========================================
    let navigator: Element<'_, Message> = if let Some(handle) = &state.viewport {
        column![
            row![
                text("NAVIGATOR")
                    .size(11)
                    .color(Color::from_rgb8(76, 215, 246)),
                Space::with_width(Length::Fill),
                button(text("FIT").size(9))
                    .style(button::text)
                    .on_press(Message::SetZoom(ZoomMode::Fit)),
                button(text("1:1").size(9))
                    .style(button::text)
                    .on_press(Message::SetZoom(ZoomMode::Actual)),
                button(text("2:1").size(9))
                    .style(button::text)
                    .on_press(Message::SetZoom(ZoomMode::Two)),
            ]
            .align_y(Center),
            canvas::Canvas::new(Navigator {
                handle: handle.clone(),
                zoom: state.zoom,
                custom_zoom: state.custom_zoom,
                pan: state.pan
            })
            .width(Length::Fixed(236.0))
            .height(Length::Fixed(140.0)),
            text(format!(
                "{} × {} px",
                state.preview_dimensions.0, state.preview_dimensions.1
            ))
            .size(9)
            .color(Color::from_rgb8(150, 152, 156)),
        ]
        .spacing(6)
        .into()
    } else {
        column![text("NAVIGATOR").size(11)].into()
    };

    let history_panel: Element<'_, Message> = {
        let entries = state.catalog.history(item.id);
        let active = state.catalog.active_history_id(item.id);
        let mut list = column![row![
            text("HISTORY & BRANCHES")
                .size(11)
                .color(Color::from_rgb8(76, 215, 246)),
            Space::with_width(Length::Fill),
            text(format!("{} steps", entries.len()))
                .size(9)
                .color(Color::from_rgb8(150, 152, 156)),
        ]
        .align_y(Center)]
        .spacing(4);

        for entry in entries.into_iter().rev() {
            let is_active = Some(entry.id) == active;
            list = list.push(
                button(
                    row![
                        text(if is_active { "●" } else { "○" })
                            .size(10)
                            .color(if is_active {
                                Color::from_rgb8(76, 215, 246)
                            } else {
                                Color::from_rgb8(150, 152, 156)
                            }),
                        text(entry.label).size(10), // <-- Pass owned String by value
                    ]
                    .spacing(6)
                    .align_y(Center),
                )
                .style(if is_active {
                    button::secondary
                } else {
                    button::text
                })
                .width(Length::Fill)
                .on_press(Message::HistorySelect(entry.id)),
            );
        }
        list.into()
    };

    let culling_panel: Element<'_, Message> = {
        let metrics = state.culling_metrics[index];
        let status_str = metrics
            .map(|m| {
                if m.sharpness < state.sharpness_threshold
                    || m.clipped_highlights > state.clipping_threshold
                {
                    ("REVIEW REQUIRED", Color::from_rgb8(255, 180, 171))
                } else {
                    ("IN FOCUS · BALANCED", Color::from_rgb8(76, 215, 246))
                }
            })
            .unwrap_or(("PENDING METRICS", Color::from_rgb8(150, 152, 156)));

        column![
            text("CULLING ANALYSIS")
                .size(11)
                .color(Color::from_rgb8(76, 215, 246)),
            text(status_str.0).size(10).color(status_str.1),
            text(
                metrics
                    .map(|m| format!("Laplacian Focus: {:.4}", m.sharpness))
                    .unwrap_or_default()
            )
            .size(9)
            .font(Font::MONOSPACE),
            text(
                metrics
                    .map(|m| format!("Clipped Highlights: {:.2}%", m.clipped_highlights * 100.0))
                    .unwrap_or_default()
            )
            .size(9)
            .font(Font::MONOSPACE),
            text("Min Sharpness")
                .size(9)
                .color(Color::from_rgb8(150, 152, 156)),
            slider(
                0.0001_f32..=0.015_f32,
                state.sharpness_threshold,
                Message::SharpnessThreshold
            )
            .step(0.0001_f32),
            text("Max Highlight Clipping")
                .size(9)
                .color(Color::from_rgb8(150, 152, 156)),
            slider(
                0.0_f32..=0.20_f32,
                state.clipping_threshold,
                Message::ClippingThreshold
            )
            .step(0.005_f32),
        ]
        .spacing(4)
        .into()
    };

    let left_deck = container(scrollable(
        column![
            navigator,
            container(history_panel)
                .padding(8)
                .style(container::rounded_box),
            container(culling_panel)
                .padding(8)
                .style(container::rounded_box),
        ]
        .spacing(12),
    ))
    .width(Length::Fixed(260.0))
    .padding(10)
    .style(container::rounded_box);

    // ==========================================
    // COLUMN 2: CENTER STAGE & FILMSTRIP (FILL)
    // ==========================================
    let center_toolbar = row![
        button(text("Loupe").size(10))
            .style(if state.view_mode == ViewMode::Single {
                button::secondary
            } else {
                button::text
            })
            .on_press(Message::SetViewMode(ViewMode::Single)),
        button(text("Side Split").size(10))
            .style(if state.view_mode == ViewMode::SplitSideBySide {
                button::secondary
            } else {
                button::text
            })
            .on_press(Message::SetViewMode(ViewMode::SplitSideBySide)),
        button(text("Top Split").size(10))
            .style(if state.view_mode == ViewMode::SplitTopBottom {
                button::secondary
            } else {
                button::text
            })
            .on_press(Message::SetViewMode(ViewMode::SplitTopBottom)),
        Space::with_width(Length::Fixed(12.0)),
        button(
            text(if state.clipping[0] {
                "▲ Black Clip ON"
            } else {
                "▲ Black Clip"
            })
            .size(10)
        )
        .style(if state.clipping[0] {
            button::secondary
        } else {
            button::text
        })
        .on_press(Message::ToggleClipping(0)),
        button(
            text(if state.clipping[1] {
                "▼ White Clip ON"
            } else {
                "▼ White Clip"
            })
            .size(10)
        )
        .style(if state.clipping[1] {
            button::danger
        } else {
            button::text
        })
        .on_press(Message::ToggleClipping(1)),
        Space::with_width(Length::Fill),
        if state.view_mode != ViewMode::Single {
            row![
                text("Split").size(10),
                slider(0.1_f32..=0.9_f32, state.divider, Message::Divider)
                    .width(Length::Fixed(90.0)),
            ]
            .spacing(6)
            .align_y(Center)
        } else {
            row![]
        },
    ]
    .spacing(4)
    .align_y(Center);

    let viewport: Element<'_, Message> = if let Some(handle) = &state.viewport {
        match state.view_mode {
            ViewMode::Single => canvas::Canvas::new(MainViewport {
                handle: handle.clone(),
                dimensions: state.preview_dimensions,
                zoom: state.zoom,
                custom_zoom: state.custom_zoom,
                pan: state.pan,
                mask_overlay: state.mask_overlay.clone(),
                mask_tool: state.mask_tool,
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
            ViewMode::SplitSideBySide => {
                let original = state.original_viewport.as_ref().unwrap_or(handle);
                row![
                    container(
                        image::Viewer::new(original.clone())
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .content_fit(ContentFit::Contain)
                    )
                    .width(Length::FillPortion((state.divider * 100.0) as u16)),
                    container(
                        image::Viewer::new(handle.clone())
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .content_fit(ContentFit::Contain)
                    )
                    .width(Length::FillPortion(((1.0 - state.divider) * 100.0) as u16)),
                ]
                .height(Length::Fill)
                .into()
            }
            ViewMode::SplitTopBottom => {
                let original = state.original_viewport.as_ref().unwrap_or(handle);
                column![
                    container(
                        image::Viewer::new(original.clone())
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .content_fit(ContentFit::Contain)
                    )
                    .height(Length::FillPortion((state.divider * 100.0) as u16)),
                    container(
                        image::Viewer::new(handle.clone())
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .content_fit(ContentFit::Contain)
                    )
                    .height(Length::FillPortion(((1.0 - state.divider) * 100.0) as u16)),
                ]
                .width(Length::Fill)
                .into()
            }
        }
    } else {
        container(text("Select photo to begin develop").size(12))
            .center(Length::Fill)
            .into()
    };

    let filmstrip: Element<'_, Message> = {
        let mut row_items = row![].spacing(6).align_y(Center);
        for (i, it) in state.items.iter().enumerate() {
            let is_sel = state.selected == Some(i);
            let thumb: Element<'_, Message> = if let Some(h) = &state.thumbnails[i] {
                image(h.clone())
                    .content_fit(ContentFit::Cover)
                    .width(Length::Fixed(84.0))
                    .height(Length::Fixed(50.0))
                    .into()
            } else {
                container(
                    text("No Preview")
                        .size(8)
                        .color(Color::from_rgb8(150, 152, 156)),
                )
                .width(Length::Fixed(84.0))
                .height(Length::Fixed(50.0))
                .align_x(iced::alignment::Horizontal::Center)
                .align_y(iced::alignment::Vertical::Center)
                .into()
            };

            row_items = row_items.push(
                button(
                    column![
                        thumb,
                        row![
                            text(format!("#{}", i + 1))
                                .size(8)
                                .color(Color::from_rgb8(150, 152, 156)),
                            Space::with_width(Length::Fixed(8.0)),
                            text(format!("★{}", it.rating))
                                .size(8)
                                .color(Color::from_rgb8(255, 193, 116)),
                        ]
                        .width(Length::Fixed(84.0))
                        .align_y(Center),
                    ]
                    .spacing(2)
                    .align_x(Center),
                )
                .width(Length::Fixed(92.0))
                .height(Length::Fixed(72.0))
                .padding(3)
                .style(if is_sel {
                    button::secondary
                } else {
                    button::text
                })
                .on_press(Message::Select(i)),
            );
        }

        container(
            column![
                row![
                    text("FILMSTRIP DOCK")
                        .size(10)
                        .color(Color::from_rgb8(76, 215, 246)),
                    Space::with_width(Length::Fill),
                    text(format!("{} of {}", index + 1, state.items.len()))
                        .size(9)
                        .color(Color::from_rgb8(150, 152, 156)),
                ]
                .align_y(Center),
                scrollable::Scrollable::with_direction(
                    row_items.height(Length::Fixed(74.0)),
                    scrollable::Direction::Horizontal(scrollable::Scrollbar::default()),
                ),
            ]
            .spacing(4),
        )
        .padding(6)
        .width(Length::Fill)
        .height(Length::Fixed(106.0))
        .style(container::rounded_box)
        .into()
    };

    let center_canvas = column![
        center_toolbar,
        container(viewport)
            .width(Length::Fill)
            .height(Length::Fill)
            .center(Length::Fill)
            .style(container::rounded_box),
        text(exif_stamp)
            .size(10)
            .font(Font::MONOSPACE)
            .color(Color::from_rgb8(170, 172, 176)),
        filmstrip,
    ]
    .spacing(6);

    // ==========================================
    // COLUMN 3: RIGHT DEVELOP INSPECTOR (350px)
    // ==========================================
    let mut controls = column![row![
        text("DEVELOP INSPECTOR")
            .size(11)
            .color(Color::from_rgb8(76, 215, 246)),
        Space::with_width(Length::Fill),
        text(&item.filename).size(10).font(Font::MONOSPACE),
    ]
    .align_y(Center),]
    .spacing(8);

    controls = controls.push(
        histogram::view(state.histogram_data.clone(), state.clipping).map(|output| match output {
            histogram::Output::Adjust(zone, delta) => Message::HistogramAdjust(zone, delta),
            histogram::Output::ToggleClipping(idx) => Message::ToggleClipping(idx),
        }),
    );

    if let Some(m) = meta {
        controls = controls.push(
            row![
                container(
                    text(format!(
                        "ISO {}",
                        m.iso.map(|v| v.to_string()).unwrap_or_else(|| "—".into())
                    ))
                    .size(9)
                )
                .padding([2, 6])
                .style(container::rounded_box),
                container(text(empty_dash(&m.shutter)).size(9))
                    .padding([2, 6])
                    .style(container::rounded_box),
                container(text(empty_dash(&m.aperture)).size(9))
                    .padding([2, 6])
                    .style(container::rounded_box),
                container(text(empty_dash(&m.lens)).size(9))
                    .padding([2, 6])
                    .style(container::rounded_box),
            ]
            .spacing(4),
        );
    }

    controls = controls.push(
        column![
            text("AI MASKING (MOBILESAM)")
                .size(10)
                .color(Color::from_rgb8(76, 215, 246)),
            row![
                button(
                    text(if state.mask_tool {
                        "Cancel Point"
                    } else {
                        "Point Mask"
                    })
                    .size(10)
                )
                .style(if state.mask_tool {
                    button::secondary
                } else {
                    button::text
                })
                .on_press(Message::MaskTool(!state.mask_tool)),
                button(text("Sky").size(10))
                    .style(button::text)
                    .on_press(Message::MaskPoint([0.5, 0.1])),
                button(text("Subject").size(10))
                    .style(button::text)
                    .on_press(Message::MaskPoint([0.5, 0.5])),
                button(text("Clear").size(10))
                    .style(button::text)
                    .on_press(Message::ClearMask),
            ]
            .spacing(4),
        ]
        .spacing(4),
    );

    controls = controls.push(
        column![
            row![
                text("TONE CURVE")
                    .size(10)
                    .color(Color::from_rgb8(76, 215, 246)),
                Space::with_width(Length::Fill),
                button(text("RGB").size(9))
                    .style(if state.curve_channel == 0 {
                        button::secondary
                    } else {
                        button::text
                    })
                    .on_press(Message::CurveChannel(0)),
                button(text("R").size(9))
                    .style(if state.curve_channel == 1 {
                        button::secondary
                    } else {
                        button::text
                    })
                    .on_press(Message::CurveChannel(1)),
                button(text("G").size(9))
                    .style(if state.curve_channel == 2 {
                        button::secondary
                    } else {
                        button::text
                    })
                    .on_press(Message::CurveChannel(2)),
                button(text("B").size(9))
                    .style(if state.curve_channel == 3 {
                        button::secondary
                    } else {
                        button::text
                    })
                    .on_press(Message::CurveChannel(3)),
            ]
            .spacing(2)
            .align_y(Center),
            widgets::tone_curve::view(state.curve_points[state.curve_channel].clone())
                .map(|p| Message::CurvePoint(p.0, p.1)),
        ]
        .spacing(4),
    );

    controls = controls.push(
        column![
            text("3-WAY COLOR GRADING")
                .size(10)
                .color(Color::from_rgb8(76, 215, 246)),
            row![
                column![
                    text("Shadows").size(9),
                    widgets::color_wheel::view(state.wheel_values[0].0, state.wheel_values[0].1)
                        .map(|v| Message::Wheel(0, v.0, v.1)),
                    slider(-1.0..=1.0, state.wheel_values[0].2, |v| Message::WheelLuma(
                        0, v
                    ))
                    .step(0.02_f32),
                ]
                .align_x(Center)
                .spacing(3),
                column![
                    text("Midtones").size(9),
                    widgets::color_wheel::view(state.wheel_values[1].0, state.wheel_values[1].1)
                        .map(|v| Message::Wheel(1, v.0, v.1)),
                    slider(-1.0..=1.0, state.wheel_values[1].2, |v| Message::WheelLuma(
                        1, v
                    ))
                    .step(0.02_f32),
                ]
                .align_x(Center)
                .spacing(3),
                column![
                    text("Highlights").size(9),
                    widgets::color_wheel::view(state.wheel_values[2].0, state.wheel_values[2].1)
                        .map(|v| Message::Wheel(2, v.0, v.1)),
                    slider(-1.0..=1.0, state.wheel_values[2].2, |v| Message::WheelLuma(
                        2, v
                    ))
                    .step(0.02_f32),
                ]
                .align_x(Center)
                .spacing(3),
            ]
            .spacing(8),
        ]
        .spacing(4),
    );

    controls = controls.push(
        column![
            text("BASIC CALIBRATION")
                .size(10)
                .color(Color::from_rgb8(76, 215, 246)),
            control(
                "Temperature",
                state.params.temperature,
                2000.0..=50000.0,
                25.0,
                Message::Temperature
            ),
            control(
                "Tint",
                state.params.tint,
                -150.0..=150.0,
                1.0,
                Message::Tint
            ),
        ]
        .spacing(2),
    );

    controls = controls.push(
        column![
            text("TONE DISTRIBUTION")
                .size(10)
                .color(Color::from_rgb8(76, 215, 246)),
            control(
                "Exposure",
                state.params.exposure,
                -5.0..=5.0,
                0.05,
                Message::Exposure
            ),
            control(
                "Contrast",
                state.params.contrast,
                -100.0..=100.0,
                1.0,
                Message::Contrast
            ),
            control(
                "Highlights",
                state.params.highlights,
                -100.0..=100.0,
                1.0,
                Message::Highlights
            ),
            control(
                "Shadows",
                state.params.shadows,
                -100.0..=100.0,
                1.0,
                Message::Shadows
            ),
            control(
                "Whites",
                state.params.whites,
                -100.0..=100.0,
                1.0,
                Message::Whites
            ),
            control(
                "Blacks",
                state.params.blacks,
                -100.0..=100.0,
                1.0,
                Message::Blacks
            ),
        ]
        .spacing(2),
    );

    controls = controls.push(
        row![
            text("Rating:").size(10),
            button(text("1").size(9))
                .style(button::text)
                .on_press(Message::Rating(1)),
            button(text("2").size(9))
                .style(button::text)
                .on_press(Message::Rating(2)),
            button(text("3").size(9))
                .style(button::text)
                .on_press(Message::Rating(3)),
            button(text("4").size(9))
                .style(button::text)
                .on_press(Message::Rating(4)),
            button(text("5").size(9))
                .style(button::text)
                .on_press(Message::Rating(5)),
            Space::with_width(Length::Fill),
            button(text("Pick").size(9))
                .style(button::text)
                .on_press(Message::Flag("Pick")),
            button(text("Reject").size(9))
                .style(button::text)
                .on_press(Message::Flag("Reject")),
        ]
        .spacing(2)
        .align_y(Center),
    );

    controls = controls.push(
        row![
            button(text("Reset").size(11))
                .style(button::text)
                .on_press(Message::Reset),
            Space::with_width(Length::Fill),
            button(text("Save Edits").size(11))
                .style(button::primary)
                .on_press(Message::Save),
        ]
        .align_y(Center),
    );

    let right_inspector = container(scrollable(controls.spacing(10)))
        .width(Length::Fixed(350.0))
        .padding(12)
        .style(container::rounded_box);

    row![left_deck, center_canvas, right_inspector]
        .spacing(6)
        .height(Length::Fill)
        .into()
}
