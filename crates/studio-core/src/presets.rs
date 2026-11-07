//! One-click animations for any layer, like Canva's Animate: how a layer comes in (at its in
//! point), goes out (at its out point), and what it does in between (a loop).
//!
//! Each writes ordinary keyframes (and, for blur and typing, an effect), worked out from the
//! layer's own position, scale, rotation and opacity, so the layer still ends up where it was
//! put and everything stays editable. Choosing again replaces what the last choice wrote, and
//! the layer remembers its choices so the Motion panel can show them. The In and Out motions
//! each have an easing of their own (`default_easing`), which can be swapped for any other
//! (`set_motion_easing`); the choice is kept when the motion or its length changes.

use crate::animation::{Animated, CubicBezier, EaseMode, Easing, Keyframe, Lerp, Vec2};
use crate::model::{Id, Project};

/// How a layer comes in or goes out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Motion {
    Fade,
    Rise,
    Drop,
    SlideLeft,
    SlideRight,
    Pop,
    Zoom,
    Spin,
    Blur,
    Bounce,
    /// Text typed out (in) or erased (out); a wipe for other layers.
    Typewriter,
}

impl Motion {
    pub const ALL: [Self; 11] = [Self::Fade, Self::Rise, Self::Drop, Self::SlideLeft, Self::SlideRight, Self::Pop, Self::Zoom, Self::Spin, Self::Blur, Self::Bounce, Self::Typewriter];

    pub fn name(self) -> &'static str {
        match self {
            Self::Fade => "Fade",
            Self::Rise => "Rise",
            Self::Drop => "Drop",
            Self::SlideLeft => "Slide Left",
            Self::SlideRight => "Slide Right",
            Self::Pop => "Pop",
            Self::Zoom => "Zoom",
            Self::Spin => "Spin",
            Self::Blur => "Blur",
            Self::Bounce => "Bounce",
            Self::Typewriter => "Typewriter",
        }
    }
}

/// What a layer does between coming in and going out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Loop {
    Pulse,
    Float,
    Swing,
    Blink,
    Turn,
    Wiggle,
}

impl Loop {
    pub const ALL: [Self; 6] = [Self::Pulse, Self::Float, Self::Swing, Self::Blink, Self::Turn, Self::Wiggle];

    pub fn name(self) -> &'static str {
        match self {
            Self::Pulse => "Pulse",
            Self::Float => "Float",
            Self::Swing => "Swing",
            Self::Blink => "Blink",
            Self::Turn => "Turn",
            Self::Wiggle => "Wiggle",
        }
    }
}

/// Which end of the layer an animation is at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    In,
    Out,
}

/// The animations chosen for a layer, so they can be shown and replaced.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LayerAnimation {
    /// The motion and how long it takes, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enter: Option<(Motion, f32)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<(Motion, f32)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub looping: Option<Loop>,
    /// The easing picked for the In or Out motion in place of the motion's own, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enter_easing: Option<Easing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_easing: Option<Easing>,
    /// A text layer's characters, words or lines coming in, and going out (see
    /// `text_animation`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_in: Option<crate::text_animation::TextAnimator>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_out: Option<crate::text_animation::TextAnimator>,
}

impl LayerAnimation {
    pub fn is_none(&self) -> bool {
        *self == Self::default()
    }
}

/// The longest an in or out animation can take, in seconds.
pub const MAX_SECONDS: f32 = 5.0;

const EASE_OUT: Easing = Easing::Curve(CubicBezier::new(0.16, 1.0, 0.3, 1.0));
const EASE_IN: Easing = Easing::Curve(CubicBezier::new(0.7, 0.0, 0.84, 0.0));
const EASE_IN_OUT: Easing = Easing::Curve(CubicBezier::new(0.65, 0.0, 0.35, 1.0));

/// The easing a motion moves with unless another is picked: a quick start that settles (In) or
/// a slow start that speeds away (Out), with a spring for Pop, a bounce for Bounce, an even
/// pace for Typewriter and a gentle both-ends curve for Fade.
pub fn default_easing(motion: Motion, phase: Phase) -> Easing {
    let mode = if phase == Phase::In { EaseMode::Out } else { EaseMode::In };
    match motion {
        Motion::Fade => EASE_IN_OUT,
        Motion::Pop => Easing::Elastic(mode),
        Motion::Bounce => Easing::Bounce(mode),
        Motion::Typewriter => Easing::Linear,
        _ if phase == Phase::In => EASE_OUT,
        _ => EASE_IN,
    }
}

/// The easing a layer's In or Out motion moves with: the one picked for it, or the motion's
/// own. `None` when there is no motion at that end.
pub fn motion_easing(animation: &LayerAnimation, phase: Phase) -> Option<Easing> {
    let (motion, picked) = match phase {
        Phase::In => (animation.enter, animation.enter_easing),
        Phase::Out => (animation.exit, animation.exit_easing),
    };
    motion.map(|(motion, _)| picked.unwrap_or_else(|| default_easing(motion, phase)))
}

/// Keyframes made from (frame, value) pairs, `easing` on each.
fn keys<T>(points: Vec<(i64, T)>, easing: Easing) -> Vec<Keyframe<T>> {
    points.into_iter().map(|(frame, value)| Keyframe { frame, value, easing }).collect()
}

/// Puts `new` keyframes into an animated value, first removing any it had within `window`
/// (frames, inclusive), so choosing again replaces rather than piles up.
fn replace<T: Copy + Lerp>(animated: &mut Animated<T>, window: (i64, i64), new: Vec<Keyframe<T>>) {
    let mut kept: Vec<Keyframe<T>> = animated.keyframes().iter().copied().filter(|key| key.frame < window.0 || key.frame > window.1).collect();
    kept.extend(new);
    if kept.is_empty() {
        return;
    }
    *animated = Animated::keyframed(kept);
}

/// A beat every `period` frames from `start` to `end`, alternating between two values.
fn beats<T: Copy>((start, end, period): (i64, i64, i64), a: T, b: T) -> Vec<(i64, T)> {
    (start..=end).step_by(period.max(1) as usize).enumerate().map(|(index, frame)| (frame, if index % 2 == 0 { a } else { b })).collect()
}

/// Removes the keyframes an animated value has within `window`. If none are left, it keeps
/// the value it had at frame `home` (the layer's own value, where the removed animation met the
/// rest of it), rather than whichever keyframe happened to be removed last.
fn clear<T: Copy + Lerp>(animated: &mut Animated<T>, window: (i64, i64), home: i64) {
    let kept = animated.value_at(home as f64);
    for frame in animated.keyframe_frames().into_iter().filter(|frame| (window.0..=window.1).contains(frame)) {
        animated.remove_keyframe(frame);
    }
    if !animated.is_animated() {
        *animated = Animated::constant(kept);
    }
}

/// The frames an in or out animation of `seconds` covers on a layer.
fn window(layer: &crate::model::Layer, phase: Phase, seconds: f32, fps: f64) -> (i64, i64) {
    let length = ((f64::from(seconds) * fps).round() as i64).clamp(1, (layer.out_frame - layer.in_frame).max(1));
    match phase {
        Phase::In => (layer.in_frame, layer.in_frame + length),
        Phase::Out => (layer.out_frame - 1 - length, layer.out_frame - 1),
    }
}

/// Sets (or, with `None`, removes) how `layer` comes in or goes out. Returns whether it changed.
pub fn set_motion(project: &mut Project, comp: Id, layer_id: Id, phase: Phase, motion: Option<Motion>, seconds: f32) -> bool {
    let Some(composition) = project.composition(comp) else { return false };
    let Some(layer) = composition.layer(layer_id).filter(|layer| layer.transform.is_some() && !layer.switches.locked) else { return false };
    let fps = composition.frame_rate.fps();
    let unit = composition.width.min(composition.height) as f32 / 1080.0;
    let seconds = if seconds.is_finite() { seconds.clamp(0.1, MAX_SECONDS) } else { 0.6 };
    // Undo the previous choice for this end first.
    let previous = match phase {
        Phase::In => layer.animation.enter,
        Phase::Out => layer.animation.exit,
    };
    if let Some((_, old_seconds)) = previous {
        let old = window(layer, phase, old_seconds, fps);
        remove_motion(project, comp, layer_id, phase, old);
    }
    let Some(layer) = project.composition(comp).and_then(|composition| composition.layer(layer_id)) else { return false };
    let span = window(layer, phase, seconds, fps);
    // The layer's own values where the animation meets the rest of it.
    let rest = match phase {
        Phase::In => span.1,
        Phase::Out => span.0,
    } as f64;
    let transform = layer.transform.as_ref().expect("checked above");
    let (position, scale, rotation, opacity) = (transform.position.value_at(rest), transform.scale.value_at(rest), transform.rotation.value_at(rest), transform.opacity.value_at(rest));
    // From the off-screen (or invisible) state to the layer's own, for In; the reverse for Out.
    let between = |away: f32, home: f32| match phase {
        Phase::In => vec![(span.0, away), (span.1, home)],
        Phase::Out => vec![(span.0, home), (span.1, away)],
    };
    let between_pair = |away: Vec2, home: Vec2| match phase {
        Phase::In => vec![(span.0, away), (span.1, home)],
        Phase::Out => vec![(span.0, home), (span.1, away)],
    };
    let offset = |by: Vec2| position + by * unit;

    let Some(layer) = project.composition_mut(comp).and_then(|composition| composition.layer_mut(layer_id)) else { return false };
    // Taking the motion off also forgets the easing picked for it.
    let (chosen, picked) = match phase {
        Phase::In => (&mut layer.animation.enter, &mut layer.animation.enter_easing),
        Phase::Out => (&mut layer.animation.exit, &mut layer.animation.exit_easing),
    };
    *chosen = motion.map(|motion| (motion, seconds));
    if motion.is_none() {
        *picked = None;
    }
    let picked = *picked;
    let Some(motion) = motion else { return true };
    // Every keyframe this motion writes moves with the same easing.
    let ease = picked.unwrap_or_else(|| default_easing(motion, phase));
    let transform = layer.transform.as_mut().expect("checked above");
    let fade = |transform: &mut crate::model::Transform| replace(&mut transform.opacity, span, keys(between(0.0, opacity), ease));
    match motion {
        Motion::Fade => fade(transform),
        Motion::Rise | Motion::Drop | Motion::SlideLeft | Motion::SlideRight => {
            let by = match motion {
                Motion::Rise => Vec2::new(0.0, 90.0),
                Motion::Drop => Vec2::new(0.0, -90.0),
                Motion::SlideLeft => Vec2::new(if phase == Phase::In { 240.0 } else { -240.0 }, 0.0),
                _ => Vec2::new(if phase == Phase::In { -240.0 } else { 240.0 }, 0.0),
            };
            replace(&mut transform.position, span, keys(between_pair(offset(by), position), ease));
            fade(transform);
        }
        Motion::Pop => replace(&mut transform.scale, span, keys(between_pair(Vec2::default(), scale), ease)),
        Motion::Zoom => {
            replace(&mut transform.scale, span, keys(between_pair(scale * 1.6, scale), ease));
            fade(transform);
        }
        Motion::Spin => {
            replace(&mut transform.rotation, span, keys(between(rotation - 180.0, rotation), ease));
            replace(&mut transform.scale, span, keys(between_pair(Vec2::default(), scale), ease));
        }
        Motion::Bounce => replace(&mut transform.position, span, keys(between_pair(offset(Vec2::new(0.0, -420.0)), position), ease)),
        Motion::Blur => {
            fade(transform);
            let blur = effect_on(project, comp, layer_id, "Gaussian Blur");
            if let Some(property) = blur {
                let layer = project.composition_mut(comp).and_then(|composition| composition.layer_mut(layer_id)).expect("found above");
                if let Some(value) = layer.effects.iter_mut().find(|effect| effect.id == property).and_then(|effect| effect.properties.first_mut()).map(|property| &mut property.value) {
                    replace(value, span, keys(between(60.0, 0.0), ease));
                }
            }
        }
        Motion::Typewriter => {
            // Typed: a wipe uncovering it from the left at an even pace.
            let wipe = effect_on(project, comp, layer_id, "Linear Wipe");
            if let Some(effect) = wipe {
                let layer = project.composition_mut(comp).and_then(|composition| composition.layer_mut(layer_id)).expect("found above");
                if let Some(effect) = layer.effects.iter_mut().find(|candidate| candidate.id == effect) {
                    if let Some(angle) = effect.properties.get_mut(1) {
                        angle.value = Animated::constant(270.0);
                    }
                    if let Some(completion) = effect.properties.first_mut() {
                        replace(&mut completion.value, span, keys(between(100.0, 0.0), ease));
                    }
                }
            }
        }
    }
    true
}

/// Picks the easing of a layer's In or Out motion (`None` goes back to the motion's own) and
/// rewrites the motion's keyframes with it. Returns whether it changed.
pub fn set_motion_easing(project: &mut Project, comp: Id, layer_id: Id, phase: Phase, easing: Option<Easing>) -> bool {
    let Some(layer) = project.composition_mut(comp).and_then(|composition| composition.layer_mut(layer_id)).filter(|layer| !layer.switches.locked) else { return false };
    let (motion, picked) = match phase {
        Phase::In => (layer.animation.enter, &mut layer.animation.enter_easing),
        Phase::Out => (layer.animation.exit, &mut layer.animation.exit_easing),
    };
    let Some((motion, seconds)) = motion else { return false };
    *picked = easing;
    set_motion(project, comp, layer_id, phase, Some(motion), seconds)
}

/// The id of the layer's effect named `name`, adding it if it has none.
fn effect_on(project: &mut Project, comp: Id, layer_id: Id, name: &str) -> Option<Id> {
    let existing = project.composition(comp)?.layer(layer_id)?.effects.iter().find(|effect| effect.name == name).map(|effect| effect.id);
    existing.or_else(|| crate::edit::add_effect(project, comp, layer_id, name))
}

/// Takes out what an animation wrote within `window`: its keyframes on the transform and on the
/// blur and wipe effects it uses.
fn remove_motion(project: &mut Project, comp: Id, layer_id: Id, phase: Phase, window: (i64, i64)) {
    if let Some(layer) = project.composition_mut(comp).and_then(|composition| composition.layer_mut(layer_id)) {
        clear_motion_keys(layer, phase, window);
    }
}

/// Takes a layer's In or Out motion off it, keyframes included, as choosing no motion does.
/// For layers not (yet) in a composition; `fps` is the frame rate of the one it belongs to.
pub fn strip_motion(layer: &mut crate::model::Layer, phase: Phase, fps: f64) {
    let recorded = match phase {
        Phase::In => (layer.animation.enter.take(), layer.animation.enter_easing.take()).0,
        Phase::Out => (layer.animation.exit.take(), layer.animation.exit_easing.take()).0,
    };
    if let Some((_, seconds)) = recorded {
        let span = window(layer, phase, seconds, fps);
        clear_motion_keys(layer, phase, span);
    }
}

/// Removes the keyframes an In or Out motion wrote within `window`: on the transform, and on
/// the blur and wipe effects that some motions use. The layer keeps its values from the end of
/// the window that is not off screen: the end of an In motion, the start of an Out one.
fn clear_motion_keys(layer: &mut crate::model::Layer, phase: Phase, window: (i64, i64)) {
    let home = match phase {
        Phase::In => window.1,
        Phase::Out => window.0,
    };
    if let Some(transform) = layer.transform.as_mut() {
        clear(&mut transform.position, window, home);
        clear(&mut transform.scale, window, home);
        clear(&mut transform.rotation, window, home);
        clear(&mut transform.opacity, window, home);
    }
    for effect in layer.effects.iter_mut().filter(|effect| matches!(effect.name.as_str(), "Gaussian Blur" | "Linear Wipe")) {
        for property in &mut effect.properties {
            clear(&mut property.value, window, home);
        }
    }
}

/// Sets (or removes) what a layer does between coming in and going out: keyframes repeating
/// over that stretch (or a Wiggle effect). Returns whether it changed.
pub fn set_loop(project: &mut Project, comp: Id, layer_id: Id, looping: Option<Loop>) -> bool {
    let Some(composition) = project.composition(comp) else { return false };
    let Some(layer) = composition.layer(layer_id).filter(|layer| layer.transform.is_some() && !layer.switches.locked) else { return false };
    let fps = composition.frame_rate.fps();
    let unit = composition.width.min(composition.height) as f32 / 1080.0;
    // Between the in and out animations (or a little in from the ends without them).
    let gap = (fps * 0.6) as i64;
    let start = layer.animation.enter.map_or(layer.in_frame + gap, |(_, seconds)| window(layer, Phase::In, seconds, fps).1 + 1);
    let end = layer.animation.exit.map_or(layer.out_frame - 1 - gap, |(_, seconds)| window(layer, Phase::Out, seconds, fps).0 - 1);
    let previous = layer.animation.looping;
    let transform = layer.transform.as_ref().expect("checked above");
    let at = start.max(layer.in_frame) as f64;
    let (position, scale, rotation, opacity) = (transform.position.value_at(at), transform.scale.value_at(at), transform.rotation.value_at(at), transform.opacity.value_at(at));
    let Some(layer) = project.composition_mut(comp).and_then(|composition| composition.layer_mut(layer_id)) else { return false };
    layer.animation.looping = looping;
    // Undo the last loop.
    if previous.is_some() {
        if let Some(transform) = layer.transform.as_mut() {
            clear(&mut transform.position, (start, end), start);
            clear(&mut transform.scale, (start, end), start);
            clear(&mut transform.rotation, (start, end), start);
            clear(&mut transform.opacity, (start, end), start);
        }
        if previous == Some(Loop::Wiggle) {
            layer.effects.retain(|effect| effect.name != "Wiggle");
        }
    }
    let Some(looping) = looping else { return true };
    if end <= start {
        return true;
    }
    let alternate = |period: f64| (start, end, (fps * period) as i64);
    let transform = layer.transform.as_mut().expect("checked above");
    match looping {
        Loop::Pulse => replace(&mut transform.scale, (start, end), keys(beats(alternate(0.45), scale, scale * 1.07), EASE_IN_OUT)),
        Loop::Float => replace(&mut transform.position, (start, end), keys(beats(alternate(0.9), position, position + Vec2::new(0.0, -16.0 * unit)), EASE_IN_OUT)),
        Loop::Swing => replace(&mut transform.rotation, (start, end), keys(beats(alternate(0.7), rotation - 5.0, rotation + 5.0), EASE_IN_OUT)),
        Loop::Blink => replace(&mut transform.opacity, (start, end), keys(beats(alternate(0.5), opacity, opacity * 0.35), Easing::Hold)),
        Loop::Turn => {
            let turns = ((end - start) as f64 / fps / 2.0).ceil().max(1.0) as f32;
            replace(&mut transform.rotation, (start, end), keys(vec![(start, rotation), (end, rotation + 360.0 * turns)], Easing::Linear));
        }
        Loop::Wiggle => {
            crate::edit::add_effect(project, comp, layer_id, "Wiggle");
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{ids, sample_project};

    /// The title: on screen from 24 to 168, at 24 fps.
    const TITLE: Id = Id(100);

    fn transform(project: &Project) -> crate::model::Transform {
        project.composition(ids::MAIN_COMP).unwrap().layer(TITLE).unwrap().transform.clone().unwrap()
    }

    #[test]
    fn taking_a_motion_off_leaves_the_layer_as_it_was() {
        for phase in [Phase::In, Phase::Out] {
            let mut project = sample_project();
            let layer = crate::edit::add_text_layer(&mut project, ids::MAIN_COMP, "Hi", crate::animation::Vec2::new(500.0, 500.0), 0).unwrap();
            let opacity = |project: &Project| project.composition(ids::MAIN_COMP).unwrap().layer(layer).unwrap().transform.as_ref().unwrap().opacity.clone();
            assert!(set_motion(&mut project, ids::MAIN_COMP, layer, phase, Some(Motion::Fade), 0.5));
            assert!(set_motion(&mut project, ids::MAIN_COMP, layer, phase, None, 0.5));
            assert_eq!(opacity(&project), Animated::constant(100.0), "{phase:?}: fully seen again, not stuck at the faded value");
        }
    }

    #[test]
    fn rising_in_starts_lower_and_ends_where_the_layer_was() {
        let mut project = sample_project();
        // Clear the sample's own fades, so only the preset's keyframes are there.
        let layer = project.composition_mut(ids::MAIN_COMP).unwrap().layer_mut(TITLE).unwrap();
        layer.transform.as_mut().unwrap().opacity = Animated::constant(100.0);
        // Half a second in at 24 fps: the rise ends on frame 36, where the title is at `home`.
        let home = transform(&project).position.value_at(36.0);
        assert!(set_motion(&mut project, ids::MAIN_COMP, TITLE, Phase::In, Some(Motion::Rise), 0.5));
        let after = transform(&project);
        assert!(after.position.value_at(24.0).y > home.y, "starts below");
        assert_eq!(after.position.value_at(36.0), home, "and lands where it was");
        assert_eq!(after.opacity.value_at(24.0), 0.0);
        // Choosing another replaces it rather than piling up.
        set_motion(&mut project, ids::MAIN_COMP, TITLE, Phase::In, Some(Motion::Fade), 0.5);
        assert!(transform(&project).position.value_at(24.0).y <= home.y + 0.5, "no rise any more");
        set_motion(&mut project, ids::MAIN_COMP, TITLE, Phase::In, None, 0.5);
        assert_eq!(transform(&project).opacity.value_at(24.0), 100.0, "and removing takes it all out");
    }

    #[test]
    fn a_picked_easing_rewrites_the_motion_and_outlives_changing_it() {
        let mut project = sample_project();
        let layer = project.composition_mut(ids::MAIN_COMP).unwrap().layer_mut(TITLE).unwrap();
        layer.transform.as_mut().unwrap().opacity = Animated::constant(100.0);
        let animation = |project: &Project| project.composition(ids::MAIN_COMP).unwrap().layer(TITLE).unwrap().animation;
        let first_position_easing = |project: &Project| transform(project).position.keyframes()[0].easing;
        // Without a motion there is nothing to ease.
        assert!(!set_motion_easing(&mut project, ids::MAIN_COMP, TITLE, Phase::In, Some(Easing::Linear)));
        set_motion(&mut project, ids::MAIN_COMP, TITLE, Phase::In, Some(Motion::Rise), 0.5);
        assert_eq!(first_position_easing(&project), default_easing(Motion::Rise, Phase::In));
        // A bounce picked for it lands on its keyframes, and is what the layer reports.
        let bounce = Easing::Bounce(EaseMode::Out);
        assert!(set_motion_easing(&mut project, ids::MAIN_COMP, TITLE, Phase::In, Some(bounce)));
        assert_eq!(first_position_easing(&project), bounce);
        assert_eq!(motion_easing(&animation(&project), Phase::In), Some(bounce));
        // A longer slide keeps it.
        set_motion(&mut project, ids::MAIN_COMP, TITLE, Phase::In, Some(Motion::SlideLeft), 1.2);
        assert_eq!(first_position_easing(&project), bounce);
        assert_eq!(transform(&project).position.keyframes().len(), 2, "the old slide's keyframes were replaced");
        // Going back to the motion's own easing, and taking the motion off forgets the pick.
        set_motion_easing(&mut project, ids::MAIN_COMP, TITLE, Phase::In, None);
        assert_eq!(first_position_easing(&project), default_easing(Motion::SlideLeft, Phase::In));
        set_motion_easing(&mut project, ids::MAIN_COMP, TITLE, Phase::In, Some(bounce));
        set_motion(&mut project, ids::MAIN_COMP, TITLE, Phase::In, None, 0.5);
        assert_eq!(animation(&project).enter_easing, None);
        assert_eq!(motion_easing(&animation(&project), Phase::In), None);
    }

    #[test]
    fn layers_saved_before_picked_easings_still_load() {
        let animation: LayerAnimation = serde_json::from_str(r#"{"enter": ["Rise", 0.5]}"#).unwrap();
        assert_eq!(animation.enter_easing, None);
        assert_eq!(motion_easing(&animation, Phase::In), Some(default_easing(Motion::Rise, Phase::In)));
    }

    #[test]
    fn going_out_ends_at_the_out_point_and_loops_fill_the_middle() {
        let mut project = sample_project();
        let layer = project.composition_mut(ids::MAIN_COMP).unwrap().layer_mut(TITLE).unwrap();
        layer.transform.as_mut().unwrap().opacity = Animated::constant(100.0);
        set_motion(&mut project, ids::MAIN_COMP, TITLE, Phase::Out, Some(Motion::Pop), 0.5);
        let scale = transform(&project).scale;
        assert_eq!(scale.value_at(167.0), Vec2::default(), "gone by its last frame");
        assert_eq!(scale.value_at(150.0), Vec2::new(100.0, 100.0));
        set_loop(&mut project, ids::MAIN_COMP, TITLE, Some(Loop::Pulse));
        let scale = transform(&project).scale;
        let sizes: Vec<f32> = (40..140).map(|frame| scale.value_at(frame as f64).x).collect();
        assert!(sizes.iter().any(|size| *size > 105.0), "pulses larger in the middle");
        assert_eq!(scale.value_at(167.0), Vec2::default(), "the out animation is untouched");
    }
}
