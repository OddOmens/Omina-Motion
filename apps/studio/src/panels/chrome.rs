//! Shared panel pieces: the panel frame every dockable panel uses, the small
//! icon buttons found in toolbars and panel footers, and a bounds probe.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, Selectable as _, Sizable as _};
use gpui_kit::{
    AnyElement, App, Bounds, ClickEvent, Context, Div, ElementId, InteractiveElement, IntoElement, ParentElement,
    Pixels, Render, SharedString, StatefulInteractiveElement, Styled, TestSupportExt as _, Window, canvas, div, prelude::FluentBuilder as _,
    px,
};

use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, Workspace};

/// A panel's frame: the body, an optional footer and, for panels with several tabs, a header
/// with those tabs (clicking one switches the panel to it; see `UiState::tab`). The panel's own
/// title is on its tab in the dock (see `panels::dock`), so a panel with a single title needs no
/// header of its own. Pressing anywhere inside makes it the active panel (which decides what
/// Delete acts on).
pub fn panel(
    id: PanelId,
    tabs: &[&str],
    footer: Option<AnyElement>,
    body: impl IntoElement,
    workspace: &Workspace,
    cx: &mut Context<Workspace>,
) -> Div {
    let active_tab = workspace.ui.tab(id);
    let header = (tabs.len() > 1).then(|| {
        div()
            .h(px(size::PANEL_HEADER_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .border_b_1()
            .border_color(hsla(color::HAIRLINE))
            // Tabs share what room there is, a long title shortened with an ellipsis.
            .child(div().flex_1().min_w_0().h_full().flex().items_center().gap_1().overflow_hidden().children(tabs.iter().enumerate().map(|(index, title)| {
                let selected = index == active_tab;
                div()
                    .id(ElementId::Name(format!("tab:{id:?}:{index}").into()))
                    .h_full()
                    .min_w_0()
                    .flex_shrink(1.0)
                    .px_1p5()
                    .flex()
                    .items_center()
                    .text_color(hsla(if selected { color::TEXT } else { color::MUTED }))
                    .when(selected, |tab| tab.border_b_2().border_color(hsla(color::TEXT)))
                    .hover(|style| style.text_color(hsla(color::TEXT)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ui.tabs.insert(id, index);
                        cx.notify();
                    }))
                    .child(div().min_w_0().truncate().child(SharedString::from(title.to_string())))
                    .test_support()
            })))
    });

    div()
        .size_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(hsla(color::PANEL))
        .capture_any_mouse_down(cx.listener(move |this, _, _, cx| {
            if this.ui.active_panel != id {
                this.ui.active_panel = id;
                cx.notify();
            }
        }))
        .children(header)
        .child(div().flex_1().min_h_0().relative().child(body))
        .children(footer.map(|footer| {
            div()
                .h(px(size::PANEL_FOOTER_HEIGHT))
                .flex_none()
                .flex()
                .items_center()
                .gap_1()
                .px_1p5()
                .border_t_1()
                .border_color(hsla(color::HAIRLINE))
                .child(footer)
        }))
}

/// A section heading inside a panel, with optional controls on the right (as in "Layer Transform   Reset").
pub fn section(title: impl Into<SharedString>, trailing: Option<AnyElement>) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .h(px(26.0))
        .mt_1()
        .child(div().text_color(hsla(color::TEXT)).child(title.into()))
        .children(trailing)
}

/// The width of each control slot on a card heading. Every heading keeps its slots, filled
/// or not, so the buttons line up in columns down a tab.
const HEADING_SLOT: f32 = 24.0;

/// A button on a card heading: a small outlined square with an icon and a tooltip. Its click
/// never reaches the heading, so it doesn't fold the card.
pub fn heading_button(
    id: impl Into<ElementId>,
    icon: IconName,
    tooltip: impl Into<SharedString>,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .outline()
        .small()
        .icon(Icon::new(icon))
        .tooltip(tooltip.into())
        .selected(active)
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            on_click(event, window, cx);
        })
}

/// A small "i" button whose tooltip explains a section, in place of a paragraph in the panel.
/// Clicking it does nothing; hovering shows the text.
pub fn info_button(id: impl Into<ElementId>, text: impl Into<SharedString>) -> AnyElement {
    heading_button(id, IconName::Info, text, false, |_, _, _| {}).into_any_element()
}

/// A card's heading: what it shows and the controls on its right, made with `heading_button`.
/// `info` takes the first slot, then each of `trailing` in order (turn on or off, reset, then
/// add or delete); a `None` keeps its slot empty so the columns stay lined up.
pub struct CardHeading {
    pub id: ElementId,
    pub icon: Option<IconName>,
    pub title: SharedString,
    /// What the section is for, shown by an info button's tooltip.
    pub info: Option<SharedString>,
    pub open: bool,
    /// Drawn faded, for something turned off (an effect that is hidden, say).
    pub muted: bool,
    pub trailing: Vec<Option<AnyElement>>,
}

/// One group of settings, as the Properties, Effects and Color tabs all show them, drawn flat
/// like the Align panel: a heading that folds the group on a click, `body` under it while
/// open, and a hairline rule below.
pub fn card(heading: CardHeading, on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static, body: Option<AnyElement>) -> Div {
    let CardHeading { id, icon, title, info, open, muted, trailing } = heading;
    let text = if muted { color::MUTED } else { color::TEXT };
    let info = info.map(|text| info_button(ElementId::Name(format!("{id}-info").into()), text));
    let slot = |control: Option<AnyElement>| div().w(px(HEADING_SLOT)).flex_none().flex().justify_center().children(control);
    let slots = div().flex_none().flex().items_center().gap_1().child(slot(info)).children(trailing.into_iter().map(slot));
    let row = div()
        .id(id)
        .flex()
        .items_center()
        .gap_1p5()
        .h(px(32.0))
        .px_2p5()
        .cursor_pointer()
        .hover(|style| style.bg(hsla(color::HOVER)))
        .on_click(on_toggle)
        .child(Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).size(px(12.0)).text_color(hsla(color::MUTED)))
        .children(icon.map(|icon| Icon::new(icon).size(px(13.0)).text_color(hsla(color::MUTED))))
        .child(div().flex_1().min_w_0().truncate().text_color(hsla(text)).child(title))
        .child(slots)
        .test_support();
    div()
        .flex_none()
        .border_b_1()
        .border_color(hsla(color::HAIRLINE))
        .child(row)
        .children(body.filter(|_| open).map(|body| div().px_2p5().pb_2p5().when(muted, |body| body.opacity(0.5)).child(body)))
}

/// The list of cards a settings tab scrolls through. Lines that aren't cards (a status line,
/// the Add Effect button) get `list_row` padding so they line up with the card headings.
pub fn card_list(id: impl Into<ElementId>) -> gpui_kit::Stateful<Div> {
    div().id(id).relative().size_full().overflow_y_scroll().flex().flex_col()
}

/// A line in a card list that isn't a card, padded like the cards' headings.
pub fn list_row() -> Div {
    div().flex_none().px_2p5().py_2()
}

/// A thin rule between panel sections.
pub fn divider() -> Div {
    div().h(px(1.0)).my_1p5().bg(hsla(color::HAIRLINE))
}

/// GPUI draws a view under the pointer during a drag; drags that move something in place draw nothing.
pub struct EmptyDragView;

impl Render for EmptyDragView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// A small ghost icon button with a tooltip. When `active` (a setting switched on, or the tool
/// in use) its icon takes the accent colour on an accent-tinted background, so it reads as on at
/// a glance.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    tooltip: impl Into<gpui_kit::SharedString>,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    let icon = if active { Icon::new(icon).text_color(hsla(color::ACCENT)) } else { Icon::new(icon) };
    Button::new(id)
        .ghost()
        .small()
        .icon(icon)
        .tooltip(tooltip)
        .selected(active)
        .on_click(on_click)
}

/// A dropdown that shows the current choice: an outlined button with the choice's name and a
/// caret. Every dropdown in panels, dialogs and settings starts here, so they share one
/// height, one text size and one look. Add `.dropdown_menu(...)` for its choices.
pub fn dropdown(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    Button::new(id).outline().small().label(label.into()).dropdown_caret(true)
}

/// A row of a dropdown menu with a test id, for menus whose items are checked on and off.
pub fn menu_row(id: &'static str, label: &'static str) -> impl IntoElement {
    div().id(id).child(label).test_support()
}

/// The same dropdown without an outline, for the bars above and below the canvas, where
/// outlines would crowd the zoom and view controls. It keeps the height and text size of
/// `dropdown`.
pub fn bar_dropdown(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    Button::new(id).ghost().small().label(label.into()).dropdown_caret(true)
}

/// The text field a name turns into while it is renamed (a Project item, a clip), with its text
/// at `text_size` so it stays the size of the name it replaces. (The input's own size presets
/// draw smaller text than the rows around them.)
pub fn name_input(input: &gpui_kit::Entity<gpui_kit::component::input::InputState>, text_size: f32) -> gpui_kit::component::input::Input {
    // Inputs draw their text at seven eighths of a custom size.
    gpui_kit::component::input::Input::new(input).with_size(gpui_kit::component::Size::Size(px(text_size / 0.875)))
}

/// A row of muted helper text, used for empty states.
pub fn hint(text: &'static str) -> Div {
    div().p_2p5().text_color(hsla(color::MUTED)).child(text)
}

/// A colored label swatch, as shown next to layers and project items.
pub fn label_swatch(rgba: u32) -> Div {
    div().size(px(10.0)).flex_none().rounded(px(2.0)).bg(gpui_kit::rgba(rgba))
}

/// An element's window-space bounds as of the last frame, in plain pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

/// Remembers where an element was drawn last frame, for layouts and pointer math that
/// depend on it. When the bounds change, the window redraws once so the layout catches up.
#[derive(Clone, Default)]
pub struct MeasuredBounds(Rc<Cell<Rect>>);

impl MeasuredBounds {
    pub fn get(&self) -> Rect {
        self.0.get()
    }

    /// An invisible element that fills its parent (which must be positioned) and records its bounds.
    pub fn probe(&self) -> impl IntoElement + use<> {
        let cell = self.0.clone();
        canvas(
            move |bounds: Bounds<Pixels>, window, _| {
                let rect = Rect {
                    left: f32::from(bounds.origin.x),
                    top: f32::from(bounds.origin.y),
                    width: f32::from(bounds.size.width),
                    height: f32::from(bounds.size.height),
                };
                if cell.get() != rect {
                    cell.set(rect);
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }
}

/// The look every floating surface shares, so popups read as part of the editor: a panel's
/// own color, the hairline border panels are divided by, and slightly rounded corners.
pub fn popup_surface<E: Styled>(surface: E) -> E {
    surface.text_size(px(size::TEXT)).bg(hsla(color::PANEL)).border_1().border_color(hsla(color::HAIRLINE)).rounded(px(size::POPUP_RADIUS)).shadow_lg()
}

/// A dimmed layer over the whole window with `card` in its middle, for popups Studio draws
/// itself (the quick search, the import progress). It takes every click, so nothing under it
/// is edited by accident.
pub fn popup_backdrop(id: impl Into<ElementId>, card: impl IntoElement) -> gpui_kit::Stateful<Div> {
    div().id(id).absolute().inset_0().flex().items_center().justify_center().bg(gpui_kit::black().opacity(0.45)).occlude().child(card)
}

/// How every dialog appears and leaves: it fades in where it will sit, in the middle of the
/// window, and fades out the same way. Nothing slides in from an edge.
const POPUP_ENTRANCE: gpui_kit::component::dialog::DialogEntrance = gpui_kit::component::dialog::DialogEntrance::Fade;

/// Opens a dialog in the middle of the window, looking like the rest of the editor. Every
/// dialog goes through here (or `open_alert_dialog`), so they all sit and look the same.
pub fn open_dialog(window: &mut Window, cx: &mut App, build: impl Fn(gpui_kit::component::dialog::Dialog, &mut Window, &mut App) -> gpui_kit::component::dialog::Dialog + 'static) {
    use gpui_kit::component::WindowExt as _;
    window.open_dialog(cx, move |dialog, window, cx| popup_surface(build(dialog, window, cx).centered().entrance(POPUP_ENTRANCE)));
}

/// Opens a short message or a yes-or-no question in the middle of the window, looking like
/// the rest of the editor.
pub fn open_alert_dialog(window: &mut Window, cx: &mut App, build: impl Fn(gpui_kit::component::dialog::AlertDialog, &mut Window, &mut App) -> gpui_kit::component::dialog::AlertDialog + 'static) {
    use gpui_kit::component::WindowExt as _;
    window.open_alert_dialog(cx, move |dialog, window, cx| popup_surface(build(dialog, window, cx).centered().entrance(POPUP_ENTRANCE)));
}
