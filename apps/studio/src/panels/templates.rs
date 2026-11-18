//! The Templates panel, opened from the rail: 73 animated templates for social media, ads and
//! marketing (see `studio_core::templates`), one per design, browsed by shape (16:9, 4:5, 9:16)
//! and by what they are for. The Colors menu makes them in another palette or a brand's look.
//! Hovering over a card plays the template on it; its + button adds it to the project as a new
//! composition, opens it and says so in a notification.
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
use crate::state::TemplateLook;
use gpui_kit::RenderImage;
use std::sync::Arc;
use crate::workspace::{PanelId, Workspace};

/// A template playing on its card while the pointer is over it: built once into a scratch
/// project, then drawn a frame at a time.
pub struct TemplatePreview {
    pub index: usize,
    format: Format,
    look: TemplateLook,
    project: studio_core::model::Project,
    comp: studio_core::model::Id,
    duration: i64,
    started: std::time::Instant,
    /// The frame last drawn, and its picture.
    pub shown: Option<(i64, Arc<RenderImage>)>,
    /// Whether a redraw for the next frame is already on its way.
    tick_pending: bool,
}

/// Pictures a second while a template plays on its card: smooth enough to judge its motion,
/// and light on the GPU.
const PREVIEW_RATE: f64 = 15.0;

/// The template frame to show `elapsed` seconds into a hover, at `PREVIEW_RATE` (templates run at
/// 30 frames a second), starting over at the end.
fn preview_frame(elapsed: f64, duration: i64) -> i64 {
    let step = (30.0 / PREVIEW_RATE) as i64;
    ((elapsed * PREVIEW_RATE).floor() as i64 * step).rem_euclid(duration.max(1))
}

/// Starts (or, with `hovered` false, stops) playing template `index` on its card.
fn hover(workspace: &mut Workspace, index: usize, hovered: bool) {
    if !hovered {
        if workspace.ui.template_preview.as_ref().is_some_and(|preview| preview.index == index) {
            workspace.ui.template_preview = None;
        }
        return;
    }
    let (format, look) = (workspace.ui.template_format, workspace.ui.template_look);
    let Some(template) = workspace.state.styled_template(index, look) else { return };
    let Some((project, comp)) = super::viewer::scratch_template(&template, format) else { return };
    let duration = project.composition(comp).map_or(1, |composition| composition.duration_frames);
    workspace.ui.template_preview = Some(TemplatePreview { index, format, look, project, comp, duration, started: std::time::Instant::now(), shown: None, tick_pending: false });
}

/// Draws the hovered template's current frame when it is due, and asks for a redraw in time for
/// the next one. Called with the other pictures while the templates are on screen.
fn keep_preview_playing(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let (format, look) = (workspace.ui.template_format, workspace.ui.template_look);
    // A new shape or colors restarts it in those.
    if let Some(index) = workspace.ui.template_preview.as_ref().filter(|preview| preview.format != format || preview.look != look).map(|preview| preview.index) {
        hover(workspace, index, true);
    }
    let Some(preview) = workspace.ui.template_preview.as_mut() else { return };
    let frame = preview_frame(preview.started.elapsed().as_secs_f64(), preview.duration);
    if preview.shown.as_ref().is_none_or(|(shown, _)| *shown != frame) {
        let (width, height) = ((CARD_WIDTH * 2.0) as u32, (card_height(format) * 2.0) as u32);
        let picture = workspace.render.template_frame(&mut preview.project, preview.comp, frame, width, height);
        let Some(picture) = picture else { return };
        preview.shown = Some((frame, picture));
    }
    if !preview.tick_pending {
        preview.tick_pending = true;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_secs_f64(1.0 / PREVIEW_RATE)).await;
            this.update(cx, |this, cx| {
                if let Some(preview) = this.ui.template_preview.as_mut() {
                    preview.tick_pending = false;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}

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
    // The start screen's sample projects need pictures whatever category is chosen.
    let wanted: Vec<usize> = if workspace.ui.start_screen.is_some() { samples().into_iter().chain(shown(workspace)).collect() } else { shown(workspace) };
    let missing: Vec<usize> = wanted.into_iter().filter(|&index| !workspace.ui.template_pictures.contains_key(&(index, format))).collect();
    keep_preview_playing(workspace, window, cx);
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

/// Template `index`, in the colors chosen in the Colors menu.
fn branded(workspace: &Workspace, index: usize) -> studio_core::templates::Template {
    workspace.state.styled_template(index, workspace.ui.template_look).unwrap_or(TEMPLATES[index])
}

/// The Colors menu: each template's own colors, one of the template palettes (with its
/// colors as a swatch), or a Library brand's colors and font.
fn brand_menu(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    use gpui_kit::assets::IconName;
    use gpui_kit::component::Icon;
    use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
    use studio_core::templates::LOOKS;
    let current = workspace.ui.template_look;
    let brands: Vec<(studio_core::model::LinkId, String)> = workspace.state.libraries.brands.iter().map(|brand| (brand.id, brand.name.clone())).collect();
    let label = match current {
        TemplateLook::Own => "Template colors".to_owned(),
        TemplateLook::Palette(index) => LOOKS.get(index).map_or_else(|| "Template colors".to_owned(), |(name, _)| (*name).to_owned()),
        TemplateLook::Brand(id) => brands.iter().find(|(brand, _)| *brand == id).map_or_else(|| "Template colors".to_owned(), |(_, name)| name.clone()),
    };
    let target = cx.weak_entity();
    crate::panels::chrome::dropdown("template-brand", label).dropdown_menu(move |menu, _, _| {
        let pick = |look: TemplateLook| {
            let target = target.clone();
            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
                target
                    .update(cx, |this, cx| {
                        this.ui.template_look = look;
                        // The pictures show the old colors: make them again.
                        this.ui.template_pictures.clear();
                        cx.notify();
                    })
                    .ok();
            }
        };
        let mut menu = menu.item(PopupMenuItem::new("Template colors").checked(current == TemplateLook::Own).on_click(pick(TemplateLook::Own))).separator();
        for (index, (name, palette)) in LOOKS.iter().enumerate() {
            let swatch = Icon::new(IconName::Circle).text_color(gpui_kit::rgb(palette.accent));
            let look = TemplateLook::Palette(index);
            menu = menu.item(PopupMenuItem::new(*name).icon(swatch).checked(current == look).on_click(pick(look)));
        }
        if !brands.is_empty() {
            menu = menu.separator();
        }
        for (id, name) in &brands {
            let look = TemplateLook::Brand(*id);
            menu = menu.item(PopupMenuItem::new(name.clone()).checked(current == look).on_click(pick(look)));
        }
        menu
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

/// Where a template card is shown, which decides what clicking it does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CardPlace {
    /// The Templates panel: adds the template to the project.
    Panel,
    /// The gallery over the editor: adds it, and closes the gallery.
    Gallery,
    /// The start screen: starts a new project from it.
    StartScreen,
}

/// A card's + button: adds the template to the project (or, on the start screen, starts a
/// new project from it).
fn add_button(index: usize, place: CardPlace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    use gpui_kit::assets::IconName;
    use gpui_kit::component::Icon;
    use gpui_kit::component::Sizable as _;
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    let (id, tip) = match place {
        CardPlace::Panel => ("template-add", "Add to project"),
        CardPlace::Gallery => ("gallery-template-add", "Add to project"),
        CardPlace::StartScreen => ("start-template-add", "Start a project from this template"),
    };
    Button::new((id, index)).icon(Icon::new(IconName::Plus)).small().primary().tooltip(tip).on_click(cx.listener(move |this, _, window, cx| add(this, index, place, window, cx)))
}

/// Adds template `index` from a card in `place`, then says so.
fn add(workspace: &mut Workspace, index: usize, place: CardPlace, window: &mut Window, cx: &mut Context<Workspace>) {
    use gpui_kit::component::WindowExt as _;
    if place == CardPlace::StartScreen {
        super::start_screen::new_from_template(workspace, index, window, cx);
        return;
    }
    let name = TEMPLATES[index].name;
    if workspace.state.add_template(index, workspace.ui.template_format, workspace.ui.template_look).is_some() {
        if place == CardPlace::Gallery {
            workspace.ui.template_gallery = false;
        }
        window.push_notification(format!("Added “{name}” to the project.").as_str(), cx);
    } else {
        window.push_notification(format!("“{name}” couldn’t be added.").as_str(), cx);
    }
    cx.notify();
}

/// One template's card, `width` points wide: its picture (playing while the pointer is over it),
/// name, and what it is for, with the + button that adds it.
fn card(workspace: &Workspace, index: usize, width: f32, place: CardPlace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let format = workspace.ui.template_format;
    let template = &TEMPLATES[index];
    let playing = workspace.ui.template_preview.as_ref().filter(|preview| preview.index == index && preview.format == format).and_then(|preview| preview.shown.as_ref()).map(|(_, picture)| picture.clone());
    let picture = playing.or_else(|| workspace.ui.template_pictures.get(&(index, format)).cloned());
    let height = card_height(format) * width / CARD_WIDTH;
    div()
        .id((
            match place {
                CardPlace::Panel => "template",
                CardPlace::Gallery => "gallery-template",
                CardPlace::StartScreen => "start-template",
            },
            index,
        ))
        .w(px(width))
        .flex()
        .flex_col()
        .gap_1()
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            hover(this, index, *hovered);
            cx.notify();
        }))
        .child(
            div()
                .relative()
                .w(px(width))
                .h(px(height))
                .rounded(px(6.0))
                .overflow_hidden()
                .bg(hsla(color::INSET))
                .border_1()
                .border_color(hsla(color::HAIRLINE))
                .hover(|style| style.border_color(hsla(color::ACCENT)))
                .children(picture.map(|picture| img(ImageSource::Render(picture)).size_full()))
                .child(div().absolute().top(px(6.0)).right(px(6.0)).child(add_button(index, place, cx))),
        )
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::TEXT)).truncate().child(template.name))
        .child(div().text_size(px(size::CAPTION_TEXT)).text_color(hsla(color::MUTED)).child(format!("{} · {:.0}s", template.category.name(), template.seconds)))
        .test_support()
}

/// Card width in the gallery, in points.
const GALLERY_CARD_WIDTH: f32 = 168.0;

/// Every template at once, over the editor: the panel's shape, category and brand choices,
/// with the templates grouped under their categories. A template's + button adds it and closes the
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
                .child(div().text_size(px(size::TITLE_TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child("All Templates"))
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
                let cards: Vec<_> = members.iter().map(|&index| card(workspace, index, GALLERY_CARD_WIDTH, CardPlace::Gallery, cx)).collect();
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
    let sheet = super::chrome::popup_surface(div().id("template-gallery"))
        .size_full()
        .p_4()
        .flex()
        .flex_col()
        .gap_3()
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

/// The start screen's Templates page body: the shape, brand and category choices over every
/// shown template at gallery size. Clicking one starts a new project from it.
pub fn start_screen_cards(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let cards: Vec<_> = shown(workspace).into_iter().map(|index| card(workspace, index, GALLERY_CARD_WIDTH, CardPlace::StartScreen, cx)).collect();
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .gap_3()
        .child(div().flex().items_center().gap_2().child(format_chips(workspace, "start-format", cx)).child(brand_menu(workspace, cx)))
        .child(div().flex().flex_wrap().gap_1().children(category_chips(workspace, "start-category", cx)))
        .child(div().id("start-template-list").flex_1().min_h_0().overflow_y_scroll().pr_2().child(div().flex().flex_wrap().gap_4().children(cards)))
}

/// The templates the start screen's Home page offers as sample projects, by name: one each of
/// a launch, a sale, a vlog title and a product announcement, so the row shows the range.
const SAMPLE_NAMES: [&str; 4] = ["Launch Day", "Half Off", "Vlog Intro", "New Feature"];

/// Where the sample projects are in `TEMPLATES`.
fn samples() -> Vec<usize> {
    SAMPLE_NAMES.iter().filter_map(|name| TEMPLATES.iter().position(|template| template.name == *name)).collect()
}

/// Sizes of a sample project's tile on the start screen, in points: the tile's height, and
/// the height of the template's picture standing inside it.
const SAMPLE_TILE_HEIGHT: f32 = 300.0;
const SAMPLE_PICTURE_HEIGHT: f32 = 252.0;

/// One sample project on the start screen: the template's picture standing on a tile in its
/// accent color, with its name underneath. Pointing at it plays the template; clicking it
/// starts a new project from it.
fn sample_card(workspace: &Workspace, index: usize, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let format = workspace.ui.template_format;
    let template = branded(workspace, index);
    let playing = workspace.ui.template_preview.as_ref().filter(|preview| preview.index == index && preview.format == format).and_then(|preview| preview.shown.as_ref()).map(|(_, picture)| picture.clone());
    let picture = playing.or_else(|| workspace.ui.template_pictures.get(&(index, format)).cloned());
    let (width, height) = format.size();
    let picture_width = SAMPLE_PICTURE_HEIGHT * width as f32 / height as f32;
    div()
        .id(("start-template", index))
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_2()
        .cursor_pointer()
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            hover(this, index, *hovered);
            cx.notify();
        }))
        .on_click(cx.listener(move |this, _, window, cx| add(this, index, CardPlace::StartScreen, window, cx)))
        .child(
            div()
                .h(px(SAMPLE_TILE_HEIGHT))
                .rounded(px(12.0))
                .overflow_hidden()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui_kit::rgb(template.palette.accent))
                .border_1()
                .border_color(hsla(color::HAIRLINE))
                .hover(|style| style.border_color(hsla(color::ACCENT)))
                .child(
                    div()
                        .w(px(picture_width))
                        .h(px(SAMPLE_PICTURE_HEIGHT))
                        .max_w_full()
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .shadow_lg()
                        .bg(gpui_kit::rgb(template.palette.background))
                        .children(picture.map(|picture| img(ImageSource::Render(picture)).size_full())),
                ),
        )
        .child(div().text_color(hsla(color::MUTED)).truncate().child(template.name))
        .test_support()
}

/// The start screen's sample projects: a row of large tiles, one for each of `SAMPLE_NAMES`,
/// sharing the page's width. Each starts a new project from its template, like the Templates
/// page's cards.
pub fn start_screen_strip(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let cards: Vec<_> = samples().into_iter().map(|index| sample_card(workspace, index, cx)).collect();
    div().id("start-template-strip").flex_none().flex().gap_4().children(cards).test_support()
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let formats = format_chips(workspace, "template-format", cx);
    let categories = category_chips(workspace, "template-category", cx);
    let brand = brand_menu(workspace, cx);
    let cards: Vec<_> = shown(workspace).into_iter().map(|index| card(workspace, index, CARD_WIDTH, CardPlace::Panel, cx)).collect();
    let view_all = {
        use gpui_kit::component::Sizable as _;
        use gpui_kit::component::button::{Button, ButtonVariants as _};
        Button::new("templates-view-all").ghost().small().label("View All Templates").on_click(cx.listener(|this, _, _, cx| {
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
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Point at a template to play it; + adds it as a new composition. Gray boxes are where your footage and photos go."))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hovered_template_plays_at_preview_rate_and_starts_over() {
        assert_eq!(preview_frame(0.0, 300), 0);
        // Half a second in: the template's frame 15, shown as the nearest preview picture.
        assert_eq!(preview_frame(0.5, 300), 14);
        assert_eq!(preview_frame(10.0, 300), 0, "a 10-second template starts over");
        assert_eq!(preview_frame(10.5, 300), 14);
    }

    #[test]
    fn every_sample_project_is_a_real_template() {
        assert_eq!(samples().len(), SAMPLE_NAMES.len());
    }
}
