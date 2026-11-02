//! The Elements panel, opened from the rail: ready-made graphics (see `studio_core::elements`)
//! grouped as shapes, lines and arrows, badges and labels, frames, social buttons and emoji
//! stickers. Clicking one adds it to the middle of the open composition at the playhead,
//! selected, ready to move, recolor and animate.
//!
//! Each tile shows the element itself, rendered a few at a time so the panel opens at once.

use gpui_kit::{
    Context, ImageSource, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _, Window, div, img, px,
};
use studio_core::elements::{ELEMENTS, Group};

use super::chrome::panel;
use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, Workspace};

/// Tile size, in points.
const TILE: f32 = 84.0;
/// Pictures made per redraw while the panel fills in.
const PICTURES_PER_FRAME: usize = 8;

/// Renders a few missing tile pictures, and asks for another frame while any are left. Called
/// from the workspace's render while the panel is open.
pub fn make_pictures(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let missing: Vec<usize> = (0..ELEMENTS.len()).filter(|index| !workspace.ui.element_pictures.contains_key(index)).collect();
    for &index in missing.iter().take(PICTURES_PER_FRAME) {
        match workspace.render.element_picture(index, (TILE * 2.0) as u32) {
            Some(picture) => {
                workspace.ui.element_pictures.insert(index, picture);
            }
            // Without a GPU there is nothing to show; stop asking.
            None => return,
        }
    }
    if missing.len() > PICTURES_PER_FRAME {
        cx.on_next_frame(window, |_, _, cx| cx.notify());
    }
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let mut list = div()
        .id("elements-list")
        .size_full()
        .overflow_y_scroll()
        .p_2()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Click to add to the middle of the frame, at the playhead."));
    for group in Group::ALL {
        let tiles = ELEMENTS.iter().enumerate().filter(|(_, element)| element.group == group).map(|(index, element)| {
            let picture = workspace.ui.element_pictures.get(&index).cloned();
            div()
                .id(("element", index))
                .w(px(TILE))
                .flex()
                .flex_col()
                .gap_0p5()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.state.add_element(index).is_empty() {
                        cx.notify();
                    }
                }))
                .child(
                    div()
                        .size(px(TILE))
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .bg(hsla(color::INSET))
                        .border_1()
                        .border_color(hsla(color::HAIRLINE))
                        .hover(|style| style.border_color(hsla(color::ACCENT)))
                        .children(picture.map(|picture| img(ImageSource::Render(picture)).size_full())),
                )
                .child(div().text_size(px(10.5)).text_color(hsla(color::MUTED)).truncate().child(element.name))
                .test_support()
        });
        list = list.child(div().pt_2().pb_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(group.name())).child(div().flex().flex_wrap().gap_1p5().children(tiles));
    }
    panel(PanelId::Elements, &["Elements"], None, list, workspace, cx)
}
