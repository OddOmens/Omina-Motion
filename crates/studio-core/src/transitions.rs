//! Transitions at cuts: what happens where one clip ends and the next begins.
//!
//! Some (cross dissolve, zoom, push, slide, spin, whip) need both clips on screen at once: the
//! second clip is started a little earlier (using footage it has before its in point), or the
//! first runs a little longer, and the second moves to a track above so it plays over the first.
//! Others (dips, flash, blur, glitch) happen on each side of the cut and need no overlap.
//! Everything is written as keyframes and effects on the two clips, so it can be adjusted or
//! undone like any edit. With only one clip (the start or end of a sequence), its half alone
//! is applied.

use crate::animation::{Animated, CubicBezier, Easing, Keyframe, Vec2};
use crate::model::{Composition, Id, Layer, LayerContent, Project};
use crate::presets::{Motion, Phase, set_motion};

/// The transitions offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    CrossDissolve,
    DipToBlack,
    Flash,
    Zoom,
    Push,
    Slide,
    Spin,
    Blur,
    Glitch,
    Whip,
}

impl Transition {
    pub const ALL: [Self; 10] = [Self::CrossDissolve, Self::DipToBlack, Self::Flash, Self::Zoom, Self::Push, Self::Slide, Self::Spin, Self::Blur, Self::Glitch, Self::Whip];

    pub fn name(self) -> &'static str {
        match self {
            Self::CrossDissolve => "Cross Dissolve",
            Self::DipToBlack => "Dip to Black",
            Self::Flash => "Flash",
            Self::Zoom => "Zoom",
            Self::Push => "Push",
            Self::Slide => "Slide",
            Self::Spin => "Spin",
            Self::Blur => "Blur",
            Self::Glitch => "Glitch",
            Self::Whip => "Whip Pan",
        }
    }

    /// Whether both clips must be on screen together.
    fn overlaps(self) -> bool {
        matches!(self, Self::CrossDissolve | Self::Zoom | Self::Push | Self::Slide | Self::Spin | Self::Whip)
    }
}

/// The clip on `layer`'s track that ends where it starts (the one before the cut at its in
/// point), or that starts where it ends (after the cut at its out point).
pub fn touching(composition: &Composition, layer: &Layer, at_start: bool) -> Option<Id> {
    composition
        .layers
        .iter()
        .filter(|other| other.id != layer.id && other.track == layer.track)
        .find(|other| if at_start { other.out_frame == layer.in_frame } else { other.in_frame == layer.out_frame })
        .map(|other| other.id)
}

/// The earliest frame a layer's in point can go back to: its source's start, or anywhere for
/// stills, text and graphics.
fn earliest_in(project: &Project, composition: &Composition, layer: &Layer) -> i64 {
    match crate::trim::source_frames(project, composition, layer) {
        Some(_) => layer.start_frame,
        None => i64::MIN,
    }
}

/// Makes `before` and `after` overlap by `frames` across their cut, with `after` above: by
/// starting `after` earlier if it has the footage, or else running `before` longer. Returns
/// the overlap (first frame, frame after the last), or `None` if neither has spare footage.
fn overlap(project: &mut Project, comp: Id, before: Id, after: Id, frames: i64) -> Option<(i64, i64)> {
    let composition = project.composition(comp)?;
    let (first, second) = (composition.layer(before)?, composition.layer(after)?);
    if first.switches.locked || second.switches.locked {
        return None;
    }
    let cut = second.in_frame;
    let can_start_earlier = earliest_in(project, composition, second) <= cut - frames;
    let first_end = crate::trim::source_frames(project, composition, first).map_or(i64::MAX, |length| first.start_frame + length);
    let can_run_longer = first.out_frame + frames <= first_end;
    let track = first.track;
    let composition = project.composition_mut(comp)?;
    let span = if can_start_earlier {
        composition.layer_mut(after)?.in_frame = cut - frames;
        (cut - frames, cut)
    } else if can_run_longer {
        composition.layer_mut(before)?.out_frame = cut + frames;
        (cut, cut + frames)
    } else {
        return None;
    };
    // The second clip plays over the first: on a track of its own, just above.
    let in_frame = composition.layer(after)?.in_frame;
    crate::tracks::place_clip(composition, after, track, in_frame);
    Some(span)
}

/// Keyframes over `span` for an animated value, from `from` to `to`, replacing any there.
fn sweep<T: Copy + crate::animation::Lerp>(animated: &mut Animated<T>, span: (i64, i64), from: T, to: T) {
    let easing = Easing::Curve(CubicBezier::new(0.65, 0.0, 0.35, 1.0));
    let mut keys: Vec<Keyframe<T>> = animated.keyframes().iter().copied().filter(|key| key.frame < span.0 || key.frame > span.1).collect();
    keys.push(Keyframe { frame: span.0, value: from, easing });
    keys.push(Keyframe { frame: span.1, value: to, easing: Easing::Linear });
    *animated = Animated::keyframed(keys);
}

/// Animates setting `index` of effect `name` on a layer (adding the effect) through `values`
/// at frames.
fn effect_keys(project: &mut Project, comp: Id, layer: Id, name: &str, index: usize, values: &[(i64, f32)]) {
    let existing = project.composition(comp).and_then(|composition| composition.layer(layer)).and_then(|found| found.effects.iter().find(|effect| effect.name == name).map(|effect| effect.id));
    let Some(effect) = existing.or_else(|| crate::edit::add_effect(project, comp, layer, name)) else { return };
    let Some(found) = project.composition_mut(comp).and_then(|composition| composition.layer_mut(layer)) else { return };
    let Some(property) = found.effects.iter_mut().find(|candidate| candidate.id == effect).and_then(|effect| effect.properties.get_mut(index)) else { return };
    let easing = Easing::Curve(CubicBezier::new(0.65, 0.0, 0.35, 1.0));
    let mut keys: Vec<Keyframe<f32>> = property.value.keyframes().iter().copied().filter(|key| !values.iter().any(|(frame, _)| *frame == key.frame)).collect();
    keys.extend(values.iter().map(|&(frame, value)| Keyframe { frame, value, easing }));
    property.value = Animated::keyframed(keys);
}

/// Puts `transition` on the cut between `before` and `after` (either may be missing: the start
/// or end of a sequence), lasting `seconds`. Returns whether anything changed.
pub fn apply(project: &mut Project, comp: Id, before: Option<Id>, after: Option<Id>, transition: Transition, seconds: f32) -> bool {
    let Some(composition) = project.composition(comp) else { return false };
    let fps = composition.frame_rate.fps();
    let width = composition.width as f32;
    let frames = ((f64::from(seconds.clamp(0.1, 4.0)) * fps).round() as i64).max(2);
    let half = (seconds * 0.5).max(0.05);
    let has_picture = |id: Option<Id>| id.and_then(|id| composition.layer(id)).is_some_and(|layer| layer.transform.is_some() && !matches!(layer.content, LayerContent::Audio { .. }));
    let (before, after) = (before.filter(|_| has_picture(before)), after.filter(|_| has_picture(after)));
    if before.is_none() && after.is_none() {
        return false;
    }
    // Both clips on screen together, where the transition needs it and the footage allows.
    let span = match (before, after) {
        (Some(first), Some(second)) if transition.overlaps() => overlap(project, comp, first, second, frames),
        _ => None,
    };
    let Some((start, end)) = span else {
        // Each side on its own: out of the first, into the second.
        let (out_motion, in_motion) = match transition {
            Transition::CrossDissolve | Transition::DipToBlack => (Motion::Fade, Motion::Fade),
            Transition::Zoom => (Motion::Zoom, Motion::Zoom),
            Transition::Push | Transition::Slide | Transition::Whip => (Motion::SlideLeft, Motion::SlideLeft),
            Transition::Spin => (Motion::Spin, Motion::Spin),
            Transition::Blur => (Motion::Blur, Motion::Blur),
            Transition::Flash | Transition::Glitch => (Motion::Fade, Motion::Fade),
        };
        let mut changed = false;
        if let Some(first) = before {
            changed |= match transition {
                Transition::Flash => flash(project, comp, first, false, frames / 2),
                Transition::Glitch => glitch(project, comp, first, false, frames / 2),
                _ => set_motion(project, comp, first, Phase::Out, Some(out_motion), half),
            };
        }
        if let Some(second) = after {
            changed |= match transition {
                Transition::Flash => flash(project, comp, second, true, frames / 2),
                Transition::Glitch => glitch(project, comp, second, true, frames / 2),
                _ => set_motion(project, comp, second, Phase::In, Some(in_motion), half),
            };
        }
        return changed;
    };
    let (first, second) = (before.expect("overlap needs both"), after.expect("overlap needs both"));
    let overlap_seconds = ((end - start) as f64 / fps) as f32;
    let end = end - 1;
    match transition {
        Transition::CrossDissolve => {
            set_motion(project, comp, second, Phase::In, Some(Motion::Fade), overlap_seconds);
        }
        Transition::Zoom => {
            set_motion(project, comp, first, Phase::Out, Some(Motion::Zoom), overlap_seconds);
            set_motion(project, comp, second, Phase::In, Some(Motion::Fade), overlap_seconds);
        }
        Transition::Spin => {
            set_motion(project, comp, second, Phase::In, Some(Motion::Spin), overlap_seconds);
        }
        Transition::Push | Transition::Slide | Transition::Whip => {
            // The second comes in from the right edge; for a push the first goes out to the left.
            let composition = project.composition_mut(comp).expect("found above");
            let shove = Vec2::new(width, 0.0);
            if let Some(transform) = composition.layer_mut(second).and_then(|layer| layer.transform.as_mut()) {
                let home = transform.position.value_at(end as f64);
                sweep(&mut transform.position, (start, end), home + shove, home);
            }
            if transition != Transition::Slide
                && let Some(transform) = composition.layer_mut(first).and_then(|layer| layer.transform.as_mut())
            {
                let home = transform.position.value_at(start as f64);
                sweep(&mut transform.position, (start, end), home, home - shove);
            }
            if transition == Transition::Whip {
                // A streak of motion blur across the move.
                let middle = (start + end) / 2;
                for layer in [first, second] {
                    effect_keys(project, comp, layer, "Directional Blur", 1, &[(start, 0.0), (middle, 120.0), (end, 0.0)]);
                }
            }
        }
        // These never overlap: they were applied side by side above.
        Transition::DipToBlack | Transition::Flash | Transition::Blur | Transition::Glitch => return false,
    }
    true
}

/// A flash of white at the cut: the clip brightens to white (or from it, coming in).
fn flash(project: &mut Project, comp: Id, layer: Id, coming_in: bool, frames: i64) -> bool {
    let Some(found) = project.composition(comp).and_then(|composition| composition.layer(layer)) else { return false };
    let (in_frame, out_frame) = (found.in_frame, found.out_frame - 1);
    let values = if coming_in { [(in_frame, 4.0), (in_frame + frames, 0.0)] } else { [(out_frame - frames, 0.0), (out_frame, 4.0)] };
    effect_keys(project, comp, layer, "Exposure", 0, &values);
    true
}

/// A burst of glitching at the cut.
fn glitch(project: &mut Project, comp: Id, layer: Id, coming_in: bool, frames: i64) -> bool {
    let Some(found) = project.composition(comp).and_then(|composition| composition.layer(layer)) else { return false };
    let (in_frame, out_frame) = (found.in_frame, found.out_frame - 1);
    let values = if coming_in { [(in_frame, 90.0), (in_frame + frames, 0.0)] } else { [(out_frame - frames, 0.0), (out_frame, 90.0)] };
    effect_keys(project, comp, layer, "Glitch", 0, &values);
    effect_keys(project, comp, layer, "RGB Split", 0, &values.map(|(frame, value)| (frame, value * 0.3)));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{self, FootageInfo};
    use crate::sample::{ids, sample_project};
    use crate::time::FrameRate;

    /// Two 4-second clips back to back on one track, each showing its middle two seconds: A at
    /// 0–48, B at 48–96. Both have a second of footage either side to spare.
    fn two_clips() -> (Project, Id, Id) {
        let mut project = sample_project();
        let info = FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: 4.0, frame_rate: FrameRate::new(24, 1), has_audio: false };
        let ids_made: Vec<Id> = (0..2)
            .map(|index| {
                let item = edit::import_footage(&mut project, format!("/media/{index}.mp4").into(), &info);
                edit::add_layer_from_item(&mut project, ids::MAIN_COMP, item, 0, 0).unwrap()
            })
            .collect();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        for (index, &id) in ids_made.iter().enumerate() {
            let layer = comp.layer_mut(id).unwrap();
            layer.start_frame = index as i64 * 48 - 24;
            layer.in_frame = index as i64 * 48;
            layer.out_frame = index as i64 * 48 + 48;
            layer.track = 40;
        }
        (project, ids_made[0], ids_made[1])
    }

    #[test]
    fn a_cross_dissolve_overlaps_the_clips_and_fades_the_second_in_on_top() {
        let (mut project, a, b) = two_clips();
        assert!(apply(&mut project, ids::MAIN_COMP, Some(a), Some(b), Transition::CrossDissolve, 1.0));
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        let (first, second) = (comp.layer(a).unwrap(), comp.layer(b).unwrap());
        assert_eq!(second.in_frame, 24, "B starts a second early, from its spare footage");
        assert!(second.track > first.track, "on a track above A");
        assert_eq!(second.transform.as_ref().unwrap().opacity.value_at(24.0), 0.0);
        assert_eq!(second.transform.as_ref().unwrap().opacity.value_at(48.0), 100.0);
        assert_eq!(touching(comp, first, false), None, "A ends where B fully shows");
    }

    #[test]
    fn a_dip_to_black_fades_each_side_without_moving_anything() {
        let (mut project, a, b) = two_clips();
        assert!(apply(&mut project, ids::MAIN_COMP, Some(a), Some(b), Transition::DipToBlack, 1.0));
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        let (first, second) = (comp.layer(a).unwrap(), comp.layer(b).unwrap());
        assert_eq!((first.out_frame, second.in_frame, second.track), (48, 48, first.track), "nothing moved");
        assert_eq!(first.transform.as_ref().unwrap().opacity.value_at(47.0), 0.0);
        assert_eq!(second.transform.as_ref().unwrap().opacity.value_at(48.0), 0.0);
    }

    #[test]
    fn a_push_slides_both_and_needs_no_footage_to_fall_back() {
        let (mut project, a, b) = two_clips();
        // No spare footage either side: the push falls back to each side sliding on its own.
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        comp.layer_mut(b).unwrap().start_frame = 48;
        comp.layer_mut(a).unwrap().start_frame = -48;
        assert!(apply(&mut project, ids::MAIN_COMP, Some(a), Some(b), Transition::Push, 1.0));
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        assert_eq!(comp.layer(b).unwrap().in_frame, 48, "B could not start earlier");
        assert!(comp.layer(b).unwrap().animation.enter.is_some());
    }
}
