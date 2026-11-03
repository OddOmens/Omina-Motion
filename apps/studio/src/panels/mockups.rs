//! The Mockups panel, opened from the rail: device mockups for iPhone, iPhone Duo, iPad and Mac
//! (see `studio_core::mockups`). Each device's card shows how it looks. Its + button opens the
//! finishes that device comes in; picking one adds the device to the open composition as one
//! layer, centered and scaled to fit, with the video or picture selected in Media on its
//! screen, if any. Looking at the cards adds nothing. Devices are added upright; rotate the
//! layer to turn one on its side.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::{
    Context, ImageSource, InteractiveElement, IntoElement, ParentElement, RenderImage, StatefulInteractiveElement, Styled, TestSupportExt as _, Window, div, img, px,
};
use studio_core::mockups::{Device, MockupRequest, Orientation};

use super::chrome::panel;
use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, Workspace};

/// Card picture width and height, in points. Each device is fitted inside.
const CARD_WIDTH: f32 = 128.0;
const CARD_HEIGHT: f32 = 128.0;

/// The panel's device pictures, each in its first finish, made once.
#[derive(Default)]
pub struct MockupsUi {
    pub pictures: HashMap<Device, Arc<RenderImage>>,
}

/// A device as its card shows it: upright, in its first finish, with nothing on its screen.
fn preview_request(device: Device) -> Option<MockupRequest> {
    let (_, finish) = device.finishes().into_iter().next()?;
    Some(MockupRequest { device, orientation: Orientation::Portrait, finish, footage: None, seconds: 1.0 })
}

/// While the panel is open: draws the device pictures that are missing.
pub fn keep_up(workspace: &mut Workspace, _window: &mut Window, _cx: &mut Context<Workspace>) {
    let side = (CARD_WIDTH.max(CARD_HEIGHT) * 2.0) as u32;
    for device in Device::ALL {
        if workspace.ui.mockups.pictures.contains_key(&device) {
            continue;
        }
        let Some(request) = preview_request(device) else { continue };
        match workspace.render.mockup_picture(&request, side) {
            Some(picture) => {
                workspace.ui.mockups.pictures.insert(device, picture);
            }
            // Without a GPU there is nothing to show.
            None => return,
        }
    }
}

/// A card's + button: a menu of the device's finishes, each adding the device in that finish.
fn add_button(index: usize, device: Device, workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let finishes = device.finishes();
    let composition = workspace.state.composition().name.clone();
    let target = cx.weak_entity();
    Button::new(("mockup-add", index)).icon(Icon::new(IconName::Plus)).small().outline().tooltip(format!("Add to {composition}")).dropdown_menu(move |menu, _, _| {
        finishes.iter().fold(menu.label("Finish"), |menu, &(name, finish)| {
            let target = target.clone();
            menu.item(PopupMenuItem::new(name).on_click(move |_, _, cx| {
                target
                    .update(cx, |this, cx| {
                        if this.state.add_mockup(device, Orientation::Portrait, finish).is_some() {
                            cx.notify();
                        }
                    })
                    .ok();
            }))
        })
    })
}

/// One device's card: its picture, name and screen size, with the + button that adds it.
fn card(index: usize, device: Device, workspace: &Workspace, cx: &mut Context<Workspace>) -> gpui_kit::AnyElement {
    let picture = workspace.ui.mockups.pictures.get(&device).cloned();
    let (w, h) = device.screen();
    div()
        .id(("mockup", index))
        .w(px(CARD_WIDTH))
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .relative()
                .w(px(CARD_WIDTH))
                .h(px(CARD_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .rounded(px(6.0))
                .bg(hsla(color::INSET))
                .border_1()
                .border_color(hsla(color::HAIRLINE))
                // The device fitted into the card, a little in from its edges.
                .children(picture.map(|picture| {
                    gpui_kit::StyledImage::object_fit(img(ImageSource::Render(picture)).w(px(CARD_WIDTH - 16.0)).h(px(CARD_HEIGHT - 16.0)), gpui_kit::ObjectFit::Contain)
                }))
                .child(div().absolute().top(px(6.0)).right(px(6.0)).child(add_button(index, device, workspace, cx))),
        )
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::TEXT)).child(device.name()))
        .child(div().text_size(px(10.0)).text_color(hsla(color::MUTED)).child(format!("Screen {w} × {h}")))
        .test_support()
        .into_any_element()
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let footage = match workspace.state.mockup_footage() {
        Some(item) => format!("On the screen: {}", item.name),
        None => "Select a video or picture in Media to put it on the screen.".to_owned(),
    };
    let cards = Device::ALL.into_iter().enumerate().map(|(index, device)| card(index, device, workspace, cx)).collect::<Vec<_>>();
    let body = div()
        .id("mockups-list")
        .size_full()
        .overflow_y_scroll()
        .p_2()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(format!("Press + and pick a finish to add a device to {}, scaled to fit. Rotate it to turn it on its side.", workspace.state.composition().name)))
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::VALUE)).child(footage))
        .child(div().flex().flex_wrap().gap_2().children(cards))
        .test_support();
    panel(PanelId::Mockups, &["Mockups"], None, body, workspace, cx)
}
