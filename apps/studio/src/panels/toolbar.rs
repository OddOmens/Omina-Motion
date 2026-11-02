//! The window's chrome around the panels: a slim top bar (the project's name and the progress
//! of background jobs) and a rail down the left side, CapCut style: at the top, the buttons that
//! pick the panel beside it (Media, Libraries, Effects) and the editing tools; at the bottom,
//! New Comp, Import and Export, then Settings. The rail holds only things that work.
//!
//! The Pen button (G) draws custom shapes point by point. The Shape button picks the Shape tool on a click; pressing and holding it (or right-clicking)
//! opens a menu beside it to choose which shape the tool draws.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use std::time::Duration;

use gpui_kit::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _,
    Window, deferred, div, prelude::FluentBuilder as _, px,
};
use studio_core::model::ShapeKind;

use crate::theme::{color, hsla, size};
use crate::workspace::{LeftPanel, Tool, Workspace};

/// Width of the left rail: room for an icon and its name.
pub const RAIL_WIDTH: f32 = 56.0;

/// The top bar: where you are on the left, and background jobs' progress on the right.
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
        // Captions, cut-outs and exports running in the background show their progress here.
        .children(super::captions::status(workspace, cx))
        .children(super::cutout::status(workspace, cx))
        .children(super::enhance::status(workspace, cx))
        .children(super::analysis::status(workspace, cx))
        .children(super::export::status(workspace, cx))
}

/// A short line between groups of rail buttons. The panel hairline is too faint against the
/// rail's darker background, so it uses the muted text color, softened.
fn separator() -> gpui_kit::Div {
    div().flex_none().h(px(1.0)).w(px(32.0)).my_1p5().bg(hsla(color::MUTED).opacity(0.45))
}

/// How long the Shape button must be held to open the shape menu.
pub const HOLD_FOR_MENU: Duration = Duration::from_millis(350);

/// A tool on the rail: the tool, its icon, its name and its key.
type RailTool = (Tool, IconName, &'static str, &'static str);

/// Tools that share one rail button. A click picks the tool the button shows (the one last
/// picked from the group); holding it, or right-clicking, opens a menu of the whole group.
pub struct ToolGroup {
    id: &'static str,
    tools: &'static [RailTool],
}

pub const TOOL_GROUPS: [ToolGroup; 3] = [
    ToolGroup { id: "tool-group-select", tools: &[(Tool::Selection, IconName::MousePointer2, "Select", "V"), (Tool::TrackSelect, IconName::ChevronsRight, "Track Select", "A")] },
    ToolGroup {
        id: "tool-group-edit",
        tools: &[
            (Tool::Cut, IconName::Scissors, "Cut", "C"),
            (Tool::Ripple, IconName::ArrowRightToLine, "Ripple", "B"),
            (Tool::Roll, IconName::Columns2, "Roll", "N"),
            (Tool::Slip, IconName::ArrowLeftRight, "Slip", "Y"),
            (Tool::Slide, IconName::MoveHorizontal, "Slide", "U"),
        ],
    },
    ToolGroup { id: "tool-group-view", tools: &[(Tool::Hand, IconName::Hand, "Hand", "H"), (Tool::Zoom, IconName::ZoomIn, "Zoom", "Z")] },
];

/// What each group does, for its menu's heading.
const GROUP_TITLES: [&str; 3] = ["Selecting", "Editing clips", "Looking around"];

/// A group's button: the current tool if it is one of the group's, else the one last picked.
fn group_button(index: usize, workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let group = &TOOL_GROUPS[index];
    let current = workspace.ui.tool;
    let shown = group.tools.iter().find(|tool| tool.0 == current).or_else(|| workspace.ui.group_tools.get(&index).and_then(|picked| group.tools.iter().find(|tool| tool.0 == *picked))).unwrap_or(&group.tools[0]);
    let (tool, icon, label, _) = *shown;
    let active = group.tools.iter().any(|tool| tool.0 == current);
    let corner = div().absolute().right(px(4.0)).bottom(px(4.0)).w(px(4.0)).h(px(1.0)).bg(hsla(color::MUTED));
    let corner_up = div().absolute().right(px(4.0)).bottom(px(4.0)).w(px(1.0)).h(px(4.0)).bg(hsla(color::MUTED));
    div()
        .relative()
        .child(
            rail_button_look(group.id, icon, label, active)
                .relative()
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, window, cx| {
                    this.set_tool(tool);
                    this.ui.tool_press = Some(index);
                    cx.notify();
                    // Still held after a moment: open the group's menu.
                    cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor().timer(HOLD_FOR_MENU).await;
                        let _ = this.update(cx, |this, cx| {
                            if this.ui.tool_press == Some(index) {
                                this.ui.tool_menu = Some(index);
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                }))
                .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.ui.tool_press = None))
                .on_mouse_down(MouseButton::Right, cx.listener(move |this, _, _, cx| {
                    this.ui.tool_menu = Some(index);
                    cx.notify();
                }))
                .child(corner)
                .child(corner_up)
                .test_support(),
        )
        .when(workspace.ui.tool_menu == Some(index), |button| button.child(group_menu(index, current, cx)))
}

/// The menu beside a group's button: each of its tools, with its key.
fn group_menu(index: usize, current: Tool, cx: &mut Context<Workspace>) -> impl IntoElement {
    let rows = TOOL_GROUPS[index].tools.iter().map(|&(tool, icon, label, key)| {
        div()
            .id(gpui_kit::ElementId::Name(format!("tool-{label}").to_lowercase().replace(' ', "-").into()))
            .h(px(30.0))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .rounded(px(6.0))
            .when(tool == current, |row| row.bg(hsla(color::ROW_SELECTED)))
            .hover(|style| style.bg(hsla(color::HOVER)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_tool(tool);
                this.ui.group_tools.insert(index, tool);
                this.ui.tool_menu = None;
                cx.notify();
            }))
            .child(Icon::new(icon).size(px(15.0)).text_color(hsla(color::MUTED)))
            .child(div().flex_1().child(label))
            .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(key))
            .test_support()
    });
    deferred(
        div()
            .id("tool-menu")
            .absolute()
            .left(px(RAIL_WIDTH - 6.0))
            .top_0()
            .w(px(190.0))
            .p_1()
            .flex()
            .flex_col()
            .gap_0p5()
            .rounded(px(10.0))
            .bg(hsla(color::POPOVER))
            .border_1()
            .border_color(hsla(color::HAIRLINE))
            .shadow_lg()
            .text_size(px(size::TEXT))
            .text_color(hsla(color::TEXT))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.ui.tool_menu = None;
                cx.notify();
            }))
            .child(div().px_2().py_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(GROUP_TITLES[index]))
            .children(rows)
            .test_support(),
    )
    .with_priority(2)
}

/// The rail icon for each shape.
pub fn shape_icon(kind: ShapeKind) -> IconName {
    match kind {
        ShapeKind::Rectangle => IconName::Square,
        ShapeKind::Ellipse => IconName::Circle,
        ShapeKind::Triangle => IconName::Triangle,
        ShapeKind::Polygon => IconName::Hexagon,
        ShapeKind::Star => IconName::Star,
        ShapeKind::Path => IconName::PenTool,
    }
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
    rail_button_look(id, icon, label, active).on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx))).test_support()
}

/// A rail button's appearance, without its behaviour.
fn rail_button_look(id: &'static str, icon: IconName, label: &'static str, active: bool) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .w(px(RAIL_WIDTH - 12.0))
        .py_1()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .rounded(px(6.0))
        .text_size(px(8.0))
        .text_color(hsla(if active { color::TEXT } else { color::MUTED }))
        .when(active, |button| button.bg(hsla(color::ROW_SELECTED)))
        .hover(|style| style.bg(hsla(color::HOVER)).text_color(hsla(color::TEXT)))
        .child(Icon::new(icon).size(px(16.0)))
        .child(label)
}

/// The Shape button: a click picks the Shape tool; a double-click adds a background that fills
/// the frame; holding it, or right-clicking, opens the shape menu. A small mark in its corner
/// says there is a menu.
fn shape_button(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let kind = workspace.ui.shape_kind;
    let active = workspace.ui.tool == Tool::Shape;
    // The corner mark: a tiny L in the bottom right.
    let corner = div().absolute().right(px(5.0)).bottom(px(5.0)).w(px(5.0)).h(px(1.0)).bg(hsla(color::MUTED));
    let corner_up = div().absolute().right(px(5.0)).bottom(px(5.0)).w(px(1.0)).h(px(5.0)).bg(hsla(color::MUTED));
    div()
        .relative()
        .child(
            rail_button_look("tool-shape", shape_icon(kind), "Shape", active)
                .relative()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &gpui_kit::MouseDownEvent, window, cx| {
                    if event.click_count >= 2 {
                        this.ui.shape_press = false;
                        this.state.add_background();
                        cx.notify();
                        return;
                    }
                    this.set_tool(Tool::Shape);
                    this.ui.shape_press = true;
                    cx.notify();
                    // Still held after a moment: open the menu.
                    cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor().timer(HOLD_FOR_MENU).await;
                        let _ = this.update(cx, |this, cx| {
                            if this.ui.shape_press {
                                this.ui.shape_menu = true;
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                }))
                .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.ui.shape_press = false))
                .on_mouse_down(MouseButton::Right, cx.listener(|this, _, _, cx| {
                    this.ui.shape_menu = true;
                    cx.notify();
                }))
                .child(corner)
                .child(corner_up)
                .test_support(),
        )
        .when(workspace.ui.shape_menu, |button| button.child(shape_menu(kind, cx)))
}

/// The menu of shapes beside the Shape button. Picking one selects the Shape tool with it.
fn shape_menu(current: ShapeKind, cx: &mut Context<Workspace>) -> impl IntoElement {
    let rows = ShapeKind::ALL.into_iter().enumerate().map(|(index, kind)| {
        div()
            .id(("shape-kind", index))
            .h(px(30.0))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .rounded(px(6.0))
            .when(kind == current, |row| row.bg(hsla(color::ROW_SELECTED)))
            .hover(|style| style.bg(hsla(color::HOVER)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.ui.shape_kind = kind;
                this.set_tool(Tool::Shape);
                this.ui.shape_menu = false;
                cx.notify();
            }))
            .child(Icon::new(shape_icon(kind)).size(px(15.0)).text_color(hsla(color::MUTED)))
            .child(div().flex_1().child(kind.name()))
            .when(kind.has_corners(), |row| row.child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("rounds")))
            .test_support()
    });
    deferred(
        div()
            .id("shape-menu")
            .absolute()
            .left(px(RAIL_WIDTH - 6.0))
            .top_0()
            .w(px(170.0))
            .p_1()
            .flex()
            .flex_col()
            .gap_0p5()
            .rounded(px(10.0))
            .bg(hsla(color::POPOVER))
            .border_1()
            .border_color(hsla(color::HAIRLINE))
            .shadow_lg()
            .text_size(px(size::TEXT))
            .text_color(hsla(color::TEXT))
            // Clicking anywhere else closes it.
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.ui.shape_menu = false;
                cx.notify();
            }))
            .children(rows)
            .test_support(),
    )
    .with_priority(2)
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
            this.set_tool(which);
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
        // At the top: the side panels, then the tools.
        .child(rail_button("rail-media", IconName::Folder, "Media", side == LeftPanel::Media, show(LeftPanel::Media), cx))
        .child(rail_button("rail-libraries", IconName::Library, "Libraries", side == LeftPanel::Libraries, show(LeftPanel::Libraries), cx))
        .child(rail_button("rail-effects", IconName::Sparkles, "Effects", side == LeftPanel::Effects, show(LeftPanel::Effects), cx))
        .child(rail_button("rail-elements", IconName::Sticker, "Elements", side == LeftPanel::Elements, show(LeftPanel::Elements), cx))
        .child(rail_button("rail-templates", IconName::LayoutTemplate, "Templates", side == LeftPanel::Templates, show(LeftPanel::Templates), cx))
        .child(rail_button("rail-mockups", IconName::TabletSmartphone, "Mockups", side == LeftPanel::Mockups, show(LeftPanel::Mockups), cx))
        .when(workspace.ui.settings.ai.captions, |rail| rail.child(rail_button("rail-captions", IconName::Captions, "Captions", side == LeftPanel::Captions, show(LeftPanel::Captions), cx)))
        .child(separator())
        .child(group_button(0, workspace, cx))
        .child(group_button(1, workspace, cx))
        .child(rail_button("tool-type", IconName::Type, "Text", tool == Tool::Type, pick(Tool::Type), cx))
        .child(shape_button(workspace, cx))
        .child(rail_button("tool-pen", IconName::PenTool, "Pen", tool == Tool::Pen, pick(Tool::Pen), cx))
        .child(rail_button("tool-rotate", IconName::RotateCw, "Rotate", tool == Tool::Rotate, pick(Tool::Rotate), cx))
        .child(group_button(2, workspace, cx))
        // At the bottom: getting work in and out (new composition ⌘N, import ⌘I, export ⌘M),
        // and Settings last.
        .child(div().flex_1())
        .child(rail_button("rail-new-comp", IconName::Clapperboard, "New Comp", false, |this, window, cx| super::composition_settings::open(this, true, window, cx), cx))
        .child(rail_button("rail-import", IconName::FileInput, "Import", false, |this, window, cx| this.import(window, cx), cx))
        .child(rail_button("rail-export", IconName::Download, "Export", false, super::export::open, cx))
        .child(separator())
        .child(rail_button("rail-settings", IconName::Settings, "Settings", false, super::settings::open, cx))
}
