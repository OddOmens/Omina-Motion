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

/// How a segment moves from one keyframe to the next.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Easing {
    Linear,
    /// Keeps the first keyframe's value until the next keyframe, then jumps.
    Hold,
    Curve(CubicBezier),
}

impl Easing {
    pub fn apply(self, t: f32) -> f32 {
        match self {
            Self::Linear => t,
            Self::Hold => 0.0,
            Self::Curve(curve) => curve.ease(t),
        }
    }
}

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

impl<T> From<AnimatedFile<T>> for Animated<T> {
    fn from(file: AnimatedFile<T>) -> Self {
        let mut keyframes = file.keyframes;
        keyframes.sort_by_key(|keyframe| keyframe.frame);
        keyframes.dedup_by_key(|keyframe| keyframe.frame);
        Self { value: file.value, keyframes }
    }
}

impl<T: Lerp> Animated<T> {
    pub fn constant(value: T) -> Self {
        Self { value, keyframes: Vec::new() }
    }

    /// Builds an animated value from keyframes in any order. The first keyframe also becomes the static value.
    pub fn keyframed(mut keyframes: Vec<Keyframe<T>>) -> Self {
        keyframes.sort_by_key(|keyframe| keyframe.frame);
        keyframes.dedup_by_key(|keyframe| keyframe.frame);
        let value = keyframes.first().map(|keyframe| keyframe.value).expect("at least one keyframe");
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
            return self.value;
        };
        if frame <= first.frame as f64 {
            return first.value;
        }
        if frame >= last.frame as f64 {
            return last.value;
        }
        // Keyframes are sorted, so the segment containing `frame` is the first whose end lies past it.
        let pair = self
            .keyframes
            .windows(2)
            .find(|pair| frame < pair[1].frame as f64)
            .expect("frame lies strictly between the first and last keyframe");
        let (from, to) = (pair[0], pair[1]);
        let t = ((frame - from.frame as f64) / (to.frame - from.frame) as f64) as f32;
        from.value.lerp(to.value, from.easing.apply(t))
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
            self.keyframes.push(Keyframe::linear(frame, self.value));
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
        self.value = f(self.value);
        for keyframe in &mut self.keyframes {
            keyframe.value = f(keyframe.value);
        }
    }

    /// Moves every keyframe by `delta` frames, keeping their spacing.
    pub fn shift(&mut self, delta: i64) {
        for keyframe in &mut self.keyframes {
            keyframe.frame += delta;
        }
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
