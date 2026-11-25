//! Effect Controls (the Effects tab of the Properties panel): every effect on the selected
//! layer as a card, laid out like the Color tab's corrections. Each parameter scrubs, takes a
//! typed value and keyframes like any other property, and settings with a modest range also
//! get a slider. The Add Effect menu at the bottom lists the effects that suit the layer.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::{AnyElement, Context, ElementId, InteractiveElement as _, IntoElement, ParentElement, Styled, TestSupportExt as _, div, prelude::FluentBuilder as _, px, rgba};
use studio_core::model::{Id, Layer};
use studio_core::property::PropertyPath;

use super::chrome::{CardHeading, card, card_list, hint, icon_button, list_row};
use super::fields::property_row;
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// Key recording that an effect is collapsed in this panel. Stored with the timeline's expanded rows,
/// but in its own "controls/" namespace so the two panels twirl independently.
fn effect_key(layer: &Layer, effect: Id) -> String {
    format!("controls/{}/{}", layer.id.0, effect.0)
}

/// Whether Studio can use the catalog effect `name`: the renderer draws it, or the mixer runs
/// it on sound. Effects that do neither yet aren't offered.
pub(crate) fn is_usable(name: &str) -> bool {
    studio_render::effects::is_rendered(name) || studio_core::effects::is_audio(name)
}

/// The icon an effect's card shows, after its category in the Effects panel.
pub(super) fn category_icon(effect: &str) -> IconName {
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
        Some(studio_core::effects::AUDIO_CATEGORY) => IconName::AudioLines,
        _ => IconName::Sparkles,
    }
}

/// The line at the top of the tab: which layer's effects these are, the button that opens
/// them in the Nodes panel, and the Add Effect menu.
fn layer_line(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let count = match layer.effects.len() {
        0 => "no effects".to_owned(),
        1 => "1 effect".to_owned(),
        count => format!("{count} effects"),
    };
    list_row()
        .flex()
        .items_center()
        .gap_1p5()
        .text_size(px(size::SMALL_TEXT))
        .text_color(hsla(color::TEXT))
        .child(Icon::new(IconName::Layers).size(px(12.0)).text_color(hsla(color::MUTED)))
        .child(div().flex_1().min_w_0().truncate().child(format!("{} · {count}", layer.name)))
        .child(save_preset_button(layer, cx))
        .child(icon_button("fx-open-nodes", IconName::Workflow, "Edit as Nodes: wire effects together in the Nodes panel", workspace.ui.nodes.open, cx.listener(|this, _, _, cx| {
            this.ui.nodes.open = !this.ui.nodes.open;
            // Opened, it comes to the front of its tab group, wherever it has been moved to.
            if this.ui.nodes.open {
                this.reveal(crate::workspace::PanelId::Nodes);
            }
            cx.notify();
        })))
        .child(add_effect_menu("fx-add".into(), layer, cx))
}

/// Saves the layer's effects (color corrections and sound effects included) as one of the
/// user's presets, and shows it under My Presets in the Effects panel, named after the layer
/// and ready to be renamed.
fn save_preset_button(layer: &Layer, cx: &mut Context<Workspace>) -> impl IntoElement {
    use gpui_kit::component::Disableable as _;
    let (layer_id, name, empty) = (layer.id, format!("{} Look", layer.name), layer.effects.is_empty());
    icon_button("fx-save-preset", IconName::BookmarkPlus, "Save as Preset: keep these effects to use on other clips", false, cx.listener(move |this, _, window, cx| {
        if let Some(id) = this.state.save_effect_preset(layer_id, &name) {
            this.ui.left_panel = crate::workspace::LeftPanel::Effects;
            this.ui.open_effect_categories.insert(super::effects::MY_PRESETS);
            let named = this.state.libraries.preset(id).map(|preset| preset.name.clone()).unwrap_or_default();
            this.begin_edit(crate::workspace::EditTarget::LibraryName(id), named, window, cx);
        }
        cx.notify();
    }))
    .disabled(empty)
}

/// The Add Effect menu: every effect Studio can use (on the picture or the sound) that suits
/// the layer, by category.
fn add_effect_menu(id: ElementId, layer: &Layer, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let layer_id = layer.id;
    let categories: Vec<(&'static str, Vec<&'static str>)> = studio_core::effects::CATEGORIES
        .iter()
        .filter(|category| studio_core::effects::suits(category, &layer.content))
        .map(|category| (category.name, category.effects.iter().map(|effect| effect.name).filter(|name| is_usable(name)).collect::<Vec<_>>()))
        .filter(|(_, effects)| !effects.is_empty())
        .collect();
    let weak = cx.weak_entity();
    Button::new(id).outline().small().icon(Icon::new(IconName::Plus)).tooltip("Add Effect (or drag one from the Effects panel onto a clip)").dropdown_menu(move |menu, _, _| {
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
pub(super) fn effect_body(layer: Id, effect: &studio_core::model::Effect, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let effect_id = effect.id;
    let mut body = div().flex().flex_col();
    if !is_usable(&effect.name) {
        body = body.child(div().pb_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("This effect is not drawn by the renderer yet; its settings are kept."));
    }
    if effect.name == "Apply LUT" {
        body = body.child(super::color_grade::lut_chooser(layer, effect_id, workspace, cx));
    }
    if effect.name == "Curves" {
        body = body.child(super::curves::editor(layer, effect_id, workspace, cx));
    }
    if effect.name == "Keyer" {
        body = body.child(key_color_row(layer, effect_id, workspace, cx));
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

/// The Keyer's key color: a swatch of the color it removes and an eyedropper that waits for a
/// click on the screen in the viewer (which shows the picture without this key meanwhile). The
/// color's channels are also the effect's first three parameters, typed or keyframed below.
fn key_color_row(layer: Id, effect: Id, workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let picking = workspace.ui.key_color_pick == Some((layer, effect));
    let swatch = workspace.state.key_color(layer, effect).map_or(0x000000ff, |color| color.to_rgba_u32());
    let picker = Button::new(ElementId::Name(format!("keyer-pick:{}", effect.0).into()))
        .xsmall()
        .icon(Icon::new(IconName::Pipette))
        .tooltip("Pick the screen's color in the viewer")
        .map(|button| if picking { button.primary() } else { button.ghost() })
        .on_click(cx.listener(move |this, _, _, cx| {
            this.ui.key_color_pick = if this.ui.key_color_pick == Some((layer, effect)) { None } else { Some((layer, effect)) };
            cx.notify();
        }));
    div()
        .flex()
        .flex_col()
        .pb_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .h(px(24.0))
                .child(div().flex_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::TEXT)).child("Key Color"))
                .child(div().w(px(36.0)).h(px(16.0)).rounded(px(3.0)).border_1().border_color(hsla(color::HAIRLINE)).bg(rgba(swatch)))
                .child(picker),
        )
        .children(picking.then(|| div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::ACCENT)).child("Now click the green or blue screen in the viewer.")))
}

pub fn body(workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let composition = workspace.state.composition();
    let selected: Vec<&Layer> = workspace.state.selected_layers.iter().filter_map(|id| composition.layer(*id)).collect();
    let [layer] = selected.as_slice() else {
        let message = if selected.is_empty() { "Select a layer to see its effects." } else { "Select a single layer to see its effects." };
        return hint(message).into_any_element();
    };
    let layer_id = layer.id;
    let mut list = card_list("effect-controls").child(layer_line(layer, workspace, cx));
    if layer.effects.is_empty() {
        list = list.child(list_row().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("No effects yet. Press + to add one."));
    }
    // The node picked in the Nodes panel is marked here and kept open.
    let picked = workspace.ui.nodes.selected;
    // Color effects show as one line, where the first of them runs, instead of a card each:
    // they are edited in the Color tab.
    let graded = super::color_grade::graded_effects(workspace, layer_id);
    let first_graded = layer.effects.iter().position(|effect| graded.contains(&effect.id));
    for (index, effect) in layer.effects.iter().enumerate() {
        let effect_id = effect.id;
        if graded.contains(&effect_id) {
            if Some(index) == first_graded {
                list = list.child(color_line(graded.len(), cx));
            }
            continue;
        }
        let key = effect_key(layer, effect_id);
        let is_picked = picked == Some(studio_core::effect_graph::NodeId::Effect(effect_id));
        // Effects start open; folding one records it.
        let open = is_picked || !workspace.state.expanded_rows.contains(&key);
        let heading = CardHeading {
            id: ElementId::Name(format!("fx-twirl:{}", effect_id.0).into()),
            icon: Some(category_icon(&effect.name)),
            title: effect.name.clone().into(),
            info: None,
            open,
            muted: !effect.enabled,
            trailing: super::color_grade::effect_buttons(|part| format!("fx-{part}:{}", effect_id.0), layer_id, effect_id, effect.enabled, cx),
        };
        let body = open.then(|| effect_body(layer_id, effect, workspace, cx));
        let card = card(heading, cx.listener(move |this, _, _, cx| {
            this.state.toggle_row(&key);
            cx.notify();
        }), body);
        list = list.child(div().when(is_picked, |wrap| wrap.border_l_2().border_color(hsla(color::ACCENT))).child(card));
    }
    // Merge nodes exist only in the node graph; their settings show here too.
    for merge in &layer.effect_graph.merges {
        list = list.child(super::nodes::merge_card(layer_id, merge, picked == Some(studio_core::effect_graph::NodeId::Merge(merge.id)), workspace, cx));
    }
    list.into_any_element()
}

/// The single line standing for the Color tab's corrections, with a button that opens it.
fn color_line(count: usize, cx: &mut Context<Workspace>) -> impl IntoElement {
    let label = if count == 1 { "1 correction".to_owned() } else { format!("{count} corrections") };
    list_row()
        .id("fx-color-line")
        .flex()
        .items_center()
        .gap_1p5()
        .border_b_1()
        .border_color(hsla(color::HAIRLINE))
        .text_size(px(size::SMALL_TEXT))
        .child(Icon::new(IconName::Palette).size(px(13.0)).text_color(hsla(color::MUTED)))
        .child(div().text_color(hsla(color::TEXT)).child("Color"))
        .child(div().flex_1().min_w_0().truncate().text_color(hsla(color::MUTED)).child(label))
        .child(Button::new("fx-open-color").ghost().small().label("Edit in Color").on_click(cx.listener(|this, _, _, cx| {
            this.ui.tabs.insert(crate::workspace::PanelId::Properties, 2);
            cx.notify();
        })))
        .test_support()
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
    // Curves go back to straight lines.
    if name == "Curves" {
        workspace.state.set_effect_lut(layer, effect, Some(studio_core::model::LutSource::Curves(Default::default())));
    }
}
