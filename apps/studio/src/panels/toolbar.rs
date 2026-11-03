//! The window's chrome around the panels: a slim top bar and a rail down the left side.
//!
//! The top bar has the app's name on the left with the editing tools beside it (as in
//! Photoshop's and Premiere's tool bars, one small icon each, named in a tooltip with its
//! key), and on the right the project and composition open and the progress of background
//! jobs. The rail, CapCut style, picks the
//! panel beside it (Media, Libraries, Effects and so on) and at the bottom has New Comp,
//! Import and Export, then Settings. Both hold only things that work.
//!
//! Some tool buttons hold more than one tool or a menu: a click picks the tool shown, and
//! pressing and holding (or right-clicking) opens a menu under it. The Color button (I) is the
//! eyedropper; its menu holds the current color, recent picks and swatches. The Shape button's
//! menu picks which shape the tool draws.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use std::time::Duration;

use gpui_kit::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _,
    Window, deferred, div, prelude::FluentBuilder as _, px,
};
use studio_core::model::ShapeKind;

use crate::theme::{color, hsla, size};
use crate::workspace::{LeftPanel, NewLayer, Tool, Workspace};

/// Width of the left rail: room for an icon and its name.
pub const RAIL_WIDTH: f32 = 56.0;
/// Width and height of a tool button in the top bar.
const TOOL_WIDTH: f32 = 30.0;
const TOOL_HEIGHT: f32 = 28.0;
/// How far below a tool button's top its menu opens.
const MENU_DROP: f32 = TOOL_HEIGHT + 6.0;

/// The top bar: the app's name and the tools on the left; where you are and background jobs'
/// progress on the right.
pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let project = workspace.state.document_name();
    let composition = workspace.state.composition().name.clone();
    // "Project / Composition", as a breadcrumb; it gives way first when the window is narrow.
    let breadcrumb = div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        .overflow_hidden()
        .child(div().min_w_0().truncate().text_color(hsla(color::MUTED)).child(project))
        .child(div().flex_none().text_color(hsla(color::MUTED)).child("/"))
        .child(div().min_w_0().truncate().text_color(hsla(color::TEXT)).child(composition));
    div()
        .w_full()
        .h(px(size::TOOLBAR_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .gap_3()
        .px_3()
        .bg(hsla(color::BACKGROUND))
        .child(div().flex_none().min_w(px(RAIL_WIDTH - 12.0)).font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(hsla(color::TEXT)).child("Studio"))
        .child(tools(workspace, cx))
        .child(breadcrumb)
        // Captions, cut-outs and exports running in the background show their progress here.
        .children(super::captions::status(workspace, cx))
        .children(super::cutout::status(workspace, cx))
        .children(super::enhance::status(workspace, cx))
        .children(super::analysis::status(workspace, cx))
        .children(super::export::status(workspace, cx))
}

/// The editing tools, in a row: selecting, editing clips, text, shapes, the pen, rotating,
/// the eyedropper, and moving around the view.
fn tools(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let tool = workspace.ui.tool;
    let pick = |which: Tool| {
        move |this: &mut Workspace, _: &mut Window, cx: &mut Context<Workspace>| {
            this.set_tool(which);
            cx.notify();
        }
    };
    let divider = || div().flex_none().w(px(1.0)).h(px(16.0)).mx_1().bg(hsla(color::MUTED).opacity(0.35));
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap_0p5()
        .child(group_button(0, workspace, cx))
        .child(group_button(1, workspace, cx))
        .child(divider())
        .child(tool_button("tool-type", IconName::Type, "Text (⌘T)", tool == Tool::Type, pick(Tool::Type), cx))
        .child(shape_button(workspace, cx))
        .child(tool_button("tool-pen", IconName::PenTool, "Pen (G)", tool == Tool::Pen, pick(Tool::Pen), cx))
        .child(tool_button("tool-rotate", IconName::RotateCw, "Rotate (W)", tool == Tool::Rotate, pick(Tool::Rotate), cx))
        .child(eyedropper_button(workspace, cx))
        .child(new_layer_button(cx))
        .child(divider())
        .child(group_button(2, workspace, cx))
}

/// The New Layer button: a menu of the layers made from nothing (a solid in the eyedropper's
/// current color, an adjustment layer, a null object), each added from the playhead on.
fn new_layer_button(cx: &mut Context<Workspace>) -> impl IntoElement {
    use gpui_kit::component::Sizable as _;
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
    let weak = cx.weak_entity();
    let item = move |label: &'static str, icon: IconName, kind: NewLayer| {
        let weak = weak.clone();
        PopupMenuItem::new(label).icon(Icon::new(icon)).on_click(move |_, _, cx| {
            weak.update(cx, |this, cx| this.new_layer(kind, cx)).ok();
        })
    };
    Button::new("tool-new-layer").ghost().small().icon(Icon::new(IconName::Layers)).tooltip("New Layer").dropdown_menu(move |menu, _, _| {
        menu.label("New Layer")
            .item(item("Solid (⌘Y)", IconName::Square, NewLayer::Solid))
            .item(item("Adjustment Layer (⌥⌘Y)", IconName::Layers, NewLayer::Adjustment))
            .item(item("Null Object (⇧⌥⌘Y)", IconName::SquareDashed, NewLayer::Null))
    })
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
            (Tool::RateStretch, IconName::Gauge, "Rate Stretch", "R"),
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
    let (tool, icon, label, key) = *shown;
    let active = group.tools.iter().any(|tool| tool.0 == current);
    let tip = if group.tools.len() > 1 { format!("{label} ({key}) · hold for more") } else { format!("{label} ({key})") };
    div()
        .relative()
        .child(
            tool_button_look(group.id, icon, tip, active)
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
                .child(menu_mark())
                .test_support(),
        )
        .when(workspace.ui.tool_menu == Some(index), |button| button.child(group_menu(index, current, cx)))
}

/// The menu under a group's button: each of its tools, with its key.
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
            .left_0()
            .top(px(MENU_DROP))
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

/// A tool button in the top bar: its icon, named in a tooltip.
fn tool_button(
    id: &'static str,
    icon: IconName,
    tip: &'static str,
    active: bool,
    on_click: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    tool_button_look(id, icon, tip.to_owned(), active).on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx))).test_support()
}

/// A tool button's appearance, without its behaviour.
fn tool_button_look(id: &'static str, icon: IconName, tip: String, active: bool) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .relative()
        .w(px(TOOL_WIDTH))
        .h(px(TOOL_HEIGHT))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .text_color(hsla(if active { color::TEXT } else { color::MUTED }))
        .when(active, |button| button.bg(hsla(color::ROW_SELECTED)))
        .hover(|style| style.bg(hsla(color::HOVER)).text_color(hsla(color::TEXT)))
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
        .child(Icon::new(icon).size(px(16.0)))
}

/// The small mark in a tool button's corner that says holding it opens a menu: a tiny L.
fn menu_mark() -> impl IntoElement {
    div()
        .absolute()
        .right(px(3.0))
        .bottom(px(3.0))
        .size(px(4.0))
        .border_r_1()
        .border_b_1()
        .border_color(hsla(color::MUTED))
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
    let tip = format!("{} (Q) · hold for shapes, double-click for a background", kind.name());
    div()
        .relative()
        .child(
            tool_button_look("tool-shape", shape_icon(kind), tip, active)
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
                .child(menu_mark())
                .test_support(),
        )
        .when(workspace.ui.shape_menu, |button| button.child(shape_menu(kind, cx)))
}

/// The Eyedropper button (I): a click picks the tool, which takes colors from the viewer.
/// Holding it, or right-clicking, opens the color menu (`color_well`). A dot in its corner
/// shows the current color.
fn eyedropper_button(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let active = workspace.ui.tool == Tool::Eyedropper;
    let current = workspace.state.colors.current;
    let dot = div()
        .absolute()
        .right(px(2.0))
        .bottom(px(2.0))
        .size(px(7.0))
        .rounded_full()
        .border_1()
        .border_color(hsla(color::MUTED))
        .bg(gpui_kit::rgba(current.with_alpha(1.0).to_rgba_u32()));
    div()
        .relative()
        .child(
            tool_button_look("tool-eyedropper", IconName::Pipette, "Eyedropper (I) · hold for colors and swatches".to_owned(), active)
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                    this.set_tool(Tool::Eyedropper);
                    this.ui.eyedropper_press = true;
                    cx.notify();
                    // Still held after a moment: open the color menu.
                    cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor().timer(HOLD_FOR_MENU).await;
                        let _ = this.update(cx, |this, cx| {
                            if this.ui.eyedropper_press {
                                this.ui.color_well = true;
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                }))
                .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.ui.eyedropper_press = false))
                .on_mouse_down(MouseButton::Right, cx.listener(|this, _, _, cx| {
                    this.ui.color_well = true;
                    cx.notify();
                }))
                .child(dot)
                .test_support(),
        )
        .when(workspace.ui.color_well, |button| button.child(super::color_well::menu(workspace, cx)))
}

/// The menu of shapes under the Shape button. Picking one selects the Shape tool with it.
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
            .left_0()
            .top(px(MENU_DROP))
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

/// The rail down the left: the panels it shows beside it (Media, Effects and so on), and
/// getting work in and out.
pub fn rail(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let side = workspace.ui.left_panel;
    let show = |panel: LeftPanel| {
        move |this: &mut Workspace, _: &mut Window, cx: &mut Context<Workspace>| {
            this.ui.left_panel = panel;
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
        // At the top: the side panels.
        .child(rail_button("rail-media", IconName::Folder, "Media", side == LeftPanel::Media, show(LeftPanel::Media), cx))
        .child(rail_button("rail-libraries", IconName::Library, "Libraries", side == LeftPanel::Libraries, show(LeftPanel::Libraries), cx))
        .child(rail_button("rail-effects", IconName::Sparkles, "Effects", side == LeftPanel::Effects, show(LeftPanel::Effects), cx))
        .child(rail_button("rail-elements", IconName::Sticker, "Elements", side == LeftPanel::Elements, show(LeftPanel::Elements), cx))
        .child(rail_button("rail-templates", IconName::LayoutTemplate, "Templates", side == LeftPanel::Templates, show(LeftPanel::Templates), cx))
        .child(rail_button("rail-mockups", IconName::TabletSmartphone, "Mockups", side == LeftPanel::Mockups, show(LeftPanel::Mockups), cx))
        .when(workspace.ui.settings.ai.captions, |rail| rail.child(rail_button("rail-captions", IconName::Captions, "Captions", side == LeftPanel::Captions, show(LeftPanel::Captions), cx)))
        // At the bottom: getting work in and out (new composition ⌘N, import ⌘I, export ⌘M),
        // and Settings last.
        .child(div().flex_1())
        .child(rail_button("rail-new-comp", IconName::Clapperboard, "New Comp", false, |this, window, cx| super::composition_settings::open(this, true, window, cx), cx))
        .child(rail_button("rail-import", IconName::FileInput, "Import", false, |this, window, cx| this.import(window, cx), cx))
        .child(rail_button("rail-export", IconName::Download, "Export", false, super::export::open, cx))
        .child(separator())
        .child(rail_button("rail-settings", IconName::Settings, "Settings", false, super::settings::open, cx))
}
