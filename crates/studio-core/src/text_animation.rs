//! Text animators, as in After Effects: a text layer's characters, words or lines come in (at
//! its in point) or go out (at its out point) one after another, each moving on its own.
//!
//! Unlike the one-click layer animations (`presets`), nothing is written as keyframes: a layer
//! keeps its In and Out animators in `LayerAnimation`, and the renderer asks `layer_poses` how
//! each piece of the text sits at a moment. The pieces are timed so the whole animation takes
//! the chosen time, however much text there is; `overlap` says how much each piece's movement
//! overlaps the next one's.

use std::ops::Range;

use crate::animation::{CubicBezier, EaseMode, Easing, Lerp, Vec2};
use crate::model::{Id, Layer, LayerContent, Project};
use crate::presets::Phase;
use crate::time::FrameRate;

/// The pieces the text moves in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TextUnit {
    #[default]
    Character,
    Word,
    Line,
}

impl TextUnit {
    pub const ALL: [Self; 3] = [Self::Character, Self::Word, Self::Line];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Character => "Character",
            Self::Word => "Word",
            Self::Line => "Line",
        }
    }
}

/// How each piece moves while it comes in (or, reversed, goes out).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TextMotion {
    Fade,
    /// Up from a little below.
    #[default]
    Rise,
    /// Down from a little above.
    Drop,
    SlideLeft,
    SlideRight,
    /// Grows from nothing.
    Pop,
    /// Shrinks into place from large, fading in.
    Zoom,
    /// Turns into place, growing.
    Spin,
    /// Appears all at once, as if typed (or disappears, as if erased).
    Typewriter,
}

impl TextMotion {
    pub const ALL: [Self; 9] = [Self::Fade, Self::Rise, Self::Drop, Self::SlideLeft, Self::SlideRight, Self::Pop, Self::Zoom, Self::Spin, Self::Typewriter];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Fade => "Fade",
            Self::Rise => "Rise",
            Self::Drop => "Drop",
            Self::SlideLeft => "Slide Left",
            Self::SlideRight => "Slide Right",
            Self::Pop => "Pop",
            Self::Zoom => "Zoom",
            Self::Spin => "Spin",
            Self::Typewriter => "Typewriter",
        }
    }
}

/// Which piece moves first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TextOrder {
    /// From the first piece to the last.
    #[default]
    Forward,
    Backward,
    /// From the middle outwards.
    FromCenter,
    /// In a shuffled order (the same every time for a layer).
    Random,
}

impl TextOrder {
    pub const ALL: [Self; 4] = [Self::Forward, Self::Backward, Self::FromCenter, Self::Random];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Forward => "First to Last",
            Self::Backward => "Last to First",
            Self::FromCenter => "From the Center",
            Self::Random => "Random",
        }
    }
}

/// How each piece's movement speeds up and slows down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TextEase {
    /// Settles gently (coming in), or sets off gently (going out).
    #[default]
    Smooth,
    Linear,
    /// Goes a little past, then settles back.
    Overshoot,
    Bounce,
}

impl TextEase {
    pub const ALL: [Self; 4] = [Self::Smooth, Self::Linear, Self::Overshoot, Self::Bounce];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Smooth => "Smooth",
            Self::Linear => "Linear",
            Self::Overshoot => "Overshoot",
            Self::Bounce => "Bounce",
        }
    }

    /// The easing curve for coming in (towards the text's place) or going out (away from it).
    fn easing(self, phase: Phase) -> Easing {
        match (self, phase) {
            (Self::Smooth, Phase::In) => Easing::Curve(CubicBezier::new(0.16, 1.0, 0.3, 1.0)),
            (Self::Smooth, Phase::Out) => Easing::Curve(CubicBezier::new(0.7, 0.0, 0.84, 0.0)),
            (Self::Linear, _) => Easing::Linear,
            (Self::Overshoot, Phase::In) => Easing::Curve(CubicBezier::new(0.34, 1.56, 0.64, 1.0)),
            (Self::Overshoot, Phase::Out) => Easing::Curve(CubicBezier::new(0.36, 0.0, 0.66, -0.56)),
            (Self::Bounce, Phase::In) => Easing::Bounce(EaseMode::Out),
            (Self::Bounce, Phase::Out) => Easing::Bounce(EaseMode::In),
        }
    }
}

/// The longest a text animation can take, and the shortest, in seconds.
pub const MAX_SECONDS: f32 = 10.0;
pub const MIN_SECONDS: f32 = 0.1;

/// One text animation: how the text's pieces come in or go out.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TextAnimator {
    pub motion: TextMotion,
    pub unit: TextUnit,
    pub order: TextOrder,
    pub ease: TextEase,
    /// How long the whole animation takes, first piece to last, in seconds.
    pub seconds: f32,
    /// How much each piece's movement overlaps the next one's, from 0 (one after another) to
    /// 1 (all at once).
    pub overlap: f32,
}

impl Default for TextAnimator {
    fn default() -> Self {
        Self { motion: TextMotion::Rise, unit: TextUnit::Character, order: TextOrder::Forward, ease: TextEase::Smooth, seconds: 1.0, overlap: 0.8 }
    }
}

/// How one piece of text sits at a moment: how see-through it is, how far it is from its
/// place (in the text's own pixels), and its size and turn about its middle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitPose {
    pub opacity: f32,
    pub offset: Vec2,
    pub scale: f32,
    /// Degrees, clockwise.
    pub rotation: f32,
}

impl UnitPose {
    /// In its place, as the text is drawn without animation.
    pub const HOME: Self = Self { opacity: 1.0, offset: Vec2::new(0.0, 0.0), scale: 1.0, rotation: 0.0 };

    fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            opacity: self.opacity.lerp(other.opacity, t).clamp(0.0, 1.0),
            offset: self.offset.lerp(other.offset, t),
            scale: self.scale.lerp(other.scale, t).max(0.0),
            rotation: self.rotation.lerp(other.rotation, t),
        }
    }
}

impl TextAnimator {
    /// The same animator with its numbers kept in range.
    pub fn sanitized(self) -> Self {
        let seconds = if self.seconds.is_finite() { self.seconds.clamp(MIN_SECONDS, MAX_SECONDS) } else { 1.0 };
        let overlap = if self.overlap.is_finite() { self.overlap.clamp(0.0, 1.0) } else { 0.8 };
        Self { seconds, overlap, ..self }
    }

    /// Where a piece is before it comes in (or after it goes out), for text `em` pixels tall.
    fn away(self, phase: Phase, em: f32) -> UnitPose {
        let hidden = UnitPose { opacity: 0.0, ..UnitPose::HOME };
        // Slides travel the way they are named, coming in and going out.
        let across = if phase == Phase::In { 1.0 } else { -1.0 };
        match self.motion {
            TextMotion::Fade | TextMotion::Typewriter => hidden,
            TextMotion::Rise => UnitPose { offset: Vec2::new(0.0, 0.5 * em * across), ..hidden },
            TextMotion::Drop => UnitPose { offset: Vec2::new(0.0, -0.5 * em * across), ..hidden },
            TextMotion::SlideLeft => UnitPose { offset: Vec2::new(0.8 * em * across, 0.0), ..hidden },
            TextMotion::SlideRight => UnitPose { offset: Vec2::new(-0.8 * em * across, 0.0), ..hidden },
            TextMotion::Pop => UnitPose { scale: 0.0, ..UnitPose::HOME },
            TextMotion::Zoom => UnitPose { scale: 2.5, ..hidden },
            TextMotion::Spin => UnitPose { scale: 0.2, rotation: -180.0 * across, ..hidden },
        }
    }

    /// How piece `index` of `count` sits `elapsed` seconds into this animation (`phase` says
    /// whether it is coming in or going out). `seed` shuffles the Random order.
    pub fn pose(self, phase: Phase, index: usize, count: usize, elapsed: f32, em: f32, seed: u64) -> UnitPose {
        let animator = self.sanitized();
        let (rank, steps) = rank(animator.order, index, count, seed);
        // Each piece takes `length`, and the next one starts `spacing` later, so that the last
        // one finishes exactly at the end.
        let length = animator.seconds / (1.0 + steps * (1.0 - animator.overlap));
        let spacing = length * (1.0 - animator.overlap);
        let progress = ((elapsed - rank * spacing) / length.max(f32::EPSILON)).clamp(0.0, 1.0);
        let away = animator.away(phase, em);
        if animator.motion == TextMotion::Typewriter {
            // Each piece appears (or disappears) the moment its turn comes.
            let shown = match phase {
                Phase::In => progress > 0.0,
                Phase::Out => progress <= 0.0,
            };
            return if shown { UnitPose::HOME } else { away };
        }
        let eased = animator.ease.easing(phase).apply(progress);
        match phase {
            Phase::In => away.lerp(UnitPose::HOME, eased),
            Phase::Out => UnitPose::HOME.lerp(away, eased),
        }
    }
}

/// Where piece `index` of `count` comes in the order, and how many steps the order has (its
/// last rank). From the center, pieces the same distance from the middle move together.
fn rank(order: TextOrder, index: usize, count: usize, seed: u64) -> (f32, f32) {
    let last = count.saturating_sub(1) as f32;
    let index_f = index as f32;
    match order {
        TextOrder::Forward => (index_f, last),
        TextOrder::Backward => (last - index_f, last),
        TextOrder::FromCenter => {
            let middle = last * 0.5;
            ((index_f - middle).abs().round(), middle.ceil())
        }
        TextOrder::Random => {
            // A shuffled order: pieces sorted by a hash of their index and the seed.
            let key = |i: usize| scramble(seed ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
            let mine = key(index);
            let before = (0..count).filter(|&other| (key(other), other) < (mine, index)).count();
            (before as f32, last)
        }
    }
}

/// A well-mixed hash of `value` (SplitMix64's finalizer).
fn scramble(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// The pieces of `text` that move on their own, as byte ranges: each character, word or line
/// (spaces and line breaks don't move; there is nothing to see in them). A piece never
/// crosses a line break.
pub fn units(text: &str, unit: TextUnit) -> Vec<Range<usize>> {
    let mut pieces = Vec::new();
    let mut line_start = 0;
    for line in text.split('\n') {
        match unit {
            TextUnit::Character => pieces.extend(line.char_indices().filter(|(_, c)| !c.is_whitespace()).map(|(at, c)| line_start + at..line_start + at + c.len_utf8())),
            TextUnit::Word => {
                let mut start = None;
                for (at, c) in line.char_indices().chain(std::iter::once((line.len(), ' '))) {
                    match (c.is_whitespace(), start) {
                        (false, None) => start = Some(at),
                        (true, Some(from)) => {
                            pieces.push(line_start + from..line_start + at);
                            start = None;
                        }
                        _ => {}
                    }
                }
            }
            TextUnit::Line => {
                let trimmed = line.trim_end();
                let leading = trimmed.len() - trimmed.trim_start().len();
                if !trimmed.trim_start().is_empty() {
                    pieces.push(line_start + leading..line_start + trimmed.len());
                }
            }
        }
        line_start += line.len() + 1;
    }
    pieces
}

/// The animation running on `layer` at `frame` (fractional frames allowed), if one is: the
/// animator and how far into it the moment is, in seconds. The In animation starts at the
/// layer's in point; the Out animation ends at its out point. Before or after them the text
/// sits still and is drawn as usual.
pub fn running(layer: &Layer, frame: f64, rate: FrameRate) -> Option<(TextAnimator, Phase, f32)> {
    let fps = rate.fps();
    let since_in = ((frame - layer.in_frame as f64) / fps) as f32;
    let length = ((layer.out_frame - layer.in_frame) as f64 / fps) as f32;
    if let Some(animator) = layer.animation.text_in.map(TextAnimator::sanitized)
        && (0.0..animator.seconds).contains(&since_in)
    {
        return Some((animator, Phase::In, since_in));
    }
    if let Some(animator) = layer.animation.text_out.map(TextAnimator::sanitized) {
        let into = since_in - (length - animator.seconds);
        if into >= 0.0 {
            return Some((animator, Phase::Out, into));
        }
    }
    None
}

/// How each piece of `layer`'s text `text` sits at `frame`, when an animation is running then:
/// the pieces (byte ranges) and their poses. `em` is the font size.
pub fn layer_poses(layer: &Layer, text: &str, em: f32, frame: f64, rate: FrameRate) -> Option<Vec<(Range<usize>, UnitPose)>> {
    let (animator, phase, elapsed) = running(layer, frame, rate)?;
    let pieces = units(text, animator.unit);
    let count = pieces.len();
    Some(pieces.into_iter().enumerate().map(|(index, range)| (range, animator.pose(phase, index, count, elapsed, em, layer.id.0))).collect())
}

/// Sets (or, with `None`, removes) how text layer `layer_id`'s text comes in or goes out.
/// Returns whether it changed; locked layers and other kinds of layer are left alone.
pub fn set_animator(project: &mut Project, comp: Id, layer_id: Id, phase: Phase, animator: Option<TextAnimator>) -> bool {
    let Some(layer) = project.composition_mut(comp).and_then(|composition| composition.layer_mut(layer_id)) else { return false };
    if layer.switches.locked || !matches!(layer.content, LayerContent::Text { .. }) {
        return false;
    }
    let slot = match phase {
        Phase::In => &mut layer.animation.text_in,
        Phase::Out => &mut layer.animation.text_out,
    };
    let animator = animator.map(TextAnimator::sanitized);
    let changed = *slot != animator;
    *slot = animator;
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animators_are_saved_with_the_project() {
        use crate::sample::{ids, sample_project};
        let mut project = sample_project();
        let text = project.composition(ids::MAIN_COMP).unwrap().layers.iter().find(|layer| matches!(layer.content, LayerContent::Text { .. })).unwrap().id;
        let animator = TextAnimator { motion: TextMotion::Spin, unit: TextUnit::Line, order: TextOrder::Random, ease: TextEase::Bounce, seconds: 2.0, overlap: 0.25 };
        set_animator(&mut project, ids::MAIN_COMP, text, Phase::Out, Some(animator));
        let view = crate::project_file::EditorView { active_composition: ids::MAIN_COMP, current_frame: 0, work_area: crate::time::FrameRange::new(0.0, 10.0) };
        let saved = crate::project_file::to_text(&project, view).unwrap();
        let (loaded, _) = crate::project_file::from_text(&saved).unwrap();
        assert_eq!(loaded.composition(ids::MAIN_COMP).unwrap().layer(text).unwrap().animation.text_out, Some(animator));
    }

    #[test]
    fn only_unlocked_text_layers_take_animators() {
        use crate::sample::{ids, sample_project};
        let mut project = sample_project();
        let comp = &project.composition(ids::MAIN_COMP).unwrap();
        let text = comp.layers.iter().find(|layer| matches!(layer.content, LayerContent::Text { .. })).unwrap().id;
        let other = comp.layers.iter().find(|layer| !matches!(layer.content, LayerContent::Text { .. })).unwrap().id;
        let wild = TextAnimator { seconds: 99.0, overlap: -3.0, ..TextAnimator::default() };
        assert!(set_animator(&mut project, ids::MAIN_COMP, text, Phase::In, Some(wild)));
        let stored = project.composition(ids::MAIN_COMP).unwrap().layer(text).unwrap().animation.text_in.unwrap();
        assert_eq!((stored.seconds, stored.overlap), (MAX_SECONDS, 0.0), "kept in range");
        assert!(!set_animator(&mut project, ids::MAIN_COMP, other, Phase::In, Some(TextAnimator::default())), "not text");
        assert!(set_animator(&mut project, ids::MAIN_COMP, text, Phase::In, None));
        assert_eq!(project.composition(ids::MAIN_COMP).unwrap().layer(text).unwrap().animation.text_in, None);
    }

    #[test]
    fn text_splits_into_characters_words_and_lines() {
        let text = "Big  sale\nnow on";
        assert_eq!(units(text, TextUnit::Character).len(), 12, "letters only, no spaces or breaks");
        let words: Vec<&str> = units(text, TextUnit::Word).into_iter().map(|range| &text[range]).collect();
        assert_eq!(words, ["Big", "sale", "now", "on"]);
        let lines: Vec<&str> = units(text, TextUnit::Line).into_iter().map(|range| &text[range]).collect();
        assert_eq!(lines, ["Big  sale", "now on"]);
        assert!(units("", TextUnit::Word).is_empty());
        assert_eq!(units("é!", TextUnit::Character), [0..2, 2..3], "whole characters, not bytes");
    }

    #[test]
    fn pieces_come_in_one_after_another_and_finish_together_at_the_end() {
        let animator = TextAnimator { motion: TextMotion::Fade, ease: TextEase::Linear, seconds: 1.0, overlap: 0.0, ..TextAnimator::default() };
        let opacity = |index, elapsed| animator.pose(Phase::In, index, 4, elapsed, 40.0, 0).opacity;
        // Four pieces, no overlap: each takes a quarter second in turn.
        assert_eq!(opacity(0, 0.0), 0.0);
        assert!((opacity(0, 0.125) - 0.5).abs() < 0.01, "the first is halfway in");
        assert_eq!(opacity(1, 0.125), 0.0, "the second hasn't started");
        assert_eq!(opacity(0, 0.3), 1.0);
        assert!((opacity(3, 0.875) - 0.5).abs() < 0.01, "the last is halfway in at seven eighths");
        assert_eq!(opacity(3, 1.0), 1.0, "and all are in at the end");
        // Overlapping fully, they all move at once.
        let together = TextAnimator { overlap: 1.0, ..animator };
        assert_eq!(together.pose(Phase::In, 0, 4, 0.5, 40.0, 0), together.pose(Phase::In, 3, 4, 0.5, 40.0, 0));
    }

    #[test]
    fn going_out_reverses_coming_in() {
        let animator = TextAnimator { motion: TextMotion::Rise, ease: TextEase::Linear, overlap: 1.0, ..TextAnimator::default() };
        let start = animator.pose(Phase::Out, 0, 3, 0.0, 40.0, 0);
        let end = animator.pose(Phase::Out, 0, 3, animator.seconds, 40.0, 0);
        assert_eq!(start, UnitPose::HOME, "it leaves from its place");
        assert_eq!(end.opacity, 0.0);
        assert!(end.offset.y < 0.0, "rising out moves up: {:?}", end.offset);
        let coming = animator.pose(Phase::In, 0, 3, 0.0, 40.0, 0);
        assert!(coming.offset.y > 0.0, "rising in comes up from below: {:?}", coming.offset);
    }

    #[test]
    fn orders_choose_which_piece_moves_first() {
        let first = |order| {
            let animator = TextAnimator { motion: TextMotion::Fade, ease: TextEase::Linear, overlap: 0.0, order, ..TextAnimator::default() };
            (0..5).find(|&index| animator.pose(Phase::In, index, 5, 0.1, 40.0, 7).opacity > 0.0)
        };
        assert_eq!(first(TextOrder::Forward), Some(0));
        assert_eq!(first(TextOrder::Backward), Some(4));
        assert_eq!(first(TextOrder::FromCenter), Some(2));
        // Random is a shuffle: every rank is used once.
        let mut ranks: Vec<f32> = (0..5).map(|index| rank(TextOrder::Random, index, 5, 7).0).collect();
        ranks.sort_by(f32::total_cmp);
        assert_eq!(ranks, [0.0, 1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn the_typewriter_shows_each_piece_whole_when_its_turn_comes() {
        let animator = TextAnimator { motion: TextMotion::Typewriter, overlap: 0.0, ..TextAnimator::default() };
        let shown = |index| animator.pose(Phase::In, index, 4, 0.3, 40.0, 0).opacity;
        assert_eq!((shown(0), shown(1), shown(2)), (1.0, 1.0, 0.0), "typed up to the second piece");
    }

    #[test]
    fn the_in_animation_runs_from_the_in_point_and_the_out_from_before_the_out_point() {
        let mut layer = crate::sample::sample_project().compositions[0].layers[0].clone();
        layer.in_frame = 30;
        layer.out_frame = 150;
        let rate = FrameRate::new(30, 1);
        assert_eq!(running(&layer, 40.0, rate), None, "no animators");
        layer.animation.text_in = Some(TextAnimator { seconds: 1.0, ..TextAnimator::default() });
        layer.animation.text_out = Some(TextAnimator { seconds: 0.5, ..TextAnimator::default() });
        assert!(running(&layer, 45.0, rate).is_some_and(|(_, phase, at)| phase == Phase::In && (at - 0.5).abs() < 0.01));
        assert_eq!(running(&layer, 90.0, rate), None, "still in the middle");
        assert!(running(&layer, 140.0, rate).is_some_and(|(_, phase, at)| phase == Phase::Out && (at - 0.1667).abs() < 0.01));
    }
}
