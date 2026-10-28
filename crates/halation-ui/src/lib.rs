use halation_catalog::{Catalog, CatalogItem};
use halation_core::DevelopParameters;
use halation_gpu::GpuEngineInfo;
use halation_pipeline::process_frame;
use halation_raw::{load_sample_raws, RawAsset};
use iced::widget::{
    button, column, container, image, row, scrollable, slider, text, Space,
};
use iced::{Center, Element, Length, Task, Theme};

const PREVIEW_W: u32 = 640;
const PREVIEW_H: u32 = 420;

pub fn run() -> iced::Result {
    iced::application("Halation - Professional Darkroom", update, view)
        .theme(|_state| Theme::Dark)
        .run()
}

pub struct State {
    catalog: Catalog,
    catalog_items: Vec<CatalogItem>,
    raw_assets: Vec<RawAsset>,
    selected_index: usize,
    active_params: DevelopParameters,
    viewport_image: image::Handle,
    histogram_image: image::Handle,
    gpu_info: GpuEngineInfo,
    status_toast: String,
}

impl Default for State {
    fn default() -> Self {
        let catalog = Catalog::open_in_memory().expect("Failed to initialize SQLite catalog");
        let catalog_items = catalog.load_items();
        let raw_assets = load_sample_raws(PREVIEW_W, PREVIEW_H);
        let gpu_info = GpuEngineInfo::probe();

        let initial_params = catalog_items[0].params.clone();
        let initial_output = process_frame(&raw_assets[0], &initial_params);

        let viewport_image = image::Handle::from_rgba(PREVIEW_W, PREVIEW_H, initial_output.display_rgba);
        let histogram_image = image::Handle::from_rgba(256, 64, initial_output.histogram_rgba);

        Self {
            catalog,
            catalog_items,
            raw_assets,
            selected_index: 0,
            active_params: initial_params,
            viewport_image,
            histogram_image,
            gpu_info,
            status_toast: "Ready".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectPhoto(usize),
    ParamExposure(f32),
    ParamContrast(f32),
    ParamHighlights(f32),
    ParamShadows(f32),
    ParamTemperature(f32),
    ParamTint(f32),
    SetRating(u8),
    SetFlag(String),
    SaveToCatalog,
    ResetDefaults,
}

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::SelectPhoto(idx) => {
            if idx < state.raw_assets.len() {
                state.selected_index = idx;
                state.active_params = state.catalog_items[idx].params.clone();
                recompute_viewport(state);
                state.status_toast = format!("Loaded {}", state.catalog_items[idx].filename);
            }
        }
        Message::ParamExposure(v) => {
            state.active_params.exposure = v;
            recompute_viewport(state);
        }
        Message::ParamContrast(v) => {
            state.active_params.contrast = v;
            recompute_viewport(state);
        }
        Message::ParamHighlights(v) => {
            state.active_params.highlights = v;
            recompute_viewport(state);
        }
        Message::ParamShadows(v) => {
            state.active_params.shadows = v;
            recompute_viewport(state);
        }
        Message::ParamTemperature(v) => {
            state.active_params.temperature = v;
            recompute_viewport(state);
        }
        Message::ParamTint(v) => {
            state.active_params.tint = v;
            recompute_viewport(state);
        }
        Message::SetRating(stars) => {
            let id = state.catalog_items[state.selected_index].id;
            state.catalog.update_rating(id, stars);
            state.catalog_items[state.selected_index].rating = stars;
            state.status_toast = format!("Rated {} stars", stars);
        }
        Message::SetFlag(flag) => {
            let id = state.catalog_items[state.selected_index].id;
            state.catalog.update_flag(id, &flag);
            state.catalog_items[state.selected_index].flag = flag.clone();
            state.status_toast = format!("Flagged as {}", flag);
        }
        Message::SaveToCatalog => {
            let id = state.catalog_items[state.selected_index].id;
            state.catalog.save_parameters(id, &state.active_params);
            state.catalog_items[state.selected_index].params = state.active_params.clone();
            state.status_toast = "Saved develop state to SQLite catalog!".into();
        }
        Message::ResetDefaults => {
            state.active_params = DevelopParameters::default();
            recompute_viewport(state);
            state.status_toast = "Reset to calibrated defaults".into();
        }
    }
    Task::none()
}

fn recompute_viewport(state: &mut State) {
    let output = process_frame(&state.raw_assets[state.selected_index], &state.active_params);
    state.viewport_image = image::Handle::from_rgba(PREVIEW_W, PREVIEW_H, output.display_rgba);
    state.histogram_image = image::Handle::from_rgba(256, 64, output.histogram_rgba);
}

pub fn view(state: &State) -> Element<Message> {
    let cur_item = &state.catalog_items[state.selected_index];
    let cur_raw = &state.raw_assets[state.selected_index];

    // 1. Top Header Bar
    let header = container(
        row![
            text("HALATION").size(15),
            Space::with_width(Length::Fixed(24.0)),
            text(format!("GPU: {} ({})", state.gpu_info.adapter_name, state.gpu_info.backend_name))
                .size(11),
            Space::with_width(Length::Fill),
            text(&state.status_toast).size(12)
        ]
        .align_y(Center),
    )
    .padding(8)
    .width(Length::Fill);

    // 2. Left Panel: Catalog Browser, Filmstrip & Badges
    let mut catalog_column = column![
        text("CATALOG ASSETS").size(12),
        Space::with_height(Length::Fixed(6.0))
    ].spacing(6);

    for (i, item) in state.catalog_items.iter().enumerate() {
        let is_selected = i == state.selected_index;
        let prefix = if is_selected { "► " } else { "   " };
        let label = format!("{}{} [★ {} | {}]", prefix, item.filename, item.rating, item.flag);
        
        catalog_column = catalog_column.push(
            button(text(label).size(11))
                .width(Length::Fill)
                .on_press(Message::SelectPhoto(i))
        );
    }

    catalog_column = catalog_column.push(Space::with_height(Length::Fixed(16.0)));
    catalog_column = catalog_column.push(text("RATING & FLAGS").size(12));
    catalog_column = catalog_column.push(
        row![
            button(text("1★").size(10)).on_press(Message::SetRating(1)),
            button(text("2★").size(10)).on_press(Message::SetRating(2)),
            button(text("3★").size(10)).on_press(Message::SetRating(3)),
            button(text("4★").size(10)).on_press(Message::SetRating(4)),
            button(text("5★").size(10)).on_press(Message::SetRating(5)),
        ].spacing(4)
    );
    catalog_column = catalog_column.push(
        row![
            button(text("Pick [P]").size(10)).on_press(Message::SetFlag("Pick".into())),
            button(text("Reject [X]").size(10)).on_press(Message::SetFlag("Reject".into())),
            button(text("Unflag").size(10)).on_press(Message::SetFlag("None".into())),
        ].spacing(4)
    );

    let left_panel = container(catalog_column).width(Length::Fixed(240.0)).padding(12);

    // 3. Center Canvas & EXIF HUD
    let center_viewport = container(
        column![
            image(state.viewport_image.clone()),
            Space::with_height(Length::Fixed(6.0)),
            text(format!(
                "{} | {} | ISO {} | {} | {}",
                cur_raw.metadata.camera, cur_raw.metadata.lens, cur_raw.metadata.iso, cur_raw.metadata.shutter, cur_raw.metadata.aperture
            )).size(11)
        ].align_x(Center)
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Center)
    .align_y(Center);

    // 4. Right Panel: Real-time Histogram & Develop Controls
    let right_panel = container(
        scrollable(
            column![
                text("RGB HISTOGRAM").size(12),
                image(state.histogram_image.clone()),
                Space::with_height(Length::Fixed(8.0)),
                row![
                    text("DEVELOP TONE").size(12),
                    Space::with_width(Length::Fill),
                    button(text("Reset").size(10)).on_press(Message::ResetDefaults)
                ].align_y(Center),
                Space::with_height(Length::Fixed(6.0)),
                // Sliders
                row![
                    text("Exposure").size(11).width(Length::Fixed(70.0)),
                    slider(-5.0..=5.0, state.active_params.exposure, Message::ParamExposure).step(0.05),
                    text(format!("{:+.2} EV", state.active_params.exposure)).size(10).width(Length::Fixed(50.0))
                ].spacing(6).align_y(Center),
                row![
                    text("Contrast").size(11).width(Length::Fixed(70.0)),
                    slider(-100.0..=100.0, state.active_params.contrast, Message::ParamContrast).step(1.0),
                    text(format!("{:.0}", state.active_params.contrast)).size(10).width(Length::Fixed(50.0))
                ].spacing(6).align_y(Center),
                row![
                    text("Highlights").size(11).width(Length::Fixed(70.0)),
                    slider(-100.0..=100.0, state.active_params.highlights, Message::ParamHighlights).step(1.0),
                    text(format!("{:.0}", state.active_params.highlights)).size(10).width(Length::Fixed(50.0))
                ].spacing(6).align_y(Center),
                row![
                    text("Shadows").size(11).width(Length::Fixed(70.0)),
                    slider(-100.0..=100.0, state.active_params.shadows, Message::ParamShadows).step(1.0),
                    text(format!("{:.0}", state.active_params.shadows)).size(10).width(Length::Fixed(50.0))
                ].spacing(6).align_y(Center),
                Space::with_height(Length::Fixed(10.0)),
                text("WHITE BALANCE").size(12),
                row![
                    text("Temp").size(11).width(Length::Fixed(70.0)),
                    slider(2000.0..=10000.0, state.active_params.temperature, Message::ParamTemperature).step(25.0),
                    text(format!("{:.0} K", state.active_params.temperature)).size(10).width(Length::Fixed(50.0))
                ].spacing(6).align_y(Center),
                row![
                    text("Tint").size(11).width(Length::Fixed(70.0)),
                    slider(-100.0..=100.0, state.active_params.tint, Message::ParamTint).step(1.0),
                    text(format!("{:.0}", state.active_params.tint)).size(10).width(Length::Fixed(50.0))
                ].spacing(6).align_y(Center),
                Space::with_height(Length::Fixed(16.0)),
                button(text("Save to Catalog").size(12))
                    .width(Length::Fill)
                    .on_press(Message::SaveToCatalog)
            ].spacing(8)
        )
    )
    .width(Length::Fixed(300.0))
    .padding(12);

    let body = row![left_panel, center_viewport, right_panel].height(Length::Fill);

    column![header, body].into()
}
