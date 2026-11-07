//! Changing how fast footage plays: a constant speed, playing backwards, and speed ramps
//! (keyframed speed, such as slowing down for a moment and speeding back up), and freeze
//! frames (one moment of a clip held still).
//!
//! A constant speed change keeps the clip's in point and the part of the source it shows, so
//! a slower clip gets longer and a faster one shorter. A ramp keeps the clip's length and
//! changes what plays within it.

use crate::animation::{Animated, CubicBezier, Easing, Keyframe};
use crate::model::{Composition, Id, LayerContent, LayerTime, Project};
use crate::trim_tools::End;

/// Whether a layer's speed can be changed: footage (stills and graphics have no speed).
pub fn has_speed(content: &LayerContent) -> bool {
    matches!(content, LayerContent::Footage { .. })
}

/// Whether `layer`'s speed can be edited now: footage that is playing (a freeze frame shows
/// one moment, so speed means nothing to it) and not locked.
fn speed_editable(layer: &&mut crate::model::Layer) -> bool {
    has_speed(&layer.content) && layer.time.hold.is_none() && !layer.switches.locked
}

/// Sets a constant speed (a percentage), ending any ramp. The clip keeps its in point and the
/// part of the source it shows. Returns whether it changed.
pub fn set_speed(composition: &mut Composition, layer_id: Id, percent: f32) -> bool {
    let Some(layer) = composition.layer_mut(layer_id).filter(speed_editable) else { return false };
    if !percent.is_finite() {
        return false;
    }
    let percent = percent.clamp(LayerTime::SPEED_RANGE.0, LayerTime::SPEED_RANGE.1);
    let new = f64::from(percent) / 100.0;
    // Amounts of source, in frames at normal speed. Summed frame by frame, so a clip that was
    // ramping keeps what its in point showed and how much source it played.
    let before_in = layer.time.source_seconds_between(layer.start_frame, layer.in_frame, 1.0);
    let played = layer.time.source_seconds_between(layer.in_frame, layer.out_frame, 1.0);
    // The source's start, as seen from the in point, stays put: what the in point shows now,
    // it shows after. Frames are shown rounded down, so the whole frame on screen is kept.
    let shown = (before_in + 1e-6).floor();
    layer.start_frame = layer.in_frame - (shown / new).round() as i64;
    layer.out_frame = layer.in_frame + ((played / new).round() as i64).max(1);
    layer.time.speed = Animated::constant(percent);
    layer.time.speed_before_ramp = None;
    true
}

/// The Rate Stretch tool: moves one end of a clip to `frame` and changes the clip's speed so
/// the same part of its source fills the new length, while the other end stays put. The speed
/// stays within `LayerTime::SPEED_RANGE`, so the end may stop short of `frame`, and a ramp
/// becomes a constant speed. Returns whether the clip changed.
pub fn stretch_to(composition: &mut Composition, layer_id: Id, end: End, frame: i64) -> bool {
    let Some(layer) = composition.layer(layer_id).filter(|layer| has_speed(&layer.content) && layer.time.hold.is_none() && !layer.switches.locked) else { return false };
    let (in_frame, out_frame) = (layer.in_frame, layer.out_frame);
    // How much source the clip shows, in frames at normal speed.
    let played = layer.time.source_seconds_between(in_frame, out_frame, 1.0);
    let length = match end {
        End::In => out_frame - frame,
        End::Out => frame - in_frame,
    }
    .max(1);
    if played <= 0.0 || !set_speed(composition, layer_id, (played / length as f64 * 100.0) as f32) {
        return false;
    }
    // `set_speed` keeps the in point; stretching from the start keeps the out point instead.
    if end == End::In
        && let Some(layer) = composition.layer_mut(layer_id)
    {
        let shift = out_frame - layer.out_frame;
        layer.shift_in_time(shift);
    }
    true
}

pub fn set_reverse(composition: &mut Composition, layer_id: Id, reverse: bool) -> bool {
    let Some(layer) = composition.layer_mut(layer_id).filter(speed_editable) else { return false };
    layer.time.reverse = reverse;
    true
}

/// Ready-made speed ramps over a clip's length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ramp {
    /// Normal, slowing right down in the middle, then back to normal.
    SlowMiddle,
    /// Starts slow and speeds up.
    SpeedUp,
    /// Starts fast and slows down to a stop-like crawl.
    SlowDown,
    /// Fast, a quick slow-motion moment, fast again: the action-cam look.
    Punch,
}

impl Ramp {
    pub const ALL: [Self; 4] = [Self::SlowMiddle, Self::SpeedUp, Self::SlowDown, Self::Punch];

    pub fn name(self) -> &'static str {
        match self {
            Self::SlowMiddle => "Slow-Mo Middle",
            Self::SpeedUp => "Speed Up",
            Self::SlowDown => "Slow Down",
            Self::Punch => "Speed Punch",
        }
    }

    /// The ramp's speeds at points through the clip (0–1), as percentages.
    fn points(self) -> &'static [(f64, f32)] {
        match self {
            Self::SlowMiddle => &[(0.0, 100.0), (0.35, 100.0), (0.5, 25.0), (0.65, 100.0), (1.0, 100.0)],
            Self::SpeedUp => &[(0.0, 40.0), (1.0, 250.0)],
            Self::SlowDown => &[(0.0, 250.0), (1.0, 30.0)],
            Self::Punch => &[(0.0, 300.0), (0.4, 300.0), (0.45, 30.0), (0.6, 30.0), (0.65, 300.0), (1.0, 300.0)],
        }
    }
}

/// Puts a speed ramp across the clip, keeping its length. The speed the clip had before is
/// remembered (switching from one ramp to another keeps the original) for `remove_ramp`.
pub fn apply_ramp(composition: &mut Composition, layer_id: Id, ramp: Ramp) -> bool {
    let Some(layer) = composition.layer_mut(layer_id).filter(speed_editable) else { return false };
    let (start, length) = (layer.in_frame, (layer.out_frame - layer.in_frame) as f64);
    let smooth = Easing::Curve(CubicBezier::new(0.45, 0.0, 0.55, 1.0));
    let keys = ramp.points().iter().map(|&(at, speed)| Keyframe { frame: start + (at * length).round() as i64, value: speed, easing: smooth }).collect();
    if let Some(factor) = layer.time.constant_factor() {
        layer.time.speed_before_ramp = Some((factor * 100.0) as f32);
    }
    layer.time.speed = Animated::keyframed(keys);
    true
}

/// Takes a ramp off, putting back the constant speed the clip had before it. A ramp only
/// changes the speed (never the clip's start, in or out), so the clip is as it was. Ramps from
/// projects saved before the speed was remembered go back to 100%. Returns whether it changed.
pub fn remove_ramp(composition: &mut Composition, layer_id: Id) -> bool {
    let Some(layer) = composition.layer_mut(layer_id).filter(speed_editable) else { return false };
    if layer.time.constant_factor().is_some() {
        return false;
    }
    let percent = layer.time.speed_before_ramp.take().unwrap_or(100.0);
    layer.time.speed = Animated::constant(percent.clamp(LayerTime::SPEED_RANGE.0, LayerTime::SPEED_RANGE.1));
    true
}

/// Inserts a freeze frame into a footage clip at `frame`, as Premiere's Insert Frame Hold
/// Segment does: the clip is split there, a still of that moment `length` frames long goes in
/// between, and everything after it on the same track moves later to make room (so nothing is
/// covered). The hold is silent. Returns the new hold's id, or `None` if the layer isn't
/// unlocked footage spanning `frame`. The composition grows if the track now runs past its end.
pub fn insert_freeze_frame(project: &mut Project, comp_id: Id, layer_id: Id, frame: i64, length: i64) -> Option<Id> {
    let composition = project.composition(comp_id)?;
    let layer = composition.layer(layer_id)?;
    let spans = layer.in_frame <= frame && frame < layer.out_frame;
    if !has_speed(&layer.content) || layer.switches.locked || !spans || length < 1 {
        return None;
    }
    let held = layer.source_seconds(frame, composition.frame_rate);
    let (track, original) = (layer.track, layer.clone());

    // Split first (unless the hold goes at the very start), then push the rest of the track on.
    if frame > original.in_frame {
        crate::cut::split_layers(project, comp_id, &[layer_id], frame);
    }
    let hold_id = project.allocate_id();
    let composition = project.composition_mut(comp_id)?;
    for later in composition.layers.iter_mut().filter(|other| other.track == track && other.in_frame >= frame && !other.switches.locked) {
        later.shift_in_time(length);
    }

    // The hold is a copy of the clip showing just that moment, with no sound.
    let mut hold = original;
    hold.id = hold_id;
    hold.name = format!("{} Freeze", hold.name);
    hold.in_frame = frame;
    hold.out_frame = frame + length;
    hold.time = LayerTime { hold: Some(held), ..LayerTime::default() };
    hold.has_audio = false;
    hold.effects.clear();
    hold.track = track;
    let above = composition.layers.iter().position(|other| other.id == layer_id).unwrap_or(0);
    composition.layers.insert(above, hold);
    let end = composition.layers.iter().filter(|other| other.track == track).map(|other| other.out_frame).max().unwrap_or(0);
    composition.duration_frames = composition.duration_frames.max(end);
    crate::tracks::normalize(composition);
    Some(hold_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Vec2;
    use crate::edit::{self, FootageInfo};
    use crate::model::Project;
    use crate::sample::{ids, sample_project};
    use crate::time::FrameRate;

    fn clip() -> (Project, Id) {
        let mut project = sample_project();
        let info = FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: 4.0, frame_rate: FrameRate::new(24, 1), has_audio: true };
        let item = edit::import_footage(&mut project, "/media/run.mp4".into(), &info);
        let layer = edit::add_layer_from_item(&mut project, ids::MAIN_COMP, item, 0, 24).unwrap();
        (project, layer)
    }

    #[test]
    fn rate_stretch_changes_the_speed_to_fill_the_new_length() {
        let (mut project, id) = clip();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        // The 96-frame clip from frame 24, stretched out to twice its length: half speed.
        assert!(stretch_to(comp, id, End::Out, 24 + 192));
        let layer = comp.layer(id).unwrap();
        assert_eq!((layer.in_frame, layer.out_frame), (24, 216));
        assert_eq!(layer.time.speed.value, 50.0);
        let rate = FrameRate::new(24, 1);
        assert_eq!(layer.source_frame(215, rate, rate), 95, "still ends on the last source frame");
        // Its start pulled in to frame 168 squeezes it into 48 frames, keeping the out point.
        assert!(stretch_to(comp, id, End::In, 168));
        let layer = comp.layer(id).unwrap();
        assert_eq!((layer.in_frame, layer.out_frame, layer.time.speed.value), (168, 216, 200.0));
        assert_eq!(layer.source_frame(168, rate, rate), 0, "still starts on the first source frame");
    }

    #[test]
    fn rate_stretch_leaves_stills_alone() {
        let mut project = sample_project();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        let text = comp.layers.iter().find(|layer| matches!(layer.content, LayerContent::Text { .. })).unwrap().id;
        let before = comp.layer(text).unwrap().clone();
        assert!(!stretch_to(comp, text, End::Out, before.out_frame + 50));
        assert_eq!(comp.layer(text).unwrap(), &before);
    }

    #[test]
    fn half_speed_doubles_the_clip_and_plays_half_as_far_each_frame() {
        let (mut project, id) = clip();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        assert!(set_speed(comp, id, 50.0));
        let layer = comp.layer(id).unwrap();
        assert_eq!((layer.in_frame, layer.out_frame), (24, 24 + 192));
        let rate = FrameRate::new(24, 1);
        assert_eq!(layer.source_frame(24, rate, rate), 0);
        assert_eq!(layer.source_frame(24 + 48, rate, rate), 24, "two seconds in, one second of source");
        // The trims know the longer clip is all source.
        assert_eq!(crate::trim::source_frames(&project, project.composition(ids::MAIN_COMP).unwrap(), project.composition(ids::MAIN_COMP).unwrap().layer(id).unwrap()), Some(192));
    }

    #[test]
    fn reversed_clips_play_the_same_frames_backwards() {
        let (mut project, id) = clip();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        set_reverse(comp, id, true);
        let layer = comp.layer(id).unwrap();
        let rate = FrameRate::new(24, 1);
        assert_eq!(layer.source_frame(24, rate, rate), 95, "starts on the last frame");
        assert_eq!(layer.source_frame(24 + 95, rate, rate), 0, "ends on the first");
    }

    #[test]
    fn leaving_a_ramp_keeps_the_frame_at_the_in_point() {
        let (mut project, id) = clip();
        // Trimmed so the in point shows a second into the source, then ramped.
        assert!(crate::trim::trim_in(&mut project, ids::MAIN_COMP, id, 48));
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        assert!(apply_ramp(comp, id, Ramp::SpeedUp));
        let rate = FrameRate::new(24, 1);
        let shown = comp.layer(id).unwrap().source_frame(48, rate, rate);
        assert!(set_speed(comp, id, 100.0));
        assert_eq!(comp.layer(id).unwrap().source_frame(48, rate, rate), shown);
    }

    #[test]
    fn moving_a_ramped_clip_takes_its_ramp_along() {
        let (mut project, id) = clip();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        assert!(apply_ramp(comp, id, Ramp::SpeedUp));
        let rate = FrameRate::new(24, 1);
        let before: Vec<i64> = (24..120).map(|frame| comp.layer(id).unwrap().source_frame(frame, rate, rate)).collect();
        comp.layer_mut(id).unwrap().shift_in_time(100);
        let after: Vec<i64> = (124..220).map(|frame| comp.layer(id).unwrap().source_frame(frame, rate, rate)).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn removing_a_ramp_puts_the_clip_back_as_it_was() {
        let (mut project, id) = clip();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        assert!(set_speed(comp, id, 50.0));
        let before = comp.layer(id).unwrap().clone();
        // Two ramps in a row: removing goes back to the speed before the first.
        assert!(apply_ramp(comp, id, Ramp::SpeedUp));
        assert!(apply_ramp(comp, id, Ramp::Punch));
        assert!(remove_ramp(comp, id));
        assert_eq!(*comp.layer(id).unwrap(), before, "speed, start, in and out all as before");
        assert!(!remove_ramp(comp, id), "nothing to remove now");
    }

    #[test]
    fn a_ramp_without_a_remembered_speed_goes_back_to_normal() {
        let (mut project, id) = clip();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        assert!(apply_ramp(comp, id, Ramp::SlowDown));
        // As a project saved before the speed was remembered loads.
        comp.layer_mut(id).unwrap().time.speed_before_ramp = None;
        assert!(remove_ramp(comp, id));
        assert!(comp.layer(id).unwrap().time.is_normal());
    }

    #[test]
    fn a_ramp_keeps_the_length_and_slows_in_the_middle() {
        let (mut project, id) = clip();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        assert!(apply_ramp(comp, id, Ramp::SlowMiddle));
        let layer = comp.layer(id).unwrap();
        assert_eq!((layer.in_frame, layer.out_frame), (24, 120));
        let rate = FrameRate::new(24, 1);
        let middle = 24 + 48;
        let step = layer.source_seconds(middle + 1, rate) - layer.source_seconds(middle, rate);
        assert!(step < 0.5 / 24.0, "slow in the middle: {step}");
    }

    #[test]
    fn a_freeze_frame_holds_one_moment_and_pushes_the_rest_later() {
        let (mut project, id) = clip();
        let rate = FrameRate::new(24, 1);
        // The 96-frame clip runs from frame 24 to 120; hold frame 48 (one second in) for 36 frames.
        let hold = insert_freeze_frame(&mut project, ids::MAIN_COMP, id, 48, 36).unwrap();
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        let first = comp.layer(id).unwrap();
        assert_eq!((first.in_frame, first.out_frame), (24, 48));
        let held = comp.layer(hold).unwrap();
        assert_eq!((held.in_frame, held.out_frame), (48, 84));
        assert_eq!(held.track, first.track);
        assert!(!held.has_audio);
        for frame in [48, 60, 83] {
            assert_eq!(held.source_frame(frame, rate, rate), 24, "always the moment it froze on");
        }
        // The rest of the clip carries on from that moment, after the hold.
        let rest = comp.layers.iter().find(|layer| layer.id != id && layer.id != hold && layer.track == first.track && layer.in_frame == 84).expect("second half");
        assert_eq!(rest.out_frame, 120 + 36);
        assert_eq!(rest.source_frame(84, rate, rate), 24);
        assert!(comp.duration_frames >= 156);
        // A hold has no speed of its own and stretches freely.
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        assert!(!set_speed(comp, hold, 50.0));
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        assert_eq!(crate::trim::source_frames(&project, comp, comp.layer(hold).unwrap()), None);
    }

    #[test]
    fn a_freeze_frame_needs_footage_under_the_frame() {
        let (mut project, id) = clip();
        assert_eq!(insert_freeze_frame(&mut project, ids::MAIN_COMP, id, 500, 24), None, "past the clip");
        assert_eq!(insert_freeze_frame(&mut project, ids::MAIN_COMP, id, 30, 0), None, "no length");
        let text = project.composition(ids::MAIN_COMP).unwrap().layers.iter().find(|layer| matches!(layer.content, LayerContent::Text { .. })).unwrap().id;
        assert_eq!(insert_freeze_frame(&mut project, ids::MAIN_COMP, text, 30, 24), None, "not footage");
    }
}
