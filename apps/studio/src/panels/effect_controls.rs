//! Effect Controls (the second tab of the Properties panel): every effect on the
//! selected layer with its parameters. Parameters scrub, take typed values and
//! keyframe like any other property.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
    div, prelude::FluentBuilder as _, px,
};
use studio_core::model::Layer;
use studio_core::property::PropertyPath;

use super::chrome::{hint, icon_button};
use super::fields::property_row;
use crate::theme::{color, hsla};
use crate::workspace::Workspace;

/// Key recording that an effect is collapsed in this panel. Stored with the timeline's expanded rows,
/// but in its own "controls/" namespace so the two panels twirl independently.
fn effect_key(layer: &Layer, effect: studio_core::model::Id) -> String {
    format!("controls/{}/{}", layer.id.0, effect.0)
}

pub fn body(workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let composition = workspace.state.composition();
    let selected: Vec<&Layer> = workspace.state.selected_layers.iter().filter_map(|id| composition.layer(*id)).collect();
    let [layer] = selected.as_slice() else {
        return hint("Select a layer to see its effects.").into_any_element();
    };
    if layer.effects.is_empty() {
        return hint("No effects. Drag one from Effects & Presets onto a layer in the timeline.").into_any_element();
    }

    let layer_id = layer.id;
    let mut list = div().id("effect-controls").size_full().overflow_y_scroll().px_2p5().py_1();
    for effect in &layer.effects {
        let effect_id = effect.id;
        let key = effect_key(layer, effect_id);
        // Effects start expanded; twirling one closed records it.
        let collapsed = workspace.state.expanded_rows.contains(&key);
        let header = div()
            .h(px(28.0))
            .flex()
            .items_center()
            .gap_1p5()
            .child(icon_button(ElementId::Name(format!("fx-on:{}", effect_id.0).into()), if effect.enabled { IconName::Eye } else { IconName::EyeOff }, if effect.enabled { "Hide Effect" } else { "Show Effect" }, false, cx.listener(move |this, _, _, cx| {
                this.state.toggle_effect(layer_id, effect_id);
                cx.notify();
            })))
            .child(
                div()
                    .id(ElementId::Name(format!("fx-twirl:{}", effect_id.0).into()))
                    .flex()
                    .items_center()
                    .gap_1()
                    .cursor_pointer()
                    .on_click(cx.listener({
                        let key = key.clone();
                        move |this, _, _, cx| {
                            this.state.toggle_row(&key);
                            cx.notify();
                        }
                    }))
                    .child(Icon::new(if collapsed { IconName::ChevronRight } else { IconName::ChevronDown }).size(px(12.0)).text_color(hsla(color::MUTED)))
                    .child(div().when(!effect.enabled, |name| name.text_color(hsla(color::MUTED))).child(effect.name.clone())),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id(ElementId::Name(format!("fx-reset:{}", effect_id.0).into()))
                    .text_color(hsla(color::VALUE))
                    .cursor_pointer()
                    .hover(|style| style.text_color(hsla(color::TEXT)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        reset_effect(this, layer_id, effect_id);
                        cx.notify();
                    }))
                    .child("Reset"),
            )
            .child(icon_button(ElementId::Name(format!("fx-remove:{}", effect_id.0).into()), IconName::Trash, "Remove Effect", false, cx.listener(move |this, _, _, cx| {
                this.state.remove_effect(layer_id, effect_id);
                cx.notify();
            })));

        list = list.child(header);
        if !studio_render::effects::is_rendered(&effect.name) {
            list = list.child(div().pl_5().pb_1().text_color(hsla(color::MUTED)).child("This effect is not drawn by the renderer yet; its settings are kept."));
        }
        if !collapsed && effect.name == "Apply LUT" {
            list = list.child(div().pl_5().child(super::color_grade::lut_chooser(layer_id, effect_id, workspace, cx)));
        }
        if !collapsed {
            for parameter in 0..effect.properties.len() {
                let path = PropertyPath::Effect { effect: effect_id, parameter };
                list = list.child(div().pl_5().child(property_row(layer_id, path, None, workspace, cx)));
            }
        }
        list = list.child(div().h(px(1.0)).my_1().bg(hsla(color::HAIRLINE)));
    }
    list.into_any_element()
}

/// Puts every parameter of an effect back to its catalog default (keyframing animated ones).
pub(crate) fn reset_effect(workspace: &mut Workspace, layer: studio_core::model::Id, effect: studio_core::model::Id) {
    let Some(name) = workspace.state.composition().layer(layer).and_then(|l| l.effects.iter().find(|e| e.id == effect)).map(|e| e.name.clone()) else {
        return;
    };
    let Some(definition) = studio_core::effects::find(&name) else { return };
    for (parameter, (_, default)) in definition.parameters.iter().enumerate() {
        let path = PropertyPath::Effect { effect, parameter };
        workspace.state.set_property_component(layer, path, 0, *default);
    }
}
