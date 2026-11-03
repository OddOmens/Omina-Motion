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

/// A panel's frame: a header with its tab(s) and menu, the body, and an optional footer.
/// With several tabs, clicking one switches the panel to it (see `UiState::tab`). With no tabs
/// there is no header, for panels that show their title in their own toolbar.
/// Pressing anywhere inside makes it the active panel (which decides what Delete acts on).
pub fn panel(
    id: PanelId,
    tabs: &[&str],
    footer: Option<AnyElement>,
    body: impl IntoElement,
    workspace: &Workspace,
    cx: &mut Context<Workspace>,
) -> Div {
    let active_tab = workspace.ui.tab(id);
    let clickable = tabs.len() > 1;
    let collapsible = matches!(id, PanelId::Properties | PanelId::Align);
    let collapsed = collapsible && workspace.ui.collapsed_panels.contains(&id);
    let header = div()
        .h(px(size::PANEL_HEADER_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        .px_1()
        .border_b_1()
        .border_color(hsla(color::HAIRLINE))
        // Tabs share what room there is, a long title (a layer's name) shortened with an
        // ellipsis, so the collapse button always stays in view.
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
                .when(clickable && selected, |tab| tab.border_b_2().border_color(hsla(color::TEXT)))
                .when(clickable, |tab| {
                    tab.hover(|style| style.text_color(hsla(color::TEXT))).on_click(cx.listener(move |this, _, _, cx| {
                        this.ui.tabs.insert(id, index);
                        cx.notify();
                    }))
                })
                .child(div().min_w_0().truncate().child(SharedString::from(title.to_string())))
                .test_support()
        })))
        // Panels in the right column fold away to their header.
        .when(collapsible, |header| {
            header.child(
                div()
                    .id(ElementId::Name(format!("collapse:{id:?}").into()))
                    .size(px(22.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.0))
                    .text_color(hsla(color::MUTED))
                    .hover(|style| style.bg(hsla(color::HOVER)).text_color(hsla(color::TEXT)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.ui.collapsed_panels.remove(&id) {
                            this.ui.collapsed_panels.insert(id);
                        }
                        cx.notify();
                    }))
                    .child(Icon::new(if collapsed { IconName::ChevronRight } else { IconName::ChevronDown }).size(px(14.0)))
                    .test_support(),
            )
        });

    div()
        .size_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .rounded(px(size::PANEL_RADIUS))
        .bg(hsla(color::PANEL))
        .border_1()
        .border_color(hsla(color::PANEL))
        .capture_any_mouse_down(cx.listener(move |this, _, _, cx| {
            if this.ui.active_panel != id {
                this.ui.active_panel = id;
                cx.notify();
            }
        }))
        .when(!tabs.is_empty(), |panel| panel.child(header))
        .when(!collapsed, |panel| panel.child(div().flex_1().min_h_0().relative().child(body)))
        .children(footer.filter(|_| !collapsed).map(|footer| {
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
        .child(div().text_color(hsla(color::TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(title.into()))
        .children(trailing)
}

/// A card's heading: what it shows and the controls on its right. Each control in
/// `trailing` must stop its click from reaching the heading, which folds the card.
pub struct CardHeading {
    pub id: ElementId,
    pub icon: Option<IconName>,
    pub title: SharedString,
    pub open: bool,
    /// Drawn faded, for something turned off (an effect that is hidden, say).
    pub muted: bool,
    pub trailing: Vec<AnyElement>,
}

/// A rounded card holding one group of settings, as the Properties, Effects and Color tabs
/// all show them: a heading that folds the card on a click, and `body` under it while open.
pub fn card(heading: CardHeading, on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static, body: Option<AnyElement>) -> Div {
    let CardHeading { id, icon, title, open, muted, trailing } = heading;
    let text = if muted { color::MUTED } else { color::TEXT };
    let row = div()
        .id(id)
        .flex()
        .items_center()
        .gap_1p5()
        .h(px(34.0))
        .px_2()
        .cursor_pointer()
        .hover(|style| style.bg(hsla(color::HOVER)))
        .on_click(on_toggle)
        .child(Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).size(px(12.0)).text_color(hsla(color::MUTED)))
        .children(icon.map(|icon| Icon::new(icon).size(px(14.0)).text_color(hsla(if muted { color::MUTED } else { color::ACCENT }))))
        .child(div().flex_1().min_w_0().truncate().font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(hsla(text)).child(title))
        .children(trailing)
        .test_support();
    div()
        .flex_none()
        .rounded(px(8.0))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .bg(hsla(color::RAISED))
        .overflow_hidden()
        .child(row)
        .children(body.filter(|_| open).map(|body| div().px_3().pb_3().when(muted, |body| body.opacity(0.5)).child(body)))
}

/// The list of cards a settings tab scrolls through, spaced as the Color tab spaces them.
pub fn card_list(id: impl Into<ElementId>) -> gpui_kit::Stateful<Div> {
    div().id(id).relative().size_full().overflow_y_scroll().px_2p5().py_2().flex().flex_col().gap_2()
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

/// A small ghost icon button with a tooltip.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    tooltip: &'static str,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .icon(Icon::new(icon))
        .tooltip(tooltip)
        .selected(active)
        .on_click(on_click)
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
