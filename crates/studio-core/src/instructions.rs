//! Carrying out instructions typed to Iris ("start at 5s, last 2 seconds and fade in") on a
//! composition. The words are read by the shared `iris::intent`; this decides which layers they
//! are about and makes the changes with Studio's own edits (moving and trimming in time,
//! splitting, the In and Out animations of `presets`, text animators and loops), then says what
//! it did in plain words, including anything it couldn't do and why.
//!
//! Timing goes first and motion after, since a layer's In and Out animations sit at its ends.

use iris::intent::{self, Loop as SaidLoop, Motion as SaidMotion, Phase as SaidPhase, Side, Step, Target, Time};

use crate::model::{Composition, Id, LayerContent, Project};
use crate::presets::{self, Loop, Motion, Phase};
use crate::property::{PropertyPath, PropertyValue, TransformProperty};
use crate::text_animation::{TextAnimator, TextEase, TextMotion, TextOrder, TextUnit};

/// How long an In or Out animation takes when no length is said, in seconds.
const USUAL_SECONDS: f32 = 0.6;
/// How long text coming in a word, letter or line at a time takes when no length is said.
const USUAL_TEXT_SECONDS: f32 = 1.0;

/// What the app knows that the words may refer to.
#[derive(Clone, Copy, Debug)]
pub struct Context {
    /// The playhead's frame ("at the playhead"), where looks are set.
    pub playhead: i64,
    /// Whether the main track keeps its clips packed together (see `tracks`).
    pub magnetic_main_track: bool,
}

/// What was done, to report.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    /// The layers changed.
    pub changed: Vec<Id>,
    /// What happened to them, as phrases ("now starts at 5s", "fades in over 0.6s").
    pub done: Vec<String>,
    /// What couldn't be done and why, and anything else worth knowing, as sentences.
    pub problems: Vec<String>,
}

/// The layers `target` means in `composition`. Without one named, the selection, else the one
/// layer whose name the message mentions, else the only layer. `Err` says what to do instead.
pub fn layers_meant(composition: &Composition, target: &Target, selected: &[Id], message: &str) -> Result<Vec<Id>, String> {
    let existing = |ids: Vec<Id>| ids.into_iter().filter(|id| composition.layer(*id).is_some()).collect::<Vec<_>>();
    match target {
        Target::Everything => Ok(composition.layers.iter().filter(|layer| !layer.switches.locked).map(|layer| layer.id).collect()),
        Target::Number(number) => composition.layers.get(number - 1).map(|layer| vec![layer.id]).ok_or_else(|| format!("There's no layer {number}: {} has {}.", composition.name, plural(composition.layers.len(), "layer"))),
        Target::Named(name) => {
            let found = named(composition, name);
            if found.is_empty() { Err(format!("I couldn't find a layer called “{name}” in {}.", composition.name)) } else { Ok(found) }
        }
        Target::Kind(kind) => {
            let found: Vec<Id> = composition.layers.iter().filter(|layer| !layer.switches.locked && is_kind(layer, *kind)).map(|layer| layer.id).collect();
            if found.is_empty() { Err(format!("{} has no {} that aren't locked.", composition.name, kind.plural())) } else { Ok(found) }
        }
        Target::Selection | Target::Unnamed => {
            let chosen = existing(selected.to_vec());
            if !chosen.is_empty() {
                return Ok(chosen);
            }
            // A layer the message names without saying "layer": "make Big Sale fade in".
            if *target == Target::Unnamed {
                let lower = message.to_lowercase();
                let mentioned: Vec<Id> = composition.layers.iter().filter(|layer| layer.name.chars().count() >= 3 && lower.contains(&layer.name.to_lowercase())).map(|layer| layer.id).collect();
                if mentioned.len() == 1 {
                    return Ok(mentioned);
                }
            }
            match composition.layers.as_slice() {
                [only] => Ok(vec![only.id]),
                [] => Err(format!("{} has no layers yet.", composition.name)),
                _ => Err("Which layer? Select it first, or name it, as in “fade the title layer in”.".to_owned()),
            }
        }
    }
}

/// The same as `layers_meant`, with the locked layers that "everything" and a kind of layer
/// otherwise leave out. Unlocking is about exactly those.
pub fn layers_meant_locked_too(composition: &Composition, target: &Target, selected: &[Id], message: &str) -> Result<Vec<Id>, String> {
    let found: Vec<Id> = match target {
        Target::Everything => composition.layers.iter().map(|layer| layer.id).collect(),
        Target::Kind(kind) => composition.layers.iter().filter(|layer| is_kind(layer, *kind)).map(|layer| layer.id).collect(),
        _ => return layers_meant(composition, target, selected, message),
    };
    if found.is_empty() { Err(format!("{} has no layers like that.", composition.name)) } else { Ok(found) }
}

/// Whether a layer is of the kind a message named ("all text layers", "every image").
fn is_kind(layer: &crate::model::Layer, kind: intent::Kind) -> bool {
    match kind {
        intent::Kind::Text => matches!(layer.content, LayerContent::Text { .. }),
        intent::Kind::Picture => matches!(layer.content, LayerContent::Image { .. }),
        intent::Kind::Shape => matches!(layer.content, LayerContent::Shape { .. }),
        intent::Kind::Video => matches!(layer.content, LayerContent::Footage { .. }),
        intent::Kind::Sound => matches!(layer.content, LayerContent::Audio { .. }),
    }
}

/// A name as its plain words: lower case, with anything but letters and digits read as a
/// space, so "B-roll_02.mov" and "b roll 02" are the same.
fn plain_words(name: &str) -> Vec<String> {
    name.to_lowercase().split(|character: char| !character.is_alphanumeric()).filter(|word| !word.is_empty()).map(str::to_owned).collect()
}

/// Whether a layer's name is `words`, or holds them as whole words in a row. This is the test
/// for words that may not name a layer at all ("hide the rulers"), so it is stricter than
/// `named`: part of a word, or a word of a layer's text, doesn't count.
pub fn has_layer_called(composition: &Composition, words: &str) -> bool {
    let wanted = plain_words(words);
    !wanted.is_empty() && composition.layers.iter().any(|layer| plain_words(&layer.name).windows(wanted.len()).any(|run| run == wanted.as_slice()))
}

/// Which layers a message is about, as the shared instruction reader names them.
pub fn target_of(message: &str) -> Target {
    intent::target_of(message)
}

/// The layers whose name is `name`, or failing that contains it, or whose text does.
fn named(composition: &Composition, name: &str) -> Vec<Id> {
    let wanted = name.to_lowercase();
    let exact: Vec<Id> = composition.layers.iter().filter(|layer| layer.name.to_lowercase() == wanted).map(|layer| layer.id).collect();
    if !exact.is_empty() {
        return exact;
    }
    let containing: Vec<Id> = composition.layers.iter().filter(|layer| layer.name.to_lowercase().contains(&wanted)).map(|layer| layer.id).collect();
    if !containing.is_empty() {
        return containing;
    }
    // The same, whatever marks the name is written with ("the b-roll layer" for "B-roll_02").
    let plain = plain_words(name).join(" ");
    let containing: Vec<Id> = composition.layers.iter().filter(|layer| !plain.is_empty() && plain_words(&layer.name).join(" ").contains(&plain)).map(|layer| layer.id).collect();
    if !containing.is_empty() {
        return containing;
    }
    composition
        .layers
        .iter()
        .filter(|layer| matches!(&layer.content, LayerContent::Text { text, .. } if text.to_lowercase().contains(&wanted)))
        .map(|layer| layer.id)
        .collect()
}

/// Carries out `steps` on each of `layers` in composition `comp`.
pub fn carry_out(project: &mut Project, comp: Id, layers: &[Id], steps: &[Step], context: Context) -> Outcome {
    let mut outcome = Outcome::default();
    for &layer in layers {
        let mut done = Vec::new();
        let mut problems = Vec::new();
        retime(project, comp, layer, steps, context, &mut done, &mut problems);
        for step in steps {
            animate(project, comp, layer, *step, &mut done, &mut problems);
            set_look(project, comp, layer, *step, context.playhead, &mut done, &mut problems);
        }
        if !done.is_empty() {
            outcome.changed.push(layer);
        }
        // The phrases for the first layer stand for all of them; the steps are the same.
        if outcome.done.is_empty() {
            outcome.done = done;
        }
        for problem in problems {
            if !outcome.problems.contains(&problem) {
                outcome.problems.push(problem);
            }
        }
    }
    outcome
}

/// A time said, as a frame of composition `composition`.
fn frame_of(time: Time, composition: &Composition, playhead: i64) -> i64 {
    match time {
        Time::Span { millis, frames } => (millis as f64 / 1000.0 * composition.frame_rate.fps()).round() as i64 + frames,
        Time::Playhead => playhead,
        Time::End => composition.duration_frames,
    }
}

/// `frames` as a short time: "5s", "2.5s", "1:05".
fn seconds_text(frames: i64, composition: &Composition) -> String {
    let seconds = frames as f64 / composition.frame_rate.fps();
    if seconds >= 60.0 {
        let minutes = (seconds / 60.0).floor();
        return format!("{}:{:05.2}", minutes as i64, seconds - minutes * 60.0).trim_end_matches('0').trim_end_matches('.').to_owned();
    }
    let rounded = (seconds * 100.0).round() / 100.0;
    format!("{rounded}s")
}

/// The timing steps: moving, trimming, extending and cutting, in the order said. In and Out
/// animations already on the layer are taken off first and put back after, so they follow its
/// new ends.
fn retime(project: &mut Project, comp: Id, layer_id: Id, steps: &[Step], context: Context, done: &mut Vec<String>, problems: &mut Vec<String>) {
    let timing: Vec<Step> = steps.iter().copied().filter(|step| matches!(step, Step::StartAt(_) | Step::TrimStart(_) | Step::EndAt(_) | Step::Last(_) | Step::Shift { .. } | Step::CutAt(_))).collect();
    let Some(composition) = project.composition(comp) else { return };
    let Some(layer) = composition.layer(layer_id) else { return };
    if timing.is_empty() {
        return;
    }
    let name = layer.name.clone();
    if layer.switches.locked {
        problems.push(format!("“{name}” is locked, so I left its timing alone."));
        return;
    }
    let (before_in, before_out) = (layer.in_frame, layer.out_frame);
    let (enter, exit) = (layer.animation.enter, layer.animation.exit);
    for (phase, kept) in [(Phase::In, enter), (Phase::Out, exit)] {
        if kept.is_some() {
            presets::set_motion(project, comp, layer_id, phase, None, USUAL_SECONDS);
        }
    }
    let mut moved_off_main = false;
    let mut cut_at = None;
    for step in timing {
        let Some(composition) = project.composition(comp) else { return };
        let Some(layer) = composition.layer(layer_id) else { return };
        let duration = composition.duration_frames;
        let (in_frame, out_frame, track) = (layer.in_frame, layer.out_frame, layer.track);
        match step {
            Step::StartAt(_) | Step::Shift { .. } => {
                let wanted = match step {
                    Step::StartAt(time) => frame_of(time, composition, context.playhead),
                    Step::Shift { by, later } => in_frame + if later { 1 } else { -1 } * frame_of(by, composition, context.playhead),
                    _ => unreachable!("matched above"),
                };
                if wanted >= duration {
                    problems.push(format!("{} ends at {}, so “{name}” can't start at {}.", composition.name, seconds_text(duration, composition), seconds_text(wanted, composition)));
                    continue;
                }
                let wanted = wanted.max(0);
                // A clip on a packed main track would be pulled back into line: it gets a track above.
                let on_packed_main = track == crate::tracks::MAIN_TRACK && context.magnetic_main_track && wanted != in_frame;
                let to_track = if on_packed_main { crate::tracks::MAIN_TRACK + 1 } else { track };
                moved_off_main |= on_packed_main;
                if let Some(composition) = project.composition_mut(comp) {
                    crate::tracks::place_clip(composition, layer_id, to_track, wanted);
                }
            }
            Step::TrimStart(time) => {
                let wanted = frame_of(time, composition, context.playhead);
                if wanted >= out_frame {
                    problems.push(format!("“{name}” ends at {}, so its start can't move to {}.", seconds_text(out_frame, composition), seconds_text(wanted, composition)));
                    continue;
                }
                // Footage can't start before its first frame, nor run into the clip before it.
                crate::trim::trim_in(project, comp, layer_id, wanted.max(0));
                let reached = project.composition(comp).and_then(|composition| composition.layer(layer_id)).map_or(wanted, |layer| layer.in_frame);
                if reached != wanted.max(0)
                    && let Some(composition) = project.composition(comp)
                {
                    problems.push(format!("“{name}” could only start from {} (its footage or the clip before it is in the way).", seconds_text(reached, composition)));
                }
            }
            Step::Last(length) => {
                let frames = frame_of(length, composition, context.playhead).max(1);
                set_out(project, comp, layer_id, in_frame + frames, &name, problems);
            }
            Step::EndAt(time) => {
                let wanted = frame_of(time, composition, context.playhead);
                if wanted <= in_frame {
                    problems.push(format!("“{name}” starts at {}, so it can't end at {}.", seconds_text(in_frame, composition), seconds_text(wanted, composition)));
                    continue;
                }
                set_out(project, comp, layer_id, wanted, &name, problems);
            }
            Step::CutAt(time) => {
                let at = frame_of(time, composition, context.playhead);
                if at <= in_frame || at >= out_frame {
                    problems.push(format!("“{name}” runs from {} to {}, so there's nothing to cut at {}.", seconds_text(in_frame, composition), seconds_text(out_frame, composition), seconds_text(at, composition)));
                    continue;
                }
                if !crate::cut::split_layers(project, comp, &[layer_id], at).created.is_empty() {
                    cut_at = Some(at);
                }
            }
            _ => {}
        }
    }
    // The animations it had, back at its new ends.
    for (phase, kept) in [(Phase::In, enter), (Phase::Out, exit)] {
        if let Some((motion, seconds)) = kept {
            presets::set_motion(project, comp, layer_id, phase, Some(motion), seconds);
        }
    }
    let Some(composition) = project.composition(comp) else { return };
    let Some(layer) = composition.layer(layer_id) else { return };
    let (in_frame, out_frame) = (layer.in_frame, layer.out_frame);
    if let Some(at) = cut_at {
        done.push(format!("is cut in two at {} (the second half is the layer above it)", seconds_text(at, composition)));
    }
    if in_frame != before_in && out_frame - in_frame == before_out - before_in && cut_at.is_none() {
        done.push(format!("now starts at {} (until {})", seconds_text(in_frame, composition), seconds_text(out_frame, composition)));
    } else if in_frame != before_in {
        done.push(format!("now starts at {} and lasts {} (until {})", seconds_text(in_frame, composition), seconds_text(out_frame - in_frame, composition), seconds_text(out_frame, composition)));
    } else if out_frame != before_out && cut_at.is_none() {
        done.push(format!("now lasts {} (until {})", seconds_text(out_frame - in_frame, composition), seconds_text(out_frame, composition)));
    }
    if moved_off_main {
        problems.push(format!("I put “{name}” on a track of its own, so the main track's clips stay packed together."));
    }
}

/// Moves the layer's end to `wanted`, as far as its source, the composition and the other clips
/// on its track allow; a clip in the way sends it to a track of its own.
fn set_out(project: &mut Project, comp: Id, layer_id: Id, wanted: i64, name: &str, problems: &mut Vec<String>) {
    let Some(composition) = project.composition(comp) else { return };
    let Some(layer) = composition.layer(layer_id) else { return };
    let mut out = wanted.min(composition.duration_frames);
    if wanted > composition.duration_frames {
        problems.push(format!("{} ends at {}, so “{name}” stops there.", composition.name, seconds_text(composition.duration_frames, composition)));
    }
    // Footage and sound can't run past the end of their source.
    if let Some(length) = crate::trim::source_frames(project, composition, layer) {
        let last = layer.start_frame + length;
        if out > last {
            problems.push(format!("“{name}” runs out of footage at {}, so it ends there.", seconds_text(last, composition)));
            out = last;
        }
    }
    let (track, in_frame) = (layer.track, layer.in_frame);
    let Some(composition) = project.composition_mut(comp) else { return };
    if let Some(layer) = composition.layer_mut(layer_id) {
        layer.out_frame = out.max(in_frame + 1);
    }
    // Longer now, it may run into the next clip on its track: then it moves to one of its own.
    crate::tracks::place_clip(composition, layer_id, track, in_frame);
}

/// Presets' motion for a way in or out from a side. Studio names slides by the way they move:
/// Slide Right comes in from the left (and goes out to the right).
fn preset_motion(motion: SaidMotion, phase: Phase) -> Option<Motion> {
    Some(match (motion, phase) {
        (SaidMotion::Fade, _) => Motion::Fade,
        (SaidMotion::Slide(Side::Left), Phase::In) | (SaidMotion::Slide(Side::Right), Phase::Out) => Motion::SlideRight,
        (SaidMotion::Slide(Side::Right), Phase::In) | (SaidMotion::Slide(Side::Left), Phase::Out) => Motion::SlideLeft,
        (SaidMotion::Slide(Side::Bottom), _) => Motion::Rise,
        (SaidMotion::Slide(Side::Top), _) => Motion::Drop,
        (SaidMotion::Pop, _) => Motion::Pop,
        (SaidMotion::Zoom, _) => Motion::Zoom,
        (SaidMotion::Spin, _) => Motion::Spin,
        (SaidMotion::Blur, _) => Motion::Blur,
        (SaidMotion::Bounce, _) => Motion::Bounce,
        (SaidMotion::Typewriter, _) => Motion::Typewriter,
        _ => return None,
    })
}

/// How a motion reads in a sentence: "fades in", "slides in from the left".
fn motion_words(motion: SaidMotion, phase: Phase) -> String {
    let way = if phase == Phase::In { "in" } else { "out" };
    let side = |side: Side| match side {
        Side::Left => "the left",
        Side::Right => "the right",
        Side::Top => "the top",
        Side::Bottom => "the bottom",
    };
    match motion {
        SaidMotion::Fade => format!("fades {way}"),
        SaidMotion::Slide(at) if phase == Phase::In => format!("slides in from {}", side(at)),
        SaidMotion::Slide(at) => format!("slides out to {}", side(at)),
        SaidMotion::Pop => format!("pops {way}"),
        SaidMotion::Zoom => format!("zooms {way}"),
        SaidMotion::Spin => format!("spins {way}"),
        SaidMotion::Blur => format!("blurs {way}"),
        SaidMotion::Bounce => format!("bounces {way}"),
        SaidMotion::Typewriter if phase == Phase::In => "types on".to_owned(),
        SaidMotion::Typewriter => "is erased as if typed backwards".to_owned(),
        SaidMotion::ByWord => format!("comes {way} word by word").replace("comes out", "goes out"),
        SaidMotion::ByLetter => format!("comes {way} letter by letter").replace("comes out", "goes out"),
        SaidMotion::ByLine => format!("comes {way} line by line").replace("comes out", "goes out"),
        SaidMotion::Suited => format!("animates {way}"),
    }
}

/// The motion that suits a layer when none is named ("animate it in"): text rises in a word at
/// a time; pictures and video zoom; small shapes pop; anything else fades.
fn suited(composition: &Composition, layer_id: Id, phase: Phase) -> SaidMotion {
    let Some(layer) = composition.layer(layer_id) else { return SaidMotion::Fade };
    let frame = (composition.width as f32 * composition.height as f32).max(1.0);
    match &layer.content {
        LayerContent::Text { .. } if phase == Phase::In => SaidMotion::ByWord,
        LayerContent::Image { .. } | LayerContent::Footage { .. } | LayerContent::Composition { .. } => SaidMotion::Zoom,
        LayerContent::Shape { .. } if layer.size.x * layer.size.y < frame * 0.06 => SaidMotion::Pop,
        _ => SaidMotion::Fade,
    }
}

/// The motion steps: ways in and out, loops, and taking them away.
fn animate(project: &mut Project, comp: Id, layer_id: Id, step: Step, done: &mut Vec<String>, problems: &mut Vec<String>) {
    let Some(composition) = project.composition(comp) else { return };
    let Some(layer) = composition.layer(layer_id) else { return };
    let name = layer.name.clone();
    let is_text = matches!(layer.content, LayerContent::Text { .. });
    let is_sound = matches!(layer.content, LayerContent::Audio { .. });
    let locked = layer.switches.locked;
    let fps = composition.frame_rate.fps();
    let phase_of = |phase: SaidPhase| if phase == SaidPhase::In { Phase::In } else { Phase::Out };
    match step {
        Step::Animate { phase, motion, millis } => {
            if locked {
                problems.push(format!("“{name}” is locked, so I didn't animate it."));
                return;
            }
            let phase = phase_of(phase);
            let motion = if motion == SaidMotion::Suited { suited(composition, layer_id, phase) } else { motion };
            let asked = millis.map(|millis| (millis as f32 / 1000.0).clamp(0.1, presets::MAX_SECONDS));
            let span = layer.out_frame - layer.in_frame;
            // Sound has no picture to move, but a fade is something it can do: its volume rises
            // from silence, or falls to it.
            if is_sound && matches!(motion, SaidMotion::Fade) {
                let seconds = asked.unwrap_or(USUAL_SECONDS);
                if let Some(composition) = project.composition_mut(comp) {
                    crate::edit::set_layer_audio(composition, layer_id, |audio| if phase == Phase::In { audio.fade_in = seconds } else { audio.fade_out = seconds });
                }
                done.push(format!("fades its sound {} over {}s", if phase == Phase::In { "in" } else { "out" }, (seconds * 100.0).round() / 100.0));
                return;
            }
            // Text a piece at a time is a text animator; other layers fade instead.
            let unit = match motion {
                SaidMotion::ByWord => Some(TextUnit::Word),
                SaidMotion::ByLetter => Some(TextUnit::Character),
                SaidMotion::ByLine => Some(TextUnit::Line),
                _ => None,
            };
            let (made, motion, seconds) = match unit {
                Some(unit) if is_text => {
                    let seconds = asked.unwrap_or(USUAL_TEXT_SECONDS);
                    let animator = TextAnimator { motion: TextMotion::Rise, unit, order: TextOrder::Forward, ease: TextEase::Smooth, seconds, overlap: 0.6 };
                    // A whole-layer motion at the same end would fight it, so it goes.
                    presets::set_motion(project, comp, layer_id, phase, None, USUAL_SECONDS);
                    (crate::text_animation::set_animator(project, comp, layer_id, phase, Some(animator)), motion, seconds)
                }
                _ => {
                    let motion = if unit.is_some() {
                        problems.push(format!("“{name}” isn't text, so it fades instead of coming a piece at a time."));
                        SaidMotion::Fade
                    } else {
                        motion
                    };
                    let seconds = asked.unwrap_or(USUAL_SECONDS);
                    let preset = preset_motion(motion, phase).expect("every motion but text pieces has a preset");
                    if is_text {
                        crate::text_animation::set_animator(project, comp, layer_id, phase, None);
                    }
                    (presets::set_motion(project, comp, layer_id, phase, Some(preset), seconds), motion, seconds)
                }
            };
            if made {
                let mut phrase = format!("{} over {}s", motion_words(motion, phase), (seconds * 100.0).round() / 100.0);
                if (seconds as f64 * fps).round() as i64 * 2 > span {
                    phrase.push_str(" (that's more than half its length)");
                }
                done.push(phrase);
            } else {
                problems.push(format!("“{name}” can't be animated that way (sound has no picture to move)."));
            }
        }
        Step::RemoveMotion(phase) => {
            let phases: Vec<Phase> = match phase {
                Some(phase) => vec![phase_of(phase)],
                None => vec![Phase::In, Phase::Out],
            };
            let mut removed = false;
            for phase in phases {
                removed |= presets::set_motion(project, comp, layer_id, phase, None, USUAL_SECONDS);
                removed |= crate::text_animation::set_animator(project, comp, layer_id, phase, None);
            }
            if removed {
                done.push(match phase {
                    Some(SaidPhase::In) => "no longer animates in".to_owned(),
                    Some(SaidPhase::Out) => "no longer animates out".to_owned(),
                    None => "no longer animates in or out".to_owned(),
                });
            } else {
                problems.push(format!("“{name}” had no In or Out animation to take away."));
            }
        }
        Step::Loop(looping) => {
            let (preset, words) = match looping {
                SaidLoop::Pulse => (Loop::Pulse, "pulses"),
                SaidLoop::Float => (Loop::Float, "floats"),
                SaidLoop::Swing => (Loop::Swing, "swings"),
                SaidLoop::Blink => (Loop::Blink, "blinks"),
                SaidLoop::Turn => (Loop::Turn, "keeps turning"),
                SaidLoop::Wiggle => (Loop::Wiggle, "wiggles"),
            };
            if presets::set_loop(project, comp, layer_id, Some(preset)) {
                done.push(format!("{words} between coming in and going out"));
            } else {
                problems.push(format!("“{name}” can't loop (it's locked, or sound)."));
            }
        }
        Step::RemoveLoop => {
            if presets::set_loop(project, comp, layer_id, None) {
                done.push("no longer loops".to_owned());
            } else {
                problems.push(format!("“{name}” had no loop to take away."));
            }
        }
        _ => {}
    }
}

/// The look steps, set at the playhead: opacity, size, turn and position. A property already
/// animated gets a keyframe there instead of changing throughout.
fn set_look(project: &mut Project, comp: Id, layer_id: Id, step: Step, playhead: i64, done: &mut Vec<String>, problems: &mut Vec<String>) {
    let (property, phrase) = match step {
        Step::Opacity(percent) => (TransformProperty::Opacity, format!("is at {percent}% opacity")),
        Step::Scale(percent) => (TransformProperty::Scale, format!("is at {percent}% size")),
        Step::Rotate(degrees) => (TransformProperty::Rotation, format!("is turned {degrees}°")),
        Step::Nudge { x, y } => {
            let way = match (x.signum(), y.signum()) {
                (-1, _) => format!("{} px left", -x),
                (1, _) => format!("{x} px right"),
                (_, -1) => format!("{} px up", -y),
                _ => format!("{y} px down"),
            };
            (TransformProperty::Position, format!("moved {way}"))
        }
        _ => return,
    };
    let Some(composition) = project.composition_mut(comp) else { return };
    let Some(layer) = composition.layer_mut(layer_id) else { return };
    if layer.switches.locked || layer.transform.is_none() {
        problems.push(format!("“{}” can't change its {} (it's locked, or sound).", layer.name, phrase.split_whitespace().last().unwrap_or("look")));
        return;
    }
    let path = PropertyPath::Transform(property);
    let Some(now) = layer.property(path, playhead) else { return };
    let animated = !now.keyframes.is_empty();
    let value = match (step, now.value) {
        (Step::Opacity(percent), _) => PropertyValue::Scalar(percent as f32),
        (Step::Scale(percent), _) => PropertyValue::Pair(crate::animation::Vec2::new(percent as f32, percent as f32)),
        (Step::Rotate(degrees), PropertyValue::Scalar(current)) => PropertyValue::Scalar(current + degrees as f32),
        (Step::Nudge { x, y }, PropertyValue::Pair(current)) => PropertyValue::Pair(current + crate::animation::Vec2::new(x as f32, y as f32)),
        _ => return,
    };
    if layer.set_property(path, playhead, value) {
        done.push(if animated { format!("{phrase} (a keyframe at the playhead, since it was already animated)") } else { phrase });
    }
}

/// "3 layers", "1 layer".
fn plural(count: usize, word: &str) -> String {
    if count == 1 { format!("1 {word}") } else { format!("{count} {word}s") }
}

/// A typed message read as instructions Studio can carry out, if it is one.
pub fn read(message: &str) -> Option<intent::Request> {
    intent::parse(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Vec2;
    use crate::edit;
    use crate::time::FrameRate;

    /// A 10-second, 30 fps composition with a title (text) and a full-frame panel (shape),
    /// both from 0 to the end; the title is selected.
    fn scene() -> (Project, Id, Id, Id) {
        let mut project = Project::new(Vec::new(), Vec::new());
        let settings = edit::CompositionSettings { name: "Promo".into(), width: 1920, height: 1080, frame_rate: FrameRate::new(30, 1), duration_frames: 300, background: crate::model::Rgba::hex(0x000000) };
        let comp = edit::new_composition(&mut project, &settings).unwrap();
        let style = crate::model::ShapeStyle::new(crate::model::ShapeKind::Rectangle);
        let panel = edit::add_shape_layer(&mut project, comp, style, Vec2::new(1920.0, 1080.0), Vec2::new(960.0, 540.0), 0).unwrap();
        let title = edit::add_text_layer(&mut project, comp, "Big Sale", Vec2::new(960.0, 540.0), 0).unwrap();
        project.composition_mut(comp).unwrap().layer_mut(title).unwrap().name = "Title".into();
        (project, comp, title, panel)
    }

    fn context() -> Context {
        Context { playhead: 90, magnetic_main_track: false }
    }

    fn run(project: &mut Project, comp: Id, selected: &[Id], message: &str) -> Result<Outcome, String> {
        let request = read(message).ok_or("not read as an instruction")?;
        let composition = project.composition(comp).unwrap();
        let layers = layers_meant(composition, &request.target, selected, message)?;
        Ok(carry_out(project, comp, &layers, &request.steps, context()))
    }

    fn span(project: &Project, comp: Id, layer: Id) -> (i64, i64) {
        let layer = project.composition(comp).unwrap().layer(layer).unwrap();
        (layer.in_frame, layer.out_frame)
    }

    #[test]
    fn start_at_five_seconds_for_two_with_a_fade_in() {
        let (mut project, comp, title, _) = scene();
        let outcome = run(&mut project, comp, &[title], "I want this layer to start at 5s and last for 2 seconds. I want it to fade in.").unwrap();
        assert_eq!(span(&project, comp, title), (150, 210));
        let layer = project.composition(comp).unwrap().layer(title).unwrap();
        assert_eq!(layer.animation.enter, Some((Motion::Fade, USUAL_SECONDS)));
        assert_eq!(outcome.done, vec!["now starts at 5s and lasts 2s (until 7s)".to_owned(), "fades in over 0.6s".to_owned()]);
        assert!(outcome.problems.is_empty(), "{:?}", outcome.problems);
    }

    #[test]
    fn a_way_out_follows_the_layer_when_it_is_shortened() {
        let (mut project, comp, title, _) = scene();
        run(&mut project, comp, &[title], "fade out over 1 second").unwrap();
        run(&mut project, comp, &[title], "end at 6s").unwrap();
        let layer = project.composition(comp).unwrap().layer(title).unwrap();
        assert_eq!(layer.out_frame, 180);
        // The fade now finishes at the new end: fully gone on the last frame, fully there a second before.
        let opacity = |frame: i64| layer.property(PropertyPath::Transform(TransformProperty::Opacity), frame).map(|snapshot| snapshot.value.component(0)).unwrap();
        assert!(opacity(150) > 99.0 && opacity(180) < 1.0, "{} {}", opacity(150), opacity(180));
    }

    #[test]
    fn layers_are_found_by_name_number_or_mention() {
        let (mut project, comp, title, panel) = scene();
        assert!(run(&mut project, comp, &[], "slide the title layer in from the left").is_ok());
        assert_eq!(project.composition(comp).unwrap().layer(title).unwrap().animation.enter.map(|(motion, _)| motion), Some(Motion::SlideRight), "comes in from the left");
        // Layer 2 counts from the top: the panel is under the title.
        run(&mut project, comp, &[], "make layer 2 start at 1s").unwrap();
        assert_eq!(span(&project, comp, panel).0, 30);
        // The name mentioned without "layer", with nothing selected.
        run(&mut project, comp, &[], "make Title pulse").unwrap();
        assert!(project.composition(comp).unwrap().layer(title).unwrap().animation.looping.is_some());
        // Nothing selected or named, with two layers: which one is asked.
        assert!(run(&mut project, comp, &[], "fade in").unwrap_err().starts_with("Which layer?"));
        assert!(run(&mut project, comp, &[], "fade “Nope” in").unwrap_err().contains("couldn't find"));
    }

    #[test]
    fn cutting_text_by_word_and_honest_problems() {
        let (mut project, comp, title, panel) = scene();
        let outcome = run(&mut project, comp, &[title], "cut it at the playhead").unwrap();
        assert_eq!(span(&project, comp, title), (0, 90));
        assert_eq!(project.composition(comp).unwrap().layers.len(), 3, "the second half is a layer of its own");
        assert!(outcome.done[0].starts_with("is cut in two at 3s"), "{:?}", outcome.done);
        // Word by word on text uses a text animator; on a shape it fades, and says so.
        run(&mut project, comp, &[title], "bring it in word by word").unwrap();
        assert!(project.composition(comp).unwrap().layer(title).unwrap().animation.text_in.is_some());
        let outcome = run(&mut project, comp, &[panel], "bring it in word by word").unwrap();
        assert!(outcome.problems[0].contains("isn't text"), "{:?}", outcome.problems);
        // Starting past the end can't be done, and says why.
        let outcome = run(&mut project, comp, &[panel], "start at 20s").unwrap();
        assert!(outcome.problems[0].contains("ends at 10s"), "{:?}", outcome.problems);
    }

    #[test]
    fn trimming_the_start_keeps_the_end() {
        let (mut project, comp, title, _) = scene();
        let outcome = run(&mut project, comp, &[title], "trim the start to 2s").unwrap();
        assert_eq!(span(&project, comp, title), (60, 300));
        assert_eq!(outcome.done, vec!["now starts at 2s and lasts 8s (until 10s)".to_owned()]);
    }

    #[test]
    fn looks_change_at_the_playhead() {
        let (mut project, comp, title, _) = scene();
        let outcome = run(&mut project, comp, &[title], "set it to 50% opacity and rotate it 15 degrees").unwrap();
        let layer = project.composition(comp).unwrap().layer(title).unwrap();
        let value = |property| layer.property(PropertyPath::Transform(property), 0).unwrap().value.component(0);
        assert_eq!((value(TransformProperty::Opacity), value(TransformProperty::Rotation)), (50.0, 15.0));
        assert_eq!(outcome.done, vec!["is at 50% opacity".to_owned(), "is turned 15°".to_owned()]);
    }
}
