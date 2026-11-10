//! Right-panel tools beyond the slider accordion: tone curve, colour grading wheels, local masks and
//! export / external editor. All views return owned (`'static`) elements built from `&State`.

use super::{action_button, button_style, fa, h_rule, label, mono, row_style, section_header_plain, State, ACCENT, PICK, TEXT_DIM, TEXT_PRIMARY, TEXT_SECONDARY};
use crate::{
    ext::{ExportScope, ExtMessage, FormatChoice, GradientKind, LocalField, MixerChannel, Tool, VisionState},
    widgets::{CurveEditor, WheelId, WheelView},
    Message,
};
use halation_color::OutputSpace;
use halation_core::{AdjustId, AiMaskKind, CurveChannel, BAND_NAMES};
use iced::{
    alignment::Vertical,
    widget::{button, column, container, progress_bar, row, slider, vertical_slider, Column, Row, Space},
    Element, Length,
};

fn ext(m: ExtMessage) -> Message {
    Message::Ext(m)
}

fn chip_button(name: &'static str, active: bool, msg: Option<Message>) -> Element<'static, Message> {
    button(label(name, 11, TEXT_PRIMARY)).padding([3, 9]).style(button_style(active, ACCENT)).on_press_maybe(msg).into()
}

fn panel(body: impl Into<Element<'static, Message>>) -> Element<'static, Message> {
    container(body).padding(12).width(Length::Fill).into()
}

fn adjust_row(state: &State, id: AdjustId) -> Element<'static, Message> {
    let value = id.get(&state.params);
    let modified = !id.is_default(&state.params);
    let reset = button(fa("rotate-left", 9.0, TEXT_DIM))
        .padding([2, 4])
        .style(row_style(false))
        .on_press_maybe(modified.then_some(ext(ExtMessage::ResetAdjust(id))));
    row![
        container(label(id.label(), 11, TEXT_SECONDARY)).width(Length::Fixed(70.0)),
        slider(id.min()..=id.max(), value, move |v| Message::Adjust(id, v))
            .step(1.0_f32)
            .on_release(Message::CommitEdit)
            .width(Length::Fill),
        container(mono(id.format_value(value), 11, TEXT_SECONDARY)).width(Length::Fixed(44.0)),
        reset,
    ]
    .spacing(6)
    .align_y(Vertical::Center)
    .into()
}

fn adjust_group(state: &State, title: &'static str, ids: &[AdjustId]) -> Element<'static, Message> {
    let mut body = Column::new().spacing(6).push(section_header_plain(title));
    for id in ids {
        body = body.push(adjust_row(state, *id));
    }
    body.into()
}

fn value_row(name: &'static str, value: f32, lo: f32, hi: f32, on_change: impl Fn(f32) -> Message + 'static) -> Element<'static, Message> {
    row![
        container(label(name, 11, TEXT_SECONDARY)).width(Length::Fixed(70.0)),
        slider(lo..=hi, value, on_change).step(1.0_f32).on_release(Message::CommitEdit).width(Length::Fill),
        container(mono(format!("{value:.0}"), 11, TEXT_SECONDARY)).width(Length::Fixed(44.0)),
        Space::with_width(Length::Fixed(19.0)),
    ]
    .spacing(6)
    .align_y(Vertical::Center)
    .into()
}

fn tool_tab(tool: Tool, active: bool) -> Element<'static, Message> {
    let color = if active { ACCENT } else { TEXT_PRIMARY };
    button(
        column![fa(tool.icon(), 14.0, color), label(tool.label(), 9, color)]
            .spacing(2)
            .align_x(iced::Alignment::Center),
    )
    .padding([4, 6])
    .width(Length::Fill)
    .style(button_style(active, ACCENT))
    .on_press(ext(ExtMessage::SetTool(tool)))
    .into()
}

pub fn tool_tabs(state: &State) -> Element<'static, Message> {
    let mut rows = Column::new().spacing(4);
    for chunk in Tool::ALL.chunks(4) {
        let mut tabs = Row::new().spacing(4);
        for tool in chunk {
            tabs = tabs.push(tool_tab(*tool, state.ext.tool == *tool));
        }
        rows = rows.push(tabs);
    }
    container(rows).padding([8, 12]).width(Length::Fill).into()
}

// ------------------------------------------------------------------------------------------------
// Curve
// ------------------------------------------------------------------------------------------------

pub fn curve_panel(state: &State) -> Element<'static, Message> {
    let mut tabs = Row::new().spacing(4);
    for ch in CurveChannel::ALL {
        tabs = tabs.push(chip_button(ch.label(), state.ext.curve_channel == ch, Some(ext(ExtMessage::CurveChannel(ch)))));
    }
    let editor = CurveEditor { curve: state.params.curve.clone(), channel: state.ext.curve_channel }.view(296.0);
    panel(
        column![
            tabs,
            editor,
            row![
                label("Click to add · drag to move · double-click or right-click removes", 10, TEXT_DIM),
            ],
            action_button("rotate-left", "Reset channel", Some(ext(ExtMessage::CurveReset))),
        ]
        .spacing(10),
    )
}

// ------------------------------------------------------------------------------------------------
// Colour grading
// ------------------------------------------------------------------------------------------------

fn wheel_cell(state: &State, id: WheelId) -> Element<'static, Message> {
    let wheel = match id {
        WheelId::Shadows => state.params.grading.shadows,
        WheelId::Midtones => state.params.grading.midtones,
        WheelId::Highlights => state.params.grading.highlights,
        WheelId::Global => state.params.grading.global,
    };
    let lum = vertical_slider(-1.0..=1.0, wheel.lum, move |v| ext(ExtMessage::WheelLum(id, v))).step(0.01).height(Length::Fixed(110.0)).on_release(Message::CommitEdit);
    column![
        label(id.label(), 11, TEXT_SECONDARY),
        row![WheelView::view(id, wheel, 116.0), lum].spacing(6).align_y(Vertical::Center),
        mono(format!("H {:>3.0}°  S {:>3.0}  L {:+.0}", wheel.hue, wheel.sat * 100.0, wheel.lum * 100.0), 9, TEXT_DIM),
    ]
    .spacing(4)
    .into()
}

pub fn color_panel(state: &State) -> Element<'static, Message> {
    let mut body = Column::new()
        .spacing(12)
        .push(row![wheel_cell(state, WheelId::Shadows), wheel_cell(state, WheelId::Midtones)].spacing(10))
        .push(row![wheel_cell(state, WheelId::Highlights), wheel_cell(state, WheelId::Global)].spacing(10))
        .push(label("Double-click a wheel to reset it", 10, TEXT_DIM));

    body = body
        .push(h_rule())
        .push(section_header_plain("BLENDING & BALANCE"))
        .push(value_row("Blending", state.params.grading.blending, 0.0, 100.0, |v| ext(ExtMessage::GradingBlending(v))))
        .push(value_row("Balance", state.params.grading.balance, -100.0, 100.0, |v| ext(ExtMessage::GradingBalance(v))));

    body = body.push(h_rule()).push(section_header_plain("HSL / COLOUR MIXER"));
    let mut channels = Row::new().spacing(4);
    for channel in MixerChannel::ALL {
        channels = channels.push(chip_button(channel.label(), state.ext.mixer_channel == channel, Some(ext(ExtMessage::MixerChannel(channel)))));
    }
    body = body.push(channels);
    for band in 0..BAND_NAMES.len() {
        body = body.push(adjust_row(state, state.ext.mixer_channel.id(band)));
    }

    body = body
        .push(h_rule())
        .push(
            row![
                section_header_plain("BLACK & WHITE"),
                chip_button("Convert", state.params.bw.enabled, Some(ext(ExtMessage::ToggleBw))),
            ]
            .spacing(8)
            .align_y(Vertical::Center),
        );
    if state.params.bw.enabled {
        for band in 0..BAND_NAMES.len() {
            body = body.push(adjust_row(state, AdjustId::BwMix(band)));
        }
    }

    body = body.push(h_rule()).push(section_header_plain("PARAMETRIC CURVE"));
    for id in AdjustId::PARAMETRIC {
        body = body.push(adjust_row(state, id));
    }
    body = body.push(label("Region splits", 10, TEXT_DIM));
    for id in AdjustId::PARAMETRIC_SPLIT {
        body = body.push(adjust_row(state, id));
    }

    panel(body)
}

// ------------------------------------------------------------------------------------------------
// Effects / Detail
// ------------------------------------------------------------------------------------------------

pub fn effects_panel(state: &State) -> Element<'static, Message> {
    panel(
        column![
            adjust_group(state, "TEXTURE & CLARITY", &[AdjustId::Texture, AdjustId::Clarity, AdjustId::Dehaze]),
            h_rule(),
            adjust_group(
                state,
                "VIGNETTE",
                &[AdjustId::Vignette, AdjustId::VignetteMidpoint, AdjustId::VignetteRoundness, AdjustId::VignetteFeather],
            ),
            h_rule(),
            adjust_group(state, "GRAIN", &[AdjustId::Grain, AdjustId::GrainSize, AdjustId::GrainRoughness]),
            label("Texture, clarity and dehaze use per-image detail maps.", 10, TEXT_DIM),
        ]
        .spacing(12),
    )
}

pub fn detail_panel(state: &State) -> Element<'static, Message> {
    panel(
        column![
            adjust_group(state, "SHARPENING", &AdjustId::DETAIL),
            label("Detail controls the sharpening radius mix; masking protects flat areas.", 10, TEXT_DIM),
        ]
        .spacing(10),
    )
}

// ------------------------------------------------------------------------------------------------
// Masks / local adjustments
// ------------------------------------------------------------------------------------------------

pub fn mask_panel(state: &State) -> Element<'static, Message> {
    let has_doc = state.doc.is_some();
    let busy = state.ext.mask_busy;
    let ai_note: Element<'static, Message> = match &state.ext.vision {
        VisionState::Unavailable(e) => label(format!("AI models unavailable — {e}"), 10, hex_warn()).into(),
        VisionState::Loading => label("Loading AI models…", 10, TEXT_DIM).into(),
        VisionState::Ready(engine) => label(format!("On-device AI · {}", engine.provider()), 10, TEXT_DIM).into(),
        VisionState::Idle => label("AI models load on demand", 10, TEXT_DIM).into(),
    };
    let ai_ok = has_doc && !busy && !matches!(state.ext.vision, VisionState::Unavailable(_));
    let buttons = column![
        row![
            chip_button("Select Subject", false, ai_ok.then(|| ext(ExtMessage::AiMask(AiMaskKind::Subject)))),
            chip_button("Select Sky", false, ai_ok.then(|| ext(ExtMessage::AiMask(AiMaskKind::Sky)))),
        ]
        .spacing(6),
        row![
            chip_button("Linear gradient", false, has_doc.then(|| ext(ExtMessage::GradientMask(GradientKind::Linear)))),
            chip_button("Radial gradient", false, has_doc.then(|| ext(ExtMessage::GradientMask(GradientKind::Radial)))),
        ]
        .spacing(6),
    ]
    .spacing(6);

    let mut body = Column::new().spacing(10).push(buttons).push(ai_note);
    if busy {
        body = body.push(label("Working…", 11, ACCENT));
    }
    if let Some(local) = &state.params.local {
        let source = match &local.source {
            Some(halation_core::MaskSource::Ai { kind: AiMaskKind::Subject, .. }) => "Subject",
            Some(halation_core::MaskSource::Ai { kind: AiMaskKind::Sky, .. }) => "Sky",
            Some(halation_core::MaskSource::Linear { .. }) => "Linear gradient",
            Some(halation_core::MaskSource::Radial { .. }) => "Radial gradient",
            None => "No mask yet",
        };
        body = body.push(h_rule()).push(label(format!("LOCAL ADJUSTMENT · {source}"), 10, TEXT_DIM));
        for field in LocalField::ALL {
            let (lo, hi) = field.range();
            let v = field.get(local);
            body = body.push(
                row![
                    container(label(field.label(), 11, TEXT_SECONDARY)).width(Length::Fixed(74.0)),
                    slider(lo..=hi, v, move |x| ext(ExtMessage::LocalParam(field, x))).step(if field == LocalField::Exposure { 0.01 } else { 1.0_f32 }).on_release(Message::CommitEdit).width(Length::Fill),
                    container(mono(if field == LocalField::Exposure { format!("{v:+.2}") } else { format!("{v:+.0}") }, 11, TEXT_SECONDARY)).width(Length::Fixed(48.0)),
                ]
                .spacing(8)
                .align_y(Vertical::Center),
            );
        }
        body = body.push(row![
            chip_button("Invert", local.invert, Some(ext(ExtMessage::LocalInvert))),
            chip_button("Remove", false, Some(ext(ExtMessage::LocalClear))),
        ].spacing(6));
    }
    panel(body)
}

fn hex_warn() -> iced::Color {
    iced::Color::from_rgb8(0xE5, 0xB4, 0x4D)
}

// ------------------------------------------------------------------------------------------------
// Export / external editor
// ------------------------------------------------------------------------------------------------

pub fn export_panel(state: &State) -> Element<'static, Message> {
    let ui = &state.ext.export;
    let mut formats = Column::new().spacing(4);
    for chunk in FormatChoice::ALL.chunks(4) {
        let mut r = Row::new().spacing(4);
        for f in chunk {
            r = r.push(chip_button(f.label(), ui.format == *f, Some(ext(ExtMessage::ExportFormat(*f)))));
        }
        formats = formats.push(r);
    }
    let quality: Element<'static, Message> = if ui.format == FormatChoice::Jpeg {
        row![
            container(label("Quality", 11, TEXT_SECONDARY)).width(Length::Fixed(74.0)),
            slider(1.0_f32..=100.0_f32, ui.quality as f32, |q| ext(ExtMessage::ExportQuality(q))).step(1.0_f32).width(Length::Fill),
            container(mono(ui.quality.to_string(), 11, TEXT_SECONDARY)).width(Length::Fixed(40.0)),
        ]
        .spacing(8)
        .align_y(Vertical::Center)
        .into()
    } else {
        Space::with_height(Length::Fixed(0.0)).into()
    };
    let dng = ui.format == FormatChoice::Dng;
    let mut spaces = Row::new().spacing(4);
    for s in [OutputSpace::Srgb, OutputSpace::DisplayP3, OutputSpace::AdobeRgb, OutputSpace::ProPhoto] {
        spaces = spaces.push(chip_button(
            match s {
                OutputSpace::Srgb => "sRGB",
                OutputSpace::DisplayP3 => "P3",
                OutputSpace::AdobeRgb => "Adobe",
                OutputSpace::ProPhoto => "ProPhoto",
            },
            ui.space == s,
            (!dng).then(|| ext(ExtMessage::ExportSpace(s))),
        ));
    }
    let mut sizes = Row::new().spacing(4);
    for (name, edge) in [("Original", None), ("4096", Some(4096)), ("2048", Some(2048)), ("1080", Some(1080))] {
        sizes = sizes.push(chip_button(name, ui.max_edge == edge, Some(ext(ExtMessage::ExportSize(edge)))));
    }
    let progress: Element<'static, Message> = match (&ui.progress, ui.running) {
        (Some(p), true) => column![
            progress_bar(0.0..=100.0, p.percent()).height(Length::Fixed(6.0)),
            mono(format!("{}/{}  ·  {:.0}%  ·  ~{:.0}s left", p.done, p.total, p.percent(), p.eta_secs), 10, TEXT_DIM),
        ]
        .spacing(4)
        .into(),
        _ => label(ui.message.clone(), 10, TEXT_DIM).into(),
    };
    let editor_name = state.ext.external.editor.as_ref().map(|e| e.name.clone());
    panel(
        column![
            section_header_plain("EXPORT"),
            row![
                chip_button("Current photo", ui.scope == ExportScope::Current, Some(ext(ExtMessage::ExportScope(ExportScope::Current)))),
                chip_button("All in view", ui.scope == ExportScope::Visible, Some(ext(ExtMessage::ExportScope(ExportScope::Visible)))),
            ]
            .spacing(4),
            formats,
            quality,
            label("Colour space", 10, TEXT_DIM),
            spaces,
            label("Longest edge (Lanczos3)", 10, TEXT_DIM),
            sizes,
            row![
                chip_button("Logo watermark…", ui.logo.is_some(), Some(ext(ExtMessage::ExportPickLogo))),
                chip_button("Clear", false, ui.logo.is_some().then(|| ext(ExtMessage::ExportLogo(None)))),
            ]
            .spacing(4),
            button(label(if ui.running { "Exporting…" } else { "Export…" }, 12, TEXT_PRIMARY))
                .padding([6, 18])
                .style(button_style(true, PICK))
                .on_press_maybe((!ui.running && state.selected.is_some()).then(|| ext(ExtMessage::ExportRun))),
            progress,
            h_rule(),
            section_header_plain("EXTERNAL EDITOR"),
            row![
                chip_button(if editor_name.is_some() { "Change…" } else { "Choose editor…" }, false, Some(ext(ExtMessage::ExternalPick))),
                chip_button("Open in editor", false, (editor_name.is_some() && state.doc.is_some()).then(|| ext(ExtMessage::ExternalOpen))),
            ]
            .spacing(4),
            label(editor_name.map_or("No editor configured".to_owned(), |n| format!("{n}  ·  sends a 16-bit ProPhoto TIFF")), 10, TEXT_DIM),
        ]
        .spacing(10),
    )
}

pub fn tool_content(state: &State) -> Option<Element<'static, Message>> {
    match state.ext.tool {
        Tool::Edit => None,
        Tool::Curve => Some(curve_panel(state)),
        Tool::Color => Some(color_panel(state)),
        Tool::Effects => Some(effects_panel(state)),
        Tool::Detail => Some(detail_panel(state)),
        Tool::Mask => Some(mask_panel(state)),
        Tool::Export => Some(export_panel(state)),
    }
}
