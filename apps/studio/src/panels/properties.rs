//! The Properties panel (first tab), Effect Controls (second tab), Color (third tab, in
//! `color_grade`) and Motion (fourth tab, in `motion`). Properties shows the selected layer's transform and, for text layers,
//! character and paragraph styling. Every value scrubs or can be typed into.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, WindowExt as _};
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, TestSupportExt as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};
use studio_core::captions::{CaptionAnimation, CaptionStyle, CaptionWord};
use studio_core::model::{BlendMode, Id, Layer, LayerContent, LayerMask, MaskMode, TextPath, Paint, PaintKind, Rgba, ShapeKind, ShapeStyle, TextAlign, TextStyle, TrackMatte};
use studio_core::property::{PropertyPath, TransformProperty};
use studio_core::subtitles::SubtitleFormat;

use super::chrome::{CardHeading, card, card_list, divider, heading_button, hint, icon_button, info_button, list_row, panel, section};
use super::color_picker;
use super::fields::{FieldTarget, property_row, value_field};
use crate::features::Feature;
use crate::state::{AudioSetting, ColorTarget, PaintSlot, ShapeSetting, Switch, TextSetting};
use crate::theme::{color, hsla, size};
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

/// The offered font families that are installed. Asking the OS is slow, so this runs once at startup.
pub fn installed_font_families(cx: &App) -> Vec<&'static str> {
    let installed = cx.text_system().all_font_names();
    FONT_FAMILIES.iter().copied().filter(|family| installed.iter().any(|name| name == family)).collect()
}

fn weight_name(weight: u16) -> &'static str {
    FONT_WEIGHTS.iter().min_by_key(|(candidate, _)| candidate.abs_diff(weight)).map_or("Regular", |(_, name)| name)
}

/// A dropdown whose choices run `choose` on the workspace.
pub(super) fn choice_menu<T: Clone + 'static>(
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

/// A labeled control in a section: its name on the left, the control filling the rest.
pub(super) fn labeled_row(label: &'static str, control: impl IntoElement) -> Div {
    div()
        .h(px(30.0))
        .flex()
        .items_center()
        .gap_2()
        .child(div().w(px(96.0)).flex_none().text_color(hsla(color::MUTED)).child(label))
        .child(div().flex_1().min_w_0().child(control))
}

/// The Mask row: which other layer shows (or hides) this one where it draws, and which of the
/// two it does. The mask layer itself is not drawn.
fn mask_row(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let target = cx.weak_entity();
    let composition = workspace.state.composition();
    let mode = layer.mask.map_or(MaskMode::Show, |mask| mask.mode);
    let mut choices: Vec<(String, bool, Option<Id>)> = vec![("None".to_owned(), layer.mask.is_none(), None)];
    choices.extend(
        composition
            .layers
            .iter()
            .filter(|candidate| candidate.id != id && candidate.content.has_picture())
            .map(|candidate| (candidate.name.clone(), layer.mask.is_some_and(|mask| mask.layer == candidate.id), Some(candidate.id))),
    );
    let current = layer.mask.and_then(|mask| composition.layer(mask.layer)).map_or("None".to_owned(), |mask| mask.name.clone());
    let pick = choice_menu("layer-mask", current, choices, move |this, chosen: Option<Id>| this.state.set_layer_mask(id, chosen.map(|layer| LayerMask { layer, mode })), target.clone());
    let mut row = div().child(labeled_row("Mask", pick));
    if let Some(mask) = layer.mask {
        let modes = MaskMode::ALL.iter().map(|&choice| (choice.name().to_owned(), choice == mask.mode, choice)).collect();
        row = row.child(labeled_row("", choice_menu("layer-mask-mode", mask.mode.name().to_owned(), modes, move |this, mode| this.state.set_layer_mask(id, Some(LayerMask { mode, ..mask })), target)));
    }
    row
}

/// How the layer combines with what is below it: blend mode, track matte (the layer above it,
/// so not offered on the top layer), parent, and the solo and effects switches.
fn layer_section(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let target = cx.weak_entity();
    let composition = workspace.state.composition();
    let mut rows = div();
    if layer.content.has_picture() {
        let modes = BlendMode::ALL.iter().map(|&mode| (mode.label().to_owned(), mode == layer.blend_mode, mode)).collect();
        rows = rows.child(labeled_row("Blend Mode", choice_menu("layer-blend", layer.blend_mode.label().to_owned(), modes, move |this, mode| this.state.set_blend_mode(id, mode), target.clone())));
        let is_top = composition.layers.first().map(|top| top.id) == Some(id);
        if !is_top || layer.track_matte != TrackMatte::None {
            let mattes = TrackMatte::ALL.iter().map(|&matte| (matte.label().to_owned(), matte == layer.track_matte, matte)).collect();
            rows = rows.child(labeled_row("Track Matte", choice_menu("layer-matte", layer.track_matte.label().to_owned(), mattes, move |this, matte| this.state.set_track_matte(id, matte), target.clone())));
        }
        rows = rows.child(mask_row(layer, workspace, cx));
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
            .when(layer.content.has_picture(), |row| row.child(switch("layer-effects", "Effects", layer.switches.effects_enabled, Switch::EffectsEnabled, cx)))
            // Adjustment layers have nothing of their own to move, so motion blur isn't offered.
            .when(layer.content.has_picture() && !matches!(layer.content, LayerContent::Adjustment), |row| {
                row.child(switch("layer-motion-blur", "Motion Blur", layer.switches.motion_blur, Switch::MotionBlur, cx))
            }),
    )
    // The shutter is the composition's, so say where it is set.
    .when(layer.switches.motion_blur && !matches!(layer.content, LayerContent::Adjustment), |rows| {
        let shutter = composition.motion_blur;
        rows.child(status_line(format!("{}° shutter, {} samples (set in Composition Settings)", shutter.shutter_angle, shutter.samples)))
    })
    // Clips with sound can be captioned.
    .when(workspace.state.caption_source(id).is_some() && workspace.ui.settings.ai.captions, |rows| {
        let busy = workspace.ui.caption_job.is_some();
        rows.child(
            div().py_1().child(
                Button::new("generate-captions")
                    .outline()
                    .small()
                    .icon(Icon::new(IconName::Captions))
                    .label(if busy { "Making captions…" } else { "Generate Captions" })
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| super::captions::generate(this, id, window, cx))),
            ),
        )
    })
}

/// Remove Background, for footage and images: pick the subject by clicking it in the viewer,
/// choose whether to keep it or take it out, and for video, follow it through the clip.
fn cutout_section(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let target = cx.weak_entity();
    let picking = workspace.ui.cutout_pick == Some(id);
    let job = workspace.ui.cutout_job.as_ref().filter(|job| job.layer == id);
    let is_video = matches!(layer.content, LayerContent::Footage { .. });
    let cut = layer.cutout.as_ref();
    let mut section = div();
    let pick = Button::new("cutout-pick")
        .small()
        .icon(Icon::new(IconName::WandSparkles))
        .label(if picking { "Done Picking" } else if cut.is_some() { "Pick Again" } else { "Pick Subject" })
        .when(picking, |button| button.primary())
        .when(!picking, |button| button.outline())
        .on_click(cx.listener(move |this, _, _, cx| {
            this.ui.cutout_pick = if this.ui.cutout_pick == Some(id) { None } else { Some(id) };
            cx.notify();
        }));
    if picking {
        section = section.child(status_line("Click to keep, Option-click to leave out."));
    }
    // Until there is a cut-out, the heading's + starts picking.
    let Some(cut) = cut else {
        return section.when(!picking, |section| section.child(add_note()));
    };
    section = section.child(div().py_1().child(pick));
    let modes = [("Keep Subject", false), ("Remove Subject", true)].map(|(label, remove)| (label.to_owned(), remove == cut.remove, remove)).to_vec();
    let mode_label = if cut.remove { "Remove Subject" } else { "Keep Subject" };
    section = section.child(labeled_row("Picked", choice_menu("cutout-mode", mode_label.to_owned(), modes, move |this, remove| this.state.set_cutout_remove(id, remove), target)));
    // Where the masks stand: being made, made for one frame, or followed through the clip.
    let state = match (job, cut.frames) {
        (Some(job), _) if job.tracking => "Following the subject through the clip…".to_owned(),
        (Some(_), _) => "Finding the subject…".to_owned(),
        (None, None) if cut.clicks.is_empty() => "Click the subject in the viewer.".to_owned(),
        (None, None) => "Not made yet.".to_owned(),
        (None, Some((start, end))) if is_video && end - start <= 1 => "Made for the clicked frame. Track it to follow the subject through the clip.".to_owned(),
        (None, Some((start, end))) if is_video => format!("Following the subject over {} frames.", end - start),
        (None, Some(_)) => "Cut out.".to_owned(),
    };
    section = section.child(status_line(state));
    if is_video {
        let busy = job.is_some();
        section = section.child(
            div().py_1().child(
                Button::new("cutout-track")
                    .outline()
                    .small()
                    .label("Track Through Clip")
                    .disabled(busy || cut.clicks.is_empty())
                    .on_click(cx.listener(move |this, _, window, cx| super::cutout::make_masks(this, id, true, window, cx))),
            ),
        );
    }
    section
}

/// Duck Under: another layer with sound that this one gets quieter under (music under a voice).
fn duck_row(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let composition = workspace.state.composition();
    let mut choices: Vec<(String, bool, Option<Id>)> = vec![("Nothing".to_owned(), layer.audio.duck_under.is_none(), None)];
    // Only a single clip's sound can be ducked under, not a nested composition's many.
    let one_clip = |other: &Layer| other.has_audio && matches!(other.content, LayerContent::Footage { .. } | LayerContent::Audio { .. });
    choices.extend(composition.layers.iter().filter(|other| other.id != id && one_clip(other)).map(|other| (other.name.clone(), layer.audio.duck_under == Some(other.id), Some(other.id))));
    let current = layer.audio.duck_under.and_then(|under| composition.layer(under)).map_or("Nothing".to_owned(), |under| under.name.clone());
    labeled_row("Duck Under", choice_menu("duck-under", current, choices, move |this, under| this.state.set_duck_under(id, under), cx.weak_entity()))
}

/// Speed, for footage: a constant speed, keyframed speed (time remapping) or a ready-made ramp, playing backwards, and making a version in
/// another shape that follows the subject (Auto Reframe).
fn speed_section(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let target = cx.weak_entity();
    let ramped = layer.time.constant_factor().is_none();
    // Speed is a keyframeable property: its stopwatch remaps time across the clip. While it
    // changes over time, a button offers the way back to the steady speed it had before.
    let speed = crate::panels::fields::property_row(id, studio_core::property::PropertyPath::Speed, None, workspace, cx);
    let unramp = ramped.then(|| {
        let back_to = layer.time.speed_before_ramp.unwrap_or(100.0);
        div().py_1().child(
            Button::new("speed-unramp")
                .ghost()
                .xsmall()
                .icon(Icon::new(IconName::RotateCcw))
                .label("Remove Speed Changes")
                .tooltip(format!("Back to a steady {back_to:.0}%, as before"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.state.remove_speed_ramp(id);
                    cx.notify();
                })),
        )
    });
    // `None` is the menu's way to take a ramp off.
    let mut ramps: Vec<(String, bool, Option<studio_core::speed::Ramp>)> = vec![("No Ramp".to_owned(), !ramped, None)];
    ramps.extend(studio_core::speed::Ramp::ALL.iter().map(|&ramp| (ramp.name().to_owned(), false, Some(ramp))));
    let ramp_label = if ramped { "Ramping" } else { "Choose a ramp…" };
    let busy = workspace.ui.analysis_job.is_some();
    div()
        .child(speed)
        .children(unramp)
        .child(labeled_row(
            "Ramp",
            choice_menu("speed-ramp", ramp_label.to_owned(), ramps, move |this, ramp| match ramp {
                Some(ramp) => this.state.apply_speed_ramp(id, ramp),
                None => this.state.remove_speed_ramp(id),
            }, target.clone()),
        ))
        .child(
            div().py_1().child(Checkbox::new("speed-reverse").label("Play backwards").checked(layer.time.reverse).on_click(cx.listener(move |this, checked: &bool, _, cx| {
                this.state.set_reverse(id, *checked);
                cx.notify();
            }))),
        )
        .when(workspace.ui.settings.ai.auto_reframe, |section_div| {
            section_div
                .child(divider())
                .child(section("Auto Reframe", Some(info_button("reframe-info", REFRAME_INFO))))
                .child(div().py_1().child(if busy {
                    div().text_color(hsla(color::MUTED)).child("Another analysis is running.").into_any_element()
                } else {
                    reframe_menu(id, cx).into_any_element()
                }))
        })
}

/// Whether a layer can be pinned to moving video: graphics and text, not footage or sound.
fn trackable(layer: &Layer) -> bool {
    layer.transform.is_some() && !matches!(layer.content, LayerContent::Footage { .. } | LayerContent::Audio { .. } | LayerContent::Adjustment)
}

/// Track Motion: pin this layer to a point that moves in a video below it.
fn track_section(layer: &Layer, workspace: &Workspace) -> Div {
    let picking = workspace.ui.track_pick == Some(layer.id);
    // The heading's + starts picking (and cancels it).
    div().child(if picking { status_line("Click the point to follow in the viewer.") } else { add_note() })
}

/// Whether a layer can be stabilized: video with a transform.
fn stabilizable(layer: &Layer) -> bool {
    // A freeze frame holds one picture, so it has no shake to steady.
    layer.transform.is_some() && matches!(layer.content, LayerContent::Footage { .. }) && layer.time.hold.is_none()
}

/// Stabilize: how smooth the steadied camera is and how much the clip was enlarged. Until the
/// clip is stabilized the heading's + starts it.
fn stabilize_section(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let Some(stabilization) = layer.stabilization.as_ref() else {
        let note = if workspace.ui.analysis_job.is_some() { status_line("Another analysis is running.") } else { add_note() };
        return div().child(note);
    };
    let enlarged = if stabilization.zoom > 1.0 { format!("Enlarged to {:.0}% to hide the moving edges.", stabilization.zoom * 100.0) } else { "Steady without enlarging.".to_owned() };
    div().child(labeled_row("Smoothness", smoothness_menu(layer.id, stabilization.smoothness, cx))).child(status_line(enlarged))
}

/// The Smoothness menu of a stabilized clip: choosing one steadies it again from the motion
/// already measured, which is instant.
fn smoothness_menu(id: Id, current: f32, cx: &mut Context<Workspace>) -> impl IntoElement {
    let target = cx.weak_entity();
    Button::new("stabilize-smoothness").outline().small().label(format!("{current:.0}%")).dropdown_caret(true).w_full().dropdown_menu(move |menu, _, _| {
        studio_core::stabilize::SMOOTHNESS_CHOICES.into_iter().fold(menu, |menu, smoothness| {
            let target = target.clone();
            menu.item(PopupMenuItem::new(format!("{smoothness:.0}%")).checked(smoothness == current).on_click(move |_, window, cx: &mut App| {
                target
                    .update(cx, |this, cx| {
                        match this.state.set_stabilize_smoothness(id, smoothness) {
                            Ok(Some(fit)) if fit.edges_show => window.push_notification(super::analysis::stabilized_message(fit, 0).as_str(), cx),
                            Ok(_) => {}
                            Err(error) => window.push_notification(error.to_string().as_str(), cx),
                        }
                        cx.notify();
                    })
                    .ok();
            }))
        })
    })
}

/// The Auto Reframe menu: one shape per item; choosing one starts the job.
fn reframe_menu(id: Id, cx: &mut Context<Workspace>) -> impl IntoElement {
    let target = cx.weak_entity();
    Button::new("auto-reframe").outline().small().label("Reframe to…").dropdown_caret(true).w_full().dropdown_menu(move |menu, _, _| {
        studio_core::templates::Format::ALL.into_iter().fold(menu, |menu, format| {
            let target = target.clone();
            menu.item(PopupMenuItem::new(format!("{} ({}×{})", format.label(), format.size().0, format.size().1)).on_click(move |_, window, cx: &mut App| {
                target.update(cx, |this, cx| super::analysis::auto_reframe(this, id, format, window, cx)).ok();
            }))
        })
    })
}

/// Strengths offered for Enhance Speech, in percent.
const ENHANCE_STRENGTHS: [f32; 5] = [50.0, 70.0, 80.0, 90.0, 100.0];

/// Enhance Speech: cleaning up a voice recording like Adobe Podcast does. The cleaned copy is
/// made in the background (see `panels::enhance`); until it is ready the original plays.
fn enhance_section(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    use studio_core::model::{EnhanceModel, SpeechEnhance};
    let id = layer.id;
    let Some(settings) = layer.audio.enhance.clone() else {
        // The heading's + sets it up.
        return div().child(add_note());
    };
    let target = cx.weak_entity();
    let models = [(EnhanceModel::Studio, "Studio (best)"), (EnhanceModel::Light, "Light (fastest)"), (EnhanceModel::Off, "Off (keep background)")];
    let model_name = models.iter().find(|(model, _)| *model == settings.model).map_or("Studio (best)", |(_, name)| name);
    let model_choices = models.iter().map(|&(model, name)| (name.to_owned(), model == settings.model, model)).collect();
    let strengths = ENHANCE_STRENGTHS.iter().map(|&strength| (format!("{strength}%"), (strength - settings.strength).abs() < 0.5, strength)).collect();
    let toggle = |name: &'static str, label: &'static str, on: bool, change: fn(&mut SpeechEnhance, bool), cx: &mut Context<Workspace>| {
        Checkbox::new(name).label(label).checked(on).on_click(cx.listener(move |this, checked: &bool, _, cx| {
            let checked = *checked;
            this.state.edit_enhance(id, |settings| change(settings, checked));
            cx.notify();
        }))
    };
    let job = workspace.ui.enhance_job.as_ref().filter(|job| job.layer == id);
    let skipped = workspace.ui.enhance_skipped.iter().any(|(layer, made_for)| *layer == id && made_for.same_sound(&settings));
    let status = if !settings.on {
        status_line("Off: the original plays.")
    } else if settings.file.is_some() {
        status_line("Cleaned: this plays and exports.")
    } else if let Some(job) = job {
        status_line(format!("{} · {}%", job.stage, (job.fraction * 100.0) as i64))
    } else if skipped {
        div().flex().items_center().gap_2().child(status_line("Not cleaned (stopped or failed).")).child(Button::new("enhance-retry").ghost().xsmall().label("Try Again").on_click(cx.listener(move |this, _, _, cx| {
            this.ui.enhance_skipped.retain(|(layer, _)| *layer != id);
            cx.notify();
        })))
    } else {
        status_line("Waiting for another clip to finish…")
    };
    div()
        .child(div().py_0p5().child(toggle("enhance-on", "On", settings.on, |settings, on| settings.on = on, cx)))
        .child(labeled_row("Noise removal", choice_menu("enhance-model", model_name.to_owned(), model_choices, move |this, model| { this.state.edit_enhance(id, |settings| settings.model = model); }, target.clone())))
        .child(labeled_row("Strength", choice_menu("enhance-strength", format!("{}%", settings.strength), strengths, move |this, strength| { this.state.edit_enhance(id, |settings| settings.strength = strength); }, target.clone())))
        .child(div().flex().flex_col().gap_1().py_1()
            .child(toggle("enhance-hum", "Remove hum", settings.remove_hum, |settings, on| settings.remove_hum = on, cx))
            .child(toggle("enhance-eq", "Voice EQ", settings.voice_eq, |settings, on| settings.voice_eq = on, cx))
            .child(toggle("enhance-deess", "Soften harsh \"s\" sounds", settings.de_ess, |settings, on| settings.de_ess = on, cx))
            .child(toggle("enhance-level", "Even out loudness (-16 LUFS)", settings.even_out, |settings, on| settings.even_out = on, cx)))
        .child(status)
}

/// A layer's sound: whether its picture shows and its sound plays (separately), its volume and
/// fades, and for a video clip, moving its sound onto a clip of its own.
fn audio_section(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let field = |setting: AudioSetting, cx: &mut Context<Workspace>| value_field(FieldTarget::Audio { layer: id, setting }, workspace, cx);
    let toggle = |name: &'static str, label: &'static str, on: bool, which: Switch, cx: &mut Context<Workspace>| {
        Checkbox::new(name).label(label).checked(on).on_click(cx.listener(move |this, _, _, cx| {
            this.state.toggle_switch(id, which);
            cx.notify();
        }))
    };
    let is_video = matches!(layer.content, LayerContent::Footage { .. });
    let busy = workspace.ui.analysis_job.is_some();
    let normal_speed = layer.time.is_normal();
    let has_markers = !workspace.state.composition().markers.is_empty();
    // Silences and beats are found in one file's sound, which a nested composition doesn't have.
    let one_file = !matches!(layer.content, LayerContent::Composition { .. });
    div()
        .child(
            div()
                .flex()
                .gap_4()
                .py_1()
                .when(is_video, |row| row.child(toggle("layer-show-video", "Show video", layer.switches.visible, Switch::Visible, cx)))
                .child(toggle("layer-play-audio", "Play audio", layer.switches.audio, Switch::Audio, cx)),
        )
        .child(labeled_row("Volume", field(AudioSetting::Volume, cx)))
        .child(labeled_row("Fade In", field(AudioSetting::FadeIn, cx)))
        .child(labeled_row("Fade Out", field(AudioSetting::FadeOut, cx)))
        .child(labeled_row("Noise Gate", field(AudioSetting::NoiseGate, cx)))
        .child(duck_row(layer, workspace, cx))
        .when(layer.audio.duck_under.is_some(), |section| section.child(labeled_row("Duck By", field(AudioSetting::DuckAmount, cx))))
        .child(
            div()
                .py_1()
                .flex()
                .flex_wrap()
                .gap_2()
                .when(one_file, |row| row.child(
                    Button::new("remove-silences")
                        .outline()
                        .small()
                        .label("Remove Silences")
                        .tooltip(if normal_speed { "Cut the pauses out and close the gaps (jump cuts)" } else { "Set the clip back to normal speed to remove its silences" })
                        .disabled(busy || !normal_speed)
                        .on_click(cx.listener(move |this, _, window, cx| super::analysis::remove_silences(this, id, window, cx))),
                ))
                .when(one_file, |row| row.child(
                    Button::new("mark-beats")
                        .outline()
                        .small()
                        .label("Mark Beats")
                        .tooltip("Put a marker on every beat; cuts and moves snap to them")
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, window, cx| super::analysis::mark_beats(this, id, window, cx))),
                ))
                .when(has_markers, |row| {
                    row.child(Button::new("clear-markers").ghost().small().label("Clear Markers").on_click(cx.listener(|this, _, _, cx| {
                        this.state.clear_markers();
                        cx.notify();
                    })))
                }),
        )
        .when(is_video, |section| {
            section.child(
                div().py_1().child(
                    Button::new("separate-audio")
                        .outline()
                        .small()
                        .icon(Icon::new(IconName::AudioLines))
                        .label("Separate Audio")
                        // The separated sound plays at normal speed, so a sped-up clip's would not match.
                        .when(!normal_speed, |button| button.disabled(true).tooltip("Set the clip back to normal speed to separate its audio"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.state.separate_audio(id).is_some() {
                                cx.notify();
                            }
                        })),
                ),
            )
        })
}

/// A caption layer's settings, for all its captions at once: the animation, how many words
/// show together, capitals, the font and colors, the spoken word's color and a background box,
/// then the words themselves (click a phrase to jump to it, double-click to fix its text).
fn captions_section(layer: &Layer, style: &CaptionStyle, words: &[CaptionWord], workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let weak = cx.weak_entity();
    let animations = CaptionAnimation::ALL.iter().map(|&animation| (animation.name().to_owned(), animation == style.animation, animation)).collect();
    let per_phrase = (1..=6u32).map(|count| (if count == 1 { "1 word".to_owned() } else { format!("{count} words") }, count == style.words_per_phrase, count)).collect();
    let uppercase = Checkbox::new("captions-uppercase").label("All capitals").checked(style.uppercase).on_click(cx.listener(move |this, checked: &bool, _, cx| {
        let checked = *checked;
        this.state.edit_captions(id, |style| style.uppercase = checked);
        cx.notify();
    }));
    let background_on = Checkbox::new("captions-background-on").checked(style.background.is_some()).on_click(cx.listener(move |this, checked: &bool, _, cx| {
        let checked = *checked;
        this.state.edit_captions(id, |style| style.background = checked.then(|| Rgba::hex(0x000000).with_alpha(0.7)));
        cx.notify();
    }));
    let mut families: Vec<&str> = workspace.ui.font_families.clone();
    if !families.contains(&style.text.font_family.as_str()) {
        families.insert(0, style.text.font_family.as_str());
    }
    let weight = style.text.font_weight;
    let family_choices = families.iter().map(|family| ((*family).to_owned(), *family == style.text.font_family, (*family).to_owned())).collect();
    let weight_choices = FONT_WEIGHTS.iter().map(|(value, name)| ((*name).to_owned(), *value == weight, *value)).collect();
    let current_family = style.text.font_family.clone();
    let stroke_width = style.text.stroke.map_or(4.0, |(_, width)| width);

    let fps = workspace.state.composition().frame_rate.fps();
    let start_frame = layer.start_frame;
    let mut transcript = div().flex().flex_col().gap_1();
    for range in studio_core::captions::phrases(words, style.words_per_phrase, fps) {
        let text = words[range.clone()].iter().map(|word| word.text.as_str()).collect::<Vec<_>>().join(" ");
        // When it shows, sync included.
        let at = start_frame + style.offset + words[range.start].start;
        let editing = workspace.ui.editing.as_ref().filter(|editing| editing.target == EditTarget::CaptionText(id, range.start, range.end)).and_then(|editing| editing.input.line());
        // Its start and end, each typed or dragged to retime it.
        let edge = |start: bool| FieldTarget::CaptionPhrase { layer: id, first: range.start, end: range.end, start };
        let times = div()
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .text_size(px(size::SMALL_TEXT))
            .text_color(hsla(color::MUTED))
            .child(value_field(edge(true), workspace, cx))
            .child("–")
            .child(value_field(edge(false), workspace, cx));
        let row = div()
            .id(("caption-phrase", range.start))
            .flex()
            .items_center()
            .gap_2()
            .px_1()
            .py_0p5()
            .rounded(px(4.0))
            .hover(|style| style.bg(hsla(color::HOVER)));
        let row = match editing {
            Some(input) => row.child(div().flex_1().child(Input::new(input).xsmall())).into_any_element(),
            None => {
                let (start, end, current) = (range.start, range.end, text.clone());
                row.cursor_pointer()
                    .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                        if event.click_count() >= 2 {
                            this.begin_edit(EditTarget::CaptionText(id, start, end), current.clone(), window, cx);
                        } else {
                            this.go_to(at, cx);
                        }
                    }))
                    .child(div().flex_1().min_w_0().child(text))
                    .test_support()
                    .into_any_element()
            }
        };
        transcript = transcript.child(div().flex().flex_col().child(times).child(row));
    }

    div()
        .child(labeled_row("Animation", choice_menu("captions-animation", style.animation.name().to_owned(), animations, move |this, animation| this.state.edit_captions(id, |style| style.animation = animation), weak.clone())))
        .child(labeled_row("Words at once", choice_menu("captions-words", format!("{}", style.words_per_phrase), per_phrase, move |this, count| this.state.edit_captions(id, |style| style.words_per_phrase = count), weak.clone())))
        .child(div().py_1().child(uppercase))
        .child(div().h(px(30.0)).child(choice_menu("captions-font-family", style.text.font_family.clone(), family_choices, move |this, family: String| this.state.set_font(id, &family, weight), weak.clone())))
        .child(div().h(px(30.0)).child(choice_menu("captions-font-weight", weight_name(weight).to_owned(), weight_choices, move |this, weight: u16| this.state.set_font(id, &current_family, weight), weak)))
        .child(div().grid().grid_cols(2).gap_x_3().child(setting_field(IconName::ALargeSmall, "Size", id, TextSetting::FontSize, workspace, cx)).child(setting_field(IconName::MoveHorizontal, "Tracking", id, TextSetting::Tracking, workspace, cx)))
        .child(paint_row("fill", "Fill", style.text.fill, ColorTarget::TextFill(id), move |this, fill| this.state.set_text_fill(id, fill), cx))
        .child(paint_row("stroke", "Stroke", style.text.stroke.map(|(color, _)| color), ColorTarget::TextStroke(id), move |this, color| this.state.set_text_stroke(id, color.map(|color| (color, stroke_width))), cx))
        .child(labeled_row("Spoken word", color_picker::swatch("captions-highlight", ColorTarget::CaptionHighlight(id), Some(style.highlight), cx)))
        .child(labeled_row("Background", div().flex().items_center().gap_2().child(background_on).child(color_picker::swatch("captions-background", ColorTarget::CaptionBackground(id), style.background, cx))))
        .child(labeled_row("Sync", div().flex().items_center().gap_2().child(value_field(FieldTarget::CaptionSync { layer: id }, workspace, cx)).child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("+ later, − earlier"))))
        // Saving the captions as a subtitle file, timed as they show in the composition.
        .when(workspace.has(Feature::CaptionFiles), |rows| rows.child(div().py_1().flex().flex_wrap().gap_2().children(SubtitleFormat::ALL.map(|format| {
            Button::new(gpui_kit::ElementId::Name(format!("captions-export-{}", format.extension()).into()))
                .outline()
                .small()
                .label(format!("Export {}…", format.name()))
                .tooltip(format!("Save these captions as a .{} subtitle file", format.extension()))
                .on_click(cx.listener(move |this, _, window, cx| super::subtitles::export(this, id, format, window, cx)))
        }))))
        .child(divider())
        .child(section(format!("Transcript · {} words", words.len()), None))
        .child(transcript)
}

/// What each section is for, shown by the info button on its heading.
const TRANSFORM_INFO: &str = "Where the layer sits, its size, turn and opacity. Pick a corner, an edge or the center for the anchor point, or drag the anchor in the viewer.";
const SPEED_INFO: &str = "A steady speed changes the clip's length; a ramp keeps it. Sound is silent while a clip ramps or plays backwards.";
const REFRAME_INFO: &str = "Makes a new composition in another shape, moving the clip to keep its subject in frame. Remove Background clicks pick the subject; otherwise it follows the middle.";
const TRACK_INFO: &str = "Pins this layer to something moving in a video: its position follows the point you click.";
const STABILIZE_INFO: &str = "Steadies shaky footage: measures how the camera moves, smooths that path and moves the clip against the shake, enlarging it just enough to hide the edges.";
const CUTOUT_INFO: &str = "Pick a subject to cut it out of its background, or an object to take it out of the shot. Click what to keep, Option-click what to leave out.";
const ENHANCE_INFO: &str = "Removes background noise and hum, then EQs, de-esses and evens out the voice to podcast loudness. Runs on this Mac; the original file is untouched.";

/// The note in a section that is empty until the heading's + adds something.
pub(crate) fn add_note() -> Div {
    status_line("Click + to add this to your layer.")
}

/// A short line of muted status text inside a section, such as what a job is doing.
pub(crate) fn status_line(text: impl Into<gpui_kit::SharedString>) -> Div {
    div().py_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(text.into())
}

/// The Reset button in the Speed heading: back to a steady 100%, forwards. Greyed out while
/// the clip already plays that way.
fn speed_reset(layer: &Layer, cx: &mut Context<Workspace>) -> AnyElement {
    let id = layer.id;
    heading_button("reset-speed", IconName::RotateCcw, "Reset Speed", false, cx.listener(move |this, _, _, cx| {
        this.state.reset_speed(id);
        cx.notify();
    }))
    .disabled(layer.time.is_normal())
    .into_any_element()
}

/// The Track Motion heading's + button: starts picking the point to follow (an X cancels).
fn track_add(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let id = layer.id;
    let picking = workspace.ui.track_pick == Some(id);
    let (icon, tip) = if picking { (IconName::X, "Cancel") } else { (IconName::Plus, "Track Motion") };
    heading_button("track-motion-add", icon, tip, picking, cx.listener(move |this, _, _, cx| {
        this.ui.track_pick = if this.ui.track_pick == Some(id) { None } else { Some(id) };
        cx.notify();
    }))
    .disabled(workspace.ui.analysis_job.is_some())
    .into_any_element()
}

/// The Stabilize heading's button: + measures the clip's motion and steadies it, and once it
/// is stabilized the trash puts the layer back as it was.
fn stabilize_button(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let id = layer.id;
    if layer.stabilization.is_some() {
        return heading_button("stabilize-remove", IconName::Trash, "Remove Stabilization", false, cx.listener(move |this, _, _, cx| {
            this.state.remove_stabilization(id);
            cx.notify();
        }))
        .into_any_element();
    }
    heading_button("stabilize-add", IconName::Plus, "Stabilize", false, cx.listener(move |this, _, window, cx| {
        super::analysis::stabilize(this, id, window, cx);
        cx.notify();
    }))
    .disabled(workspace.ui.analysis_job.is_some())
    .into_any_element()
}

/// The Remove Background heading's button: + picks a subject while there is no cut-out, and
/// the trash clears the cut-out once there is one.
fn cutout_button(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let id = layer.id;
    if layer.cutout.is_none() {
        let picking = workspace.ui.cutout_pick == Some(id);
        let (icon, tip) = if picking { (IconName::X, "Done Picking") } else { (IconName::Plus, "Pick Subject") };
        return heading_button("cutout-add", icon, tip, picking, cx.listener(move |this, _, _, cx| {
            this.ui.cutout_pick = if this.ui.cutout_pick == Some(id) { None } else { Some(id) };
            cx.notify();
        }))
        .into_any_element();
    }
    heading_button("cutout-clear", IconName::Trash, "Clear Cut-out", false, cx.listener(move |this, _, _, cx| {
        if this.ui.cutout_job.as_ref().is_some_and(|job| job.layer == id) {
            super::cutout::cancel(this);
        }
        this.state.clear_cutout(id);
        this.ui.cutout_pick = None;
        cx.notify();
    }))
    .into_any_element()
}

/// The Enhance Speech heading's button: + sets it up on the clip, and once it is, the trash
/// takes it off again.
fn enhance_button(layer: &Layer, cx: &mut Context<Workspace>) -> AnyElement {
    use studio_core::model::SpeechEnhance;
    let id = layer.id;
    let set_up = layer.audio.enhance.is_some();
    let (name, icon, tip) = if set_up { ("enhance-remove", IconName::Trash, "Remove Enhance Speech") } else { ("enhance-add", IconName::Plus, "Enhance Speech") };
    heading_button(name, icon, tip, false, cx.listener(move |this, _, _, cx| {
        this.state.set_enhance(id, (!set_up).then(SpeechEnhance::default));
        cx.notify();
    }))
    .into_any_element()
}

/// The Reset button in the Layer Transform heading.
fn transform_reset(id: Id, cx: &mut Context<Workspace>) -> AnyElement {
    heading_button("reset-transform", IconName::RotateCcw, "Reset Transform", false, cx.listener(move |this, _, _, cx| {
        this.state.reset_transform(id);
        cx.notify();
    }))
    .into_any_element()
}

fn transform_section(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let linked = workspace.state.scale_linked;
    let mut rows = div();
    for property in TransformProperty::ALL {
        // Videos and images get Size above Scale: the pixels they are drawn at, leaving Scale
        // free to animate from 0 to 100% of that.
        if property == TransformProperty::Scale && workspace.state.media_size(id).is_some() {
            rows = rows.child(media_size_row(id, workspace, cx));
        }
        let extra = (property == TransformProperty::Scale).then(|| {
            let icon = if linked { IconName::Link } else { IconName::Unlink };
            icon_button("scale-link", icon, "Constrain Proportions", linked, cx.listener(|this, _, _, cx| {
                this.state.scale_linked = !this.state.scale_linked;
                cx.notify();
            }))
            .into_any_element()
        });
        rows = rows.child(property_row(id, PropertyPath::Transform(property), extra, workspace, cx));
        if property == TransformProperty::AnchorPoint {
            rows = rows.child(anchor_grid(layer, workspace, cx));
        }
    }
    rows
}

/// The Size row of a video or image layer, laid out like the property rows around it: width and
/// height in pixels with a link to keep their proportions. Size isn't animated, so the stopwatch
/// column is empty.
fn media_size_row(id: Id, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let linked = workspace.state.media_size_linked;
    let link = icon_button("media-size-link", if linked { IconName::Link } else { IconName::Unlink }, "Constrain Proportions", linked, cx.listener(|this, _, _, cx| {
        this.state.media_size_linked = !this.state.media_size_linked;
        cx.notify();
    }));
    let field = |side: usize, cx: &mut Context<Workspace>| value_field(FieldTarget::MediaSize { layer: id, side }, workspace, cx);
    div()
        .h(px(26.0))
        .flex()
        .items_center()
        .gap_2()
        .child(div().w(px(18.0)).flex_none())
        .child(div().w(px(104.0)).flex_none().truncate().child("Size"))
        .child(div().w(px(18.0)).flex_none().child(link))
        .child(div().flex().gap_3().child(field(0, cx)).child(field(1, cx)))
}

/// Nine buttons in the layout of the layer's box (corners, edge middles, center): pressing one
/// moves the anchor point there, keeping the layer where it is. The one the anchor is on is lit.
fn anchor_grid(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let frame = workspace.state.current_frame as f64;
    let anchor = layer.transform.as_ref().map(|transform| transform.anchor_point.value_at(frame));
    let size = layer.size;
    let at = |(fx, fy): (f32, f32)| anchor.is_some_and(|a| (a.x - size.x * fx).abs() < 0.5 && (a.y - size.y * fy).abs() < 0.5);
    let cell = |index: usize, fraction: (f32, f32), cx: &mut Context<Workspace>| {
        let lit = at(fraction);
        div()
            .id(("anchor-grid", index))
            .size(px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(3.0))
            .cursor_pointer()
            .hover(|style| style.bg(hsla(color::HOVER)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.state.set_anchor_fraction(id, fraction);
                cx.notify();
            }))
            .child(div().size(px(if lit { 7.0 } else { 5.0 })).rounded_full().bg(hsla(if lit { color::ACCENT } else { color::MUTED })))
            .test_support()
    };
    // Three rows of three, top to bottom, as the points sit on the layer's box.
    let mut grid = div().flex_none().flex().flex_col().gap(px(1.0)).p(px(2.0)).rounded(px(5.0)).border_1().border_color(hsla(color::HAIRLINE));
    for row in 0..3 {
        let mut line = div().flex().gap(px(1.0));
        for column in 0..3 {
            let index = row * 3 + column;
            line = line.child(cell(index, crate::state::ANCHOR_POINTS[index], cx));
        }
        grid = grid.child(line);
    }
    div()
        .flex()
        .items_center()
        .gap_3()
        .py_1()
        .pl(px(28.0))
        .child(grid)
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

/// A text fill or stroke row: an on/off checkbox, its color (which opens the color wheel), and its label.
fn paint_row(
    id: &'static str,
    label: &'static str,
    paint: Option<Rgba>,
    target: ColorTarget,
    set: impl Fn(&mut Workspace, Option<Rgba>) + Clone + 'static,
    cx: &mut Context<Workspace>,
) -> Div {
    let workspace = cx.weak_entity();
    let last = paint.unwrap_or(Rgba::hex(0xffffff));
    let toggle = Checkbox::new(ElementId::Name(format!("{id}-on").into())).checked(paint.is_some()).on_click(move |checked: &bool, _, cx: &mut App| {
        let (set, checked) = (set.clone(), *checked);
        workspace
            .update(cx, |this, cx| {
                set(this, checked.then_some(last));
                cx.notify();
            })
            .ok();
    });
    div()
        .flex()
        .items_center()
        .gap_2()
        .h(px(30.0))
        .child(toggle)
        .child(color_picker::swatch(ElementId::Name(format!("{id}-color").into()), target, paint, cx))
        .child(div().child(label))
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

    let source_text = if let Some(editing) = workspace.ui.editing.as_ref().filter(|e| e.target == EditTarget::SourceText(id) && !e.in_viewer) {
        // Several lines: Return starts a new one; ⌘Return or a click elsewhere finishes.
        div().flex_1().min_h(px(48.0)).p_1().rounded(px(4.0)).border_1().border_color(hsla(color::HAIRLINE)).child(editing.input.element()).into_any_element()
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
        .child(paint_row("fill", "Fill", style.fill, ColorTarget::TextFill(id), move |this, fill| this.state.set_text_fill(id, fill), cx))
        .child(paint_row("stroke", "Stroke", style.stroke.map(|(color, _)| color), ColorTarget::TextStroke(id), move |this, color| this.state.set_text_stroke(id, color.map(|color| (color, stroke_width))), cx))
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
        .child(text_path_rows(layer, workspace, cx))
}

/// Path: set the text along one of the composition's pen paths, and where along it it starts.
fn text_path_rows(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let composition = workspace.state.composition();
    let start = layer.text_path.map_or(0.0, |on| on.start);
    let mut choices: Vec<(String, bool, Option<Id>)> = vec![("None".to_owned(), layer.text_path.is_none(), None)];
    choices.extend(
        composition
            .layers
            .iter()
            .filter(|candidate| studio_core::edit::can_set_text_on(composition, id, candidate.id))
            .map(|candidate| (candidate.name.clone(), layer.text_path.is_some_and(|on| on.layer == candidate.id), Some(candidate.id))),
    );
    let current = layer.text_path.and_then(|on| composition.layer(on.layer)).map_or("None".to_owned(), |path| path.name.clone());
    let menu = choice_menu("text-path", current, choices, move |this, chosen: Option<Id>| this.state.set_text_path(id, chosen.map(|layer| TextPath { layer, start })), cx.weak_entity());
    let rows = div().child(labeled_row("On Path", menu));
    if layer.text_path.is_none() {
        let has_paths = composition.layers.iter().any(|candidate| studio_core::edit::can_set_text_on(composition, id, candidate.id));
        return rows.when(!has_paths, |rows| rows.child(hint("Draw a path with the Pen tool to set this text along it.")));
    }
    rows.child(labeled_row("Start", value_field(FieldTarget::TextPathStart { layer: id }, workspace, cx)))
}

/// One of a shape's paints (fill or stroke): on or off, solid or gradient, its colors (each
/// opens the color wheel), and the gradient's angle or the stroke's width.
fn shape_paint_rows(id: Id, slot: PaintSlot, paint: Option<Paint>, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let weak = cx.weak_entity();
    let (label, key, angle, default) = match slot {
        PaintSlot::Fill => ("Fill", "fill", ShapeSetting::FillAngle, Paint::Solid(Rgba::hex(0x8b5cf6))),
        PaintSlot::Stroke => ("Stroke", "stroke", ShapeSetting::StrokeAngle, Paint::Solid(Rgba::hex(0xffffff))),
    };
    let toggle = {
        let weak = weak.clone();
        Checkbox::new(ElementId::Name(format!("shape-{key}-on").into())).checked(paint.is_some()).on_click(move |checked: &bool, _, cx: &mut App| {
            let checked = *checked;
            weak.update(cx, |this, cx| {
                this.state.set_shape_paint(id, slot, checked.then_some(default));
                cx.notify();
            })
            .ok();
        })
    };
    let kind = paint.map(Paint::kind);
    let kinds = PaintKind::ALL.iter().map(|&choice| (choice.name().to_owned(), Some(choice) == kind, choice)).collect();
    let menu = choice_menu(
        ElementId::Name(format!("shape-{key}-type").into()),
        kind.map_or("None", PaintKind::name).to_owned(),
        kinds,
        move |this, choice: PaintKind| {
            let current = this.state.shape_paint(id, slot).unwrap_or(default);
            this.state.set_shape_paint(id, slot, Some(current.into_kind(choice)));
        },
        weak,
    );
    let mut rows = div().child(labeled_row(label, div().flex().items_center().gap_2().child(toggle).child(div().flex_1().child(menu))));
    if let Some(paint) = paint {
        let mut controls = div().flex().items_center().gap_1p5();
        match paint.gradient() {
            // A gradient gets the stop editor; a solid its one color.
            // Taller than a labeled row, so it gets a row of its own, lined up with the values.
            Some(gradient) => {
                let editor = super::gradient_editor::editor(id, slot, gradient, workspace, cx);
                rows = rows.child(div().flex().gap_2().py_1().child(div().w(px(96.0)).flex_none()).child(div().flex_1().min_w_0().child(editor)));
            }
            None => {
                let target = ColorTarget::Shape { layer: id, slot, stop: 0 };
                controls = controls.child(color_picker::swatch(ElementId::Name(format!("shape-{key}-color:0").into()), target, paint.colors().first().copied(), cx));
            }
        }
        if paint.kind() == PaintKind::Linear {
            controls = controls.child(div().pl_2().text_color(hsla(color::MUTED)).child("Angle")).child(value_field(FieldTarget::Shape { layer: id, setting: angle }, workspace, cx));
        }
        if slot == PaintSlot::Stroke {
            controls = controls.child(div().pl_2().text_color(hsla(color::MUTED)).child("Width")).child(value_field(FieldTarget::Shape { layer: id, setting: ShapeSetting::StrokeWidth }, workspace, cx));
        }
        // A radial gradient fill has nothing more to show.
        let has_controls = paint.gradient().is_none() || paint.kind() == PaintKind::Linear || slot == PaintSlot::Stroke;
        if has_controls {
            rows = rows.child(labeled_row("", controls));
        }
    }
    rows
}

/// A shape layer's settings, laid out like After Effects': which shape, its size (or, for
/// polygons and stars, outer and inner radius), corners, points, fill and stroke.
fn shape_section(layer: &Layer, style: &ShapeStyle, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let weak = cx.weak_entity();
    let kinds = ShapeKind::ALL.iter().map(|&kind| (kind.name().to_owned(), kind == style.kind, kind)).collect();
    let field = |setting: ShapeSetting, cx: &mut Context<Workspace>| value_field(FieldTarget::Shape { layer: id, setting }, workspace, cx);
    let points_label = if style.kind == ShapeKind::Star { "Points" } else { "Sides" };
    let linked = workspace.state.shape_size_linked;
    let link = icon_button("shape-size-link", if linked { IconName::Link } else { IconName::Unlink }, "Keep proportions", linked, cx.listener(|this, _, _, cx| {
        this.state.shape_size_linked = !this.state.shape_size_linked;
        cx.notify();
    }));
    let size_row = labeled_row(
        "Size",
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(muted_label("W"))
            .child(field(ShapeSetting::Width, cx))
            .child(link)
            .child(muted_label("H"))
            .child(field(ShapeSetting::Height, cx)),
    );
    // Polygons and stars are sized by radius, as in After Effects.
    let radial = style.kind.has_points();
    let is_path = style.kind == ShapeKind::Path;
    let closed = match &layer.content {
        LayerContent::Shape { path: Some(path), .. } => path.closed,
        _ => true,
    };
    div()
        // A pen path keeps its own outline; the drawn shapes can switch between each other.
        .when(!is_path, |section| section.child(labeled_row("Shape", choice_menu("shape-kind", style.kind.name().to_owned(), kinds, move |this, kind| this.state.set_shape_kind(id, kind), weak))))
        .when(is_path, |section| {
            section.child(labeled_row(
                "Path",
                Checkbox::new("path-closed").label("Closed").checked(closed).on_click(cx.listener(move |this, _, _, cx| {
                    this.state.set_path_closed(id, !closed);
                    cx.notify();
                })),
            ))
        })
        .when(!radial, |section| section.child(size_row))
        .when(radial, |section| section.child(labeled_row("Outer Radius", field(ShapeSetting::OuterRadius, cx))))
        .when(style.kind == ShapeKind::Star, |section| section.child(labeled_row("Inner Radius", field(ShapeSetting::InnerRadius, cx))))
        .when(radial, |section| section.child(labeled_row(points_label, field(ShapeSetting::Points, cx))))
        .when(style.kind.has_corners(), |section| section.child(labeled_row("Corner Radius", field(ShapeSetting::CornerRadius, cx))))
        // An open path is a line: it has nothing inside to fill.
        .when(closed, |section| section.child(shape_paint_rows(id, PaintSlot::Fill, style.fill, workspace, cx)))
        .child(shape_paint_rows(id, PaintSlot::Stroke, style.stroke.map(|stroke| stroke.paint), workspace, cx))
}

/// A small muted label beside a field, such as "W" or "H".
fn muted_label(text: &'static str) -> Div {
    div().text_color(hsla(color::MUTED)).child(text)
}

fn properties_body(workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let composition = workspace.state.composition();
    let selected: Vec<&Layer> = workspace.state.selected_layers.iter().filter_map(|id| composition.layer(*id)).collect();
    let [layer] = selected.as_slice() else {
        let message = if selected.is_empty() { "Select a layer to see its properties." } else { "Select a single layer to edit its properties." };
        return hint(message).into_any_element();
    };
    let id = layer.id;
    let transform = if layer.transform.is_some() {
        fold("transform", "Layer Transform", Some(TRANSFORM_INFO), [Some(transform_reset(id, cx)), None], workspace, cx, |cx| transform_section(layer, workspace, cx))
    } else {
        list_row().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Audio layers have no transform.")
    };
    let text = match &layer.content {
        LayerContent::Text { text, style } => Some(fold("text", "Text", None, [None, None], workspace, cx, |cx| text_section(layer, text, style, workspace, cx))),
        LayerContent::Shape { style, .. } => Some(fold("shape", "Shape", None, [None, None], workspace, cx, |cx| shape_section(layer, style, workspace, cx))),
        LayerContent::Captions { words, style } => Some(fold("captions", "Captions", None, [None, None], workspace, cx, |cx| captions_section(layer, style, words, workspace, cx))),
        _ => None,
    };
    // Tell the user when an edit will change several keyframes at once.
    let selected_keys = workspace.state.selected_keyframes.iter().filter(|key| key.layer == layer.id).count();
    let mass_edit_note = (selected_keys > 1).then(|| {
        list_row()
            .flex()
            .items_center()
            .gap_1()
            .text_size(px(size::SMALL_TEXT))
            .text_color(hsla(color::MUTED))
            .child(format!("{selected_keys} keyframes selected"))
            .child(info_button("mass-edit-info", "With the playhead on one of them, a change applies to every selected keyframe of that property."))
    });
    card_list("properties-body")
        .children(mass_edit_note)
        .child(fold("layer", "Layer", None, [None, None], workspace, cx, |cx| layer_section(layer, workspace, cx)))
        .child(transform)
        .children(text)
        .children((workspace.has(Feature::Speed) && studio_core::speed::has_speed(&layer.content) && layer.time.hold.is_none()).then(|| fold("speed", "Speed", Some(SPEED_INFO), [Some(speed_reset(layer, cx)), None], workspace, cx, |cx| speed_section(layer, workspace, cx))))
        .children((workspace.has(Feature::Stabilize) && stabilizable(layer)).then(|| fold("stabilize", "Stabilize", Some(STABILIZE_INFO), [None, Some(stabilize_button(layer, workspace, cx))], workspace, cx, |cx| stabilize_section(layer, workspace, cx))))
        .children((workspace.has(Feature::TrackMotion) && trackable(layer)).then(|| fold("track", "Track Motion", Some(TRACK_INFO), [None, Some(track_add(layer, workspace, cx))], workspace, cx, |_| track_section(layer, workspace))))
        .children((studio_core::cutout::can_cut_out(layer) && workspace.ui.settings.ai.remove_background).then(|| fold("cutout", "Remove Background", Some(CUTOUT_INFO), [None, Some(cutout_button(layer, workspace, cx))], workspace, cx, |cx| cutout_section(layer, workspace, cx))))
        .children(workspace.state.has_sound(layer.id).then(|| fold("audio", "Audio", None, [None, None], workspace, cx, |cx| audio_section(layer, workspace, cx))))
        .children((workspace.state.can_enhance(layer.id) && workspace.ui.settings.ai.enhance_speech).then(|| fold("enhance", "Enhance Speech", Some(ENHANCE_INFO), [None, Some(enhance_button(layer, cx))], workspace, cx, |cx| enhance_section(layer, workspace, cx))))
        .into_any_element()
}

/// A Properties section, as a card like the Effects and Color tabs': a heading with an arrow
/// (click to open or close it), and `body` underneath while it is open. Sections start
/// closed. `info` explains the section in its info button's tooltip; `controls` are the
/// heading's reset button and its add or delete button, each optional, kept in those slots so
/// they line up from section to section.
pub(super) fn fold(
    key: &'static str,
    title: &'static str,
    info: Option<&'static str>,
    controls: [Option<AnyElement>; 2],
    workspace: &Workspace,
    cx: &mut Context<Workspace>,
    body: impl FnOnce(&mut Context<Workspace>) -> Div,
) -> Div {
    let open = workspace.ui.section_open(key);
    let heading = CardHeading {
        id: ElementId::Name(format!("section-{key}").into()),
        icon: Some(section_icon(key)),
        title: title.into(),
        info: info.map(Into::into),
        open,
        muted: false,
        trailing: controls.into(),
    };
    let body = open.then(|| body(cx).into_any_element());
    card(heading, cx.listener(move |this, _, _, cx| {
        this.ui.toggle_section(key);
        cx.notify();
    }), body)
}

/// The icon on a Properties section's heading.
fn section_icon(key: &str) -> IconName {
    match key {
        "layer" => IconName::Layers,
        "transform" => IconName::Move,
        "text" => IconName::Type,
        "shape" => IconName::Shapes,
        "captions" => IconName::Captions,
        "animate" => IconName::Sparkles,
        "textanim" => IconName::ALargeSmall,
        "morph" => IconName::Blend,
        "easing" => IconName::Spline,
        "speed" => IconName::Hourglass,
        "track" => IconName::Crosshair,
        "stabilize" => IconName::Scan,
        "cutout" => IconName::Eraser,
        "audio" => IconName::Volume2,
        "enhance" => IconName::AudioLines,
        _ => IconName::Settings,
    }
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let body = match workspace.ui.tab(PanelId::Properties) {
        0 => properties_body(workspace, cx),
        1 => super::effect_controls::body(workspace, cx),
        2 => super::color_grade::body(workspace, cx),
        _ => super::motion::body(workspace, cx),
    };
    panel(PanelId::Properties, &["Properties", "Effects", "Color", "Motion"], None, body, workspace, cx)
}
