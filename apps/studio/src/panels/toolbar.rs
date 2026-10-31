//! The top bar's tool palette (3D tools are left out on purpose). Tools that are
//! not built yet are shown disabled, so nothing in the bar pretends to work.

use gpui_kit::assets::IconName;
use gpui_kit::component::Disableable as _;
use gpui_kit::{Context, IntoElement, ParentElement, Styled, div, px};

use super::chrome::icon_button;
use crate::theme::{color, hsla, size};
use crate::workspace::{Tool, Workspace};

/// Each tool: its icon, tooltip, and the working tool it selects (`None` for ones still to come).
const TOOL_GROUPS: &[&[(IconName, &str, Option<Tool>)]] = &[
    &[
        (IconName::MousePointer2, "Selection Tool (V)", Some(Tool::Selection)),
        (IconName::Hand, "Hand Tool (coming soon)", None),
        (IconName::ZoomIn, "Zoom Tool (coming soon)", None),
    ],
    &[(IconName::RotateCw, "Rotation Tool (coming soon)", None), (IconName::Crosshair, "Pan Behind Tool (coming soon)", None)],
    &[
        (IconName::Square, "Rectangle Tool (coming soon)", None),
        (IconName::PenTool, "Pen Tool (coming soon)", None),
        (IconName::Type, "Type Tool (⌘T): click in the viewer to add text", Some(Tool::Type)),
    ],
    &[
        (IconName::Brush, "Brush Tool (coming soon)", None),
        (IconName::Stamp, "Clone Stamp Tool (coming soon)", None),
        (IconName::Eraser, "Eraser Tool (coming soon)", None),
        (IconName::WandSparkles, "Roto Brush Tool (coming soon)", None),
    ],
    &[(IconName::Pin, "Puppet Position Pin Tool (coming soon)", None)],
];

fn separator() -> gpui_kit::Div {
    div().w(px(1.0)).h(px(20.0)).mx_1().bg(hsla(color::HAIRLINE))
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let active = workspace.ui.tool;
    let mut tools = div().flex().items_center().gap_0p5();
    let mut index: usize = 0;
    for (group_index, group) in TOOL_GROUPS.iter().enumerate() {
        if group_index > 0 {
            tools = tools.child(separator());
        }
        for &(icon, tooltip, tool) in group.iter() {
            let button = icon_button(("tool", index), icon, tooltip, tool == Some(active), cx.listener(move |this, _, _, cx| {
                if let Some(tool) = tool {
                    this.ui.tool = tool;
                    cx.notify();
                }
            }));
            tools = tools.child(button.disabled(tool.is_none()));
            index += 1;
        }
    }

    div()
        .h(px(size::TOOLBAR_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .bg(hsla(color::PANEL))
        .child(tools)
        // An export running in the background shows its progress at the right.
        .child(div().flex_1())
        .children(super::export::status(workspace, cx))
}
