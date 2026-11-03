//! The Color tab of the Properties panel, after Premiere's Lumetri Color: grading for the
//! selected clip or for the whole video.
//!
//! The tab starts empty, with a tile for each kind of correction. Each correction is a card
//! holding one catalog effect (Basic Correction, a LUT, Color Wheels, Tone Curve, Levels,
//! Vignette) on the graded layer, listed in the order the effects apply. Its controls are
//! sliders over the effect's own parameters, with tracks colored by what they do (blue to amber
//! for temperature, say); each also takes a typed or scrubbed value, and a double-click puts it
//! back. Keyframing is in the Effects tab, which shows the same effects. Grading the whole video
//! puts the effects on the Color Grade adjustment layer at the top of the timeline (made the
//! first time; see `state::grade`).

use std::sync::{Arc, OnceLock};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable as _, WindowExt as _};
use gpui_kit::{
    AnyElement, AppContext as _, Context, DragMoveEvent, ElementId, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Point, RenderImage, StatefulInteractiveElement, Styled, TestSupportExt as _, Window, div, img,
    linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px, relative,
};
use studio_core::model::{Id, LutSource, Rgba};
use studio_core::property::PropertyPath;

use super::chrome::{CardHeading, EmptyDragView, MeasuredBounds, card, card_list, icon_button};
use super::fields::{FieldTarget, value_field};
use crate::state::GradeTarget;
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// A kind of correction the tab offers.
pub struct Section {
    /// The catalog effect it is.
    effect: &'static str,
    title: &'static str,
    /// Names its card's elements, and remembers whether the card is folded.
    key: &'static str,
    icon: IconName,
    /// A few words on what it is for, on its tile in the empty tab.
    about: &'static str,
}

/// The corrections, in the order the tab offers them.
const SECTIONS: [Section; 6] = [
    Section { effect: "Basic Correction", title: "Basic Correction", key: "grade-basic", icon: IconName::Sun, about: "White balance, light and saturation" },
    Section { effect: "Apply LUT", title: "Creative Look (LUT)", key: "grade-lut", icon: IconName::Palette, about: "A film look, or your own .cube" },
    Section { effect: "Color Wheels", title: "Color Wheels", key: "grade-wheels", icon: IconName::CircleDot, about: "Tint shadows, midtones, highlights" },
    Section { effect: "Tone Curve", title: "Curves", key: "grade-curve", icon: IconName::Spline, about: "Lift or lower bands of brightness" },
    Section { effect: "Levels", title: "Levels", key: "grade-levels", icon: IconName::ChartColumn, about: "Set the black and white points" },
    Section { effect: "Vignette", title: "Vignette", key: "grade-vignette", icon: IconName::Aperture, about: "Darken the edges of the frame" },
];

/// Each color wheel's title, and its hue, amount and brightness parameters in Color Wheels.
const WHEELS: [(&str, usize, usize, usize); 3] = [("Shadows", 0, 1, 2), ("Midtones", 3, 4, 5), ("Highlights", 6, 7, 8)];
/// The largest and smallest a color wheel is drawn; between them it fits the panel's width.
const WHEEL_MAX: f32 = 104.0;
const WHEEL_MIN: f32 = 64.0;

/// A color wheel being dragged: which one, on which layer's Color Wheels effect.
#[derive(Clone, Copy)]
struct WheelDrag {
    index: usize,
    layer: Id,
    effect: Id,
}

/// A slider being dragged: one parameter of one effect.
#[derive(Clone, Copy)]
struct SliderDrag {
    layer: Id,
    effect: Id,
    parameter: usize,
}

/// What a slider's track shows behind it, so the direction it changes the picture reads at a
/// glance.
#[derive(Clone, Copy)]
enum Track {
    Plain,
    /// Blue to amber.
    Temperature,
    /// Green to magenta.
    Tint,
    /// Black to white.
    Light,
    /// Gray to full color.
    Saturation,
    /// Around the color wheel, red to red (for hue angles from 0° to 360°).
    Hue,
}

/// What importing a `.cube` file does once it is in the LUT library.
#[derive(Clone, Copy)]
pub enum LutImport {
    /// Use it on this Apply LUT effect (layer, effect).
    Effect(Id, Id),
    /// Add it as an Apply LUT effect to the selected clips.
    Selection,
}

/// Asks for a `.cube` file, copies it into the LUT library and uses it as `then` says. A file
/// that isn't a readable LUT is refused, with the reason.
pub fn import_lut(then: LutImport, window: &mut Window, cx: &mut Context<Workspace>) {
    let chosen = cx.prompt_for_paths(gpui_kit::PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Import LUT".into()) });
    cx.spawn_in(window, async move |this, cx| {
        let Ok(Ok(Some(paths))) = chosen.await else { return };
        this.update_in(cx, |this, window, cx| {
            for path in paths {
                use_imported_lut(this, &path, then, window, cx);
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Imports the LUT at `path` into the library and uses it as `then` says.
pub fn use_imported_lut(workspace: &mut Workspace, path: &std::path::Path, then: LutImport, window: &mut Window, cx: &mut Context<Workspace>) {
    match crate::lut_library::import(path) {
        Ok(saved) => {
            workspace.ui.user_luts = crate::lut_library::list();
            let source = LutSource::File(saved);
            match then {
                LutImport::Effect(layer, effect) => workspace.state.set_effect_lut(layer, effect, Some(source)),
                LutImport::Selection => {
                    for layer in workspace.state.selected_layers.clone() {
                        workspace.state.add_lut(layer, source.clone());
                    }
                }
            }
        }
        Err(problem) => window.push_notification(problem.as_str(), cx),
    }
}

/// The LUT menu for an Apply LUT effect: Studio's looks, the imported LUTs, and Import. Below it,
/// a warning when the chosen file is missing.
pub fn lut_chooser(layer: Id, effect: Id, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let current = workspace.state.effect_lut(layer, effect).cloned();
    let mut files = workspace.ui.user_luts.clone();
    // A file chosen from outside the library (in a project from another machine, say) is listed too.
    if let Some(LutSource::File(path)) = &current
        && !files.contains(path)
    {
        files.insert(0, path.clone());
    }
    let label = current.as_ref().map_or_else(|| "Choose a LUT…".to_owned(), LutSource::name);
    let weak = cx.weak_entity();
    let chosen = current.clone();
    let menu = Button::new(ElementId::Name(format!("lut-menu:{}", effect.0).into())).outline().small().label(label).dropdown_caret(true).w_full().dropdown_menu(move |menu, _, _| {
        let choose = |menu: gpui_kit::component::menu::PopupMenu, name: String, source: LutSource| {
            let weak = weak.clone();
            let checked = chosen.as_ref() == Some(&source);
            menu.item(PopupMenuItem::new(name).checked(checked).on_click(move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    this.state.set_effect_lut(layer, effect, Some(source.clone()));
                    cx.notify();
                })
                .ok();
            }))
        };
        let mut menu = menu.scrollable(true).max_h(px(420.0));
        for (index, (group, looks)) in studio_core::grade::LOOK_GROUPS.iter().enumerate() {
            if index > 0 {
                menu = menu.separator();
            }
            menu = menu.label(*group);
            for look in *looks {
                menu = choose(menu, (*look).to_owned(), LutSource::Look((*look).to_owned()));
            }
        }
        if !files.is_empty() {
            menu = menu.separator().label("My LUTs");
            for path in &files {
                let source = LutSource::File(path.clone());
                menu = choose(menu, source.name(), source);
            }
        }
        let weak = weak.clone();
        menu.separator().item(PopupMenuItem::new("Import .cube File…").on_click(move |_, window, cx| {
            weak.update(cx, |_, cx| import_lut(LutImport::Effect(layer, effect), window, cx)).ok();
        }))
    });
    div()
        .flex()
        .flex_col()
        .gap_1()
        .py_1()
        .child(menu)
        .children(lut_warning(current.as_ref()).map(|text| div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(text)))
        .into_any_element()
}

/// Why an Apply LUT effect changes nothing, if it doesn't: no LUT chosen, or its file is missing.
fn lut_warning(current: Option<&LutSource>) -> Option<String> {
    match current {
        Some(LutSource::File(path)) if !path.exists() => Some(format!("{} is missing, so nothing is applied. Choose it again or import it.", path.display())),
        None => Some("No LUT chosen yet, so nothing is applied.".to_owned()),
        _ => None,
    }
}

/// The picture of what `source` does to the current frame, `width` × `height` points, or a
/// plain box while it is being made (and for no LUT).
pub fn lut_thumbnail(source: Option<&LutSource>, width: impl Into<gpui_kit::Length> + Clone, height: Pixels, workspace: &Workspace) -> AnyElement {
    let picture = source.and_then(|source| workspace.ui.lut_previews.picture(source));
    div()
        .w(width)
        .h(height)
        .flex_none()
        .rounded(px(4.0))
        .overflow_hidden()
        .bg(hsla(color::INSET))
        .children(picture.map(|picture| gpui_kit::StyledImage::object_fit(img(picture).size_full(), gpui_kit::ObjectFit::Cover)))
        .into_any_element()
}

/// One LUT in the browser: its picture and name. Clicking it puts it on the effect.
fn lut_tile(index: usize, source: LutSource, chosen: bool, (layer, effect): (Id, Id), workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let name = source.name();
    div()
        .id(("lut-tile", index))
        .flex()
        .flex_col()
        .gap_0p5()
        .min_w_0()
        .cursor_pointer()
        .child(
            div()
                .rounded(px(5.0))
                .border_2()
                .border_color(if chosen { hsla(color::ACCENT) } else { gpui_kit::transparent_black() })
                .child(lut_thumbnail(Some(&source), gpui_kit::relative(1.0), px(48.0), workspace)),
        )
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(if chosen { color::TEXT } else { color::MUTED })).truncate().child(name))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.state.set_effect_lut(layer, effect, Some(source.clone()));
            cx.notify();
        }))
        .test_support()
        .into_any_element()
}

/// The Color panel's LUT: the chosen one's picture and name, and a Browse button that opens
/// every LUT as a picture of the current frame, grouped as the looks are, then the imported
/// ones and Import.
fn lut_browser(layer: Id, effect: Id, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let current = workspace.state.effect_lut(layer, effect).cloned();
    let open = workspace.ui.lut_browser == Some(effect);
    let label = current.as_ref().map_or_else(|| "No LUT".to_owned(), LutSource::name);
    let chosen_row = div()
        .flex()
        .items_center()
        .gap_2()
        .child(lut_thumbnail(current.as_ref(), px(64.0), px(36.0), workspace))
        .child(div().flex_1().min_w_0().truncate().child(label))
        .child(
            Button::new(ElementId::Name(format!("lut-browse:{}", effect.0).into()))
                .small()
                .label(if open { "Done" } else { "Browse" })
                .map(|button| if open { button.primary() } else { button.outline() })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ui.lut_browser = if this.ui.lut_browser == Some(effect) { None } else { Some(effect) };
                    cx.notify();
                })),
        );
    let mut browser = div().flex().flex_col().gap_1().py_1().child(chosen_row);
    browser = browser.children(lut_warning(current.as_ref()).map(|text| div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(text)));
    if !open {
        return browser.into_any_element();
    }
    let group_heading = |title: &'static str| div().pt_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(title);
    let grid = || div().grid().grid_cols(3).gap_x_1p5().gap_y_1();
    let mut index = 0;
    for (group, looks) in studio_core::grade::LOOK_GROUPS {
        let mut tiles = grid();
        for look in *looks {
            let source = LutSource::Look((*look).to_owned());
            tiles = tiles.child(lut_tile(index, source.clone(), current.as_ref() == Some(&source), (layer, effect), workspace, cx));
            index += 1;
        }
        browser = browser.child(group_heading(group)).child(tiles);
    }
    // The imported LUTs, and one chosen from outside the library (a project from another machine).
    let mut files: Vec<LutSource> = workspace.ui.user_luts.iter().map(|path| LutSource::File(path.clone())).collect();
    if let Some(source @ LutSource::File(_)) = &current
        && !files.contains(source)
    {
        files.insert(0, source.clone());
    }
    let mut tiles = grid();
    for source in files {
        tiles = tiles.child(lut_tile(index, source.clone(), current.as_ref() == Some(&source), (layer, effect), workspace, cx));
        index += 1;
    }
    let import = div()
        .id(ElementId::Name(format!("lut-import:{}", effect.0).into()))
        .flex()
        .flex_col()
        .gap_0p5()
        .cursor_pointer()
        .child(
            div()
                .h(px(52.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .border_1()
                .border_dashed()
                .border_color(hsla(color::HAIRLINE))
                .text_color(hsla(color::MUTED))
                .hover(|style| style.text_color(hsla(color::TEXT)).border_color(hsla(color::TEXT)))
                .child(Icon::new(IconName::Plus).size(px(16.0))),
        )
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Import .cube…"))
        .on_click(cx.listener(move |_, _, window, cx| import_lut(LutImport::Effect(layer, effect), window, cx)))
        .test_support();
    browser.child(group_heading("My LUTs")).child(tiles.child(import)).into_any_element()
}

/// The hue circle shown on each color wheel: hue around it, and stronger toward the edge.
fn wheel_image() -> Arc<RenderImage> {
    static IMAGE: OnceLock<Arc<RenderImage>> = OnceLock::new();
    IMAGE
        .get_or_init(|| {
            let side = 208u32;
            let radius = side as f32 / 2.0;
            super::color_picker::make_image(side, side, |x, y| {
                let (dx, dy) = (x as f32 + 0.5 - radius, y as f32 + 0.5 - radius);
                let distance = (dx * dx + dy * dy).sqrt() / radius;
                if distance > 1.0 {
                    return [0, 0, 0, 0];
                }
                let hue = (-dy).atan2(dx).to_degrees().rem_euclid(360.0);
                let color = Rgba::from_hsv(hue, distance * 0.85, 0.55);
                // A one-pixel soft edge, so the circle is not jagged.
                let alpha = ((1.0 - distance) * radius).clamp(0.0, 1.0);
                let channel = |value: f32| (value * 255.0).round() as u8;
                [channel(color.r), channel(color.g), channel(color.b), channel(alpha)]
            })
        })
        .clone()
}

/// The hue (degrees) and amount (0–100) at `position` on a wheel with `bounds`.
fn wheel_value(position: Point<Pixels>, bounds: super::chrome::Rect) -> (f32, f32) {
    let radius = (bounds.width.min(bounds.height) / 2.0).max(1.0);
    let dx = f32::from(position.x) - (bounds.left + bounds.width / 2.0);
    let dy = f32::from(position.y) - (bounds.top + bounds.height / 2.0);
    let hue = (-dy).atan2(dx).to_degrees().rem_euclid(360.0);
    let amount = ((dx * dx + dy * dy).sqrt() / radius).min(1.0) * 100.0;
    (hue, amount)
}

/// Moves a color wheel's dot to `position`, setting its hue and amount.
fn drag_wheel(workspace: &mut Workspace, drag: WheelDrag, position: Point<Pixels>) {
    let (hue, amount) = wheel_value(position, workspace.ui.wheel_bounds[drag.index].get());
    let (_, hue_parameter, amount_parameter, _) = WHEELS[drag.index];
    let state = &mut workspace.state;
    state.set_property_component(drag.layer, PropertyPath::Effect { effect: drag.effect, parameter: hue_parameter }, 0, hue);
    state.set_property_component(drag.layer, PropertyPath::Effect { effect: drag.effect, parameter: amount_parameter }, 0, amount);
}

/// The value of one of an effect's parameters at the playhead (0 if it is missing).
fn parameter_value(layer: Id, effect: Id, parameter: usize, workspace: &Workspace) -> f32 {
    let frame = workspace.state.current_frame;
    workspace.state.composition().layer(layer).and_then(|layer| layer.property(PropertyPath::Effect { effect, parameter }, frame)).map_or(0.0, |snapshot| snapshot.value.component(0))
}

/// How wide each color wheel is drawn: as large as fits three across the panel.
fn wheel_size(workspace: &Workspace) -> f32 {
    // The panel's side padding, the card's padding and the gaps between the wheels.
    let room = workspace.ui.grade_bounds.get().width - 20.0 - 24.0 - 16.0;
    if room <= 0.0 { WHEEL_MAX } else { (room / 3.0).clamp(WHEEL_MIN, WHEEL_MAX) }
}

/// The three color wheels of a Color Wheels effect, each with its brightness slider under it.
/// Drag the dot toward a hue to tint that range; double-click a wheel to take its tint off.
fn wheels(layer: Id, effect: Id, workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let side = wheel_size(workspace);
    div().flex().justify_between().gap_2().py_1().children(WHEELS.iter().enumerate().map(|(index, &(title, hue_parameter, amount_parameter, brightness_parameter))| {
        let hue = parameter_value(layer, effect, hue_parameter, workspace);
        let amount = parameter_value(layer, effect, amount_parameter, workspace) / 100.0;
        let reach = side / 2.0 - 6.0;
        let (dot_x, dot_y) = (side / 2.0 + hue.to_radians().cos() * amount * reach, side / 2.0 - hue.to_radians().sin() * amount * reach);
        let drag = WheelDrag { index, layer, effect };
        let circle = div()
            .id(("grade-wheel", index))
            .relative()
            .size(px(side))
            .rounded_full()
            .cursor_crosshair()
            .child(workspace.ui.wheel_bounds[index].probe())
            .child(img(wheel_image()).size_full())
            // The middle: no tint.
            .child(div().absolute().left(px(side / 2.0 - 1.0)).top(px(side / 2.0 - 1.0)).size(px(2.0)).bg(hsla(color::MUTED)))
            .child(div().absolute().left(px(dot_x - 6.0)).top(px(dot_y - 6.0)).size(px(12.0)).rounded_full().border_2().border_color(hsla(0xffffff)).shadow_md())
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                if event.click_count >= 2 {
                    this.state.set_property_component(layer, PropertyPath::Effect { effect, parameter: amount_parameter }, 0, 0.0);
                } else {
                    this.ui.grade_pressed = true;
                    drag_wheel(this, drag, event.position);
                }
                cx.notify();
            }))
            .on_drag(drag, |_, _, _, cx| cx.new(|_| EmptyDragView))
            // Each wheel hears every wheel drag; it follows only its own.
            .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<WheelDrag>, _, cx| {
                let moved = *event.drag(cx);
                if moved.index == index && moved.effect == effect {
                    drag_wheel(this, moved, event.event.position);
                    cx.notify();
                }
            }))
            .test_support();
        div()
            .w(px(side))
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .child(circle)
            .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(title))
            .child(div().w_full().child(slider_track(layer, effect, brightness_parameter, Track::Light, workspace, cx)))
    }))
}

/// The bounds of a slider's track as drawn last frame, remembered for dragging.
fn slider_bounds(workspace: &Workspace, effect: Id, parameter: usize) -> MeasuredBounds {
    workspace.ui.slider_bounds.borrow_mut().entry((effect, parameter)).or_default().clone()
}

/// The parameter's range, decimals and default, from the effect and the catalog.
fn parameter_range(layer: Id, effect: Id, parameter: usize, workspace: &Workspace) -> Option<(f32, f32, usize, f32)> {
    let found = workspace.state.composition().layer(layer)?.effects.iter().find(|candidate| candidate.id == effect)?;
    let info = found.properties.get(parameter)?.info;
    let default = studio_core::effects::find(&found.name)?.parameters.get(parameter)?.1;
    Some((info.min, info.max, info.decimals, default))
}

/// Sets a parameter from where `position` falls along its slider's track.
fn drag_slider(workspace: &mut Workspace, drag: SliderDrag, position: Point<Pixels>) {
    let Some((min, max, decimals, _)) = parameter_range(drag.layer, drag.effect, drag.parameter, workspace) else { return };
    let track = slider_bounds(workspace, drag.effect, drag.parameter).get();
    if track.width <= 0.0 {
        return;
    }
    let fraction = ((f32::from(position.x) - track.left) / track.width).clamp(0.0, 1.0);
    let scale = 10f32.powi(decimals as i32);
    let value = ((min + fraction * (max - min)) * scale).round() / scale;
    workspace.state.set_property_component(drag.layer, PropertyPath::Effect { effect: drag.effect, parameter: drag.parameter }, 0, value);
}

/// Puts a parameter back to the catalog's default.
fn reset_parameter(workspace: &mut Workspace, layer: Id, effect: Id, parameter: usize) {
    if let Some((_, _, _, default)) = parameter_range(layer, effect, parameter, workspace) {
        workspace.state.set_property_component(layer, PropertyPath::Effect { effect, parameter }, 0, default);
    }
}

/// The colors behind a slider's track.
fn track_fill(track: Track) -> gpui_kit::Background {
    let gradient = |from: u32, to: u32| linear_gradient(90.0, linear_color_stop(gpui_kit::rgb(from), 0.0), linear_color_stop(gpui_kit::rgb(to), 1.0));
    match track {
        Track::Plain => hsla(color::HAIRLINE).into(),
        Track::Temperature => gradient(0x3f7fd9, 0xe3a33b),
        Track::Tint => gradient(0x3fae5a, 0xc04fb8),
        Track::Light => gradient(0x050505, 0xf2f2f2),
        Track::Saturation => gradient(0x7a7a7a, 0xe2453c),
        // Drawn by `track_bar` as several pieces; this is only what shows under them.
        Track::Hue => hsla(color::HAIRLINE).into(),
    }
}

/// The colored bar of a slider's track. A hue track runs through six colors, which takes one
/// two-color gradient for each step between them.
fn track_bar(track: Track) -> gpui_kit::Div {
    let bar = div().absolute().left_0().right_0().top(px(6.0)).h(px(4.0)).rounded_full().overflow_hidden();
    match track {
        Track::Hue => {
            const STOPS: [u32; 7] = [0xff0000, 0xffff00, 0x00ff00, 0x00ffff, 0x0000ff, 0xff00ff, 0xff0000];
            bar.flex().children(STOPS.windows(2).map(|pair| {
                div().flex_1().h_full().bg(linear_gradient(90.0, linear_color_stop(gpui_kit::rgb(pair[0]), 0.0), linear_color_stop(gpui_kit::rgb(pair[1]), 1.0)))
            }))
        }
        _ => bar.bg(track_fill(track)),
    }
}

/// The slider the Effects tab shows under an effect parameter, when one helps: for settings
/// with a modest range (a slider over thousands of pixels or many turns would be too coarse
/// to use). Hue angles get a rainbow track.
pub(crate) fn effect_slider_track(layer: Id, effect: Id, parameter: usize, workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let info = workspace.state.composition().layer(layer)?.effects.iter().find(|e| e.id == effect)?.properties.get(parameter)?.info;
    let span = info.max - info.min;
    let hue = info.unit == "°" && info.min == 0.0 && info.max == 360.0;
    let useful = span > 0.0 && (hue || (span <= 500.0 && info.unit != "°"));
    useful.then(|| slider_track(layer, effect, parameter, if hue { Track::Hue } else { Track::Plain }, workspace, cx))
}

/// A slider's track: click or drag along it to set the value, double-click to put it back. A
/// small mark shows where the default is.
fn slider_track(layer: Id, effect: Id, parameter: usize, track: Track, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let Some((min, max, _, default)) = parameter_range(layer, effect, parameter, workspace) else { return div().into_any_element() };
    let span = (max - min).max(f32::EPSILON);
    let at = |value: f32| ((value - min) / span).clamp(0.0, 1.0);
    let value = parameter_value(layer, effect, parameter, workspace);
    let drag = SliderDrag { layer, effect, parameter };
    let default_mark = (default > min && default < max).then(|| div().absolute().top(px(3.0)).h(px(10.0)).w(px(1.0)).left(relative(at(default))).bg(hsla(color::MUTED)));
    div()
        .id(ElementId::Name(format!("grade-slider:{}:{parameter}", effect.0).into()))
        .relative()
        .w_full()
        .h(px(16.0))
        .cursor_pointer()
        .child(slider_bounds(workspace, effect, parameter).probe())
        .child(track_bar(track))
        .children(default_mark)
        .child(div().absolute().top(px(2.0)).left(relative(at(value))).ml(px(-6.0)).size(px(12.0)).rounded_full().bg(gpui_kit::rgb(0xffffff)).border_1().border_color(gpui_kit::rgb(0x000000)).shadow_md())
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            if event.click_count >= 2 {
                reset_parameter(this, layer, effect, parameter);
            } else {
                this.ui.grade_pressed = true;
                drag_slider(this, drag, event.position);
            }
            cx.notify();
        }))
        .on_drag(drag, |_, _, _, cx| cx.new(|_| EmptyDragView))
        // Each slider hears every slider drag; it follows only its own.
        .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<SliderDrag>, _, cx| {
            let moved = *event.drag(cx);
            if moved.effect == effect && moved.parameter == parameter {
                drag_slider(this, moved, event.event.position);
                cx.notify();
            }
        }))
        .test_support()
        .into_any_element()
}

/// A labeled slider: the parameter's name (brighter once it is changed; double-click it to put
/// the value back), its value (scrub it or type one) and the track.
fn slider(layer: Id, effect: Id, parameter: usize, track: Track, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let Some(name) = workspace.state.composition().layer(layer).and_then(|l| l.effects.iter().find(|e| e.id == effect)).and_then(|e| e.properties.get(parameter)).map(|p| p.name.clone()) else {
        return div().into_any_element();
    };
    let changed = parameter_range(layer, effect, parameter, workspace).is_some_and(|(_, _, _, default)| (parameter_value(layer, effect, parameter, workspace) - default).abs() > 1e-4);
    let label = div()
        .id(ElementId::Name(format!("grade-label:{}:{parameter}", effect.0).into()))
        .flex_1()
        .min_w_0()
        .truncate()
        .text_color(hsla(if changed { color::TEXT } else { color::MUTED }))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            if event.click_count >= 2 {
                reset_parameter(this, layer, effect, parameter);
                cx.notify();
            }
        }))
        .child(name)
        .test_support();
    let field = value_field(FieldTarget::Property { layer, path: PropertyPath::Effect { effect, parameter }, component: 0 }, workspace, cx);
    div()
        .flex()
        .flex_col()
        .py_0p5()
        .child(div().h(px(22.0)).flex().items_center().gap_2().child(label).child(field))
        .child(slider_track(layer, effect, parameter, track, workspace, cx))
        .into_any_element()
}

/// A small heading over a group of sliders, with room for a control on its right.
fn group_heading(title: &'static str, trailing: Option<AnyElement>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .h(px(24.0))
        .mt_1()
        .child(div().flex_1().text_size(px(size::SMALL_TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(hsla(color::MUTED)).child(title))
        .children(trailing)
}

/// Basic Correction, in Lumetri's groups: White Balance (with the picker), Light and Color.
fn basic_correction(layer: Id, effect: Id, workspace: &Workspace, cx: &mut Context<Workspace>) -> gpui_kit::Div {
    let picking = workspace.ui.white_balance_pick == Some((layer, effect));
    let picker = Button::new(ElementId::Name(format!("white-balance-pick:{}", effect.0).into()))
        .xsmall()
        .icon(Icon::new(IconName::Pipette))
        .tooltip("Pick something gray or white in the viewer to fix the color cast")
        .map(|button| if picking { button.primary() } else { button.ghost() })
        .on_click(cx.listener(move |this, _, _, cx| {
            this.ui.white_balance_pick = if this.ui.white_balance_pick == Some((layer, effect)) { None } else { Some((layer, effect)) };
            cx.notify();
        }));
    let tracks = [Track::Temperature, Track::Tint, Track::Light, Track::Light, Track::Light, Track::Light, Track::Light, Track::Light, Track::Saturation, Track::Saturation];
    let sliders = |range: std::ops::Range<usize>, cx: &mut Context<Workspace>| range.map(|parameter| slider(layer, effect, parameter, tracks[parameter], workspace, cx)).collect::<Vec<_>>();
    div()
        .child(group_heading("White Balance", Some(picker.into_any_element())))
        .children(picking.then(|| div().pb_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::ACCENT)).child("Now click something gray or white in the viewer.")))
        .children(sliders(0..2, cx))
        .child(group_heading("Light", None))
        .children(sliders(2..8, cx))
        .child(group_heading("Color", None))
        .children(sliders(8..10, cx))
}

/// A correction's controls, under its card's heading.
fn controls(section: &Section, layer: Id, effect: Id, workspace: &Workspace, cx: &mut Context<Workspace>) -> gpui_kit::Div {
    let count = workspace.state.composition().layer(layer).and_then(|l| l.effects.iter().find(|e| e.id == effect)).map_or(0, |e| e.properties.len());
    let sliders = |track: &dyn Fn(usize) -> Track, cx: &mut Context<Workspace>| (0..count).map(|parameter| slider(layer, effect, parameter, track(parameter), workspace, cx)).collect::<Vec<_>>();
    match section.effect {
        "Basic Correction" => basic_correction(layer, effect, workspace, cx),
        "Apply LUT" => div().child(lut_browser(layer, effect, workspace, cx)).children(sliders(&|_| Track::Plain, cx)),
        "Color Wheels" => div().child(wheels(layer, effect, workspace, cx)),
        // Gamma is the one Levels setting that isn't a black or white point.
        "Levels" => div().children(sliders(&|parameter| if parameter == 2 { Track::Plain } else { Track::Light }, cx)),
        _ => div().children(sliders(&|_| Track::Plain, cx)),
    }
}

/// One correction's card, for the effect `effect` on `layer`: a heading (fold, turn off,
/// reset, remove) and, while open, its controls.
fn section_card(section: &'static Section, layer: Id, effect: Id, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let key = section.key;
    let enabled = workspace.state.composition().layer(layer).and_then(|l| l.effects.iter().find(|e| e.id == effect)).is_some_and(|e| e.enabled);
    // Cards start open; folding one is remembered.
    let open = !workspace.ui.toggled_sections.contains(key);
    let heading = CardHeading {
        id: ElementId::Name(format!("{key}-heading").into()),
        icon: Some(section.icon),
        title: section.title.into(),
        open,
        muted: !enabled,
        trailing: effect_buttons(|part| format!("{key}-{part}"), layer, effect, enabled, cx),
    };
    let body = open.then(|| controls(section, layer, effect, workspace, cx).into_any_element());
    card(heading, cx.listener(move |this, _, _, cx| {
        this.ui.toggle_section(key);
        cx.notify();
    }), body)
    .into_any_element()
}

/// The buttons on an effect card's heading: turn the effect off or on, reset it, remove it.
/// `name` gives each its element id from "on", "reset" or "remove". Used by the Color and
/// Effects tabs alike.
pub(crate) fn effect_buttons(name: impl Fn(&str) -> String, layer: Id, effect: Id, enabled: bool, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    vec![
        icon_button(ElementId::Name(name("on").into()), if enabled { IconName::Eye } else { IconName::EyeOff }, if enabled { "Turn Off" } else { "Turn On" }, false, cx.listener(move |this, _, _, cx| {
            // The heading folds on a click; the buttons in it don't.
            cx.stop_propagation();
            this.state.toggle_effect(layer, effect);
            cx.notify();
        }))
        .into_any_element(),
        icon_button(ElementId::Name(name("reset").into()), IconName::RotateCcw, "Reset", false, cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            super::effect_controls::reset_effect(this, layer, effect);
            cx.notify();
        }))
        .into_any_element(),
        icon_button(ElementId::Name(name("remove").into()), IconName::Trash, "Remove", false, cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.state.remove_effect(layer, effect);
            cx.notify();
        }))
        .into_any_element(),
    ]
}

/// Adds the correction `section` to what `target` grades, opening its card.
fn add_section(workspace: &mut Workspace, target: GradeTarget, section: &Section, window: &mut Window, cx: &mut Context<Workspace>) {
    match workspace.state.add_grade_effect(target, section.effect) {
        Some((_, effect)) => {
            workspace.ui.toggled_sections.remove(section.key);
            // A new LUT opens its browser, since it has no LUT yet.
            if section.effect == "Apply LUT" {
                workspace.ui.lut_browser = Some(effect);
            }
        }
        None => window.push_notification("This clip is locked, or can't take color grading.", cx),
    }
    cx.notify();
}

/// The corrections not on `layer` yet (all of them when there is no layer).
fn addable(layer: Option<Id>, workspace: &Workspace) -> Vec<&'static Section> {
    SECTIONS.iter().filter(|section| layer.is_none_or(|layer| workspace.state.effect_named(layer, section.effect).is_none())).collect()
}

/// The empty tab: a tile for each correction, saying what it is for. Grading a clip with none
/// selected shows the same tiles, dimmed and doing nothing, so both ways of grading look alike.
fn add_tiles(target: GradeTarget, usable: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    let tiles = SECTIONS.iter().map(|section| {
        let tile = div()
            .id(ElementId::Name(format!("grade-add:{}", section.key).into()))
            .flex()
            .flex_col()
            .gap_1()
            .p_2p5()
            .rounded(px(8.0))
            .border_1()
            .border_color(hsla(color::HAIRLINE))
            .bg(hsla(color::RAISED))
            .child(div().flex().items_center().gap_1p5().child(Icon::new(section.icon).size(px(14.0)).text_color(hsla(color::ACCENT))).child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(section.title)))
            .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(section.about));
        let tile = if usable {
            tile.cursor_pointer().hover(|style| style.bg(hsla(color::HOVER)).border_color(hsla(color::ACCENT))).on_click(cx.listener(move |this, _, window, cx| add_section(this, target, section, window, cx)))
        } else {
            tile.opacity(0.45)
        };
        tile.test_support()
    });
    let hint = if usable { "Add a correction to start grading." } else { "Select a clip to add corrections to it." };
    div().flex().flex_col().gap_2().pt_1().child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(hint)).child(div().grid().grid_cols(2).gap_2().children(tiles))
}

/// The Add Correction menu under the cards: each correction not added yet.
fn add_menu(target: GradeTarget, missing: Vec<&'static Section>, cx: &mut Context<Workspace>) -> impl IntoElement {
    let weak = cx.weak_entity();
    Button::new("grade-add").ghost().small().w_full().icon(Icon::new(IconName::Plus)).label("Add Correction").dropdown_caret(true).dropdown_menu(move |menu, _, _| {
        missing.iter().fold(menu, |menu, &section| {
            let weak = weak.clone();
            menu.item(PopupMenuItem::new(section.title).icon(Icon::new(section.icon)).on_click(move |_, window, cx| {
                weak.update(cx, |this, cx| add_section(this, target, section, window, cx)).ok();
            }))
        })
    })
}

/// The switch at the top: grade the selected clip or the whole video, as one segmented control.
fn target_switch(target: GradeTarget, cx: &mut Context<Workspace>) -> impl IntoElement {
    let choice = |id: &'static str, label: &'static str, which: GradeTarget, cx: &mut Context<Workspace>| {
        Button::new(id)
            .small()
            .label(label)
            .flex_1()
            .map(|button| if target == which { button.primary() } else { button.ghost() })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.ui.grade_target = which;
                cx.notify();
            }))
    };
    div()
        .flex()
        .gap_0p5()
        .p_0p5()
        .rounded(px(7.0))
        .bg(hsla(color::INSET))
        .child(choice("grade-target-clip", "This Clip", GradeTarget::Clip, cx))
        .child(choice("grade-target-video", "Whole Video", GradeTarget::Video, cx))
}

/// The line under the switch: what is being graded.
fn target_line(target: GradeTarget, layer_name: Option<&str>) -> impl IntoElement {
    let (icon, text) = match (target, layer_name) {
        (GradeTarget::Clip, Some(name)) => (IconName::Film, name.to_owned()),
        (GradeTarget::Clip, None) => (IconName::Film, "Select one clip in the timeline to grade it.".to_owned()),
        (GradeTarget::Video, Some(name)) => (IconName::Layers, format!("{name} · over everything below it")),
        (GradeTarget::Video, None) => (IconName::Layers, "A Color Grade layer goes on top, over the whole video.".to_owned()),
    };
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .py_1()
        .text_size(px(size::SMALL_TEXT))
        .text_color(hsla(if layer_name.is_some() { color::TEXT } else { color::MUTED }))
        .child(Icon::new(icon).size(px(12.0)).text_color(hsla(color::MUTED)))
        .child(div().min_w_0().truncate().child(text))
}

pub fn body(workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let target = workspace.ui.grade_target;
    let layer = workspace.state.grade_layer(target);
    let layer_name = layer.and_then(|id| workspace.state.composition().layer(id)).map(|layer| layer.name.clone());
    let mut list = card_list("color-grade")
        .child(workspace.ui.grade_bounds.probe())
        .child(div().flex().flex_col().gap_1().child(target_switch(target, cx)).child(target_line(target, layer_name.as_deref())));
    if target == GradeTarget::Clip && layer.is_none() {
        return list.child(add_tiles(target, false, cx)).into_any_element();
    }
    // The corrections there are, in the order their effects apply.
    let mut shown: Vec<(usize, Id, Id, &'static Section)> = Vec::new();
    if let Some(layer) = layer {
        let effects = workspace.state.composition().layer(layer).map(|layer| layer.effects.as_slice()).unwrap_or_default();
        for section in &SECTIONS {
            if let Some(effect) = workspace.state.effect_named(layer, section.effect) {
                let order = effects.iter().position(|candidate| candidate.id == effect).unwrap_or(usize::MAX);
                shown.push((order, layer, effect, section));
            }
        }
    }
    shown.sort_by_key(|&(order, ..)| order);
    if shown.is_empty() {
        return list.child(add_tiles(target, true, cx)).into_any_element();
    }
    for (_, layer, effect, section) in shown {
        list = list.child(section_card(section, layer, effect, workspace, cx));
    }
    let missing = addable(layer, workspace);
    list.when(!missing.is_empty(), |list| list.child(add_menu(target, missing, cx))).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::chrome::Rect;
    use gpui_kit::point;

    #[test]
    fn wheel_positions_map_to_hue_and_amount() {
        let bounds = Rect { left: 0.0, top: 0.0, width: 100.0, height: 100.0 };
        let (hue, amount) = wheel_value(point(px(100.0), px(50.0)), bounds);
        assert!((hue - 0.0).abs() < 0.01 && (amount - 100.0).abs() < 0.01, "right edge is red at full strength");
        let (hue, amount) = wheel_value(point(px(50.0), px(25.0)), bounds);
        assert!((hue - 90.0).abs() < 0.01 && (amount - 50.0).abs() < 0.01, "straight up, halfway out");
        assert_eq!(wheel_value(point(px(50.0), px(50.0)), bounds).1, 0.0, "the middle has no tint");
        assert_eq!(wheel_value(point(px(500.0), px(50.0)), bounds).1, 100.0, "outside holds at the edge");
    }
}
