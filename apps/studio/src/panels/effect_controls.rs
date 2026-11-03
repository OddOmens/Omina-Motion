//! Effect Controls (the Effects tab of the Properties panel): every effect on the selected
//! layer as a card, laid out like the Color tab's corrections. Each parameter scrubs, takes a
//! typed value and keyframes like any other property, and settings with a modest range also
//! get a slider. The Add Effect menu at the bottom lists the effects that suit the layer.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::{AnyElement, Context, ElementId, IntoElement, ParentElement, Styled, div, px};
use studio_core::model::{Id, Layer};
use studio_core::property::PropertyPath;

use super::chrome::{CardHeading, card, card_list, hint};
use super::fields::property_row;
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// Key recording that an effect is collapsed in this panel. Stored with the timeline's expanded rows,
/// but in its own "controls/" namespace so the two panels twirl independently.
fn effect_key(layer: &Layer, effect: Id) -> String {
    format!("controls/{}/{}", layer.id.0, effect.0)
}

/// The icon an effect's card shows, after its category in the Effects panel.
fn category_icon(effect: &str) -> IconName {
    match studio_core::effects::category_of(effect).map(|category| category.name) {
        Some("Blur & Sharpen") => IconName::CircleDashed,
        Some("Transform") => IconName::Crop,
        Some("Color Correction") => IconName::Palette,
        Some("Distort") => IconName::Wind,
        Some("Generate") => IconName::WandSparkles,
        Some("Keying") => IconName::Eraser,
        Some("Noise & Grain") => IconName::Blend,
        Some("Motion") => IconName::MoveHorizontal,
        Some("Retro & Glitch") => IconName::Ghost,
        Some("Transition") => IconName::SquarePlay,
        _ => IconName::Sparkles,
    }
}

/// The line at the top of the tab: which layer's effects these are.
fn layer_line(layer: &Layer) -> impl IntoElement {
    let count = match layer.effects.len() {
        0 => "no effects".to_owned(),
        1 => "1 effect".to_owned(),
        count => format!("{count} effects"),
    };
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .text_size(px(size::SMALL_TEXT))
        .text_color(hsla(color::TEXT))
        .child(Icon::new(IconName::Layers).size(px(12.0)).text_color(hsla(color::MUTED)))
        .child(div().min_w_0().truncate().child(format!("{} · {count}", layer.name)))
}

/// The Add Effect menu: every effect the renderer draws that suits the layer, by category.
fn add_effect_menu(layer: &Layer, cx: &mut Context<Workspace>) -> impl IntoElement {
    let layer_id = layer.id;
    let categories: Vec<(&'static str, Vec<&'static str>)> = studio_core::effects::CATEGORIES
        .iter()
        .filter(|category| studio_core::effects::suits(category, &layer.content))
        .map(|category| (category.name, category.effects.iter().map(|effect| effect.name).filter(|name| studio_render::effects::is_rendered(name)).collect::<Vec<_>>()))
        .filter(|(_, effects)| !effects.is_empty())
        .collect();
    let weak = cx.weak_entity();
    Button::new("fx-add").ghost().small().w_full().icon(Icon::new(IconName::Plus)).label("Add Effect").dropdown_caret(true).dropdown_menu(move |menu, _, _| {
        let mut menu = menu.scrollable(true).max_h(px(420.0));
        for (index, (category, effects)) in categories.iter().enumerate() {
            if index > 0 {
                menu = menu.separator();
            }
            menu = menu.label(*category);
            for &name in effects {
                let weak = weak.clone();
                menu = menu.item(PopupMenuItem::new(name).on_click(move |_, _, cx| {
                    weak.update(cx, |this, cx| {
                        this.state.add_effect(layer_id, name);
                        cx.notify();
                    })
                    .ok();
                }));
            }
        }
        menu
    })
}

/// One effect's controls: the LUT chooser for Apply LUT, then each parameter's row (with its
/// keyframe buttons) and, where it helps, a slider under it.
fn effect_body(layer: Id, effect: &studio_core::model::Effect, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let effect_id = effect.id;
    let mut body = div().flex().flex_col();
    if !studio_render::effects::is_rendered(&effect.name) {
        body = body.child(div().pb_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("This effect is not drawn by the renderer yet; its settings are kept."));
    }
    if effect.name == "Apply LUT" {
        body = body.child(super::color_grade::lut_chooser(layer, effect_id, workspace, cx));
    }
    for parameter in 0..effect.properties.len() {
        let path = PropertyPath::Effect { effect: effect_id, parameter };
        body = body.child(property_row(layer, path, None, workspace, cx));
        // The slider lines up under the value fields, past the stopwatch and the name.
        if let Some(slider) = super::color_grade::effect_slider_track(layer, effect_id, parameter, workspace, cx) {
            body = body.child(div().pl(px(26.0)).pb_1().child(slider));
        }
    }
    body.into_any_element()
}

pub fn body(workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let composition = workspace.state.composition();
    let selected: Vec<&Layer> = workspace.state.selected_layers.iter().filter_map(|id| composition.layer(*id)).collect();
    let [layer] = selected.as_slice() else {
        let message = if selected.is_empty() { "Select a layer to see its effects." } else { "Select a single layer to see its effects." };
        return hint(message).into_any_element();
    };
    let layer_id = layer.id;
    let mut list = card_list("effect-controls").child(layer_line(layer));
    if layer.effects.is_empty() {
        list = list.child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Add an effect here, or drag one from the Effects panel onto a clip."));
    }
    for effect in &layer.effects {
        let effect_id = effect.id;
        let key = effect_key(layer, effect_id);
        // Effects start open; folding one records it.
        let open = !workspace.state.expanded_rows.contains(&key);
        let heading = CardHeading {
            id: ElementId::Name(format!("fx-twirl:{}", effect_id.0).into()),
            icon: Some(category_icon(&effect.name)),
            title: effect.name.clone().into(),
            open,
            muted: !effect.enabled,
            trailing: super::color_grade::effect_buttons(|part| format!("fx-{part}:{}", effect_id.0), layer_id, effect_id, effect.enabled, cx),
        };
        let body = open.then(|| effect_body(layer_id, effect, workspace, cx));
        list = list.child(card(heading, cx.listener(move |this, _, _, cx| {
            this.state.toggle_row(&key);
            cx.notify();
        }), body));
    }
    list.child(add_effect_menu(layer, cx)).into_any_element()
}

/// Puts every parameter of an effect back to its catalog default (keyframing animated ones).
pub(crate) fn reset_effect(workspace: &mut Workspace, layer: Id, effect: Id) {
    let Some(name) = workspace.state.composition().layer(layer).and_then(|l| l.effects.iter().find(|e| e.id == effect)).map(|e| e.name.clone()) else {
        return;
    };
    let Some(definition) = studio_core::effects::find(&name) else { return };
    for (parameter, (_, default)) in definition.parameters.iter().enumerate() {
        let path = PropertyPath::Effect { effect, parameter };
        workspace.state.set_property_component(layer, path, 0, *default);
    }
}
