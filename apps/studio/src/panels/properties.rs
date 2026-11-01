//! The Properties panel (first tab) and Effect Controls (second tab). Properties
//! shows the selected layer's transform and, for text layers, character and
//! paragraph styling. Every value scrubs or can be typed into.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, TestSupportExt as _, WeakEntity, div, prelude::FluentBuilder as _, px, rgba,
};
use studio_core::model::{BlendMode, Id, Layer, LayerContent, Rgba, ShapeKind, ShapeStyle, TextAlign, TextStyle, TrackMatte};
use studio_core::property::{PropertyPath, TransformProperty};

use super::chrome::{divider, hint, icon_button, panel, section};
use super::fields::{FieldTarget, property_row, value_field};
use crate::state::{ShapeSetting, Switch, TextSetting};
use crate::theme::{color, hsla};
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

/// Swatches offered for text fill and stroke.
const SWATCHES: &[(&str, u32)] = &[
    ("White", 0xffffff),
    ("Black", 0x000000),
    ("Gray", 0x808080),
    ("Red", 0xe5484d),
    ("Orange", 0xf76b15),
    ("Yellow", 0xffe629),
    ("Green", 0x30a46c),
    ("Blue", 0x2d8ceb),
    ("Purple", 0x8e4ec6),
    ("Pink", 0xe93d82),
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
    }
    rows
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

/// A fill or stroke row: on/off checkbox, color swatch with a palette menu, and its label.
fn paint_row(
    id: &'static str,
    label: &'static str,
    paint: Option<Rgba>,
    set: impl Fn(&mut Workspace, Option<Rgba>) + Clone + 'static,
    cx: &mut Context<Workspace>,
) -> Div {
    let workspace = cx.weak_entity();
    let enabled = paint.is_some();
    let last = paint.unwrap_or(Rgba::hex(0xffffff));
    let toggle = {
        let set = set.clone();
        let workspace = workspace.clone();
        Checkbox::new(ElementId::Name(format!("{id}-on").into())).checked(enabled).on_click(move |checked: &bool, _, cx: &mut App| {
            let (set, checked) = (set.clone(), *checked);
            workspace
                .update(cx, |this, cx| {
                    set(this, checked.then_some(last));
                    cx.notify();
                })
                .ok();
        })
    };
    let swatches = SWATCHES
        .iter()
        .map(|(name, hex)| ((*name).to_owned(), paint.is_some_and(|p| p.to_rgba_u32() == Rgba::hex(*hex).to_rgba_u32()), Rgba::hex(*hex)))
        .collect();
    let set_color = move |this: &mut Workspace, color: Rgba| set(this, Some(color));

    div()
        .flex()
        .items_center()
        .gap_2()
        .h(px(28.0))
        .child(toggle)
        .child(
            div()
                .size(px(18.0))
                .rounded(px(2.0))
                .border_1()
                .border_color(hsla(color::HAIRLINE))
                .bg(rgba(paint.map_or(0x00000000, |p| p.to_rgba_u32()))),
        )
        .child(div().w(px(44.0)).child(label))
        .child(div().w(px(110.0)).child(choice_menu(ElementId::Name(format!("{id}-color").into()), "Color".into(), swatches, set_color, workspace)))
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

    let source_text = if let Some(editing) = workspace.ui.editing.as_ref().filter(|e| e.target == EditTarget::SourceText(id)) {
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
        .child(paint_row("fill", "Fill", style.fill, move |this, fill| this.state.set_text_fill(id, fill), cx))
        .child(paint_row("stroke", "Stroke", style.stroke.map(|(color, _)| color), move |this, color| this.state.set_text_stroke(id, color.map(|color| (color, stroke_width))), cx))
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

/// A shape layer's settings: which shape, its fill, and (where it has them) rounded corners
/// and the number of sides or points.
fn shape_section(layer: &Layer, style: &ShapeStyle, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let weak = cx.weak_entity();
    let kinds = ShapeKind::ALL.iter().map(|&kind| (kind.name().to_owned(), kind == style.kind, kind)).collect();
    let fill = style.fill;
    let swatches = SWATCHES.iter().map(|(name, hex)| ((*name).to_owned(), fill.to_rgba_u32() == Rgba::hex(*hex).to_rgba_u32(), Rgba::hex(*hex))).collect();
    let swatch = div().size(px(18.0)).flex_none().rounded(px(4.0)).border_1().border_color(hsla(color::HAIRLINE)).bg(rgba(fill.to_rgba_u32()));
    let field = |setting: ShapeSetting, cx: &mut Context<Workspace>| value_field(FieldTarget::Shape { layer: id, setting }, workspace, cx);
    let points_label = if style.kind == ShapeKind::Star { "Points" } else { "Sides" };
    div()
        .child(divider())
        .child(section("Shape", None))
        .child(labeled_row("Shape", choice_menu("shape-kind", style.kind.name().to_owned(), kinds, move |this, kind| this.state.set_shape_kind(id, kind), weak.clone())))
        .child(labeled_row("Fill", div().flex().items_center().gap_2().child(swatch).child(div().flex_1().child(choice_menu("shape-fill", "Color".into(), swatches, move |this, color| this.state.set_shape_fill(id, color), weak)))))
        .when(style.kind.has_corners(), |section| section.child(labeled_row("Corner Radius", field(ShapeSetting::CornerRadius, cx))))
        .when(style.kind.has_points(), |section| section.child(labeled_row(points_label, field(ShapeSetting::Points, cx))))
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
