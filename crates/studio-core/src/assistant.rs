//! The Motion Assistant: a set of plain rules that read a composition and point out things an
//! editor or animator would usually want to fix, each with a one-click fix where there is one.
//! Nothing here guesses or learns: every suggestion comes from a rule that can be switched
//! off, says what it measured, and the two keyframe rules that depend on a number (how short a
//! move is "too fast", how close keyframes are "nearly lined up") take it from `Options`.
//!
//! The rules, by area:
//! - **Motion** (this file): motion at a constant speed, moves that are too fast, keyframes
//!   that do nothing, keyframes outside their clip, and keyframes nearly lined up.
//! - **Editing** (`clips`): gaps of a few frames between clips, clips only a few frames long,
//!   and jump cuts inside one shot.
//! - **Audio** (`clips`): sound that starts abruptly, music at full volume under a voice, and
//!   sound boosted past full volume.
//! - **Text** (`clips`): text on screen too briefly to read, and text running off the frame.
//! - **Picture** (`clips`): footage and images scaled up past their own size.

mod clips;

pub use clips::clipping;

use serde::{Deserialize, Serialize};

use crate::animation::{EASY_EASE, Easing};
use crate::model::{Composition, Id, Layer};
use crate::property::{PropertyPath, PropertyValue, TransformProperty};

/// The part of the work a rule is about. Suggestions are grouped and colored by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Area {
    Motion,
    Editing,
    Audio,
    Text,
    Picture,
}

impl Area {
    pub const ALL: [Self; 5] = [Self::Motion, Self::Editing, Self::Audio, Self::Text, Self::Picture];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Motion => "Motion",
            Self::Editing => "Editing",
            Self::Audio => "Audio",
            Self::Text => "Text",
            Self::Picture => "Picture",
        }
    }
}

/// One of the assistant's rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Check {
    ConstantSpeed,
    FastMove,
    RedundantKeyframes,
    HiddenKeyframes,
    NearlyAligned,
    TinyGap,
    ShortClip,
    JumpCut,
    AbruptSound,
    MusicOverVoice,
    Clipping,
    BriefText,
    TextOffFrame,
    Upscaled,
}

impl Check {
    pub const ALL: [Self; 14] = [
        Self::ConstantSpeed,
        Self::FastMove,
        Self::RedundantKeyframes,
        Self::HiddenKeyframes,
        Self::NearlyAligned,
        Self::TinyGap,
        Self::ShortClip,
        Self::JumpCut,
        Self::AbruptSound,
        Self::MusicOverVoice,
        Self::Clipping,
        Self::BriefText,
        Self::TextOffFrame,
        Self::Upscaled,
    ];

    /// The part of the work the rule is about.
    pub const fn area(self) -> Area {
        match self {
            Self::ConstantSpeed | Self::FastMove | Self::RedundantKeyframes | Self::HiddenKeyframes | Self::NearlyAligned => Area::Motion,
            Self::TinyGap | Self::ShortClip | Self::JumpCut => Area::Editing,
            Self::AbruptSound | Self::MusicOverVoice | Self::Clipping => Area::Audio,
            Self::BriefText | Self::TextOffFrame => Area::Text,
            Self::Upscaled => Area::Picture,
        }
    }

    /// The rule's name, as shown beside its switch.
    pub const fn name(self) -> &'static str {
        match self {
            Self::ConstantSpeed => "Constant-speed motion",
            Self::FastMove => "Moves that are too fast",
            Self::RedundantKeyframes => "Keyframes that do nothing",
            Self::HiddenKeyframes => "Keyframes outside their clip",
            Self::NearlyAligned => "Keyframes nearly lined up",
            Self::TinyGap => "Tiny gaps between clips",
            Self::ShortClip => "Clips a few frames long",
            Self::JumpCut => "Jump cuts in one shot",
            Self::AbruptSound => "Sound that starts abruptly",
            Self::MusicOverVoice => "Music as loud as a voice",
            Self::Clipping => "Sound boosted too far",
            Self::BriefText => "Text shown too briefly",
            Self::TextOffFrame => "Text off the frame",
            Self::Upscaled => "Pictures scaled up",
        }
    }

    /// What the rule looks for, in a sentence.
    pub const fn about(self) -> &'static str {
        match self {
            Self::ConstantSpeed => "Position, scale, rotation and path animation with no easing at all.",
            Self::FastMove => "A change between two keyframes that is over in very few frames.",
            Self::RedundantKeyframes => "A keyframe with the same value as the keyframes before and after it.",
            Self::HiddenKeyframes => "Keyframes before a clip starts or after it ends, which never show.",
            Self::NearlyAligned => "A keyframe a frame or so away from keyframes on other layers.",
            Self::TinyGap => "A gap of one to three frames between two clips on a track, which shows as a flash.",
            Self::ShortClip => "A clip under six frames long, usually a sliver left over from trimming.",
            Self::JumpCut => "Two pieces of the same footage side by side with under a second missing between them.",
            Self::AbruptSound => "A sound clip trimmed to start partway in, with no fade in, which can click.",
            Self::MusicOverVoice => "A long sound clip playing under a voice without getting quieter for it.",
            Self::Clipping => "A clip whose volume is raised until its loudest moment distorts.",
            Self::BriefText => "A title or caption gone before it can be read at a comfortable pace.",
            Self::TextOffFrame => "A text layer that reaches past the edge of the picture.",
            Self::Upscaled => "Footage or an image scaled past its own size, which looks soft.",
        }
    }
}

/// Which rules run, and the numbers two of them use. Saved with the app's settings; anything
/// missing from an older settings file takes its default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub constant_speed: bool,
    pub fast_moves: bool,
    pub redundant_keyframes: bool,
    pub hidden_keyframes: bool,
    pub nearly_aligned: bool,
    pub tiny_gaps: bool,
    pub short_clips: bool,
    pub jump_cuts: bool,
    pub abrupt_sound: bool,
    pub music_over_voice: bool,
    pub clipping: bool,
    pub brief_text: bool,
    pub text_off_frame: bool,
    pub upscaled: bool,
    /// A move shorter than this many frames counts as too fast.
    pub shortest_move: i64,
    /// Keyframes on different layers this many frames apart or fewer count as nearly lined up.
    pub aligned_within: i64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            constant_speed: true,
            fast_moves: true,
            redundant_keyframes: true,
            hidden_keyframes: true,
            nearly_aligned: true,
            tiny_gaps: true,
            short_clips: true,
            jump_cuts: true,
            abrupt_sound: true,
            music_over_voice: true,
            clipping: true,
            brief_text: true,
            text_off_frame: true,
            upscaled: true,
            shortest_move: 4,
            aligned_within: 1,
        }
    }
}

impl Options {
    /// The allowed range of `shortest_move`, in frames.
    pub const SHORTEST_MOVE_RANGE: (i64, i64) = (2, 30);
    /// The allowed range of `aligned_within`, in frames.
    pub const ALIGNED_WITHIN_RANGE: (i64, i64) = (1, 5);

    /// Whether a rule is switched on.
    pub const fn runs(&self, check: Check) -> bool {
        match check {
            Check::ConstantSpeed => self.constant_speed,
            Check::FastMove => self.fast_moves,
            Check::RedundantKeyframes => self.redundant_keyframes,
            Check::HiddenKeyframes => self.hidden_keyframes,
            Check::NearlyAligned => self.nearly_aligned,
            Check::TinyGap => self.tiny_gaps,
            Check::ShortClip => self.short_clips,
            Check::JumpCut => self.jump_cuts,
            Check::AbruptSound => self.abrupt_sound,
            Check::MusicOverVoice => self.music_over_voice,
            Check::Clipping => self.clipping,
            Check::BriefText => self.brief_text,
            Check::TextOffFrame => self.text_off_frame,
            Check::Upscaled => self.upscaled,
        }
    }

    /// Switches a rule on or off.
    pub const fn set(&mut self, check: Check, on: bool) {
        match check {
            Check::ConstantSpeed => self.constant_speed = on,
            Check::FastMove => self.fast_moves = on,
            Check::RedundantKeyframes => self.redundant_keyframes = on,
            Check::HiddenKeyframes => self.hidden_keyframes = on,
            Check::NearlyAligned => self.nearly_aligned = on,
            Check::TinyGap => self.tiny_gaps = on,
            Check::ShortClip => self.short_clips = on,
            Check::JumpCut => self.jump_cuts = on,
            Check::AbruptSound => self.abrupt_sound = on,
            Check::MusicOverVoice => self.music_over_voice = on,
            Check::Clipping => self.clipping = on,
            Check::BriefText => self.brief_text = on,
            Check::TextOffFrame => self.text_off_frame = on,
            Check::Upscaled => self.upscaled = on,
        }
    }

    /// The same options with both numbers inside their allowed ranges (a hand-edited settings
    /// file could hold anything).
    pub fn sound(mut self) -> Self {
        self.shortest_move = self.shortest_move.clamp(Self::SHORTEST_MOVE_RANGE.0, Self::SHORTEST_MOVE_RANGE.1);
        self.aligned_within = self.aligned_within.clamp(Self::ALIGNED_WITHIN_RANGE.0, Self::ALIGNED_WITHIN_RANGE.1);
        self
    }
}

/// A change the assistant can make for a suggestion.
#[derive(Clone, Debug, PartialEq)]
pub enum Fix {
    /// Easy Ease the moves that leave these keyframes.
    Ease { layer: Id, path: PropertyPath, frames: Vec<i64> },
    /// Move every keyframe of the property at or after `from` later by `by` frames.
    Lengthen { layer: Id, path: PropertyPath, from: i64, by: i64 },
    /// Remove these keyframes.
    Remove { layer: Id, path: PropertyPath, frames: Vec<i64> },
    /// Move one keyframe to another frame.
    Move { layer: Id, path: PropertyPath, from: i64, to: i64 },
    /// Move every clip on `track` that starts at or after frame `at` earlier by `by` frames.
    CloseGap { track: i32, at: i64, by: i64 },
    /// Delete a clip.
    DeleteClip { layer: Id },
    /// Give a clip's sound a fade in of this many seconds.
    FadeIn { layer: Id, seconds: f32 },
    /// Make a clip's sound get quieter while another layer's is playing.
    Duck { layer: Id, under: Id },
    /// Set a clip's volume, in decibels.
    SetVolume { layer: Id, db: f32 },
}

impl Fix {
    /// The name on the fix's button.
    pub fn label(&self) -> String {
        match self {
            Self::Ease { .. } => "Easy Ease".to_owned(),
            Self::Lengthen { by, .. } => format!("Lengthen by {by}"),
            Self::Remove { frames, .. } if frames.len() == 1 => "Remove It".to_owned(),
            Self::Remove { .. } => "Remove Them".to_owned(),
            Self::Move { to, .. } => format!("Move to Frame {to}"),
            Self::CloseGap { .. } => "Close the Gap".to_owned(),
            Self::DeleteClip { .. } => "Delete Clip".to_owned(),
            Self::FadeIn { .. } => "Add a Short Fade".to_owned(),
            Self::Duck { .. } => "Duck Under the Voice".to_owned(),
            Self::SetVolume { db, .. } => format!("Set Volume to {db:+.1} dB"),
        }
    }

    /// Whether the fix only removes the problem, without changing how the piece looks, sounds
    /// or is timed otherwise. "Fix All Safe" makes these and leaves the rest to be judged one
    /// by one.
    pub const fn is_safe(&self) -> bool {
        matches!(self, Self::Remove { .. } | Self::Move { .. } | Self::FadeIn { .. } | Self::SetVolume { .. })
    }
}

/// One thing the assistant noticed.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub check: Check,
    pub layer: Id,
    /// The property it is about, for the rules that read keyframes.
    pub path: Option<PropertyPath>,
    /// The frame to look at: where the playhead goes for "Show".
    pub frame: i64,
    /// What was noticed, in a few words.
    pub title: String,
    /// Why it matters and what the fix does.
    pub detail: String,
    /// What was measured, so the suggestion can be judged: the frames, sizes or levels.
    pub evidence: String,
    /// The one-click fix, when there is a safe one.
    pub fix: Option<Fix>,
}

impl Suggestion {
    /// Identifies this suggestion while what it is about stays put, so one the person
    /// dismissed doesn't come straight back.
    pub fn key(&self) -> String {
        format!("{:?}:{}:{:?}:{}", self.check, self.layer.0, self.path, self.frame)
    }
}

/// The most suggestions returned for one composition, so a huge project can't flood the list.
pub const MAX_SUGGESTIONS: usize = 60;

/// One animated property of a layer, with its keyframes in time order.
struct Track<'a> {
    layer: &'a Layer,
    path: PropertyPath,
    name: String,
    /// Each keyframe's frame and how the move leaving it is eased.
    keys: Vec<(i64, Easing)>,
    /// Each keyframe's value. Empty for properties with no single number (a shape's outline
    /// or style), which the rules that compare values skip.
    values: Vec<PropertyValue>,
}

/// Whether the property is one that moves things on screen, where easing and timing are felt.
/// Fades, speed and effect amounts are left to taste.
fn is_motion(path: PropertyPath) -> bool {
    matches!(
        path,
        PropertyPath::Transform(TransformProperty::Position | TransformProperty::Scale | TransformProperty::Rotation | TransformProperty::AnchorPoint) | PropertyPath::ShapePath
    )
}

fn same(a: PropertyValue, b: PropertyValue) -> bool {
    a.components() == b.components() && (0..a.components()).all(|index| (a.component(index) - b.component(index)).abs() < 1e-4)
}

/// Every keyframed property of every unlocked layer.
fn tracks(composition: &Composition) -> Vec<Track<'_>> {
    let mut tracks = Vec::new();
    for layer in composition.layers.iter().filter(|layer| !layer.switches.locked) {
        for path in layer.property_paths() {
            let keys = layer.keyframe_easings(path);
            if keys.is_empty() {
                continue;
            }
            let Some(snapshot) = layer.property(path, 0) else { continue };
            let has_number = !matches!(path, PropertyPath::ShapePath | PropertyPath::ShapeStyle);
            let values = if has_number { keys.iter().filter_map(|&(frame, _)| layer.property(path, frame).map(|at| at.value)).collect() } else { Vec::new() };
            tracks.push(Track { layer, path, name: snapshot.name, keys, values });
        }
    }
    tracks
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// Motion animated with nothing but linear moves.
fn constant_speed(track: &Track<'_>, found: &mut Vec<Suggestion>) {
    if !is_motion(track.path) || track.keys.len() < 2 {
        return;
    }
    // The last keyframe's easing leads nowhere, so only the others count.
    let moves = &track.keys[..track.keys.len() - 1];
    if !moves.iter().all(|(_, easing)| *easing == Easing::Linear) {
        return;
    }
    // A property whose value never changes isn't moving, however it is eased.
    if !track.values.is_empty() && track.values.windows(2).all(|pair| same(pair[0], pair[1])) {
        return;
    }
    found.push(Suggestion {
        check: Check::ConstantSpeed,
        layer: track.layer.id,
        path: Some(track.path),
        frame: track.keys[0].0,
        title: format!("{}: {} moves at a constant speed", track.layer.name, track.name),
        detail: "Nothing in real life starts and stops instantly. Easy Ease lets it speed up and slow down.".to_owned(),
        evidence: format!("{} linear from frame {} to {}", plural(track.keys.len(), "keyframe", "keyframes"), track.keys[0].0, track.keys[track.keys.len() - 1].0),
        fix: Some(Fix::Ease { layer: track.layer.id, path: track.path, frames: track.keys.iter().map(|(frame, _)| *frame).collect() }),
    });
}

/// A change that is over in fewer frames than `shortest`.
fn fast_moves(track: &Track<'_>, shortest: i64, fps: f64, found: &mut Vec<Suggestion>) {
    if !is_motion(track.path) || track.values.len() != track.keys.len() {
        return;
    }
    for (index, pair) in track.keys.windows(2).enumerate() {
        let ((from, easing), (to, _)) = (pair[0], pair[1]);
        let frames = to - from;
        // A hold is a jump on purpose; equal values aren't a move.
        if easing == Easing::Hold || frames >= shortest || same(track.values[index], track.values[index + 1]) {
            continue;
        }
        let by = shortest - frames;
        found.push(Suggestion {
            check: Check::FastMove,
            layer: track.layer.id,
            path: Some(track.path),
            frame: from,
            title: format!("{}: {} changes in {}", track.layer.name, track.name, plural(frames as usize, "frame", "frames")),
            detail: format!("That is {:.2} seconds, which can read as a jump. The fix moves the later {} keyframes {} later.", frames as f64 / fps, track.name, plural(by as usize, "frame", "frames")),
            evidence: format!("Frame {from} to {to}"),
            fix: Some(Fix::Lengthen { layer: track.layer.id, path: track.path, from: to, by }),
        });
    }
}

/// Keyframes that hold the same value as both their neighbours.
fn redundant_keyframes(track: &Track<'_>, found: &mut Vec<Suggestion>) {
    if track.values.len() != track.keys.len() || track.keys.len() < 3 {
        return;
    }
    let frames: Vec<i64> = (1..track.keys.len() - 1).filter(|&index| same(track.values[index - 1], track.values[index]) && same(track.values[index], track.values[index + 1])).map(|index| track.keys[index].0).collect();
    let Some(&first) = frames.first() else { return };
    found.push(Suggestion {
        check: Check::RedundantKeyframes,
        layer: track.layer.id,
        path: Some(track.path),
        frame: first,
        title: format!("{}: {} do nothing", track.layer.name, plural(frames.len(), &format!("{} keyframe", track.name), &format!("{} keyframes", track.name))),
        detail: "The keyframes before and after hold the same value, so removing them changes nothing on screen.".to_owned(),
        evidence: format!("At {}", frames.iter().map(|frame| format!("frame {frame}")).collect::<Vec<_>>().join(", ")),
        fix: Some(Fix::Remove { layer: track.layer.id, path: track.path, frames: frames.clone() }),
    });
}

/// Keyframes before the clip starts or after it ends.
fn hidden_keyframes(track: &Track<'_>, found: &mut Vec<Suggestion>) {
    let (start, end) = (track.layer.in_frame, track.layer.out_frame);
    let outside: Vec<i64> = track.keys.iter().map(|(frame, _)| *frame).filter(|&frame| frame < start || frame > end).collect();
    let Some(&first) = outside.first() else { return };
    found.push(Suggestion {
        check: Check::HiddenKeyframes,
        layer: track.layer.id,
        path: Some(track.path),
        frame: first,
        title: format!("{}: {} outside the clip", track.layer.name, plural(outside.len(), &format!("{} keyframe is", track.name), &format!("{} keyframes are", track.name))),
        detail: "That part of the animation never shows. Lengthen the clip or move the keyframes.".to_owned(),
        evidence: format!("Clip runs from frame {start} to {end}; first keyframe outside is at {first}"),
        fix: None,
    });
}

/// Keyframes within `within` frames of a frame where more layers have keyframes.
fn nearly_aligned(tracks: &[Track<'_>], within: i64, found: &mut Vec<Suggestion>) {
    // How many layers have a keyframe on each frame.
    let mut layers_at: std::collections::BTreeMap<i64, std::collections::BTreeSet<Id>> = std::collections::BTreeMap::new();
    for track in tracks {
        for (frame, _) in &track.keys {
            layers_at.entry(*frame).or_default().insert(track.layer.id);
        }
    }
    for track in tracks {
        for &(frame, _) in &track.keys {
            let here = layers_at[&frame].len();
            // The busiest nearby frame used by other layers; the earlier one on a tie.
            let target = layers_at
                .range(frame - within..=frame + within)
                .filter(|(other, layers)| **other != frame && layers.iter().any(|layer| *layer != track.layer.id))
                .map(|(other, layers)| (*other, layers.len()))
                .filter(|&(other, count)| count > here || (count == here && other < frame))
                .max_by_key(|&(other, count)| (count, std::cmp::Reverse(other)));
            let Some((to, _)) = target else { continue };
            // Moving onto another keyframe of the same property would replace it.
            if track.keys.iter().any(|(taken, _)| *taken == to) {
                continue;
            }
            let apart = (to - frame).unsigned_abs() as usize;
            found.push(Suggestion {
                check: Check::NearlyAligned,
                layer: track.layer.id,
                path: Some(track.path),
                frame,
                title: format!("{}: {} keyframe is {} off", track.layer.name, track.name, plural(apart, "frame", "frames")),
                detail: "Lining it up with the other layers' keyframes makes things land together.".to_owned(),
                evidence: format!("This keyframe is on frame {frame}; others are on frame {to}"),
                fix: Some(Fix::Move { layer: track.layer.id, path: track.path, from: frame, to }),
            });
        }
    }
}

/// Everything the switched-on rules notice in `composition`, earliest first, at most
/// `MAX_SUGGESTIONS`. Locked layers are left out: nothing on them can be fixed. Sound boosted
/// too far isn't included, as it needs the sound itself measured: see `clipping`.
pub fn suggestions(composition: &Composition, options: &Options) -> Vec<Suggestion> {
    let options = options.sound();
    let fps = composition.frame_rate.fps().max(1.0);
    let tracks = tracks(composition);
    let mut found = Vec::new();
    for track in &tracks {
        if options.constant_speed {
            constant_speed(track, &mut found);
        }
        if options.fast_moves {
            fast_moves(track, options.shortest_move, fps, &mut found);
        }
        if options.redundant_keyframes {
            redundant_keyframes(track, &mut found);
        }
        if options.hidden_keyframes {
            hidden_keyframes(track, &mut found);
        }
    }
    if options.nearly_aligned {
        nearly_aligned(&tracks, options.aligned_within, &mut found);
    }
    clips::find(composition, &options, &mut found);
    found.sort_by_key(|suggestion| (suggestion.frame, suggestion.check));
    found.truncate(MAX_SUGGESTIONS);
    found
}

/// Makes a suggestion's fix. Returns whether anything changed; a fix for a layer that has
/// since been locked or deleted does nothing.
pub fn apply(composition: &mut Composition, fix: &Fix) -> bool {
    let (layer, path) = match fix {
        Fix::Ease { layer, path, .. } | Fix::Lengthen { layer, path, .. } | Fix::Remove { layer, path, .. } | Fix::Move { layer, path, .. } => (layer, path),
        _ => return clips::apply(composition, fix),
    };
    let Some(layer) = composition.layer_mut(*layer).filter(|layer| !layer.switches.locked) else { return false };
    let existing: Vec<i64> = layer.keyframe_easings(*path).iter().map(|(frame, _)| *frame).collect();
    let mut changed = false;
    match fix {
        Fix::Ease { frames, .. } => {
            for frame in frames.iter().filter(|frame| existing.contains(frame)) {
                if let Some(animated) = layer.animated_mut(*path) {
                    animated.set_easing(*frame, Easing::Curve(EASY_EASE));
                    changed = true;
                }
            }
        }
        Fix::Lengthen { from, by, .. } => {
            // The last keyframe first, so none lands on one that hasn't moved yet.
            for frame in existing.iter().rev().filter(|frame| **frame >= *from && *by > 0) {
                if let Some(animated) = layer.animated_mut(*path) {
                    changed |= animated.move_keyframe(*frame, frame + by);
                }
            }
        }
        Fix::Remove { frames, .. } => {
            for frame in frames.iter().filter(|frame| existing.contains(frame)) {
                if let Some(animated) = layer.animated_mut(*path) {
                    animated.remove_keyframe(*frame);
                    changed = true;
                }
            }
        }
        Fix::Move { from, to, .. } => {
            if existing.contains(from) && !existing.contains(to)
                && let Some(animated) = layer.animated_mut(*path)
            {
                changed = animated.move_keyframe(*from, *to);
            }
        }
        // Handled by `clips::apply` above.
        Fix::CloseGap { .. } | Fix::DeleteClip { .. } | Fix::FadeIn { .. } | Fix::Duck { .. } | Fix::SetVolume { .. } => {}
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{Animated, Keyframe, Vec2};
    use crate::sample::{ids, sample_project};

    const POSITION: PropertyPath = PropertyPath::Transform(TransformProperty::Position);

    /// The sample composition with every keyframe cleared, so each test starts from nothing,
    /// and the ids of two of its layers.
    fn blank() -> (Composition, Id, Id) {
        let mut composition = sample_project().composition(ids::MAIN_COMP).unwrap().clone();
        for layer in &mut composition.layers {
            for path in layer.property_paths() {
                for (frame, _) in layer.keyframe_easings(path) {
                    if let Some(animated) = layer.animated_mut(path) {
                        animated.remove_keyframe(frame);
                    }
                }
            }
            layer.in_frame = 0;
            layer.out_frame = 400;
        }
        let (first, second) = (composition.layers[0].id, composition.layers[1].id);
        (composition, first, second)
    }

    /// Adds linear keyframes to an animated value, keeping the ones it has.
    fn add_keys<T: Clone + crate::animation::Blend>(animated: &mut Animated<T>, keys: impl IntoIterator<Item = (i64, T)>) {
        let mut all = animated.keyframes().to_vec();
        all.extend(keys.into_iter().map(|(frame, value)| Keyframe::linear(frame, value)));
        *animated = Animated::keyframed(all);
    }

    /// Keyframes a layer's position at each `(frame, x)`.
    fn key_position(composition: &mut Composition, layer: Id, keys: &[(i64, f32)]) {
        let position = &mut composition.layer_mut(layer).unwrap().transform.as_mut().unwrap().position;
        add_keys(position, keys.iter().map(|&(frame, x)| (frame, Vec2::new(x, 0.0))));
    }

    fn only(check: Check) -> Options {
        let mut options = Options::default();
        for other in Check::ALL {
            options.set(other, other == check);
        }
        options
    }

    #[test]
    fn a_blank_composition_has_nothing_to_suggest() {
        let (composition, ..) = blank();
        assert!(suggestions(&composition, &Options::default()).is_empty());
    }

    #[test]
    fn linear_motion_is_noticed_and_easy_ease_fixes_it() {
        let (mut composition, layer, _) = blank();
        key_position(&mut composition, layer, &[(10, 0.0), (40, 100.0)]);
        let found = suggestions(&composition, &only(Check::ConstantSpeed));
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].layer, found[0].path, found[0].frame), (layer, Some(POSITION), 10));
        assert!(apply(&mut composition, found[0].fix.as_ref().unwrap()));
        assert_eq!(composition.layer(layer).unwrap().keyframe_easings(POSITION)[0].1, Easing::Curve(EASY_EASE));
        assert!(suggestions(&composition, &only(Check::ConstantSpeed)).is_empty(), "fixed, so it isn't suggested again");
    }

    #[test]
    fn a_fade_is_left_to_taste() {
        let (mut composition, layer, _) = blank();
        let opacity = &mut composition.layer_mut(layer).unwrap().transform.as_mut().unwrap().opacity;
        add_keys(opacity, [(0, 0.0), (20, 100.0)]);
        assert!(suggestions(&composition, &only(Check::ConstantSpeed)).is_empty());
    }

    #[test]
    fn a_two_frame_move_is_lengthened_and_later_keyframes_move_along() {
        let (mut composition, layer, _) = blank();
        key_position(&mut composition, layer, &[(10, 0.0), (12, 100.0), (50, 200.0)]);
        let found = suggestions(&composition, &only(Check::FastMove));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].fix, Some(Fix::Lengthen { layer, path: POSITION, from: 12, by: 2 }));
        assert!(apply(&mut composition, found[0].fix.as_ref().unwrap()));
        let frames: Vec<i64> = composition.layer(layer).unwrap().keyframe_easings(POSITION).iter().map(|(frame, _)| *frame).collect();
        assert_eq!(frames, vec![10, 14, 52]);
        // A longer limit catches it again; a hold never counts.
        let stricter = Options { shortest_move: 10, ..only(Check::FastMove) };
        assert_eq!(suggestions(&composition, &stricter).len(), 1);
        composition.layer_mut(layer).unwrap().animated_mut(POSITION).unwrap().set_easing(10, Easing::Hold);
        assert!(suggestions(&composition, &stricter).is_empty());
    }

    #[test]
    fn a_keyframe_between_two_equal_ones_is_removed() {
        let (mut composition, layer, _) = blank();
        key_position(&mut composition, layer, &[(0, 50.0), (20, 50.0), (40, 50.0), (60, 90.0)]);
        let found = suggestions(&composition, &only(Check::RedundantKeyframes));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].fix, Some(Fix::Remove { layer, path: POSITION, frames: vec![20] }));
        assert!(apply(&mut composition, found[0].fix.as_ref().unwrap()));
        let transform = composition.layer(layer).unwrap().transform.as_ref().unwrap();
        assert_eq!(transform.position.keyframe_frames(), vec![0, 40, 60]);
        assert_eq!(transform.position.value_at(20.0), Vec2::new(50.0, 0.0), "nothing changed on screen");
    }

    #[test]
    fn keyframes_past_the_clips_end_are_noticed_without_a_fix() {
        let (mut composition, layer, _) = blank();
        key_position(&mut composition, layer, &[(10, 0.0), (500, 100.0)]);
        let found = suggestions(&composition, &only(Check::HiddenKeyframes));
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].frame, &found[0].fix), (500, &None));
    }

    #[test]
    fn a_keyframe_one_frame_off_moves_onto_the_others() {
        let (mut composition, first, second) = blank();
        key_position(&mut composition, first, &[(30, 0.0)]);
        add_keys(&mut composition.layer_mut(first).unwrap().transform.as_mut().unwrap().rotation, [(30, 10.0)]);
        key_position(&mut composition, second, &[(31, 0.0)]);
        let found = suggestions(&composition, &only(Check::NearlyAligned));
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].fix, Some(Fix::Move { layer: second, path: POSITION, from: 31, to: 30 }));
        assert!(apply(&mut composition, found[0].fix.as_ref().unwrap()));
        assert!(suggestions(&composition, &only(Check::NearlyAligned)).is_empty());
        // Two frames apart is a deliberate stagger at the default setting, but not at a wider one.
        key_position(&mut composition, second, &[(62, 0.0)]);
        key_position(&mut composition, first, &[(60, 0.0)]);
        assert!(suggestions(&composition, &only(Check::NearlyAligned)).is_empty());
        assert_eq!(suggestions(&composition, &Options { aligned_within: 2, ..only(Check::NearlyAligned) }).len(), 1);
    }

    #[test]
    fn rules_switched_off_and_locked_layers_say_nothing() {
        let (mut composition, layer, _) = blank();
        key_position(&mut composition, layer, &[(10, 0.0), (12, 100.0)]);
        assert!(!suggestions(&composition, &Options::default()).is_empty());
        let mut none = Options::default();
        for check in Check::ALL {
            none.set(check, false);
        }
        assert!(suggestions(&composition, &none).is_empty());
        let fix = suggestions(&composition, &only(Check::ConstantSpeed))[0].fix.clone().unwrap();
        composition.layer_mut(layer).unwrap().switches.locked = true;
        assert!(suggestions(&composition, &Options::default()).is_empty());
        assert!(!apply(&mut composition, &fix), "a locked layer is left alone");
    }

    #[test]
    fn numbers_from_a_settings_file_are_kept_in_range() {
        let wild = Options { shortest_move: -3, aligned_within: 900, ..Options::default() }.sound();
        assert_eq!((wild.shortest_move, wild.aligned_within), (2, 5));
        let read: Options = serde_json::from_str(r#"{"fast_moves":false}"#).unwrap();
        assert_eq!(read, Options { fast_moves: false, ..Options::default() });
    }
}
