//! The Effects & Presets panel (first tab): the built-in effect catalog by category.
//! Effects drag onto timeline layers. The second tab is the Flow easing panel.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{
    AppContext as _, Context, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement,
    Styled, Window, div, px,
};

use studio_core::effects::CATEGORIES;

use super::chrome::panel;
use crate::theme::{color, hsla, size};
use crate::workspace::{EffectDrag, PanelId, Workspace};

/// What follows the pointer while an effect is dragged.
struct EffectDragPreview(String);

impl Render for EffectDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded(px(3.0))
            .bg(hsla(color::ROW_SELECTED))
            .border_1()
            .border_color(hsla(color::ACCENT))
            .text_size(px(size::TEXT))
            .text_color(hsla(color::TEXT))
            .child(self.0.clone())
    }
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let tabs = ["Effects & Presets", "Flow"];
    if workspace.ui.tab(PanelId::Effects) == 1 {
        let body = super::flow::body(workspace, cx);
        return panel(PanelId::Effects, &tabs, None, body, workspace, cx);
    }
    let open = &workspace.ui.open_effect_categories;
    let mut list = div().id("effects-list").size_full().overflow_y_scroll().py_1();
    for (category_index, category) in CATEGORIES.iter().enumerate() {
        let (category, effects): (&'static str, _) = (category.name, category.effects);
        let is_open = open.contains(category);
        list = list.child(
            div()
                .id(("effect-category", category_index))
                .h(px(size::ROW_HEIGHT))
                .flex()
                .items_center()
                .gap_1p5()
                .px_2()
                .hover(|style| style.bg(hsla(color::HOVER)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.ui.open_effect_categories.remove(category) {
                        this.ui.open_effect_categories.insert(category);
                    }
                    cx.notify();
                }))
                .child(
                    Icon::new(if is_open { IconName::ChevronDown } else { IconName::ChevronRight })
                        .size(px(12.0))
                        .text_color(hsla(color::MUTED)),
                )
                .child(category),
        );
        if is_open {
            for (effect_index, effect) in effects.iter().map(|definition| definition.name).enumerate() {
                let row = div()
                    .id(("effect", category_index * 100 + effect_index))
                    .h(px(size::ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .pl(px(28.0))
                    .child(Icon::new(IconName::Sparkles).size(px(12.0)).text_color(hsla(color::MUTED)))
                    .child(effect);
                // Effects the renderer does not draw yet are listed but cannot be applied.
                let row = if studio_render::effects::is_rendered(effect) {
                    row.cursor_grab().hover(|style| style.bg(hsla(color::HOVER))).on_drag(EffectDrag { name: effect.to_owned() }, |drag, _, _, cx| {
                        cx.new(|_| EffectDragPreview(drag.name.clone()))
                    })
                } else {
                    row.text_color(hsla(color::MUTED)).child(div().text_size(px(size::SMALL_TEXT)).child("coming soon"))
                };
                list = list.child(row);
            }
        }
    }
    panel(PanelId::Effects, &tabs, None, list, workspace, cx)
}
