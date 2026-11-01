//! The Templates panel, opened from the rail: 100 animated templates for social media and
//! marketing (see `studio_core::templates`), browsed by shape (16:9, 4:5, 9:16) and by what
//! they are for. Clicking one adds it to the project as a new composition and opens it.
//!
//! Each card shows the template itself, rendered a few at a time so the panel opens at once.

use gpui_kit::{
    Context, ImageSource, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _, Window, div, img,
    prelude::FluentBuilder as _, px,
};
use studio_core::templates::{Category, Format, TEMPLATES};

use super::chrome::panel;
use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, Workspace};

/// Card picture widths, in points (the height follows the format).
const CARD_WIDTH: f32 = 128.0;
/// Pictures made per redraw while the panel fills in.
const PICTURES_PER_FRAME: usize = 4;

/// Where in each template its picture is taken: most of the way in, when its parts have arrived.
fn picture_seconds(seconds: f64) -> f64 {
    (seconds * 0.55).min(7.5)
}

fn card_height(format: Format) -> f32 {
    let (width, height) = format.size();
    CARD_WIDTH * height as f32 / width as f32
}

/// The templates the panel shows: those in the chosen category (or all).
fn shown(workspace: &Workspace) -> Vec<usize> {
    let category = workspace.ui.template_category;
    (0..TEMPLATES.len()).filter(|&index| category.is_none_or(|category| TEMPLATES[index].category == category)).collect()
}

/// Renders a few of the shown templates' pictures that are missing, and asks for another
/// frame while any are left. Called from the workspace's render while the panel is open.
pub fn make_pictures(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let format = workspace.ui.template_format;
    let missing: Vec<usize> = shown(workspace).into_iter().filter(|&index| !workspace.ui.template_pictures.contains_key(&(index, format))).collect();
    if missing.is_empty() {
        return;
    }
    // At twice the card's size, for sharp pictures on high-density screens.
    let (width, height) = ((CARD_WIDTH * 2.0) as u32, (card_height(format) * 2.0) as u32);
    for &index in missing.iter().take(PICTURES_PER_FRAME) {
        let template = &TEMPLATES[index];
        if let Some(picture) = workspace.render.template_picture(template, format, picture_seconds(template.seconds), width, height) {
            workspace.ui.template_pictures.insert((index, format), picture);
        } else {
            // Without a GPU there is nothing to show; stop asking.
            return;
        }
    }
    if missing.len() > PICTURES_PER_FRAME {
        cx.on_next_frame(window, |_, _, cx| cx.notify());
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

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let format = workspace.ui.template_format;
    let formats = div().flex().gap_1().children(Format::ALL.into_iter().enumerate().map(|(index, choice)| {
        chip(("template-format", index), choice.label(), choice == format)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.ui.template_format = choice;
                cx.notify();
            }))
            .test_support()
    }));
    let current = workspace.ui.template_category;
    let mut categories = vec![chip("template-category-all", "All", current.is_none())
        .on_click(cx.listener(|this, _, _, cx| {
            this.ui.template_category = None;
            cx.notify();
        }))
        .test_support()];
    categories.extend(Category::ALL.into_iter().enumerate().map(|(index, category)| {
        chip(("template-category", index), category.name(), current == Some(category))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.ui.template_category = Some(category);
                cx.notify();
            }))
            .test_support()
    }));
    let height = card_height(format);
    let cards = shown(workspace).into_iter().map(|index| {
        let template = &TEMPLATES[index];
        let picture = workspace.ui.template_pictures.get(&(index, format)).cloned();
        div()
            .id(("template", index))
            .w(px(CARD_WIDTH))
            .flex()
            .flex_col()
            .gap_1()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.state.add_template(index, this.ui.template_format).is_some() {
                    cx.notify();
                }
            }))
            .child(
                div()
                    .w(px(CARD_WIDTH))
                    .h(px(height))
                    .rounded(px(6.0))
                    .overflow_hidden()
                    .bg(hsla(color::INSET))
                    .border_1()
                    .border_color(hsla(color::HAIRLINE))
                    .hover(|style| style.border_color(hsla(color::ACCENT)))
                    .children(picture.map(|picture| img(ImageSource::Render(picture)).size_full())),
            )
            .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::TEXT)).truncate().child(template.name))
            .child(div().text_size(px(10.0)).text_color(hsla(color::MUTED)).child(format!("{} · {:.0}s", template.category.name(), template.seconds)))
            .test_support()
    });
    let body = div()
        .id("templates-list")
        .size_full()
        .overflow_y_scroll()
        .p_2()
        .flex()
        .flex_col()
        .gap_2()
        .child(formats)
        .child(div().flex().flex_wrap().gap_1().children(categories))
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Click a template to add it as a new composition. Gray boxes are where your footage and photos go."))
        .child(div().flex().flex_wrap().gap_2().children(cards));
    panel(PanelId::Templates, &["Templates"], None, body, workspace, cx)
}
