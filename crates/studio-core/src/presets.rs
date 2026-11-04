//! One-click animations for any layer, like Canva's Animate: how a layer comes in (at its in
//! point), goes out (at its out point), and what it does in between (a loop).
//!
//! Each writes ordinary keyframes (and, for blur and typing, an effect), worked out from the
//! layer's own position, scale, rotation and opacity, so the layer still ends up where it was
//! put and everything stays editable. Choosing again replaces what the last choice wrote, and
//! the layer remembers its choices so Properties can show them.

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

/// Removes the keyframes an animated value has within `window`, keeping its value there.
fn clear<T: Copy + Lerp>(animated: &mut Animated<T>, window: (i64, i64)) {
    for frame in animated.keyframe_frames().into_iter().filter(|frame| (window.0..=window.1).contains(frame)) {
        animated.remove_keyframe(frame);
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
        remove_motion(project, comp, layer_id, old);
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
    let ease = if phase == Phase::In { EASE_OUT } else { EASE_IN };
    let offset = |by: Vec2| position + by * unit;

    let Some(layer) = project.composition_mut(comp).and_then(|composition| composition.layer_mut(layer_id)) else { return false };
    match phase {
        Phase::In => layer.animation.enter = motion.map(|motion| (motion, seconds)),
        Phase::Out => layer.animation.exit = motion.map(|motion| (motion, seconds)),
    }
    let Some(motion) = motion else { return true };
    let transform = layer.transform.as_mut().expect("checked above");
    let fade = |transform: &mut crate::model::Transform, easing: Easing| replace(&mut transform.opacity, span, keys(between(0.0, opacity), easing));
    match motion {
        Motion::Fade => fade(transform, EASE_IN_OUT),
        Motion::Rise | Motion::Drop | Motion::SlideLeft | Motion::SlideRight => {
            let by = match motion {
                Motion::Rise => Vec2::new(0.0, 90.0),
                Motion::Drop => Vec2::new(0.0, -90.0),
                Motion::SlideLeft => Vec2::new(if phase == Phase::In { 240.0 } else { -240.0 }, 0.0),
                _ => Vec2::new(if phase == Phase::In { -240.0 } else { 240.0 }, 0.0),
            };
            replace(&mut transform.position, span, keys(between_pair(offset(by), position), ease));
            fade(transform, ease);
        }
        Motion::Pop => {
            let spring = if phase == Phase::In { Easing::Elastic(EaseMode::Out) } else { Easing::Elastic(EaseMode::In) };
            replace(&mut transform.scale, span, keys(between_pair(Vec2::default(), scale), spring));
        }
        Motion::Zoom => {
            replace(&mut transform.scale, span, keys(between_pair(scale * 1.6, scale), ease));
            fade(transform, ease);
        }
        Motion::Spin => {
            replace(&mut transform.rotation, span, keys(between(rotation - 180.0, rotation), ease));
            replace(&mut transform.scale, span, keys(between_pair(Vec2::default(), scale), ease));
        }
        Motion::Bounce => {
            let bounce = if phase == Phase::In { Easing::Bounce(EaseMode::Out) } else { Easing::Bounce(EaseMode::In) };
            replace(&mut transform.position, span, keys(between_pair(offset(Vec2::new(0.0, -420.0)), position), bounce));
        }
        Motion::Blur => {
            fade(transform, ease);
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
                        replace(&mut completion.value, span, keys(between(100.0, 0.0), Easing::Linear));
                    }
                }
            }
        }
    }
    true
}

/// The id of the layer's effect named `name`, adding it if it has none.
fn effect_on(project: &mut Project, comp: Id, layer_id: Id, name: &str) -> Option<Id> {
    let existing = project.composition(comp)?.layer(layer_id)?.effects.iter().find(|effect| effect.name == name).map(|effect| effect.id);
    existing.or_else(|| crate::edit::add_effect(project, comp, layer_id, name))
}

/// Takes out what an animation wrote within `window`: its keyframes on the transform and on the
/// blur and wipe effects it uses.
fn remove_motion(project: &mut Project, comp: Id, layer_id: Id, window: (i64, i64)) {
    let Some(layer) = project.composition_mut(comp).and_then(|composition| composition.layer_mut(layer_id)) else { return };
    if let Some(transform) = layer.transform.as_mut() {
        clear(&mut transform.position, window);
        clear(&mut transform.scale, window);
        clear(&mut transform.rotation, window);
        clear(&mut transform.opacity, window);
    }
    for effect in layer.effects.iter_mut().filter(|effect| matches!(effect.name.as_str(), "Gaussian Blur" | "Linear Wipe")) {
        for property in &mut effect.properties {
            clear(&mut property.value, window);
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
            clear(&mut transform.position, (start, end));
            clear(&mut transform.scale, (start, end));
            clear(&mut transform.rotation, (start, end));
            clear(&mut transform.opacity, (start, end));
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
