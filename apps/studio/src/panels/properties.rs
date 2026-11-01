//! The Properties panel (first tab) and Effect Controls (second tab). Properties
//! shows the selected layer's transform and, for text layers, character and
//! paragraph styling. Every value scrubs or can be typed into.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, TestSupportExt as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};
use studio_core::captions::{CaptionAnimation, CaptionStyle, CaptionWord};
use studio_core::model::{BlendMode, Id, Layer, LayerContent, Paint, PaintKind, Rgba, ShapeKind, ShapeStyle, TextAlign, TextStyle, TrackMatte};
use studio_core::property::{PropertyPath, TransformProperty};

use super::chrome::{divider, hint, icon_button, panel, section};
use super::color_picker;
use super::fields::{FieldTarget, property_row, value_field};
use crate::state::{AudioSetting, ColorTarget, PaintSlot, ShapeSetting, Switch, TextSetting};
use crate::theme::{color, hsla, size};
use crate::workspace::{EditTarget, PanelId, Workspace};

/// Font families offered in the Text section, shown only if installed. The layer's own font is always listed.
const FONT_FAMILIES: &[&str] = &[
    "Inter", "SF Pro Display", "Helvetica Neue", "Helvetica", "Arial", "Avenir Next", "Futura", "Gill Sans",
    "Georgia", "Times New Roman", "Baskerville", "Didot", "Menlo", "Courier New", "Impact",
];

const FONT_WEIGHTS: &[(u16, &str)] = &[
    (100, "Thin"),
    (200, "Extra Light"),
    (300, "Light"),
    (400, "Regular"),
    (500, "Medium"),
    (600, "Semibold"),
    (700, "Bold"),
    (800, "Extra Bold"),
    (900, "Black"),
];

/// The offered font families that are installed. Asking the OS is slow, so this runs once at startup.
pub fn installed_font_families(cx: &App) -> Vec<&'static str> {
    let installed = cx.text_system().all_font_names();
    FONT_FAMILIES.iter().copied().filter(|family| installed.iter().any(|name| name == family)).collect()
}

fn weight_name(weight: u16) -> &'static str {
    FONT_WEIGHTS.iter().min_by_key(|(candidate, _)| candidate.abs_diff(weight)).map_or("Regular", |(_, name)| name)
}

/// A dropdown whose choices run `choose` on the workspace.
fn choice_menu<T: Clone + 'static>(
    id: impl Into<ElementId>,
    label: String,
    choices: Vec<(String, bool, T)>,
    choose: impl Fn(&mut Workspace, T) + 'static,
    workspace: WeakEntity<Workspace>,
) -> impl IntoElement {
    let choose = std::rc::Rc::new(choose);
    let choices = std::rc::Rc::new(choices);
    Button::new(id).outline().small().label(label).dropdown_caret(true).w_full().dropdown_menu(move |menu, _, _| {
        choices.iter().fold(menu, |menu, (label, checked, value)| {
            let (workspace, choose, value) = (workspace.clone(), choose.clone(), value.clone());
            menu.item(PopupMenuItem::new(label.clone()).checked(*checked).on_click(move |_, _, cx: &mut App| {
                workspace
                    .update(cx, |this, cx| {
                        choose(this, value.clone());
                        cx.notify();
                    })
                    .ok();
            }))
        })
    })
}

/// A labeled control in the Layer section.
fn labeled_row(label: &'static str, control: impl IntoElement) -> Div {
    div()
        .h(px(30.0))
        .flex()
        .items_center()
        .gap_2()
        .child(div().w(px(96.0)).flex_none().text_color(hsla(color::MUTED)).child(label))
        .child(div().flex_1().min_w_0().child(control))
}

/// How the layer combines with what is below it: blend mode, track matte (the layer above it,
/// so not offered on the top layer), parent, and the solo and effects switches.
fn layer_section(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let target = cx.weak_entity();
    let composition = workspace.state.composition();
    let mut rows = div().child(section("Layer", None));
    if layer.content.has_picture() {
        let modes = BlendMode::ALL.iter().map(|&mode| (mode.label().to_owned(), mode == layer.blend_mode, mode)).collect();
        rows = rows.child(labeled_row("Blend Mode", choice_menu("layer-blend", layer.blend_mode.label().to_owned(), modes, move |this, mode| this.state.set_blend_mode(id, mode), target.clone())));
        let is_top = composition.layers.first().map(|top| top.id) == Some(id);
        if !is_top || layer.track_matte != TrackMatte::None {
            let mattes = TrackMatte::ALL.iter().map(|&matte| (matte.label().to_owned(), matte == layer.track_matte, matte)).collect();
            rows = rows.child(labeled_row("Track Matte", choice_menu("layer-matte", layer.track_matte.label().to_owned(), mattes, move |this, matte| this.state.set_track_matte(id, matte), target.clone())));
        }
        // Only layers that can be the parent: not this one, and none of its children (a loop).
        let mut parents: Vec<(String, bool, Option<Id>)> = vec![("None".to_owned(), layer.parent.is_none(), None)];
        parents.extend(
            composition
                .layers
                .iter()
                .filter(|candidate| studio_core::edit::can_parent(composition, id, candidate.id))
                .map(|candidate| (candidate.name.clone(), layer.parent == Some(candidate.id), Some(candidate.id))),
        );
        let parent_name = layer.parent.and_then(|parent| composition.layer(parent)).map_or("None".to_owned(), |parent| parent.name.clone());
        rows = rows.child(labeled_row("Parent", choice_menu("layer-parent", parent_name, parents, move |this, parent| this.state.set_parent(id, parent), target)));
    }
    let switch = |id_name: &'static str, label: &'static str, on: bool, which: Switch, cx: &mut Context<Workspace>| {
        Checkbox::new(id_name).label(label).checked(on).on_click(cx.listener(move |this, _, _, cx| {
            this.state.toggle_switch(id, which);
            cx.notify();
        }))
    };
    rows.child(
        div()
            .flex()
            .gap_4()
            .py_1()
            .child(switch("layer-solo", "Solo", layer.switches.solo, Switch::Solo, cx))
            .when(layer.content.has_picture(), |row| row.child(switch("layer-effects", "Effects", layer.switches.effects_enabled, Switch::EffectsEnabled, cx))),
    )
    // Clips with sound can be captioned.
    .when(workspace.state.caption_source(id).is_some(), |rows| {
        let busy = workspace.ui.caption_job.is_some();
        rows.child(
            div().py_1().child(
                Button::new("generate-captions")
                    .outline()
                    .small()
                    .icon(Icon::new(IconName::Captions))
                    .label(if busy { "Making captions…" } else { "Generate Captions" })
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| super::captions::generate(this, id, window, cx))),
            ),
        )
    })
}

/// A layer's sound: whether its picture shows and its sound plays (separately), its volume and
/// fades, and for a video clip, moving its sound onto a clip of its own.
fn audio_section(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let field = |setting: AudioSetting, cx: &mut Context<Workspace>| value_field(FieldTarget::Audio { layer: id, setting }, workspace, cx);
    let toggle = |name: &'static str, label: &'static str, on: bool, which: Switch, cx: &mut Context<Workspace>| {
        Checkbox::new(name).label(label).checked(on).on_click(cx.listener(move |this, _, _, cx| {
            this.state.toggle_switch(id, which);
            cx.notify();
        }))
    };
    let is_video = matches!(layer.content, LayerContent::Footage { .. });
    div()
        .child(divider())
        .child(section("Audio", None))
        .child(
            div()
                .flex()
                .gap_4()
                .py_1()
                .when(is_video, |row| row.child(toggle("layer-show-video", "Show video", layer.switches.visible, Switch::Visible, cx)))
                .child(toggle("layer-play-audio", "Play audio", layer.switches.audio, Switch::Audio, cx)),
        )
        .child(labeled_row("Volume", field(AudioSetting::Volume, cx)))
        .child(labeled_row("Fade In", field(AudioSetting::FadeIn, cx)))
        .child(labeled_row("Fade Out", field(AudioSetting::FadeOut, cx)))
        .when(is_video, |section| {
            section.child(
                div().py_1().child(
                    Button::new("separate-audio")
                        .outline()
                        .small()
                        .icon(Icon::new(IconName::AudioLines))
                        .label("Separate Audio")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.state.separate_audio(id).is_some() {
                                cx.notify();
                            }
                        })),
                ),
            )
        })
}

/// A caption layer's settings, for all its captions at once: the animation, how many words
/// show together, capitals, the font and colors, the spoken word's color and a background box,
/// then the words themselves (click a phrase to jump to it, double-click to fix its text).
fn captions_section(layer: &Layer, style: &CaptionStyle, words: &[CaptionWord], workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let weak = cx.weak_entity();
    let animations = CaptionAnimation::ALL.iter().map(|&animation| (animation.name().to_owned(), animation == style.animation, animation)).collect();
    let per_phrase = (1..=6u32).map(|count| (if count == 1 { "1 word".to_owned() } else { format!("{count} words") }, count == style.words_per_phrase, count)).collect();
    let uppercase = Checkbox::new("captions-uppercase").label("All capitals").checked(style.uppercase).on_click(cx.listener(move |this, checked: &bool, _, cx| {
        let checked = *checked;
        this.state.edit_captions(id, |style| style.uppercase = checked);
        cx.notify();
    }));
    let background_on = Checkbox::new("captions-background-on").checked(style.background.is_some()).on_click(cx.listener(move |this, checked: &bool, _, cx| {
        let checked = *checked;
        this.state.edit_captions(id, |style| style.background = checked.then(|| Rgba::hex(0x000000).with_alpha(0.7)));
        cx.notify();
    }));
    let mut families: Vec<&str> = workspace.ui.font_families.clone();
    if !families.contains(&style.text.font_family.as_str()) {
        families.insert(0, style.text.font_family.as_str());
    }
    let weight = style.text.font_weight;
    let family_choices = families.iter().map(|family| ((*family).to_owned(), *family == style.text.font_family, (*family).to_owned())).collect();
    let weight_choices = FONT_WEIGHTS.iter().map(|(value, name)| ((*name).to_owned(), *value == weight, *value)).collect();
    let current_family = style.text.font_family.clone();
    let stroke_width = style.text.stroke.map_or(4.0, |(_, width)| width);

    let fps = workspace.state.composition().frame_rate.fps();
    let rate = workspace.state.composition().frame_rate;
    let start_frame = layer.start_frame;
    let mut transcript = div().flex().flex_col().gap_0p5();
    for range in studio_core::captions::phrases(words, style.words_per_phrase, fps) {
        let text = words[range.clone()].iter().map(|word| word.text.as_str()).collect::<Vec<_>>().join(" ");
        let at = start_frame + words[range.start].start;
        let editing = workspace.ui.editing.as_ref().filter(|editing| editing.target == EditTarget::CaptionText(id, range.start, range.end));
        let row = div()
            .id(("caption-phrase", range.start))
            .flex()
            .items_center()
            .gap_2()
            .px_1()
            .py_0p5()
            .rounded(px(4.0))
            .hover(|style| style.bg(hsla(color::HOVER)))
            .child(div().w(px(70.0)).flex_none().font_family("Menlo").text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(studio_core::time::format_timecode(at, rate)));
        let row = match editing {
            Some(editing) => row.child(div().flex_1().child(Input::new(&editing.input).xsmall())).into_any_element(),
            None => {
                let (start, end, current) = (range.start, range.end, text.clone());
                row.cursor_pointer()
                    .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                        if event.click_count() >= 2 {
                            this.begin_edit(EditTarget::CaptionText(id, start, end), current.clone(), window, cx);
                        } else {
                            this.go_to(at, cx);
                        }
                    }))
                    .child(div().flex_1().min_w_0().child(text))
                    .test_support()
                    .into_any_element()
            }
        };
        transcript = transcript.child(row);
    }

    div()
        .child(divider())
        .child(section("Captions", None))
        .child(labeled_row("Animation", choice_menu("captions-animation", style.animation.name().to_owned(), animations, move |this, animation| this.state.edit_captions(id, |style| style.animation = animation), weak.clone())))
        .child(labeled_row("Words at once", choice_menu("captions-words", format!("{}", style.words_per_phrase), per_phrase, move |this, count| this.state.edit_captions(id, |style| style.words_per_phrase = count), weak.clone())))
        .child(div().py_1().child(uppercase))
        .child(div().h(px(30.0)).child(choice_menu("captions-font-family", style.text.font_family.clone(), family_choices, move |this, family: String| this.state.set_font(id, &family, weight), weak.clone())))
        .child(div().h(px(30.0)).child(choice_menu("captions-font-weight", weight_name(weight).to_owned(), weight_choices, move |this, weight: u16| this.state.set_font(id, &current_family, weight), weak)))
        .child(div().grid().grid_cols(2).gap_x_3().child(setting_field(IconName::ALargeSmall, "Size", id, TextSetting::FontSize, workspace, cx)).child(setting_field(IconName::MoveHorizontal, "Tracking", id, TextSetting::Tracking, workspace, cx)))
        .child(paint_row("fill", "Fill", style.text.fill, ColorTarget::TextFill(id), move |this, fill| this.state.set_text_fill(id, fill), cx))
        .child(paint_row("stroke", "Stroke", style.text.stroke.map(|(color, _)| color), ColorTarget::TextStroke(id), move |this, color| this.state.set_text_stroke(id, color.map(|color| (color, stroke_width))), cx))
        .child(labeled_row("Spoken word", color_picker::swatch("captions-highlight", ColorTarget::CaptionHighlight(id), Some(style.highlight), cx)))
        .child(labeled_row("Background", div().flex().items_center().gap_2().child(background_on).child(color_picker::swatch("captions-background", ColorTarget::CaptionBackground(id), style.background, cx))))
        .child(divider())
        .child(section(format!("Transcript · {} words", words.len()), None))
        .child(transcript)
}

fn transform_section(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let reset = div()
        .id("reset-transform")
        .text_color(hsla(color::VALUE))
        .cursor_pointer()
        .hover(|style| style.text_color(hsla(color::TEXT)))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.state.reset_transform(id);
            cx.notify();
        }))
        .child("Reset")
        .into_any_element();
    let linked = workspace.state.scale_linked;
    let mut rows = div().child(section("Layer Transform", Some(reset)));
    for property in TransformProperty::ALL {
        let extra = (property == TransformProperty::Scale).then(|| {
            let icon = if linked { IconName::Link } else { IconName::Unlink };
            icon_button("scale-link", icon, "Constrain Proportions", linked, cx.listener(|this, _, _, cx| {
                this.state.scale_linked = !this.state.scale_linked;
                cx.notify();
            }))
            .into_any_element()
        });
        rows = rows.child(property_row(id, PropertyPath::Transform(property), extra, workspace, cx));
        if property == TransformProperty::AnchorPoint {
            rows = rows.child(anchor_grid(layer, workspace, cx));
        }
    }
    rows
}

/// Nine buttons in the layout of the layer's box (corners, edge middles, center): pressing one
/// moves the anchor point there, keeping the layer where it is. The one the anchor is on is lit.
fn anchor_grid(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let frame = workspace.state.current_frame as f64;
    let anchor = layer.transform.as_ref().map(|transform| transform.anchor_point.value_at(frame));
    let size = layer.size;
    let at = |(fx, fy): (f32, f32)| anchor.is_some_and(|a| (a.x - size.x * fx).abs() < 0.5 && (a.y - size.y * fy).abs() < 0.5);
    let cell = |index: usize, fraction: (f32, f32), cx: &mut Context<Workspace>| {
        let lit = at(fraction);
        div()
            .id(("anchor-grid", index))
            .size(px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(3.0))
            .cursor_pointer()
            .hover(|style| style.bg(hsla(color::HOVER)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.state.set_anchor_fraction(id, fraction);
                cx.notify();
            }))
            .child(div().size(px(if lit { 7.0 } else { 5.0 })).rounded_full().bg(hsla(if lit { color::ACCENT } else { color::MUTED })))
            .test_support()
    };
    // Three rows of three, top to bottom, as the points sit on the layer's box.
    let mut grid = div().flex_none().flex().flex_col().gap(px(1.0)).p(px(2.0)).rounded(px(5.0)).border_1().border_color(hsla(color::HAIRLINE));
    for row in 0..3 {
        let mut line = div().flex().gap(px(1.0));
        for column in 0..3 {
            let index = row * 3 + column;
            line = line.child(cell(index, crate::state::ANCHOR_POINTS[index], cx));
        }
        grid = grid.child(line);
    }
    div()
        .flex()
        .items_center()
        .gap_3()
        .py_1()
        .pl(px(28.0))
        .child(grid)
        .child(div().flex_1().min_w_0().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Pick a corner, an edge or the center, or drag the anchor in the viewer to put it anywhere."))
}

/// A labeled row with an icon and a text-setting field, as in the character grid.
fn setting_field(icon: IconName, tooltip: &'static str, layer: Id, setting: TextSetting, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .h(px(26.0))
        .child(div().w(px(18.0)).flex().justify_center().child(Icon::new(icon).size(px(14.0)).text_color(hsla(color::MUTED))))
        .child(div().w(px(56.0)).text_color(hsla(color::MUTED)).child(tooltip))
        .child(value_field(FieldTarget::Text { layer, setting }, workspace, cx))
}

/// A text fill or stroke row: an on/off checkbox, its color (which opens the color wheel), and its label.
fn paint_row(
    id: &'static str,
    label: &'static str,
    paint: Option<Rgba>,
    target: ColorTarget,
    set: impl Fn(&mut Workspace, Option<Rgba>) + Clone + 'static,
    cx: &mut Context<Workspace>,
) -> Div {
    let workspace = cx.weak_entity();
    let last = paint.unwrap_or(Rgba::hex(0xffffff));
    let toggle = Checkbox::new(ElementId::Name(format!("{id}-on").into())).checked(paint.is_some()).on_click(move |checked: &bool, _, cx: &mut App| {
        let (set, checked) = (set.clone(), *checked);
        workspace
            .update(cx, |this, cx| {
                set(this, checked.then_some(last));
                cx.notify();
            })
            .ok();
    });
    div()
        .flex()
        .items_center()
        .gap_2()
        .h(px(30.0))
        .child(toggle)
        .child(color_picker::swatch(ElementId::Name(format!("{id}-color").into()), target, paint, cx))
        .child(div().child(label))
}

fn text_section(layer: &Layer, text: &str, style: &TextStyle, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let weak = cx.weak_entity();

    let mut families: Vec<&str> = workspace.ui.font_families.clone();
    if !families.contains(&style.font_family.as_str()) {
        families.insert(0, style.font_family.as_str());
    }
    let weight = style.font_weight;
    let family_choices = families.iter().map(|family| ((*family).to_owned(), *family == style.font_family, (*family).to_owned())).collect();
    let weight_choices = FONT_WEIGHTS.iter().map(|(value, name)| ((*name).to_owned(), *value == weight, *value)).collect();
    let current_family = style.font_family.clone();

    let source_text = if let Some(editing) = workspace.ui.editing.as_ref().filter(|e| e.target == EditTarget::SourceText(id) && !e.in_viewer) {
        div().flex_1().child(Input::new(&editing.input).small()).into_any_element()
    } else {
        let current = text.to_owned();
        div()
            .id("source-text")
            .flex_1()
            .px_1()
            .border_b_1()
            .border_color(hsla(color::HAIRLINE))
            .text_color(hsla(color::VALUE))
            .cursor_text()
            .hover(|style| style.border_color(hsla(color::VALUE)))
            .on_click(cx.listener(move |this, _, window, cx| this.begin_edit(EditTarget::SourceText(id), current.clone(), window, cx)))
            .child(if text.is_empty() { "Click to type".to_owned() } else { text.to_owned() })
            .test_support()
            .into_any_element()
    };

    let align_button = |key: &'static str, icon: IconName, tooltip: &'static str, align: TextAlign, cx: &mut Context<Workspace>| {
        icon_button(key, icon, tooltip, style.align == align, cx.listener(move |this, _, _, cx| {
            this.state.set_text_align(id, align);
            cx.notify();
        }))
    };
    let stroke_width = style.stroke.map_or(2.0, |(_, width)| width);

    div()
        .child(divider())
        .child(section("Text", None))
        .child(div().flex().items_center().gap_2().h(px(28.0)).child(div().w(px(56.0)).child("Source")).child(source_text))
        .child(div().h(px(30.0)).child(choice_menu("font-family", style.font_family.clone(), family_choices, move |this, family: String| this.state.set_font(id, &family, weight), weak.clone())))
        .child(div().h(px(30.0)).child(choice_menu("font-weight", weight_name(weight).to_owned(), weight_choices, move |this, weight: u16| this.state.set_font(id, &current_family, weight), weak)))
        .child(
            div()
                .grid()
                .grid_cols(2)
                .gap_x_3()
                .child(setting_field(IconName::ALargeSmall, "Size", id, TextSetting::FontSize, workspace, cx))
                .child(setting_field(IconName::ArrowUpDown, "Leading", id, TextSetting::Leading, workspace, cx))
                .child(setting_field(IconName::MoveHorizontal, "Tracking", id, TextSetting::Tracking, workspace, cx))
                .children(style.stroke.is_some().then(|| setting_field(IconName::PenLine, "Stroke", id, TextSetting::StrokeWidth, workspace, cx))),
        )
        .child(paint_row("fill", "Fill", style.fill, ColorTarget::TextFill(id), move |this, fill| this.state.set_text_fill(id, fill), cx))
        .child(paint_row("stroke", "Stroke", style.stroke.map(|(color, _)| color), ColorTarget::TextStroke(id), move |this, color| this.state.set_text_stroke(id, color.map(|color| (color, stroke_width))), cx))
        .child(divider())
        .child(section("Paragraph", None))
        .child(
            div()
                .flex()
                .gap_1()
                .child(align_button("align-text-left", IconName::TextAlignStart, "Left Align Text", TextAlign::Left, cx))
                .child(align_button("align-text-center", IconName::TextAlignCenter, "Center Text", TextAlign::Center, cx))
                .child(align_button("align-text-right", IconName::TextAlignEnd, "Right Align Text", TextAlign::Right, cx)),
        )
}

/// One of a shape's paints (fill or stroke): on or off, solid or gradient, its colors (each
/// opens the color wheel), and the gradient's angle or the stroke's width.
fn shape_paint_rows(id: Id, slot: PaintSlot, paint: Option<Paint>, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let weak = cx.weak_entity();
    let (label, key, angle, default) = match slot {
        PaintSlot::Fill => ("Fill", "fill", ShapeSetting::FillAngle, Paint::Solid(Rgba::hex(0x8b5cf6))),
        PaintSlot::Stroke => ("Stroke", "stroke", ShapeSetting::StrokeAngle, Paint::Solid(Rgba::hex(0xffffff))),
    };
    let toggle = {
        let weak = weak.clone();
        Checkbox::new(ElementId::Name(format!("shape-{key}-on").into())).checked(paint.is_some()).on_click(move |checked: &bool, _, cx: &mut App| {
            let checked = *checked;
            weak.update(cx, |this, cx| {
                this.state.set_shape_paint(id, slot, checked.then_some(default));
                cx.notify();
            })
            .ok();
        })
    };
    let kind = paint.map(Paint::kind);
    let kinds = PaintKind::ALL.iter().map(|&choice| (choice.name().to_owned(), Some(choice) == kind, choice)).collect();
    let menu = choice_menu(
        ElementId::Name(format!("shape-{key}-type").into()),
        kind.map_or("None", PaintKind::name).to_owned(),
        kinds,
        move |this, choice: PaintKind| {
            let current = this.state.shape_paint(id, slot).unwrap_or(default);
            this.state.set_shape_paint(id, slot, Some(current.into_kind(choice)));
        },
        weak,
    );
    let mut rows = div().child(labeled_row(label, div().flex().items_center().gap_2().child(toggle).child(div().flex_1().child(menu))));
    if let Some(paint) = paint {
        let colors = paint.colors();
        let last = colors.len() - 1;
        let mut controls = div().flex().items_center().gap_1p5();
        for (stop, color) in colors.into_iter().enumerate() {
            let target = ColorTarget::Shape { layer: id, slot, stop };
            controls = controls.child(color_picker::swatch(ElementId::Name(format!("shape-{key}-color:{stop}").into()), target, Some(color), cx));
            if stop < last {
                controls = controls.child(Icon::new(IconName::ArrowRight).size(px(12.0)).text_color(hsla(color::MUTED)));
            }
        }
        if paint.kind() == PaintKind::Linear {
            controls = controls.child(div().pl_2().text_color(hsla(color::MUTED)).child("Angle")).child(value_field(FieldTarget::Shape { layer: id, setting: angle }, workspace, cx));
        }
        if slot == PaintSlot::Stroke {
            controls = controls.child(div().pl_2().text_color(hsla(color::MUTED)).child("Width")).child(value_field(FieldTarget::Shape { layer: id, setting: ShapeSetting::StrokeWidth }, workspace, cx));
        }
        rows = rows.child(labeled_row("", controls));
    }
    rows
}

/// A shape layer's settings, laid out like After Effects': which shape, its size (or, for
/// polygons and stars, outer and inner radius), corners, points, fill and stroke.
fn shape_section(layer: &Layer, style: &ShapeStyle, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let weak = cx.weak_entity();
    let kinds = ShapeKind::ALL.iter().map(|&kind| (kind.name().to_owned(), kind == style.kind, kind)).collect();
    let field = |setting: ShapeSetting, cx: &mut Context<Workspace>| value_field(FieldTarget::Shape { layer: id, setting }, workspace, cx);
    let points_label = if style.kind == ShapeKind::Star { "Points" } else { "Sides" };
    let linked = workspace.state.shape_size_linked;
    let link = icon_button("shape-size-link", if linked { IconName::Link } else { IconName::Unlink }, "Keep proportions", linked, cx.listener(|this, _, _, cx| {
        this.state.shape_size_linked = !this.state.shape_size_linked;
        cx.notify();
    }));
    let size_row = labeled_row(
        "Size",
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(muted_label("W"))
            .child(field(ShapeSetting::Width, cx))
            .child(link)
            .child(muted_label("H"))
            .child(field(ShapeSetting::Height, cx)),
    );
    // Polygons and stars are sized by radius, as in After Effects.
    let radial = style.kind.has_points();
    div()
        .child(divider())
        .child(section("Shape", None))
        .child(labeled_row("Shape", choice_menu("shape-kind", style.kind.name().to_owned(), kinds, move |this, kind| this.state.set_shape_kind(id, kind), weak)))
        .when(!radial, |section| section.child(size_row))
        .when(radial, |section| section.child(labeled_row("Outer Radius", field(ShapeSetting::OuterRadius, cx))))
        .when(style.kind == ShapeKind::Star, |section| section.child(labeled_row("Inner Radius", field(ShapeSetting::InnerRadius, cx))))
        .when(radial, |section| section.child(labeled_row(points_label, field(ShapeSetting::Points, cx))))
        .when(style.kind.has_corners(), |section| section.child(labeled_row("Corner Radius", field(ShapeSetting::CornerRadius, cx))))
        .child(shape_paint_rows(id, PaintSlot::Fill, style.fill, workspace, cx))
        .child(shape_paint_rows(id, PaintSlot::Stroke, style.stroke.map(|stroke| stroke.paint), workspace, cx))
}

/// A small muted label beside a field, such as "W" or "H".
fn muted_label(text: &'static str) -> Div {
    div().text_color(hsla(color::MUTED)).child(text)
}

fn properties_body(workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let composition = workspace.state.composition();
    let selected: Vec<&Layer> = workspace.state.selected_layers.iter().filter_map(|id| composition.layer(*id)).collect();
    let [layer] = selected.as_slice() else {
        let message = if selected.is_empty() { "Select a layer to see its properties." } else { "Select a single layer to edit its properties." };
        return hint(message).into_any_element();
    };
    let transform = if layer.transform.is_some() {
        transform_section(layer, workspace, cx)
    } else {
        div().child(hint("Audio layers have no transform."))
    };
    let text = match &layer.content {
        LayerContent::Text { text, style } => Some(text_section(layer, text, style, workspace, cx)),
        LayerContent::Shape { style } => Some(shape_section(layer, style, workspace, cx)),
        LayerContent::Captions { words, style } => Some(captions_section(layer, style, words, workspace, cx)),
        _ => None,
    };
    // Tell the user when an edit will change several keyframes at once.
    let selected_keys = workspace.state.selected_keyframes.iter().filter(|key| key.layer == layer.id).count();
    let mass_edit_note = (selected_keys > 1).then(|| {
        div()
            .mt_2()
            .px_2()
            .py_1()
            .rounded(px(3.0))
            .bg(hsla(color::INSET))
            .text_color(hsla(color::MUTED))
            .child(format!("{selected_keys} keyframes selected. With the playhead on one of them, a change applies to every selected keyframe of that property."))
    });
    div()
        .id("properties-body")
        .size_full()
        .overflow_y_scroll()
        .px_2p5()
        .pb_3()
        .children(mass_edit_note)
        .child(layer_section(layer, workspace, cx))
        .child(divider())
        .child(transform)
        .children(text)
        .children(workspace.state.has_sound(layer.id).then(|| audio_section(layer, workspace, cx)))
        .into_any_element()
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let composition = workspace.state.composition();
    let selected: Vec<&Layer> = workspace.state.selected_layers.iter().filter_map(|id| composition.layer(*id)).collect();
    let subject = match selected.as_slice() {
        [] => "No Selection".to_owned(),
        [layer] => layer.name.clone(),
        layers => format!("{} Layers", layers.len()),
    };
    let properties_title = format!("Properties: {subject}");
    let body = if workspace.ui.tab(PanelId::Properties) == 0 {
        properties_body(workspace, cx)
    } else {
        super::effect_controls::body(workspace, cx)
    };
    panel(PanelId::Properties, &[properties_title.as_str(), "Effect Controls"], None, body, workspace, cx)
}
