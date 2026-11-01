//! The window's chrome around the panels: a slim top bar (the project's name, export
//! progress and the Export button) and a rail down the left side, CapCut style: buttons that
//! pick the panel beside it (Media, Effects), the editing tools, and import. The rail holds
//! only things that work; more are added as they are built.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{
    Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::theme::{color, hsla, size};
use crate::workspace::{LeftPanel, Tool, Workspace};

/// Width of the left rail: room for an icon and its name.
pub const RAIL_WIDTH: f32 = 68.0;

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
    div().h(px(1.0)).w(px(36.0)).my_1p5().bg(hsla(color::HAIRLINE))
}

/// A rail button: an icon with its name underneath, as in CapCut's side bar.
fn rail_button(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    active: bool,
    on_click: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    div()
        .id(id)
        .w(px(RAIL_WIDTH - 12.0))
        .py_1p5()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .rounded(px(8.0))
        .text_size(px(10.0))
        .text_color(hsla(if active { color::TEXT } else { color::MUTED }))
        .when(active, |button| button.bg(hsla(color::ROW_SELECTED)))
        .hover(|style| style.bg(hsla(color::HOVER)).text_color(hsla(color::TEXT)))
        .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
        .child(Icon::new(icon).size(px(20.0)))
        .child(label)
        .test_support()
}

/// The rail down the left: the panels it shows beside it (Media, Effects), the editing tools,
/// and import and new composition.
pub fn rail(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let (tool, side) = (workspace.ui.tool, workspace.ui.left_panel);
    let show = |panel: LeftPanel| {
        move |this: &mut Workspace, _: &mut Window, cx: &mut Context<Workspace>| {
            this.ui.left_panel = panel;
            cx.notify();
        }
    };
    let pick = |which: Tool| {
        move |this: &mut Workspace, _: &mut Window, cx: &mut Context<Workspace>| {
            this.ui.tool = which;
            cx.notify();
        }
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
        .child(rail_button("rail-media", IconName::Folder, "Media", side == LeftPanel::Media, show(LeftPanel::Media), cx))
        .child(rail_button("rail-effects", IconName::Sparkles, "Effects", side == LeftPanel::Effects, show(LeftPanel::Effects), cx))
        .child(separator())
        .child(rail_button("tool-selection", IconName::MousePointer2, "Select", tool == Tool::Selection, pick(Tool::Selection), cx))
        .child(rail_button("tool-type", IconName::Type, "Text", tool == Tool::Type, pick(Tool::Type), cx))
        .child(separator())
        .child(rail_button("rail-import", IconName::FileInput, "Import", false, |this, window, cx| this.import(window, cx), cx))
        .child(rail_button("rail-new-comp", IconName::Clapperboard, "New Comp", false, |this, window, cx| super::composition_settings::open(this, true, window, cx), cx))
}
