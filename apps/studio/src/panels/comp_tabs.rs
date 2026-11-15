//! The composition tabs across the top of the Composition panel: one per open composition, as
//! documents are in Canvas. Clicking a tab shows that composition in the viewer and timeline,
//! its × closes the tab (the composition stays in the project), and + makes a new composition
//! in a tab of its own. The tabs themselves are kept by the editor state (see `state::tabs`).

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{Context, ElementId, InteractiveElement, IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, TestSupportExt as _, div, prelude::FluentBuilder as _, px};

use super::chrome::icon_button;
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// The widest a tab grows, so a few long names still leave room for the rest.
const TAB_MAX_WIDTH: f32 = 220.0;

/// The row of tabs, with the + button at its end.
pub fn strip(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let state = &workspace.state;
    let tabs: Vec<_> = state
        .tabs()
        .into_iter()
        .filter_map(|id| {
            let name = state.project.composition(id).map(|composition| composition.name.clone())?;
            Some(tab(id, name, id == state.active_comp, state.can_close_tab(id), cx))
        })
        .collect();
    let new_tab = icon_button("comp-tab-new", IconName::Plus, "New composition", false, cx.listener(|this, _, window, cx| {
        super::composition_settings::open(this, true, window, cx);
    }));
    div()
        .id("comp-tabs")
        .h(px(size::PANEL_HEADER_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        .px_1()
        .border_b_1()
        .border_color(hsla(color::HAIRLINE))
        // Tabs share what room there is, a long name shortened with an ellipsis.
        .child(div().flex_1().min_w_0().h_full().flex().items_center().gap_0p5().overflow_hidden().children(tabs).child(new_tab))
        .test_support()
}

/// One composition's tab: its name, and a × to close it when it isn't the only tab.
fn tab(id: studio_core::model::Id, name: String, selected: bool, closable: bool, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let close = closable.then(|| {
        div()
            .id(ElementId::Name(format!("comp-tab-close:{}", id.0).into()))
            .flex_none()
            .size(px(16.0))
            .rounded_sm()
            .flex()
            .items_center()
            .justify_center()
            .text_color(hsla(color::MUTED))
            .hover(|style| style.bg(hsla(color::HAIRLINE)).text_color(hsla(color::TEXT)))
            .child(Icon::new(IconName::X).size(px(12.0)))
            .on_click(cx.listener(move |this, _, _, cx| {
                // The click closes the tab only; it doesn't also open it.
                cx.stop_propagation();
                if this.state.close_tab(id) {
                    cx.notify();
                }
            }))
            .test_support()
    });
    div()
        .id(ElementId::Name(format!("comp-tab:{}", id.0).into()))
        .h(px(24.0))
        .min_w_0()
        .max_w(px(TAB_MAX_WIDTH))
        .flex_shrink(1.0)
        .pl_2()
        // Room for the × when there is one; even sides when there isn't.
        .pr(px(if closable { 4.0 } else { 8.0 }))
        .gap_1()
        .rounded_md()
        .flex()
        .items_center()
        .text_color(hsla(if selected { color::TEXT } else { color::MUTED }))
        .when(selected, |tab| tab.bg(hsla(color::HAIRLINE)))
        .hover(|style| style.text_color(hsla(color::TEXT)))
        .on_click(cx.listener(move |this, _, _, cx| {
            if this.state.active_comp != id {
                this.state.open_composition(id);
                cx.notify();
            }
        }))
        .child(div().min_w_0().truncate().child(SharedString::from(name)))
        .children(close)
        .test_support()
}
