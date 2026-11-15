//! Ready-made graphics to drop into a composition, like Canva's Elements: shapes, lines and
//! arrows, callouts for pointing things out, titles and captions, badges and labels, photo
//! frames, social buttons, and emoji stickers.
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
    Callouts,
    Titles,
    Badges,
    Frames,
    Social,
    Stickers,
}

impl Group {
    pub const ALL: [Self; 8] = [Self::Shapes, Self::Lines, Self::Callouts, Self::Titles, Self::Badges, Self::Frames, Self::Social, Self::Stickers];

    pub fn name(self) -> &'static str {
        match self {
            Self::Shapes => "Shapes",
            Self::Lines => "Lines & Arrows",
            Self::Callouts => "Callouts",
            Self::Titles => "Titles & Captions",
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
    /// A regular polygon with this many corners.
    Sided(u32),
    /// A filled shape of a given size and color: kind, corner radius, (width, height), color.
    Block(ShapeKind, f32, (f32, f32), u32),
    /// A shape drawn as an outline only: kind, corner radius, (width, height), color, line width.
    Outline(ShapeKind, f32, (f32, f32), u32, f32),
    /// A filled outline through corners given in a 0–1 box: corners, (width, height), color.
    Cutout(&'static [(f32, f32)], (f32, f32), u32),
    /// A filled circle with a character or two on it: words, fill, ink.
    Disc(&'static str, u32, u32),
    /// Words on their own: text, size, ink.
    Words(&'static str, f32, u32),
    /// A dark rounded box with a line of text, as captions are shown.
    Caption(&'static str),
    /// A name and a role beside an accent bar, on a dark plate.
    LowerThird,
    /// A large title over a smaller line.
    TitleCard(&'static str, &'static str),
    /// An empty picture slot to drop a photo or video on: kind, (width, height), corner radius.
    Slot(ShapeKind, (f32, f32), f32),
    Polaroid,
    BrowserWindow,
    SplitScreen,
    /// Black bars across the top and bottom of the frame, for a widescreen look.
    CinemaBars,
    PlayButton,
    ProgressBar,
    Heart,
    Line,
    CurvedArrow,
    Burst(&'static str),
    Seal(&'static str),
    Tag(&'static str),
    Ribbon(&'static str),
    Pill(&'static str, u32, u32),
    Speech(&'static str),
    PhotoFrame,
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

/// Emoji stickers: the name shown, and the emoji.
const EMOJI: [(&str, &str); 40] = [
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
    ("Thumbs Up", "👍"),
    ("Clap", "👏"),
    ("Raised Hands", "🙌"),
    ("Surprised", "😮"),
    ("Thinking", "🤔"),
    ("Cool", "😎"),
    ("Celebrate", "🥳"),
    ("Crying", "😢"),
    ("Boom", "💥"),
    ("Lightning", "⚡"),
    ("Target", "🎯"),
    ("Trophy", "🏆"),
    ("Money", "💰"),
    ("Megaphone", "📣"),
    ("Alarm", "⏰"),
    ("Camera", "📸"),
    ("Clapperboard", "🎬"),
    ("Gift", "🎁"),
    ("Bell", "🔔"),
    ("Chat", "💬"),
    ("Chart Up", "📈"),
    ("Point Right", "👉"),
    ("Point Up", "👆"),
    ("No", "❌"),
];

// Outlines in a 0–1 box, for the elements cut from a single shape.
const ARROW_RIGHT: &[(f32, f32)] = &[(0.0, 0.38), (0.62, 0.38), (0.62, 0.12), (1.0, 0.5), (0.62, 0.88), (0.62, 0.62), (0.0, 0.62)];
const ARROW_LEFT: &[(f32, f32)] = &[(1.0, 0.38), (0.38, 0.38), (0.38, 0.12), (0.0, 0.5), (0.38, 0.88), (0.38, 0.62), (1.0, 0.62)];
const ARROW_UP: &[(f32, f32)] = &[(0.38, 1.0), (0.38, 0.38), (0.12, 0.38), (0.5, 0.0), (0.88, 0.38), (0.62, 0.38), (0.62, 1.0)];
const ARROW_DOWN: &[(f32, f32)] = &[(0.38, 0.0), (0.38, 0.62), (0.12, 0.62), (0.5, 1.0), (0.88, 0.62), (0.62, 0.62), (0.62, 0.0)];
const DOUBLE_ARROW: &[(f32, f32)] = &[(0.0, 0.5), (0.25, 0.12), (0.25, 0.38), (0.75, 0.38), (0.75, 0.12), (1.0, 0.5), (0.75, 0.88), (0.75, 0.62), (0.25, 0.62), (0.25, 0.88)];
const CHEVRON: &[(f32, f32)] = &[(0.2, 0.0), (0.45, 0.0), (0.85, 0.5), (0.45, 1.0), (0.2, 1.0), (0.6, 0.5)];
const CORNER_MARK: &[(f32, f32)] = &[(0.0, 0.0), (1.0, 0.0), (1.0, 0.14), (0.14, 0.14), (0.14, 1.0), (0.0, 1.0)];
const DIAMOND: &[(f32, f32)] = &[(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)];
const PLUS: &[(f32, f32)] = &[(0.38, 0.0), (0.62, 0.0), (0.62, 0.38), (1.0, 0.38), (1.0, 0.62), (0.62, 0.62), (0.62, 1.0), (0.38, 1.0), (0.38, 0.62), (0.0, 0.62), (0.0, 0.38), (0.38, 0.38)];
const CROSS: &[(f32, f32)] = &[(0.15, 0.0), (0.5, 0.35), (0.85, 0.0), (1.0, 0.15), (0.65, 0.5), (1.0, 0.85), (0.85, 1.0), (0.5, 0.65), (0.15, 1.0), (0.0, 0.85), (0.35, 0.5), (0.0, 0.15)];
const CHECK: &[(f32, f32)] = &[(0.0, 0.55), (0.14, 0.41), (0.36, 0.63), (0.86, 0.13), (1.0, 0.27), (0.36, 0.91)];
const PARALLELOGRAM: &[(f32, f32)] = &[(0.2, 0.0), (1.0, 0.0), (0.8, 1.0), (0.0, 1.0)];
const LIGHTNING: &[(f32, f32)] = &[(0.6, 0.0), (0.15, 0.55), (0.45, 0.55), (0.35, 1.0), (0.85, 0.4), (0.55, 0.4)];
const BOOKMARK: &[(f32, f32)] = &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.5, 0.75), (0.0, 1.0)];
const PLAY: &[(f32, f32)] = &[(0.1, 0.0), (1.0, 0.5), (0.1, 1.0)];

/// The elements other than the emoji stickers, in panel order.
const DRAWN: &[Element] = &[
    element("Circle", Group::Shapes, Kind::Shape(ShapeKind::Ellipse, 0.0)),
    element("Square", Group::Shapes, Kind::Shape(ShapeKind::Rectangle, 0.0)),
    element("Rounded Box", Group::Shapes, Kind::Shape(ShapeKind::Rectangle, 48.0)),
    element("Pill", Group::Shapes, Kind::Block(ShapeKind::Rectangle, 80.0, (420.0, 160.0), ACCENT)),
    element("Triangle", Group::Shapes, Kind::Shape(ShapeKind::Triangle, 0.0)),
    element("Diamond", Group::Shapes, Kind::Cutout(DIAMOND, (300.0, 300.0), ACCENT)),
    element("Pentagon", Group::Shapes, Kind::Sided(5)),
    element("Hexagon", Group::Shapes, Kind::Sided(6)),
    element("Octagon", Group::Shapes, Kind::Sided(8)),
    element("Star", Group::Shapes, Kind::Shape(ShapeKind::Star, 0.0)),
    element("Heart", Group::Shapes, Kind::Heart),
    element("Ring", Group::Shapes, Kind::Outline(ShapeKind::Ellipse, 0.0, (320.0, 320.0), ACCENT, 24.0)),
    element("Outline Box", Group::Shapes, Kind::Outline(ShapeKind::Rectangle, 24.0, (420.0, 300.0), ACCENT, 16.0)),
    element("Plus", Group::Shapes, Kind::Cutout(PLUS, (280.0, 280.0), ACCENT)),
    element("Parallelogram", Group::Shapes, Kind::Cutout(PARALLELOGRAM, (420.0, 220.0), ACCENT)),
    element("Lightning Bolt", Group::Shapes, Kind::Cutout(LIGHTNING, (240.0, 340.0), 0xfacc15)),
    element("Bookmark", Group::Shapes, Kind::Cutout(BOOKMARK, (220.0, 320.0), 0xef4444)),
    element("Line", Group::Lines, Kind::Line),
    element("Thin Line", Group::Lines, Kind::Block(ShapeKind::Rectangle, 2.0, (640.0, 4.0), 0xffffff)),
    element("Vertical Line", Group::Lines, Kind::Block(ShapeKind::Rectangle, 5.0, (10.0, 480.0), 0xffffff)),
    element("Arrow", Group::Lines, Kind::Cutout(ARROW_RIGHT, (420.0, 240.0), 0xffffff)),
    element("Arrow Left", Group::Lines, Kind::Cutout(ARROW_LEFT, (420.0, 240.0), 0xffffff)),
    element("Arrow Up", Group::Lines, Kind::Cutout(ARROW_UP, (240.0, 420.0), 0xffffff)),
    element("Arrow Down", Group::Lines, Kind::Cutout(ARROW_DOWN, (240.0, 420.0), 0xffffff)),
    element("Curved Arrow", Group::Lines, Kind::CurvedArrow),
    element("Double Arrow", Group::Lines, Kind::Cutout(DOUBLE_ARROW, (460.0, 200.0), 0xffffff)),
    element("Chevron", Group::Lines, Kind::Cutout(CHEVRON, (200.0, 280.0), 0xffffff)),
    element("Corner Mark", Group::Lines, Kind::Cutout(CORNER_MARK, (180.0, 180.0), 0xffffff)),
    element("Highlight Ring", Group::Callouts, Kind::Outline(ShapeKind::Ellipse, 0.0, (360.0, 360.0), 0xef4444, 12.0)),
    element("Highlight Box", Group::Callouts, Kind::Outline(ShapeKind::Rectangle, 12.0, (480.0, 300.0), 0xef4444, 10.0)),
    element("Underline", Group::Callouts, Kind::Block(ShapeKind::Rectangle, 8.0, (420.0, 16.0), 0xfacc15)),
    element("Highlighter", Group::Callouts, Kind::Block(ShapeKind::Rectangle, 6.0, (460.0, 70.0), 0xfde047)),
    element("Check Mark", Group::Callouts, Kind::Cutout(CHECK, (260.0, 220.0), 0x22c55e)),
    element("Cross Mark", Group::Callouts, Kind::Cutout(CROSS, (240.0, 240.0), 0xef4444)),
    element("Step 1", Group::Callouts, Kind::Disc("1", ACCENT, 0xffffff)),
    element("Step 2", Group::Callouts, Kind::Disc("2", ACCENT, 0xffffff)),
    element("Step 3", Group::Callouts, Kind::Disc("3", ACCENT, 0xffffff)),
    element("Question", Group::Callouts, Kind::Disc("?", 0x0a84ff, 0xffffff)),
    element("Warning", Group::Callouts, Kind::Disc("!", 0xf59e0b, 0x111111)),
    element("Lower Third", Group::Titles, Kind::LowerThird),
    element("Title Card", Group::Titles, Kind::TitleCard("Your Title", "A short line under it")),
    element("Caption", Group::Titles, Kind::Caption("Add your caption here")),
    element("Quote", Group::Titles, Kind::Words("“Say something memorable.”", 64.0, 0xffffff)),
    element("Chapter", Group::Titles, Kind::Pill("Chapter 1", 0xffffff, 0x111111)),
    element("Handle", Group::Titles, Kind::Words("@yourname", 60.0, 0xffffff)),
    element("Hashtag", Group::Titles, Kind::Words("#trending", 60.0, 0xffffff)),
    element("Website", Group::Titles, Kind::Pill("www.yoursite.com", 0x111111, 0xffffff)),
    element("Price", Group::Titles, Kind::Words("$29.99", 110.0, 0xffffff)),
    element("Countdown", Group::Titles, Kind::Words("3", 300.0, 0xffffff)),
    element("New Burst", Group::Badges, Kind::Burst("NEW")),
    element("Free Burst", Group::Badges, Kind::Burst("FREE")),
    element("Sale Seal", Group::Badges, Kind::Seal("SALE")),
    element("Half Off Seal", Group::Badges, Kind::Seal("50%")),
    element("Number One", Group::Badges, Kind::Seal("#1")),
    element("Price Tag", Group::Badges, Kind::Tag("$29")),
    element("Ribbon", Group::Badges, Kind::Ribbon("LIMITED")),
    element("Best Seller", Group::Badges, Kind::Ribbon("BEST SELLER")),
    element("Coming Soon", Group::Badges, Kind::Ribbon("COMING SOON")),
    element("Label", Group::Badges, Kind::Pill("Link in bio", 0x111111, 0xffffff)),
    element("New Label", Group::Badges, Kind::Pill("NEW", 0x22c55e, 0xffffff)),
    element("Live Badge", Group::Badges, Kind::Pill("● LIVE", 0xef4444, 0xffffff)),
    element("Tip Label", Group::Badges, Kind::Pill("TIP", 0xfacc15, 0x111111)),
    element("Verified", Group::Badges, Kind::Disc("✓", 0x0a84ff, 0xffffff)),
    element("Five Stars", Group::Badges, Kind::Words("★★★★★", 90.0, 0xfacc15)),
    element("Speech Bubble", Group::Badges, Kind::Speech("Hi there!")),
    element("Photo Frame", Group::Frames, Kind::PhotoFrame),
    element("Square Frame", Group::Frames, Kind::Slot(ShapeKind::Rectangle, (400.0, 400.0), 12.0)),
    element("Wide Frame", Group::Frames, Kind::Slot(ShapeKind::Rectangle, (640.0, 360.0), 12.0)),
    element("Story Frame", Group::Frames, Kind::Slot(ShapeKind::Rectangle, (360.0, 640.0), 36.0)),
    element("Circle Frame", Group::Frames, Kind::Slot(ShapeKind::Ellipse, (360.0, 360.0), 0.0)),
    element("Polaroid", Group::Frames, Kind::Polaroid),
    element("Split Screen", Group::Frames, Kind::SplitScreen),
    element("Phone Frame", Group::Frames, Kind::PhoneFrame),
    element("Browser Window", Group::Frames, Kind::BrowserWindow),
    element("Cinematic Bars", Group::Frames, Kind::CinemaBars),
    element("Subscribe", Group::Social, Kind::Pill("SUBSCRIBE", 0xef4444, 0xffffff)),
    element("Follow", Group::Social, Kind::Pill("+ Follow", 0x0a84ff, 0xffffff)),
    element("Shop Now", Group::Social, Kind::Pill("Shop Now →", 0x111111, 0xffffff)),
    element("Watch Now", Group::Social, Kind::Pill("▶ Watch now", 0xffffff, 0x111111)),
    element("Learn More", Group::Social, Kind::Pill("Learn more", ACCENT, 0xffffff)),
    element("Join Now", Group::Social, Kind::Pill("Join now", 0x22c55e, 0xffffff)),
    element("Comment Below", Group::Social, Kind::Pill("Comment below ↓", 0x111111, 0xffffff)),
    element("Share", Group::Social, Kind::Pill("Share ↗", 0x0a84ff, 0xffffff)),
    element("Link in Description", Group::Social, Kind::Pill("Link in description", 0xffffff, 0x111111)),
    element("Swipe Up", Group::Social, Kind::Pill("Swipe up ↑", 0xffffff, 0x111111)),
    element("Like", Group::Social, Kind::Like),
    element("Like Count", Group::Social, Kind::Words("♥ 1.2K", 80.0, 0xef4444)),
    element("Notification", Group::Social, Kind::Disc("1", 0xef4444, 0xffffff)),
    element("Play Button", Group::Social, Kind::PlayButton),
    element("Progress Bar", Group::Social, Kind::ProgressBar),
];

/// Every element, in panel order: the drawn ones, then the emoji stickers.
pub static ELEMENTS: std::sync::LazyLock<Vec<Element>> = std::sync::LazyLock::new(|| {
    let stickers = EMOJI.iter().map(|&(name, emoji)| element(name, Group::Stickers, Kind::Emoji(emoji)));
    DRAWN.iter().copied().chain(stickers).collect()
});

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
        Kind::Sided(corners) => {
            let id = shape(project, ShapeKind::Polygon, Vec2::new(320.0, 320.0), center, solid(ACCENT), None, 0.0);
            if let Some(id) = id {
                crate::edit::edit_shape(project.composition_mut(comp).expect("added to it"), id, |style| style.points = corners);
            }
            id.into_iter().collect()
        }
        Kind::Block(kind, corner, (width, height), color) => shape(project, kind, Vec2::new(width, height), center, solid(color), None, corner).into_iter().collect(),
        Kind::Outline(kind, corner, (width, height), color, line) => {
            shape(project, kind, Vec2::new(width, height), center, None, Some(Stroke { paint: Paint::Solid(Rgba::hex(color)), width: line }), corner).into_iter().collect()
        }
        Kind::Cutout(corners, (width, height), color) => {
            path_layer(project, comp, &polygon(corners), center, Vec2::new(width, height), ShapeStyle { fill: solid(color), ..ShapeStyle::new(ShapeKind::Path) }, frame)
        }
        Kind::Disc(words, fill, ink) => with_words(project, comp, frame, center, words, 130.0, ink, |project| shape(project, ShapeKind::Ellipse, Vec2::new(220.0, 220.0), center, solid(fill), None, 0.0)),
        Kind::Words(words, size, ink) => text_layer(project, comp, frame, center, words, "Avenir Next", size, ink).into_iter().collect(),
        Kind::Caption(words) => {
            let width = words.chars().count() as f32 * 27.0 + 80.0;
            with_words(project, comp, frame, center, words, 48.0, 0xffffff, |project| shape(project, ShapeKind::Rectangle, Vec2::new(width, 96.0), center, solid(0x111111), None, 14.0))
        }
        Kind::LowerThird => {
            // A dark plate with an accent bar at its left; the name and role start beside the bar.
            let plate = shape(project, ShapeKind::Rectangle, Vec2::new(640.0, 150.0), center, solid(0x111111), None, 12.0);
            let bar = shape(project, ShapeKind::Rectangle, Vec2::new(12.0, 102.0), center + Vec2::new(-290.0, 0.0), solid(ACCENT), None, 6.0);
            let left = center.x - 262.0;
            let name_line = text_from_left(project, comp, frame, Vec2::new(left, center.y - 24.0), "Your Name", 56.0, 0xffffff);
            let role_line = text_from_left(project, comp, frame, Vec2::new(left, center.y + 34.0), "Title or role", 34.0, 0xa1a1aa);
            group_under(project, comp, frame, plate, [bar, name_line, role_line])
        }
        Kind::TitleCard(title, under) => {
            let title_line = text_layer(project, comp, frame, center + Vec2::new(0.0, -40.0), title, "Avenir Next", 120.0, 0xffffff);
            let under_line = text_layer(project, comp, frame, center + Vec2::new(0.0, 70.0), under, "Avenir Next", 44.0, 0xd4d4d8);
            group_under(project, comp, frame, title_line, [under_line])
        }
        Kind::Slot(kind, (width, height), corner) => {
            let slot = shape(project, kind, Vec2::new(width, height), center, Some(photo_fill()), Some(Stroke { paint: Paint::Solid(Rgba::hex(0xffffff)), width: 14.0 }), corner);
            if let Some(id) = slot {
                name(project, comp, id, "Drop your photo here");
            }
            slot.into_iter().collect()
        }
        Kind::Polaroid => {
            // A white card with the photo toward its top and room for a caption under it.
            let card = shape(project, ShapeKind::Rectangle, Vec2::new(420.0, 500.0), center, solid(0xffffff), None, 8.0);
            let photo = shape(project, ShapeKind::Rectangle, Vec2::new(380.0, 380.0), center + Vec2::new(0.0, -40.0), Some(photo_fill()), None, 4.0);
            let caption = text_layer(project, comp, frame, center + Vec2::new(0.0, 198.0), "Caption", "Avenir Next", 40.0, 0x111111);
            if let Some(photo) = photo {
                name(project, comp, photo, "Drop your photo here");
            }
            group_under(project, comp, frame, card, [photo, caption])
        }
        Kind::BrowserWindow => {
            // A light window with three colored dots in its title bar and the page under it.
            let window = shape(project, ShapeKind::Rectangle, Vec2::new(760.0, 500.0), center, solid(0xe5e7eb), None, 18.0);
            let page = shape(project, ShapeKind::Rectangle, Vec2::new(736.0, 432.0), center + Vec2::new(0.0, 22.0), Some(photo_fill()), None, 8.0);
            if let Some(page) = page {
                name(project, comp, page, "Drop your screen recording here");
            }
            let dots = [0xef4444, 0xf59e0b, 0x22c55e].into_iter().enumerate().map(|(index, color)| {
                shape(project, ShapeKind::Ellipse, Vec2::new(16.0, 16.0), center + Vec2::new(-352.0 + index as f32 * 26.0, -224.0), solid(color), None, 0.0)
            });
            let dots: Vec<Option<Id>> = dots.collect();
            group_under(project, comp, frame, window, std::iter::once(page).chain(dots))
        }
        Kind::SplitScreen => {
            // Two picture slots side by side that move as one.
            let slot = |project: &mut Project, offset: f32| {
                let id = shape(project, ShapeKind::Rectangle, Vec2::new(340.0, 480.0), center + Vec2::new(offset, 0.0), Some(photo_fill()), Some(Stroke { paint: Paint::Solid(Rgba::hex(0xffffff)), width: 10.0 }), 12.0);
                if let Some(id) = id {
                    name(project, comp, id, "Drop your photo here");
                }
                id
            };
            let left = slot(project, -180.0);
            let right = slot(project, 180.0);
            group_under(project, comp, frame, left, [right])
        }
        Kind::CinemaBars => {
            // Bars across the whole frame, each an eighth of its height, whatever its size.
            let (width, height) = project.composition(comp).map_or((1920.0, 1080.0), |composition| (composition.width as f32, composition.height as f32));
            let bar = height / 8.0;
            let middle = Vec2::new(width / 2.0, height / 2.0);
            let top = shape(project, ShapeKind::Rectangle, Vec2::new(width, bar), Vec2::new(middle.x, bar / 2.0), solid(0x000000), None, 0.0);
            let bottom = shape(project, ShapeKind::Rectangle, Vec2::new(width, bar), Vec2::new(middle.x, height - bar / 2.0), solid(0x000000), None, 0.0);
            group_under(project, comp, frame, top, [bottom])
        }
        Kind::PlayButton => {
            let disc = shape(project, ShapeKind::Ellipse, Vec2::new(240.0, 240.0), center, solid(0xffffff), None, 0.0);
            let arrow = path_layer(project, comp, &polygon(PLAY), center + Vec2::new(8.0, 0.0), Vec2::new(90.0, 100.0), ShapeStyle { fill: solid(0x111111), ..ShapeStyle::new(ShapeKind::Path) }, frame).first().copied();
            group_under(project, comp, frame, disc, [arrow])
        }
        Kind::ProgressBar => {
            // A track with the part played so far filled in from its left end.
            let track = shape(project, ShapeKind::Rectangle, Vec2::new(640.0, 24.0), center, solid(0x3f3f46), None, 12.0);
            let played = shape(project, ShapeKind::Rectangle, Vec2::new(384.0, 24.0), center + Vec2::new(-128.0, 0.0), solid(ACCENT), None, 12.0);
            group_under(project, comp, frame, track, [played])
        }
        Kind::Heart => path_layer(project, comp, &heart(), center, Vec2::new(300.0, 300.0), ShapeStyle { fill: solid(0xef4444), ..ShapeStyle::new(ShapeKind::Path) }, frame),
        Kind::Line => shape(project, ShapeKind::Rectangle, Vec2::new(480.0, 10.0), center, solid(0xffffff), None, 5.0).into_iter().collect(),
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
            let photo = shape(project, ShapeKind::Rectangle, Vec2::new(520.0, 380.0), center, Some(Paint::linear(Rgba::hex(0x3f3f46), Rgba::hex(0x27272a), 60.0)), None, 6.0);
            parent_pair(project, comp, frame, frame_id, photo, "Drop your photo here")
        }
        Kind::PhoneFrame => {
            let body = shape(project, ShapeKind::Rectangle, Vec2::new(360.0, 740.0), center, solid(0x18181b), None, 56.0);
            let screen = shape(project, ShapeKind::Rectangle, Vec2::new(330.0, 710.0), center, Some(Paint::linear(Rgba::hex(0x3f3f46), Rgba::hex(0x27272a), 60.0)), None, 42.0);
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

/// The grey gradient shown where a photo or video is to be dropped.
fn photo_fill() -> Paint {
    Paint::linear(Rgba::hex(0x3f3f46), Rgba::hex(0x27272a), 60.0)
}

/// A line of text whose left edge starts at `left_middle`, vertically centered on it.
fn text_from_left(project: &mut Project, comp: Id, frame: i64, left_middle: Vec2, words: &str, size: f32, ink: u32) -> Option<Id> {
    let style = TextStyle { font_family: "Avenir Next".into(), font_weight: 800, font_size: size, line_height: size * 1.15, ..TextStyle::default() };
    let width = style.estimate_size(words).x;
    text_layer(project, comp, frame, left_middle + Vec2::new(width / 2.0, 0.0), words, "Avenir Next", size, ink)
}

/// Parents each of `parts` that was made to `main`, so the element moves as one. Returns the
/// layers made, `main` first. Nothing if `main` could not be made.
fn group_under(project: &mut Project, comp: Id, frame: i64, main: Option<Id>, parts: impl IntoIterator<Item = Option<Id>>) -> Vec<Id> {
    let Some(main) = main else { return Vec::new() };
    let parts: Vec<Id> = parts.into_iter().flatten().collect();
    if let Some(composition) = project.composition_mut(comp) {
        for part in &parts {
            crate::edit::set_parent(composition, *part, Some(main), frame);
        }
    }
    std::iter::once(main).chain(parts).collect()
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
    ShapePath { points: corners.iter().map(|&(x, y)| PathPoint::corner(Vec2::new(x, y))).collect(), closed: true, ..Default::default() }
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
        ..Default::default()
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
        ..Default::default()
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
        // Tiles are told apart by name, so no two in a group share one.
        for (index, element) in ELEMENTS.iter().enumerate() {
            let twin = ELEMENTS[..index].iter().find(|other| other.group == element.group && other.name == element.name);
            assert!(twin.is_none(), "two elements called {} in {}", element.name, element.group.name());
        }
    }

    #[test]
    fn the_parts_of_a_built_element_move_with_its_main_layer() {
        for name in ["Lower Third", "Polaroid", "Browser Window", "Play Button", "Progress Bar", "Cinematic Bars"] {
            let mut project = sample_project();
            let index = ELEMENTS.iter().position(|element| element.name == name).unwrap();
            let added = add(&mut project, ids::MAIN_COMP, index, Vec2::new(960.0, 540.0), 0);
            assert!(added.len() > 1, "{name} is made of several layers");
            let composition = project.composition(ids::MAIN_COMP).unwrap();
            for part in &added[1..] {
                assert_eq!(composition.layer(*part).unwrap().parent, Some(added[0]), "{name}");
            }
        }
    }

    #[test]
    fn cinematic_bars_span_the_frame_at_its_top_and_bottom() {
        let mut project = sample_project();
        let index = ELEMENTS.iter().position(|element| element.name == "Cinematic Bars").unwrap();
        let added = add(&mut project, ids::MAIN_COMP, index, Vec2::new(100.0, 100.0), 0);
        let composition = project.composition(ids::MAIN_COMP).unwrap();
        let (width, height) = (composition.width as f32, composition.height as f32);
        for bar in &added {
            assert_eq!(composition.layer(*bar).unwrap().size, Vec2::new(width, height / 8.0));
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
