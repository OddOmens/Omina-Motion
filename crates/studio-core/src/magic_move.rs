//! Magic Move: one composition turning into another, as Keynote's Magic Move turns one slide
//! into the next.
//!
//! `magic_move` builds a new composition that plays the first composition, then a transition of
//! the chosen length, then the second composition. During the transition, layers the two share
//! (matched by name) change from how they look at the end of the first into how they look at
//! the start of the second:
//!
//! - two shapes morph, outline and colors included (see `morph::morph_to`);
//! - two layers showing the same thing (the same text, image or color, at the same size) move,
//!   scale, turn and fade into place;
//! - two layers showing different things move into place while one fades into the other.
//!
//! Layers only in the first composition fade out, and layers only in the second fade in.
//!
//! The transition is made of "bridge" layers: copies of the layers involved that are shown only
//! while it plays, each keyframed from one end of it to the other. The copies of the two
//! compositions themselves are left as they were (apart from In and Out animations, whose job
//! the transition takes over), so their own animations still play before and after it.

use std::collections::HashMap;

use crate::animation::{Animated, Blend, Easing, Keyframe, Vec2};
use crate::edit::CompositionSettings;
use crate::model::{Composition, Id, Layer, LayerContent, LayerMask, MAX_FRAME, Project, TextPath, TrackMatte};
use crate::morph::{MAX_MORPH_FRAMES, morph_to};

/// Why a Magic Move could not be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MagicMoveError {
    /// One of the compositions does not exist.
    Missing,
    /// A composition cannot move into itself.
    SameComposition,
    /// The two compositions differ in frame size or frame rate.
    Mismatched,
    /// The result would be longer than a composition can be.
    TooLong,
}

impl std::fmt::Display for MagicMoveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Missing => "That composition no longer exists.",
            Self::SameComposition => "Pick a different composition to move into.",
            Self::Mismatched => "Magic Move needs two compositions of the same size and frame rate.",
            Self::TooLong => "The two compositions together are too long.",
        })
    }
}

/// What a Magic Move made: the new composition, and how many layers changed in each way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MagicMove {
    pub composition: Id,
    /// Pairs of layers matched by name that move (or morph) into each other.
    pub matched: usize,
    /// Layers only in the first composition, faded out.
    pub faded_out: usize,
    /// Layers only in the second composition, faded in.
    pub faded_in: usize,
}

/// The compositions `from` can Magic Move into: every other composition of the same size and
/// frame rate, in project order.
pub fn targets(project: &Project, from: Id) -> Vec<Id> {
    let Some(source) = project.composition(from) else { return Vec::new() };
    project.compositions.iter().filter(|other| other.id != from && fits(source, other)).map(|other| other.id).collect()
}

/// Whether two compositions can be joined: the same frame size and frame rate.
fn fits(a: &Composition, b: &Composition) -> bool {
    (a.width, a.height) == (b.width, b.height) && a.frame_rate == b.frame_rate
}

/// Builds a composition that plays `from`, turns it into `to` over `frames` frames with
/// `easing`, then plays `to` (see the module notes), and adds it to the project. It is named
/// "<from> → <to>" and has `from`'s background.
pub fn magic_move(project: &mut Project, from: Id, to: Id, frames: i64, easing: Easing) -> Result<MagicMove, MagicMoveError> {
    let first = project.composition(from).ok_or(MagicMoveError::Missing)?.clone();
    let second = project.composition(to).ok_or(MagicMoveError::Missing)?.clone();
    if from == to {
        return Err(MagicMoveError::SameComposition);
    }
    if !fits(&first, &second) {
        return Err(MagicMoveError::Mismatched);
    }
    let length = frames.clamp(1, MAX_MORPH_FRAMES);
    // The transition runs from `start` (the first frame after `from` ends) to `end` (the first
    // frame of `to`).
    let (start, end) = (first.duration_frames, first.duration_frames + length);
    let duration = end + second.duration_frames;
    if duration > MAX_FRAME {
        return Err(MagicMoveError::TooLong);
    }
    let settings = CompositionSettings { name: format!("{} → {}", first.name, second.name), width: first.width, height: first.height, frame_rate: first.frame_rate, duration_frames: duration, background: first.background };
    let comp_id = crate::edit::new_composition(project, &settings).map_err(|_| MagicMoveError::Mismatched)?;

    // Copies of both compositions' layers, the second's moved to start after the transition.
    // Layers starting after the first composition ends (left there when it was shortened) never
    // show in it, so they are not copied: cut short at its end they would end before starting.
    let showing: Vec<Layer> = first.layers.iter().filter(|layer| layer.in_frame < start).cloned().collect();
    let mut before = copied(project, &showing, 0);
    let mut after = copied(project, &second.layers, end);

    // The transition takes over from the Out motions of layers still showing as it starts and
    // the In motions of those showing as it ends: their keyframes go, so a matched layer moves
    // straight across instead of fading out and back in. Done before the first composition's
    // layers are cut short at its end, since each motion's frames are measured from the layer's own ends.
    for layer in before.iter_mut().filter(|layer| layer.is_active_at(start - 1) && layer.transform.is_some() && layer.content.has_picture()) {
        crate::presets::strip_motion(layer, crate::presets::Phase::Out, first.frame_rate.fps());
        layer.animation.text_out = None;
    }
    for layer in after.iter_mut().filter(|layer| layer.is_active_at(end) && layer.transform.is_some() && layer.content.has_picture()) {
        crate::presets::strip_motion(layer, crate::presets::Phase::In, second.frame_rate.fps());
        layer.animation.text_in = None;
    }
    for layer in &mut before {
        layer.out_frame = layer.out_frame.min(start);
    }

    // The layers seen as the transition starts and as it ends, top first.
    let shown = |layer: &Layer, frame: i64| layer.is_active_at(frame) && layer.transform.is_some() && layer.content.has_picture();
    let leaving: Vec<usize> = (0..before.len()).filter(|&index| shown(&before[index], start - 1)).collect();
    let arriving: Vec<usize> = (0..after.len()).filter(|&index| shown(&after[index], end)).collect();
    // Matched by name, each with the first unmatched layer of the same name.
    let mut taken = vec![false; arriving.len()];
    let mut pairs = Vec::new();
    let mut gone = Vec::new();
    for &index in &leaving {
        let name = before[index].name.trim();
        match arriving.iter().enumerate().find(|&(slot, &other)| !taken[slot] && after[other].name.trim() == name) {
            Some((slot, &other)) => {
                taken[slot] = true;
                pairs.push((index, other));
            }
            None => gone.push(index),
        }
    }
    let new: Vec<usize> = arriving.iter().zip(&taken).filter(|(_, taken)| !**taken).map(|(&index, _)| index).collect();

    let window = Window { start, end, easing };
    let mut bridges = Vec::new();
    let mut morphs = Vec::new();
    for &(a, b) in &pairs {
        let (leaving, arriving) = (bridge(project, &before[a], start - 1, window), bridge(project, &after[b], end, window));
        let both_shapes = [&leaving, &arriving].iter().all(|layer| matches!(layer.content, LayerContent::Shape { .. }));
        if both_shapes {
            // Morphed below, once both are in the composition.
            morphs.push((leaving.id, arriving.id));
            bridges.extend([leaving, arriving]);
        } else if leaving.content == arriving.content && leaving.size == arriving.size {
            bridges.push(moving(leaving, &arriving, window));
        } else {
            bridges.extend(crossfade(leaving, arriving, window));
        }
    }
    for &index in &gone {
        let mut layer = bridge(project, &before[index], start - 1, window);
        fade(&mut layer, window, true);
        bridges.push(layer);
    }
    for &index in &new {
        let mut layer = bridge(project, &after[index], end, window);
        fade(&mut layer, window, false);
        bridges.push(layer);
    }
    place_bridges(&mut bridges);

    let composition = project.composition_mut(comp_id).ok_or(MagicMoveError::Missing)?;
    // Every picture track moves up one, leaving the main track empty: the magnetic main track
    // would otherwise close any gap in it and pull the second half out of time.
    for layer in before.iter_mut().chain(after.iter_mut()).chain(bridges.iter_mut()) {
        if layer.track >= crate::tracks::MAIN_TRACK {
            layer.track += 1;
        }
    }
    composition.layers = before.into_iter().chain(after).chain(bridges).collect();
    composition.markers = first.markers.iter().cloned().chain(second.markers.iter().map(|marker| crate::model::Marker { frame: marker.frame + end, ..marker.clone() })).collect();
    for (leaving, arriving) in morphs {
        // The layer morphs into the arriving one's look; that one is only needed for it.
        let locked = composition.layer(leaving).is_some_and(|layer| layer.switches.locked);
        if let Some(layer) = composition.layer_mut(leaving) {
            layer.switches.locked = false;
        }
        let morphed = morph_to(composition, leaving, arriving, start, length, easing).is_ok();
        if let Some(layer) = composition.layer_mut(leaving) {
            layer.switches.locked = locked;
            layer.out_frame = end;
        }
        if morphed {
            composition.layers.retain(|layer| layer.id != arriving);
        } else if let Some(layer) = composition.layer_mut(arriving) {
            // Could not morph (which should not happen): show the arriving one as it is instead.
            layer.switches.visible = true;
        }
    }
    crate::tracks::normalize(composition);
    Ok(MagicMove { composition: comp_id, matched: pairs.len(), faded_out: gone.len(), faded_in: new.len() })
}

/// When the transition runs (from `start` up to, not including, `end`) and how it eases.
#[derive(Clone, Copy)]
struct Window {
    start: i64,
    end: i64,
    easing: Easing,
}

/// Copies of `layers` with fresh ids (their effects too), moved `delta` frames later. Parents,
/// masks, text paths and ducking that point within the group point to the copies; ones that
/// point outside it are dropped.
fn copied(project: &mut Project, layers: &[Layer], delta: i64) -> Vec<Layer> {
    let mut new_ids: HashMap<Id, Id> = HashMap::new();
    let mut copies: Vec<Layer> = layers
        .iter()
        .map(|original| {
            let mut copy = original.clone();
            copy.id = project.allocate_id();
            new_ids.insert(original.id, copy.id);
            for effect in &mut copy.effects {
                effect.id = project.allocate_id();
            }
            copy.shift_in_time(delta);
            copy
        })
        .collect();
    let moved = |old: Id| new_ids.get(&old).copied();
    for copy in &mut copies {
        copy.parent = copy.parent.and_then(moved);
        copy.mask = copy.mask.and_then(|mask| moved(mask.layer).map(|layer| LayerMask { layer, ..mask }));
        copy.text_path = copy.text_path.and_then(|on| moved(on.layer).map(|layer| TextPath { layer, ..on }));
        copy.audio.duck_under = copy.audio.duck_under.and_then(moved);
    }
    copies
}

/// A bridge: a copy of `layer` (with a fresh id) shown only during the transition, holding the
/// look it has at `frame`. It keeps its parent, mask and text path, which are copies in the
/// same composition, but no track matte (it has no matte layer of its own above it).
fn bridge(project: &mut Project, layer: &Layer, frame: i64, window: Window) -> Layer {
    let mut copy = layer.clone();
    copy.id = project.allocate_id();
    for effect in &mut copy.effects {
        effect.id = project.allocate_id();
    }
    copy.in_frame = window.start;
    copy.out_frame = window.end;
    copy.track_matte = TrackMatte::None;
    copy.animation = Default::default();
    let at = frame as f64;
    if let Some(transform) = copy.transform.as_mut() {
        hold(&mut transform.anchor_point, at);
        hold(&mut transform.position, at);
        hold(&mut transform.scale, at);
        hold(&mut transform.rotation, at);
        hold(&mut transform.opacity, at);
    }
    // A morphing shape keeps the outline and style it has at `frame`.
    if let (Some(path), Some(style)) = (copy.shape_path_at(at).map(|path| path.into_owned()), copy.shape_style_at(at))
        && let LayerContent::Shape { style: own_style, path: own_path } = &mut copy.content
    {
        *own_style = style;
        *own_path = Some(path);
    }
    copy.morph = None;
    copy
}

/// Replaces an animated value with the value it has at `frame`.
fn hold<T: Blend>(animated: &mut Animated<T>, frame: f64) {
    *animated = Animated::constant(animated.value_at(frame));
}

/// Keyframes a value from `from` at the start of the transition to `to` at its end.
fn ramp<T: Blend>(animated: &mut Animated<T>, from: T, to: T, window: Window) {
    *animated = Animated::keyframed(vec![Keyframe { frame: window.start, value: from, easing: window.easing }, Keyframe::linear(window.end, to)]);
}

/// Fades a bridge out over the transition (or, with `out` false, in to its own opacity).
fn fade(layer: &mut Layer, window: Window, out: bool) {
    let Some(transform) = layer.transform.as_mut() else { return };
    let opacity = transform.opacity.value;
    let (from, to) = if out { (opacity, 0.0) } else { (0.0, opacity) };
    ramp(&mut transform.opacity, from, to, window);
}

/// The leaving bridge moved, scaled, turned and faded into the arriving one's place (they show
/// the same thing).
fn moving(mut leaving: Layer, arriving: &Layer, window: Window) -> Layer {
    if let (Some(from), Some(to)) = (leaving.transform.as_mut(), arriving.transform.as_ref()) {
        let (position, scale, rotation, opacity) = (from.position.value, from.scale.value, from.rotation.value, from.opacity.value);
        ramp(&mut from.position, position, to.position.value, window);
        ramp(&mut from.scale, scale, to.scale.value, window);
        ramp(&mut from.rotation, rotation, to.rotation.value, window);
        ramp(&mut from.opacity, opacity, to.opacity.value, window);
    }
    leaving
}

/// Two bridges showing different things: both move from the leaving layer's place to the
/// arriving one's, the leaving one fading out as the arriving one fades in. Each is scaled so it
/// covers the same area as the other where that one is fully shown.
fn crossfade(mut leaving: Layer, mut arriving: Layer, window: Window) -> [Layer; 2] {
    let ratio = |a: Vec2, b: Vec2| Vec2::new(if b.x > 0.0 { a.x / b.x } else { 1.0 }, if b.y > 0.0 { a.y / b.y } else { 1.0 });
    let times = |a: Vec2, b: Vec2| Vec2::new(a.x * b.x, a.y * b.y);
    let (leaving_size, arriving_size) = (leaving.size, arriving.size);
    if let (Some(from), Some(to)) = (leaving.transform.as_mut(), arriving.transform.as_mut()) {
        let (position, scale, rotation, opacity) = ((from.position.value, to.position.value), (from.scale.value, to.scale.value), (from.rotation.value, to.rotation.value), (from.opacity.value, to.opacity.value));
        ramp(&mut from.position, position.0, position.1, window);
        ramp(&mut from.scale, scale.0, times(scale.1, ratio(arriving_size, leaving_size)), window);
        ramp(&mut from.rotation, rotation.0, rotation.1, window);
        ramp(&mut from.opacity, opacity.0, 0.0, window);
        ramp(&mut to.position, position.0, position.1, window);
        ramp(&mut to.scale, times(scale.0, ratio(leaving_size, arriving_size)), scale.1, window);
        ramp(&mut to.rotation, rotation.0, rotation.1, window);
        ramp(&mut to.opacity, 0.0, opacity.1, window);
    }
    [leaving, arriving]
}

/// Gives each bridge the track of the layer it came from, moving one up past any bridge already
/// on that track (only bridges are shown during the transition, so only they can collide).
fn place_bridges(bridges: &mut [Layer]) {
    let mut used: Vec<i32> = Vec::new();
    for layer in bridges.iter_mut() {
        let sound = crate::tracks::is_sound_only(layer);
        while used.contains(&layer.track) {
            layer.track += if sound { -1 } else { 1 };
        }
        used.push(layer.track);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Paint, Rgba, ShapeKind, ShapeStyle};

    /// A project with two 1920 × 1080 compositions: "Intro" with a square named "Card", a title
    /// and a circle named "Spot", and "Main" with a star named "Card", the same title moved and a
    /// solid named "Backdrop". Returns the project and the two compositions.
    fn two_scenes() -> (Project, Id, Id) {
        let mut project = Project::new(Vec::new(), Vec::new());
        let settings = |name: &str| CompositionSettings { name: name.into(), width: 1920, height: 1080, frame_rate: crate::time::FrameRate::new(30, 1), duration_frames: 60, background: Rgba::hex(0x000000) };
        let intro = crate::edit::new_composition(&mut project, &settings("Intro")).unwrap();
        let main = crate::edit::new_composition(&mut project, &settings("Main")).unwrap();
        let card = crate::edit::add_shape_layer(&mut project, intro, ShapeStyle::new(ShapeKind::Rectangle), Vec2::new(200.0, 200.0), Vec2::new(400.0, 500.0), 0).unwrap();
        let spot = crate::edit::add_shape_layer(&mut project, intro, ShapeStyle::new(ShapeKind::Ellipse), Vec2::new(100.0, 100.0), Vec2::new(1500.0, 300.0), 0).unwrap();
        let star = crate::edit::add_shape_layer(&mut project, main, ShapeStyle { fill: Some(Paint::Solid(Rgba::hex(0xff0000))), ..ShapeStyle::new(ShapeKind::Star) }, Vec2::new(400.0, 400.0), Vec2::new(1400.0, 540.0), 0).unwrap();
        let backdrop = crate::edit::add_shape_layer(&mut project, main, ShapeStyle::new(ShapeKind::Rectangle), Vec2::new(1920.0, 200.0), Vec2::new(960.0, 980.0), 0).unwrap();
        for (comp, id, name) in [(intro, card, "Card"), (intro, spot, "Spot"), (main, star, "Card"), (main, backdrop, "Backdrop")] {
            project.composition_mut(comp).unwrap().layer_mut(id).unwrap().name = name.into();
        }
        // The same title in both, moved and turned in the second.
        for (comp, position, rotation) in [(intro, Vec2::new(960.0, 200.0), 0.0), (main, Vec2::new(500.0, 900.0), 10.0)] {
            let style = crate::model::TextStyle::default();
            let id = crate::edit::add_shape_layer(&mut project, comp, ShapeStyle::new(ShapeKind::Rectangle), Vec2::new(10.0, 10.0), position, 0).unwrap();
            let layer = project.composition_mut(comp).unwrap().layer_mut(id).unwrap();
            layer.name = "Title".into();
            layer.content = LayerContent::Text { text: "Hello".into(), style: style.clone() };
            layer.size = style.estimate_size("Hello");
            layer.transform = Some(crate::model::Transform::centered(layer.size, position));
            layer.transform.as_mut().unwrap().rotation = Animated::constant(rotation);
        }
        (project, intro, main)
    }

    fn named<'a>(composition: &'a Composition, name: &str, frame: i64) -> Vec<&'a Layer> {
        composition.layers.iter().filter(|layer| layer.name == name && layer.is_active_at(frame)).collect()
    }

    #[test]
    fn layers_are_matched_by_name_and_the_rest_fade() {
        let (mut project, intro, main) = two_scenes();
        let made = magic_move(&mut project, intro, main, 20, Easing::Linear).unwrap();
        assert_eq!((made.matched, made.faded_out, made.faded_in), (2, 1, 1), "Card and Title match; Spot leaves; Backdrop arrives");
        let composition = project.composition(made.composition).unwrap();
        assert_eq!(composition.name, "Intro → Main");
        assert_eq!(composition.duration_frames, 60 + 20 + 60);
        assert_eq!(crate::project_file::check(&project), Ok(()), "the result is a sound project");

        // The card morphs from the square into the red star during the transition.
        let [card] = named(composition, "Card", 70)[..] else { panic!("one card while it moves") };
        assert!(card.is_morphing());
        let transform = card.transform.as_ref().unwrap();
        assert_eq!(transform.position.value_at(60.0), Vec2::new(400.0, 500.0));
        assert_eq!(transform.position.value_at(80.0), Vec2::new(1400.0, 540.0));
        assert_eq!(transform.position.value_at(70.0), Vec2::new(900.0, 520.0), "halfway, linearly");
        // The title is the same text, so it simply moves and turns.
        let [title] = named(composition, "Title", 70)[..] else { panic!("one title while it moves") };
        assert_eq!(title.transform.as_ref().unwrap().rotation.value_at(70.0), 5.0);
        // Spot fades out and Backdrop fades in.
        let [spot] = named(composition, "Spot", 70)[..] else { panic!() };
        assert_eq!(spot.transform.as_ref().unwrap().opacity.value_at(70.0), 50.0);
        let [backdrop] = named(composition, "Backdrop", 70)[..] else { panic!() };
        assert_eq!(backdrop.transform.as_ref().unwrap().opacity.value_at(65.0), 25.0);

        // Before and after, each composition's own layers show, once each.
        assert_eq!(named(composition, "Card", 10).len(), 1);
        assert!(named(composition, "Backdrop", 10).is_empty());
        assert_eq!(named(composition, "Card", 100).len(), 1);
        assert!(named(composition, "Spot", 100).is_empty());
        // Nothing sits on the magnetic main track, and no two clips on a track overlap.
        assert!(composition.layers.iter().all(|layer| layer.track != crate::tracks::MAIN_TRACK));
        for (index, layer) in composition.layers.iter().enumerate() {
            for other in &composition.layers[index + 1..] {
                assert!(layer.track != other.track || layer.out_frame <= other.in_frame || other.out_frame <= layer.in_frame, "{} and {} overlap", layer.name, other.name);
            }
        }
        // The source compositions are untouched.
        assert_eq!(project.composition(intro).unwrap().layers.len(), 3);
    }

    #[test]
    fn different_content_with_the_same_name_crossfades() {
        let (mut project, intro, main) = two_scenes();
        let title = project.composition(main).unwrap().layers.iter().find(|layer| layer.name == "Title").unwrap().id;
        if let LayerContent::Text { text, .. } = &mut project.composition_mut(main).unwrap().layer_mut(title).unwrap().content {
            *text = "Goodbye".into();
        }
        let made = magic_move(&mut project, intro, main, 20, Easing::Linear).unwrap();
        let composition = project.composition(made.composition).unwrap();
        let titles = named(composition, "Title", 70);
        assert_eq!(titles.len(), 2, "the old and new text cross");
        let opacities: Vec<f32> = titles.iter().map(|layer| layer.transform.as_ref().unwrap().opacity.value_at(75.0)).collect();
        assert!(opacities.contains(&25.0) && opacities.contains(&75.0), "{opacities:?}");
    }

    #[test]
    fn matched_layers_move_across_without_their_own_in_and_out_motions() {
        let (mut project, intro, main) = two_scenes();
        let title_in = |project: &Project, comp: Id| project.composition(comp).unwrap().layers.iter().find(|layer| layer.name == "Title").unwrap().id;
        let (leaving, arriving) = (title_in(&project, intro), title_in(&project, main));
        crate::presets::set_motion(&mut project, intro, leaving, crate::presets::Phase::Out, Some(crate::presets::Motion::Fade), 0.5);
        crate::presets::set_motion(&mut project, main, arriving, crate::presets::Phase::In, Some(crate::presets::Motion::Fade), 0.5);
        let made = magic_move(&mut project, intro, main, 20, Easing::Linear).unwrap();
        let composition = project.composition(made.composition).unwrap();
        // Fully seen as the first scene ends, all through the move, and as the second begins.
        for frame in [55, 59, 60, 70, 79, 80, 85] {
            let titles = named(composition, "Title", frame);
            assert_eq!(titles.len(), 1, "one title at frame {frame}");
            assert_eq!(titles[0].transform.as_ref().unwrap().opacity.value_at(frame as f64), 100.0, "the title is fully seen at frame {frame}");
            assert!(titles[0].animation.enter.is_none() && titles[0].animation.exit.is_none());
        }
        // The source compositions keep their motions.
        assert!(project.composition(intro).unwrap().layer(leaving).unwrap().animation.exit.is_some());
    }

    #[test]
    fn layers_after_the_end_of_a_shortened_composition_are_left_out() {
        let (mut project, intro, main) = two_scenes();
        let late = crate::edit::add_shape_layer(&mut project, intro, ShapeStyle::new(ShapeKind::Ellipse), Vec2::new(50.0, 50.0), Vec2::new(100.0, 100.0), 50).unwrap();
        project.composition_mut(intro).unwrap().layer_mut(late).unwrap().name = "Late".into();
        // Shortened after the layer was added, which leaves the layer where it was.
        project.composition_mut(intro).unwrap().duration_frames = 40;
        let made = magic_move(&mut project, intro, main, 20, Easing::Linear).unwrap();
        let composition = project.composition(made.composition).unwrap();
        assert!(composition.layers.iter().all(|layer| layer.name != "Late"));
        assert_eq!(crate::project_file::check(&project), Ok(()), "the project can still be saved");
    }

    #[test]
    fn only_matching_compositions_can_be_joined() {
        let (mut project, intro, main) = two_scenes();
        assert_eq!(targets(&project, intro), vec![main]);
        assert_eq!(magic_move(&mut project, intro, intro, 20, Easing::Linear), Err(MagicMoveError::SameComposition));
        project.composition_mut(main).unwrap().width = 1080;
        assert!(targets(&project, intro).is_empty());
        assert_eq!(magic_move(&mut project, intro, main, 20, Easing::Linear), Err(MagicMoveError::Mismatched));
        assert_eq!(magic_move(&mut project, intro, Id(999_999), 20, Easing::Linear), Err(MagicMoveError::Missing));
    }
}
