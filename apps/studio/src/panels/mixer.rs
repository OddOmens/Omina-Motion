//! The Mixer panel, opened from the rail: a level meter for the whole mix while it plays, and
//! under it every clip of the open composition that has sound, each with a mute switch, its
//! volume and its pan. The same settings are in a clip's Audio card; here they are side by
//! side, to balance clips against each other while listening.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::{Context, ElementId, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _, div, prelude::FluentBuilder as _, px, relative};
use studio_core::model::Id;

use super::chrome::panel;
use super::fields::{FieldTarget, value_field};
use crate::state::{AudioSetting, Switch};
use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, Workspace};

/// The quietest level the meter shows, in decibels below full scale.
const METER_FLOOR_DB: f32 = -60.0;
/// From here up the meter turns amber, then red at full scale: little room is left.
const METER_WARN_DB: f32 = -6.0;
/// How much of its height a meter bar keeps each screen frame as the sound falls away, so the
/// bars drop smoothly instead of flickering with every block of sound.
const METER_FALL: f32 = 0.86;

/// The master meter's state: each side's level as shown now (1.0 is full scale), and whether
/// the mix has gone over full scale since the warning was last cleared.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Meter {
    pub levels: [f32; 2],
    pub clipped: bool,
}

impl Meter {
    /// Takes in the loudest levels mixed since the last screen frame: a bar jumps up to a
    /// louder level at once and falls back gradually.
    pub fn take(&mut self, peaks: [f32; 2]) {
        for (level, peak) in self.levels.iter_mut().zip(peaks) {
            let peak = if peak.is_finite() { peak.max(0.0) } else { 0.0 };
            *level = peak.max(*level * METER_FALL);
            if *level < 1e-4 {
                *level = 0.0;
            }
            self.clipped |= peak > 1.0;
        }
    }

    /// Drops the bars to silence, when playback stops. The clipping warning stays until cleared.
    pub fn rest(&mut self) {
        self.levels = [0.0; 2];
    }
}

/// A level as decibels below full scale; silence is the meter's floor.
pub fn decibels(level: f32) -> f32 {
    if level <= 0.0 { METER_FLOOR_DB } else { (20.0 * level.log10()).max(METER_FLOOR_DB) }
}

/// How far along the meter a level reaches, from 0 (the floor) to 1 (full scale).
fn meter_fraction(level: f32) -> f32 {
    ((decibels(level) - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0)
}

/// One side's bar: its letter, the bar (green, amber near the top, red at full scale) and the
/// level in decibels.
fn meter_bar(side: &'static str, level: f32) -> gpui_kit::Div {
    let db = decibels(level);
    let bar_color = if level > 1.0 {
        gpui_kit::rgb(0xef4444)
    } else if db >= METER_WARN_DB {
        gpui_kit::rgb(0xf59e0b)
    } else {
        gpui_kit::rgb(0x4ade80)
    };
    let reading = if level <= 0.0 { "-∞".to_owned() } else { format!("{db:.0}") };
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().min_w(px(10.0)).flex_none().text_color(hsla(color::MUTED)).child(side))
        .child(div().flex_1().h(px(8.0)).rounded(px(2.0)).bg(hsla(color::INSET)).overflow_hidden().child(div().h_full().w(relative(meter_fraction(level))).bg(bar_color)))
        // Wide enough for the longest reading on one line, so the rows never change height as
        // the level moves.
        .child(div().w(px(50.0)).flex_none().whitespace_nowrap().text_right().text_size(px(size::SMALL_TEXT)).font_family(crate::theme::MONO_FONT).text_color(hsla(color::MUTED)).child(format!("{reading} dB")))
}

/// The master meter: both sides' bars, and a warning that stays lit after the mix has clipped
/// until it is clicked.
fn master(workspace: &Workspace, cx: &mut Context<Workspace>) -> gpui_kit::Div {
    let meter = workspace.ui.meter;
    let warning = div()
        .id("mixer-clip")
        .px_1p5()
        .rounded(px(3.0))
        .text_size(px(size::SMALL_TEXT))
        .when(meter.clipped, |warning| warning.bg(gpui_kit::rgb(0xef4444)).text_color(gpui_kit::white()).cursor_pointer())
        .when(!meter.clipped, |warning| warning.text_color(hsla(color::MUTED)))
        .child(if meter.clipped { "Clipped: click to clear" } else { "No clipping" })
        .on_click(cx.listener(|this, _, _, cx| {
            this.ui.meter.clipped = false;
            cx.notify();
        }))
        .test_support();
    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .p_2p5()
        .border_b_1()
        .border_color(hsla(color::HAIRLINE))
        .child(div().flex().items_center().child(div().flex_1().child("Master")).child(warning))
        .child(meter_bar("L", meter.levels[0]))
        .child(meter_bar("R", meter.levels[1]))
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(if workspace.state.is_playing() { "The level of everything playing." } else { "Play to see the level of the mix." }))
}

/// One clip's strip: whether it is heard, its name (a click selects the clip), its volume and
/// its pan.
fn strip(workspace: &Workspace, id: Id, name: &str, heard: bool, selected: bool, cx: &mut Context<Workspace>) -> gpui_kit::AnyElement {
    let field = |setting: AudioSetting, cx: &mut Context<Workspace>| value_field(FieldTarget::Audio { layer: id, setting }, workspace, cx);
    let mute = Checkbox::new(ElementId::NamedInteger("mixer-heard".into(), id.0)).checked(heard).on_click(cx.listener(move |this, _, _, cx| {
        this.state.toggle_switch(id, Switch::Audio);
        cx.notify();
    }));
    div()
        .id(ElementId::NamedInteger("mixer-strip".into(), id.0))
        .flex()
        .items_center()
        .gap_2()
        .px_2p5()
        .h(px(30.0))
        .when(selected, |strip| strip.bg(hsla(color::ROW_SELECTED)))
        .hover(|style| style.bg(hsla(color::HOVER)))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.state.select_layer(id, false);
            cx.notify();
        }))
        .child(mute)
        .child(div().flex_1().min_w_0().truncate().when(!heard, |name| name.text_color(hsla(color::MUTED))).child(name.to_owned()))
        .child(field(AudioSetting::Volume, cx))
        .child(field(AudioSetting::Pan, cx))
        .test_support()
        .into_any_element()
}

/// The voiceover row: a button that starts recording from the microphone at the playhead (and
/// stops it), and while recording, the microphone's level.
fn voiceover(workspace: &Workspace, cx: &mut Context<Workspace>) -> gpui_kit::Div {
    let recording = workspace.ui.voiceover.as_ref();
    let button = Button::new("mixer-voiceover")
        .small()
        .icon(Icon::new(IconName::Mic))
        .label(if recording.is_some() { "Stop Recording" } else { "Record Voiceover" })
        .tooltip("Records the microphone from the playhead while the video plays, then adds the recording to the timeline. Use headphones.")
        .when(recording.is_some(), |button| button.danger())
        .when(recording.is_none(), |button| button.outline())
        .on_click(cx.listener(|this, _, window, cx| this.toggle_voiceover(window, cx)));
    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .p_2p5()
        .border_b_1()
        .border_color(hsla(color::HAIRLINE))
        .child(div().flex().child(button))
        .children(recording.map(|_| meter_bar("Mic", workspace.ui.mic_meter.levels[0])))
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let composition = workspace.state.composition();
    let sounding: Vec<(Id, String, bool)> = composition.layers.iter().filter(|layer| workspace.state.has_sound(layer.id)).map(|layer| (layer.id, layer.name.clone(), layer.switches.audio)).collect();
    let strips = if sounding.is_empty() {
        div().p_2p5().text_color(hsla(color::MUTED)).child("No clip in this composition has sound.").into_any_element()
    } else {
        div()
            .id("mixer-strips")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            // What the two numbers on each strip are.
            .child(div().flex().gap_2().px_2p5().pt_2().pb_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(div().flex_1().child("Clip")).child(div().min_w(px(52.0)).child("Volume")).child(div().min_w(px(52.0)).child("Pan")))
            .children(sounding.iter().map(|(id, name, heard)| strip(workspace, *id, name, *heard, workspace.state.selected_layers.contains(id), cx)))
            .into_any_element()
    };
    panel(PanelId::Mixer, &[], None, div().size_full().flex().flex_col().child(master(workspace, cx)).child(voiceover(workspace, cx)).child(strips), workspace, cx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meter_jumps_up_falls_back_and_remembers_clipping() {
        let mut meter = Meter::default();
        meter.take([0.5, 0.25]);
        assert_eq!(meter.levels, [0.5, 0.25]);
        // Silence next: the bars fall, they don't vanish.
        meter.take([0.0, 0.0]);
        assert!(meter.levels[0] < 0.5 && meter.levels[0] > 0.3);
        assert!(!meter.clipped);
        meter.take([1.2, 0.0]);
        assert!(meter.clipped);
        meter.rest();
        assert_eq!(meter.levels, [0.0, 0.0]);
        assert!(meter.clipped, "the warning stays until it is cleared");
    }

    #[test]
    fn levels_read_in_decibels() {
        assert_eq!(decibels(1.0), 0.0);
        assert!((decibels(0.5) + 6.02).abs() < 0.01);
        assert_eq!(decibels(0.0), METER_FLOOR_DB);
        assert_eq!(meter_fraction(1.0), 1.0);
        assert_eq!(meter_fraction(0.0), 0.0);
    }
}
