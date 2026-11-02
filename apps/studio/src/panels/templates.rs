//! The Templates panel, opened from the rail: 225 animated templates for social media and
//! marketing (see `studio_core::templates`), browsed by shape (16:9, 4:5, 9:16) and by what
//! they are for. Clicking one adds it to the project as a new composition and opens it.
//! View All opens a gallery over the editor with every template at a larger size, grouped by
//! what they are for.
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
        let template = branded(workspace, index);
        if let Some(picture) = workspace.render.template_picture(&template, format, picture_seconds(template.seconds), width, height) {
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

/// Template `index`, in the chosen brand's look if one is chosen.
fn branded(workspace: &Workspace, index: usize) -> studio_core::templates::Template {
    match workspace.ui.template_brand.and_then(|brand| workspace.state.libraries.brand(brand)) {
        Some(brand) => TEMPLATES[index].branded(brand),
        None => TEMPLATES[index],
    }
}

/// The brand menu: the template's own look, or a Library brand's colors and font.
fn brand_menu(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    use gpui_kit::component::Sizable as _;
    use gpui_kit::component::button::Button;
    use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
    let current = workspace.ui.template_brand;
    let brands: Vec<(studio_core::model::LinkId, String)> = workspace.state.libraries.brands.iter().map(|brand| (brand.id, brand.name.clone())).collect();
    let label = current.and_then(|id| brands.iter().find(|(brand, _)| *brand == id)).map_or_else(|| "Template colors".to_owned(), |(_, name)| name.clone());
    let target = cx.weak_entity();
    Button::new("template-brand").outline().xsmall().label(label).dropdown_caret(true).dropdown_menu(move |menu, _, _| {
        let choices = std::iter::once((None, "Template colors".to_owned())).chain(brands.iter().map(|(id, name)| (Some(*id), name.clone())));
        choices.fold(menu, |menu, (brand, name)| {
            let target = target.clone();
            menu.item(PopupMenuItem::new(name).checked(brand == current).on_click(move |_, _, cx| {
                target
                    .update(cx, |this, cx| {
                        this.ui.template_brand = brand;
                        // The pictures show the old look: make them again.
                        this.ui.template_pictures.clear();
                        cx.notify();
                    })
                    .ok();
            }))
        })
    })
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

/// The 16:9, 4:5 and 9:16 chips. `prefix` keeps the panel's and the gallery's ids apart.
fn format_chips(workspace: &Workspace, prefix: &'static str, cx: &mut Context<Workspace>) -> gpui_kit::Div {
    let format = workspace.ui.template_format;
    div().flex().gap_1().children(Format::ALL.into_iter().enumerate().map(|(index, choice)| {
        chip((prefix, index), choice.label(), choice == format)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.ui.template_format = choice;
                cx.notify();
            }))
            .test_support()
    }))
}

/// "All" and a chip per category. `prefix` keeps the panel's and the gallery's ids apart.
fn category_chips(workspace: &Workspace, prefix: &'static str, cx: &mut Context<Workspace>) -> Vec<gpui_kit::AnyElement> {
    let current = workspace.ui.template_category;
    let mut categories = vec![chip((prefix, usize::MAX), "All", current.is_none())
        .on_click(cx.listener(|this, _, _, cx| {
            this.ui.template_category = None;
            cx.notify();
        }))
        .test_support()
        .into_any_element()];
    categories.extend(Category::ALL.into_iter().enumerate().map(|(index, category)| {
        chip((prefix, index), category.name(), current == Some(category))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.ui.template_category = Some(category);
                cx.notify();
            }))
            .test_support()
            .into_any_element()
    }));
    categories
}

/// One template's card, `width` points wide: its picture, name, and what it is for. Clicking it
/// adds the template (and closes the gallery, for `in_gallery` cards).
fn card(workspace: &Workspace, index: usize, width: f32, in_gallery: bool, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let format = workspace.ui.template_format;
    let template = &TEMPLATES[index];
    let picture = workspace.ui.template_pictures.get(&(index, format)).cloned();
    let height = card_height(format) * width / CARD_WIDTH;
    div()
        .id((if in_gallery { "gallery-template" } else { "template" }, index))
        .w(px(width))
        .flex()
        .flex_col()
        .gap_1()
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| {
            if this.state.add_template(index, this.ui.template_format, this.ui.template_brand).is_some() {
                if in_gallery {
                    this.ui.template_gallery = false;
                }
                cx.notify();
            }
        }))
        .child(
            div()
                .w(px(width))
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
}

/// Card width in the gallery, in points.
const GALLERY_CARD_WIDTH: f32 = 168.0;

/// Every template at once, over the editor: the panel's shape, category and brand choices,
/// with the templates grouped under their categories. Clicking one adds it and closes the
/// gallery; the close button, a click outside it, or Escape closes it.
pub fn gallery(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<gpui_kit::AnyElement> {
    use gpui_kit::assets::IconName;
    use gpui_kit::component::Icon;
    use gpui_kit::{IntoElement as _, black, deferred};
    if !workspace.ui.template_gallery {
        return None;
    }
    let shown = shown(workspace);
    let close = div()
        .id("gallery-close")
        .size(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .text_color(hsla(color::MUTED))
        .hover(|style| style.bg(hsla(color::HOVER)).text_color(hsla(color::TEXT)))
        .on_click(cx.listener(|this, _, _, cx| {
            this.ui.template_gallery = false;
            cx.notify();
        }))
        .child(Icon::new(IconName::X).size(px(16.0)))
        .test_support();
    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            div()
                .flex()
                .items_baseline()
                .gap_2()
                .child(div().text_size(px(16.0)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child("All Templates"))
                .child(div().text_color(hsla(color::MUTED)).child(format!("{} templates", shown.len()))),
        )
        .child(div().flex().items_center().gap_2().child(format_chips(workspace, "gallery-format", cx)).child(brand_menu(workspace, cx)).child(close));
    let categories = div().flex().flex_wrap().gap_1().children(category_chips(workspace, "gallery-category", cx));
    // Grouped under their categories, in the order the chips list them.
    let groups: Vec<_> = Category::ALL
        .into_iter()
        .filter_map(|category| {
            let members: Vec<usize> = shown.iter().copied().filter(|&index| TEMPLATES[index].category == category).collect();
            (!members.is_empty()).then(|| {
                let cards: Vec<_> = members.iter().map(|&index| card(workspace, index, GALLERY_CARD_WIDTH, true, cx)).collect();
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(format!("{} · {}", category.name(), members.len())))
                    .child(div().flex().flex_wrap().gap_3().children(cards))
            })
        })
        .collect();
    let list = div().id("gallery-list").flex_1().min_h_0().overflow_y_scroll().pr_2().flex().flex_col().gap_5().children(groups);
    let sheet = div()
        .id("template-gallery")
        .size_full()
        .p_4()
        .flex()
        .flex_col()
        .gap_3()
        .rounded(px(12.0))
        .bg(hsla(color::POPOVER))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .shadow_lg()
        .text_size(px(size::TEXT))
        .text_color(hsla(color::TEXT))
        // Clicks inside stay inside; a click on the dimmed edge closes it.
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(header)
        .child(categories)
        .child(list)
        .test_support();
    let backdrop = div()
        .id("gallery-backdrop")
        .absolute()
        .inset_0()
        .p(px(40.0))
        .bg(black().opacity(0.5))
        .occlude()
        .on_mouse_down(gpui_kit::MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.ui.template_gallery = false;
            cx.notify();
        }))
        .child(sheet);
    Some(deferred(backdrop).with_priority(3).into_any_element())
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let formats = format_chips(workspace, "template-format", cx);
    let categories = category_chips(workspace, "template-category", cx);
    let brand = brand_menu(workspace, cx);
    let cards: Vec<_> = shown(workspace).into_iter().map(|index| card(workspace, index, CARD_WIDTH, false, cx)).collect();
    let view_all = {
        use gpui_kit::component::Sizable as _;
        use gpui_kit::component::button::{Button, ButtonVariants as _};
        Button::new("templates-view-all").ghost().xsmall().label("View All Templates").on_click(cx.listener(|this, _, _, cx| {
            this.ui.template_gallery = true;
            cx.notify();
        }))
    };
    let body = div()
        .id("templates-list")
        .size_full()
        .overflow_y_scroll()
        .p_2()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().flex().items_center().justify_between().gap_2().child(formats).child(brand))
        .child(div().flex().flex_wrap().gap_1().children(categories))
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Click a template to add it as a new composition. Gray boxes are where your footage and photos go."))
        .child(div().flex().flex_wrap().gap_2().children(cards));
    // View All along the bottom, always in sight however far the list is scrolled.
    let footer = div()
        .flex()
        .items_center()
        .justify_between()
        .w_full()
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(format!("{} templates", TEMPLATES.len())))
        .child(view_all);
    panel(PanelId::Templates, &["Templates"], Some(footer.into_any_element()), body, workspace, cx)
}
