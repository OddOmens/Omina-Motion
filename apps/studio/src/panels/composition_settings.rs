//! The Composition Settings dialog, used both for a new composition (Cmd+N, or
//! the Project panel's New Composition button) and for changing the open one
//! (Cmd+K). Laid out like After Effects': name, a preset menu, frame size with an
//! optional locked aspect ratio, frame rate, duration as timecode, the background
//! color, and the motion blur shutter (After Effects keeps it under Advanced). Any frame
//! size up to 30,000 px works.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::Sizable as _;
use gpui_kit::{AppContext as _, Context, Entity, IntoElement, ParentElement, Styled, Window, div, px, rgba};
use studio_core::edit::CompositionSettings;
use studio_core::model::Rgba;
use studio_core::motion_blur::MotionBlur;
use studio_core::time::{FrameRate, format_timecode, parse_timecode};

use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// Common formats: name, width, height, frames per second.
const PRESETS: &[(&str, u32, u32, &str)] = &[
    ("HD 1080p", 1920, 1080, "30"),
    ("HD 1080p 24", 1920, 1080, "24"),
    ("HD 720p", 1280, 720, "30"),
    ("4K UHD", 3840, 2160, "30"),
    ("Social Vertical 9:16", 1080, 1920, "30"),
    ("Social Square 1:1", 1080, 1080, "30"),
    ("Social Portrait 4:5", 1080, 1350, "30"),
];

/// Frame rates offered beside the frame rate field.
const FRAME_RATES: &[&str] = &["23.976", "24", "25", "29.97", "30", "50", "59.94", "60"];

/// Background colors offered, with their names.
const BACKGROUNDS: &[(&str, u32)] = &[
    ("Black", 0x000000),
    ("Charcoal", 0x1b1b1f),
    ("Gray", 0x808080),
    ("White", 0xffffff),
    ("Dark Red", 0x2b0d0d),
    ("Midnight Blue", 0x0d1b2a),
    ("Forest Green", 0x0f2a1d),
];

/// Width of the label column, so every field lines up as in After Effects.
const LABEL_WIDTH: f32 = 150.0;

/// Parses a typed frame rate. Broadcast rates map to their exact fractions (29.97 → 30000/1001).
pub fn parse_frame_rate(text: &str) -> Option<FrameRate> {
    let fps: f64 = text.trim().trim_end_matches("fps").trim().parse().ok()?;
    if !(fps.is_finite() && fps > 0.0 && fps <= 999.0) {
        return None;
    }
    for (nominal, rate) in [(23.976, 24000), (29.97, 30000), (47.952, 48000), (59.94, 60000), (119.88, 120000)] {
        if (fps - nominal).abs() < 0.005 {
            return Some(FrameRate::new(rate, 1001));
        }
    }
    if fps.fract() == 0.0 {
        Some(FrameRate::new(fps as u32, 1))
    } else {
        Some(FrameRate::new((fps * 1000.0).round() as u32, 1000))
    }
}

fn format_frame_rate(rate: FrameRate) -> String {
    let fps = rate.fps();
    if fps.fract() == 0.0 { format!("{fps}") } else { format!("{fps:.3}").trim_end_matches('0').to_owned() }
}

/// A frame size's aspect ratio, as After Effects writes it: "16:9 (1.78)".
fn aspect_label(width: u32, height: u32) -> String {
    fn gcd(a: u32, b: u32) -> u32 {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    if width == 0 || height == 0 {
        return "—".into();
    }
    let divisor = gcd(width, height);
    format!("{}:{} ({:.2})", width / divisor, height / divisor, width as f64 / height as f64)
}

fn background_name(color: Rgba) -> String {
    if color.a <= 0.0 {
        return "Transparent".to_owned();
    }
    BACKGROUNDS
        .iter()
        .find(|(_, hex)| Rgba::hex(*hex).to_rgba_u32() == color.to_rgba_u32())
        .map_or_else(|| "Custom".to_owned(), |(name, _)| (*name).to_owned())
}

/// The dialog's text inputs, and the choices it keeps outside them.
#[derive(Clone)]
struct Fields {
    name: Entity<InputState>,
    width: Entity<InputState>,
    height: Entity<InputState>,
    frame_rate: Entity<InputState>,
    duration: Entity<InputState>,
    /// Motion blur's shutter angle (degrees) and samples, for layers with the switch on.
    shutter_angle: Entity<InputState>,
    samples: Entity<InputState>,
    background: Rc<Cell<Rgba>>,
    /// While set, width and height keep this ratio (width / height).
    locked_ratio: Rc<Cell<Option<f64>>>,
}

impl Fields {
    fn text(input: &Entity<InputState>, cx: &gpui_kit::App) -> String {
        input.read(cx).value().trim().to_owned()
    }

    /// Reads and checks the inputs, returning a message for the first problem found.
    fn read(&self, cx: &gpui_kit::App) -> Result<CompositionSettings, String> {
        let width = Self::text(&self.width, cx).parse::<u32>().map_err(|_| "Width must be a whole number of pixels.")?;
        let height = Self::text(&self.height, cx).parse::<u32>().map_err(|_| "Height must be a whole number of pixels.")?;
        let frame_rate = parse_frame_rate(&Self::text(&self.frame_rate, cx)).ok_or("Enter a frame rate such as 24, 29.97 or 60.")?;
        let duration_frames = parse_timecode(&Self::text(&self.duration, cx), frame_rate)
            .filter(|frames| *frames > 0)
            .ok_or("Enter a duration as timecode (0:00:10:00) or seconds (10).")?;
        let settings = CompositionSettings { name: Self::text(&self.name, cx), width, height, frame_rate, duration_frames, background: self.background.get() };
        settings.validate()?;
        Ok(settings)
    }

    /// Reads and checks the motion blur fields.
    fn read_motion_blur(&self, cx: &gpui_kit::App) -> Result<MotionBlur, String> {
        let shutter_angle = Self::text(&self.shutter_angle, cx).trim_end_matches('°').trim().parse::<f32>().map_err(|_| "Enter a shutter angle in degrees, such as 180.")?;
        let samples = Self::text(&self.samples, cx).parse::<u32>().map_err(|_| "Motion blur samples must be a whole number.")?;
        let settings = MotionBlur { shutter_angle, samples };
        settings.validate()?;
        Ok(settings)
    }

    /// The width and height as typed, if both are numbers.
    fn frame_size(&self, cx: &gpui_kit::App) -> Option<(u32, u32)> {
        Some((Self::text(&self.width, cx).parse().ok()?, Self::text(&self.height, cx).parse().ok()?))
    }
}

/// A row of the dialog: a right-aligned label, then its controls.
fn row(label: &'static str, controls: impl IntoElement) -> gpui_kit::Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .min_h(px(34.0))
        .child(div().w(px(LABEL_WIDTH)).flex_none().flex().justify_end().text_color(hsla(color::TEXT)).child(format!("{label}:")))
        .child(div().flex_1().min_w_0().flex().items_center().gap_3().child(controls))
}

fn rule() -> gpui_kit::Div {
    div().h(px(1.0)).my_2().bg(hsla(color::HAIRLINE))
}

fn muted(text: impl Into<gpui_kit::SharedString>) -> gpui_kit::Div {
    div().text_color(hsla(color::MUTED)).child(text.into())
}

/// Keeps width and height in their locked ratio: a change to one sets the other.
fn link_frame_size(workspace: &mut Workspace, fields: &Fields, window: &mut Window, cx: &mut Context<Workspace>) {
    let syncing = Rc::new(Cell::new(false));
    let mut link = |from: &Entity<InputState>, to: &Entity<InputState>, from_is_width: bool| {
        let (to, ratio, syncing) = (to.clone(), fields.locked_ratio.clone(), syncing.clone());
        cx.subscribe_in(from, window, move |_, input, event: &InputEvent, window, cx| {
            if !matches!(event, InputEvent::Change) || syncing.get() {
                return;
            }
            let (Some(ratio), Ok(value)) = (ratio.get(), input.read(cx).value().trim().parse::<f64>()) else {
                window.refresh();
                return;
            };
            let other = if from_is_width { value / ratio } else { value * ratio };
            syncing.set(true);
            to.update(cx, |state, cx| state.set_value(format!("{}", other.round().max(1.0) as u32), window, cx));
            syncing.set(false);
            window.refresh();
        })
    };
    let subscriptions = vec![link(&fields.width, &fields.height, true), link(&fields.height, &fields.width, false)];
    // The other inputs only refresh the live labels (aspect ratio, "is 0:00:10:00").
    let refresh = |input: &Entity<InputState>, cx: &mut Context<Workspace>| cx.subscribe_in(input, window, |_, _, _: &InputEvent, window, _| window.refresh());
    let mut all = subscriptions;
    all.push(refresh(&fields.frame_rate, cx));
    all.push(refresh(&fields.duration, cx));
    workspace.ui.dialog_subscriptions = all;
}

/// Opens the dialog. With `create`, OK makes a new composition; otherwise it changes the open one.
pub fn open(workspace: &mut Workspace, create: bool, window: &mut Window, cx: &mut Context<Workspace>) {
    let base = workspace.state.composition_settings();
    // New compositions start transparent, as in After Effects: set a color here, or add a
    // solid or shape layer, to fill them.
    let base = if create {
        // The size, rate and length new compositions start with are set in Settings.
        let defaults = crate::state::new_composition_settings(workspace.ui.settings.new_composition);
        CompositionSettings { name: format!("Comp {}", workspace.state.project.compositions.len() + 1), ..defaults }
    } else {
        base
    };
    // A new composition starts with the default shutter; an open one shows its own.
    let shutter = if create { MotionBlur::default() } else { workspace.state.composition().motion_blur };
    let make = |value: String, window: &mut Window, cx: &mut Context<Workspace>| cx.new(|cx| InputState::new(window, cx).default_value(value));
    let fields = Fields {
        name: make(base.name.clone(), window, cx),
        width: make(base.width.to_string(), window, cx),
        height: make(base.height.to_string(), window, cx),
        frame_rate: make(format_frame_rate(base.frame_rate), window, cx),
        duration: make(format_timecode(base.duration_frames, base.frame_rate), window, cx),
        shutter_angle: make(format!("{}", shutter.shutter_angle), window, cx),
        samples: make(shutter.samples.to_string(), window, cx),
        background: Rc::new(Cell::new(base.background)),
        locked_ratio: Rc::new(Cell::new(None)),
    };
    link_frame_size(workspace, &fields, window, cx);
    let target = cx.weak_entity();

    window.open_dialog(cx, move |dialog, _, cx| {
        let size_now = fields.frame_size(cx);
        let rate_now = parse_frame_rate(&Fields::text(&fields.frame_rate, cx));
        let preset_label = size_now
            .and_then(|(w, h)| PRESETS.iter().find(|p| (p.1, p.2) == (w, h) && rate_now == parse_frame_rate(p.3)))
            .map_or_else(|| "Custom".to_owned(), |p| format!("{} · {}×{} · {} fps", p.0, p.1, p.2, p.3));

        let preset_menu = {
            let fields = fields.clone();
            Button::new("comp-preset").outline().small().label(preset_label).dropdown_caret(true).w_full().dropdown_menu(move |menu, _, _| {
                PRESETS.iter().fold(menu, |menu, &(name, width, height, fps)| {
                    let fields = fields.clone();
                    menu.item(PopupMenuItem::new(format!("{name} · {width}×{height} · {fps} fps")).on_click(move |_, window, cx| {
                        // Presets set their own shape, so they unlock the ratio.
                        fields.locked_ratio.set(None);
                        fields.width.update(cx, |input, cx| input.set_value(width.to_string(), window, cx));
                        fields.height.update(cx, |input, cx| input.set_value(height.to_string(), window, cx));
                        fields.frame_rate.update(cx, |input, cx| input.set_value(fps.to_owned(), window, cx));
                        window.refresh();
                    }))
                })
            })
        };

        let aspect = size_now.map_or_else(|| "—".to_owned(), |(w, h)| aspect_label(w, h));
        let locked = fields.locked_ratio.get().is_some();
        let lock = {
            let fields = fields.clone();
            Checkbox::new("comp-lock-aspect").checked(locked).label(format!("Lock Aspect Ratio to {aspect}")).on_click(move |checked: &bool, window, cx| {
                let ratio = fields.frame_size(cx).filter(|(_, h)| *h > 0).map(|(w, h)| f64::from(w) / f64::from(h));
                fields.locked_ratio.set(if *checked { ratio } else { None });
                window.refresh();
            })
        };
        let px_suffix = || div().text_color(hsla(color::MUTED)).child("px");
        let size_block = div()
            .flex()
            .items_center()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(row("Width", div().flex().items_center().gap_2().child(div().w(px(90.0)).child(Input::new(&fields.width).small())).child(px_suffix())))
                    .child(row("Height", div().flex().items_center().gap_2().child(div().w(px(90.0)).child(Input::new(&fields.height).small())).child(px_suffix()))),
            )
            .child(lock);

        let rate_menu = {
            let fields = fields.clone();
            Button::new("comp-rate-menu").outline().small().dropdown_caret(true).dropdown_menu(move |menu, _, _| {
                FRAME_RATES.iter().fold(menu, |menu, &fps| {
                    let fields = fields.clone();
                    menu.item(PopupMenuItem::new(fps).on_click(move |_, window, cx| {
                        fields.frame_rate.update(cx, |input, cx| input.set_value(fps.to_owned(), window, cx));
                        window.refresh();
                    }))
                })
            })
        };

        let duration_note = rate_now
            .and_then(|rate| parse_timecode(&Fields::text(&fields.duration, cx), rate).map(|frames| format!("is {}  Base {}", format_timecode(frames, rate), rate.timecode_base())))
            .unwrap_or_else(|| "Enter timecode or seconds".to_owned());
        let size_note = size_now.map_or_else(String::new, |(w, h)| format!("{w} × {h}, {:.1} MB per 16-bit frame", f64::from(w) * f64::from(h) * 8.0 / 1_000_000.0));

        let background = fields.background.get();
        let background_menu = {
            let fields = fields.clone();
            Button::new("comp-background").outline().small().label(background_name(background)).dropdown_caret(true).dropdown_menu(move |menu, _, _| {
                let transparent = {
                    let fields = fields.clone();
                    PopupMenuItem::new("Transparent").checked(fields.background.get().a <= 0.0).on_click(move |_, window, _| {
                        fields.background.set(Rgba::TRANSPARENT);
                        window.refresh();
                    })
                };
                BACKGROUNDS.iter().fold(menu.item(transparent), |menu, &(name, hex)| {
                    let fields = fields.clone();
                    let checked = fields.background.get().to_rgba_u32() == Rgba::hex(hex).to_rgba_u32();
                    menu.item(PopupMenuItem::new(name).checked(checked).on_click(move |_, window, _| {
                        fields.background.set(Rgba::hex(hex));
                        window.refresh();
                    }))
                })
            })
        };

        let (ok_fields, ok_target) = (fields.clone(), target.clone());
        dialog
            .title(if create { "New Composition" } else { "Composition Settings" })
            .w(px(620.0))
            .centered()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .text_size(px(size::TEXT + 1.0))
                    .child(row("Composition Name", div().flex_1().child(Input::new(&fields.name))))
                    .child(rule())
                    .child(row("Preset", div().flex_1().child(preset_menu)))
                    .child(size_block)
                    .child(row("Frame Aspect Ratio", muted(aspect.clone())))
                    .child(row("Frame Rate", div().flex().items_center().gap_2().child(div().w(px(90.0)).child(Input::new(&fields.frame_rate).small())).child(rate_menu).child(muted("frames per second"))))
                    .child(rule())
                    .child(row("Duration", div().flex().items_center().gap_3().child(div().w(px(130.0)).child(Input::new(&fields.duration).small())).child(muted(duration_note))))
                    .child(row("Frame Size", muted(size_note)))
                    .child(rule())
                    .child(row(
                        "Background Color",
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(div().w(px(44.0)).h(px(24.0)).rounded(px(4.0)).border_1().border_color(hsla(color::HAIRLINE)).bg(rgba(background.to_rgba_u32())))
                            .child(background_menu),
                    ))
                    .child(rule())
                    .child(row(
                        "Motion Blur",
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().w(px(70.0)).child(Input::new(&fields.shutter_angle).small()))
                            .child(muted("° shutter"))
                            .child(div().w(px(60.0)).child(Input::new(&fields.samples).small()))
                            .child(muted("samples")),
                    )),
            )
            // A plain dialog draws no buttons of its own; Cancel closes it and OK runs `on_ok`.
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.outline().label("Cancel")))
                    .child(DialogAction::new().child(Button::new("comp-settings-ok").primary().label("OK"))),
            )
            .on_ok(move |_, window, cx| match ok_fields.read(cx).and_then(|settings| Ok((settings, ok_fields.read_motion_blur(cx)?))) {
                Ok((settings, shutter)) => {
                    let result = ok_target
                        .update(cx, |this, cx| {
                            let result = if create { this.state.create_composition(&settings) } else { this.state.update_composition(&settings) };
                            // Either way the composition just set up is the open one.
                            let result = result.and_then(|()| this.state.set_motion_blur(shutter));
                            this.ui.dialog_subscriptions.clear();
                            cx.notify();
                            result
                        })
                        .unwrap_or_else(|_| Err("The editor window closed.".into()));
                    match result {
                        Ok(()) => true,
                        Err(message) => {
                            window.push_notification(message.as_str(), cx);
                            false
                        }
                    }
                }
                Err(message) => {
                    window.push_notification(message.as_str(), cx);
                    false
                }
            })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_rates_parse_exactly() {
        assert_eq!(parse_frame_rate("24"), Some(FrameRate::new(24, 1)));
        assert_eq!(parse_frame_rate("29.97"), Some(FrameRate::new(30000, 1001)));
        assert_eq!(parse_frame_rate("23.976 fps"), Some(FrameRate::new(24000, 1001)));
        assert_eq!(parse_frame_rate("12.5"), Some(FrameRate::new(12500, 1000)));
        assert_eq!(parse_frame_rate("0"), None);
        assert_eq!(parse_frame_rate("fast"), None);
    }

    #[test]
    fn aspect_ratios_read_like_after_effects() {
        assert_eq!(aspect_label(1920, 1080), "16:9 (1.78)");
        assert_eq!(aspect_label(1080, 1350), "4:5 (0.80)");
        assert_eq!(aspect_label(0, 1080), "—");
    }

    #[test]
    fn frame_rates_display_compactly() {
        assert_eq!(format_frame_rate(FrameRate::new(30, 1)), "30");
        assert_eq!(format_frame_rate(FrameRate::new(30000, 1001)), "29.97");
    }
}
