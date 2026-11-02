//! The color picker: click a color chip anywhere in the editor and it opens beside it.
//!
//! The Custom tab has a square for saturation (left to right) and brightness (top to bottom),
//! a hue slider, an opacity slider, hex and opacity fields, and the colors already used in the
//! project. For a shape's fill or stroke, it also switches between a solid color and linear or
//! radial gradients, and between a gradient's two colors. The Libraries tab lists the brands'
//! colors: picking one links it, so the color follows the library from then on. Every change
//! applies at once, and a press-and-drag is one undo step.

use std::sync::{Arc, OnceLock};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::{
    AnyElement, AppContext as _, Context, DragMoveEvent, Entity, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Point, RenderImage, StatefulInteractiveElement, Styled, Subscription, TestSupportExt as _, Window,
    anchored, deferred, div, img, linear_color_stop, linear_gradient, point, prelude::FluentBuilder as _, px, rgba,
};
use studio_core::model::{LinkId, Paint, PaintKind, Rgba};

use crate::panels::chrome::{EmptyDragView, MeasuredBounds, Rect};
use crate::state::{ColorTarget, PaintSlot};
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// Common colors, offered when the project has few colors of its own.
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

/// Width of the picker's controls, in points.
const WIDTH: f32 = 236.0;
/// Height of the saturation/brightness square.
const FIELD_HEIGHT: f32 = 200.0;
/// Height of the hue and opacity sliders.
const SLIDER: f32 = 14.0;

/// The picker's two tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerTab {
    Custom,
    Libraries,
}

/// An open picker: what it edits, where it opens, and its hue, saturation, value and opacity.
/// These are kept here rather than read back from the color, so the hue survives dragging to
/// gray or black.
pub struct Picker {
    pub target: ColorTarget,
    pub tab: PickerTab,
    at: Point<Pixels>,
    hue: f32,
    saturation: f32,
    value: f32,
    alpha: f32,
    pub(crate) hex: Entity<InputState>,
    pub(crate) opacity: Entity<InputState>,
    field: MeasuredBounds,
    hue_bar: MeasuredBounds,
    alpha_bar: MeasuredBounds,
    /// While the square or a slider is pressed, so the press and the drag after it are one undo step.
    pub pressed: bool,
    /// The eyedropper is on: the next click in the viewer picks the color there.
    pub sampling: bool,
    _events: [Subscription; 2],
}

/// What the pointer drags inside the picker.
#[derive(Clone, Copy)]
enum PickerDrag {
    Field,
    Hue,
    Alpha,
}

/// Builds a GPU image from a function of (x, y) giving RGBA bytes.
fn make_image(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Arc<RenderImage> {
    let mut bgra = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let [r, g, b, a] = pixel(x, y);
            bgra.extend([b, g, r, a]);
        }
    }
    let buffer = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(width, height, bgra).expect("the image holds width × height pixels");
    Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
}

/// Every hue from red back to red, left to right, made once.
fn hue_strip() -> Arc<RenderImage> {
    static IMAGE: OnceLock<Arc<RenderImage>> = OnceLock::new();
    IMAGE
        .get_or_init(|| {
            make_image(512, 4, |x, _| {
                let color = Rgba::from_hsv(x as f32 / 511.0 * 360.0, 1.0, 1.0);
                let channel = |value: f32| (value * 255.0).round() as u8;
                [channel(color.r), channel(color.g), channel(color.b), 255]
            })
        })
        .clone()
}

/// A gray checkerboard, shown behind the opacity slider, made once.
fn checkerboard() -> Arc<RenderImage> {
    static IMAGE: OnceLock<Arc<RenderImage>> = OnceLock::new();
    IMAGE
        .get_or_init(|| {
            let cell = 8;
            make_image(WIDTH as u32 * 2, SLIDER as u32 * 2, |x, y| {
                let shade = if (x / cell + y / cell) % 2 == 0 { 0x66 } else { 0x3a };
                [shade, shade, shade, 255]
            })
        })
        .clone()
}

/// The fraction (0–1) across `bounds` at `position`, and down it.
fn fractions(position: Point<Pixels>, bounds: Rect) -> (f32, f32) {
    let x = (f32::from(position.x) - bounds.left) / bounds.width.max(1.0);
    let y = (f32::from(position.y) - bounds.top) / bounds.height.max(1.0);
    (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0))
}

fn gpui_color(color: Rgba, alpha: f32) -> gpui_kit::Rgba {
    gpui_kit::Rgba { r: color.r, g: color.g, b: color.b, a: alpha }
}

impl Picker {
    fn color(&self) -> Rgba {
        Rgba::from_hsv(self.hue, self.saturation, self.value).with_alpha(self.alpha)
    }

    fn set_from(&mut self, color: Rgba) {
        let (hue, saturation, value) = color.to_hsv();
        // Keep the hue when the new color has none (gray), so the square keeps its color.
        if saturation > 0.0 && value > 0.0 {
            self.hue = hue;
        }
        (self.saturation, self.value, self.alpha) = (saturation, value, color.a);
    }
}

/// Opens the picker for `target`, next to `at` (window coordinates).
pub fn open(workspace: &mut Workspace, target: ColorTarget, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Workspace>) {
    let current = workspace.state.color_of(target).unwrap_or(Rgba::hex(0xffffff));
    let (hue, saturation, value) = current.to_hsv();
    let hex = cx.new(|cx| InputState::new(window, cx).default_value(current.to_hex().trim_start_matches('#').to_owned()));
    let opacity = cx.new(|cx| InputState::new(window, cx).default_value(format!("{}", (current.a * 100.0).round())));
    // Typed values apply on Enter or when the field loses focus.
    let committed = |event: &InputEvent| matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur);
    let hex_events = cx.subscribe_in(&hex, window, move |this, input, event: &InputEvent, window, cx| {
        if !committed(event) {
            return;
        }
        let Some(color) = Rgba::parse_hex(&input.read(cx).value()) else { return };
        let Some(picker) = this.ui.color_picker.as_mut() else { return };
        // Left as it was: writing it back would unlink a Library color for nothing.
        if color.to_hex() == picker.color().to_hex() {
            return;
        }
        let alpha = picker.alpha;
        picker.set_from(color.with_alpha(alpha));
        apply(this, window, cx);
    });
    let opacity_events = cx.subscribe_in(&opacity, window, move |this, input, event: &InputEvent, window, cx| {
        if !committed(event) {
            return;
        }
        let Some(percent) = crate::panels::fields::parse_value(&input.read(cx).value()) else { return };
        let Some(picker) = this.ui.color_picker.as_mut() else { return };
        // The field shows whole percents, so an unchanged one would round the opacity and unlink it.
        if percent == (picker.alpha * 100.0).round() {
            return;
        }
        picker.alpha = (percent / 100.0).clamp(0.0, 1.0);
        apply(this, window, cx);
    });
    workspace.ui.color_picker = Some(Picker {
        target,
        tab: PickerTab::Custom,
        at,
        hue,
        saturation,
        value,
        alpha: current.a,
        hex,
        opacity,
        field: MeasuredBounds::default(),
        hue_bar: MeasuredBounds::default(),
        alpha_bar: MeasuredBounds::default(),
        pressed: false,
        sampling: false,
        _events: [hex_events, opacity_events],
    });
    cx.notify();
}

/// Writes the picker's color to its target (unlinked, as it was picked by hand) and shows it in
/// the fields.
fn apply(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(picker) = workspace.ui.color_picker.as_ref() else { return };
    let (target, color) = (picker.target, picker.color());
    let (hex, opacity) = (picker.hex.clone(), picker.opacity.clone());
    workspace.state.set_color(target, color);
    show_in_fields(&hex, &opacity, color, window, cx);
    cx.notify();
}

fn show_in_fields(hex: &Entity<InputState>, opacity: &Entity<InputState>, color: Rgba, window: &mut Window, cx: &mut Context<Workspace>) {
    hex.update(cx, |input, cx| input.set_value(color.to_hex().trim_start_matches('#').to_owned(), window, cx));
    opacity.update(cx, |input, cx| input.set_value(format!("{}", (color.a * 100.0).round()), window, cx));
}

/// Uses a color picked with the eyedropper and turns the eyedropper off.
pub fn sampled(workspace: &mut Workspace, color: Rgba, window: &mut Window, cx: &mut Context<Workspace>) {
    if let Some(picker) = workspace.ui.color_picker.as_mut() {
        picker.sampling = false;
    }
    choose(workspace, color, window, cx);
}

/// Uses `color` as it is (a library color keeps its link) and moves the controls to it.
fn choose(workspace: &mut Workspace, color: Rgba, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(picker) = workspace.ui.color_picker.as_mut() else { return };
    picker.set_from(color);
    let (target, hex, opacity) = (picker.target, picker.hex.clone(), picker.opacity.clone());
    workspace.state.set_color(target, color);
    show_in_fields(&hex, &opacity, color, window, cx);
    cx.notify();
}

/// A clickable color chip that opens the picker for `target`. Linked colors show a small dot.
pub fn swatch(id: impl Into<gpui_kit::ElementId>, target: ColorTarget, current: Option<Rgba>, cx: &mut Context<Workspace>) -> impl IntoElement {
    let fill = current.map_or(0x00000000, Rgba::to_rgba_u32);
    let linked = current.is_some_and(|color| color.link.is_some());
    div()
        .id(id.into())
        .relative()
        .size(px(22.0))
        .flex_none()
        .rounded(px(5.0))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .bg(rgba(fill))
        .cursor_pointer()
        .hover(|style| style.border_color(hsla(color::TEXT)))
        .when(linked, |chip| {
            chip.child(div().absolute().right(px(-3.0)).bottom(px(-3.0)).size(px(8.0)).rounded_full().border_1().border_color(hsla(color::PANEL)).bg(hsla(color::HIGHLIGHT)))
        })
        // Pressing here would also count as a press outside an open picker, closing it; open
        // the new one on mouse-up so that close comes first.
        .on_mouse_up(MouseButton::Left, cx.listener(move |this, event: &gpui_kit::MouseUpEvent, window, cx| {
            open(this, target, event.position, window, cx);
        }))
        .test_support()
}

/// A round slider thumb at `fraction` along a slider, filled with `fill`.
fn thumb(fraction: f32, fill: u32) -> impl IntoElement {
    div()
        .absolute()
        .top(px(-3.0))
        .left(px(fraction * WIDTH - (SLIDER + 6.0) / 2.0))
        .size(px(SLIDER + 6.0))
        .rounded_full()
        .border_3()
        .border_color(hsla(0xffffff))
        .bg(rgba(fill))
        .shadow_md()
}

/// A slider track, measuring where it is drawn.
fn track(id: &'static str, bounds: &MeasuredBounds) -> gpui_kit::Stateful<gpui_kit::Div> {
    div().id(id).relative().w(px(WIDTH)).h(px(SLIDER)).flex_none().cursor_pointer().child(bounds.probe())
}

/// Starts a press on one of the picker's controls.
fn press(drag: PickerDrag, cx: &mut Context<Workspace>) -> impl Fn(&MouseDownEvent, &mut Window, &mut gpui_kit::App) + 'static {
    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
        cx.stop_propagation();
        pick(this, drag, event.position, window, cx);
        if let Some(picker) = this.ui.color_picker.as_mut() {
            picker.pressed = true;
        }
    })
}

/// Moves a control to `position` and applies the color.
fn pick(workspace: &mut Workspace, drag: PickerDrag, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(picker) = workspace.ui.color_picker.as_mut() else { return };
    match drag {
        PickerDrag::Field => {
            let (x, y) = fractions(position, picker.field.get());
            (picker.saturation, picker.value) = (x, 1.0 - y);
        }
        PickerDrag::Hue => picker.hue = fractions(position, picker.hue_bar.get()).0 * 360.0,
        PickerDrag::Alpha => picker.alpha = fractions(position, picker.alpha_bar.get()).0,
    }
    apply(workspace, window, cx);
}

/// For a shape's fill or stroke: buttons for solid, linear and radial paint, and (for a
/// gradient) which of its two colors is being picked.
fn paint_controls(workspace: &Workspace, layer: studio_core::model::Id, slot: PaintSlot, stop: usize, cx: &mut Context<Workspace>) -> AnyElement {
    let paint = workspace.state.shape_paint(layer, slot);
    let kind = paint.map(Paint::kind);
    let chip = |choice: PaintKind, look: gpui_kit::Div, cx: &mut Context<Workspace>| {
        div()
            .id(("paint-kind", choice as usize))
            .size(px(28.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .when(kind == Some(choice), |chip| chip.bg(hsla(color::ROW_SELECTED)))
            .hover(|style| style.bg(hsla(color::HOVER)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                let current = this.state.shape_paint(layer, slot).unwrap_or(Paint::Solid(Rgba::hex(0xffffff)));
                this.state.set_shape_paint(layer, slot, Some(current.into_kind(choice)));
                if let Some(picker) = this.ui.color_picker.as_mut() {
                    picker.target = ColorTarget::Shape { layer, slot, stop: 0 };
                }
                cx.notify();
            }))
            .child(look)
            .test_support()
    };
    let ink = gpui_color(Rgba::hex(0xd4d4d8), 1.0);
    let clear = gpui_color(Rgba::hex(0xd4d4d8), 0.0);
    let solid = div().size(px(14.0)).rounded(px(3.0)).bg(ink);
    let linear = div().size(px(14.0)).rounded(px(3.0)).border_1().border_color(ink).bg(linear_gradient(90.0, linear_color_stop(clear, 0.0), linear_color_stop(ink, 1.0)));
    let radial = div().size(px(14.0)).rounded_full().border_1().border_color(ink).flex().items_center().justify_center().child(div().size(px(5.0)).rounded_full().bg(ink));
    let mut row = div()
        .flex()
        .items_center()
        .gap_1()
        .child(chip(PaintKind::Solid, solid, cx))
        .child(chip(PaintKind::Linear, linear, cx))
        .child(chip(PaintKind::Radial, radial, cx));
    if let Some(paint) = paint.filter(|paint| paint.kind() != PaintKind::Solid) {
        row = row.child(div().flex_1());
        for (index, color) in paint.colors().into_iter().enumerate() {
            row = row.child(
                div()
                    .id(("gradient-stop", index))
                    .size(px(20.0))
                    .rounded(px(5.0))
                    .border_2()
                    .border_color(hsla(if index == stop { 0xffffff } else { color::HAIRLINE }))
                    .bg(rgba(color.to_rgba_u32()))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let Some(color) = this.state.color_of(ColorTarget::Shape { layer, slot, stop: index }) else { return };
                        if let Some(picker) = this.ui.color_picker.as_mut() {
                            picker.target = ColorTarget::Shape { layer, slot, stop: index };
                            picker.set_from(color);
                            let (hex, opacity) = (picker.hex.clone(), picker.opacity.clone());
                            show_in_fields(&hex, &opacity, color, window, cx);
                        }
                        cx.notify();
                    }))
                    .test_support(),
            );
        }
    }
    row.into_any_element()
}

/// A row of colors to click.
fn color_row(key: &'static str, colors: Vec<Rgba>, cx: &mut Context<Workspace>) -> impl IntoElement {
    div().flex().flex_wrap().gap_1p5().children(colors.into_iter().enumerate().map(|(index, color)| {
        div()
            .id((key, index))
            .size(px(22.0))
            .rounded(px(5.0))
            .border_1()
            .border_color(hsla(color::HAIRLINE))
            .bg(rgba(color.to_rgba_u32()))
            .cursor_pointer()
            .hover(|style| style.border_color(hsla(color::TEXT)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| choose(this, color, window, cx)))
            .test_support()
    }))
}

/// "Save to library": adds the picker's color to a brand and links the target to it.
fn save_to_library(brands: Vec<(LinkId, String)>, cx: &mut Context<Workspace>) -> AnyElement {
    let workspace = cx.weak_entity();
    Button::new("color-save-to-library").ghost().xsmall().icon(Icon::new(IconName::Plus)).label("Save to library").dropdown_menu(move |menu, _, _| {
        brands.iter().fold(menu, |menu, (brand, name)| {
            let (workspace, brand) = (workspace.clone(), *brand);
            menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _, cx| {
                workspace
                    .update(cx, |this, cx| {
                        let Some(picker) = this.ui.color_picker.as_ref() else { return };
                        let (target, color) = (picker.target, picker.color());
                        if let Some(id) = this.state.add_library_color(brand, color) {
                            this.state.set_color(target, color.linked(Some(id)));
                        }
                        cx.notify();
                    })
                    .ok();
            }))
        })
    })
    .into_any_element()
}

fn custom_tab(workspace: &Workspace, picker: &Picker, cx: &mut Context<Workspace>) -> gpui_kit::Div {
    let color = picker.color();
    let pure = Rgba::from_hsv(picker.hue, 1.0, 1.0);
    let opaque = color.with_alpha(1.0);
    let white = |alpha| gpui_color(Rgba::hex(0xffffff), alpha);
    let black = |alpha| gpui_color(Rgba::hex(0x000000), alpha);

    // Saturation runs left to right over the pure hue, brightness top to bottom.
    let field = div()
        .id("color-field")
        .relative()
        .w(px(WIDTH))
        .h(px(FIELD_HEIGHT))
        .rounded(px(8.0))
        .bg(rgba(pure.to_rgba_u32()))
        .cursor_crosshair()
        .child(picker.field.probe())
        .child(div().absolute().size_full().rounded(px(8.0)).bg(linear_gradient(90.0, linear_color_stop(white(1.0), 0.0), linear_color_stop(white(0.0), 1.0))))
        .child(div().absolute().size_full().rounded(px(8.0)).bg(linear_gradient(180.0, linear_color_stop(black(0.0), 0.0), linear_color_stop(black(1.0), 1.0))))
        .child(
            div()
                .absolute()
                .left(px(picker.saturation * WIDTH - 9.0))
                .top(px((1.0 - picker.value) * FIELD_HEIGHT - 9.0))
                .size(px(18.0))
                .rounded_full()
                .border_3()
                .border_color(hsla(0xffffff))
                .bg(rgba(opaque.to_rgba_u32()))
                .shadow_md(),
        )
        .on_mouse_down(MouseButton::Left, press(PickerDrag::Field, cx))
        .on_drag(PickerDrag::Field, |_, _, _, cx| cx.new(|_| EmptyDragView))
        .test_support();

    let hue = track("color-hue", &picker.hue_bar)
        .child(div().absolute().size_full().rounded(px(SLIDER / 2.0)).overflow_hidden().child(img(hue_strip()).size_full()))
        .child(thumb(picker.hue / 360.0, pure.to_rgba_u32()))
        .on_mouse_down(MouseButton::Left, press(PickerDrag::Hue, cx))
        .on_drag(PickerDrag::Hue, |_, _, _, cx| cx.new(|_| EmptyDragView))
        .test_support();

    let alpha = track("color-alpha", &picker.alpha_bar)
        .child(div().absolute().size_full().rounded(px(SLIDER / 2.0)).overflow_hidden().child(img(checkerboard()).size_full()))
        .child(div().absolute().size_full().rounded(px(SLIDER / 2.0)).bg(linear_gradient(90.0, linear_color_stop(gpui_color(opaque, 0.0), 0.0), linear_color_stop(gpui_color(opaque, 1.0), 1.0))))
        .child(thumb(picker.alpha, color.to_rgba_u32()))
        .on_mouse_down(MouseButton::Left, press(PickerDrag::Alpha, cx))
        .on_drag(PickerDrag::Alpha, |_, _, _, cx| cx.new(|_| EmptyDragView))
        .test_support();

    let values = div()
        .flex()
        .items_center()
        .gap_1p5()
        .child(div().h(px(28.0)).px_2().flex().items_center().rounded(px(6.0)).border_1().border_color(hsla(color::HAIRLINE)).text_color(hsla(color::MUTED)).child("Hex"))
        .child(div().flex_1().child(Input::new(&picker.hex).small().prefix(div().text_color(hsla(color::MUTED)).child("#"))))
        .child(div().w(px(64.0)).child(Input::new(&picker.opacity).small().suffix(div().text_color(hsla(color::MUTED)).child("%"))))
        // The eyedropper: click it, then click anywhere in the viewer's picture.
        .child(
            Button::new("picker-eyedropper")
                .icon(Icon::new(IconName::Pipette))
                .small()
                .tooltip("Pick a color from the viewer")
                .map(|button| if picker.sampling { button.primary() } else { button.ghost() })
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(picker) = this.ui.color_picker.as_mut() {
                        picker.sampling = !picker.sampling;
                    }
                    cx.notify();
                })),
        );

    // Colors already in the project, then common ones to fill out a short row.
    let mut used = workspace.state.project_colors();
    for &(_, hex) in SWATCHES {
        if !used.iter().any(|color| color.same_look(Rgba::hex(hex))) {
            used.push(Rgba::hex(hex));
        }
    }
    used.truncate(16);

    let brands: Vec<(LinkId, String)> = workspace.state.libraries.brands.iter().map(|brand| (brand.id, brand.name.clone())).collect();
    let save = if !brands.is_empty() && !matches!(picker.target, ColorTarget::Library(_)) { Some(save_to_library(brands, cx)) } else { None };

    let mut tab = div().flex().flex_col().gap_3();
    if let ColorTarget::Shape { layer, slot, stop } = picker.target {
        tab = tab.child(paint_controls(workspace, layer, slot, stop, cx));
    }
    tab.child(field)
        .child(div().flex().flex_col().gap_3().pt_1().child(hue).child(alpha))
        .child(values)
        .child(div().h(px(1.0)).bg(hsla(color::HAIRLINE)))
        .child(
            div()
                .flex()
                .items_center()
                .text_size(px(size::SMALL_TEXT))
                .text_color(hsla(color::MUTED))
                .child(div().flex_1().child("In this project"))
                .children(save),
        )
        .child(color_row("color-swatch", used, cx))
}

fn libraries_tab(workspace: &Workspace, cx: &mut Context<Workspace>) -> gpui_kit::Div {
    let brands = &workspace.state.libraries.brands;
    let mut tab = div().flex().flex_col().gap_3().w(px(WIDTH));
    if brands.iter().all(|brand| brand.colors.is_empty()) {
        return tab.child(div().text_color(hsla(color::MUTED)).child("No library colors yet. Make a brand in the Libraries panel, or save a color to one from the Custom tab."));
    }
    for brand in brands.iter().filter(|brand| !brand.colors.is_empty()) {
        let colors = brand.colors.iter().map(|item| item.color.linked(Some(item.id))).collect();
        tab = tab
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_size(px(size::SMALL_TEXT))
                    .text_color(hsla(color::MUTED))
                    .child(Icon::new(IconName::Folder).size(px(12.0)))
                    .child(brand.name.clone()),
            )
            .child(color_row("library-swatch", colors, cx));
    }
    tab
}

/// The open picker, drawn above everything, or nothing.
pub fn popover(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<impl IntoElement + use<>> {
    let picker = workspace.ui.color_picker.as_ref()?;
    // A library color can't be linked to another library color.
    let can_link = !matches!(picker.target, ColorTarget::Library(_));
    let tab_button = |label: &'static str, tab: PickerTab, cx: &mut Context<Workspace>| {
        let selected = picker.tab == tab;
        div()
            .id(("picker-tab", tab as usize))
            .px_2p5()
            .py_1()
            .rounded(px(6.0))
            .when(selected, |button| button.bg(hsla(color::RAISED)))
            .text_color(hsla(if selected { color::TEXT } else { color::MUTED }))
            .hover(|style| style.text_color(hsla(color::TEXT)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(picker) = this.ui.color_picker.as_mut() {
                    picker.tab = tab;
                }
                cx.notify();
            }))
            .child(label)
            .test_support()
    };
    let close = div()
        .id("color-picker-close")
        .size(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .text_color(hsla(color::MUTED))
        .hover(|style| style.bg(hsla(color::HOVER)).text_color(hsla(color::TEXT)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(|this, _, _, cx| {
            this.ui.color_picker = None;
            cx.notify();
        }))
        .child(Icon::new(IconName::X).size(px(14.0)))
        .test_support();
    let header = div()
        .flex()
        .items_center()
        .gap_1()
        .child(tab_button("Custom", PickerTab::Custom, cx))
        .when(can_link, |header| header.child(tab_button("Libraries", PickerTab::Libraries, cx)))
        .child(div().flex_1())
        .child(close);

    // Linked colors say where they come from, with a button to stop following the library.
    let linked = workspace.state.linked_color(picker.target).map(|(brand, name)| {
        let target = picker.target;
        div()
            .flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .py_1()
            .rounded(px(6.0))
            .bg(hsla(color::INSET))
            .text_size(px(size::SMALL_TEXT))
            .child(Icon::new(IconName::Link).size(px(12.0)).text_color(hsla(color::HIGHLIGHT)))
            .child(div().flex_1().truncate().child(format!("{brand} · {name}")))
            .child(
                div()
                    .id("color-unlink")
                    .text_color(hsla(color::MUTED))
                    .hover(|style| style.text_color(hsla(color::TEXT)))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(color) = this.state.color_of(target) {
                            this.state.set_color(target, color.linked(None));
                        }
                        cx.notify();
                    }))
                    .child("Detach")
                    .test_support(),
            )
    });

    let content = match picker.tab {
        PickerTab::Libraries if can_link => libraries_tab(workspace, cx),
        _ => custom_tab(workspace, picker, cx),
    };
    let body = div()
        .id("color-picker")
        .w(px(WIDTH + 24.0))
        .p_3()
        .flex()
        .flex_col()
        .gap_3()
        .rounded(px(12.0))
        .bg(hsla(color::POPOVER))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .shadow_lg()
        .text_size(px(size::TEXT))
        .text_color(hsla(color::TEXT))
        // Clicking anywhere else closes it.
        // While the eyedropper is on, it stays open for the click in the viewer.
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            if this.ui.color_picker.as_ref().is_some_and(|picker| picker.sampling) {
                return;
            }
            this.ui.color_picker = None;
            cx.notify();
        }))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        // Dragging on the square or a slider keeps picking, even outside them.
        .on_drag_move(cx.listener(|this, event: &DragMoveEvent<PickerDrag>, window, cx| {
            let drag = *event.drag(cx);
            pick(this, drag, event.event.position, window, cx);
        }))
        .child(header)
        .children(linked)
        .child(content)
        .test_support();
    let at = point(picker.at.x + px(12.0), picker.at.y - px(60.0));
    Some(deferred(anchored().position(at).snap_to_window().child(body)).with_priority(3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_map_to_fractions_inside_the_control() {
        let bounds = Rect { left: 100.0, top: 50.0, width: 200.0, height: 100.0 };
        assert_eq!(fractions(point(px(200.0), px(75.0)), bounds), (0.5, 0.25));
        assert_eq!(fractions(point(px(0.0), px(500.0)), bounds), (0.0, 1.0), "outside, it holds at the edge");
    }
}
