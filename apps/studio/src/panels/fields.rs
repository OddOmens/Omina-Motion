//! Editable value fields, the blue underlined numbers used across Properties,
//! Effect Controls and the timeline. Drag one sideways to scrub it (Shift for
//! ten times faster, Alt for ten times finer), or click it to type a value.

use gpui_kit::component::input::Input;
use gpui_kit::component::Sizable as _;
use gpui_kit::{
    AnyElement, AppContext as _, ClickEvent, Context, ElementId, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _, div, px,
};
use studio_core::model::Id;
use studio_core::property::{PropertyPath, TransformProperty};

use super::chrome::EmptyDragView;
use crate::state::{AudioSetting, EditorState, ShapeSetting, TextSetting};
use crate::theme::{color, hsla};
use crate::workspace::{EditTarget, Workspace};

/// A number that a value field edits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FieldTarget {
    /// One component (0 = x or the only value, 1 = y) of a layer property.
    Property { layer: Id, path: PropertyPath, component: usize },
    Text { layer: Id, setting: TextSetting },
    Shape { layer: Id, setting: ShapeSetting },
    Audio { layer: Id, setting: AudioSetting },
    /// How far along its path a text layer set on one starts.
    TextPathStart { layer: Id },
    /// One side (0 width, 1 height) of a video or image layer's Size, in pixels.
    MediaSize { layer: Id, side: usize },
    /// Where a gradient stop sits along a shape's gradient, as a percentage.
    GradientStop { layer: Id, slot: crate::state::PaintSlot, stop: usize },
    /// How long a text layer's In or Out text animation takes, in seconds.
    TextAnimationSeconds { layer: Id, phase: studio_core::presets::Phase },
    /// How much the pieces of a text animation overlap, as a percentage.
    TextAnimationOverlap { layer: Id, phase: studio_core::presets::Phase },
    /// A caption layer's sync: how many frames later (negative, earlier) its captions show.
    CaptionSync { layer: Id },
    /// When the caption phrase made of words `first..end` starts (`start`) or ends, in
    /// seconds of the composition.
    CaptionPhrase { layer: Id, first: usize, end: usize, start: bool },
    /// One of an object matte's edge settings.
    MatteEdge { layer: Id, setting: MatteEdgeSetting },
}

/// An object matte edge setting a field edits (see `model::MatteEdges`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatteEdgeSetting {
    Feather,
    Contrast,
    ShiftEdge,
    ReduceChatter,
    RefineDetail,
}

impl MatteEdgeSetting {
    pub const ALL: [Self; 5] = [Self::Feather, Self::Contrast, Self::ShiftEdge, Self::ReduceChatter, Self::RefineDetail];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Feather => "Feather",
            Self::Contrast => "Contrast",
            Self::ShiftEdge => "Shift Edge",
            Self::ReduceChatter => "Reduce Chatter",
            Self::RefineDetail => "Refine Detail",
        }
    }

    /// The setting's value in `edges`, or a place to write it.
    fn slot(self, edges: &mut studio_core::model::MatteEdges) -> &mut f32 {
        match self {
            Self::Feather => &mut edges.feather,
            Self::Contrast => &mut edges.contrast,
            Self::ShiftEdge => &mut edges.shift_edge,
            Self::ReduceChatter => &mut edges.reduce_chatter,
            Self::RefineDetail => &mut edges.refine_detail,
        }
    }
}

/// How a field's number is shown and how fast it scrubs.
#[derive(Clone, Copy, Debug)]
pub struct FieldFormat {
    pub step: f32,
    pub decimals: usize,
    pub unit: &'static str,
    pub rotation: bool,
}

/// Drag payload while scrubbing a field.
#[derive(Clone, Copy, Debug)]
pub struct FieldDrag(pub FieldTarget);

impl FieldTarget {
    /// The field's current value and format, or `None` if its layer or property is gone.
    pub fn read(self, state: &EditorState) -> Option<(f32, FieldFormat)> {
        match self {
            Self::Property { layer, path, component } => {
                let snapshot = state.composition().layer(layer)?.property(path, state.current_frame)?;
                let format = FieldFormat {
                    step: snapshot.info.step,
                    decimals: snapshot.info.decimals,
                    unit: snapshot.info.unit,
                    rotation: path == PropertyPath::Transform(TransformProperty::Rotation),
                };
                Some((snapshot.value.component(component), format))
            }
            Self::Text { layer, setting } => {
                let style = state.text_style_of(layer)?;
                let (step, decimals, unit) = match setting {
                    TextSetting::FontSize | TextSetting::Leading => (0.5, 0, "px"),
                    TextSetting::Tracking => (1.0, 0, ""),
                    TextSetting::StrokeWidth | TextSetting::StrokeDash | TextSetting::StrokeGap => (0.1, 1, "px"),
                    TextSetting::StrokeMiter => (0.05, 1, ""),
                };
                Some((setting.get(style), FieldFormat { step, decimals, unit, rotation: false }))
            }
            Self::Shape { layer, setting } => {
                let style = state.shape_style_now(layer)?;
                let shape = state.composition().layer(layer)?;
                // An animated outline's width and height are read at the playhead; the other
                // settings are measured against the layer's own box.
                let size = if matches!(setting, ShapeSetting::Width | ShapeSetting::Height) { shape.shape_size_at(state.current_frame as f64) } else { shape.size };
                let (step, decimals, unit) = match setting {
                    ShapeSetting::Width | ShapeSetting::Height | ShapeSetting::OuterRadius | ShapeSetting::InnerRadius => (1.0, 0, "px"),
                    ShapeSetting::CornerRadius => (0.5, 0, "px"),
                    ShapeSetting::StrokeWidth => (0.1, 1, "px"),
                    ShapeSetting::Points => (0.1, 0, ""),
                    ShapeSetting::FillAngle | ShapeSetting::StrokeAngle => (1.0, 0, "°"),
                    ShapeSetting::TrimStart | ShapeSetting::TrimEnd | ShapeSetting::TrimOffset => (0.5, 0, "%"),
                };
                Some((setting.get(&style, size), FieldFormat { step, decimals, unit, rotation: false }))
            }
            Self::Audio { layer, setting } => {
                let value = state.audio_setting(layer, setting)?;
                let (step, decimals, unit) = match setting {
                    AudioSetting::Volume => (0.1, 1, " dB"),
                    AudioSetting::FadeIn | AudioSetting::FadeOut => (0.01, 2, " s"),
                    AudioSetting::DuckAmount => (0.1, 0, " dB"),
                    AudioSetting::NoiseGate => (0.5, 0, "%"),
                    AudioSetting::Pan => (1.0, 0, ""),
                };
                Some((value, FieldFormat { step, decimals, unit, rotation: false }))
            }
            Self::TextPathStart { layer } => {
                let start = state.composition().layer(layer)?.text_path?.start;
                Some((start, FieldFormat { step: 1.0, decimals: 0, unit: "px", rotation: false }))
            }
            Self::MediaSize { layer, side } => {
                let size = state.media_size(layer)?;
                Some((if side == 0 { size.x } else { size.y }, FieldFormat { step: 1.0, decimals: 0, unit: "px", rotation: false }))
            }
            Self::GradientStop { layer, slot, stop } => {
                let at = state.shape_gradient(layer, slot)?.stops().get(stop)?.at;
                Some((at * 100.0, FieldFormat { step: 0.5, decimals: 1, unit: "%", rotation: false }))
            }
            Self::TextAnimationSeconds { layer, phase } => {
                let seconds = state.text_animator(layer, phase)?.seconds;
                Some((seconds, FieldFormat { step: 0.01, decimals: 2, unit: " s", rotation: false }))
            }
            Self::TextAnimationOverlap { layer, phase } => {
                let overlap = state.text_animator(layer, phase)?.overlap;
                Some((overlap * 100.0, FieldFormat { step: 0.5, decimals: 0, unit: "%", rotation: false }))
            }
            Self::CaptionSync { layer } => {
                let offset = state.caption_style(layer)?.offset;
                Some((offset as f32, FieldFormat { step: 0.1, decimals: 0, unit: " fr", rotation: false }))
            }
            Self::CaptionPhrase { layer, first, end, start } => {
                let frame = state.caption_phrase_frame(layer, first..end, start)?;
                let fps = state.composition().frame_rate.fps();
                Some(((frame as f64 / fps) as f32, FieldFormat { step: 0.005, decimals: 2, unit: " s", rotation: false }))
            }
            Self::MatteEdge { layer, setting } => {
                let mut edges = state.composition().layer(layer)?.cutout.as_ref()?.edges;
                let unit = if setting == MatteEdgeSetting::Feather { "px" } else { "%" };
                Some((*setting.slot(&mut edges), FieldFormat { step: 0.5, decimals: 0, unit, rotation: false }))
            }
        }
    }

    pub fn write(self, state: &mut EditorState, value: f32) {
        match self {
            Self::Property { layer, path, component } => state.set_property_component(layer, path, component, value),
            Self::Text { layer, setting } => state.set_text_setting(layer, setting, value),
            Self::Shape { layer, setting } => state.set_shape_setting(layer, setting, value),
            Self::Audio { layer, setting } => state.set_audio_setting(layer, setting, value),
            Self::TextPathStart { layer } => state.set_text_path_start(layer, value),
            Self::MediaSize { layer, side } => state.set_media_size_side(layer, side, value),
            Self::GradientStop { layer, slot, stop } => state.set_gradient_stop_location(layer, slot, stop, value / 100.0),
            Self::TextAnimationSeconds { layer, phase } => state.edit_text_animator(layer, phase, |animator| animator.seconds = value),
            Self::TextAnimationOverlap { layer, phase } => state.edit_text_animator(layer, phase, |animator| animator.overlap = value / 100.0),
            Self::CaptionSync { layer } => state.edit_captions(layer, |style| style.offset = value.round() as i64),
            Self::CaptionPhrase { layer, first, end, start } => {
                let frame = (f64::from(value) * state.composition().frame_rate.fps()).round() as i64;
                state.retime_caption_phrase(layer, first..end, start, frame);
            }
            Self::MatteEdge { layer, setting } => {
                if let Some(mut edges) = state.composition().layer(layer).and_then(|layer| layer.cutout.as_ref()).map(|cut| cut.edges) {
                    *setting.slot(&mut edges) = value;
                    state.set_matte_edges(layer, edges);
                }
            }
        }
    }

    fn element_id(self) -> ElementId {
        ElementId::Name(format!("field:{self:?}").into())
    }
}

/// Text shown in a field: rotation as revolutions plus degrees ("1x+30.0°"), others with their unit.
pub fn format_value(value: f32, format: FieldFormat) -> String {
    if format.rotation {
        let revolutions = (value / 360.0).trunc();
        let degrees = value - revolutions * 360.0;
        return format!("{}x{:+.*}°", revolutions as i64, format.decimals, degrees);
    }
    format!("{:.*}{}", format.decimals, value, format.unit)
}

/// Reads a typed value, ignoring units and spaces ("120 px", "45°", "50%"). Rotations may be
/// typed the way they are shown, as revolutions and degrees ("1x+30°" is 390°). Returns `None`
/// for non-numbers.
pub fn parse_value(text: &str) -> Option<f32> {
    let number = |part: &str| -> Option<f32> {
        let cleaned: String = part.chars().filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+')).collect();
        cleaned.parse::<f32>().ok().filter(|value| value.is_finite())
    };
    if let Some((revolutions, degrees)) = text.split_once(['x', 'X'])
        && let (Some(revolutions), Some(degrees)) = (number(revolutions), number(degrees))
    {
        return Some(revolutions * 360.0 + degrees).filter(|value| value.is_finite());
    }
    number(text)
}

impl Workspace {
    /// Scrubs the field being dragged to `pointer_x`, relative to where the drag began.
    pub fn scrub_field(&mut self, target: FieldTarget, pointer_x: f32, coarse: bool, fine: bool) {
        let Some((origin_x, start)) = self.ui.field_anchor else { return };
        let Some((_, format)) = target.read(&self.state) else { return };
        let speed = if coarse { 10.0 } else if fine { 0.1 } else { 1.0 };
        target.write(&mut self.state, start + (pointer_x - origin_x) * format.step * speed);
    }
}

/// A value field for `target`. While it is being typed into, it shows a text input instead.
pub fn value_field(target: FieldTarget, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let Some((value, format)) = target.read(&workspace.state) else { return div().into_any_element() };

    if let Some(input) = workspace.ui.editing.as_ref().filter(|editing| editing.target == EditTarget::Number(target)).and_then(|editing| editing.input.line()) {
        return div().w(px(76.0)).child(Input::new(input).small()).into_any_element();
    }

    div()
        .id(target.element_id())
        .min_w(px(52.0))
        .px_0p5()
        .border_b_1()
        .border_color(hsla(color::HAIRLINE))
        // Fixed-width digits, so the number doesn't shift sideways while it is dragged.
        .font_family(crate::theme::MONO_FONT)
        .text_color(hsla(color::VALUE))
        .cursor_ew_resize()
        .hover(|style| style.border_color(hsla(color::VALUE)))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, _| {
            if let Some((start, _)) = target.read(&this.state) {
                this.ui.field_anchor = Some((f32::from(event.position.x), start));
            }
        }))
        .on_drag(FieldDrag(target), |_, _, _, cx| cx.new(|_| EmptyDragView))
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            let text = target.read(&this.state).map(|(value, format)| format!("{:.*}", format.decimals, value)).unwrap_or_default();
            this.begin_edit(EditTarget::Number(target), text, window, cx);
        }))
        .child(format_value(value, format))
        .test_support()
        .into_any_element()
}

/// Which small control a property row shows: each has its own icon, drawn at the size that
/// makes the three look the same weight side by side.
#[derive(Clone, Copy, PartialEq)]
pub enum RowControl {
    /// Starts or stops animating the property.
    Stopwatch,
    /// Adds or removes a keyframe at the playhead.
    Keyframe,
    /// Moves the playhead to the previous keyframe.
    PreviousKeyframe,
    /// Moves the playhead to the next keyframe.
    NextKeyframe,
}

/// One of a property row's small square controls, lit in the accent color when `lit`. The
/// caller adds the click handler, so every animatable row (transform properties, a shape's
/// path) is drawn from the same pieces.
pub fn row_control(id: impl Into<ElementId>, control: RowControl, lit: bool) -> gpui_kit::Stateful<gpui_kit::Div> {
    use gpui_kit::assets::IconName;
    use gpui_kit::component::Icon;

    let (icon, icon_size) = match control {
        RowControl::Stopwatch => (IconName::Timer, 14.0),
        RowControl::Keyframe => (IconName::Diamond, 11.0),
        RowControl::PreviousKeyframe => (IconName::ChevronLeft, 12.0),
        RowControl::NextKeyframe => (IconName::ChevronRight, 12.0),
    };
    div()
        .id(id)
        .size(px(ROW_CONTROL_SIZE))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.0))
        .hover(|style| style.bg(hsla(color::HOVER)))
        .child(Icon::new(icon).size(px(icon_size)).text_color(hsla(if lit { color::ACCENT } else { color::MUTED })))
}

/// The side of a property row's square controls, and the width of the columns that hold one.
const ROW_CONTROL_SIZE: f32 = 18.0;

/// How far from a card's left edge the names start: the stopwatch column and the gap after it.
/// Anything in a card that isn't a row (a checkbox, a button, a note) is indented this far, so
/// it lines up under the names and only stopwatches sit in the left margin.
pub const NAME_INDENT: f32 = ROW_CONTROL_SIZE + 8.0;

/// The columns every Properties row shares, so names and values line up from row to row and
/// card to card: a stopwatch (empty for a setting that isn't animated), the name, one small
/// extra control such as a link or a checkbox, then the values, which take the rest of the row.
pub fn grid_row(stopwatch: Option<AnyElement>, name: impl Into<gpui_kit::SharedString>, extra: Option<AnyElement>, values: impl IntoElement) -> gpui_kit::Div {
    // A minimum, not a fixed height: a row grows for a taller value such as a text box.
    div()
        .min_h(px(28.0))
        .flex()
        .items_center()
        .gap_2()
        .child(div().w(px(ROW_CONTROL_SIZE)).flex_none().children(stopwatch))
        .child(div().w(px(104.0)).flex_none().truncate().child(name.into()))
        .child(div().w(px(ROW_CONTROL_SIZE)).flex_none().flex().justify_center().children(extra))
        .child(div().flex_1().min_w_0().child(values))
}

/// A property row: stopwatch, name, one field per component, and (when animated) buttons that
/// step between keyframes and add or remove one at the playhead. `extra` sits between the name
/// and the fields.
pub fn property_row(
    layer: Id,
    path: PropertyPath,
    extra: Option<AnyElement>,
    workspace: &Workspace,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let frame = workspace.state.current_frame;
    let Some(snapshot) = workspace.state.composition().layer(layer).and_then(|l| l.property(path, frame)) else {
        return div().into_any_element();
    };
    let animated = !snapshot.keyframes.is_empty();
    let on_keyframe = snapshot.keyframes.contains(&frame);
    let key = format!("{path:?}:{}", layer.0);
    let control_id = |name: &str| ElementId::Name(format!("{name}:{key}").into());

    let stopwatch = row_control(control_id("stopwatch"), RowControl::Stopwatch, animated).on_click(cx.listener(move |this, _, _, cx| {
        this.state.toggle_stopwatch(layer, path);
        cx.notify();
    }))
    .test_support();

    let fields = (0..snapshot.value.components())
        .map(|component| value_field(FieldTarget::Property { layer, path, component }, workspace, cx))
        .collect::<Vec<_>>();

    // Previous / next keyframe of this property, as in After Effects' keyframe navigator.
    let jump = |forward: bool, cx: &mut Context<Workspace>| {
        let (name, control) = if forward { ("next-keyframe", RowControl::NextKeyframe) } else { ("previous-keyframe", RowControl::PreviousKeyframe) };
        row_control(control_id(name), control, false).on_click(cx.listener(move |this, _, _, cx| {
            this.state.jump_to_keyframe(layer, path, forward);
            cx.notify();
        }))
        .test_support()
    };
    let previous = animated.then(|| jump(false, cx));
    let next = animated.then(|| jump(true, cx));
    let keyframe_button = animated.then(|| {
        row_control(control_id("keyframe"), RowControl::Keyframe, on_keyframe).on_click(cx.listener(move |this, _, _, cx| {
            this.state.toggle_keyframe_at_playhead(layer, path);
            cx.notify();
        }))
    });

    grid_row(Some(stopwatch.into_any_element()), snapshot.name.clone(), extra, div().flex().gap_3().children(fields))
        .children(previous)
        .children(keyframe_button)
        .children(next)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(decimals: usize, unit: &'static str) -> FieldFormat {
        FieldFormat { step: 1.0, decimals, unit, rotation: false }
    }

    #[test]
    fn values_format_with_units_and_revolutions() {
        assert_eq!(format_value(100.0, plain(0, "%")), "100%");
        assert_eq!(format_value(12.345, plain(1, "")), "12.3");
        let rotation = FieldFormat { step: 0.5, decimals: 1, unit: "°", rotation: true };
        assert_eq!(format_value(45.0, rotation), "0x+45.0°");
        assert_eq!(format_value(390.0, rotation), "1x+30.0°");
        assert_eq!(format_value(-30.0, rotation), "0x-30.0°");
    }

    #[test]
    fn typed_values_ignore_units() {
        assert_eq!(parse_value("120 px"), Some(120.0));
        assert_eq!(parse_value("-45°"), Some(-45.0));
        assert_eq!(parse_value("50%"), Some(50.0));
        assert_eq!(parse_value("abc"), None);
        // Rotation, typed as it is shown.
        assert_eq!(parse_value("1x+30.0°"), Some(390.0));
        assert_eq!(parse_value("-1x-30°"), Some(-390.0));
        assert_eq!(parse_value("0x+45"), Some(45.0));
        let rotation = FieldFormat { step: 0.5, decimals: 1, unit: "°", rotation: true };
        assert_eq!(parse_value(&format_value(-390.0, rotation)), Some(-390.0), "what is shown reads back");
        assert_eq!(parse_value(""), None);
    }
}
