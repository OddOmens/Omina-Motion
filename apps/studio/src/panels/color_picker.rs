//! The color picker: click a color chip anywhere in the editor and a wheel opens beside it.
//! Around the wheel is hue, from its center out is saturation, and the bar under it is
//! brightness. A hex field takes typed colors, and a row of swatches gives common ones.
//! Every change applies at once, and a drag on the wheel or bar is one undo step.

use std::sync::{Arc, OnceLock};

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable as _;
use gpui_kit::{
    AppContext as _, Context, DragMoveEvent, Entity, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Point, RenderImage, StatefulInteractiveElement, Styled, Subscription, TestSupportExt as _, Window,
    anchored, deferred, div, img, linear_color_stop, linear_gradient, point, px, rgba,
};
use studio_core::model::Rgba;

use crate::panels::chrome::{EmptyDragView, MeasuredBounds, Rect};
use crate::state::ColorTarget;
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// Common colors, shown under the wheel and in menus that offer a palette.
pub const SWATCHES: &[(&str, u32)] = &[
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

/// Diameter of the wheel on screen, in points.
const WHEEL: f32 = 180.0;
/// The wheel image is drawn at twice that, for sharp edges on high-density screens.
const WHEEL_PIXELS: u32 = 360;

/// An open picker: what it edits, where it opens, and its hue, saturation and value. These
/// are kept here rather than read back from the color, so the hue survives dragging to gray.
pub struct Picker {
    pub target: ColorTarget,
    at: Point<Pixels>,
    hue: f32,
    saturation: f32,
    value: f32,
    hex: Entity<InputState>,
    wheel: MeasuredBounds,
    bar: MeasuredBounds,
    /// While the wheel or bar is pressed, so the press and the drag after it are one undo step.
    pub pressed: bool,
    _hex_events: Subscription,
}

/// What the pointer drags inside the picker.
#[derive(Clone, Copy)]
enum PickerDrag {
    Wheel,
    Brightness,
}

/// The hue/saturation disc as a GPU image, made once: hue runs around it (red to the right,
/// turning clockwise), saturation from gray at the center to full at the rim, at full value.
fn wheel_image() -> Arc<RenderImage> {
    static IMAGE: OnceLock<Arc<RenderImage>> = OnceLock::new();
    IMAGE
        .get_or_init(|| {
            let size = WHEEL_PIXELS;
            let radius = size as f32 / 2.0;
            let mut bgra = Vec::with_capacity((size * size * 4) as usize);
            for y in 0..size {
                for x in 0..size {
                    let (dx, dy) = (x as f32 + 0.5 - radius, y as f32 + 0.5 - radius);
                    let distance = (dx * dx + dy * dy).sqrt();
                    let color = Rgba::from_hsv(dy.atan2(dx).to_degrees(), (distance / radius).min(1.0), 1.0);
                    // A one-pixel soft rim.
                    let alpha = (radius - distance + 0.5).clamp(0.0, 1.0);
                    let channel = |value: f32| (value * alpha * 255.0).round() as u8;
                    bgra.extend([channel(color.b), channel(color.g), channel(color.r), (alpha * 255.0).round() as u8]);
                }
            }
            let buffer = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(size, size, bgra).expect("the wheel holds size × size pixels");
            Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
        })
        .clone()
}

/// Hue and saturation at `position` on a wheel drawn in `bounds`.
fn wheel_pick(position: Point<Pixels>, bounds: Rect) -> (f32, f32) {
    let radius = bounds.width.max(1.0) / 2.0;
    let (dx, dy) = (f32::from(position.x) - (bounds.left + radius), f32::from(position.y) - (bounds.top + bounds.height / 2.0));
    (dy.atan2(dx).to_degrees().rem_euclid(360.0), ((dx * dx + dy * dy).sqrt() / radius).min(1.0))
}

/// Brightness at `position` along a bar drawn in `bounds` (dark on the left).
fn bar_pick(position: Point<Pixels>, bounds: Rect) -> f32 {
    ((f32::from(position.x) - bounds.left) / bounds.width.max(1.0)).clamp(0.0, 1.0)
}

fn gpui_color(color: Rgba) -> gpui_kit::Rgba {
    gpui_kit::Rgba { r: color.r, g: color.g, b: color.b, a: 1.0 }
}

/// Opens the picker for `target`, next to `at` (window coordinates).
pub fn open(workspace: &mut Workspace, target: ColorTarget, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Workspace>) {
    let current = workspace.state.color_of(target).unwrap_or(Rgba::hex(0xffffff));
    let (hue, saturation, value) = current.to_hsv();
    let hex = cx.new(|cx| InputState::new(window, cx).default_value(current.to_hex()));
    // A typed color applies on Enter or when the field loses focus.
    let hex_events = cx.subscribe_in(&hex, window, |this, input, event: &InputEvent, window, cx| {
        if !matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
            return;
        }
        let Some(color) = Rgba::parse_hex(&input.read(cx).value()) else { return };
        if let Some(picker) = this.ui.color_picker.as_mut() {
            (picker.hue, picker.saturation, picker.value) = color.to_hsv();
        }
        apply(this, color, window, cx);
    });
    workspace.ui.color_picker = Some(Picker {
        target,
        at,
        hue,
        saturation,
        value,
        hex,
        wheel: MeasuredBounds::default(),
        bar: MeasuredBounds::default(),
        pressed: false,
        _hex_events: hex_events,
    });
    cx.notify();
}

/// Writes `color` to the picker's target and shows it in the hex field.
fn apply(workspace: &mut Workspace, color: Rgba, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(picker) = workspace.ui.color_picker.as_ref() else { return };
    let target = picker.target;
    let hex = picker.hex.clone();
    workspace.state.set_color(target, color);
    hex.update(cx, |input, cx| input.set_value(color.to_hex(), window, cx));
    cx.notify();
}

/// Applies the picker's own hue, saturation and value.
fn apply_hsv(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(picker) = workspace.ui.color_picker.as_ref() else { return };
    let color = Rgba::from_hsv(picker.hue, picker.saturation, picker.value);
    apply(workspace, color, window, cx);
}

/// A clickable color chip that opens the picker for `target`.
pub fn swatch(id: impl Into<gpui_kit::ElementId>, target: ColorTarget, current: Option<Rgba>, cx: &mut Context<Workspace>) -> impl IntoElement {
    let fill = current.map_or(0x00000000, Rgba::to_rgba_u32);
    div()
        .id(id.into())
        .size(px(22.0))
        .flex_none()
        .rounded(px(5.0))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .bg(rgba(fill))
        .cursor_pointer()
        .hover(|style| style.border_color(hsla(color::TEXT)))
        // Pressing here would also count as a press outside an open picker, closing it; open
        // the new one on mouse-up so that close comes first.
        .on_mouse_up(MouseButton::Left, cx.listener(move |this, event: &gpui_kit::MouseUpEvent, window, cx| {
            open(this, target, event.position, window, cx);
        }))
        .test_support()
}

/// The open picker, drawn above everything, or nothing.
pub fn popover(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<impl IntoElement + use<>> {
    let picker = workspace.ui.color_picker.as_ref()?;
    let color = Rgba::from_hsv(picker.hue, picker.saturation, picker.value);
    let full = Rgba::from_hsv(picker.hue, picker.saturation, 1.0);

    // Where the marker sits on the wheel.
    let angle = picker.hue.to_radians();
    let reach = picker.saturation * WHEEL / 2.0;
    let marker = (WHEEL / 2.0 + angle.cos() * reach, WHEEL / 2.0 + angle.sin() * reach);
    let wheel = div()
        .id("color-wheel")
        .relative()
        .size(px(WHEEL))
        .flex_none()
        .cursor_crosshair()
        .child(picker.wheel.probe())
        .child(img(wheel_image()).absolute().size_full())
        // The disc shows hues at full brightness; darken it to show the current brightness.
        .child(div().absolute().size_full().rounded_full().bg(rgba(((1.0 - picker.value) * 255.0).round() as u32)))
        .child(
            div()
                .absolute()
                .left(px(marker.0 - 7.0))
                .top(px(marker.1 - 7.0))
                .size(px(14.0))
                .rounded_full()
                .border_2()
                .border_color(hsla(0xffffff))
                .bg(rgba(color.to_rgba_u32()))
                .shadow_md(),
        )
        .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            let Some(picker) = this.ui.color_picker.as_mut() else { return };
            (picker.hue, picker.saturation) = wheel_pick(event.position, picker.wheel.get());
            picker.pressed = true;
            apply_hsv(this, window, cx);
        }))
        .on_drag(PickerDrag::Wheel, |_, _, _, cx| cx.new(|_| EmptyDragView))
        .test_support();

    let bar = div()
        .id("color-brightness")
        .relative()
        .w(px(WHEEL))
        .h(px(14.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .bg(linear_gradient(90.0, linear_color_stop(gpui_color(Rgba::hex(0x000000)), 0.0), linear_color_stop(gpui_color(full), 1.0)))
        .cursor_ew_resize()
        .child(picker.bar.probe())
        .child(
            div()
                .absolute()
                .top(px(-2.0))
                .left(px(picker.value * WHEEL - 4.0))
                .w(px(8.0))
                .h(px(16.0))
                .rounded(px(3.0))
                .border_2()
                .border_color(hsla(0xffffff))
                .shadow_md(),
        )
        .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            let Some(picker) = this.ui.color_picker.as_mut() else { return };
            picker.value = bar_pick(event.position, picker.bar.get());
            picker.pressed = true;
            apply_hsv(this, window, cx);
        }))
        .on_drag(PickerDrag::Brightness, |_, _, _, cx| cx.new(|_| EmptyDragView))
        .test_support();

    let swatches = SWATCHES.iter().enumerate().map(|(index, &(_, hex))| {
        div()
            .id(("color-swatch", index))
            .size(px(16.0))
            .rounded(px(4.0))
            .border_1()
            .border_color(hsla(color::HAIRLINE))
            .bg(rgba(Rgba::hex(hex).to_rgba_u32()))
            .cursor_pointer()
            .hover(|style| style.border_color(hsla(color::TEXT)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| {
                let color = Rgba::hex(hex);
                if let Some(picker) = this.ui.color_picker.as_mut() {
                    (picker.hue, picker.saturation, picker.value) = color.to_hsv();
                }
                apply(this, color, window, cx);
            }))
            .test_support()
    });

    let body = div()
        .id("color-picker")
        .w(px(WHEEL + 24.0))
        .p_3()
        .flex()
        .flex_col()
        .gap_3()
        .rounded(px(10.0))
        .bg(hsla(color::POPOVER))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .shadow_lg()
        .text_size(px(size::TEXT))
        // Clicking anywhere else closes it.
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.ui.color_picker = None;
            cx.notify();
        }))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        // Dragging on the wheel or the bar keeps picking, even outside them.
        .on_drag_move(cx.listener(|this, event: &DragMoveEvent<PickerDrag>, window, cx| {
            let Some(picker) = this.ui.color_picker.as_mut() else { return };
            match *event.drag(cx) {
                PickerDrag::Wheel => (picker.hue, picker.saturation) = wheel_pick(event.event.position, picker.wheel.get()),
                PickerDrag::Brightness => picker.value = bar_pick(event.event.position, picker.bar.get()),
            }
            apply_hsv(this, window, cx);
        }))
        .child(wheel)
        .child(bar)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().size(px(22.0)).flex_none().rounded(px(5.0)).border_1().border_color(hsla(color::HAIRLINE)).bg(rgba(color.to_rgba_u32())))
                .child(div().flex_1().child(Input::new(&picker.hex).small())),
        )
        .child(div().flex().flex_wrap().gap_1().children(swatches))
        .test_support();
    let at = point(picker.at.x + px(12.0), picker.at.y - px(40.0));
    Some(deferred(anchored().position(at).snap_to_window().child(body)).with_priority(3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wheel_maps_angle_to_hue_and_distance_to_saturation() {
        let bounds = Rect { left: 0.0, top: 0.0, width: 100.0, height: 100.0 };
        let (hue, saturation) = wheel_pick(point(px(100.0), px(50.0)), bounds);
        assert!(hue.abs() < 0.01 && (saturation - 1.0).abs() < 0.01, "right edge is full red: {hue} {saturation}");
        let (hue, saturation) = wheel_pick(point(px(50.0), px(75.0)), bounds);
        assert!((hue - 90.0).abs() < 0.01 && (saturation - 0.5).abs() < 0.01, "halfway down is half-saturated 90°");
        assert_eq!(wheel_pick(point(px(500.0), px(50.0)), bounds).1, 1.0, "outside the rim is still the rim");
        assert_eq!(bar_pick(point(px(-20.0), px(0.0)), bounds), 0.0);
    }
}
