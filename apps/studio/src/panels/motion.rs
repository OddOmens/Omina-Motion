//! The Motion tab of the right panel: everything about how things move. The graph editor
//! (Keyframe Easing) for the keyframes selected in the timeline; the layer's In, Out and Loop
//! animations, each In and Out with an easing to pick or a curve to drag; for text, the
//! animators that bring it in a character, word or line at a time; and, for shapes, morphing
//! (see `morph`).

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::{
    AnyElement, Context, Div, ElementId, InteractiveElement, IntoElement, ParentElement, Styled, TestSupportExt as _, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};
use studio_core::animation::{EASING_FAMILIES, EaseMode, Easing};
use studio_core::model::{Id, Layer, LayerContent};
use studio_core::presets::Phase;

use super::chrome::{card_list, heading_button, hint};
use super::fields::{FieldTarget, value_field};
use super::graph_editor::{CurveTarget, curve_editor, phase_key};
use super::properties::{add_note, choice_menu, fold, labeled_row};
use crate::theme::{color, hsla};
use crate::workspace::Workspace;

/// The Motion tab's place among the right panel's tabs (after Properties, Effects and Color).
pub const TAB: usize = 3;

/// What each section is for, shown by the info button on its heading.
const EASING_INFO: &str = "Eases the keyframes selected in the timeline. Pick a curve from the list, or drag the two handles to shape your own. Click the line between two keyframes, or double-click one, to pick it.";
const ANIMATE_INFO: &str = "One-click ways for the layer to come in, go out, and move in between. Each In and Out has an easing: pick one, or drag the curve's handles.";
const TEXT_ANIMATION_INFO: &str = "Brings the text in, or takes it out, a character, word or line at a time, as After Effects' text animators do. The duration covers every piece; overlap sets how much each piece's move runs into the next one's.";

/// The Motion tab: Keyframe Easing always (it works on keyframes of any layer), then the
/// selected layer's Animate and, for text, Text Animation sections.
pub fn body(workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let composition = workspace.state.composition();
    let selected: Vec<&Layer> = workspace.state.selected_layers.iter().filter_map(|id| composition.layer(*id)).collect();
    if selected.is_empty() {
        return hint("Select a layer to animate it.").into_any_element();
    }
    let layer = match selected.as_slice() {
        [layer] => Some(*layer),
        _ => None,
    };
    let easing = fold("easing", "Keyframe Easing", Some(EASING_INFO), [None, None], workspace, cx, |cx| super::graph_editor::keyframes_section(workspace, cx));
    card_list("motion-body")
        .child(easing)
        .children(layer.filter(|layer| layer.transform.is_some()).map(|layer| fold("animate", "Animate", Some(ANIMATE_INFO), animate_buttons(layer, cx), workspace, cx, |cx| animate_section(layer, cx))))
        .children(layer.filter(|layer| matches!(layer.content, LayerContent::Text { .. })).map(|layer| fold("textanim", "Text Animation", Some(TEXT_ANIMATION_INFO), text_animation_buttons(layer, cx), workspace, cx, |cx| text_animation_section(layer, workspace, cx))))
        .children(layer.filter(|layer| matches!(layer.content, LayerContent::Shape { .. })).map(|layer| fold("morph", "Morph", Some(super::morph::MORPH_INFO), [None, None], workspace, cx, |cx| super::morph::section(layer, workspace, cx))))
        .children((selected.len() == 2).then(|| fold("morph", "Morph", Some(super::morph::MORPH_INFO), [None, None], workspace, cx, |cx| super::morph::pair_section(workspace, cx))))
        .children(layer.is_none().then(|| hint("Select a single layer to give it In, Out and Loop animations.")))
        .test_support()
        .into_any_element()
}

/// Ready-made easings for an In or Out motion, by name: each family in the form that suits
/// the end (settling at the end for In, speeding away for Out), and an even Linear pace.
fn motion_easings(phase: Phase) -> Vec<(&'static str, Easing)> {
    let family = |name: &str| EASING_FAMILIES.iter().find(|(family, _)| *family == name).map(|(_, forms)| *forms);
    // The family's In, Out and Both forms are at 0, 1 and 2.
    let end = if phase == Phase::In { 1 } else { 0 };
    let mode = if phase == Phase::In { EaseMode::Out } else { EaseMode::In };
    let mut easings = Vec::new();
    for (label, name, form) in [("Smooth", "Cubic", 2), ("Gentle", "Sine", end), ("Snappy", "Expo", end), ("Overshoot", "Back", end)] {
        if let Some(forms) = family(name) {
            easings.push((label, Easing::Curve(forms[form])));
        }
    }
    easings.extend([("Spring", Easing::Elastic(mode)), ("Bounce", Easing::Bounce(mode)), ("Linear", Easing::Linear)]);
    easings
}

/// Under an In or Out motion: its easing as a row of choices (Default is the motion's own),
/// and its curve, whose handles drag to shape a custom one.
fn easing_block(layer: &Layer, phase: Phase, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let key = phase_key(phase);
    let picked = match phase {
        Phase::In => layer.animation.enter_easing,
        Phase::Out => layer.animation.exit_easing,
    };
    let shown = studio_core::presets::motion_easing(&layer.animation, phase).unwrap_or(Easing::Linear);
    let choice = |name: &'static str, easing: Option<Easing>, cx: &mut Context<Workspace>| {
        Button::new(ElementId::Name(format!("motion-{key}-ease-{name}").into()))
            .xsmall()
            .label(name)
            .map(|button| if picked == easing { button.primary() } else { button.ghost() })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.state.set_motion_easing(id, phase, easing);
                cx.notify();
            }))
    };
    let mut choices = vec![choice("Default", None, cx)];
    for (name, easing) in motion_easings(phase) {
        choices.push(choice(name, Some(easing), cx));
    }
    // Lined up under the menus, past the row's label.
    div()
        .pl(px(104.0))
        .pb_2()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(div().flex().flex_wrap().gap_1().children(choices))
        .child(curve_editor(CurveTarget::Motion(phase), shown, 110.0, cx))
}

/// How long an in or out animation can be chosen to take, in seconds.
const ANIMATION_LENGTHS: [f32; 5] = [0.3, 0.5, 0.8, 1.2, 2.0];

/// Animate: one-click ways for the layer to come in, go out, and move in between. The In and
/// Out motions each show their easing under them, to pick from or reshape.
fn animate_section(layer: &Layer, cx: &mut Context<Workspace>) -> Div {
    use studio_core::presets::{Loop, Motion, Phase};
    let id = layer.id;
    let target = cx.weak_entity();
    let animation = layer.animation;
    let mut row = |phase: Phase, current: Option<(Motion, f32)>, key: &'static str, label: &'static str| {
        let mut choices: Vec<(String, bool, Option<Motion>)> = vec![("None".to_owned(), current.is_none(), None)];
        choices.extend(Motion::ALL.iter().map(|&motion| (motion.name().to_owned(), current.is_some_and(|(chosen, _)| chosen == motion), Some(motion))));
        let seconds = current.map_or(0.5, |(_, seconds)| seconds);
        let name = current.map_or("None".to_owned(), |(motion, _)| motion.name().to_owned());
        let motion_menu = choice_menu(format!("animate-{key}"), name, choices, move |this, motion| this.state.set_layer_motion(id, phase, motion, seconds), target.clone());
        let lengths = ANIMATION_LENGTHS.iter().map(|&length| (format!("{length} s"), (length - seconds).abs() < 0.01, length)).collect();
        let chosen = current.map(|(motion, _)| motion);
        let length_menu = choice_menu(format!("animate-{key}-length"), format!("{seconds} s"), lengths, move |this, length| this.state.set_layer_motion(id, phase, chosen, length), target.clone());
        let row = labeled_row(label, div().flex().gap_1().child(div().flex_1().child(motion_menu)).when(current.is_some(), |row| row.child(div().w(px(72.0)).child(length_menu))));
        div().child(row).children(current.is_some().then(|| easing_block(layer, phase, cx)))
    };
    let mut loops: Vec<(String, bool, Option<Loop>)> = vec![("None".to_owned(), animation.looping.is_none(), None)];
    loops.extend(Loop::ALL.iter().map(|&looping| (looping.name().to_owned(), animation.looping == Some(looping), Some(looping))));
    let loop_name = animation.looping.map_or("None".to_owned(), |looping| looping.name().to_owned());
    div()
        .child(row(Phase::In, animation.enter, "in", "In"))
        .child(row(Phase::Out, animation.exit, "out", "Out"))
        .child(labeled_row("Loop", choice_menu("animate-loop", loop_name, loops, move |this, looping| this.state.set_layer_loop(id, looping), target.clone())))
}


/// The Text Animation heading's buttons: Reset removes both animations (greyed out when there
/// are none), and + adds the In or Out one that is missing, with Rise by character to start.
fn text_animation_buttons(layer: &Layer, cx: &mut Context<Workspace>) -> [Option<AnyElement>; 2] {
    use studio_core::presets::Phase;
    use studio_core::text_animation::TextAnimator;
    let id = layer.id;
    let (has_in, has_out) = (layer.animation.text_in.is_some(), layer.animation.text_out.is_some());
    let reset = heading_button("text-animation-reset", IconName::RotateCcw, "Remove Text Animations", false, cx.listener(move |this, _, _, cx| {
        this.state.set_text_animator(id, Phase::In, None);
        this.state.set_text_animator(id, Phase::Out, None);
        cx.notify();
    }))
    .disabled(!has_in && !has_out);
    let weak = cx.weak_entity();
    let add = Button::new("text-animation-add").outline().small().icon(Icon::new(IconName::Plus)).tooltip("Add Text Animation").disabled(has_in && has_out).dropdown_menu(move |menu, _, _| {
        [(Phase::In, "Animate In", has_in), (Phase::Out, "Animate Out", has_out)].into_iter().fold(menu, |menu, (phase, label, present)| {
            let weak = weak.clone();
            menu.item(PopupMenuItem::new(label).disabled(present).on_click(move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    this.state.set_text_animator(id, phase, Some(TextAnimator::default()));
                    cx.notify();
                })
                .ok();
            }))
        })
    });
    [Some(reset.into_any_element()), Some(add.into_any_element())]
}

/// Text Animation: the text's In and Out animators, each with its motion, the pieces it moves
/// in (character, word or line), their order and easing, how long it takes and how much the
/// pieces overlap.
fn text_animation_section(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    use studio_core::presets::Phase;
    let animators = [(Phase::In, "In", layer.animation.text_in), (Phase::Out, "Out", layer.animation.text_out)];
    if animators.iter().all(|(_, _, animator)| animator.is_none()) {
        return div().child(add_note());
    }
    let mut body = div().flex().flex_col().gap_2();
    for (phase, title, animator) in animators {
        if let Some(animator) = animator {
            body = body.child(text_animator_block(layer.id, phase, title, animator, workspace, cx));
        }
    }
    body
}

/// A dropdown setting one choice (`set`) of layer `id`'s text animator for `phase`.
fn animator_menu<T: Clone + 'static>(
    menu_id: String,
    label: &str,
    choices: Vec<(String, bool, T)>,
    (id, phase): (Id, studio_core::presets::Phase),
    set: fn(&mut studio_core::text_animation::TextAnimator, T),
    target: &WeakEntity<Workspace>,
) -> impl IntoElement {
    choice_menu(menu_id, label.to_owned(), choices, move |this: &mut Workspace, value: T| this.state.edit_text_animator(id, phase, |animator| set(animator, value)), target.clone())
}

/// One text animator's settings, under a small heading with its Remove button.
fn text_animator_block(id: Id, phase: studio_core::presets::Phase, title: &'static str, animator: studio_core::text_animation::TextAnimator, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    use studio_core::text_animation::{TextEase, TextMotion, TextOrder, TextUnit};
    let key = match phase {
        studio_core::presets::Phase::In => "in",
        studio_core::presets::Phase::Out => "out",
    };
    let target = cx.weak_entity();
    let motions = TextMotion::ALL.iter().map(|&motion| (motion.name().to_owned(), motion == animator.motion, motion)).collect();
    let orders = TextOrder::ALL.iter().map(|&order| (order.name().to_owned(), order == animator.order, order)).collect();
    let eases = TextEase::ALL.iter().map(|&ease| (ease.name().to_owned(), ease == animator.ease, ease)).collect();
    let menu_id = |name: &str| format!("text-animation-{key}-{name}");
    let motion_menu = animator_menu(menu_id("motion"), animator.motion.name(), motions, (id, phase), |animator, motion| animator.motion = motion, &target);
    let order_menu = animator_menu(menu_id("order"), animator.order.name(), orders, (id, phase), |animator, order| animator.order = order, &target);
    let ease_menu = animator_menu(menu_id("ease"), animator.ease.name(), eases, (id, phase), |animator, ease| animator.ease = ease, &target);
    // By: one segmented switch for the three kinds of piece.
    let units = div().flex().gap_0p5().p_0p5().rounded(px(6.0)).bg(hsla(color::INSET)).children(TextUnit::ALL.map(|unit| {
        Button::new(ElementId::Name(format!("text-animation-{key}-unit-{}", unit.name()).into()))
            .xsmall()
            .label(unit.name())
            .flex_1()
            .map(|button| if unit == animator.unit { button.primary() } else { button.ghost() })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.state.edit_text_animator(id, phase, |animator| animator.unit = unit);
                cx.notify();
            }))
    }));
    let remove = heading_button(ElementId::Name(format!("text-animation-{key}-remove").into()), IconName::Trash, "Remove", false, cx.listener(move |this, _, _, cx| {
        this.state.set_text_animator(id, phase, None);
        cx.notify();
    }));
    div()
        .child(div().flex().items_center().h(px(26.0)).child(div().flex_1().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(title)).child(remove))
        .child(labeled_row("Animate", motion_menu))
        .child(labeled_row("By", units))
        .child(labeled_row("Order", order_menu))
        .child(labeled_row("Easing", ease_menu))
        .child(labeled_row("Duration", value_field(FieldTarget::TextAnimationSeconds { layer: id, phase }, workspace, cx)))
        .child(labeled_row("Overlap", value_field(FieldTarget::TextAnimationOverlap { layer: id, phase }, workspace, cx)))
}

/// The Animate heading's buttons: Reset takes every animation off (greyed out when there is
/// none), and + opens a menu of the In, Out and Loop animations to add.
fn animate_buttons(layer: &Layer, cx: &mut Context<Workspace>) -> [Option<AnyElement>; 2] {
    use studio_core::presets::{Loop, Motion, Phase};
    let id = layer.id;
    let animation = layer.animation;
    let animated = animation.enter.is_some() || animation.exit.is_some() || animation.looping.is_some();
    let reset = heading_button("animate-reset", IconName::RotateCcw, "Remove Animations", false, cx.listener(move |this, _, _, cx| {
        this.state.set_layer_motion(id, Phase::In, None, 0.5);
        this.state.set_layer_motion(id, Phase::Out, None, 0.5);
        this.state.set_layer_loop(id, None);
        cx.notify();
    }))
    .disabled(!animated);
    let weak = cx.weak_entity();
    // One submenu each for In, Out and Loop, so the list stays short.
    let add = Button::new("animate-add").outline().small().icon(Icon::new(IconName::Plus)).tooltip("Add Animation").dropdown_menu(move |menu, window, cx| {
        let mut menu = menu;
        for (phase, title, current) in [(Phase::In, "In", animation.enter), (Phase::Out, "Out", animation.exit)] {
            let weak = weak.clone();
            menu = menu.submenu(title, window, cx, move |mut submenu, _, _| {
                // The length already chosen is kept, or half a second.
                let seconds = current.map_or(0.5, |(_, seconds)| seconds);
                for motion in Motion::ALL {
                    let weak = weak.clone();
                    let chosen = current.is_some_and(|(picked, _)| picked == motion);
                    submenu = submenu.item(PopupMenuItem::new(motion.name()).checked(chosen).on_click(move |_, _, cx| {
                        weak.update(cx, |this, cx| {
                            this.state.set_layer_motion(id, phase, Some(motion), seconds);
                            cx.notify();
                        })
                        .ok();
                    }));
                }
                submenu
            });
        }
        let weak = weak.clone();
        menu.submenu("Loop", window, cx, move |mut submenu, _, _| {
            for looping in Loop::ALL {
                let weak = weak.clone();
                submenu = submenu.item(PopupMenuItem::new(looping.name()).checked(animation.looping == Some(looping)).on_click(move |_, _, cx| {
                    weak.update(cx, |this, cx| {
                        this.state.set_layer_loop(id, Some(looping));
                        cx.notify();
                    })
                    .ok();
                }));
            }
            submenu
        })
    });
    // A menu button handles its own click, so the press is kept from the heading here.
    let add = div().on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(add);
    [Some(reset.into_any_element()), Some(add.into_any_element())]
}
