//! The Libraries panel, opened from the rail: brands (folders) holding colors and text styles
//! that every project can use. Clicking a color or text style uses it on the selected layers,
//! linked, so changing it here later changes it everywhere it is used. A color's chip opens the
//! color wheel to change the library color itself.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, WindowExt as _};
use gpui_kit::{
    ClickEvent, Context, Div, ElementId, InteractiveElement, IntoElement, MouseButton, ParentElement, StatefulInteractiveElement, Styled,
    TestSupportExt as _, Window, div, px, rgba,
};
use studio_core::library::Brand;
use studio_core::model::{LayerContent, LinkId, Rgba};

use super::chrome::{hint, icon_button, panel};
use super::color_picker;
use crate::state::ColorTarget;
use crate::theme::{color, hsla, size};
use crate::workspace::{EditTarget, PanelId, Workspace};

/// A name that can be renamed in place: double-click it to type a new one.
pub(crate) fn name_label(id: LinkId, name: &str, workspace: &Workspace, cx: &mut Context<Workspace>) -> gpui_kit::AnyElement {
    if let Some(input) = workspace.ui.editing.as_ref().filter(|editing| editing.target == EditTarget::LibraryName(id)).and_then(|editing| editing.input.line()) {
        return div().flex_1().child(Input::new(input).xsmall()).into_any_element();
    }
    let current = name.to_owned();
    div()
        .id(ElementId::Name(format!("library-name:{}", id.0).into()))
        .flex_1()
        .min_w_0()
        .truncate()
        .child(name.to_owned())
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            if event.click_count() >= 2 {
                cx.stop_propagation();
                this.begin_edit(EditTarget::LibraryName(id), current.clone(), window, cx);
            }
        }))
        .test_support()
        .into_any_element()
}

/// A small trash button that deletes one color or text style.
fn delete_button(id: LinkId, cx: &mut Context<Workspace>) -> impl IntoElement {
    icon_button(ElementId::Name(format!("library-delete:{}", id.0).into()), IconName::Trash, "Delete", false, cx.listener(move |this, _, _, cx| {
        this.state.delete_library_item(id);
        cx.notify();
    }))
    .xsmall()
}

/// Asks before deleting a brand and everything in it.
fn confirm_delete_brand(brand: &Brand, window: &mut Window, cx: &mut Context<Workspace>) {
    let (id, name) = (brand.id, brand.name.clone());
    let count = brand.colors.len() + brand.text_styles.len();
    let workspace = cx.weak_entity();
    window.open_alert_dialog(cx, move |dialog, _, _| {
        let workspace = workspace.clone();
        let contents = match count {
            0 => "It is empty.".to_owned(),
            1 => "Its 1 color or text style goes with it.".to_owned(),
            n => format!("Its {n} colors and text styles go with it."),
        };
        dialog
            .title(format!("Delete “{name}”?"))
            .description(format!("{contents} Projects that use them keep their look. A backup of your libraries is kept in Studio's settings folder."))
            .ok_text("Delete")
            .show_cancel(true)
            .on_ok(move |_, _, cx| {
                workspace
                    .update(cx, |this, cx| {
                        this.state.delete_library_item(id);
                        cx.notify();
                    })
                    .ok();
                true
            })
    });
}

/// The color a new library color starts from: the selected layer's fill, or white.
fn selection_color(workspace: &Workspace) -> Rgba {
    let state = &workspace.state;
    let layer = state.selected_layers.first().and_then(|&id| state.composition().layer(id));
    let color = match layer.map(|layer| &layer.content) {
        Some(LayerContent::Shape { style, .. }) => style.fill.map(|paint| paint.colors()[0]),
        Some(LayerContent::Text { style, .. }) => style.fill,
        Some(LayerContent::Solid { color }) => Some(*color),
        _ => None,
    };
    color.unwrap_or(Rgba::hex(0xffffff)).linked(None)
}

/// A subheading inside a brand ("Colors", "Text Styles") with its add button.
fn subheading(title: &'static str, add: impl IntoElement) -> Div {
    div()
        .h(px(26.0))
        .pl(px(30.0))
        .pr_1()
        .flex()
        .items_center()
        .text_size(px(size::SMALL_TEXT))
        .text_color(hsla(color::MUTED))
        .child(div().flex_1().child(title))
        .child(add)
}

fn brand_section(brand: &Brand, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = brand.id;
    let open = !workspace.ui.closed_brands.contains(&id);
    let header = div()
        .id(ElementId::Name(format!("library-brand:{}", id.0).into()))
        .group("brand")
        .h(px(32.0))
        .mx_1()
        .px_1p5()
        .flex()
        .items_center()
        .gap_1p5()
        .rounded(px(6.0))
        .hover(|style| style.bg(hsla(color::HOVER)))
        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            if event.click_count() == 1 && !this.ui.closed_brands.remove(&id) {
                this.ui.closed_brands.insert(id);
            }
            cx.notify();
        }))
        .child(Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).size(px(12.0)).text_color(hsla(color::MUTED)))
        .child(Icon::new(if open { IconName::FolderOpen } else { IconName::Folder }).size(px(15.0)).text_color(hsla(color::MUTED)))
        .child(name_label(id, &brand.name, workspace, cx))
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(format!("{}", brand.colors.len() + brand.text_styles.len())))
        .child({
            let brand = brand.clone();
            icon_button(ElementId::Name(format!("library-delete-brand:{}", id.0).into()), IconName::Trash, "Delete brand and everything in it", false, cx.listener(move |_, _, window, cx| {
                confirm_delete_brand(&brand, window, cx);
            }))
            .xsmall()
        })
        .test_support();
    let mut section = div().child(header);
    if !open {
        return section;
    }

    let add_color = icon_button(ElementId::Name(format!("library-add-color:{}", id.0).into()), IconName::Plus, "Add a color (from the selected layer)", false, cx.listener(move |this, event: &ClickEvent, window, cx| {
        let start = selection_color(this);
        if let Some(color) = this.state.add_library_color(id, start) {
            // Open the wheel on it straight away, to pick the color.
            color_picker::open(this, ColorTarget::Library(color), event.position(), window, cx);
        }
        cx.notify();
    }))
    .xsmall();
    section = section.child(subheading("Colors", add_color));
    if brand.colors.is_empty() {
        section = section.child(div().pl(px(30.0)).pb_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("No colors yet."));
    }
    for item in &brand.colors {
        let color_id = item.id;
        section = section.child(
            div()
                .id(ElementId::Name(format!("library-color:{}", color_id.0).into()))
                .h(px(30.0))
                .mx_1()
                .pl(px(26.0))
                .pr_1()
                .flex()
                .items_center()
                .gap_2()
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|style| style.bg(hsla(color::HOVER)))
                // Using a color: on the selected layers, linked.
                .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                    if event.click_count() == 1 && this.state.apply_library_color(color_id) > 0 {
                        cx.notify();
                    }
                }))
                .child(color_picker::swatch(ElementId::Name(format!("library-color-chip:{}", color_id.0).into()), ColorTarget::Library(color_id), Some(item.color), cx))
                .child(name_label(color_id, &item.name, workspace, cx))
                .child(div().font_family(crate::theme::MONO_FONT).text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(item.color.to_hex()))
                .child(delete_button(color_id, cx))
                .test_support(),
        );
    }

    let text_layer = workspace.state.selected_layers.iter().copied().find(|&layer| workspace.state.text_of(layer).is_some());
    let add_style = icon_button(ElementId::Name(format!("library-add-text:{}", id.0).into()), IconName::Plus, "Save the selected text layer's style", false, cx.listener(move |this, _, _, cx| {
        if let Some(layer) = text_layer {
            this.state.add_text_style_from(id, layer);
            cx.notify();
        }
    }))
    .xsmall()
    .disabled(text_layer.is_none());
    section = section.child(subheading("Text Styles", add_style));
    if brand.text_styles.is_empty() {
        section = section.child(div().pl(px(30.0)).pb_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Select a text layer and press + to save its style."));
    }
    for item in &brand.text_styles {
        let style_id = item.id;
        let fill = item.fill.map_or(0xffffffff, Rgba::to_rgba_u32);
        section = section.child(
            div()
                .id(ElementId::Name(format!("library-text:{}", style_id.0).into()))
                .h(px(40.0))
                .mx_1()
                .pl(px(26.0))
                .pr_1()
                .flex()
                .items_center()
                .gap_2()
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|style| style.bg(hsla(color::HOVER)))
                .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                    let layers = this.state.selected_layers.clone();
                    if event.click_count() == 1 && this.state.apply_text_style(style_id, &layers) > 0 {
                        cx.notify();
                    }
                }))
                // A sample in the style's own font, weight and color.
                .child(
                    div()
                        .w(px(28.0))
                        .flex_none()
                        .text_size(px(16.0))
                        .font_family(item.font_family.clone())
                        .font_weight(gpui_kit::FontWeight(f32::from(item.font_weight)))
                        .text_color(rgba(fill))
                        .child("Aa"),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(name_label(style_id, &item.name, workspace, cx))
                        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).truncate().child(item.summary())),
                )
                .child(delete_button(style_id, cx))
                .test_support(),
        );
    }
    section.child(div().h(px(6.0)))
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let new_brand = div()
        .id("library-new-brand")
        .mx_2()
        .my_2()
        .h(px(30.0))
        .flex()
        .items_center()
        .justify_center()
        .gap_1p5()
        .rounded(px(7.0))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .hover(|style| style.bg(hsla(color::HOVER)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(|this, _, window, cx| {
            let count = this.state.libraries.brands.len();
            let id = this.state.add_brand(&format!("Brand {}", count + 1));
            // Name it straight away.
            let name = this.state.libraries.brand(id).map(|brand| brand.name.clone()).unwrap_or_default();
            this.begin_edit(EditTarget::LibraryName(id), name, window, cx);
            cx.notify();
        }))
        .child(Icon::new(IconName::Plus).size(px(14.0)))
        .child("New Brand")
        .test_support();
    let mut list = div().id("libraries-list").size_full().overflow_y_scroll().pb_2().child(new_brand);
    if workspace.state.libraries.brands.is_empty() {
        list = list.child(hint("A brand holds colors and text styles every project can use. Using one links it: change it here and it changes everywhere."));
    }
    for brand in &workspace.state.libraries.brands {
        list = list.child(brand_section(brand, workspace, cx));
    }
    panel(PanelId::Libraries, &["Libraries"], None, list, workspace, cx)
}
