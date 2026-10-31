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
use crate::state::{EditorState, TextSetting};
use crate::theme::{color, hsla};
use crate::workspace::{EditTarget, Workspace};

/// A number that a value field edits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FieldTarget {
    /// One component (0 = x or the only value, 1 = y) of a layer property.
    Property { layer: Id, path: PropertyPath, component: usize },
    Text { layer: Id, setting: TextSetting },
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
                let (_, style) = state.text_of(layer)?;
                let (step, decimals, unit) = match setting {
                    TextSetting::FontSize | TextSetting::Leading => (0.5, 0, "px"),
                    TextSetting::Tracking => (1.0, 0, ""),
                    TextSetting::StrokeWidth => (0.1, 1, "px"),
                };
                Some((setting.get(style), FieldFormat { step, decimals, unit, rotation: false }))
            }
        }
    }

    pub fn write(self, state: &mut EditorState, value: f32) {
        match self {
            Self::Property { layer, path, component } => state.set_property_component(layer, path, component, value),
            Self::Text { layer, setting } => state.set_text_setting(layer, setting, value),
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

/// Reads a typed value, ignoring units and spaces ("120 px", "45°", "50%"). Returns `None` for non-numbers.
pub fn parse_value(text: &str) -> Option<f32> {
    let cleaned: String = text.chars().filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+')).collect();
    cleaned.parse::<f32>().ok().filter(|value| value.is_finite())
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
        .children(keyframe_button)
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
        assert_eq!(parse_value(""), None);
    }
}
