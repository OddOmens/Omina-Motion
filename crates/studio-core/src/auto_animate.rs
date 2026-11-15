//! Animate for me: gives a still design an entrance in one go, the way a motion designer would
//! rough it in. Each layer gets a way in that suits what it is (the headline's words rise in,
//! photos zoom, small shapes pop, thin lines draw on, other text slides up), and layers that
//! start together come in one after another in reading order, top to bottom, left to right.
//!
//! It is built from the one-click In animations (`presets`) and text animators
//! (`text_animation`), so every choice shows in the Motion panel and can be changed or removed
//! there. Layers animated by hand, backgrounds, video clips and sound are left alone, and
//! running it again replaces what it did before rather than adding to it.

use crate::align::{Rect, layer_bounds};
use crate::model::{Composition, Id, Layer, LayerContent, Project};
use crate::presets::{Loop, Motion, Phase};
use crate::text_animation::{TextAnimator, TextEase, TextMotion, TextOrder, TextUnit};

/// The overall feel asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    /// Gentle rises, zooms and fades: suits most designs.
    #[default]
    Smooth,
    /// Pops, bounces and a little life in the small shapes.
    Bouncy,
    /// Quick slides with little delay between layers.
    Snappy,
}

impl Style {
    pub const ALL: [Self; 3] = [Self::Smooth, Self::Bouncy, Self::Snappy];

    pub fn name(self) -> &'static str {
        match self {
            Self::Smooth => "smooth",
            Self::Bouncy => "bouncy",
            Self::Snappy => "snappy",
        }
    }

    /// How long each layer takes to come in, and the delay between neighbors, in seconds.
    fn timing(self) -> (f32, f64) {
        match self {
            Self::Smooth => (0.7, 0.15),
            Self::Bouncy => (0.6, 0.12),
            Self::Snappy => (0.4, 0.07),
        }
    }
}

/// The longest any layer is held back so that it follows its neighbors, in seconds, so a busy
/// design still finishes arriving promptly.
const MAX_DELAY: f64 = 1.2;
/// Layers starting within this many seconds of each other are treated as arriving together.
const TOGETHER: f64 = 1.5;

/// What a layer is, for choosing how it comes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Role {
    Headline,
    Text,
    Photo,
    /// A thin bar or rule.
    Line,
    /// A small shape: an icon, badge, dot or sparkle.
    Accent,
    /// A larger shape or panel that isn't the background.
    Panel,
    /// A video clip, only when chosen on purpose.
    Clip,
}

impl Role {
    /// How a layer of this role is described in the summary: one, and several.
    fn names(self) -> (&'static str, &'static str) {
        match self {
            Self::Headline => ("headline", "headlines"),
            Self::Text => ("line of text", "lines of text"),
            Self::Photo => ("photo", "photos"),
            Self::Line => ("line", "lines"),
            Self::Accent => ("small shape", "small shapes"),
            Self::Panel => ("panel", "panels"),
            Self::Clip => ("clip", "clips"),
        }
    }
}

/// How one layer comes in: as a whole (`Motion`) or piece by piece (a text animator).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Entrance {
    Layer(Motion),
    Text(TextAnimator),
}

impl Entrance {
    /// What it does, in a few words, for the summary ("rise in", "pop in").
    fn verb(self) -> &'static str {
        match self {
            Self::Text(animator) => match animator.motion {
                TextMotion::Pop => "pop in letter by letter",
                TextMotion::SlideLeft | TextMotion::SlideRight => "slide in word by word",
                _ => "rise in word by word",
            },
            Self::Layer(motion) => match motion {
                Motion::Fade => "fade in",
                Motion::Rise => "rise in",
                Motion::Drop => "drop in",
                Motion::SlideLeft | Motion::SlideRight => "slide in",
                Motion::Pop => "pop in",
                Motion::Zoom => "zoom in",
                Motion::Spin => "spin in",
                Motion::Blur => "come into focus",
                Motion::Bounce => "bounce in",
                Motion::Typewriter => "draw on",
            },
        }
    }
}

/// The way in for a layer of `role` in `style`. `short` is for a few characters of text.
fn entrance(role: Role, style: Style, seconds: f32, short: bool) -> Entrance {
    let text = |motion: TextMotion, unit: TextUnit, ease: TextEase, overlap: f32| Entrance::Text(TextAnimator { motion, unit, order: TextOrder::Forward, ease, seconds: seconds * 1.6, overlap });
    match (role, style) {
        (Role::Headline, Style::Smooth) => text(TextMotion::Rise, TextUnit::Word, TextEase::Smooth, 0.55),
        (Role::Headline, Style::Bouncy) if short => text(TextMotion::Pop, TextUnit::Character, TextEase::Overshoot, 0.7),
        (Role::Headline, Style::Bouncy) => text(TextMotion::Rise, TextUnit::Word, TextEase::Overshoot, 0.5),
        (Role::Headline, Style::Snappy) => text(TextMotion::SlideLeft, TextUnit::Word, TextEase::Smooth, 0.35),
        (Role::Text, Style::Smooth) => Entrance::Layer(Motion::Rise),
        (Role::Text, Style::Bouncy) => Entrance::Layer(Motion::Pop),
        (Role::Text, Style::Snappy) => Entrance::Layer(Motion::SlideLeft),
        (Role::Photo, Style::Smooth) => Entrance::Layer(Motion::Zoom),
        (Role::Photo, Style::Bouncy) => Entrance::Layer(Motion::Pop),
        (Role::Photo, Style::Snappy) => Entrance::Layer(Motion::SlideRight),
        (Role::Line, _) => Entrance::Layer(Motion::Typewriter),
        (Role::Accent, Style::Bouncy) => Entrance::Layer(Motion::Spin),
        (Role::Accent, _) => Entrance::Layer(Motion::Pop),
        (Role::Panel, Style::Smooth) => Entrance::Layer(Motion::Fade),
        (Role::Panel, Style::Bouncy) => Entrance::Layer(Motion::Rise),
        (Role::Panel, Style::Snappy) => Entrance::Layer(Motion::SlideLeft),
        (Role::Clip, _) => Entrance::Layer(Motion::Fade),
    }
}

/// What animating did, for the assistant to describe.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    /// Each layer animated, with how it now comes in, in the order they arrive.
    pub animated: Vec<(Id, String)>,
    /// One line per kind of layer: "the headline rises in word by word", "3 photos zoom in".
    pub described: Vec<String>,
    /// Layers left as they were because they already have keyframes or text animation of
    /// their own.
    pub kept: usize,
    /// Backgrounds, which are left still so the frame is never empty.
    pub backgrounds: usize,
}

/// Whether a layer moves already in a way the person made: keyframes, or a text animator, not
/// written by the one-click animations this replaces.
fn animated_by_hand(layer: &Layer) -> bool {
    if layer.animation.enter.is_some() || layer.animation.text_in.is_some() {
        return false;
    }
    let keyed = layer.transform.as_ref().is_some_and(|transform| transform.position.is_animated() || transform.scale.is_animated() || transform.rotation.is_animated() || transform.opacity.is_animated());
    keyed || layer.effects.iter().any(|effect| effect.properties.iter().any(|property| property.value.is_animated()))
}

/// What a layer is, or `None` for one this leaves alone (sound, adjustments, captions, the
/// background, and video clips unless `chosen`).
fn role(composition: &Composition, layer: &Layer, bounds: Rect, biggest_text: f32, chosen: bool) -> Option<Role> {
    let frame_area = (composition.width as f32 * composition.height as f32).max(1.0);
    let size = bounds.max - bounds.min;
    let area = (size.x * size.y).max(0.0) / frame_area;
    let thin = size.x.max(size.y) / size.x.min(size.y).max(1.0) > 6.0;
    // Filling (nearly) the whole frame: the background, which stays put unless chosen.
    let fills = area > 0.85;
    match &layer.content {
        LayerContent::Text { text, style } => {
            let scale = layer.transform.as_ref().map_or(1.0, |transform| transform.scale.value.y.abs() / 100.0);
            let shown = style.font_size * scale;
            let words = text.split_whitespace().count();
            Some(if shown >= biggest_text * 0.75 && words <= 12 { Role::Headline } else { Role::Text })
        }
        LayerContent::Image { .. } | LayerContent::Composition { .. } if fills && !chosen => None,
        LayerContent::Image { .. } | LayerContent::Composition { .. } => Some(Role::Photo),
        LayerContent::Shape { .. } if crate::slots::is_placeholder(layer) => (!fills || chosen).then_some(Role::Photo),
        LayerContent::Shape { .. } | LayerContent::Solid { .. } if fills && !chosen => None,
        LayerContent::Shape { .. } | LayerContent::Solid { .. } if thin => Some(Role::Line),
        LayerContent::Shape { .. } | LayerContent::Solid { .. } if area < 0.06 => Some(Role::Accent),
        LayerContent::Shape { .. } | LayerContent::Solid { .. } => Some(Role::Panel),
        LayerContent::Footage { .. } if chosen => Some(Role::Clip),
        _ => None,
    }
}

/// One layer to animate, with what it is and where it sits.
struct Planned {
    id: Id,
    role: Role,
    short: bool,
    center: crate::animation::Vec2,
    /// How far up the stack it is, from the bottom layer (0).
    depth: usize,
    in_frame: i64,
    out_frame: i64,
}

/// Animates composition `comp`: the layers in `only` when it names any (the selection), or every
/// layer. Layers starting together are staggered by holding later ones back a little (their
/// in point moves, never earlier), so running it twice gives the same result. Returns what was
/// done; nothing changes when there was nothing to animate.
pub fn animate(project: &mut Project, comp: Id, only: &[Id], style: Style) -> Summary {
    let mut summary = Summary::default();
    let Some(composition) = project.composition(comp) else { return summary };
    let fps = composition.frame_rate.fps();
    let chosen = |layer: &Layer| only.is_empty() || only.contains(&layer.id);
    let candidates: Vec<&Layer> = composition.layers.iter().filter(|layer| chosen(layer) && layer.switches.visible && !layer.switches.locked && layer.transform.is_some()).collect();
    let biggest_text = candidates
        .iter()
        .filter_map(|layer| match &layer.content {
            LayerContent::Text { style, .. } => Some(style.font_size * layer.transform.as_ref().map_or(1.0, |transform| transform.scale.value.y.abs() / 100.0)),
            _ => None,
        })
        .fold(0.0, f32::max);
    let count = composition.layers.len();
    let depth_of = |id: Id| composition.layers.iter().position(|layer| layer.id == id).map_or(0, |index| count - 1 - index);
    let mut planned = Vec::new();
    for layer in candidates {
        // Where it sits once settled: a few seconds in, after any entrance an earlier run gave
        // it, so running again reads the design the same way.
        let settled = (layer.in_frame + (3.0 * fps) as i64).min(layer.out_frame - 1).max(layer.in_frame);
        let Some(bounds) = layer_bounds(composition, layer.id, settled) else { continue };
        let Some(role) = role(composition, layer, bounds, biggest_text, !only.is_empty()) else {
            if matches!(layer.content, LayerContent::Image { .. } | LayerContent::Composition { .. } | LayerContent::Shape { .. } | LayerContent::Solid { .. }) {
                summary.backgrounds += 1;
            }
            continue;
        };
        if animated_by_hand(layer) {
            summary.kept += 1;
            continue;
        }
        let short = matches!(&layer.content, LayerContent::Text { text, .. } if text.chars().filter(|c| !c.is_whitespace()).count() <= 10);
        planned.push(Planned { id: layer.id, role, short, center: bounds.center(), depth: depth_of(layer.id), in_frame: layer.in_frame, out_frame: layer.out_frame });
    }
    if planned.is_empty() {
        return summary;
    }

    // Layers that start together arrive in reading order: groups by start time, then top to
    // bottom (rows a tenth of the frame tall), then left to right (columns a quarter of the
    // frame wide). Layers in the same place come in back to front, so a button's pill is there
    // before its label and a frame before the photo in it.
    let (row, column) = (composition.height as f32 / 10.0, composition.width as f32 / 4.0);
    let place = |layer: &Planned| ((layer.center.y / row).floor() as i64, (layer.center.x / column).floor() as i64, layer.depth);
    planned.sort_by_key(|plan| plan.in_frame);
    let (seconds, step) = style.timing();
    let together = (TOGETHER * fps) as i64;
    let mut groups: Vec<Vec<Planned>> = Vec::new();
    for layer in planned {
        match groups.last_mut() {
            Some(group) if layer.in_frame <= group[0].in_frame + together => group.push(layer),
            _ => groups.push(vec![layer]),
        }
    }
    let mut order = Vec::new();
    for mut group in groups {
        let start = group.iter().map(|layer| layer.in_frame).min().unwrap_or(0);
        group.sort_by_key(place);
        // The headline leads, as the first thing to read.
        if let Some(index) = group.iter().position(|layer| layer.role == Role::Headline) {
            let headline = group.remove(index);
            group.insert(0, headline);
        }
        for (index, layer) in group.into_iter().enumerate() {
            let delay = ((index as f64 * step).min(MAX_DELAY) * fps).round() as i64;
            // Never moved earlier, never left too short to finish coming in, and video clips
            // keep their place in the edit.
            let slot = (start + delay).max(layer.in_frame);
            let room = (f64::from(seconds) * fps * 2.0) as i64;
            let in_frame = if layer.role != Role::Clip && slot + room <= layer.out_frame { slot } else { layer.in_frame };
            order.push((layer, in_frame));
        }
    }

    let mut tally: Vec<(Role, Entrance, usize)> = Vec::new();
    for (layer, in_frame) in order {
        if let Some(found) = project.composition_mut(comp).and_then(|composition| composition.layer_mut(layer.id)) {
            found.in_frame = in_frame;
        }
        // What an earlier run chose is replaced, whichever kind it was.
        crate::presets::set_motion(project, comp, layer.id, Phase::In, None, seconds);
        crate::text_animation::set_animator(project, comp, layer.id, Phase::In, None);
        let way_in = entrance(layer.role, style, seconds, layer.short);
        let done = match way_in {
            Entrance::Layer(motion) => crate::presets::set_motion(project, comp, layer.id, Phase::In, Some(motion), seconds),
            Entrance::Text(animator) => crate::text_animation::set_animator(project, comp, layer.id, Phase::In, Some(animator)),
        };
        if !done {
            continue;
        }
        // In the bouncy style, small shapes keep a gentle pulse once they are in.
        if style == Style::Bouncy && layer.role == Role::Accent {
            crate::presets::set_loop(project, comp, layer.id, Some(Loop::Pulse));
        }
        summary.animated.push((layer.id, way_in.verb().to_owned()));
        match tally.iter_mut().find(|(role, entrance, _)| *role == layer.role && entrance.verb() == way_in.verb()) {
            Some((_, _, count)) => *count += 1,
            None => tally.push((layer.role, way_in, 1)),
        }
    }
    tally.sort_by_key(|(role, ..)| *role);
    summary.described = tally
        .into_iter()
        .map(|(role, way_in, count)| {
            let (one, many) = role.names();
            // "rise in" reads "the headline rises in"; several read "3 photos zoom in".
            if count == 1 {
                let (first, rest) = way_in.verb().split_once(' ').unwrap_or((way_in.verb(), ""));
                let verb = if first.ends_with('h') || first.ends_with('s') { format!("{first}es") } else { format!("{first}s") };
                format!("the {one} {verb} {rest}").trim_end().to_owned()
            } else {
                format!("{count} {many} {}", way_in.verb())
            }
        })
        .collect();
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Project;
    use crate::templates::{Format, TEMPLATES, build};

    /// A template's composition with every animation taken off and every layer starting at
    /// frame 0: a still design, as someone might have laid it out by hand.
    fn still_design(name: &str) -> (Project, Id) {
        let template = TEMPLATES.iter().find(|template| template.name == name).unwrap();
        let mut project = Project::new(Vec::new(), Vec::new());
        let comp = build(&mut project, template, Format::Vertical).unwrap();
        let composition = project.composition_mut(comp).unwrap();
        for layer in &mut composition.layers {
            layer.in_frame = 0;
            layer.animation = Default::default();
            layer.effects.clear();
            if let Some(transform) = layer.transform.as_mut() {
                for keyed in [&mut transform.opacity, &mut transform.rotation] {
                    *keyed = crate::animation::Animated::constant(keyed.value_at(150.0));
                }
                transform.position = crate::animation::Animated::constant(transform.position.value_at(150.0));
                transform.scale = crate::animation::Animated::constant(transform.scale.value_at(150.0));
            }
        }
        (project, comp)
    }

    #[test]
    fn a_still_design_comes_in_layer_by_layer_with_the_headline_first() {
        let (mut project, comp) = still_design("Launch Day");
        let summary = animate(&mut project, comp, &[], Style::Smooth);
        assert!(summary.animated.len() >= 4, "{summary:?}");
        assert_eq!(summary.backgrounds, 1, "the background stays still");
        let composition = project.composition(comp).unwrap();
        let first = composition.layer(summary.animated[0].0).unwrap();
        assert!(matches!(first.content, LayerContent::Text { .. }) && first.animation.text_in.is_some(), "the headline leads, word by word");
        // They arrive one after another.
        let starts: Vec<i64> = summary.animated.iter().map(|(id, _)| composition.layer(*id).unwrap().in_frame).collect();
        assert!(starts.windows(2).all(|pair| pair[0] <= pair[1]) && starts.last() > starts.first(), "{starts:?}");
        assert!(summary.described.iter().any(|line| line.starts_with("the headline")), "{:?}", summary.described);
    }

    #[test]
    fn running_it_again_changes_nothing_and_another_style_replaces_the_first() {
        let (mut project, comp) = still_design("Launch Day");
        animate(&mut project, comp, &[], Style::Smooth);
        let once = project.composition(comp).unwrap().clone();
        animate(&mut project, comp, &[], Style::Smooth);
        assert_eq!(project.composition(comp).unwrap(), &once, "the same again");
        let summary = animate(&mut project, comp, &[], Style::Bouncy);
        assert_eq!(summary.kept, 0, "its own animation isn't mistaken for the person's");
        let composition = project.composition(comp).unwrap();
        assert!(summary.animated.iter().all(|(id, _)| {
            let layer = composition.layer(*id).unwrap();
            layer.animation.enter.is_some() != layer.animation.text_in.is_some()
        }), "each layer has exactly one way in");
    }

    #[test]
    fn only_the_selection_and_nothing_animated_by_hand() {
        let (mut project, comp) = still_design("Launch Day");
        let composition = project.composition_mut(comp).unwrap();
        let ids: Vec<Id> = composition.layers.iter().filter(|layer| matches!(layer.content, LayerContent::Text { .. })).map(|layer| layer.id).collect();
        // The person animated the first text layer themselves.
        let hand = composition.layer_mut(ids[0]).unwrap();
        hand.transform.as_mut().unwrap().opacity = crate::animation::Animated::keyframed(vec![crate::animation::Keyframe { frame: 0, value: 0.0, easing: crate::animation::Easing::Linear }, crate::animation::Keyframe { frame: 10, value: 100.0, easing: crate::animation::Easing::Linear }]);
        let summary = animate(&mut project, comp, &ids[..2], Style::Snappy);
        assert_eq!(summary.kept, 1);
        assert_eq!(summary.animated.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![ids[1]]);
    }

    #[test]
    fn every_template_can_be_animated_and_still_saves() {
        for template in &TEMPLATES {
            let (mut project, comp) = still_design(template.name);
            for style in Style::ALL {
                animate(&mut project, comp, &[], style);
            }
            let view = crate::project_file::EditorView { active_composition: comp, current_frame: 0, work_area: crate::time::FrameRange::new(0.0, 30.0) };
            let text = crate::project_file::to_text(&project, view).unwrap();
            crate::project_file::from_text(&text).unwrap_or_else(|error| panic!("{}: {error}", template.name));
        }
    }
}
