//! The Effects panel, opened from the rail: the built-in effects by category. Only effects the
//! renderer draws are listed, and with a clip selected only the ones that change how that clip
//! looks (no colour correction on text, nothing on audio). Each effect can be dragged onto a
//! clip in the timeline, or added to the selected clips with its + button. Categories start
//! folded, so the panel opens as a short list; each one opens with a click.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::{
    AppContext as _, Context, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement,
    Styled, TestSupportExt as _, Window, div, prelude::FluentBuilder as _, px,
};

use studio_core::effects::{CATEGORIES, suits};
use studio_core::model::{Id, LayerContent, LutSource};

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

/// The name of the user's presets category, which also records whether it is unfolded.
pub(crate) const MY_PRESETS: &str = "My Presets";

/// My Presets: the effect presets the user saved (Save as Preset in the Effects tab), each
/// applied by its + (to the selected clips) or by dragging it onto a clip. Double-click a name
/// to rename it; the trash deletes it. Nothing shows until there is a preset.
fn presets_section(workspace: &Workspace, can_apply: bool, cx: &mut Context<Workspace>) -> Vec<gpui_kit::AnyElement> {
    let presets = &workspace.state.libraries.presets;
    if presets.is_empty() {
        return Vec::new();
    }
    let open = workspace.ui.open_effect_categories.contains(MY_PRESETS);
    let mut rows = vec![
        div()
            .id("presets-category")
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
                if !this.ui.open_effect_categories.remove(MY_PRESETS) {
                    this.ui.open_effect_categories.insert(MY_PRESETS);
                }
                cx.notify();
            }))
            .child(Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).size(px(12.0)))
            .child(format!("{MY_PRESETS} ({})", presets.len()))
            .test_support()
            .into_any_element(),
    ];
    if !open {
        return rows;
    }
    for preset in presets {
        let id = preset.id;
        let apply = div()
            .id(gpui_kit::ElementId::Name(format!("preset-apply:{}", id.0).into()))
            .size(px(22.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .text_color(hsla(color::MUTED))
            .hover(|style| style.bg(hsla(color::RAISED)).text_color(hsla(color::TEXT)))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                let layers = this.state.selected_layers.clone();
                this.state.apply_effect_preset(id, &layers);
                cx.notify();
            }))
            .child(Icon::new(IconName::Plus).size(px(14.0)))
            .test_support();
        let delete = super::chrome::icon_button(gpui_kit::ElementId::Name(format!("preset-delete:{}", id.0).into()), IconName::Trash, "Delete preset", false, cx.listener(move |this, _, _, cx| {
            this.state.delete_library_item(id);
            cx.notify();
        }))
        .xsmall();
        let drag = EffectDrag { name: preset.name.clone(), lut: None, preset: Some(id) };
        rows.push(
            div()
                .id(gpui_kit::ElementId::Name(format!("preset:{}", id.0).into()))
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
                .on_drag(drag, |drag, _, _, cx| cx.new(|_| EffectDragPreview(drag.name.clone())))
                .child(Icon::new(IconName::Bookmark).size(px(12.0)).text_color(hsla(color::MUTED)))
                .child(super::libraries::name_label(id, &preset.name, workspace, cx))
                .child(div().flex_none().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(preset.summary()))
                .when(can_apply, |row| row.child(apply))
                .child(delete)
                .test_support()
                .into_any_element(),
        );
    }
    rows
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
    // The user's own presets first, the effects they reach for most.
    list = list.children(presets_section(workspace, !selected.is_empty(), cx));
    // Transitions next: dragged onto the start or end of a clip, they go on that cut.
    let pictures = selected.iter().all(|(_, _, content)| !matches!(content, LayerContent::Audio { .. }));
    if pictures {
        let open = workspace.ui.open_effect_categories.contains("Transitions");
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
                    if !this.ui.open_effect_categories.remove("Transitions") {
                        this.ui.open_effect_categories.insert("Transitions");
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
            .filter(|name| super::effect_controls::is_usable(name))
            .collect();
        if effects.is_empty() || !offered(category) {
            continue;
        }
        shown += effects.len();
        let name = category.name;
        let is_open = workspace.ui.open_effect_categories.contains(name);
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
                    if !this.ui.open_effect_categories.remove(name) {
                        this.ui.open_effect_categories.insert(name);
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
    // LUTs: Studio's looks and the imported ones, each an Apply LUT effect with that table.
    let grading = CATEGORIES.iter().find(|category| category.name == "Color Correction").is_some_and(offered) && pictures;
    if grading {
        shown += 1;
        list = list.child(lut_category(workspace, !selected.is_empty(), cx));
    }
    if shown == 0 {
        list = list.child(note("Audio clips have no picture, so no effects apply to them.".to_owned()));
    }
    panel(PanelId::Effects, &tabs, None, list, workspace, cx)
}

/// The LUTs category: each look and imported LUT, then Import.
fn lut_category(workspace: &Workspace, can_apply: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    let open = workspace.ui.open_effect_categories.contains("LUTs");
    let heading = div()
        .id("luts-category")
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
            if !this.ui.open_effect_categories.remove("LUTs") {
                this.ui.open_effect_categories.insert("LUTs");
            }
            cx.notify();
        }))
        .child(Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).size(px(12.0)))
        .child("LUTs")
        .test_support();
    let mut category = div().child(heading);
    if open {
        let looks = studio_core::grade::looks().map(|look| LutSource::Look(look.to_owned()));
        let files = workspace.ui.user_luts.iter().map(|path| LutSource::File(path.clone()));
        for (index, source) in looks.chain(files).enumerate() {
            category = category.child(lut_row(index, source, can_apply, workspace, cx));
        }
        category = category.child(
            div().px_3().py_1().child(
                Button::new("import-lut").ghost().small().w_full().icon(Icon::new(IconName::FileInput)).label("Import .cube LUT…").on_click(cx.listener(|_, _, window, cx| {
                    super::color_grade::import_lut(super::color_grade::LutImport::Selection, window, cx);
                })),
            ),
        );
    }
    category
}

/// One LUT, with a picture of what it does to the current frame: drag it onto a clip, or
/// press + to add it to the selected clips. While the pointer is over it, the viewer shows it
/// on the selected clips (or the whole picture when none is selected).
fn lut_row(index: usize, source: LutSource, can_apply: bool, workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let name = source.name();
    let thumbnail = super::color_grade::lut_thumbnail(Some(&source), px(48.0), px(27.0), workspace);
    let drag = EffectDrag { name: "Apply LUT".to_owned(), lut: Some(source.clone()), preset: None };
    let hovered_source = source.clone();
    let apply = div()
        .id(("lut-apply", index))
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
                this.state.add_lut(layer, source.clone());
            }
            cx.notify();
        }))
        .child(Icon::new(IconName::Plus).size(px(14.0)))
        .test_support();
    div()
        .id(("lut", index))
        .h(px(34.0))
        .mx_1()
        .flex()
        .items_center()
        .gap_2()
        .pl(px(20.0))
        .pr_1()
        .rounded(px(6.0))
        .cursor_grab()
        .hover(|style| style.bg(hsla(color::HOVER)))
        .on_drag(drag, |drag, _, _, cx| cx.new(|_| EffectDragPreview(drag.lut.as_ref().map_or_else(|| drag.name.clone(), LutSource::name))))
        .on_hover(cx.listener({
            let source = hovered_source;
            move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.ui.lut_hover = Some(source.clone());
                } else if this.ui.lut_hover.as_ref() == Some(&source) {
                    this.ui.lut_hover = None;
                }
                cx.notify();
            }
        }))
        .child(thumbnail)
        .child(div().flex_1().min_w_0().truncate().child(name))
        .when(can_apply, |row| row.child(apply))
        .test_support()
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
            // Its settings show where it is edited: color in the Color tab, the rest in Effects.
            this.ui.tabs.insert(crate::workspace::PanelId::Properties, super::color_grade::tab_for_effect(effect));
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
        .on_drag(EffectDrag { name: effect.to_owned(), lut: None, preset: None }, |drag, _, _, cx| cx.new(|_| EffectDragPreview(drag.name.clone())))
        .child(Icon::new(IconName::Sparkles).size(px(12.0)).text_color(hsla(color::MUTED)))
        .child(div().flex_1().child(effect))
        .when(can_apply, |row| row.child(apply))
        .test_support()
}
