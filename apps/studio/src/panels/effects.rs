//! The Effects panel, opened from the rail: the built-in effects by category. Only effects the
//! renderer draws are listed, and with a clip selected only the ones that change how that clip
//! looks (no colour correction on text, nothing on audio). Each effect can be dragged onto a
//! clip in the timeline, or added to the selected clips with its + button.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{
    AppContext as _, Context, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement,
    Styled, TestSupportExt as _, Window, div, prelude::FluentBuilder as _, px,
};

use studio_core::effects::{CATEGORIES, suits};
use studio_core::model::{Id, LayerContent};

use super::chrome::panel;
use crate::theme::{color, hsla, size};
use crate::workspace::{EffectDrag, PanelId, TransitionDrag, Workspace};

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

/// The selected clips, with what they show, in selection order.
fn selected_clips(workspace: &Workspace) -> Vec<(Id, String, LayerContent)> {
    let composition = workspace.state.composition();
    workspace
        .state
        .selected_layers
        .iter()
        .filter_map(|&id| composition.layer(id).map(|layer| (id, layer.name.clone(), layer.content.clone())))
        .collect()
}

/// A line of muted text under the panel's tab, saying what the list is for.
fn note(text: String) -> impl IntoElement {
    div().px_3().py_2().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(text)
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let tabs = ["Effects"];
    let selected = selected_clips(workspace);
    // An effect is offered when it suits every selected clip (or always, with none selected).
    let offered = |category| selected.iter().all(|(_, _, content)| suits(category, content));
    let mut list = div().id("effects-list").size_full().overflow_y_scroll().pb_2();
    list = list.child(note(match selected.as_slice() {
        [] => "Select a clip to apply effects, or drag one onto a clip.".to_owned(),
        [(_, name, _)] => format!("Effects for {name}"),
        clips => format!("Effects for {} clips", clips.len()),
    }));

    // Transitions first: dragged onto the start or end of a clip, they go on that cut.
    let pictures = selected.iter().all(|(_, _, content)| !matches!(content, LayerContent::Audio { .. }));
    if pictures {
        let open = !workspace.ui.closed_effect_categories.contains("Transitions");
        list = list.child(
            div()
                .id("transitions-category")
                .h(px(size::ROW_HEIGHT + 4.0))
                .mx_1()
                .flex()
                .items_center()
                .gap_1p5()
                .px_2()
                .rounded(px(6.0))
                .text_color(hsla(color::MUTED))
                .hover(|style| style.bg(hsla(color::HOVER)))
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.ui.closed_effect_categories.remove("Transitions") {
                        this.ui.closed_effect_categories.insert("Transitions");
                    }
                    cx.notify();
                }))
                .child(Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).size(px(12.0)))
                .child("Transitions")
                .test_support(),
        );
        if open {
            for (index, transition) in studio_core::transitions::Transition::ALL.into_iter().enumerate() {
                list = list.child(transition_row(index, transition, !selected.is_empty(), cx));
            }
        }
    }

    let mut shown = 0;
    for (category_index, category) in CATEGORIES.iter().enumerate() {
        let effects: Vec<&'static str> = category
            .effects
            .iter()
            .map(|definition| definition.name)
            .filter(|name| studio_render::effects::is_rendered(name))
            .collect();
        if effects.is_empty() || !offered(category) {
            continue;
        }
        shown += effects.len();
        let name = category.name;
        let is_open = !workspace.ui.closed_effect_categories.contains(name);
        list = list.child(
            div()
                .id(("effect-category", category_index))
                .h(px(size::ROW_HEIGHT + 4.0))
                .mx_1()
                .flex()
                .items_center()
                .gap_1p5()
                .px_2()
                .rounded(px(6.0))
                .text_color(hsla(color::MUTED))
                .hover(|style| style.bg(hsla(color::HOVER)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.ui.closed_effect_categories.remove(name) {
                        this.ui.closed_effect_categories.insert(name);
                    }
                    cx.notify();
                }))
                .child(Icon::new(if is_open { IconName::ChevronDown } else { IconName::ChevronRight }).size(px(12.0)))
                .child(name)
                .test_support(),
        );
        if is_open {
            for effect in effects {
                list = list.child(effect_row(effect, !selected.is_empty(), cx));
            }
        }
    }
    if shown == 0 {
        list = list.child(note("Audio clips have no picture, so no effects apply to them.".to_owned()));
    }
    panel(PanelId::Effects, &tabs, None, list, workspace, cx)
}

/// How long transitions added from the panel take, in seconds.
const TRANSITION_SECONDS: f32 = 0.6;

/// One transition: drag it onto the start or end of a clip, or press + to put it at the start
/// of the selected clips.
fn transition_row(index: usize, transition: studio_core::transitions::Transition, can_apply: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    let apply = div()
        .id(("transition-apply", index))
        .size(px(22.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .text_color(hsla(color::MUTED))
        .hover(|style| style.bg(hsla(color::RAISED)).text_color(hsla(color::TEXT)))
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            for layer in this.state.selected_layers.clone() {
                this.state.add_transition(layer, true, transition, TRANSITION_SECONDS);
            }
            cx.notify();
        }))
        .child(Icon::new(IconName::Plus).size(px(14.0)))
        .test_support();
    div()
        .id(("transition", index))
        .h(px(size::ROW_HEIGHT + 4.0))
        .mx_1()
        .flex()
        .items_center()
        .gap_2()
        .pl(px(26.0))
        .pr_1()
        .rounded(px(6.0))
        .cursor_grab()
        .hover(|style| style.bg(hsla(color::HOVER)))
        .on_drag(TransitionDrag { transition }, |drag, _, _, cx| cx.new(|_| EffectDragPreview(drag.transition.name().to_owned())))
        .child(Icon::new(IconName::Blend).size(px(12.0)).text_color(hsla(color::MUTED)))
        .child(div().flex_1().child(transition.name()))
        .when(can_apply, |row| row.child(apply))
        .test_support()
}

/// One effect: drag it onto a clip, or press + to add it to the selected clips.
fn effect_row(effect: &'static str, can_apply: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    let apply = div()
        .id(format!("effect-apply:{effect}"))
        .size(px(22.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .text_color(hsla(color::MUTED))
        .hover(|style| style.bg(hsla(color::RAISED)).text_color(hsla(color::TEXT)))
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            for layer in this.state.selected_layers.clone() {
                this.state.add_effect(layer, effect);
            }
            cx.notify();
        }))
        .child(Icon::new(IconName::Plus).size(px(14.0)))
        .test_support();
    div()
        .id(format!("effect:{effect}"))
        .group("effect-row")
        .h(px(size::ROW_HEIGHT + 4.0))
        .mx_1()
        .flex()
        .items_center()
        .gap_2()
        .pl(px(26.0))
        .pr_1()
        .rounded(px(6.0))
        .cursor_grab()
        .hover(|style| style.bg(hsla(color::HOVER)))
        .on_drag(EffectDrag { name: effect.to_owned() }, |drag, _, _, cx| cx.new(|_| EffectDragPreview(drag.name.clone())))
        .child(Icon::new(IconName::Sparkles).size(px(12.0)).text_color(hsla(color::MUTED)))
        .child(div().flex_1().child(effect))
        .when(can_apply, |row| row.child(apply))
        .test_support()
}
