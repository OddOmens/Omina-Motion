//! Carrying out Motion's own chores typed to Iris: moving the playhead and playing, the
//! composition's settings and work area, arranging layers in time, sound, markers, making
//! layers, speed, and the check before exporting. The words are read by
//! `studio_core::chores`; this makes each change with the editor's own edits, so it behaves
//! exactly as the menus and panels do, and says what it did in plain words. A chore that
//! can't be done says why instead, and nothing is reported that didn't happen.
//!
//! A chore whose tool is switched off in Settings ▸ Tools & Features is explained, not done.

use std::collections::HashSet;
use std::path::Path;

use gpui_kit::{Context, Window};
use iris::intent::{Step, Target, Time};
use studio_core::assistant::{Fix, Suggestion};
use studio_core::chat::{listed, plural, spoken};
use studio_core::chores::{Chore, Ends, Export, Format, Jump, Level, Made, Pace, Pick, Request, Span};
use studio_core::instructions::layers_meant;
use studio_core::model::{Id, Layer, LayerAudio, LayerContent, LayerTime, Marker, Paint, ProjectItemKind, Rgba, ShapeStyle};
use studio_core::time::{FrameRange, format_timecode};

use crate::app_settings::AppSettings;
use crate::features::Feature;
use crate::state::{AudioSetting, EditorState};
use crate::workspace::Workspace;

/// What became of something Iris was asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The project changed, in a way Undo takes back.
    Changed,
    /// Something was done that isn't a change to the project: the playhead moved, the
    /// selection changed, a command was run.
    Did,
    /// It was answered in words, and there was nothing to do.
    Said,
    /// It was understood, but couldn't be done; the words say why.
    Failed,
    /// It wasn't understood.
    Unknown,
}

/// What Iris did about a request, to say and to remember.
#[derive(Clone, Debug, PartialEq)]
pub struct Done {
    /// What she says about it.
    pub said: String,
    pub outcome: Outcome,
    /// The layers made or changed, which "it" means next when nothing is selected.
    pub layers: Vec<Id>,
    /// Short replies offered under the answer, beyond "Undo that".
    pub replies: Vec<String>,
}

impl Done {
    pub fn changed(said: impl Into<String>, layers: Vec<Id>) -> Self {
        Self { said: said.into(), outcome: Outcome::Changed, layers, replies: Vec::new() }
    }

    pub fn did(said: impl Into<String>) -> Self {
        Self { said: said.into(), outcome: Outcome::Did, layers: Vec::new(), replies: Vec::new() }
    }

    pub fn said(said: impl Into<String>) -> Self {
        Self { said: said.into(), outcome: Outcome::Said, layers: Vec::new(), replies: Vec::new() }
    }

    pub fn failed(said: impl Into<String>) -> Self {
        Self { said: said.into(), outcome: Outcome::Failed, layers: Vec::new(), replies: Vec::new() }
    }

    /// The same, with short replies to offer under it.
    pub fn offering(mut self, replies: &[&str]) -> Self {
        self.replies = replies.iter().map(|reply| (*reply).to_owned()).collect();
        self
    }
}

/// What "it" can mean beyond the selection.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pointed<'a> {
    /// The layers the step before this one, in the same message, named or changed. They come
    /// ahead of the selection, so "start the title at 5s and fade in" fades the title whatever
    /// is selected.
    pub carried: &'a [Id],
    /// The layers Iris last made or changed, for when nothing is selected.
    pub last: &'a [Id],
}

/// What a chore may need to know beyond the project.
pub struct Around<'a> {
    pub settings: &'a AppSettings,
    /// What Iris's rules notice in the open composition now.
    pub suggestions: &'a [Suggestion],
    /// What "it" can mean beyond the selection.
    pub pointed: Pointed<'a>,
    /// Whether the export is set to the work area rather than the whole composition.
    pub work_area_only: bool,
}

/// How long a sound fade is when no length is said, in seconds.
const USUAL_FADE_SECONDS: f32 = 1.0;
/// The longest a composition is made by asking, in hours.
const LONGEST_HOURS: f64 = 6.0;
/// The most characters a text layer is made with.
const MOST_TEXT: usize = 500;
/// The most names a reply spells out.
const MOST_NAMED: usize = 6;

/// A feature's name as Settings lists it.
fn feature_name(feature: Feature) -> &'static str {
    crate::features::GROUPS.iter().flat_map(|(_, features)| features.iter()).find(|info| info.feature == feature).map_or("That", |info| info.name)
}

/// What Iris says instead of doing a chore whose feature is switched off.
pub fn turned_off(feature: Feature) -> Done {
    Done::failed(format!("{} is turned off in Settings ▸ Tools & Features. Turn it on and ask me again.", feature_name(feature)))
}

/// A moment said, as a frame of the open composition.
fn frame_at(time: Time, state: &EditorState) -> i64 {
    let composition = state.composition();
    match time {
        Time::Span { millis, frames } => (millis as f64 / 1000.0 * composition.frame_rate.fps()).round() as i64 + frames,
        Time::Playhead => state.current_frame,
        Time::End => composition.duration_frames,
    }
}

fn timecode(state: &EditorState, frame: i64) -> String {
    format_timecode(frame, state.composition().frame_rate)
}

/// A length in frames as people say it: "2.5 seconds".
fn length_words(state: &EditorState, frames: i64) -> String {
    spoken(frames as f64 / state.composition().frame_rate.fps().max(1.0))
}

/// Names in quotes, as a sentence lists them, or a count when there are many.
pub fn quoted_names(names: &[String]) -> String {
    let names: Vec<String> = names.iter().map(|name| format!("“{name}”")).collect();
    match names.as_slice() {
        [] => "nothing".to_owned(),
        [one] => one.clone(),
        [first, second] => format!("{first} and {second}"),
        [first, second, third] => format!("{first}, {second} and {third}"),
        _ => format!("{} layers", names.len()),
    }
}

/// Layers' names in quotes, as a sentence lists them, or a count when there are many.
pub fn named(state: &EditorState, layers: &[Id]) -> String {
    let names: Vec<String> = layers.iter().filter_map(|id| state.composition().layer(*id)).map(|layer| layer.name.clone()).collect();
    quoted_names(&names)
}

/// The layers a request is about (see `layers_meant`). A request that names none means the
/// layers the step before it in the same message was about, then the selection, and with
/// nothing selected what Iris last made or changed.
pub fn layers_of(state: &EditorState, pointed: Pointed, target: &Target, message: &str) -> Result<Vec<Id>, String> {
    let composition = state.composition();
    let still_there = |layers: &[Id]| layers.iter().copied().filter(|id| composition.layer(*id).is_some()).collect::<Vec<Id>>();
    let names_none = matches!(target, Target::Selection | Target::Unnamed);
    let carried = still_there(pointed.carried);
    if names_none && !carried.is_empty() {
        // A layer the step mentions by name ("fade Big Sale in") is still the one meant.
        let mentioned = if *target == Target::Unnamed && composition.layers.len() > 1 { layers_meant(composition, target, &[], message).ok() } else { None };
        return Ok(mentioned.unwrap_or(carried));
    }
    let found = layers_meant(composition, target, &state.selected_layers, message);
    if found.is_ok() || !state.selected_layers.is_empty() || !names_none {
        return found;
    }
    let last = still_there(pointed.last);
    if last.is_empty() { found } else { Ok(last) }
}

/// The layers an arranging chore is about, in the order to arrange them: the selection in the
/// order it was made, otherwise the layers named, in the order they play. Arranging takes at
/// least two.
pub fn arranged(state: &EditorState, pointed: Pointed, target: &Target, message: &str) -> Result<Vec<Id>, String> {
    let composition = state.composition();
    let named = matches!(target, Target::Named(_) | Target::Kind(_) | Target::Number(_));
    let carried: Vec<Id> = pointed.carried.iter().copied().filter(|id| composition.layer(*id).is_some()).collect();
    // "Line them all up": with several just named or selected, "all" means all of those.
    let layers = if carried.len() >= 2 && !named {
        carried
    } else if state.selected_layers.len() >= 2 && !named {
        state.selected_layers.clone()
    } else if named || *target == Target::Everything {
        let mut found = layers_of(state, pointed, target, message)?;
        found.sort_by_key(|id| composition.layer(*id).map_or(0, |layer| layer.in_frame));
        found
    } else {
        Vec::new()
    };
    if layers.len() < 2 {
        return Err("Select two or more layers first, in the order they should play, or say which: “all text layers”, “everything”.".to_owned());
    }
    Ok(layers)
}

/// The layers with sound a sound chore is about. With none selected or named, the only layer
/// with sound is meant.
fn sound_layers(state: &EditorState, pointed: Pointed, target: &Target, message: &str) -> Result<Vec<Id>, String> {
    let has_sound = |id: &Id| state.has_sound(*id);
    let all: Vec<Id> = state.composition().layers.iter().map(|layer| layer.id).filter(has_sound).collect();
    if all.is_empty() {
        return Err(format!("Nothing in {} has sound.", state.composition().name));
    }
    let meant = match target {
        Target::Everything => all,
        Target::Selection | Target::Unnamed => match layers_of(state, pointed, target, message) {
            Ok(found) => found,
            Err(_) if all.len() == 1 => all,
            Err(_) => return Err(format!("Which one? {} have sound: {}. Select one, or name it.", plural(all.len(), "layer", "layers"), named(state, &all))),
        },
        _ => layers_of(state, pointed, target, message)?,
    };
    let sounding: Vec<Id> = meant.iter().copied().filter(has_sound).collect();
    if sounding.is_empty() {
        return Err(format!("{} {} no sound.", named(state, &meant), if meant.len() == 1 { "has" } else { "have" }));
    }
    Ok(sounding)
}

/// Every keyframe's frame on `layers`, in order.
fn keyframe_frames(state: &EditorState, layers: &[Id]) -> Vec<i64> {
    let mut frames: Vec<i64> = layers
        .iter()
        .filter_map(|id| state.composition().layer(*id))
        .flat_map(|layer| layer.property_paths().into_iter().flat_map(|path| layer.keyframe_easings(path).into_iter().map(|(frame, _)| frame)).collect::<Vec<_>>())
        .collect();
    frames.sort_unstable();
    frames.dedup();
    frames
}

/// Moves the playhead to `frame`, kept inside the composition, and says where it is.
fn go_to(state: &mut EditorState, frame: i64) -> Done {
    let last = state.composition().duration_frames - 1;
    state.set_current_frame(frame);
    let at = state.current_frame;
    let note = if frame > last {
        " That is the last frame: the composition ends there."
    } else if frame < 0 {
        " That is the first frame."
    } else {
        ""
    };
    Done::did(format!("The playhead is at {} (frame {at}).{note}", timecode(state, at)))
}

/// Carries out a jump of the playhead.
fn jump(state: &mut EditorState, jump: &Jump, request: &Request, message: &str, around: &Around) -> Done {
    let now = state.current_frame;
    match jump {
        Jump::LayerEdge { start } => {
            let layers = match layers_of(state, around.pointed, &request.target, message) {
                Ok(layers) => layers,
                Err(why) => return Done::failed(why),
            };
            let spans = || layers.iter().filter_map(|id| state.composition().layer(*id));
            // The end is the last frame it shows on.
            let Some(frame) = (if *start { spans().map(|layer| layer.in_frame).min() } else { spans().map(|layer| layer.out_frame - 1).max() }) else {
                return Done::failed("That layer is gone.");
            };
            let who = named(state, &layers);
            let mut done = go_to(state, frame);
            done.said.push_str(&format!(" That is where {who} {}.", if *start { "starts" } else { "ends" }));
            done.layers = layers;
            done
        }
        Jump::WorkAreaEdge { start } => {
            let frame = if *start { state.work_area.start.round() as i64 } else { state.work_area.end.round() as i64 - 1 };
            go_to(state, frame)
        }
        Jump::To(time) => {
            // "The end" is the last frame that shows.
            let frame = if *time == Time::End { state.composition().duration_frames - 1 } else { frame_at(*time, state) };
            go_to(state, frame)
        }
        Jump::By { by, forward } => {
            let by = frame_at(*by, state);
            go_to(state, if *forward { now + by } else { now - by })
        }
        Jump::Marker { next } => {
            let frames = state.composition().markers.iter().map(|marker| marker.frame);
            let found = if *next { frames.filter(|frame| *frame > now).min() } else { frames.filter(|frame| *frame < now).max() };
            match found {
                Some(frame) => go_to(state, frame),
                None if state.composition().markers.is_empty() => Done::failed("There are no markers on this composition. Say “add a marker here” to make one."),
                None => Done::failed(format!("There's no marker {} the playhead.", if *next { "after" } else { "before" })),
            }
        }
        Jump::MarkerCalled(label) => {
            let wanted = label.to_lowercase();
            let markers = &state.composition().markers;
            let found = markers.iter().find(|marker| marker.label.to_lowercase() == wanted).or_else(|| markers.iter().find(|marker| marker.label.to_lowercase().contains(&wanted)));
            match found.map(|marker| marker.frame) {
                Some(frame) => go_to(state, frame),
                None => {
                    let labels: Vec<String> = markers.iter().map(|marker| marker.label.clone()).collect();
                    let have = if labels.is_empty() { "There are no markers yet.".to_owned() } else { format!("The markers are: {}.", listed(&labels, MOST_NAMED)) };
                    Done::failed(format!("I couldn't find a marker called “{label}”. {have}"))
                }
            }
        }
        Jump::Keyframe { next } => {
            // The selected layers' keyframes, or every layer's when none is selected.
            let layers = if state.selected_layers.is_empty() { state.composition().layers.iter().map(|layer| layer.id).collect() } else { state.selected_layers.clone() };
            let frames = keyframe_frames(state, &layers);
            let found = if *next { frames.iter().copied().find(|frame| *frame > now) } else { frames.iter().copied().rfind(|frame| *frame < now) };
            match found {
                Some(frame) => go_to(state, frame),
                None if frames.is_empty() => Done::failed(if state.selected_layers.is_empty() { "There are no keyframes in this composition yet." } else { "The selected layers have no keyframes." }),
                None => Done::failed(format!("There's no keyframe {} the playhead.", if *next { "after" } else { "before" })),
            }
        }
    }
}

/// Changes the open composition's settings with `change`, as the Composition Settings dialog
/// does. `Err` says why the settings won't do.
fn change_composition(state: &mut EditorState, change: impl FnOnce(&mut studio_core::edit::CompositionSettings)) -> Result<(), String> {
    let mut settings = state.composition_settings();
    change(&mut settings);
    state.update_composition(&settings)
}

/// The composition's length, frame rate and size.
fn composition_chore(state: &mut EditorState, chore: &Chore) -> Done {
    let name = state.composition().name.clone();
    let fps = state.composition().frame_rate.fps().max(1.0);
    let before = state.composition().duration_frames;
    match chore {
        Chore::Duration(length) => {
            let frames = frame_at(*length, state).max(1);
            if frames as f64 / fps > LONGEST_HOURS * 3600.0 {
                return Done::failed(format!("That is longer than I'll make a composition: {LONGEST_HOURS} hours is the most. Composition Settings takes any length."));
            }
            if frames == before {
                return Done::said(format!("{name} is already {} long.", length_words(state, frames)));
            }
            if let Err(why) = change_composition(state, |settings| settings.duration_frames = frames) {
                return Done::failed(why);
            }
            let mut said = format!("Done: {name} is now {} long ({}).", length_words(state, frames), plural(frames as usize, "frame", "frames"));
            let cut_off = state.composition().layers.iter().filter(|layer| layer.out_frame > frames).count();
            if frames < before && cut_off > 0 {
                said.push_str(&format!(" {} past the new end: nothing is trimmed, the rest just isn't shown.", plural(cut_off, "layer runs", "layers run")));
            }
            Done::changed(said, Vec::new())
        }
        Chore::FrameRate(thousandths) => {
            let Some(rate) = super::composition_settings::parse_frame_rate(&format!("{}", f64::from(*thousandths) / 1000.0)) else {
                return Done::failed("That isn't a frame rate I can set.");
            };
            if rate == state.composition().frame_rate {
                return Done::said(format!("{name} already plays at {}.", rate.label()));
            }
            // The composition stays as long as it was in seconds.
            let frames = ((before as f64 / fps) * rate.fps()).round().max(1.0) as i64;
            if let Err(why) = change_composition(state, |settings| {
                settings.frame_rate = rate;
                settings.duration_frames = frames;
            }) {
                return Done::failed(why);
            }
            Done::changed(
                format!(
                    "Done: {name} now plays at {} and is still {} long ({}). Layers and keyframes keep their frame numbers, so they now fall at different moments: check the timing.",
                    rate.label(),
                    length_words(state, frames),
                    plural(frames as usize, "frame", "frames")
                ),
                Vec::new(),
            )
        }
        Chore::Size { width, height } => {
            let (now_width, now_height) = (state.composition().width, state.composition().height);
            let (width, height) = (width.unwrap_or(now_width), height.unwrap_or(now_height));
            if (width, height) == (now_width, now_height) {
                return Done::said(format!("{name} is already {width} × {height}."));
            }
            if let Err(why) = change_composition(state, |settings| {
                settings.width = width;
                settings.height = height;
            }) {
                return Done::failed(why);
            }
            Done::changed(
                format!("Done: {name} is now {width} × {height}. Layers keep their places, measured from the top left, so check the framing. For a copy rearranged to fit instead, undo this and say “make a {width}x{height} version”."),
                Vec::new(),
            )
        }
        Chore::DurationBy { by, longer } => {
            let by = frame_at(*by, state).max(1);
            let frames = if *longer { before + by } else { before - by };
            if frames < 1 {
                return Done::failed(format!("{name} is only {} long, so it can't be {} shorter.", length_words(state, before), length_words(state, by)));
            }
            let mut done = composition_chore(state, &Chore::Duration(Time::Span { millis: 0, frames }));
            if done.outcome == Outcome::Changed {
                done.said.push_str(&format!(" That is {} {} than it was.", length_words(state, by), if *longer { "longer" } else { "shorter" }));
            }
            done
        }
        _ => unreachable!("only the composition's own settings come here"),
    }
}

/// Sets the work area and says where it now is.
fn set_work_area(state: &mut EditorState, start: i64, end: i64) -> Done {
    if end <= start {
        return Done::failed(format!("The work area can't end at {} when it starts at {}.", timecode(state, end), timecode(state, start)));
    }
    let before = state.work_area;
    state.set_work_area(FrameRange::new(start as f64, end as f64));
    let (start, end) = (state.work_area.start.round() as i64, state.work_area.end.round() as i64);
    let said = format!("The work area runs from {} to {}, which is {}.", timecode(state, start), timecode(state, end), length_words(state, end - start));
    if state.work_area == before { Done::said(format!("{said} It was already.")) } else { Done::did(said) }
}

/// The work area: its ends, the whole composition, around layers, and trimming to it.
fn work_area_chore(state: &mut EditorState, request: &Request, message: &str, around: &Around) -> Done {
    let duration = state.composition().duration_frames;
    let (start, end) = (state.work_area.start.round() as i64, state.work_area.end.round() as i64);
    match &request.chore {
        Chore::WorkArea { from, to } => {
            let start = from.map_or(start, |time| frame_at(time, state)).clamp(0, duration - 1);
            let end = to.map_or(end, |time| frame_at(time, state)).min(duration);
            set_work_area(state, start, end)
        }
        Chore::WorkAreaWhole => set_work_area(state, 0, duration),
        Chore::WorkAreaAround => {
            let layers = match layers_of(state, around.pointed, &request.target, message) {
                Ok(layers) => layers,
                Err(why) => return Done::failed(why),
            };
            let spans = || layers.iter().filter_map(|id| state.composition().layer(*id));
            let (Some(first), Some(last)) = (spans().map(|layer| layer.in_frame).min(), spans().map(|layer| layer.out_frame).max()) else {
                return Done::failed("There's no layer to put the work area around.");
            };
            set_work_area(state, first.clamp(0, duration - 1), last.min(duration))
        }
        Chore::TrimToWorkArea => {
            let comp = state.composition().id;
            let name = state.composition().name.clone();
            let Some(trimmed) = studio_core::arrange::trim_composition(&mut state.project, comp, start, end) else {
                return Done::said(format!("The work area is already the whole of {name}, so there is nothing to trim. Set it first: “set the work area from 2s to 6s”."));
            };
            // The playhead, the view and the selection follow the shorter composition.
            let settings = state.composition_settings();
            if let Err(why) = state.update_composition(&settings) {
                return Done::failed(why);
            }
            state.set_work_area(FrameRange::new(0.0, trimmed.frames as f64));
            let kept: Vec<Id> = state.selected_layers.iter().copied().filter(|id| state.composition().layer(*id).is_some()).collect();
            state.selected_layers = kept;
            let mut said = format!("Done: {name} is now {} long, just what the work area held.", length_words(state, trimmed.frames));
            if trimmed.removed > 0 {
                said.push_str(&format!(" {} that ended before it {} removed.", plural(trimmed.removed, "layer", "layers"), if trimmed.removed == 1 { "is" } else { "are" }));
            }
            if trimmed.left_after > 0 {
                said.push_str(&format!(" {} after the new end and {} still in the timeline.", plural(trimmed.left_after, "layer starts", "layers start"), if trimmed.left_after == 1 { "is" } else { "are" }));
            }
            Done::changed(said, Vec::new())
        }
        _ => unreachable!("only the work area's chores come here"),
    }
}

/// Staggering, sequencing, closing gaps and reversing the order.
fn arranging_chore(state: &mut EditorState, request: &Request, message: &str, around: &Around) -> Done {
    if request.chore == Chore::CloseGaps {
        if !around.settings.feature_on(Feature::CloseGaps) {
            return turned_off(Feature::CloseGaps);
        }
        return match state.close_gaps() {
            0 => Done::said("There are no gaps to close."),
            moved => Done::changed(format!("Done: I closed the gaps. {} earlier, on {}.", plural(moved, "clip moved", "clips moved"), if state.selected_layers.is_empty() { "every track" } else { "the selected layers' tracks" }), Vec::new()),
        };
    }
    if matches!(request.chore, Chore::Stagger(_) | Chore::Sequence) && !around.settings.feature_on(Feature::SequenceLayers) {
        return turned_off(Feature::SequenceLayers);
    }
    let layers = match arranged(state, around.pointed, &request.target, message) {
        Ok(layers) => layers,
        Err(why) => return Done::failed(why),
    };
    let comp = state.composition().id;
    let first = named(state, &layers[..1]);
    let count = plural(layers.len(), "layer", "layers");
    let Some(composition) = state.project.composition_mut(comp) else { return Done::failed("The composition is gone.") };
    match &request.chore {
        Chore::Stagger(step) => {
            let fps = composition.frame_rate.fps();
            let frames = match step {
                Some(Time::Span { millis, frames }) => ((*millis as f64 / 1000.0 * fps).round() as i64 + frames).max(1),
                _ => crate::workspace::STAGGER_FRAMES,
            };
            if !studio_core::edit::stagger_layers(composition, &layers, Some(frames)) {
                return Done::said(format!("They already start {} apart.", plural(frames as usize, "frame", "frames")));
            }
            Done::changed(format!("Done: {count} now start {} apart, one after the other from {first}.", plural(frames as usize, "frame", "frames")), layers)
        }
        Chore::Sequence => {
            if !studio_core::edit::stagger_layers(composition, &layers, None) {
                return Done::said("They already play one after another.");
            }
            Done::changed(format!("Done: {count} now play one after another, starting with {first}."), layers)
        }
        Chore::ReverseOrder => {
            if !studio_core::arrange::reverse_order(composition, &layers) {
                return Done::failed("I couldn't change their order: fewer than two of them are unlocked.");
            }
            Done::changed(format!("Done: {count} now play in the opposite order."), layers)
        }
        _ => unreachable!("only arranging chores come here"),
    }
}

/// A volume in decibels as it is shown: "−6.0 dB (50%)".
fn volume_words(db: f32) -> String {
    if db <= LayerAudio::VOLUME_RANGE.0 {
        return "silent".to_owned();
    }
    format!("at {db:+.1} dB ({:.0}%)", 10f32.powf(db / 20.0) * 100.0)
}

/// Muting, volume and fades of the sound.
fn sound_chore(state: &mut EditorState, request: &Request, message: &str, around: &Around) -> Done {
    let layers = match sound_layers(state, around.pointed, &request.target, message) {
        Ok(layers) => layers,
        Err(why) => return Done::failed(why),
    };
    let who = named(state, &layers);
    let is = if layers.len() == 1 { "is" } else { "are" };
    match &request.chore {
        Chore::Mute(mute) => {
            let comp = state.composition().id;
            let Some(composition) = state.project.composition_mut(comp) else { return Done::failed("The composition is gone.") };
            let mut changed = Vec::new();
            for id in &layers {
                if let Some(layer) = composition.layer_mut(*id).filter(|layer| layer.switches.audio == *mute) {
                    layer.switches.audio = !mute;
                    changed.push(*id);
                }
            }
            if changed.is_empty() {
                return Done::said(format!("{who} {is} already {}.", if *mute { "muted" } else { "playing its sound" }));
            }
            let who = named(state, &changed);
            Done::changed(format!("Done: {who} {} {}.", if changed.len() == 1 { "is" } else { "are" }, if *mute { "muted" } else { "heard again" }), changed)
        }
        Chore::Volume(level) => {
            let (lowest, highest) = LayerAudio::VOLUME_RANGE;
            let mut keyed = false;
            let mut reached = None;
            for id in &layers {
                let now = state.audio_setting(*id, AudioSetting::Volume).unwrap_or(0.0);
                let wanted = match level {
                    Level::Percent(percent) if *percent <= 0 => lowest,
                    Level::Percent(percent) => 20.0 * (*percent as f32 / 100.0).log10(),
                    Level::Decibels(tenths) => *tenths as f32 / 10.0,
                    Level::By(tenths) => now + *tenths as f32 / 10.0,
                };
                keyed |= state.composition().layer(*id).is_some_and(|layer| layer.audio.volume_db.is_animated());
                state.set_audio_setting(*id, AudioSetting::Volume, wanted.clamp(lowest, highest));
                reached = Some((wanted, state.audio_setting(*id, AudioSetting::Volume).unwrap_or(now)));
            }
            let Some((wanted, db)) = reached else { return Done::failed("There was no sound to change.") };
            let mut said = format!("Done: {who} {is} {}.", volume_words(db));
            if wanted > highest {
                said.push_str(&format!(" {highest:+.0} dB is as loud as it goes."));
            }
            if keyed {
                said.push_str(" That is a keyframe at the playhead, since the volume was already animated.");
            }
            Done::changed(said, layers)
        }
        Chore::SoundFade { ends, length } => {
            let fps = state.composition().frame_rate.fps().max(1.0);
            let seconds = match length {
                Some(Time::Span { millis, frames }) => *millis as f32 / 1000.0 + (*frames as f64 / fps) as f32,
                _ => USUAL_FADE_SECONDS,
            };
            let settings: &[AudioSetting] = match ends {
                Ends::In => &[AudioSetting::FadeIn],
                Ends::Out => &[AudioSetting::FadeOut],
                Ends::Both => &[AudioSetting::FadeIn, AudioSetting::FadeOut],
            };
            let mut reached = seconds;
            for id in &layers {
                for setting in settings {
                    state.set_audio_setting(*id, *setting, seconds);
                    // A fade is never longer than its clip.
                    reached = reached.min(state.audio_setting(*id, *setting).unwrap_or(seconds));
                }
            }
            let ends = match ends {
                Ends::In => "in",
                Ends::Out => "out",
                Ends::Both => "in and out",
            };
            let said = if seconds <= 0.0 {
                format!("Done: the sound of {who} no longer fades {ends}.")
            } else {
                let short = if reached < seconds { " (as long as the clip allows)" } else { "" };
                format!("Done: the sound of {who} fades {ends} over {}{short}.", spoken(f64::from(reached)))
            };
            Done::changed(said, layers)
        }
        _ => unreachable!("only sound chores come here"),
    }
}

/// The one layer with sound whose name `text` holds, if exactly one does.
fn sound_layer_named_in(state: &EditorState, text: &str) -> Option<Id> {
    let mut found = state.composition().layers.iter().filter(|layer| layer.name.chars().count() >= 3 && text.contains(&layer.name.to_lowercase()) && state.has_sound(layer.id));
    found.next().filter(|_| found.next().is_none()).map(|layer| layer.id)
}

/// Turning music down under a voice, or stopping that.
fn duck(state: &mut EditorState, on: bool, request: &Request, message: &str, around: &Around) -> Done {
    if !on {
        let ducking: Vec<Id> = match sound_layers(state, around.pointed, &request.target, message) {
            Ok(layers) => layers.into_iter().filter(|id| state.composition().layer(*id).is_some_and(|layer| layer.audio.duck_under.is_some())).collect(),
            Err(why) => return Done::failed(why),
        };
        if ducking.is_empty() {
            return Done::said("Nothing there is ducking under another layer.");
        }
        for id in &ducking {
            state.set_duck_under(*id, None);
        }
        return Done::changed(format!("Done: {} no longer {} under anything.", named(state, &ducking), if ducking.len() == 1 { "ducks" } else { "duck" }), ducking);
    }
    let lower = message.to_lowercase();
    let sounding: Vec<Id> = state.composition().layers.iter().map(|layer| layer.id).filter(|id| state.has_sound(*id)).collect();
    let selected: Vec<Id> = state.selected_layers.iter().copied().filter(|id| sounding.contains(id)).collect();
    // Said outright: "duck Music under Voice", with the music named or selected.
    let said_outright = lower.split_once(" under ").and_then(|(music, voice)| {
        let voice = sound_layer_named_in(state, voice)?;
        let music = sound_layer_named_in(state, music).or(match selected.as_slice() {
            [one] => Some(*one),
            _ => None,
        })?;
        (music != voice).then_some(vec![(music, voice)])
    });
    // Otherwise what the music-under-a-voice rule has noticed, and failing that the only
    // pairing there can be: the one sound clip, under the one other layer with sound.
    let noticed: Vec<(Id, Id)> = around.suggestions.iter().filter_map(|suggestion| if let Some(Fix::Duck { layer, under }) = suggestion.fix { Some((layer, under)) } else { None }).collect();
    let only_pair = || {
        let beds: Vec<Id> = sounding.iter().copied().filter(|id| state.composition().layer(*id).is_some_and(|layer| matches!(layer.content, LayerContent::Audio { .. }))).collect();
        let music = match (selected.as_slice(), beds.as_slice()) {
            ([one], _) => *one,
            ([], [one]) => *one,
            _ => return None,
        };
        match sounding.iter().copied().filter(|id| *id != music).collect::<Vec<_>>().as_slice() {
            [voice] => Some(vec![(music, *voice)]),
            _ => None,
        }
    };
    let Some(pairs) = said_outright.or((!noticed.is_empty()).then_some(noticed)).or_else(only_pair) else {
        return Done::failed("Tell me which is which, by the layers' names: “duck Music under Voice”. Or select the music and say “duck it under Voice”.");
    };
    for (music, voice) in &pairs {
        state.set_duck_under(*music, Some(*voice));
    }
    let (music, voice) = pairs[0];
    let by = state.audio_setting(music, AudioSetting::DuckAmount).unwrap_or(0.0);
    let more = if pairs.len() > 1 { format!(" The same for {} more.", pairs.len() - 1) } else { String::new() };
    Done::changed(format!("Done: {} now ducks under {}: it drops by {by:.0} dB while that is sounding, and comes back in between.{more} Properties ▸ Audio sets how far.", named(state, &[music]), named(state, &[voice])), pairs.iter().map(|(music, _)| *music).collect())
}

/// Adding, naming and removing markers.
fn marker_chore(state: &mut EditorState, chore: &Chore, around: &Around) -> Done {
    if !around.settings.feature_on(Feature::Markers) {
        return turned_off(Feature::Markers);
    }
    let comp = state.composition().id;
    let duration = state.composition().duration_frames;
    match chore {
        Chore::Marker { at, label } => {
            let frame = frame_at(*at, state);
            if !(0..duration).contains(&frame) {
                return Done::failed(format!("{} is outside the composition, which ends at {}.", timecode(state, frame), timecode(state, duration)));
            }
            let at = timecode(state, frame);
            let Some(composition) = state.project.composition_mut(comp) else { return Done::failed("The composition is gone.") };
            match (composition.markers.iter_mut().find(|marker| marker.frame == frame), label) {
                (Some(marker), Some(label)) if marker.label != *label => {
                    marker.label = label.clone();
                    Done::changed(format!("Done: the marker at {at} is now called “{label}”."), Vec::new())
                }
                (Some(marker), _) => Done::said(format!("There is already a marker at {at}, called “{}”.", marker.label)),
                (None, label) => {
                    composition.markers.push(Marker { frame, label: label.clone().unwrap_or_else(|| "Marker".to_owned()) });
                    composition.markers.sort_by_key(|marker| marker.frame);
                    let called = label.as_ref().map_or_else(String::new, |label| format!(", called “{label}”"));
                    Done::changed(format!("Done: there is a marker at {at}{called}."), Vec::new())
                }
            }
        }
        Chore::RemoveMarker(at) => {
            let frame = frame_at(*at, state);
            let at = timecode(state, frame);
            let Some(composition) = state.project.composition_mut(comp) else { return Done::failed("The composition is gone.") };
            let before = composition.markers.len();
            composition.markers.retain(|marker| marker.frame != frame);
            if composition.markers.len() == before {
                return Done::failed(format!("There's no marker at {at}. Go to it first (“next marker”), or say where it is."));
            }
            Done::changed(format!("Done: the marker at {at} is gone."), Vec::new())
        }
        Chore::ClearMarkers => match state.composition().markers.len() {
            0 => Done::said("There are no markers to remove."),
            count => {
                state.clear_markers();
                Done::changed(format!("Done: {} removed.", plural(count, "marker is", "markers are")), Vec::new())
            }
        },
        _ => unreachable!("only marker chores come here"),
    }
}

/// A color said, as the model keeps it.
fn rgba(rgb: [u8; 3]) -> Rgba {
    Rgba::hex((u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]))
}

/// Making a layer: text, a solid, a shape, a background, a null object or an adjustment layer.
fn make(state: &mut EditorState, what: &Made, color: &Option<([u8; 3], String)>, around: &Around) -> Done {
    let needs = match what {
        Made::Text(_) => Feature::TextTool,
        Made::Shape { .. } | Made::Background => Feature::ShapeTool,
        Made::Solid | Made::Null | Made::Adjustment => Feature::NewLayer,
    };
    if !around.settings.feature_on(needs) {
        return turned_off(needs);
    }
    let comp = state.composition().id;
    let frame_size = state.composition().size();
    let center = frame_size * 0.5;
    let from = format!("from the playhead ({}) on", timecode(state, state.current_frame));
    let color_words = color.as_ref().map(|(_, name)| name.clone());
    let made = match what {
        Made::Text(text) if text.trim().is_empty() => return Done::said("What should it say? Tell me the words: add text saying “Hello”."),
        Made::Text(text) => {
            let text: String = text.chars().take(MOST_TEXT).collect();
            state.add_text_layer(&text, center).map(|id| (id, format!("a text layer saying “{text}”, in the middle of the frame {from}")))
        }
        Made::Solid => state.add_solid().map(|id| {
            if let Some((rgb, _)) = color
                && let Some(layer) = state.project.composition_mut(comp).and_then(|composition| composition.layer_mut(id))
            {
                layer.content = LayerContent::Solid { color: rgba(*rgb) };
            }
            (id, format!("a solid in {} that fills the frame {from}", color_words.clone().unwrap_or_else(|| "the current color".to_owned())))
        }),
        Made::Shape { kind, even } => {
            let shorter = frame_size.x.min(frame_size.y);
            let size = if *even { studio_core::animation::Vec2::new(shorter / 3.0, shorter / 3.0) } else { studio_core::animation::Vec2::new(shorter / 2.0, shorter / 3.0) };
            let mut style = ShapeStyle::new(*kind);
            if let Some((rgb, _)) = color {
                style.fill = Some(Paint::Solid(rgba(*rgb)));
            }
            let frame = state.current_frame;
            studio_core::edit::add_shape_layer(&mut state.project, comp, style, size, center, frame).map(|id| {
                state.select_layer(id, false);
                let shade = color_words.clone().map_or_else(String::new, |name| format!("{name} "));
                (id, format!("a {shade}{} in the middle of the frame {from}", kind.name().to_lowercase()))
            })
        }
        Made::Background => state.add_background().map(|id| {
            if let Some((rgb, _)) = color {
                studio_core::edit::edit_shape(state.project.composition_mut(comp).expect("the open composition"), id, |style| style.fill = Some(Paint::Solid(rgba(*rgb))));
            }
            (id, format!("a {} background behind everything, for the whole composition", color_words.clone().unwrap_or_else(|| "white".to_owned())))
        }),
        Made::Null => state.add_null().map(|id| (id, format!("a null object in the middle of the frame {from}, for other layers to be parented to"))),
        Made::Adjustment => state.add_adjustment_layer().map(|id| (id, format!("an adjustment layer {from}: effects on it change everything below"))),
    };
    let Some((id, description)) = made else { return Done::failed("I couldn't add that layer.") };
    let done = Done::changed(format!("Done: I added {}: {description}. It is selected, so carry on with “it”.", named(state, &[id])), vec![id]);
    match what {
        Made::Text(_) => done.offering(&["Bring it in word by word", "Put it at the top"]),
        Made::Shape { .. } | Made::Solid | Made::Background => done.offering(&["Fade it in"]),
        Made::Null | Made::Adjustment => done,
    }
}

/// Speed, playing backwards and freeze frames, which only video clips have.
fn speed_chore(state: &mut EditorState, request: &Request, message: &str, around: &Around) -> Done {
    let needs = if request.chore == Chore::Freeze { Feature::FreezeFrame } else { Feature::Speed };
    if !around.settings.feature_on(needs) {
        return turned_off(needs);
    }
    let is_clip = |layer: &Layer| studio_core::speed::has_speed(&layer.content);
    // With none selected or named, the one video clip under the playhead is meant.
    let meant = layers_of(state, around.pointed, &request.target, message).or_else(|why| {
        let under: Vec<Id> = state.composition().layers.iter().filter(|layer| is_clip(layer) && layer.is_active_at(state.current_frame)).map(|layer| layer.id).collect();
        if under.len() == 1 && matches!(request.target, Target::Selection | Target::Unnamed) { Ok(under) } else { Err(why) }
    });
    let meant = match meant {
        Ok(layers) => layers,
        Err(why) => return Done::failed(why),
    };
    let clips: Vec<Id> = meant.iter().copied().filter(|id| state.composition().layer(*id).is_some_and(is_clip)).collect();
    if clips.is_empty() {
        return Done::failed(format!("{} {} a video clip, and only video clips have a speed to change. To change how fast it animates, say “make the fade faster” or “make the fade slower”.", named(state, &meant), if meant.len() == 1 { "isn't" } else { "aren't" }));
    }
    let span = |state: &EditorState, id: Id| state.composition().layer(id).map_or(0, |layer| layer.out_frame - layer.in_frame);
    let mut changed = Vec::new();
    let mut problems = Vec::new();
    let mut told = String::new();
    for id in clips {
        let name = named(state, &[id]);
        match &request.chore {
            Chore::Speed(pace) => {
                let before = span(state, id);
                let wanted = match (pace, state.speed_of(id)) {
                    (Pace::Normal, _) => 100.0,
                    (Pace::Percent(percent), _) => *percent as f32,
                    (Pace::Times { times, over }, Some(now)) => now * *times as f32 / (*over).max(1) as f32,
                    (Pace::Times { .. }, None) => {
                        problems.push(format!("{name} has a speed ramp, so say the speed outright: “50% speed”."));
                        continue;
                    }
                };
                let speed = wanted.clamp(LayerTime::SPEED_RANGE.0, LayerTime::SPEED_RANGE.1);
                if *pace == Pace::Normal { state.reset_speed(id) } else { state.set_speed(id, speed) }
                if state.speed_of(id).is_none_or(|now| (now - speed).abs() > 0.5) {
                    problems.push(format!("{name} couldn't change speed (it is locked, or a freeze frame)."));
                    continue;
                }
                told = format!("now plays at {speed:.0}%, so it is {} long instead of {}", length_words(state, span(state, id)), length_words(state, before));
                if (wanted - speed).abs() > 0.5 {
                    told.push_str(&format!(" ({:.0}% to {:.0}% is as far as speed goes)", LayerTime::SPEED_RANGE.0, LayerTime::SPEED_RANGE.1));
                }
                changed.push(id);
            }
            Chore::Reverse(reverse) => {
                let was = state.composition().layer(id).is_some_and(|layer| layer.time.reverse);
                state.set_reverse(id, *reverse);
                let is = state.composition().layer(id).is_some_and(|layer| layer.time.reverse);
                if was == *reverse {
                    problems.push(format!("{name} already plays {}.", if *reverse { "backwards" } else { "forwards" }));
                } else if is != *reverse {
                    problems.push(format!("{name} couldn't be reversed (it is locked, or a freeze frame)."));
                } else {
                    told = if *reverse { "now plays backwards" } else { "plays forwards again" }.to_owned();
                    changed.push(id);
                }
            }
            Chore::Freeze => {
                if !state.can_freeze_at_playhead(id) {
                    problems.push(format!("{name} isn't playing at the playhead (or is locked), so there's no frame of it to hold there."));
                } else if state.insert_freeze_frame(id) {
                    let hold = state.selected_layers.clone();
                    let said = format!("Done: {name} holds on the frame at {} for 2 seconds, and what came after it on its track moved later. The hold is selected.", timecode(state, state.current_frame));
                    return Done::changed(said, hold);
                }
            }
            _ => unreachable!("only speed chores come here"),
        }
    }
    let problems = problems.join(" ");
    if changed.is_empty() {
        return Done::failed(if problems.is_empty() { "That didn't change anything.".to_owned() } else { problems });
    }
    let each = if changed.len() > 1 { "Each of " } else { "" };
    let said = format!("Done: {each}{} {told}. {problems}", named(state, &changed));
    Done::changed(said.trim_end(), changed)
}

/// Makes the layers' In and Out animations take `percent` of the time they take now: both the
/// one-click motions and text coming in a piece at a time.
fn motion_pace(state: &mut EditorState, percent: i64, ends: Ends, request: &Request, message: &str, around: &Around) -> Done {
    use studio_core::presets::{MAX_SECONDS, Phase};
    let layers = match layers_of(state, around.pointed, &request.target, message) {
        Ok(layers) => layers,
        Err(why) => return Done::failed(why),
    };
    let phases: &[Phase] = match ends {
        Ends::In => &[Phase::In],
        Ends::Out => &[Phase::Out],
        Ends::Both => &[Phase::In, Phase::Out],
    };
    let paced = |seconds: f32| (seconds * percent as f32 / 100.0).clamp(0.1, MAX_SECONDS);
    let mut changed = Vec::new();
    let mut told = Vec::new();
    for &id in &layers {
        let Some(layer) = state.composition().layer(id).filter(|layer| !layer.switches.locked) else { continue };
        let animation = layer.animation;
        for &phase in phases {
            let way = if phase == Phase::In { "in" } else { "out" };
            let (motion, text) = if phase == Phase::In { (animation.enter, animation.text_in) } else { (animation.exit, animation.text_out) };
            let mut note = |before: f32, after: f32| {
                if (before - after).abs() > 0.005 {
                    told.push(format!("its way {way} takes {after:.2}s instead of {before:.2}s"));
                    changed.push(id);
                }
            };
            if let Some((motion, seconds)) = motion {
                state.set_layer_motion(id, phase, Some(motion), paced(seconds));
                note(seconds, paced(seconds));
            }
            if let Some(animator) = text {
                state.edit_text_animator(id, phase, |animator| animator.seconds = paced(animator.seconds));
                note(animator.seconds, paced(animator.seconds));
            }
        }
    }
    changed.dedup();
    if changed.is_empty() {
        let which = match ends {
            Ends::In => "way in",
            Ends::Out => "way out",
            Ends::Both => "way in or out",
        };
        return Done::failed(format!("{} has no {which} to change (or it is already as {} as it goes). Give it one first: “fade it in”.", named(state, &layers), if percent > 100 { "slow" } else { "quick" }));
    }
    // One layer's changes are spelled out; several are summed up.
    let what = if changed.len() == 1 { told.join(" and ") } else { format!("each now animates in {percent}% of the time it took") };
    Done::changed(format!("Done: for {}, {what}.", named(state, &changed)), changed)
}

/// Makes each layer `by` longer or shorter than it is, as "last" would with the new length.
fn change_lengths(state: &mut EditorState, by: Time, longer: bool, layers: &[Id]) -> Done {
    use studio_core::instructions::{Context, carry_out};
    let by = frame_at(by, state).max(1);
    let comp = state.composition().id;
    let context = Context { playhead: state.current_frame, magnetic_main_track: state.magnetic_main_track };
    let (mut changed, mut told, mut problems) = (Vec::new(), String::new(), Vec::new());
    // How far the last layer changed really went: its footage or the composition's end may
    // stop it short of what was asked.
    let mut reached = 0;
    for &id in layers {
        let Some(layer) = state.composition().layer(id) else { continue };
        let length = layer.out_frame - layer.in_frame;
        let wanted = if longer { length + by } else { length - by };
        if wanted < 1 {
            problems.push(format!("“{}” is only {} long, so it can't be {} shorter.", layer.name, length_words(state, length), length_words(state, by)));
            continue;
        }
        let outcome = carry_out(&mut state.project, comp, &[id], &[Step::Last(Time::Span { millis: 0, frames: wanted })], context);
        problems.extend(outcome.problems);
        if let Some(phrase) = outcome.done.first() {
            told = phrase.clone();
            changed.push(id);
            reached = state.composition().layer(id).map_or(0, |layer| (layer.out_frame - layer.in_frame - length).abs());
        }
    }
    let problems = problems.join(" ");
    if changed.is_empty() {
        return Done::failed(if problems.is_empty() { "That didn't change anything.".to_owned() } else { problems });
    }
    let way = if longer { "longer" } else { "shorter" };
    let said = if changed.len() == 1 {
        format!("Done: {} {told}. That is {} {way} than it was. {problems}", named(state, &changed), length_words(state, reached))
    } else {
        format!("Done: {} are each {} {way}, as far as their footage and the composition allow. {problems}", named(state, &changed), length_words(state, by))
    };
    Done::changed(said.trim_end(), changed)
}

/// The export format a format asked for by name stands for.
fn export_format(format: Format) -> studio_export::ExportFormat {
    use studio_export::ExportFormat;
    use studio_media::MovieFormat;
    match format {
        Format::Mp4 => ExportFormat::Movie(MovieFormat::H264),
        Format::Hevc => ExportFormat::Movie(MovieFormat::H265),
        Format::Mov => ExportFormat::Movie(MovieFormat::ProRes422),
        Format::MovWithAlpha => ExportFormat::Movie(MovieFormat::ProRes4444),
        Format::WebM => ExportFormat::Movie(MovieFormat::WebM),
        Format::Gif => ExportFormat::Gif,
        Format::PngSequence => ExportFormat::PngSequence,
        Format::Lottie => ExportFormat::Lottie,
        Format::DotLottie => ExportFormat::DotLottie,
        Format::Svg => ExportFormat::AnimatedSvg,
    }
}

/// Exports straight away, through the export queue: the open composition or every one, in
/// each format asked for, into the Exports folder (see `export::exports_folder`), never over
/// a file already there. The reply says where each file will land; the queue reports back in
/// the conversation when each is written or fails (see `assistant::export_finished`).
fn export_now(workspace: &mut Workspace, export: &Export, window: &mut Window, cx: &mut Context<Workspace>) -> Done {
    let choice = workspace.ui.export_choice;
    export_with(workspace, export, choice, window, cx)
}

/// Exports as `export_now` does, with `choice` in place of the Export window's choices.
fn export_with(workspace: &mut Workspace, export: &Export, choice: super::export::ExportChoice, window: &mut Window, cx: &mut Context<Workspace>) -> Done {
    use super::export_queue::{QueueStatus, QueuedExport};
    if !workspace.has(Feature::Export) {
        return turned_off(Feature::Export);
    }
    // Recipes set to run before each export run first, so their changes are exported.
    super::export::before_export(workspace, window, cx);
    // "Export a vertical version": the copy is made and opened first, then exported whole.
    let mut opening = String::new();
    if let Some((width, height, label)) = &export.size {
        let copy = super::assistant::make_resized_copy(workspace, *width, *height, label);
        if copy.outcome != Outcome::Changed {
            return copy;
        }
        opening = format!("{} ", copy.said);
    }
    let formats: Vec<studio_export::ExportFormat> = if export.formats.is_empty() { vec![choice.format] } else { export.formats.iter().copied().map(export_format).collect() };
    let folder = super::export::exports_folder(workspace.state.file());
    if let Err(error) = std::fs::create_dir_all(&folder) {
        return Done::failed(format!("I couldn't export: the folder {} could not be made ({error}).", folder.display()));
    }
    // What is exported: every composition whole, or the open one over the frames said (or,
    // with nothing said, the frames the Export dialog is set to).
    let state = &workspace.state;
    let work_area = state.work_area.start.round() as i64..state.work_area.end.round() as i64;
    let whole = 0..state.composition().duration_frames;
    let (frames, span_words) = match export.span {
        _ if export.all || export.size.is_some() => (whole, "whole".to_owned()),
        Some(Span::Whole) => (whole, "the whole composition".to_owned()),
        Some(Span::WorkArea) => (work_area.clone(), format!("the work area ({} to {})", timecode(state, work_area.start), timecode(state, work_area.end))),
        None if choice.work_area_only && work_area != whole => (work_area.clone(), format!("the work area ({} to {}), as the Export dialog is set", timecode(state, work_area.start), timecode(state, work_area.end))),
        None => (whole, "the whole composition".to_owned()),
    };
    let compositions: Vec<(Id, String, std::ops::Range<i64>)> = if export.all {
        state.project.compositions.iter().map(|composition| (composition.id, composition.name.clone(), 0..composition.duration_frames)).collect()
    } else {
        vec![(state.active_comp, state.composition().name.clone(), frames)]
    };
    // Exports someone put in the queue and hasn't started yet run when the queue does.
    let already_waiting = workspace.ui.export_queue.items.iter().filter(|item| item.status == QueueStatus::Waiting).count();
    let mut paths = Vec::new();
    for (composition, name, frames) in &compositions {
        for format in &formats {
            // Never over a file already there, nor one an export still waiting is going to write.
            let path = super::export::free_path(&folder, name, format.extension(), |path| workspace.ui.export_queue.items.iter().any(|item| item.request.path == path));
            let request = studio_export::ExportRequest {
                frame_rate: choice.frame_rate,
                include_audio: choice.include_audio,
                scale: choice.scale,
                quality: choice.quality,
                speed: choice.speed,
                audio_kbps: choice.audio_kbps,
                ..studio_export::ExportRequest::new(*composition, frames.clone(), *format, path.clone())
            };
            let project = workspace.state.project.clone();
            workspace.ui.export_queue.items.push(QueuedExport { composition: name.clone(), project, request, status: QueueStatus::Waiting, from_assistant: true });
            paths.push((path, format.label()));
        }
    }
    // The queue takes them one after another. If it is already working it reaches these in
    // turn; an export started from the Export dialog has to finish before it can start.
    let waits = if workspace.ui.export_queue.running {
        " The export queue is already working and will reach it in turn."
    } else if workspace.ui.export_job.is_some() {
        " Another export is running, so this waits in the export queue: start the queue (File ▸ Export Queue…) once that one finishes."
    } else {
        super::export_queue::start(workspace, window, cx);
        if already_waiting > 0 { " The exports already waiting in the export queue run first." } else { "" }
    };
    let said = match paths.as_slice() {
        [(path, label)] => format!("{opening}Exporting {} as {label}, {span_words}. It will land at {}.{waits} I'll say here when it is written; the toolbar shows how far it has got.", compositions[0].1, path.display()),
        _ => {
            let lines: Vec<String> = paths.iter().map(|(path, label)| format!("• {} ({label})", path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned()))).collect();
            let what = if export.all { format!("{}, each {span_words}", plural(compositions.len(), "composition", "compositions")) } else { format!("{}, {span_words}", compositions[0].1) };
            format!("{opening}Exporting {what}: {} into {}.{waits}\n{}\nI'll say here as each is written.", plural(paths.len(), "file", "files"), folder.display(), lines.join("\n"))
        }
    };
    Done::did(said)
}

/// Answers a message about export presets (`iris::export_presets`): exports with one, saves the
/// Export window's choices as one, forgets one, or lists them.
pub fn export_preset(workspace: &mut Workspace, said: iris::export_presets::Said, window: &mut Window, cx: &mut Context<Workspace>) -> Done {
    use super::export_presets::{self as presets, PresetSettings};
    use iris::export_presets::Said;
    match said {
        Said::Use(name) => {
            let Some(preset) = presets::find(&name) else { return Done::failed(format!("I don't know an export preset called “{name}”.")) };
            let settings = PresetSettings::of(&preset);
            let choice = settings.choice();
            // A composition of another shape than the preset's is copied at its size first.
            let composition = workspace.state.composition();
            let size = settings.frame.filter(|frame| *frame != (composition.width, composition.height)).map(|(width, height)| (width, height, preset.name.clone()));
            let span = Some(if choice.work_area_only { Span::WorkArea } else { Span::Whole });
            let mut done = export_with(workspace, &Export { formats: Vec::new(), all: false, span, size }, choice, window, cx);
            if done.outcome != Outcome::Failed {
                done.said = format!("With the “{}” preset ({}). {}", preset.name, settings.describe(), done.said);
            }
            done
        }
        Said::Save(name) => match presets::save_current(&workspace.ui.export_choice, &name) {
            Ok(said) => Done::said(said),
            Err(why) => Done::failed(why),
        },
        Said::Forget(name) => {
            let mut saved = presets::saved();
            let before = saved.len();
            saved.retain(|preset| !preset.name.eq_ignore_ascii_case(&name));
            if saved.len() == before {
                let own = presets::built_in().iter().any(|preset| preset.name.eq_ignore_ascii_case(&name));
                return Done::failed(if own { format!("“{name}” is a preset Motion comes with, so I can't forget it.") } else { format!("I don't have an export preset called “{name}”.") });
            }
            match presets::keep_saved(&saved) {
                Ok(()) => Done::said(format!("Forgotten: the export preset “{name}”.")),
                Err(why) => Done::failed(why),
            }
        }
        Said::List => {
            let listed = iris::export_presets::listed(&presets::saved(), &presets::built_in());
            Done::said(format!("Export presets:\n{listed}\nSay “export for” and a name to export with one, or “save these export settings as” and a name to keep the Export window's choices."))
        }
        Said::Problem(why) => Done::failed(why),
    }
}

/// The check before exporting: missing media, the composition's length and work area, what
/// the rules have noticed, and what is hidden, with a plain verdict first. `exists` says
/// whether a file is on disk.
pub fn preflight(state: &EditorState, around: &Around, exists: &dyn Fn(&Path) -> bool) -> Done {
    let composition = state.composition();
    let project = &state.project;
    let name = &composition.name;
    let fps = composition.frame_rate.fps().max(1.0);
    let is_file = |kind: ProjectItemKind| matches!(kind, ProjectItemKind::Footage | ProjectItemKind::Image | ProjectItemKind::Audio);
    let used: HashSet<Id> = project
        .compositions
        .iter()
        .flat_map(|composition| &composition.layers)
        .filter_map(|layer| match layer.content {
            LayerContent::Footage { item } | LayerContent::Image { item } | LayerContent::Audio { item } => Some(item),
            _ => None,
        })
        .collect();
    let missing: Vec<String> = project.items.iter().filter(|item| used.contains(&item.id) && item.path.as_ref().is_some_and(|path| !exists(path))).map(|item| item.name.clone()).collect();
    let unused = project.items.iter().filter(|item| is_file(item.kind) && !used.contains(&item.id)).count();
    let showing: Vec<&Layer> = composition.layers.iter().filter(|layer| layer.switches.visible || !layer.content.has_picture()).collect();
    let hidden = composition.layers.len() - showing.len();
    let last_end = showing.iter().map(|layer| layer.out_frame).max().unwrap_or(0).min(composition.duration_frames);
    let duration = composition.duration_frames;
    let (work_start, work_end) = (state.work_area.start.round() as i64, state.work_area.end.round() as i64);
    let whole_work_area = work_start == 0 && work_end == duration;
    let safe = around.suggestions.iter().filter(|suggestion| suggestion.fix.as_ref().is_some_and(Fix::is_safe)).count();

    // What stops an export, then what is worth a look, then what is fine.
    let (mut blockers, mut warnings, mut fine) = (Vec::new(), Vec::new(), Vec::new());
    if showing.is_empty() {
        blockers.push(format!("{name} has nothing in it to show or hear."));
    }
    if missing.is_empty() {
        fine.push("Every file it uses is in place.".to_owned());
    } else {
        blockers.push(format!("{} missing: {}. Put {} back where the project expects, or import again.", plural(missing.len(), "file is", "files are"), listed(&missing, MOST_NAMED), if missing.len() == 1 { "it" } else { "them" }));
    }
    let length = spoken(duration as f64 / fps);
    if !showing.is_empty() && last_end < duration {
        warnings.push(format!("{name} is {length} long, but the last layer ends at {}, so the final {} are empty.", timecode(state, last_end), spoken((duration - last_end) as f64 / fps)));
    } else if !showing.is_empty() {
        fine.push(format!("{name} is {length} long and filled to the end."));
    }
    let work_span = format!("{} to {} ({} of {length})", timecode(state, work_start), timecode(state, work_end), spoken((work_end - work_start) as f64 / fps));
    match (whole_work_area, around.work_area_only) {
        (true, _) => fine.push("The work area is the whole composition.".to_owned()),
        (false, true) => warnings.push(format!("The export is set to the work area, which is only {work_span}.")),
        (false, false) => fine.push(format!("The export is set to the whole composition; the work area is {work_span}.")),
    }
    match around.suggestions.len() {
        0 => fine.push("Nothing I look for stands out.".to_owned()),
        open => warnings.push(format!("{} I noticed {} still open{}.", plural(open, "thing", "things"), if open == 1 { "is" } else { "are" }, if safe > 0 { format!(" ({safe} safe to fix at once)") } else { String::new() })),
    }
    if hidden > 0 {
        warnings.push(format!("{} hidden and won't be in the export.", plural(hidden, "layer is", "layers are")));
    }
    if unused > 0 {
        fine.push(format!("{} not used anywhere, which doesn't affect the export.", plural(unused, "imported file is", "imported files are")));
    }

    let verdict = if !blockers.is_empty() {
        "Not ready yet."
    } else if !warnings.is_empty() {
        "Nearly: nothing stops the export, but have a look at these first."
    } else {
        "Ready to export."
    };
    let lines = |heading: &str, items: &[String]| if items.is_empty() { String::new() } else { format!("\n{heading}\n{}", items.iter().map(|item| format!("• {item}")).collect::<Vec<_>>().join("\n")) };
    let said = format!("{verdict}{}{}{}", lines("In the way:", &blockers), lines("Worth a look:", &warnings), lines("Fine:", &fine));
    let mut replies = Vec::new();
    if safe > 0 {
        replies.push("Fix everything");
    }
    if hidden > 0 {
        replies.push("Unhide everything");
    }
    if !whole_work_area {
        replies.push("Reset the work area");
    }
    if blockers.is_empty() && around.settings.feature_on(Feature::Export) {
        replies.push("Export");
    }
    Done::said(said).offering(&replies)
}

/// Carries out the chores that only need the editor's state.
fn on_state(state: &mut EditorState, request: &Request, message: &str, around: &Around) -> Done {
    match &request.chore {
        Chore::Go(place) => jump(state, place, request, message, around),
        Chore::Duration(_) | Chore::DurationBy { .. } | Chore::FrameRate(_) | Chore::Size { .. } => composition_chore(state, &request.chore),
        Chore::MotionPace { percent, ends } => motion_pace(state, *percent, *ends, request, message, around),
        Chore::WorkArea { .. } | Chore::WorkAreaWhole | Chore::WorkAreaAround | Chore::TrimToWorkArea => work_area_chore(state, request, message, around),
        Chore::Stagger(_) | Chore::Sequence | Chore::CloseGaps | Chore::ReverseOrder => arranging_chore(state, request, message, around),
        Chore::Mute(_) | Chore::Volume(_) | Chore::SoundFade { .. } => sound_chore(state, request, message, around),
        Chore::Duck(on) => duck(state, *on, request, message, around),
        Chore::Marker { .. } | Chore::RemoveMarker(_) | Chore::ClearMarkers => marker_chore(state, &request.chore, around),
        Chore::Make { what, color } => make(state, what, color, around),
        Chore::Speed(_) | Chore::Reverse(_) | Chore::Freeze => speed_chore(state, request, message, around),
        Chore::Preflight => preflight(state, around, &|path| path.exists()),
        Chore::Play { .. } | Chore::Pause | Chore::LineUp(_) | Chore::SameLength(_) | Chore::LengthBy { .. } | Chore::ShiftBy { .. } | Chore::Export(_) | Chore::ExportFrame | Chore::RevealExport | Chore::SendFrame(_) => {
            unreachable!("`carry_out` does these itself")
        }
    }
}

/// Carries out a chore in the open composition and says what happened.
pub fn carry_out(workspace: &mut Workspace, request: &Request, message: &str, window: &mut Window, cx: &mut Context<Workspace>) -> Done {
    let (carried, last) = (workspace.ui.assistant.carried.clone(), workspace.ui.assistant.last_layers.clone());
    let pointed = Pointed { carried: &carried, last: &last };
    match &request.chore {
        Chore::Export(export) => export_now(workspace, export, window, cx),
        Chore::SendFrame(to) => super::assistant_automation::send_frame(workspace, *to, window, cx),
        Chore::ExportFrame => super::assistant::save_frame(workspace, window, cx),
        Chore::RevealExport => match workspace.ui.assistant.last_export.clone() {
            None => Done::failed("I haven't exported anything yet in this session. Say “export as mp4” or “save this frame as a PNG” first."),
            Some(path) => match super::project::reveal_in_finder(&path) {
                Ok(()) => Done::did(format!("Showing {} in the Finder.", path.display())),
                Err(why) => Done::failed(format!("I couldn't show {}: {why}.", path.display())),
            },
        },
        // A length changed by an amount is "last" with each layer's own new length; a start
        // moved with no amount said is "move it later" with the usual step.
        Chore::LengthBy { by, longer } => match layers_of(&workspace.state, pointed, &request.target, message) {
            Ok(layers) => change_lengths(&mut workspace.state, *by, *longer, &layers),
            Err(why) => Done::failed(why),
        },
        Chore::ShiftBy { by, later } => match layers_of(&workspace.state, pointed, &request.target, message) {
            Ok(layers) => {
                let mut done = super::assistant::instruct_layers(workspace, &layers, &[Step::Shift { by: *by, later: *later }]);
                if done.outcome == Outcome::Changed {
                    done.said.push_str(&format!(" That was {} {}: say “more” or “less” to go on from there.", length_words(&workspace.state, frame_at(*by, &workspace.state)), if *later { "later" } else { "earlier" }));
                }
                done
            }
            Err(why) => Done::failed(why),
        },
        Chore::Play { from } => {
            if let Some(from) = from {
                let frame = frame_at(*from, &workspace.state);
                workspace.state.set_current_frame(frame);
            }
            if workspace.state.is_playing() {
                return Done::said("It is already playing. Say “pause” to stop.");
            }
            workspace.toggle_playback(cx);
            Done::did(format!("Playing from {}. Say “pause” to stop.", timecode(&workspace.state, workspace.state.current_frame)))
        }
        Chore::Pause => {
            if !workspace.state.is_playing() {
                return Done::said("Nothing is playing.");
            }
            workspace.toggle_playback(cx);
            Done::did(format!("Paused at {}.", timecode(&workspace.state, workspace.state.current_frame)))
        }
        // Lining up and matching lengths are instructions for each layer, carried out as
        // "start at" and "last" are.
        Chore::LineUp(_) | Chore::SameLength(_) => {
            let layers = match arranged(&workspace.state, pointed, &request.target, message) {
                Ok(layers) => layers,
                Err(why) => return Done::failed(why),
            };
            let step = match &request.chore {
                Chore::LineUp(at) => Step::StartAt(*at),
                Chore::SameLength(pick) => {
                    let lengths = layers.iter().filter_map(|id| workspace.state.composition().layer(*id)).map(|layer| layer.out_frame - layer.in_frame);
                    let frames = match pick {
                        Pick::First => lengths.take(1).next(),
                        Pick::Longest => lengths.max(),
                        Pick::Shortest => lengths.min(),
                    };
                    Step::Last(Time::Span { millis: 0, frames: frames.unwrap_or(1) })
                }
                _ => unreachable!("matched just above"),
            };
            super::assistant::instruct_layers(workspace, &layers, &[step])
        }
        _ => {
            let suggestions = super::assistant::open_suggestions(workspace);
            let around = Around { settings: &workspace.ui.settings, suggestions: &suggestions, pointed, work_area_only: workspace.ui.export_choice.work_area_only };
            on_state(&mut workspace.state, request, message, &around)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads `message` as a chore and carries it out on `state` with everything switched on.
    fn run(state: &mut EditorState, message: &str) -> Done {
        run_with(state, message, &AppSettings::default(), &[])
    }

    fn run_with(state: &mut EditorState, message: &str, settings: &AppSettings, suggestions: &[Suggestion]) -> Done {
        let request = studio_core::chores::read(message).unwrap_or_else(|| panic!("“{message}” should read as a chore"));
        on_state(state, &request, message, &Around { settings, suggestions, pointed: Pointed::default(), work_area_only: false })
    }

    /// The sample project with the main track left as it is, so clips stay where they are put.
    fn state() -> EditorState {
        let mut state = EditorState::new();
        state.magnetic_main_track = false;
        state
    }

    /// The first layer with sound, and the first that is footage.
    fn sound_and_clip(state: &EditorState) -> (Id, Id) {
        let layers = &state.composition().layers;
        let sound = layers.iter().find(|layer| state.has_sound(layer.id)).expect("the sample has sound").id;
        let clip = layers.iter().find(|layer| studio_core::speed::has_speed(&layer.content)).expect("the sample has footage").id;
        (sound, clip)
    }

    #[test]
    fn the_playhead_goes_to_moments_markers_and_keyframes() {
        let mut state = state();
        let fps = state.composition().frame_rate.fps();
        let last = state.composition().duration_frames - 1;
        assert_eq!(run(&mut state, "go to 2s").outcome, Outcome::Did);
        assert_eq!(state.current_frame, (2.0 * fps).round() as i64);
        run(&mut state, "jump to frame 12");
        assert_eq!(state.current_frame, 12);
        run(&mut state, "back 2 frames");
        assert_eq!(state.current_frame, 10);
        assert!(run(&mut state, "go to the end").said.contains(&format!("frame {last}")));
        assert!(run(&mut state, "go to 10 minutes").said.contains("ends there"), "past the end is said, not hidden");
        run(&mut state, "go to the start");
        assert_eq!(state.current_frame, 0);

        // Markers: by position and by name.
        assert_eq!(run(&mut state, "next marker").outcome, Outcome::Failed, "there are none yet");
        state.set_current_frame(30);
        assert_eq!(run(&mut state, "add a marker called Chorus").outcome, Outcome::Changed);
        assert_eq!(run(&mut state, "add a marker at frame 60").outcome, Outcome::Changed);
        state.set_current_frame(0);
        run(&mut state, "next marker");
        assert_eq!(state.current_frame, 30);
        run(&mut state, "next marker");
        assert_eq!(state.current_frame, 60);
        run(&mut state, "previous marker");
        assert_eq!(state.current_frame, 30);
        state.set_current_frame(0);
        run(&mut state, "go to the chorus marker");
        assert_eq!(state.current_frame, 30);
        assert!(run(&mut state, "go to the Verse marker").said.contains("Chorus"), "the markers there are named");

        // Keyframes of the whole composition when nothing is selected.
        let frames = keyframe_frames(&state, &state.composition().layers.iter().map(|layer| layer.id).collect::<Vec<_>>());
        assert!(frames.len() >= 2, "the sample is animated");
        state.set_current_frame(frames[0]);
        run(&mut state, "next keyframe");
        assert_eq!(state.current_frame, frames[1]);
        run(&mut state, "previous keyframe");
        assert_eq!(state.current_frame, frames[0]);
    }

    #[test]
    fn the_composition_grows_and_shrinks_by_a_stated_step_and_edges_are_gone_to() {
        let mut state = state();
        let fps = state.composition().frame_rate.fps().round() as i64;
        let before = state.composition().duration_frames;
        let done = run(&mut state, "make the composition longer");
        assert_eq!((done.outcome, state.composition().duration_frames), (Outcome::Changed, before + fps));
        assert!(done.said.contains("1 second longer"), "{}", done.said);
        run(&mut state, "shorten the comp by 2 seconds");
        assert_eq!(state.composition().duration_frames, before - fps);
        assert_eq!(run(&mut state, "shorten the composition by 10 minutes").outcome, Outcome::Failed, "it isn't that long");
        // A layer's ends, and the work area's.
        let title = state.composition().layers.iter().find(|layer| layer.name == "Title").map(|layer| (layer.in_frame, layer.out_frame)).unwrap();
        run(&mut state, "go to the start of the title");
        assert_eq!(state.current_frame, title.0);
        assert!(run(&mut state, "go to where the title ends").said.contains("ends"));
        assert_eq!(state.current_frame, title.1 - 1, "the last frame it shows on");
        state.set_work_area(FrameRange::new(10.0, 20.0));
        run(&mut state, "go to the end of the work area");
        assert_eq!(state.current_frame, 19);
        assert!(run(&mut state, "go to the start of “Nope”").said.contains("couldn't find"));
    }

    #[test]
    fn markers_are_added_named_and_removed() {
        let mut state = state();
        state.set_current_frame(24);
        run(&mut state, "add a marker here");
        assert_eq!(state.composition().markers, vec![Marker { frame: 24, label: "Marker".to_owned() }]);
        assert_eq!(run(&mut state, "add a marker here").outcome, Outcome::Said, "one is there already");
        assert_eq!(run(&mut state, "rename this marker to Drop").outcome, Outcome::Changed);
        assert_eq!(state.composition().markers[0].label, "Drop");
        assert_eq!(run(&mut state, "remove the marker at frame 99").outcome, Outcome::Failed);
        assert_eq!(run(&mut state, "delete this marker").outcome, Outcome::Changed);
        assert!(state.composition().markers.is_empty());
        assert_eq!(run(&mut state, "clear all markers").outcome, Outcome::Said);
        // Switched off, it is explained and nothing is added.
        let mut settings = AppSettings::default();
        settings.set_feature(Feature::Markers, false);
        let done = run_with(&mut state, "add a marker here", &settings, &[]);
        assert!(done.outcome == Outcome::Failed && done.said.contains("turned off in Settings"), "{}", done.said);
        assert!(state.composition().markers.is_empty());
    }

    #[test]
    fn the_compositions_length_rate_and_size_change_as_the_dialog_would() {
        let mut state = state();
        let fps = state.composition().frame_rate.fps();
        let done = run(&mut state, "make the composition 4 seconds long");
        assert_eq!(done.outcome, Outcome::Changed);
        assert_eq!(state.composition().duration_frames, (4.0 * fps).round() as i64);
        assert!(state.work_area.end <= state.composition().duration_frames as f64, "the work area follows");
        assert_eq!(run(&mut state, "make the composition 4 seconds long").outcome, Outcome::Said, "already that long");
        assert_eq!(run(&mut state, "make the composition 9 hours long").outcome, Outcome::Failed);

        // A new frame rate keeps the length in seconds, and says frames kept their numbers.
        let done = run(&mut state, "set the frame rate to 60");
        assert_eq!((state.composition().frame_rate.fps(), state.composition().duration_frames), (60.0, 240));
        assert!(done.said.contains("keep their frame numbers"), "{}", done.said);

        let done = run(&mut state, "set the size to 1080x1920");
        assert_eq!((state.composition().width, state.composition().height), (1080, 1920));
        assert!(done.said.contains("top left"), "{}", done.said);
        run(&mut state, "set the composition width to 720");
        assert_eq!((state.composition().width, state.composition().height), (720, 1920));
    }

    #[test]
    fn the_work_area_is_set_reset_and_trimmed_to() {
        let mut state = state();
        let fps = state.composition().frame_rate.fps();
        let duration = state.composition().duration_frames;
        let frames = |seconds: f64| (seconds * fps).round();
        assert_eq!(run(&mut state, "set the work area from 1s to 3s").outcome, Outcome::Did);
        assert_eq!((state.work_area.start, state.work_area.end), (frames(1.0), frames(3.0)));
        state.set_current_frame(frames(2.0) as i64);
        run(&mut state, "end the work area here");
        assert_eq!((state.work_area.start, state.work_area.end), (frames(1.0), frames(2.0)));
        assert_eq!(run(&mut state, "set the work area from 3s to 1s").outcome, Outcome::Failed);
        let done = run(&mut state, "trim the composition to the work area");
        assert_eq!(done.outcome, Outcome::Changed);
        assert_eq!(state.composition().duration_frames, frames(1.0) as i64);
        assert_eq!((state.work_area.start, state.work_area.end), (0.0, frames(1.0)));
        assert!(state.current_frame < state.composition().duration_frames);
        assert_eq!(run(&mut state, "trim the composition to the work area").outcome, Outcome::Said, "nothing left to trim");
        assert!(duration > state.composition().duration_frames);
    }

    #[test]
    fn layers_are_staggered_sequenced_and_reversed_in_the_order_selected() {
        let mut state = state();
        let pictures: Vec<Id> = state.composition().layers.iter().filter(|layer| layer.content.has_picture() && !layer.switches.locked).map(|layer| layer.id).take(3).collect();
        let starts = |state: &EditorState| pictures.iter().map(|id| state.composition().layer(*id).unwrap().in_frame).collect::<Vec<_>>();
        assert_eq!(run(&mut state, "stagger them by 5 frames").outcome, Outcome::Failed, "nothing is selected");
        state.selected_layers = pictures.clone();
        let first = starts(&state)[0];
        assert_eq!(run(&mut state, "stagger them by 5 frames").outcome, Outcome::Changed);
        assert_eq!(starts(&state), vec![first, first + 5, first + 10]);
        assert_eq!(run(&mut state, "stagger them by 5 frames").outcome, Outcome::Said, "already so");
        run(&mut state, "reverse their order");
        assert_eq!(starts(&state), vec![first + 10, first + 5, first]);
        // Switched off, sequencing is explained instead.
        let mut settings = AppSettings::default();
        settings.set_feature(Feature::SequenceLayers, false);
        assert!(run_with(&mut state, "sequence them", &settings, &[]).said.contains("Sequence and Stagger is turned off"));
    }

    #[test]
    fn sound_is_muted_set_and_faded() {
        let mut state = state();
        let (sound, _) = sound_and_clip(&state);
        state.select_layer(sound, false);
        let audible = |state: &EditorState| state.composition().layer(sound).unwrap().switches.audio;
        let was = audible(&state);
        let done = run(&mut state, if was { "mute it" } else { "unmute it" });
        assert_eq!((done.outcome, audible(&state)), (Outcome::Changed, !was));
        assert_eq!(run(&mut state, if was { "mute it" } else { "unmute it" }).outcome, Outcome::Said, "already so");

        let done = run(&mut state, "set the volume to 50%");
        let db = state.audio_setting(sound, AudioSetting::Volume).unwrap();
        assert!((db + 6.02).abs() < 0.01, "{db}");
        assert!(done.said.contains("-6.0 dB (50%)"), "{}", done.said);
        run(&mut state, "make it a bit louder");
        assert!((state.audio_setting(sound, AudioSetting::Volume).unwrap() - (db + 1.5)).abs() < 0.01);
        assert!(run(&mut state, "volume to 40 db").said.contains("as loud as it goes"));
        assert_eq!(state.audio_setting(sound, AudioSetting::Volume), Some(LayerAudio::VOLUME_RANGE.1));

        run(&mut state, "fade the audio in over half a second");
        assert_eq!(state.audio_setting(sound, AudioSetting::FadeIn), Some(0.5));
        run(&mut state, "fade the sound in and out");
        assert_eq!((state.audio_setting(sound, AudioSetting::FadeIn), state.audio_setting(sound, AudioSetting::FadeOut)), (Some(1.0), Some(1.0)));
        assert!(run(&mut state, "remove the audio fade out").said.contains("no longer fades out"));
        assert_eq!(state.audio_setting(sound, AudioSetting::FadeOut), Some(0.0));

        // A layer without sound says so.
        let silent = state.composition().layers.iter().find(|layer| !state.has_sound(layer.id)).unwrap().id;
        state.select_layer(silent, false);
        assert!(run(&mut state, "mute it").said.contains("no sound"));
    }

    #[test]
    fn layers_are_made_where_the_tools_would_make_them() {
        let mut state = state();
        state.set_current_frame(12);
        let before = state.composition().layers.len();
        let done = run(&mut state, "add a text layer saying “Hello there”");
        let text = state.composition().layer(done.layers[0]).unwrap();
        assert!(matches!(&text.content, LayerContent::Text { text, .. } if text == "Hello there") && text.in_frame == 12);
        assert_eq!(state.selected_layers, done.layers, "ready for “it”");
        assert_eq!(run(&mut state, "add a title").outcome, Outcome::Said, "what it says is asked for");

        let done = run(&mut state, "add a dark red solid");
        assert!(matches!(state.composition().layer(done.layers[0]).unwrap().content, LayerContent::Solid { color } if color == rgba([137, 34, 32])));
        let done = run(&mut state, "add a blue circle");
        let circle = state.composition().layer(done.layers[0]).unwrap();
        assert!(matches!(&circle.content, LayerContent::Shape { style, .. } if style.fill == Some(Paint::Solid(rgba([30, 136, 229])))) && circle.size.x == circle.size.y);
        run(&mut state, "add a null object");
        run(&mut state, "add an adjustment layer");
        assert_eq!(state.composition().layers.len(), before + 5);

        let mut settings = AppSettings::default();
        settings.set_feature(Feature::TextTool, false);
        assert!(run_with(&mut state, "add text saying “No”", &settings, &[]).said.contains("Text is turned off"));
        assert_eq!(state.composition().layers.len(), before + 5);
    }

    #[test]
    fn only_video_clips_change_speed_and_they_say_their_new_length() {
        let mut state = state();
        let (_, clip) = sound_and_clip(&state);
        state.select_layer(clip, false);
        let length = |state: &EditorState| {
            let layer = state.composition().layer(clip).unwrap();
            layer.out_frame - layer.in_frame
        };
        let before = length(&state);
        let done = run(&mut state, "make it twice as fast");
        assert_eq!((done.outcome, state.speed_of(clip)), (Outcome::Changed, Some(200.0)));
        assert!(length(&state) < before && done.said.contains("200%"), "{}", done.said);
        run(&mut state, "half speed");
        assert_eq!(state.speed_of(clip), Some(100.0));
        // With no amount said, a moderate step from the speed it has, which the reply states.
        assert!(run(&mut state, "speed it up").said.contains("125%"));
        assert!(run(&mut state, "slow it down").said.contains("100%"));
        assert_eq!(state.speed_of(clip), Some(100.0));
        run(&mut state, "reverse it");
        assert!(state.composition().layer(clip).unwrap().time.reverse);
        assert!(run(&mut state, "reverse it").said.contains("already plays backwards"));
        run(&mut state, "back to normal speed");
        assert!(state.composition().layer(clip).unwrap().time.is_normal());

        let still = state.composition().layers.iter().find(|layer| matches!(layer.content, LayerContent::Text { .. })).unwrap().id;
        state.select_layer(still, false);
        assert!(run(&mut state, "make it twice as fast").said.contains("only video clips"));
    }

    #[test]
    fn the_check_before_exporting_gives_a_verdict_and_its_reasons() {
        let mut state = state();
        let settings = AppSettings::default();
        let around = Around { settings: &settings, suggestions: &[], pointed: Pointed::default(), work_area_only: true };
        // Every file in place: nothing stops the export.
        let done = preflight(&state, &around, &|_| true);
        assert!(!done.said.starts_with("Not ready"), "{}", done.said);
        assert!(done.replies.contains(&"Export".to_owned()));
        // A missing file that is used stops it, and is named.
        let gone = std::path::PathBuf::from("/gone/clip.mov");
        let used = state.composition().layers.iter().find_map(|layer| if let LayerContent::Footage { item } = layer.content { Some(item) } else { None }).expect("the sample has footage");
        let item = state.project.items.iter_mut().find(|item| item.id == used).unwrap();
        item.path = Some(gone.clone());
        let name = item.name.clone();
        let done = preflight(&state, &around, &|path| path != gone);
        assert!(done.said.starts_with("Not ready yet.") && done.said.contains(&name), "{}", done.said);
        assert!(!done.replies.contains(&"Export".to_owned()));
        // A work area that is only part of it, when the export is set to it, is worth a look.
        state.set_work_area(FrameRange::new(0.0, 24.0));
        let done = preflight(&state, &around, &|_| true);
        assert!(done.said.contains("set to the work area") && done.replies.contains(&"Reset the work area".to_owned()), "{}", done.said);
        // Nothing here changes the project.
        assert_eq!(done.outcome, Outcome::Said);
    }
}
