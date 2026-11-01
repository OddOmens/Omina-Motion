//! Changing how fast footage plays: a constant speed, playing backwards, and speed ramps
//! (keyframed speed, such as slowing down for a moment and speeding back up).
//!
//! A constant speed change keeps the clip's in point and the part of the source it shows, so
//! a slower clip gets longer and a faster one shorter. A ramp keeps the clip's length and
//! changes what plays within it.

use crate::animation::{Animated, CubicBezier, Easing, Keyframe};
use crate::model::{Composition, Id, LayerContent, LayerTime};

/// Whether a layer's speed can be changed: footage (stills and graphics have no speed).
pub fn has_speed(content: &LayerContent) -> bool {
    matches!(content, LayerContent::Footage { .. })
}

/// Sets a constant speed (a percentage), ending any ramp. The clip keeps its in point and the
/// part of the source it shows. Returns whether it changed.
pub fn set_speed(composition: &mut Composition, layer_id: Id, percent: f32) -> bool {
    let Some(layer) = composition.layer_mut(layer_id).filter(|layer| has_speed(&layer.content) && !layer.switches.locked) else { return false };
    if !percent.is_finite() {
        return false;
    }
    let percent = percent.clamp(LayerTime::SPEED_RANGE.0, LayerTime::SPEED_RANGE.1);
    let old = layer.time.constant_factor().unwrap_or(1.0);
    let new = f64::from(percent) / 100.0;
    // The source's start, as seen from the in point, stays put: what the in point shows now,
    // it shows after.
    let shown_seconds = (layer.in_frame - layer.start_frame) as f64 * old;
    let length = layer.out_frame - layer.in_frame;
    let new_length = ((length as f64 * old / new).round() as i64).max(1);
    layer.start_frame = layer.in_frame - (shown_seconds / new).round() as i64;
    layer.out_frame = layer.in_frame + new_length;
    layer.time.speed = Animated::constant(percent);
    true
}

pub fn set_reverse(composition: &mut Composition, layer_id: Id, reverse: bool) -> bool {
    let Some(layer) = composition.layer_mut(layer_id).filter(|layer| has_speed(&layer.content) && !layer.switches.locked) else { return false };
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

/// Puts a speed ramp across the clip, keeping its length.
pub fn apply_ramp(composition: &mut Composition, layer_id: Id, ramp: Ramp) -> bool {
    let Some(layer) = composition.layer_mut(layer_id).filter(|layer| has_speed(&layer.content) && !layer.switches.locked) else { return false };
    let (start, length) = (layer.in_frame, (layer.out_frame - layer.in_frame) as f64);
    let smooth = Easing::Curve(CubicBezier::new(0.45, 0.0, 0.55, 1.0));
    let keys = ramp.points().iter().map(|&(at, speed)| Keyframe { frame: start + (at * length).round() as i64, value: speed, easing: smooth }).collect();
    layer.time.speed = Animated::keyframed(keys);
    true
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
}
