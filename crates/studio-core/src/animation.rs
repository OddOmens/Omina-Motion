//! Animatable property values. A property either holds one value or a list of
//! keyframes. Each keyframe carries the easing of the segment that leaves it,
//! so curves can be set per segment, as in the Flow plugin for After Effects.

use std::ops::{Add, Mul, Sub};

/// A 2D value such as a position, anchor point or scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// The vector's length.
    pub fn length(self) -> f32 {
        self.x.hypot(self.y)
    }
}

impl Add for Vec2 {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y)
    }
}

impl Sub for Vec2 {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Self;
    fn mul(self, factor: f32) -> Self {
        Self::new(self.x * factor, self.y * factor)
    }
}

/// Values that can be blended between two keyframes. `t` runs from 0 (at `self`) to 1 (at `other`)
/// and may go past either end for curves that overshoot.
pub trait Lerp: Copy {
    fn lerp(self, other: Self, t: f32) -> Self;
}

/// Values an animated property can hold: anything that blends between two keyframes. Every
/// `Lerp` value is one; so is a shape's outline (see `morph`), which is too large to copy and
/// needs its points matched up before it can blend.
pub trait Blend: Clone {
    fn blend(&self, other: &Self, t: f32) -> Self;
}

impl<T: Lerp> Blend for T {
    fn blend(&self, other: &Self, t: f32) -> Self {
        self.lerp(*other, t)
    }
}

impl Lerp for f32 {
    fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }
}

impl Lerp for Vec2 {
    fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }
}

/// A CSS-style cubic Bézier timing curve from (0, 0) to (1, 1) with control points
/// (x1, y1) and (x2, y2). The x values are clamped to 0–1 so time never runs backwards;
/// the y values may leave 0–1 to overshoot.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CubicBezier {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl CubicBezier {
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    /// Point on one axis of the curve at curve parameter `s`, for control values `a` and `b`.
    fn axis(a: f32, b: f32, s: f32) -> f32 {
        let inv = 1.0 - s;
        3.0 * inv * inv * s * a + 3.0 * inv * s * s * b + s * s * s
    }

    /// The eased progress for linear progress `t` (0–1).
    pub fn ease(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let (x1, x2) = (self.x1.clamp(0.0, 1.0), self.x2.clamp(0.0, 1.0));
        // x(s) is monotonic for x values in 0–1, so bisection always finds the parameter for `t`.
        let (mut low, mut high) = (0.0_f32, 1.0_f32);
        for _ in 0..32 {
            let mid = (low + high) * 0.5;
            if Self::axis(x1, x2, mid) < t {
                low = mid;
            } else {
                high = mid;
            }
        }
        Self::axis(self.y1, self.y2, (low + high) * 0.5)
    }
}

/// Which end of a segment an easing shapes: the start, the end, or both, as in the classic
/// "ease in / ease out / ease in-out" families.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum EaseMode {
    In,
    Out,
    Both,
}

impl EaseMode {
    pub const ALL: [Self; 3] = [Self::In, Self::Out, Self::Both];

    pub const fn label(self) -> &'static str {
        match self {
            Self::In => "In",
            Self::Out => "Out",
            Self::Both => "Both",
        }
    }
}

/// How a segment moves from one keyframe to the next.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Easing {
    Linear,
    /// Keeps the first keyframe's value until the next keyframe, then jumps.
    Hold,
    Curve(CubicBezier),
    /// Overshoots and springs back, settling at the end (or winding up at the start).
    Elastic(EaseMode),
    /// Bounces against the end value like a dropped ball (or off the start).
    Bounce(EaseMode),
}

impl Easing {
    pub fn apply(self, t: f32) -> f32 {
        match self {
            Self::Linear => t,
            Self::Hold => 0.0,
            Self::Curve(curve) => curve.ease(t),
            Self::Elastic(mode) => shaped(mode, t, elastic_out),
            Self::Bounce(mode) => shaped(mode, t, bounce_out),
        }
    }
}

/// An easing from its "out" form: "in" is the same played backwards, "both" is "in" for the
/// first half and "out" for the second.
fn shaped(mode: EaseMode, t: f32, out: fn(f32) -> f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match mode {
        EaseMode::Out => out(t),
        EaseMode::In => 1.0 - out(1.0 - t),
        EaseMode::Both if t < 0.5 => (1.0 - out(1.0 - 2.0 * t)) * 0.5,
        EaseMode::Both => (1.0 + out(2.0 * t - 1.0)) * 0.5,
    }
}

/// The standard elastic ease-out: a decaying oscillation around the end value.
fn elastic_out(t: f32) -> f32 {
    if t <= 0.0 || t >= 1.0 {
        return t.clamp(0.0, 1.0);
    }
    let period = std::f32::consts::TAU / 3.0;
    2f32.powf(-10.0 * t) * ((t * 10.0 - 0.75) * period).sin() + 1.0
}

/// The standard bounce ease-out: four parabolic bounces of shrinking height.
fn bounce_out(t: f32) -> f32 {
    const N: f32 = 7.5625;
    const D: f32 = 2.75;
    if t < 1.0 / D {
        N * t * t
    } else if t < 2.0 / D {
        let t = t - 1.5 / D;
        N * t * t + 0.75
    } else if t < 2.5 / D {
        let t = t - 2.25 / D;
        N * t * t + 0.9375
    } else {
        let t = t - 2.625 / D;
        N * t * t + 0.984_375
    }
}

/// The classic easing families as curves, for the graph editor: each with its In, Out and
/// Both (in-out) forms. Elastic and Bounce, which a single curve can't draw, are `Easing`
/// variants of their own.
pub const EASING_FAMILIES: &[(&str, [CubicBezier; 3])] = &[
    ("Quad", [CubicBezier::new(0.11, 0.0, 0.5, 0.0), CubicBezier::new(0.5, 1.0, 0.89, 1.0), CubicBezier::new(0.45, 0.0, 0.55, 1.0)]),
    ("Cubic", [CubicBezier::new(0.32, 0.0, 0.67, 0.0), CubicBezier::new(0.33, 1.0, 0.68, 1.0), CubicBezier::new(0.65, 0.0, 0.35, 1.0)]),
    ("Quart", [CubicBezier::new(0.5, 0.0, 0.75, 0.0), CubicBezier::new(0.25, 1.0, 0.5, 1.0), CubicBezier::new(0.76, 0.0, 0.24, 1.0)]),
    ("Quint", [CubicBezier::new(0.64, 0.0, 0.78, 0.0), CubicBezier::new(0.22, 1.0, 0.36, 1.0), CubicBezier::new(0.83, 0.0, 0.17, 1.0)]),
    ("Sine", [CubicBezier::new(0.12, 0.0, 0.39, 0.0), CubicBezier::new(0.61, 1.0, 0.88, 1.0), CubicBezier::new(0.37, 0.0, 0.63, 1.0)]),
    ("Expo", [CubicBezier::new(0.7, 0.0, 0.84, 0.0), CubicBezier::new(0.16, 1.0, 0.3, 1.0), CubicBezier::new(0.87, 0.0, 0.13, 1.0)]),
    ("Circ", [CubicBezier::new(0.55, 0.0, 1.0, 0.45), CubicBezier::new(0.0, 0.55, 0.45, 1.0), CubicBezier::new(0.85, 0.0, 0.15, 1.0)]),
    ("Back", [CubicBezier::new(0.36, 0.0, 0.66, -0.56), CubicBezier::new(0.34, 1.56, 0.64, 1.0), CubicBezier::new(0.68, -0.6, 0.32, 1.6)]),
];

/// After Effects' Easy Ease (F9): a symmetric slow-in, slow-out curve.
pub const EASY_EASE: CubicBezier = CubicBezier::new(0.33, 0.0, 0.67, 1.0);

/// Named curves for the Flow panel.
pub const EASING_PRESETS: &[(&str, CubicBezier)] = &[
    ("Ease", CubicBezier::new(0.25, 0.1, 0.25, 1.0)),
    ("Ease In", CubicBezier::new(0.42, 0.0, 1.0, 1.0)),
    ("Ease Out", CubicBezier::new(0.0, 0.0, 0.58, 1.0)),
    ("Ease In Out", CubicBezier::new(0.42, 0.0, 0.58, 1.0)),
    ("Expo Out", CubicBezier::new(0.16, 1.0, 0.3, 1.0)),
    ("Expo In Out", CubicBezier::new(0.87, 0.0, 0.13, 1.0)),
    ("Quint Out", CubicBezier::new(0.22, 1.0, 0.36, 1.0)),
    ("Back Out", CubicBezier::new(0.34, 1.56, 0.64, 1.0)),
    ("Back In Out", CubicBezier::new(0.68, -0.6, 0.32, 1.6)),
];

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Keyframe<T> {
    pub frame: i64,
    pub value: T,
    /// Easing of the segment from this keyframe to the next one.
    pub easing: Easing,
}

impl<T> Keyframe<T> {
    pub fn linear(frame: i64, value: T) -> Self {
        Self { frame, value, easing: Easing::Linear }
    }
}

/// A property value that may change over time.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(from = "AnimatedFile<T>")]
pub struct Animated<T> {
    /// The value used when there are no keyframes.
    pub value: T,
    /// Keyframes sorted by frame, never two on one frame. Empty means not animated.
    keyframes: Vec<Keyframe<T>>,
}

/// How an animated value is read from a project file. Keyframes from a file may be in any
/// order or doubled up; converting restores the rule that they are sorted and one per frame.
#[derive(serde::Deserialize)]
struct AnimatedFile<T> {
    value: T,
    keyframes: Vec<Keyframe<T>>,
}

/// Reads a number that may or may not be keyframed: project files from before a setting could
/// be animated hold it as a bare number, and later ones as an animated value.
pub fn number_or_animated<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Animated<f32>, D::Error> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Saved {
        Number(f32),
        Animated(Animated<f32>),
    }
    Ok(match <Saved as serde::Deserialize>::deserialize(deserializer)? {
        Saved::Number(value) => Animated::constant(value),
        Saved::Animated(animated) => animated,
    })
}

impl<T> From<AnimatedFile<T>> for Animated<T> {
    fn from(file: AnimatedFile<T>) -> Self {
        let mut keyframes = file.keyframes;
        keyframes.sort_by_key(|keyframe| keyframe.frame);
        keyframes.dedup_by_key(|keyframe| keyframe.frame);
        Self { value: file.value, keyframes }
    }
}

impl<T: Blend> Animated<T> {
    pub fn constant(value: T) -> Self {
        Self { value, keyframes: Vec::new() }
    }

    /// Builds an animated value from keyframes in any order. The first keyframe also becomes the static value.
    pub fn keyframed(mut keyframes: Vec<Keyframe<T>>) -> Self {
        keyframes.sort_by_key(|keyframe| keyframe.frame);
        keyframes.dedup_by_key(|keyframe| keyframe.frame);
        let value = keyframes.first().map(|keyframe| keyframe.value.clone()).expect("at least one keyframe");
        Self { value, keyframes }
    }

    pub fn is_animated(&self) -> bool {
        !self.keyframes.is_empty()
    }

    pub fn keyframes(&self) -> &[Keyframe<T>] {
        &self.keyframes
    }

    /// The frames that have keyframes, in order.
    pub fn keyframe_frames(&self) -> Vec<i64> {
        self.keyframes.iter().map(|keyframe| keyframe.frame).collect()
    }

    fn index_of(&self, frame: i64) -> Result<usize, usize> {
        self.keyframes.binary_search_by_key(&frame, |keyframe| keyframe.frame)
    }

    pub fn has_keyframe_at(&self, frame: i64) -> bool {
        self.index_of(frame).is_ok()
    }

    /// The value at `frame`: held before the first and after the last keyframe, eased in between.
    pub fn value_at(&self, frame: f64) -> T {
        let (Some(first), Some(last)) = (self.keyframes.first(), self.keyframes.last()) else {
            return self.value.clone();
        };
        if frame <= first.frame as f64 {
            return first.value.clone();
        }
        if frame >= last.frame as f64 {
            return last.value.clone();
        }
        // Keyframes are sorted, so the segment containing `frame` is the first whose end lies past it.
        let pair = self
            .keyframes
            .windows(2)
            .find(|pair| frame < pair[1].frame as f64)
            .expect("frame lies strictly between the first and last keyframe");
        let (from, to) = (&pair[0], &pair[1]);
        // In floating point: subtracting frame numbers as integers could overflow on extreme frames.
        let t = ((frame - from.frame as f64) / (to.frame as f64 - from.frame as f64)) as f32;
        from.value.blend(&to.value, from.easing.apply(t))
    }

    /// Sets the value at `frame`. Animated properties gain or update a keyframe there
    /// (auto-keyframing, as in After Effects); others simply change their value.
    pub fn set_at(&mut self, frame: i64, value: T) {
        if !self.is_animated() {
            self.value = value;
            return;
        }
        match self.index_of(frame) {
            Ok(index) => self.keyframes[index].value = value,
            Err(index) => {
                // A new keyframe inside a segment keeps that segment's easing on both halves.
                let easing = index.checked_sub(1).map_or(Easing::Linear, |previous| self.keyframes[previous].easing);
                self.keyframes.insert(index, Keyframe { frame, value, easing });
            }
        }
    }

    /// The stopwatch: turning it on adds a keyframe at `frame` with the current value; turning it off
    /// removes all keyframes and keeps the value the property has at `frame`.
    pub fn toggle_stopwatch(&mut self, frame: i64) {
        if self.is_animated() {
            self.value = self.value_at(frame as f64);
            self.keyframes.clear();
        } else {
            self.keyframes.push(Keyframe::linear(frame, self.value.clone()));
        }
    }

    /// Adds a keyframe at `frame` holding the current value there, or removes the keyframe already there.
    /// Removing the last keyframe turns animation off and keeps that value.
    pub fn toggle_keyframe(&mut self, frame: i64) {
        match self.index_of(frame) {
            Ok(index) => {
                let removed = self.keyframes.remove(index);
                if self.keyframes.is_empty() {
                    self.value = removed.value;
                }
            }
            Err(_) if !self.is_animated() => self.toggle_stopwatch(frame),
            Err(_) => {
                let value = self.value_at(frame as f64);
                self.set_at(frame, value);
            }
        }
    }

    pub fn remove_keyframe(&mut self, frame: i64) {
        if let Ok(index) = self.index_of(frame) {
            let removed = self.keyframes.remove(index);
            if self.keyframes.is_empty() {
                self.value = removed.value;
            }
        }
    }

    /// Moves the keyframe on `from` to `to`. Refused (returns false) if `to` already has a keyframe.
    pub fn move_keyframe(&mut self, from: i64, to: i64) -> bool {
        if from == to {
            return true;
        }
        if self.has_keyframe_at(to) {
            return false;
        }
        let Ok(index) = self.index_of(from) else { return false };
        let mut keyframe = self.keyframes.remove(index);
        keyframe.frame = to;
        let insert_at = self.index_of(to).unwrap_err();
        self.keyframes.insert(insert_at, keyframe);
        true
    }

    pub fn set_easing(&mut self, frame: i64, easing: Easing) {
        if let Ok(index) = self.index_of(frame) {
            self.keyframes[index].easing = easing;
        }
    }

    pub fn easing_at(&self, frame: i64) -> Option<Easing> {
        self.index_of(frame).ok().map(|index| self.keyframes[index].easing)
    }

    /// Each keyframe's frame and the easing of the segment leaving it.
    pub fn keyframe_easings(&self) -> Vec<(i64, Easing)> {
        self.keyframes.iter().map(|keyframe| (keyframe.frame, keyframe.easing)).collect()
    }

    /// Replaces the static value and every keyframe value with `f(value)`.
    pub fn map_values(&mut self, f: impl Fn(T) -> T) {
        self.value = f(self.value.clone());
        for keyframe in &mut self.keyframes {
            keyframe.value = f(keyframe.value.clone());
        }
    }

    /// Moves every keyframe by `delta` frames, keeping their spacing.
    pub fn shift(&mut self, delta: i64) {
        for keyframe in &mut self.keyframes {
            keyframe.frame += delta;
        }
    }

    /// Whether the static value and every keyframe's value pass `check` (for checking files).
    pub fn is_sound_with(&self, check: impl Fn(&T) -> bool) -> bool {
        check(&self.value) && self.keyframes.iter().all(|keyframe| check(&keyframe.value))
    }

    /// The nearest keyframe frame before `frame`, if any.
    pub fn previous_keyframe(&self, frame: i64) -> Option<i64> {
        self.keyframes.iter().rev().map(|keyframe| keyframe.frame).find(|&candidate| candidate < frame)
    }

    /// The nearest keyframe frame after `frame`, if any.
    pub fn next_keyframe(&self, frame: i64) -> Option<i64> {
        self.keyframes.iter().map(|keyframe| keyframe.frame).find(|&candidate| candidate > frame)
    }
}

#[cfg(test)]
mod easing_family_tests {
    use super::*;

    #[test]
    fn elastic_and_bounce_start_and_end_on_their_keyframes() {
        for mode in EaseMode::ALL {
            for easing in [Easing::Elastic(mode), Easing::Bounce(mode)] {
                assert!(easing.apply(0.0).abs() < 1e-4, "{easing:?} starts at 0");
                assert!((easing.apply(1.0) - 1.0).abs() < 1e-4, "{easing:?} ends at 1");
            }
        }
        // Both forms pass through the middle.
        assert!((Easing::Bounce(EaseMode::Both).apply(0.5) - 0.5).abs() < 1e-4);
        assert!((Easing::Elastic(EaseMode::Both).apply(0.5) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn elastic_overshoots_and_bounce_stays_inside() {
        let samples = |easing: Easing| (0..=200).map(move |i| easing.apply(i as f32 / 200.0));
        assert!(samples(Easing::Elastic(EaseMode::Out)).any(|v| v > 1.01), "elastic springs past the end");
        assert!(samples(Easing::Bounce(EaseMode::Out)).all(|v| (-1e-4..=1.0001).contains(&v)), "bounce never passes the end");
    }

    #[test]
    fn in_is_out_played_backwards() {
        for t in [0.1_f32, 0.3, 0.7, 0.9] {
            let (ease_in, ease_out) = (Easing::Bounce(EaseMode::In).apply(t), Easing::Bounce(EaseMode::Out).apply(1.0 - t));
            assert!((ease_in - (1.0 - ease_out)).abs() < 1e-5);
        }
    }

    #[test]
    fn every_family_runs_from_start_to_end() {
        for (name, curves) in EASING_FAMILIES {
            for curve in curves {
                assert!(curve.ease(0.0).abs() < 1e-3 && (curve.ease(1.0) - 1.0).abs() < 1e-3, "{name} {curve:?}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fade() -> Animated<f32> {
        Animated::keyframed(vec![Keyframe::linear(20, 100.0), Keyframe::linear(10, 0.0)])
    }

    #[test]
    fn constant_values_never_change() {
        let opacity = Animated::constant(50.0_f32);
        assert!(!opacity.is_animated());
        assert_eq!(opacity.value_at(1000.0), 50.0);
    }

    #[test]
    fn keyframes_hold_outside_and_interpolate_inside() {
        let opacity = fade();
        assert_eq!(opacity.keyframes()[0].frame, 10, "keyframes are sorted");
        assert_eq!(opacity.value_at(0.0), 0.0);
        assert_eq!(opacity.value_at(15.0), 50.0);
        assert_eq!(opacity.value_at(30.0), 100.0);
    }

    #[test]
    fn vectors_interpolate_per_component() {
        let position = Animated::keyframed(vec![
            Keyframe::linear(0, Vec2::new(0.0, 100.0)),
            Keyframe::linear(10, Vec2::new(100.0, 0.0)),
        ]);
        assert_eq!(position.value_at(5.0), Vec2::new(50.0, 50.0));
    }

    #[test]
    fn curves_match_their_endpoints_and_shape() {
        let ease_out = CubicBezier::new(0.0, 0.0, 0.58, 1.0);
        assert!(ease_out.ease(0.0).abs() < 1e-4);
        assert!((ease_out.ease(1.0) - 1.0).abs() < 1e-4);
        assert!(ease_out.ease(0.5) > 0.5, "ease out is ahead of linear at the midpoint");
        let back_out = CubicBezier::new(0.34, 1.56, 0.64, 1.0);
        assert!((0..=100).any(|step| back_out.ease(step as f32 / 100.0) > 1.0), "back out overshoots");
    }

    #[test]
    fn segment_easing_shapes_interpolation() {
        let mut opacity = fade();
        opacity.set_easing(10, Easing::Hold);
        assert_eq!(opacity.value_at(19.0), 0.0);
        opacity.set_easing(10, Easing::Curve(CubicBezier::new(0.42, 0.0, 1.0, 1.0)));
        assert!(opacity.value_at(15.0) < 50.0, "ease in starts slowly");
    }

    #[test]
    fn setting_values_auto_keyframes_animated_properties() {
        let mut still = Animated::constant(1.0_f32);
        still.set_at(5, 2.0);
        assert_eq!((still.value_at(0.0), still.is_animated()), (2.0, false));

        let mut animated = fade();
        animated.set_easing(10, Easing::Hold);
        animated.set_at(15, 80.0);
        assert_eq!(animated.keyframes().len(), 3);
        assert_eq!(animated.easing_at(15), Some(Easing::Hold), "a split segment keeps its easing");
        animated.set_at(10, 5.0);
        assert_eq!(animated.keyframes().len(), 3, "an existing keyframe is updated, not duplicated");
    }

    #[test]
    fn stopwatch_and_keyframe_toggles() {
        let mut value = Animated::constant(7.0_f32);
        value.toggle_stopwatch(12);
        assert!(value.has_keyframe_at(12));
        value.toggle_keyframe(30);
        assert_eq!(value.keyframes().len(), 2);
        value.toggle_keyframe(30);
        value.toggle_keyframe(12);
        assert!(!value.is_animated());
        assert_eq!(value.value, 7.0, "removing the last keyframe keeps its value");

        let mut faded = fade();
        faded.toggle_stopwatch(15);
        assert_eq!((faded.is_animated(), faded.value), (false, 50.0), "stopwatch off keeps the current value");
    }

    #[test]
    fn keyframes_move_without_colliding() {
        let mut opacity = fade();
        assert!(opacity.move_keyframe(20, 40));
        assert_eq!(opacity.next_keyframe(10), Some(40));
        assert!(!opacity.move_keyframe(10, 40), "cannot land on another keyframe");
        assert_eq!(opacity.previous_keyframe(40), Some(10));
    }
}
