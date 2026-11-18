//! The Align panel: lines selected layers up with the composition or with each
//! other, and spaces three or more evenly.

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::Disableable as _;
use gpui_kit::{Context, IntoElement, ParentElement, Styled, div, px};
use studio_core::align::{AlignTarget, Alignment, Distribution};

use super::chrome::{icon_button, panel};
use crate::theme::{color, hsla};
use crate::workspace::{PanelId, Workspace};

const ALIGNMENTS: [(Alignment, IconName, &str); 6] = [
    (Alignment::Left, IconName::AlignStartVertical, "Align Left"),
    (Alignment::HorizontalCenter, IconName::AlignCenterVertical, "Align Horizontal Center"),
    (Alignment::Right, IconName::AlignEndVertical, "Align Right"),
    (Alignment::Top, IconName::AlignStartHorizontal, "Align Top"),
    (Alignment::VerticalCenter, IconName::AlignCenterHorizontal, "Align Vertical Center"),
    (Alignment::Bottom, IconName::AlignEndHorizontal, "Align Bottom"),
];

const DISTRIBUTIONS: [(Distribution, IconName, &str); 6] = [
    (Distribution::Top, IconName::AlignVerticalDistributeStart, "Distribute Top Edges"),
    (Distribution::VerticalCenter, IconName::AlignVerticalDistributeCenter, "Distribute Vertical Centers"),
    (Distribution::Bottom, IconName::AlignVerticalDistributeEnd, "Distribute Bottom Edges"),
    (Distribution::Left, IconName::AlignHorizontalDistributeStart, "Distribute Left Edges"),
    (Distribution::HorizontalCenter, IconName::AlignHorizontalDistributeCenter, "Distribute Horizontal Centers"),
    (Distribution::Right, IconName::AlignHorizontalDistributeEnd, "Distribute Right Edges"),
];

fn target_label(target: AlignTarget) -> &'static str {
    match target {
        AlignTarget::Composition => "Composition",
        AlignTarget::Selection => "Selection",
    }
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let target = workspace.state.align_target;
    let selected = workspace.state.selected_layers.len();
    let weak = cx.weak_entity();

    let target_menu = crate::panels::chrome::dropdown("align-target", target_label(target)).dropdown_menu(move |menu, _, _| {
        [AlignTarget::Composition, AlignTarget::Selection].into_iter().fold(menu, |menu, option| {
            let weak = weak.clone();
            menu.item(PopupMenuItem::new(target_label(option)).checked(option == target).on_click(move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    this.state.align_target = option;
                    cx.notify();
                })
                .ok();
            }))
        })
    });

    let align_buttons: Vec<_> = ALIGNMENTS
        .iter()
        .map(|&(alignment, icon, label)| {
            icon_button(("align", alignment as usize), icon, label, false, cx.listener(move |this, _, _, cx| {
                this.state.align_selected(alignment);
                cx.notify();
            }))
            .disabled(selected == 0 || (target == AlignTarget::Selection && selected < 2))
        })
        .collect();
    let distribute_buttons: Vec<_> = DISTRIBUTIONS
        .iter()
        .map(|&(distribution, icon, label)| {
            icon_button(("distribute", distribution as usize), icon, label, false, cx.listener(move |this, _, _, cx| {
                this.state.distribute_selected(distribution);
                cx.notify();
            }))
            .disabled(selected < 3)
        })
        .collect();

    let body = div()
        .p_2p5()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().flex().items_center().gap_2().child("Align Layers to:").child(target_menu))
        .child(div().flex().gap_1().children(align_buttons))
        .child(div().h(px(1.0)).bg(hsla(color::HAIRLINE)))
        .child("Distribute Layers:")
        .child(div().flex().gap_1().children(distribute_buttons));

    panel(PanelId::Align, &["Align"], None, body, workspace, cx)
}
