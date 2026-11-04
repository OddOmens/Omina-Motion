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
    /// A footage layer's constant speed, as a percentage.
    Speed { layer: Id },
    /// One side (0 width, 1 height) of a video or image layer's Size, in pixels.
    MediaSize { layer: Id, side: usize },
    /// Where a gradient stop sits along a shape's gradient, as a percentage.
    GradientStop { layer: Id, slot: crate::state::PaintSlot, stop: usize },
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
                    TextSetting::StrokeWidth => (0.1, 1, "px"),
                };
                Some((setting.get(style), FieldFormat { step, decimals, unit, rotation: false }))
            }
            Self::Shape { layer, setting } => {
                let style = state.shape_of(layer)?;
                let size = state.composition().layer(layer)?.size;
                let (step, decimals, unit) = match setting {
                    ShapeSetting::Width | ShapeSetting::Height | ShapeSetting::OuterRadius | ShapeSetting::InnerRadius => (1.0, 0, "px"),
                    ShapeSetting::CornerRadius => (0.5, 0, "px"),
                    ShapeSetting::StrokeWidth => (0.1, 1, "px"),
                    ShapeSetting::Points => (0.1, 0, ""),
                    ShapeSetting::FillAngle | ShapeSetting::StrokeAngle => (1.0, 0, "°"),
                };
                Some((setting.get(style, size), FieldFormat { step, decimals, unit, rotation: false }))
            }
            Self::Audio { layer, setting } => {
                let value = state.audio_setting(layer, setting)?;
                let (step, decimals, unit) = match setting {
                    AudioSetting::Volume => (0.1, 1, " dB"),
                    AudioSetting::FadeIn | AudioSetting::FadeOut => (0.01, 2, " s"),
                    AudioSetting::DuckAmount => (0.1, 0, " dB"),
                    AudioSetting::NoiseGate => (0.5, 0, "%"),
                };
                Some((value, FieldFormat { step, decimals, unit, rotation: false }))
            }
            Self::Speed { layer } => {
                let speed = state.speed_of(layer)?;
                Some((speed, FieldFormat { step: 0.5, decimals: 0, unit: "%", rotation: false }))
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
        }
    }

    pub fn write(self, state: &mut EditorState, value: f32) {
        match self {
            Self::Property { layer, path, component } => state.set_property_component(layer, path, component, value),
            Self::Text { layer, setting } => state.set_text_setting(layer, setting, value),
            Self::Shape { layer, setting } => state.set_shape_setting(layer, setting, value),
            Self::Audio { layer, setting } => state.set_audio_setting(layer, setting, value),
            Self::TextPathStart { layer } => state.set_text_path_start(layer, value),
            Self::Speed { layer } => state.set_speed(layer, value),
            Self::MediaSize { layer, side } => state.set_media_size_side(layer, side, value),
            Self::GradientStop { layer, slot, stop } => state.set_gradient_stop_location(layer, slot, stop, value / 100.0),
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

    if let Some(editing) = workspace.ui.editing.as_ref().filter(|editing| editing.target == EditTarget::Number(target)) {
        return div().w(px(76.0)).child(Input::new(&editing.input).xsmall()).into_any_element();
    }

    div()
        .id(target.element_id())
        .min_w(px(52.0))
        .px_0p5()
        .border_b_1()
        .border_color(hsla(color::HAIRLINE))
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

/// A property row: stopwatch, name, one field per component, and (when animated) a button
/// that adds or removes a keyframe at the playhead. `extra` sits between the name and the fields.
pub fn property_row(
    layer: Id,
    path: PropertyPath,
    extra: Option<AnyElement>,
    workspace: &Workspace,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    use gpui_kit::assets::IconName;
    use gpui_kit::component::Icon;

    let frame = workspace.state.current_frame;
    let Some(snapshot) = workspace.state.composition().layer(layer).and_then(|l| l.property(path, frame)) else {
        return div().into_any_element();
    };
    let animated = !snapshot.keyframes.is_empty();
    let on_keyframe = snapshot.keyframes.contains(&frame);
    let key = format!("{path:?}:{}", layer.0);

    let stopwatch = div()
        .id(ElementId::Name(format!("stopwatch:{key}").into()))
        .size(px(18.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.0))
        .hover(|style| style.bg(hsla(color::HOVER)))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.state.toggle_stopwatch(layer, path);
            cx.notify();
        }))
        .child(Icon::new(IconName::Timer).size(px(14.0)).text_color(hsla(if animated { color::ACCENT } else { color::MUTED })))
        .test_support();

    let fields = (0..snapshot.value.components())
        .map(|component| value_field(FieldTarget::Property { layer, path, component }, workspace, cx))
        .collect::<Vec<_>>();

    // Previous / next keyframe of this property, as in After Effects' keyframe navigator.
    let jump = |forward: bool, cx: &mut Context<Workspace>| {
        div()
            .id(ElementId::Name(format!("{}:{key}", if forward { "next-keyframe" } else { "previous-keyframe" }).into()))
            .size(px(18.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(3.0))
            .hover(|style| style.bg(hsla(color::HOVER)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.state.jump_to_keyframe(layer, path, forward);
                cx.notify();
            }))
            .child(Icon::new(if forward { IconName::ChevronRight } else { IconName::ChevronLeft }).size(px(12.0)).text_color(hsla(color::MUTED)))
            .test_support()
    };
    let previous = animated.then(|| jump(false, cx));
    let next = animated.then(|| jump(true, cx));
    let keyframe_button = animated.then(|| {
        div()
            .id(ElementId::Name(format!("keyframe:{key}").into()))
            .size(px(18.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(3.0))
            .hover(|style| style.bg(hsla(color::HOVER)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.state.toggle_keyframe_at_playhead(layer, path);
                cx.notify();
            }))
            .child(Icon::new(IconName::Diamond).size(px(11.0)).text_color(hsla(if on_keyframe { color::ACCENT } else { color::MUTED })))
    });

    div()
        .h(px(26.0))
        .flex()
        .items_center()
        .gap_2()
        .child(stopwatch)
        .child(div().w(px(104.0)).flex_none().truncate().child(snapshot.name.clone()))
        .child(div().w(px(18.0)).flex_none().children(extra))
        .child(div().flex().gap_3().children(fields))
        .child(div().flex_1())
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
