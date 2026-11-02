//! The Mockups panel, opened from the rail: device mockups for iPhone, iPhone Duo, iPad and Mac
//! (see `studio_core::mockups`). Pick which way up, the composition's shape and a finish, then
//! click a device: it makes the device's Screen composition (with the video or picture selected
//! in Media on it, if any) and the mockup showing it, and opens the mockup.
//!
//! Finishes are a few ready-made ones plus, once Studio knows where OpenConnect is, each
//! OpenConnect app's device frame colors (read in the background; see `openconnect`).

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::{
    Context, ImageSource, InteractiveElement, IntoElement, ParentElement, RenderImage, StatefulInteractiveElement, Styled, TestSupportExt as _, Window, div, img,
    prelude::FluentBuilder as _, px,
};
use studio_core::mockups::{Device, Finish, MockupRequest, Orientation};
use studio_core::model::Rgba;
use studio_core::templates::Format;

use super::chrome::panel;
use crate::openconnect::AppFinish;
use crate::theme::{color, hsla, size};
use crate::workspace::{EditTarget, PanelId, Workspace};

/// Card picture width, in points.
const CARD_WIDTH: f32 = 128.0;

/// Which finish is chosen: a ready-made one, or an OpenConnect app's (by name, so it survives
/// reading the apps again).
#[derive(Clone, Debug, PartialEq)]
pub enum FinishChoice {
    Preset(usize),
    App(String),
}

/// How reading OpenConnect's apps went.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum OpenConnectStatus {
    #[default]
    NotRead,
    Reading,
    Read,
    Failed(String),
}

/// The panel's choices, its device pictures, and the finishes read from OpenConnect.
pub struct MockupsUi {
    pub orientation: Orientation,
    pub format: Format,
    pub finish: FinishChoice,
    /// Whether the mockup gets a background (the finish's color), or stays see-through.
    pub background: bool,
    /// Pictures of each device as currently chosen; cleared when a choice changes.
    pub pictures: HashMap<Device, Arc<RenderImage>>,
    pub apps: Vec<AppFinish>,
    pub status: OpenConnectStatus,
    /// The address the apps were last read from, so a new address reads them again.
    pub read_from: Option<String>,
}

impl Default for MockupsUi {
    fn default() -> Self {
        Self {
            orientation: Orientation::Portrait,
            format: Format::Wide,
            finish: FinishChoice::Preset(0),
            background: true,
            pictures: HashMap::new(),
            apps: Vec::new(),
            status: OpenConnectStatus::NotRead,
            read_from: None,
        }
    }
}

impl MockupsUi {
    /// The chosen finish, with or without its background. A finish without a background color
    /// gets a dark one when a background is asked for.
    pub fn finish(&self) -> Finish {
        let presets = Finish::presets();
        let finish = match &self.finish {
            FinishChoice::Preset(index) => presets.get(*index).unwrap_or(&presets[0]).1,
            FinishChoice::App(name) => self.apps.iter().find(|app| &app.name == name).map_or(presets[0].1, |app| app.finish),
        };
        let background = if self.background { Some(finish.background.unwrap_or(Rgba::hex(0x101114))) } else { None };
        Finish { background, ..finish }
    }

    fn finish_name(&self) -> String {
        match &self.finish {
            FinishChoice::Preset(index) => Finish::presets().get(*index).map_or("Space Black", |(name, _)| name).to_owned(),
            FinishChoice::App(name) => name.clone(),
        }
    }

    fn request(&self, device: Device) -> MockupRequest {
        MockupRequest { device, orientation: self.orientation, format: self.format, finish: self.finish(), footage: None, seconds: 10.0 }
    }
}

/// While the panel is open: reads OpenConnect's apps once Studio knows where it is, and draws
/// the device pictures that are missing.
pub fn keep_up(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let api = workspace.ui.settings.openconnect_api.clone();
    if let Some(api) = api.filter(|api| workspace.ui.mockups.read_from.as_ref() != Some(api)) {
        read_openconnect(workspace, api, window, cx);
    }
    let (width, height) = picture_size(workspace.ui.mockups.format);
    for device in Device::ALL {
        if workspace.ui.mockups.pictures.contains_key(&device) {
            continue;
        }
        let request = workspace.ui.mockups.request(device);
        match workspace.render.mockup_picture(&request, width, height) {
            Some(picture) => {
                workspace.ui.mockups.pictures.insert(device, picture);
            }
            // Without a GPU there is nothing to show.
            None => return,
        }
    }
}

/// Picture size in pixels for a card: twice the card, for sharp high-density screens.
fn picture_size(format: Format) -> (u32, u32) {
    let (width, height) = format.size();
    let w = CARD_WIDTH * 2.0;
    (w as u32, (w * height as f32 / width as f32) as u32)
}

/// Reads the finishes of the OpenConnect apps at `api` in the background.
fn read_openconnect(workspace: &mut Workspace, api: String, window: &mut Window, cx: &mut Context<Workspace>) {
    workspace.ui.mockups.read_from = Some(api.clone());
    workspace.ui.mockups.status = OpenConnectStatus::Reading;
    cx.spawn_in(window, async move |this, cx| {
        let reading = api.clone();
        let result = cx.background_executor().spawn(async move { crate::openconnect::fetch(&reading) }).await;
        this.update(cx, |this, cx| {
            // A newer address was set meanwhile: that read wins.
            if this.ui.mockups.read_from.as_deref() != Some(api.as_str()) {
                return;
            }
            match result {
                Ok(apps) => {
                    this.ui.mockups.apps = apps;
                    this.ui.mockups.status = OpenConnectStatus::Read;
                }
                Err(message) => this.ui.mockups.status = OpenConnectStatus::Failed(message),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Stores the OpenConnect address typed into the panel (empty forgets it) and reads its apps.
pub fn set_openconnect_api(workspace: &mut Workspace, text: &str, window: &mut Window, cx: &mut Context<Workspace>) {
    let api = Some(text.trim().trim_end_matches('/').to_owned()).filter(|api| !api.is_empty());
    crate::app_settings::update(|settings| settings.openconnect_api = api.clone());
    workspace.ui.settings.openconnect_api = api.clone();
    let mockups = &mut workspace.ui.mockups;
    mockups.read_from = None;
    mockups.apps.clear();
    mockups.status = OpenConnectStatus::NotRead;
    if let FinishChoice::App(_) = mockups.finish {
        mockups.finish = FinishChoice::Preset(0);
        mockups.pictures.clear();
    }
    if let Some(api) = api {
        read_openconnect(workspace, api, window, cx);
    }
}

fn chip(id: impl Into<gpui_kit::ElementId>, label: &'static str, active: bool) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .px_2()
        .h(px(24.0))
        .flex()
        .items_center()
        .rounded(px(12.0))
        .border_1()
        .border_color(hsla(if active { color::ACCENT } else { color::HAIRLINE }))
        .when(active, |chip| chip.bg(hsla(color::ROW_SELECTED)))
        .text_size(px(size::SMALL_TEXT))
        .text_color(hsla(if active { color::TEXT } else { color::MUTED }))
        .hover(|style| style.bg(hsla(color::HOVER)))
        .cursor_pointer()
        .child(label)
}

/// The finish menu: the ready-made finishes, then each OpenConnect app's.
fn finish_menu(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let ui = &workspace.ui.mockups;
    let current = ui.finish.clone();
    let mut choices: Vec<(String, FinishChoice)> = Finish::presets().iter().enumerate().map(|(index, (name, _))| ((*name).to_owned(), FinishChoice::Preset(index))).collect();
    let presets = choices.len();
    choices.extend(ui.apps.iter().map(|app| (app.name.clone(), FinishChoice::App(app.name.clone()))));
    let target = cx.weak_entity();
    Button::new("mockup-finish").outline().xsmall().label(ui.finish_name()).dropdown_caret(true).dropdown_menu(move |menu, _, _| {
        choices.iter().enumerate().fold(menu, |menu, (index, (name, choice))| {
            let menu = if index == presets { menu.separator().label("OpenConnect apps") } else { menu };
            let (target, choice) = (target.clone(), choice.clone());
            menu.item(PopupMenuItem::new(name.clone()).checked(choice == current).on_click(move |_, _, cx| {
                target
                    .update(cx, |this, cx| {
                        this.ui.mockups.finish = choice.clone();
                        this.ui.mockups.pictures.clear();
                        cx.notify();
                    })
                    .ok();
            }))
        })
    })
}

/// Where OpenConnect is, and how reading its apps went.
fn openconnect_section(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let ui = &workspace.ui.mockups;
    let api = workspace.ui.settings.openconnect_api.clone();
    let editing = workspace.ui.editing.as_ref().filter(|editing| editing.target == EditTarget::OpenConnectApi).map(|editing| editing.input.clone());
    let address = match editing {
        Some(input) => gpui_kit::component::input::Input::new(&input).xsmall().into_any_element(),
        None => {
            let shown = api.clone().unwrap_or_else(|| "Set the address…".to_owned());
            let start = api.clone().unwrap_or_default();
            div()
                .id("openconnect-address")
                .px_1p5()
                .py_0p5()
                .rounded(px(4.0))
                .border_1()
                .border_color(hsla(color::HAIRLINE))
                .text_size(px(size::SMALL_TEXT))
                .text_color(hsla(if api.is_some() { color::TEXT } else { color::MUTED }))
                .truncate()
                .cursor_text()
                .hover(|style| style.border_color(hsla(color::VALUE)))
                // Kept from the panel, which would take focus back and close the field at once.
                .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.begin_edit(EditTarget::OpenConnectApi, start.clone(), window, cx);
                }))
                .child(shown)
                .test_support()
                .into_any_element()
        }
    };
    let status = match (&ui.status, api.is_some()) {
        (_, false) => "Use your OpenConnect apps' device colors: enter its API address, such as http://host:3100/studio/api.".to_owned(),
        (OpenConnectStatus::Reading, _) => "Reading your apps…".to_owned(),
        (OpenConnectStatus::Read, _) => format!("{} apps in the finish menu.", ui.apps.len()),
        (OpenConnectStatus::Failed(message), _) => message.clone(),
        (OpenConnectStatus::NotRead, _) => String::new(),
    };
    let refresh = api.is_some() && ui.status != OpenConnectStatus::Reading;
    div()
        .flex()
        .flex_col()
        .gap_1()
        .pt_2()
        .child(div().text_size(px(size::SMALL_TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child("OpenConnect"))
        .child(address)
        .child(
            div()
                .flex()
                .items_start()
                .justify_between()
                .gap_2()
                .child(div().flex_1().min_w_0().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(status))
                .when(refresh, |row| {
                    row.child(Button::new("openconnect-refresh").ghost().xsmall().label("Refresh").on_click(cx.listener(|this, _, _, cx| {
                        // Reading again happens on the next redraw.
                        this.ui.mockups.read_from = None;
                        cx.notify();
                    })))
                }),
        )
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let ui = &workspace.ui.mockups;
    let pick = |this: &mut Workspace, change: &dyn Fn(&mut MockupsUi)| {
        change(&mut this.ui.mockups);
        this.ui.mockups.pictures.clear();
    };
    let ways = [(Orientation::Portrait, "Upright"), (Orientation::Landscape, "Sideways")];
    let orientation = div().flex().gap_1().children(ways.into_iter().enumerate().map(|(index, (choice, label))| {
        chip(("mockup-orientation", index), label, ui.orientation == choice)
            .on_click(cx.listener(move |this, _, _, cx| {
                pick(this, &|ui| ui.orientation = choice);
                cx.notify();
            }))
            .test_support()
    }));
    let formats = div().flex().gap_1().children(Format::ALL.into_iter().enumerate().map(|(index, choice)| {
        chip(("mockup-format", index), choice.label(), ui.format == choice)
            .on_click(cx.listener(move |this, _, _, cx| {
                pick(this, &|ui| ui.format = choice);
                cx.notify();
            }))
            .test_support()
    }));
    let background = Checkbox::new("mockup-background").label("Background").checked(ui.background).on_click(cx.listener(move |this, _, _, cx| {
        pick(this, &|ui| ui.background = !ui.background);
        cx.notify();
    }));
    let footage = match workspace.state.mockup_footage() {
        Some(item) => format!("On the screen: {}", item.name),
        None => "Select a video or picture in Media first to put it on the screen.".to_owned(),
    };
    let (width, height) = picture_size(ui.format);
    let card_height = CARD_WIDTH * height as f32 / width as f32;
    let cards = Device::ALL.into_iter().enumerate().map(|(index, device)| {
        let picture = ui.pictures.get(&device).cloned();
        let sideways = ui.orientation == Orientation::Landscape && device.turns();
        let (w, h) = device.screen();
        let screen = if sideways { format!("{h} × {w}") } else { format!("{w} × {h}") };
        div()
            .id(("mockup", index))
            .w(px(CARD_WIDTH))
            .flex()
            .flex_col()
            .gap_1()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                let ui = &this.ui.mockups;
                let (orientation, format, finish) = (ui.orientation, ui.format, ui.finish());
                if this.state.add_mockup(device, orientation, format, finish).is_some() {
                    cx.notify();
                }
            }))
            .child(
                div()
                    .w(px(CARD_WIDTH))
                    .h(px(card_height))
                    .rounded(px(6.0))
                    .overflow_hidden()
                    .bg(hsla(color::INSET))
                    .border_1()
                    .border_color(hsla(color::HAIRLINE))
                    .hover(|style| style.border_color(hsla(color::ACCENT)))
                    .children(picture.map(|picture| img(ImageSource::Render(picture)).size_full())),
            )
            .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::TEXT)).child(device.name()))
            .child(div().text_size(px(10.0)).text_color(hsla(color::MUTED)).child(format!("Screen {screen}")))
            .test_support()
    }).collect::<Vec<_>>();
    let (finishes, openconnect) = (finish_menu(workspace, cx), openconnect_section(workspace, cx));
    let body = div()
        .id("mockups-list")
        .size_full()
        .overflow_y_scroll()
        .p_2()
        .flex()
        .flex_col()
        .gap_2()
        .child(orientation)
        .child(div().flex().items_center().justify_between().gap_2().child(formats).child(background))
        .child(div().flex().items_center().gap_2().child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Finish")).child(finishes))
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Click a device to make its mockup. Your footage goes in its Screen composition; the mockup follows it."))
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::VALUE)).child(footage))
        .child(div().flex().flex_wrap().gap_2().children(cards))
        .child(openconnect)
        .test_support();
    panel(PanelId::Mockups, &["Mockups"], None, body, workspace, cx)
}
