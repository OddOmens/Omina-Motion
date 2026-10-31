//! The Composition Settings dialog, used both for a new composition (Cmd+N, or
//! the Project panel's New Composition button) and for changing the open one
//! (Cmd+K). Any frame size up to 30,000 px works; presets fill in common sizes.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::Sizable as _;
use gpui_kit::{AppContext as _, Context, Entity, IntoElement, ParentElement, Styled, Window, div, px};
use studio_core::edit::CompositionSettings;
use studio_core::time::FrameRate;

use crate::theme::{color, hsla};
use crate::workspace::Workspace;

/// Common frame sizes: label, width, height.
const PRESETS: &[(&str, u32, u32)] = &[
    ("HD 1080p", 1920, 1080),
    ("4K UHD", 3840, 2160),
    ("Vertical 9:16", 1080, 1920),
    ("Square 1:1", 1080, 1080),
    ("Portrait 4:5", 1080, 1350),
];

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

/// The dialog's text inputs.
#[derive(Clone)]
struct Fields {
    name: Entity<InputState>,
    width: Entity<InputState>,
    height: Entity<InputState>,
    frame_rate: Entity<InputState>,
    duration_seconds: Entity<InputState>,
}

impl Fields {
    /// Reads and checks the inputs, returning a message for the first problem found.
    fn read(&self, base: &CompositionSettings, cx: &gpui_kit::App) -> Result<CompositionSettings, String> {
        let text = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
        let width = text(&self.width).parse::<u32>().map_err(|_| "Width must be a whole number of pixels.")?;
        let height = text(&self.height).parse::<u32>().map_err(|_| "Height must be a whole number of pixels.")?;
        let frame_rate = parse_frame_rate(&text(&self.frame_rate)).ok_or("Enter a frame rate such as 24, 29.97 or 60.")?;
        let seconds = text(&self.duration_seconds).parse::<f64>().ok().filter(|s| s.is_finite() && *s > 0.0).ok_or("Duration must be a positive number of seconds.")?;
        let settings = CompositionSettings {
            name: text(&self.name),
            width,
            height,
            frame_rate,
            duration_frames: frame_rate.seconds_to_frames(seconds).max(1),
            background: base.background,
        };
        settings.validate()?;
        Ok(settings)
    }
}

fn labeled(label: &'static str, input: &Entity<InputState>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(div().w(px(110.0)).text_color(hsla(color::MUTED)).child(label))
        .child(div().flex_1().child(Input::new(input).small()))
}

/// Opens the dialog. With `create`, OK makes a new composition; otherwise it changes the open one.
pub fn open(workspace: &mut Workspace, create: bool, window: &mut Window, cx: &mut Context<Workspace>) {
    let base = workspace.state.composition_settings();
    let base = if create {
        CompositionSettings { name: format!("Comp {}", workspace.state.project.compositions.len() + 1), ..base }
    } else {
        base
    };
    let seconds = base.duration_frames as f64 / base.frame_rate.fps();
    let make = |value: String, window: &mut Window, cx: &mut Context<Workspace>| cx.new(|cx| InputState::new(window, cx).default_value(value));
    let fields = Fields {
        name: make(base.name.clone(), window, cx),
        width: make(base.width.to_string(), window, cx),
        height: make(base.height.to_string(), window, cx),
        frame_rate: make(format_frame_rate(base.frame_rate), window, cx),
        duration_seconds: make(format!("{seconds:.2}").trim_end_matches('0').trim_end_matches('.').to_owned(), window, cx),
    };
    let target = cx.weak_entity();

    window.open_dialog(cx, move |dialog, _, _| {
        let presets = PRESETS.iter().enumerate().map(|(index, &(label, width, height))| {
            let fields = fields.clone();
            Button::new(("comp-preset", index)).outline().xsmall().label(label).on_click(move |_, window, cx| {
                fields.width.update(cx, |input, cx| input.set_value(width.to_string(), window, cx));
                fields.height.update(cx, |input, cx| input.set_value(height.to_string(), window, cx));
            })
        });
        let (ok_fields, ok_target, ok_base) = (fields.clone(), target.clone(), base.clone());
        dialog
            .title(if create { "New Composition" } else { "Composition Settings" })
            .w(px(460.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(labeled("Name", &fields.name))
                    .child(div().flex().flex_wrap().gap_1().children(presets))
                    .child(labeled("Width (px)", &fields.width))
                    .child(labeled("Height (px)", &fields.height))
                    .child(labeled("Frame rate (fps)", &fields.frame_rate))
                    .child(labeled("Duration (seconds)", &fields.duration_seconds)),
            )
            // A plain dialog draws no buttons of its own; Cancel closes it and OK runs `on_ok`.
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.outline().label("Cancel")))
                    .child(DialogAction::new().child(Button::new("comp-settings-ok").primary().label("OK"))),
            )
            .on_ok(move |_, window, cx| match ok_fields.read(&ok_base, cx) {
                Ok(settings) => {
                    let result = ok_target
                        .update(cx, |this, cx| {
                            let result = if create { this.state.create_composition(&settings) } else { this.state.update_composition(&settings) };
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
    fn frame_rates_display_compactly() {
        assert_eq!(format_frame_rate(FrameRate::new(30, 1)), "30");
        assert_eq!(format_frame_rate(FrameRate::new(30000, 1001)), "29.97");
    }
}
