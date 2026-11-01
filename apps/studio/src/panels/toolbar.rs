//! The window's chrome around the panels: a slim top bar (the project's name, export
//! progress and the Export button) and a rail of tools down the left side. The rail holds
//! only tools that work; tools are added there as they are built.

use gpui_kit::assets::IconName;
use gpui_kit::{
    Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _, div, px,
};

use super::chrome::icon_button;
use crate::theme::{color, hsla, size};
use crate::workspace::{Tool, Workspace};

/// Width of the left tool rail.
pub const RAIL_WIDTH: f32 = 48.0;

/// The top bar: where you are on the left, and export on the right.
pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let project = workspace.state.document_name();
    let composition = workspace.state.composition().name.clone();
    div()
        .h(px(size::TOOLBAR_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .bg(hsla(color::BACKGROUND))
        // "Project / Composition", as a breadcrumb.
        .child(div().text_color(hsla(color::MUTED)).child(project))
        .child(div().text_color(hsla(color::MUTED)).child("/"))
        .child(div().text_color(hsla(color::TEXT)).child(composition))
        .child(div().flex_1())
        // An export running in the background shows its progress here.
        .children(super::export::status(workspace, cx))
        // The one bright button on screen: export the composition (⌘M).
        .child(
            div()
                .id("toolbar-export")
                .px_3()
                .py_1()
                .rounded(px(6.0))
                .bg(hsla(color::HIGHLIGHT))
                .text_color(hsla(0x111111))
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .hover(|style| style.opacity(0.9))
                .on_click(cx.listener(|this, _, window, cx| super::export::open(this, window, cx)))
                .child("Export")
                .test_support(),
        )
}

fn separator() -> gpui_kit::Div {
    div().h(px(1.0)).w(px(24.0)).my_1().bg(hsla(color::HAIRLINE))
}

/// The tool rail down the left: import and new composition, then the editing tools.
pub fn rail(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let active = workspace.ui.tool;
    let tool = |id: &'static str, icon: IconName, tooltip: &'static str, which: Tool, cx: &mut Context<Workspace>| {
        icon_button(id, icon, tooltip, active == which, cx.listener(move |this, _, _, cx| {
            this.ui.tool = which;
            cx.notify();
        }))
    };
    div()
        .w(px(RAIL_WIDTH))
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .py_2()
        .bg(hsla(color::BACKGROUND))
        .child(icon_button("rail-import", IconName::FileInput, "Import Media (⌘I)", false, cx.listener(|this, _, window, cx| this.import(window, cx))))
        .child(icon_button("rail-new-comp", IconName::Clapperboard, "New Composition (⌘N)", false, cx.listener(|this, _, window, cx| {
            super::composition_settings::open(this, true, window, cx);
        })))
        .child(separator())
        .child(tool("tool-selection", IconName::MousePointer2, "Selection Tool (V)", Tool::Selection, cx))
        .child(tool("tool-type", IconName::Type, "Type Tool (⌘T): click in the viewer to add text", Tool::Type, cx))
}
