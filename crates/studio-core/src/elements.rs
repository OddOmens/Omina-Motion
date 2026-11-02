//! Ready-made graphics to drop into a composition, like Canva's Elements: shapes, lines and
//! arrows, badges and labels, photo frames, social buttons, and emoji stickers.
//!
//! Each is made from ordinary layers (shapes, pen paths and text), so it can be recolored,
//! resized and animated like anything else. Elements with words have their text parented to
//! their shape, so the two move together.

use crate::animation::Vec2;
use crate::model::{Id, Paint, Project, Rgba, ShapeKind, ShapeStyle, Stroke, TextStyle};
use crate::path::{PathPoint, ShapePath};

/// How the Elements panel groups them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Shapes,
    Lines,
    Badges,
    Frames,
    Social,
    Stickers,
}

impl Group {
    pub const ALL: [Self; 6] = [Self::Shapes, Self::Lines, Self::Badges, Self::Frames, Self::Social, Self::Stickers];

    pub fn name(self) -> &'static str {
        match self {
            Self::Shapes => "Shapes",
            Self::Lines => "Lines & Arrows",
            Self::Badges => "Badges & Labels",
            Self::Frames => "Frames",
            Self::Social => "Social",
            Self::Stickers => "Stickers",
        }
    }
}

/// What an element is made of.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Shape(ShapeKind, f32),
    Heart,
    Line,
    Arrow,
    CurvedArrow,
    DoubleArrow,
    Burst(&'static str),
    Seal(&'static str),
    Tag(&'static str),
    Ribbon(&'static str),
    Pill(&'static str, u32, u32),
    Speech(&'static str),
    PhotoFrame,
    CircleFrame,
    PhoneFrame,
    Like,
    Emoji(&'static str),
}

/// One element in the catalog.
#[derive(Clone, Copy, Debug)]
pub struct Element {
    pub name: &'static str,
    pub group: Group,
    kind: Kind,
}

const fn element(name: &'static str, group: Group, kind: Kind) -> Element {
    Element { name, group, kind }
}

const EMOJI: [(&str, &str); 16] = [
    ("Fire", "🔥"),
    ("Heart", "❤️"),
    ("Star", "⭐"),
    ("Laughing", "😂"),
    ("Heart Eyes", "😍"),
    ("Party", "🎉"),
    ("Eyes", "👀"),
    ("Hundred", "💯"),
    ("Sparkles", "✨"),
    ("Point Down", "👇"),
    ("Check", "✅"),
    ("Rocket", "🚀"),
    ("Idea", "💡"),
    ("Pin", "📍"),
    ("Cart", "🛒"),
    ("Music", "🎵"),
];

/// Every element, in panel order.
pub static ELEMENTS: [Element; 41] = [
    element("Circle", Group::Shapes, Kind::Shape(ShapeKind::Ellipse, 0.0)),
    element("Square", Group::Shapes, Kind::Shape(ShapeKind::Rectangle, 0.0)),
    element("Rounded Box", Group::Shapes, Kind::Shape(ShapeKind::Rectangle, 48.0)),
    element("Triangle", Group::Shapes, Kind::Shape(ShapeKind::Triangle, 0.0)),
    element("Star", Group::Shapes, Kind::Shape(ShapeKind::Star, 0.0)),
    element("Hexagon", Group::Shapes, Kind::Shape(ShapeKind::Polygon, 0.0)),
    element("Heart", Group::Shapes, Kind::Heart),
    element("Line", Group::Lines, Kind::Line),
    element("Arrow", Group::Lines, Kind::Arrow),
    element("Curved Arrow", Group::Lines, Kind::CurvedArrow),
    element("Double Arrow", Group::Lines, Kind::DoubleArrow),
    element("New Burst", Group::Badges, Kind::Burst("NEW")),
    element("Sale Seal", Group::Badges, Kind::Seal("SALE")),
    element("Price Tag", Group::Badges, Kind::Tag("$29")),
    element("Ribbon", Group::Badges, Kind::Ribbon("LIMITED")),
    element("Label", Group::Badges, Kind::Pill("Link in bio", 0x111111, 0xffffff)),
    element("Speech Bubble", Group::Badges, Kind::Speech("Hi there!")),
    element("Photo Frame", Group::Frames, Kind::PhotoFrame),
    element("Circle Frame", Group::Frames, Kind::CircleFrame),
    element("Phone Frame", Group::Frames, Kind::PhoneFrame),
    element("Subscribe", Group::Social, Kind::Pill("SUBSCRIBE", 0xef4444, 0xffffff)),
    element("Follow", Group::Social, Kind::Pill("+ Follow", 0x0a84ff, 0xffffff)),
    element("Shop Now", Group::Social, Kind::Pill("Shop Now →", 0x111111, 0xffffff)),
    element("Like", Group::Social, Kind::Like),
    element("Swipe Up", Group::Social, Kind::Pill("Swipe up ↑", 0xffffff, 0x111111)),
    element(EMOJI[0].0, Group::Stickers, Kind::Emoji(EMOJI[0].1)),
    element(EMOJI[1].0, Group::Stickers, Kind::Emoji(EMOJI[1].1)),
    element(EMOJI[2].0, Group::Stickers, Kind::Emoji(EMOJI[2].1)),
    element(EMOJI[3].0, Group::Stickers, Kind::Emoji(EMOJI[3].1)),
    element(EMOJI[4].0, Group::Stickers, Kind::Emoji(EMOJI[4].1)),
    element(EMOJI[5].0, Group::Stickers, Kind::Emoji(EMOJI[5].1)),
    element(EMOJI[6].0, Group::Stickers, Kind::Emoji(EMOJI[6].1)),
    element(EMOJI[7].0, Group::Stickers, Kind::Emoji(EMOJI[7].1)),
    element(EMOJI[8].0, Group::Stickers, Kind::Emoji(EMOJI[8].1)),
    element(EMOJI[9].0, Group::Stickers, Kind::Emoji(EMOJI[9].1)),
    element(EMOJI[10].0, Group::Stickers, Kind::Emoji(EMOJI[10].1)),
    element(EMOJI[11].0, Group::Stickers, Kind::Emoji(EMOJI[11].1)),
    element(EMOJI[12].0, Group::Stickers, Kind::Emoji(EMOJI[12].1)),
    element(EMOJI[13].0, Group::Stickers, Kind::Emoji(EMOJI[13].1)),
    element(EMOJI[14].0, Group::Stickers, Kind::Emoji(EMOJI[14].1)),
    element(EMOJI[15].0, Group::Stickers, Kind::Emoji(EMOJI[15].1)),
];

/// The color elements start in.
const ACCENT: u32 = 0x8b5cf6;

/// Adds element `index` of `ELEMENTS` to a composition, centered on `center`, starting at
/// `frame`. Returns its layers, the main one first.
pub fn add(project: &mut Project, comp: Id, index: usize, center: Vec2, frame: i64) -> Vec<Id> {
    let Some(element) = ELEMENTS.get(index) else { return Vec::new() };
    let solid = |hex: u32| Some(Paint::Solid(Rgba::hex(hex)));
    let shape = |project: &mut Project, kind: ShapeKind, size: Vec2, at: Vec2, fill: Option<Paint>, stroke: Option<Stroke>, corner: f32| -> Option<Id> {
        let style = ShapeStyle { fill, stroke, corner_radius: corner, ..ShapeStyle::new(kind) };
        crate::edit::add_shape_layer(project, comp, style, size, at, frame)
    };
    let main = match element.kind {
        Kind::Shape(kind, corner) => {
            let id = shape(project, kind, Vec2::new(320.0, 320.0), center, solid(ACCENT), None, corner);
            let style_points = if kind == ShapeKind::Polygon { Some(6) } else { None };
            if let (Some(id), Some(points)) = (id, style_points) {
                crate::edit::edit_shape(project.composition_mut(comp).expect("added to it"), id, |style| style.points = points);
            }
            id.into_iter().collect()
        }
        Kind::Heart => path_layer(project, comp, &heart(), center, Vec2::new(300.0, 300.0), ShapeStyle { fill: solid(0xef4444), ..ShapeStyle::new(ShapeKind::Path) }, frame),
        Kind::Line => shape(project, ShapeKind::Rectangle, Vec2::new(480.0, 10.0), center, solid(0xffffff), None, 5.0).into_iter().collect(),
        Kind::Arrow => path_layer(project, comp, &polygon(&[(0.0, 0.38), (0.62, 0.38), (0.62, 0.12), (1.0, 0.5), (0.62, 0.88), (0.62, 0.62), (0.0, 0.62)]), center, Vec2::new(420.0, 240.0), ShapeStyle { fill: solid(0xffffff), ..ShapeStyle::new(ShapeKind::Path) }, frame),
        Kind::DoubleArrow => path_layer(project, comp, &polygon(&[(0.0, 0.5), (0.25, 0.12), (0.25, 0.38), (0.75, 0.38), (0.75, 0.12), (1.0, 0.5), (0.75, 0.88), (0.75, 0.62), (0.25, 0.62), (0.25, 0.88)]), center, Vec2::new(460.0, 200.0), ShapeStyle { fill: solid(0xffffff), ..ShapeStyle::new(ShapeKind::Path) }, frame),
        Kind::CurvedArrow => {
            let style = ShapeStyle { fill: None, stroke: Some(Stroke { paint: Paint::Solid(Rgba::hex(0xffffff)), width: 14.0 }), ..ShapeStyle::new(ShapeKind::Path) };
            path_layer(project, comp, &curved_arrow(), center, Vec2::new(420.0, 320.0), style, frame)
        }
        Kind::Burst(words) => with_words(project, comp, frame, center, words, 70.0, 0x111111, |project| {
            let id = shape(project, ShapeKind::Star, Vec2::new(300.0, 300.0), center, solid(0xfacc15), None, 6.0)?;
            crate::edit::edit_shape(project.composition_mut(comp)?, id, |style| {
                style.points = 14;
                style.inner_radius = 0.78;
            });
            Some(id)
        }),
        Kind::Seal(words) => with_words(project, comp, frame, center, words, 72.0, 0xffffff, |project| shape(project, ShapeKind::Ellipse, Vec2::new(280.0, 280.0), center, solid(0xef4444), Some(Stroke { paint: Paint::Solid(Rgba::hex(0xffffff)), width: 10.0 }), 0.0)),
        Kind::Tag(words) => with_words(project, comp, frame, center + Vec2::new(24.0, 0.0), words, 72.0, 0x111111, |project| {
            let tag = polygon(&[(0.0, 0.5), (0.22, 0.0), (1.0, 0.0), (1.0, 1.0), (0.22, 1.0)]);
            path_layer(project, comp, &tag, center, Vec2::new(360.0, 170.0), ShapeStyle { fill: solid(0xfacc15), ..ShapeStyle::new(ShapeKind::Path) }, frame).first().copied()
        }),
        Kind::Ribbon(words) => with_words(project, comp, frame, center, words, 54.0, 0xffffff, |project| {
            let ribbon = polygon(&[(0.0, 0.0), (1.0, 0.0), (0.92, 0.5), (1.0, 1.0), (0.0, 1.0), (0.08, 0.5)]);
            path_layer(project, comp, &ribbon, center, Vec2::new(560.0, 130.0), ShapeStyle { fill: solid(0xdb2777), ..ShapeStyle::new(ShapeKind::Path) }, frame).first().copied()
        }),
        Kind::Pill(words, fill, ink) => {
            let width = words.chars().count() as f32 * 30.0 + 110.0;
            with_words(project, comp, frame, center, words, 52.0, ink, |project| shape(project, ShapeKind::Rectangle, Vec2::new(width, 104.0), center, solid(fill), None, 52.0))
        }
        Kind::Speech(words) => {
            let ids = with_words(project, comp, frame, center, words, 54.0, 0x111111, |project| shape(project, ShapeKind::Rectangle, Vec2::new(420.0, 180.0), center, solid(0xffffff), None, 60.0));
            // The tail, below the bubble's left side, moving with it.
            if let (Some(&bubble), Some(tail)) = (ids.first(), shape(project, ShapeKind::Triangle, Vec2::new(70.0, 60.0), center + Vec2::new(-110.0, 112.0), solid(0xffffff), None, 6.0)) {
                if let Some(composition) = project.composition_mut(comp) {
                    if let Some(transform) = composition.layer_mut(tail).and_then(|layer| layer.transform.as_mut()) {
                        transform.rotation = crate::animation::Animated::constant(200.0);
                    }
                    crate::edit::set_parent(composition, tail, Some(bubble), frame);
                }
                ids.into_iter().chain(std::iter::once(tail)).collect()
            } else {
                ids
            }
        }
        Kind::PhotoFrame => {
            let frame_id = shape(project, ShapeKind::Rectangle, Vec2::new(560.0, 420.0), center, solid(0xffffff), None, 12.0);
            let photo = shape(project, ShapeKind::Rectangle, Vec2::new(520.0, 380.0), center, Some(Paint::Linear { start: Rgba::hex(0x3f3f46), end: Rgba::hex(0x27272a), angle: 60.0 }), None, 6.0);
            parent_pair(project, comp, frame, frame_id, photo, "Drop your photo here")
        }
        Kind::CircleFrame => {
            let ring = shape(project, ShapeKind::Ellipse, Vec2::new(360.0, 360.0), center, Some(Paint::Linear { start: Rgba::hex(0x3f3f46), end: Rgba::hex(0x27272a), angle: 60.0 }), Some(Stroke { paint: Paint::Solid(Rgba::hex(0xffffff)), width: 14.0 }), 0.0);
            if let Some(id) = ring {
                name(project, comp, id, "Drop your photo here");
            }
            ring.into_iter().collect()
        }
        Kind::PhoneFrame => {
            let body = shape(project, ShapeKind::Rectangle, Vec2::new(360.0, 740.0), center, solid(0x18181b), None, 56.0);
            let screen = shape(project, ShapeKind::Rectangle, Vec2::new(330.0, 710.0), center, Some(Paint::Linear { start: Rgba::hex(0x3f3f46), end: Rgba::hex(0x27272a), angle: 60.0 }), None, 42.0);
            parent_pair(project, comp, frame, body, screen, "Drop your screen recording here")
        }
        Kind::Like => text_layer(project, comp, frame, center, "♥", "Helvetica Neue", 260.0, 0xef4444).into_iter().collect(),
        Kind::Emoji(emoji) => text_layer(project, comp, frame, center, emoji, "Apple Color Emoji", 220.0, 0xffffff).into_iter().collect(),
    };
    if let Some(&first) = main.first() {
        name(project, comp, first, element.name);
    }
    main
}

fn name(project: &mut Project, comp: Id, layer: Id, name: &str) {
    if let Some(found) = project.composition_mut(comp).and_then(|composition| composition.layer_mut(layer)) {
        found.name = name.to_owned();
    }
}

/// A text layer in `font` at `size`, centered on `center`.
#[allow(clippy::too_many_arguments)]
fn text_layer(project: &mut Project, comp: Id, frame: i64, center: Vec2, words: &str, font: &str, size: f32, ink: u32) -> Option<Id> {
    let id = crate::edit::add_text_layer(project, comp, words, center, frame)?;
    let style = TextStyle { font_family: font.into(), font_weight: 800, font_size: size, line_height: size * 1.15, fill: Some(Rgba::hex(ink)), ..TextStyle::default() };
    let composition = project.composition_mut(comp)?;
    crate::edit::edit_text(composition, id, |_, current| *current = style.clone());
    crate::edit::set_text_box(composition, id, style.estimate_size(words));
    Some(id)
}

/// A shape made by `make`, with `words` on it parented to it. Returns both, the shape first.
#[allow(clippy::too_many_arguments)]
fn with_words(project: &mut Project, comp: Id, frame: i64, center: Vec2, words: &str, size: f32, ink: u32, make: impl FnOnce(&mut Project) -> Option<Id>) -> Vec<Id> {
    let Some(base) = make(project) else { return Vec::new() };
    let Some(text) = text_layer(project, comp, frame, center, words, "Avenir Next", size, ink) else { return vec![base] };
    if let Some(composition) = project.composition_mut(comp) {
        crate::edit::set_parent(composition, text, Some(base), frame);
    }
    vec![base, text]
}

/// A frame and what goes in it, the inner one parented to the outer and named for what to drop.
fn parent_pair(project: &mut Project, comp: Id, frame: i64, outer: Option<Id>, inner: Option<Id>, inner_name: &str) -> Vec<Id> {
    let (Some(outer), Some(inner)) = (outer, inner) else { return outer.into_iter().chain(inner).collect() };
    name(project, comp, inner, inner_name);
    if let Some(composition) = project.composition_mut(comp) {
        crate::edit::set_parent(composition, inner, Some(outer), frame);
    }
    vec![outer, inner]
}

/// A pen-path layer from `path` (in a 0–1 box), `size` pixels wide and high, centered on `center`.
fn path_layer(project: &mut Project, comp: Id, path: &ShapePath, center: Vec2, size: Vec2, style: ShapeStyle, frame: i64) -> Vec<Id> {
    let placed = path.mapped(|v| center + Vec2::new((v.x - 0.5) * size.x, (v.y - 0.5) * size.y));
    let id = crate::edit::add_path_layer(project, comp, &placed, style, frame);
    // `add_path_layer` gives open paths a default stroke; keep the one asked for.
    if let (Some(id), Some(stroke)) = (id, style.stroke)
        && let Some(composition) = project.composition_mut(comp)
    {
        crate::edit::edit_shape(composition, id, |current| current.stroke = Some(stroke));
    }
    id.into_iter().collect()
}

/// A closed outline through `corners` (0–1 box).
fn polygon(corners: &[(f32, f32)]) -> ShapePath {
    ShapePath { points: corners.iter().map(|&(x, y)| PathPoint::corner(Vec2::new(x, y))).collect(), closed: true }
}

/// A heart, drawn with curves.
fn heart() -> ShapePath {
    let point = |x: f32, y: f32, hin: (f32, f32), hout: (f32, f32)| PathPoint { at: Vec2::new(x, y), handle_in: Vec2::new(hin.0, hin.1), handle_out: Vec2::new(hout.0, hout.1) };
    ShapePath {
        points: vec![
            point(0.5, 0.28, (-0.08, -0.22), (0.08, -0.22)),
            point(0.95, 0.32, (0.0, -0.3), (0.0, 0.25)),
            point(0.5, 0.92, (0.25, -0.2), (-0.25, -0.2)),
            point(0.05, 0.32, (0.0, 0.25), (0.0, -0.3)),
        ],
        closed: true,
    }
}

/// An open curve sweeping up to the right, ending in an arrowhead.
fn curved_arrow() -> ShapePath {
    ShapePath {
        points: vec![
            PathPoint { at: Vec2::new(0.05, 0.85), handle_in: Vec2::default(), handle_out: Vec2::new(0.1, -0.5) },
            PathPoint { at: Vec2::new(0.9, 0.2), handle_in: Vec2::new(-0.45, 0.05), handle_out: Vec2::default() },
            PathPoint::corner(Vec2::new(0.74, 0.08)),
            PathPoint::corner(Vec2::new(0.9, 0.2)),
            PathPoint::corner(Vec2::new(0.76, 0.34)),
        ],
        closed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{ids, sample_project};

    #[test]
    fn every_element_adds_layers_that_pass_the_file_checks() {
        let mut project = sample_project();
        for (index, element) in ELEMENTS.iter().enumerate() {
            let added = add(&mut project, ids::MAIN_COMP, index, Vec2::new(960.0, 540.0), 0);
            assert!(!added.is_empty(), "{} added nothing", element.name);
        }
        let composition = project.composition(ids::MAIN_COMP).unwrap();
        let view = crate::project_file::EditorView { active_composition: ids::MAIN_COMP, current_frame: 0, work_area: crate::time::FrameRange::new(0.0, composition.duration_frames as f64) };
        let text = crate::project_file::to_text(&project, view).unwrap();
        crate::project_file::from_text(&text).unwrap();
        for group in Group::ALL {
            assert!(ELEMENTS.iter().any(|element| element.group == group), "{} is empty", group.name());
        }
    }

    #[test]
    fn a_badges_words_move_with_it() {
        let mut project = sample_project();
        let index = ELEMENTS.iter().position(|element| element.name == "Sale Seal").unwrap();
        let added = add(&mut project, ids::MAIN_COMP, index, Vec2::new(500.0, 400.0), 0);
        let composition = project.composition(ids::MAIN_COMP).unwrap();
        assert_eq!(composition.layer(added[1]).unwrap().parent, Some(added[0]));
    }
}
