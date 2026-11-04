//! The project data model: project items, compositions, layers and their properties.

use crate::gradient::Gradient;
use crate::animation::{Animated, Vec2};
use crate::time::FrameRate;

/// The furthest a frame number can be from frame 0: about 1,400 years at 24 fps. Frames stay
/// well inside this, so time arithmetic (spans, shifts, sums) can never overflow; project
/// files with frames beyond it are refused.
pub const MAX_FRAME: i64 = 1 << 40;

/// Identifies any object in a project. Ids are unique within a project and never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Id(pub u64);

/// An item in a library (a brand, color or text style); see `crate::library`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct LinkId(pub u64);

/// A color with straight (non-premultiplied) alpha, each channel 0.0–1.0.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
    /// The library color this one comes from. A linked color follows its library color when
    /// that changes (see `library::sync_project`); picking any other color unlinks it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkId>,
}

impl Rgba {
    /// Builds an opaque color from 0xRRGGBB.
    pub const fn hex(rgb: u32) -> Self {
        Self {
            r: ((rgb >> 16) & 0xff) as f32 / 255.0,
            g: ((rgb >> 8) & 0xff) as f32 / 255.0,
            b: (rgb & 0xff) as f32 / 255.0,
            a: 1.0,
            link: None,
        }
    }

    /// No color at all: a composition background that lets everything below show through.
    pub const TRANSPARENT: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0, link: None };

    /// The same color with its opacity set (0–1).
    pub fn with_alpha(self, alpha: f32) -> Self {
        Self { a: if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 1.0 }, ..self }
    }

    /// The same color, linked to library color `link` (or unlinked, with `None`).
    pub fn linked(self, link: Option<LinkId>) -> Self {
        Self { link, ..self }
    }

    /// Whether two colors look the same (ignoring links), to the nearest 8-bit step.
    pub fn same_look(self, other: Self) -> bool {
        self.to_rgba_u32() == other.to_rgba_u32()
    }

    /// Packs the color as 0xRRGGBBAA, the format GPUI's `rgba()` expects.
    pub fn to_rgba_u32(self) -> u32 {
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
        (channel(self.r) << 24) | (channel(self.g) << 16) | (channel(self.b) << 8) | channel(self.a)
    }
}

impl Rgba {
    /// Hue (0–360°), saturation and value (0–1), for the color wheel.
    pub fn to_hsv(self) -> (f32, f32, f32) {
        let (r, g, b) = (self.r.clamp(0.0, 1.0), self.g.clamp(0.0, 1.0), self.b.clamp(0.0, 1.0));
        let max = r.max(g).max(b);
        let delta = max - r.min(g).min(b);
        let hue = if delta <= f32::EPSILON {
            0.0
        } else if max == r {
            60.0 * ((g - b) / delta).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };
        let saturation = if max <= f32::EPSILON { 0.0 } else { delta / max };
        (hue, saturation, max)
    }

    /// An opaque color from hue (degrees), saturation and value (0–1).
    pub fn from_hsv(hue: f32, saturation: f32, value: f32) -> Self {
        let (s, v) = (saturation.clamp(0.0, 1.0), value.clamp(0.0, 1.0));
        let h = hue.rem_euclid(360.0) / 60.0;
        let chroma = v * s;
        let x = chroma * (1.0 - (h % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (chroma, x, 0.0),
            1 => (x, chroma, 0.0),
            2 => (0.0, chroma, x),
            3 => (0.0, x, chroma),
            4 => (x, 0.0, chroma),
            _ => (chroma, 0.0, x),
        };
        let m = v - chroma;
        Self { r: r + m, g: g + m, b: b + m, a: 1.0, link: None }
    }

    /// The color as "#RRGGBB".
    pub fn to_hex(self) -> String {
        format!("#{:06X}", self.to_rgba_u32() >> 8)
    }

    /// Reads "#RGB" or "#RRGGBB" (the "#" is optional).
    pub fn parse_hex(text: &str) -> Option<Self> {
        let digits = text.trim().trim_start_matches('#');
        if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let value = u32::from_str_radix(digits, 16).ok()?;
        match digits.len() {
            6 => Some(Self::hex(value)),
            3 => {
                let expand = |nibble: u32| (nibble & 0xf) * 0x11;
                Some(Self::hex((expand(value >> 8) << 16) | (expand(value >> 4) << 8) | expand(value)))
            }
            _ => None,
        }
    }
}

/// How an area is filled: one color, or a gradient over the layer's box (see `gradient`).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(from = "PaintFile")]
pub enum Paint {
    Solid(Rgba),
    /// Color stops along the line from the gradient's `from` to its `to`.
    Linear(Gradient),
    /// Color stops out from the gradient's `from` (the center) to its `to` (the edge).
    Radial(Gradient),
}

/// How `Paint` is read from a file: shape fills were saved as a plain color at first, and
/// gradients as two colors with an angle before they had stops.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum PaintFile {
    Color(Rgba),
    Paint(PaintTagged),
}

#[derive(serde::Deserialize)]
enum PaintTagged {
    Solid(Rgba),
    Linear(LinearFile),
    Radial(RadialFile),
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum LinearFile {
    Stops(Box<Gradient>),
    TwoColors { start: Rgba, end: Rgba, angle: f32 },
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum RadialFile {
    Stops(Box<Gradient>),
    TwoColors { inner: Rgba, outer: Rgba },
}

impl From<PaintFile> for Paint {
    fn from(file: PaintFile) -> Self {
        match file {
            PaintFile::Color(color) | PaintFile::Paint(PaintTagged::Solid(color)) => Self::Solid(color),
            PaintFile::Paint(PaintTagged::Linear(LinearFile::Stops(gradient))) => Self::Linear(*gradient),
            PaintFile::Paint(PaintTagged::Linear(LinearFile::TwoColors { start, end, angle })) => Self::linear(start, end, angle),
            PaintFile::Paint(PaintTagged::Radial(RadialFile::Stops(gradient))) => Self::Radial(*gradient),
            PaintFile::Paint(PaintTagged::Radial(RadialFile::TwoColors { inner, outer })) => Self::radial(inner, outer),
        }
    }
}

/// Which kind of paint, for the fill type menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintKind {
    Solid,
    Linear,
    Radial,
}

impl PaintKind {
    pub const ALL: [PaintKind; 3] = [Self::Solid, Self::Linear, Self::Radial];

    pub fn name(self) -> &'static str {
        match self {
            Self::Solid => "Solid Color",
            Self::Linear => "Linear Gradient",
            Self::Radial => "Radial Gradient",
        }
    }
}

impl Paint {
    /// A two-color linear gradient across the box at `angle` degrees (0 left to right).
    pub fn linear(start: Rgba, end: Rgba, angle: f32) -> Self {
        Self::Linear(Gradient::linear_at_angle(start, end, angle))
    }

    /// A two-color radial gradient from the middle of the box out to its sides.
    pub fn radial(inner: Rgba, outer: Rgba) -> Self {
        Self::Radial(Gradient::two(inner, outer, crate::gradient::RADIAL_FROM, crate::gradient::RADIAL_TO))
    }

    pub fn kind(self) -> PaintKind {
        match self {
            Self::Solid(_) => PaintKind::Solid,
            Self::Linear(_) => PaintKind::Linear,
            Self::Radial(_) => PaintKind::Radial,
        }
    }

    /// The gradient, for the gradient kinds.
    pub fn gradient(self) -> Option<Gradient> {
        match self {
            Self::Solid(_) => None,
            Self::Linear(gradient) | Self::Radial(gradient) => Some(gradient),
        }
    }

    /// The same kind of paint with its gradient replaced (a solid stays as it is).
    pub fn with_gradient(self, gradient: Gradient) -> Self {
        match self {
            Self::Solid(_) => self,
            Self::Linear(_) => Self::Linear(gradient),
            Self::Radial(_) => Self::Radial(gradient),
        }
    }

    /// The colors it is made of: one for a solid, each stop's for a gradient.
    pub fn colors(self) -> Vec<Rgba> {
        match self {
            Self::Solid(color) => vec![color],
            Self::Linear(gradient) | Self::Radial(gradient) => gradient.stops().iter().map(|stop| stop.color).collect(),
        }
    }

    /// Every color in it, to change in place.
    pub fn colors_mut(&mut self) -> Vec<&mut Rgba> {
        match self {
            Self::Solid(color) => vec![color],
            Self::Linear(gradient) | Self::Radial(gradient) => gradient.colors_mut().collect(),
        }
    }

    /// The same paint with color number `index` (see `colors`) replaced.
    pub fn with_color(self, index: usize, color: Rgba) -> Self {
        match self {
            Self::Solid(_) => Self::Solid(color),
            Self::Linear(gradient) => Self::Linear(gradient.with_color(index, color)),
            Self::Radial(gradient) => Self::Radial(gradient.with_color(index, color)),
        }
    }

    /// Changes to another kind of paint, keeping its colors: a solid becomes a gradient from its
    /// color to black, a gradient becomes a solid of its first color, and linear and radial
    /// swap keeping their stops (starting from the new kind's usual placement).
    pub fn into_kind(self, kind: PaintKind) -> Self {
        use crate::gradient::{LINEAR_FROM, LINEAR_TO, RADIAL_FROM, RADIAL_TO};
        let stops = match self {
            Self::Solid(color) => Gradient::two(color, Rgba::hex(0x000000), LINEAR_FROM, LINEAR_TO),
            Self::Linear(gradient) | Self::Radial(gradient) => gradient,
        };
        match (self, kind) {
            (_, PaintKind::Solid) => Self::Solid(self.colors()[0]),
            (Self::Linear(_), PaintKind::Linear) | (Self::Radial(_), PaintKind::Radial) => self,
            (_, PaintKind::Linear) => Self::Linear(Gradient::new(stops.stops(), LINEAR_FROM, LINEAR_TO)),
            (_, PaintKind::Radial) => Self::Radial(Gradient::new(stops.stops(), RADIAL_FROM, RADIAL_TO)),
        }
    }
}

/// An outline drawn along a shape's edge, centered on it as in After Effects.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Stroke {
    pub paint: Paint,
    /// In layer pixels.
    pub width: f32,
}

/// The widest stroke allowed, in layer pixels.
pub const MAX_STROKE_WIDTH: f32 = 1000.0;

/// Color tags for layers and project items, using the After Effects default label palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LabelColor {
    None,
    Red,
    Yellow,
    Aqua,
    Pink,
    Lavender,
    Peach,
    Seafoam,
    Blue,
    Green,
    Purple,
    Orange,
}

impl LabelColor {
    /// Every label, in the order After Effects lists them.
    pub const ALL: [Self; 12] = [
        Self::None,
        Self::Red,
        Self::Yellow,
        Self::Aqua,
        Self::Pink,
        Self::Lavender,
        Self::Peach,
        Self::Seafoam,
        Self::Blue,
        Self::Green,
        Self::Purple,
        Self::Orange,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Red => "Red",
            Self::Yellow => "Yellow",
            Self::Aqua => "Aqua",
            Self::Pink => "Pink",
            Self::Lavender => "Lavender",
            Self::Peach => "Peach",
            Self::Seafoam => "Seafoam",
            Self::Blue => "Blue",
            Self::Green => "Green",
            Self::Purple => "Purple",
            Self::Orange => "Orange",
        }
    }

    pub const fn color(self) -> Rgba {
        Rgba::hex(match self {
            Self::None => 0x5a5a5a,
            Self::Red => 0xb53838,
            Self::Yellow => 0xe4d84c,
            Self::Aqua => 0xa9cbc7,
            Self::Pink => 0xe5bcc9,
            Self::Lavender => 0xa9a9ca,
            Self::Peach => 0xe7c19e,
            Self::Seafoam => 0xb3c7b3,
            Self::Blue => 0x677de0,
            Self::Green => 0x4aa44c,
            Self::Purple => 0x8e2c9a,
            Self::Orange => 0xe8920d,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BlendMode {
    Normal,
    Add,
    Multiply,
    Screen,
    Overlay,
    SoftLight,
    HardLight,
    Difference,
}

impl BlendMode {
    pub const ALL: [Self; 8] = [
        Self::Normal,
        Self::Add,
        Self::Multiply,
        Self::Screen,
        Self::Overlay,
        Self::SoftLight,
        Self::HardLight,
        Self::Difference,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Add => "Add",
            Self::Multiply => "Multiply",
            Self::Screen => "Screen",
            Self::Overlay => "Overlay",
            Self::SoftLight => "Soft Light",
            Self::HardLight => "Hard Light",
            Self::Difference => "Difference",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TrackMatte {
    None,
    Alpha,
    AlphaInverted,
    Luma,
    LumaInverted,
}

impl TrackMatte {
    pub const ALL: [Self; 5] = [Self::None, Self::Alpha, Self::AlphaInverted, Self::Luma, Self::LumaInverted];

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "No Matte",
            Self::Alpha => "Alpha",
            Self::AlphaInverted => "Alpha Inverted",
            Self::Luma => "Luma",
            Self::LumaInverted => "Luma Inverted",
        }
    }
}

/// On/off switches shown in the timeline's switch columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LayerSwitches {
    pub visible: bool,
    pub audio: bool,
    pub solo: bool,
    pub locked: bool,
    pub shy: bool,
    pub best_quality: bool,
    pub effects_enabled: bool,
    pub frame_blending: bool,
    pub motion_blur: bool,
    pub adjustment: bool,
}

impl Default for LayerSwitches {
    fn default() -> Self {
        Self {
            visible: true,
            audio: false,
            solo: false,
            locked: false,
            shy: false,
            best_quality: true,
            effects_enabled: true,
            frame_blending: false,
            motion_blur: false,
            adjustment: false,
        }
    }
}

/// A layer's 2D transform. Positions are in composition pixels, scale in percent,
/// rotation in degrees (clockwise) and opacity in percent.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Transform {
    pub anchor_point: Animated<Vec2>,
    pub position: Animated<Vec2>,
    pub scale: Animated<Vec2>,
    pub rotation: Animated<f32>,
    pub opacity: Animated<f32>,
}

impl Transform {
    /// A transform that places a source of `size` centered at `position` at full size.
    pub fn centered(size: Vec2, position: Vec2) -> Self {
        Self {
            anchor_point: Animated::constant(size * 0.5),
            position: Animated::constant(position),
            scale: Animated::constant(Vec2::new(100.0, 100.0)),
            rotation: Animated::constant(0.0),
            opacity: Animated::constant(100.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// Font sizes allowed, in pixels.
pub const FONT_SIZE_RANGE: (f32, f32) = (1.0, 2000.0);
/// Line heights allowed, in pixels.
pub const LINE_HEIGHT_RANGE: (f32, f32) = (1.0, 4000.0);
/// Letter spacing allowed, in thousandths of the font size.
pub const TRACKING_RANGE: (f32, f32) = (-1000.0, 10000.0);
/// The largest a layer's box may be on either side, in pixels.
pub const MAX_LAYER_SIDE: f32 = 100_000.0;

/// Character and paragraph styling for a text layer.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TextStyle {
    pub font_family: String,
    pub font_weight: u16,
    pub font_size: f32,
    pub line_height: f32,
    /// Extra spacing between characters, in thousandths of an em, as in After Effects.
    pub tracking: f32,
    pub fill: Option<Rgba>,
    pub stroke: Option<(Rgba, f32)>,
    pub align: TextAlign,
    /// The library text style this follows (font, weight, size, leading, tracking and fill).
    /// Changing one of those by hand unlinks it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<LinkId>,
}

impl TextStyle {
    /// Approximate pixel size of `text` in this style, used for layer bounds until real
    /// text shaping is available. Assumes an average glyph width of 0.58 em.
    /// Whether the style's numbers are ones the text renderer can lay out: finite and within
    /// the ranges the editor allows (a line height of 0, say, would stop it).
    pub fn is_sound(&self) -> bool {
        let within = |value: f32, (low, high): (f32, f32)| value.is_finite() && (low..=high).contains(&value);
        within(self.font_size, FONT_SIZE_RANGE)
            && within(self.line_height, LINE_HEIGHT_RANGE)
            && within(self.tracking, TRACKING_RANGE)
            && self.stroke.is_none_or(|(_, width)| width.is_finite() && (0.0..=MAX_STROKE_WIDTH).contains(&width))
    }

    pub fn estimate_size(&self, text: &str) -> Vec2 {
        let lines: Vec<&str> = if text.is_empty() { vec![""] } else { text.lines().collect() };
        let tracking_px = self.tracking / 1000.0 * self.font_size;
        let widest = lines
            .iter()
            .map(|line| {
                let glyphs = line.chars().count() as f32;
                glyphs * self.font_size * 0.58 + (glyphs - 1.0).max(0.0) * tracking_px
            })
            .fold(0.0_f32, f32::max);
        Vec2::new(widest.max(self.font_size * 0.5), lines.len() as f32 * self.line_height)
    }
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_family: "Inter".into(),
            font_weight: 700,
            font_size: 120.0,
            line_height: 144.0,
            tracking: 0.0,
            fill: Some(Rgba::hex(0xffffff)),
            stroke: None,
            align: TextAlign::Center,
            template: None,
        }
    }
}

/// The outline of a shape layer. Each fills the layer's box (`Layer::size`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
    Triangle,
    /// A regular polygon with `ShapeStyle::points` sides.
    Polygon,
    /// A star with `ShapeStyle::points` points.
    Star,
    /// A custom outline drawn with the Pen tool (the layer's `ShapePath`).
    Path,
}

impl ShapeKind {
    /// The shapes the Shape tool draws (a path is drawn with the Pen tool instead).
    pub const ALL: [ShapeKind; 5] = [Self::Rectangle, Self::Ellipse, Self::Triangle, Self::Polygon, Self::Star];

    pub fn name(self) -> &'static str {
        match self {
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Triangle => "Triangle",
            Self::Polygon => "Polygon",
            Self::Star => "Star",
            Self::Path => "Path",
        }
    }

    /// Whether the shape has corners that can be rounded (the drawn shapes except the ellipse;
    /// a pen path's corners are its own points).
    pub fn has_corners(self) -> bool {
        !matches!(self, Self::Ellipse | Self::Path)
    }

    /// Whether the number of sides or points can be changed.
    pub fn has_points(self) -> bool {
        matches!(self, Self::Polygon | Self::Star)
    }
}

/// How a shape layer is drawn.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShapeStyle {
    pub kind: ShapeKind,
    /// `None` draws no fill (an outline only, with a stroke).
    pub fill: Option<Paint>,
    #[serde(default)]
    pub stroke: Option<Stroke>,
    /// Radius of the rounded corners, in layer pixels. Ignored for ellipses.
    pub corner_radius: f32,
    /// Sides of a polygon, or points of a star (3 to `MAX_SHAPE_POINTS`).
    pub points: u32,
    /// A star's inner radius as a fraction of its outer radius (After Effects' Inner Radius).
    #[serde(default = "default_inner_radius")]
    pub inner_radius: f32,
}

fn default_inner_radius() -> f32 {
    0.5
}

/// The range a star's inner radius (a fraction of the outer one) is kept in.
pub const INNER_RADIUS_RANGE: (f32, f32) = (0.05, 1.0);

/// The most sides or points a polygon or star can have.
pub const MAX_SHAPE_POINTS: u32 = 64;

impl ShapeStyle {
    /// A shape of `kind` in the default fill, with square corners.
    pub fn new(kind: ShapeKind) -> Self {
        let points = if kind == ShapeKind::Star { 5 } else { 6 };
        Self { kind, fill: Some(Paint::Solid(Rgba::hex(0x8b5cf6))), stroke: None, corner_radius: 0.0, points, inner_radius: default_inner_radius() }
    }

    /// The largest corner radius that makes sense in a box of `size`: half its shorter side.
    pub fn max_corner_radius(size: Vec2) -> f32 {
        size.x.min(size.y) * 0.5
    }
}

/// What a layer shows.
// A shape's style is the largest by far (its fill and stroke can each hold a gradient of up to
// eight stops), but layers are few and boxing it would add a step to every shape read.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum LayerContent {
    Footage { item: Id },
    Image { item: Id },
    Audio { item: Id },
    Solid { color: Rgba },
    Text { text: String, style: TextStyle },
    /// A drawn shape. `path` holds the outline of a `ShapeKind::Path` shape and is `None` for
    /// the others.
    Shape {
        style: ShapeStyle,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<crate::path::ShapePath>,
    },
    /// Timed words, shown a phrase at a time in one style (see `captions`).
    Captions { words: Vec<crate::captions::CaptionWord>, style: crate::captions::CaptionStyle },
    Adjustment,
    Composition { composition: Id },
}

impl LayerContent {
    /// Whether the layer draws anything in the viewer.
    pub fn has_picture(&self) -> bool {
        !matches!(self, Self::Audio { .. })
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EffectProperty {
    pub name: String,
    /// Display and range information. Not saved: it comes from the effect catalog, so project
    /// files pick up catalog improvements (see `project_file`).
    #[serde(skip, default = "crate::property::PropertyInfo::unknown")]
    pub info: crate::property::PropertyInfo,
    pub value: Animated<f32>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Effect {
    pub id: Id,
    pub name: String,
    pub enabled: bool,
    pub properties: Vec<EffectProperty>,
    /// The lookup table an Apply LUT effect uses; `None` for every other effect, and for an
    /// Apply LUT whose table has not been chosen yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lut: Option<LutSource>,
}

/// Where a LUT comes from: one of Studio's built-in looks (see `grade::LOOK_GROUPS`), or a `.cube` file.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum LutSource {
    Look(String),
    File(std::path::PathBuf),
}

impl LutSource {
    /// The name shown for it: the look's, or the file's without its extension.
    pub fn name(&self) -> String {
        match self {
            Self::Look(name) => name.clone(),
            Self::File(path) => path.file_stem().map_or_else(|| path.display().to_string(), |stem| stem.to_string_lossy().into_owned()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Layer {
    pub id: Id,
    pub name: String,
    pub label: LabelColor,
    pub content: LayerContent,
    /// Size of the layer's source in pixels, before transforms.
    pub size: Vec2,
    /// First visible frame, in composition time.
    pub in_frame: i64,
    /// Frame after the last visible frame (exclusive).
    pub out_frame: i64,
    /// The composition frame at which the source's first frame plays. It equals `in_frame`
    /// until the layer is trimmed or slipped; only time-based sources (footage, audio) use it.
    /// Files saved before trimming existed leave it out; it is then set to `in_frame` on loading.
    #[serde(default = "Layer::unset_start")]
    pub start_frame: i64,
    /// Which track the layer is on (see `tracks`): 0 is the main track, picture tracks count
    /// up from it and sound-only tracks down. Older files leave it out; on loading, each
    /// layer then gets its own track in its stacking order.
    #[serde(default = "Layer::unset_track")]
    pub track: i32,
    pub blend_mode: BlendMode,
    pub track_matte: TrackMatte,
    /// Another layer in the same composition whose shape shows or hides this one. The mask
    /// layer itself is not drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<LayerMask>,
    /// Background removal, for footage and image layers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cutout: Option<Cutout>,
    /// For a text layer: the pen path its letters follow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_path: Option<TextPath>,
    pub parent: Option<Id>,
    pub switches: LayerSwitches,
    /// `None` for layers without a picture, such as audio.
    pub transform: Option<Transform>,
    pub effects: Vec<Effect>,
    pub has_audio: bool,
    /// Volume and fades, for layers with sound.
    #[serde(default)]
    pub audio: LayerAudio,
    /// How fast (and which way) footage plays.
    #[serde(default, skip_serializing_if = "LayerTime::is_normal")]
    pub time: LayerTime,
    /// The one-click animations chosen for the layer (see `presets`).
    #[serde(default, skip_serializing_if = "crate::presets::LayerAnimation::is_none")]
    pub animation: crate::presets::LayerAnimation,
}

/// How fast a footage layer's source plays, and whether backwards.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LayerTime {
    /// Speed as a percentage (100 is normal, 50 is slow motion, 200 is double). Keyframes
    /// make a speed ramp.
    pub speed: Animated<f32>,
    /// Plays the same part of the source backwards.
    #[serde(default)]
    pub reverse: bool,
    /// While a ramp is on, the constant speed the clip had before it (a percentage), so
    /// removing the ramp puts the clip back exactly as it was. `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_before_ramp: Option<f32>,
}

impl Default for LayerTime {
    fn default() -> Self {
        Self { speed: Animated::constant(100.0), reverse: false, speed_before_ramp: None }
    }
}

impl LayerTime {
    /// The slowest and fastest speeds offered, as percentages.
    pub const SPEED_RANGE: (f32, f32) = (5.0, 2000.0);

    pub fn is_normal(&self) -> bool {
        *self == Self::default()
    }

    /// A speed as a fraction (1 is normal), kept in range.
    fn factor(percent: f32) -> f64 {
        let percent = if percent.is_finite() { percent.clamp(Self::SPEED_RANGE.0, Self::SPEED_RANGE.1) } else { 100.0 };
        f64::from(percent) / 100.0
    }

    /// The constant speed as a fraction, or `None` while it ramps.
    pub fn constant_factor(&self) -> Option<f64> {
        (!self.speed.is_animated()).then(|| Self::factor(self.speed.value))
    }

    /// Seconds of source played from `from` to `to` (composition frames), at `fps`: the
    /// speed summed frame by frame, so ramps play the right amount.
    pub(crate) fn source_seconds_between(&self, from: i64, to: i64, fps: f64) -> f64 {
        if let Some(factor) = self.constant_factor() {
            return (to - from) as f64 / fps * factor;
        }
        let (low, high, sign) = if to >= from { (from, to, 1.0) } else { (to, from, -1.0) };
        let total: f64 = (low..high).map(|frame| Self::factor(self.speed.value_at(frame as f64 + 0.5))).sum();
        sign * total / fps
    }
}

/// Text set along a pen path: each letter sits on the path, turned to follow it. The path is
/// another layer's (a pen shape in the same composition), where it is on screen; the text
/// layer's own position and rotation are not used, but its scale sets the letters' size.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TextPath {
    pub layer: Id,
    /// How far along the path the text starts, in composition pixels.
    pub start: f32,
}

/// A layer used as a mask: where it draws, the masked layer is shown (or hidden).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LayerMask {
    pub layer: Id,
    pub mode: MaskMode,
}

/// Whether a mask shows the layer inside its shape or hides it there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MaskMode {
    /// The layer shows only where the mask draws.
    #[default]
    Show,
    /// The layer is cut away where the mask draws.
    Hide,
}

impl MaskMode {
    pub const ALL: [Self; 2] = [Self::Show, Self::Hide];

    pub fn name(self) -> &'static str {
        match self {
            Self::Show => "Show Inside",
            Self::Hide => "Hide Inside",
        }
    }
}

/// Background removal on a footage or image layer: clicks on one frame pick out a subject, and
/// masks made from them (by the `studio-cutout` model, followed through the clip for video)
/// cut it out of everything around it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Cutout {
    /// The source frame the clicks were placed on (0 for an image).
    pub frame: i64,
    pub clicks: Vec<CutoutClick>,
    /// Remove what was picked (an object to take out) instead of keeping it (a subject to
    /// take out of its background).
    #[serde(default)]
    pub remove: bool,
    /// The folder holding the masks, one grayscale PNG per source frame (`<frame>.png`), in
    /// Studio's cache. `None` until masks are made.
    #[serde(default)]
    pub masks: Option<std::path::PathBuf>,
    /// The source frames that have masks made from the current clicks (start, end exclusive).
    /// Frames outside it use the nearest mask made.
    #[serde(default)]
    pub frames: Option<(i64, i64)>,
}

/// A click picking out a cut-out's subject: where on the source picture (fractions of its width
/// and height) and whether it marks something to keep or something to leave out.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CutoutClick {
    pub at: Vec2,
    pub keep: bool,
}

/// The most clicks a cut-out can have.
pub const MAX_CUTOUT_CLICKS: usize = 64;

impl Cutout {
    /// The mask file for source frame `index`: the nearest frame that has one, or `None` when no
    /// masks have been made yet.
    pub fn mask_path(&self, index: i64) -> Option<std::path::PathBuf> {
        let (folder, (start, end)) = (self.masks.as_ref()?, self.frames?);
        let index = index.clamp(start, (end - 1).max(start));
        Some(folder.join(format!("{index}.png")))
    }
}

/// A layer's sound settings.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LayerAudio {
    /// Volume in decibels: 0 leaves it as it is; -6 is about half as loud.
    pub volume_db: f32,
    /// Seconds over which the sound rises from silence at the layer's in point.
    pub fade_in: f32,
    /// Seconds over which it falls to silence before the out point.
    pub fade_out: f32,
    /// Another layer whose sound this one gets quieter under (music under a voice), and by
    /// how many decibels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duck_under: Option<Id>,
    #[serde(default = "LayerAudio::default_duck_db")]
    pub duck_db: f32,
    /// How strongly quiet background sound (hiss, hum, room noise) is turned down between
    /// louder sound, 0 (off) to 100.
    #[serde(default)]
    pub noise_gate: f32,
    /// Enhance Speech: cleaning up a voice recording (see `SpeechEnhance`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enhance: Option<SpeechEnhance>,
}

impl Default for LayerAudio {
    fn default() -> Self {
        Self { volume_db: 0.0, fade_in: 0.0, fade_out: 0.0, duck_under: None, duck_db: Self::default_duck_db(), noise_gate: 0.0, enhance: None }
    }
}

/// Which model Enhance Speech removes background noise with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EnhanceModel {
    /// RNNoise: very quick, good on steady noise (fans, hiss).
    Light,
    /// DeepFilterNet3: the best result, on any background.
    #[default]
    Studio,
    /// Keep the background; only the voice EQ, de-essing and levelling run.
    Off,
}

/// Enhance Speech, like Adobe Podcast's: background noise removed, hum notched out, the voice
/// EQ'd, de-essed and brought to podcast loudness. The app renders a cleaned copy of the clip's
/// whole sound once (`file`) and plays that instead of the original, which stays untouched.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpeechEnhance {
    /// Whether the cleaned sound plays (off compares with the original, without losing it).
    pub on: bool,
    pub model: EnhanceModel,
    /// How much of the cleaned sound is used, 0 to 100; below 100 some of the original is
    /// mixed back in, for a more natural sound.
    pub strength: f32,
    pub remove_hum: bool,
    pub voice_eq: bool,
    pub de_ess: bool,
    /// Compress and bring to -16 LUFS.
    pub even_out: bool,
    /// The cleaned copy for these settings, once it has been made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<std::path::PathBuf>,
}

impl Default for SpeechEnhance {
    fn default() -> Self {
        Self { on: true, model: EnhanceModel::Studio, strength: 90.0, remove_hum: true, voice_eq: true, de_ess: true, even_out: true, file: None }
    }
}

impl SpeechEnhance {
    /// Whether `other` asks for the same sound (whether either is on, and which file holds it,
    /// don't matter).
    pub fn same_sound(&self, other: &SpeechEnhance) -> bool {
        let settings = |enhance: &SpeechEnhance| SpeechEnhance { on: true, file: None, ..enhance.clone() };
        settings(self) == settings(other)
    }
}

impl LayerAudio {
    /// The volume range offered, in decibels.
    pub const VOLUME_RANGE: (f32, f32) = (-60.0, 12.0);
    /// How far ducking can lower the sound, in decibels.
    pub const DUCK_RANGE: (f32, f32) = (0.0, 40.0);

    fn default_duck_db() -> f32 {
        14.0
    }

    /// The volume as a factor to multiply samples by.
    pub fn gain(&self) -> f32 {
        let db = if self.volume_db.is_finite() { self.volume_db.clamp(Self::VOLUME_RANGE.0, Self::VOLUME_RANGE.1) } else { 0.0 };
        if db <= Self::VOLUME_RANGE.0 { 0.0 } else { 10f32.powf(db / 20.0) }
    }
}

impl Layer {
    /// Marks a start frame missing from an older project file (see `start_frame`).
    pub(crate) const UNSET_START: i64 = i64::MIN;

    fn unset_start() -> i64 {
        Self::UNSET_START
    }

    fn unset_track() -> i32 {
        crate::tracks::UNSET_TRACK
    }

    /// Whether the layer is present at `frame`.
    pub fn is_active_at(&self, frame: i64) -> bool {
        (self.in_frame..self.out_frame).contains(&frame)
    }

    /// Which frame of a time-based source (`source_rate` frames a second) plays at composition
    /// frame `frame`: time since the source's start (which trimming and slipping move),
    /// converted to the source's rate.
    pub fn source_frame(&self, frame: i64, comp_rate: FrameRate, source_rate: FrameRate) -> i64 {
        (self.source_seconds(frame, comp_rate) * source_rate.fps() + 1e-6).floor() as i64
    }

    /// Seconds into a time-based source at composition frame `frame`, following the layer's
    /// speed (and ramps) from where the source starts. Reversed, the layer plays the same part
    /// of the source backwards: its in point shows what its last frame would have.
    pub fn source_seconds(&self, frame: i64, comp_rate: FrameRate) -> f64 {
        let fps = comp_rate.fps();
        let forward = |frame: i64| self.time.source_seconds_between(self.start_frame, frame, fps);
        if self.time.reverse {
            forward(self.in_frame) + forward(self.out_frame - 1) - forward(frame)
        } else {
            forward(frame)
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Composition {
    pub id: Id,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub duration_frames: i64,
    pub background: Rgba,
    /// Stacking order: index 0 is the top layer.
    pub layers: Vec<Layer>,
    /// Points marked on the timeline (such as the beats of a song), in frame order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<Marker>,
}

/// A point marked on a composition's timeline. Moves and trims snap to markers.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Marker {
    pub frame: i64,
    /// What it marks ("Beat", say); shown on hover.
    pub label: String,
}

impl Composition {
    pub fn layer(&self, id: Id) -> Option<&Layer> {
        self.layers.iter().find(|layer| layer.id == id)
    }

    pub fn layer_mut(&mut self, id: Id) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|layer| layer.id == id)
    }

    pub fn size(&self) -> Vec2 {
        Vec2::new(self.width as f32, self.height as f32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ProjectItemKind {
    Folder,
    Composition,
    Footage,
    Image,
    Solid,
    Audio,
}

impl ProjectItemKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Folder => "Folder",
            Self::Composition => "Composition",
            Self::Footage => "Footage",
            Self::Image => "Image",
            Self::Solid => "Solid",
            Self::Audio => "Audio",
        }
    }
}

/// An entry in the Project panel: imported media, a solid, a composition or a folder.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProjectItem {
    pub id: Id,
    pub name: String,
    pub kind: ProjectItemKind,
    pub label: LabelColor,
    pub parent: Option<Id>,
    /// Pixel size for items with a picture.
    pub size: Option<Vec2>,
    /// Length of time-based media. Stills have no intrinsic duration.
    pub duration_seconds: Option<f64>,
    pub frame_rate: Option<FrameRate>,
    pub has_audio: bool,
    /// Fill color for solids.
    pub color: Option<Rgba>,
    /// Where imported media lives on disk. `None` for solids, compositions and folders.
    pub path: Option<std::path::PathBuf>,
}

impl ProjectItem {
    /// One-line summary for the Project panel header, e.g. "1920 × 1080 · 0:00:10:00 · 24 fps".
    pub fn details(&self) -> String {
        let mut parts = Vec::new();
        if let Some(size) = self.size {
            parts.push(format!("{} × {}", size.x, size.y));
        }
        if let Some(seconds) = self.duration_seconds {
            let rate = self.frame_rate.unwrap_or(FrameRate::new(24, 1));
            parts.push(crate::time::format_timecode(rate.seconds_to_frames(seconds), rate));
        }
        if let Some(rate) = self.frame_rate {
            parts.push(rate.label());
        }
        parts.join(" · ")
    }
}

/// A whole project: the Project panel's items and the compositions they describe.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Project {
    pub items: Vec<ProjectItem>,
    pub compositions: Vec<Composition>,
    next_id: u64,
}

impl Project {
    pub fn new(items: Vec<ProjectItem>, compositions: Vec<Composition>) -> Self {
        let next_id = Self::highest_id(&items, &compositions).saturating_add(1);
        Self { items, compositions, next_id }
    }

    /// The highest id used by any item, composition, layer or effect.
    fn highest_id(items: &[ProjectItem], compositions: &[Composition]) -> u64 {
        let layers = compositions.iter().flat_map(|comp| &comp.layers);
        let layer_ids = layers.clone().map(|layer| layer.id.0);
        let effect_ids = layers.flat_map(|layer| &layer.effects).map(|effect| effect.id.0);
        let ids = items.iter().map(|item| item.id.0).chain(compositions.iter().map(|comp| comp.id.0));
        ids.chain(layer_ids).chain(effect_ids).max().unwrap_or(0)
    }

    /// Makes sure new ids start above every id in use, for example after loading a file
    /// whose saved counter is stale or was edited by hand.
    pub fn refresh_next_id(&mut self) {
        self.next_id = self.next_id.max(Self::highest_id(&self.items, &self.compositions).saturating_add(1));
    }

    /// Returns a fresh id that no object in this project uses.
    pub fn allocate_id(&mut self) -> Id {
        let id = Id(self.next_id);
        self.next_id += 1;
        id
    }

    /// Calls `visit` on every color in the project: solid items, composition backgrounds, and
    /// the colors of solid, text and shape layers.
    pub fn for_each_color_mut(&mut self, mut visit: impl FnMut(&mut Rgba)) {
        for color in self.items.iter_mut().filter_map(|item| item.color.as_mut()) {
            visit(color);
        }
        for composition in &mut self.compositions {
            visit(&mut composition.background);
            for layer in &mut composition.layers {
                match &mut layer.content {
                    LayerContent::Solid { color } => visit(color),
                    LayerContent::Text { style, .. } => {
                        style.fill.iter_mut().for_each(&mut visit);
                        style.stroke.iter_mut().for_each(|(color, _)| visit(color));
                    }
                    LayerContent::Captions { style, .. } => {
                        style.text.fill.iter_mut().for_each(&mut visit);
                        style.text.stroke.iter_mut().for_each(|(color, _)| visit(color));
                        visit(&mut style.highlight);
                        style.background.iter_mut().for_each(&mut visit);
                    }
                    LayerContent::Shape { style, .. } => {
                        let paints = style.fill.iter_mut().chain(style.stroke.iter_mut().map(|stroke| &mut stroke.paint));
                        for paint in paints {
                            paint.colors_mut().into_iter().for_each(&mut visit);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn item(&self, id: Id) -> Option<&ProjectItem> {
        self.items.iter().find(|item| item.id == id)
    }

    pub fn composition(&self, id: Id) -> Option<&Composition> {
        self.compositions.iter().find(|comp| comp.id == id)
    }

    pub fn composition_mut(&mut self, id: Id) -> Option<&mut Composition> {
        self.compositions.iter_mut().find(|comp| comp.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_round_trip_through_hsv_and_hex() {
        for hex in [0xff0000, 0x00ff00, 0x0000ff, 0x8b5cf6, 0x808080, 0x000000, 0xffffff, 0x123456] {
            let color = Rgba::hex(hex);
            let (h, s, v) = color.to_hsv();
            assert_eq!(Rgba::from_hsv(h, s, v).to_rgba_u32(), color.to_rgba_u32(), "{hex:06x}");
            assert_eq!(Rgba::parse_hex(&color.to_hex()), Some(color));
        }
        assert_eq!(Rgba::parse_hex("#f0a").map(|c| c.to_hex()), Some("#FF00AA".to_owned()));
        assert_eq!(Rgba::parse_hex("+12345"), None);
        assert_eq!(Rgba::parse_hex("#12345"), None);
    }

    #[test]
    fn paints_change_kind_keeping_their_colors() {
        let red = Rgba::hex(0xff0000);
        let gradient = Paint::Solid(red).into_kind(PaintKind::Linear);
        assert_eq!(gradient.colors(), vec![red, Rgba::hex(0x000000)]);
        let blue = Rgba::hex(0x0000ff);
        assert_eq!(gradient.with_color(1, blue).into_kind(PaintKind::Radial), Paint::radial(red, blue));
        assert_eq!(Paint::radial(red, blue).into_kind(PaintKind::Solid), Paint::Solid(red));
    }

    #[test]
    fn gradients_saved_with_two_colors_and_an_angle_still_load() {
        let red = Rgba::hex(0xff0000);
        let blue = Rgba::hex(0x0000ff);
        let old = r#"{"Linear":{"start":{"r":1.0,"g":0.0,"b":0.0,"a":1.0},"end":{"r":0.0,"g":0.0,"b":1.0,"a":1.0},"angle":90.0}}"#;
        assert_eq!(serde_json::from_str::<Paint>(old).unwrap(), Paint::linear(red, blue, 90.0));
        let old = r#"{"Radial":{"inner":{"r":1.0,"g":0.0,"b":0.0,"a":1.0},"outer":{"r":0.0,"g":0.0,"b":1.0,"a":1.0}}}"#;
        assert_eq!(serde_json::from_str::<Paint>(old).unwrap(), Paint::radial(red, blue));
        // And a gradient with stops saves and loads as itself.
        let mut gradient = Paint::linear(red, blue, 0.0).gradient().unwrap();
        gradient.add_stop(0.3);
        let paint = Paint::Linear(gradient);
        assert_eq!(serde_json::from_str::<Paint>(&serde_json::to_string(&paint).unwrap()).unwrap(), paint);
    }

    #[test]
    fn colors_round_trip_through_hex() {
        assert_eq!(Rgba::hex(0x2d8ceb).to_rgba_u32(), 0x2d8cebff);
    }

    #[test]
    fn new_ids_never_collide_with_existing_ones() {
        let mut project = crate::sample::sample_project();
        let existing: Vec<u64> = project.items.iter().map(|item| item.id.0).collect();
        let fresh = project.allocate_id();
        assert!(!existing.contains(&fresh.0));
        assert_ne!(project.allocate_id(), fresh);
    }

    #[test]
    fn layers_are_active_between_in_and_out() {
        let comp = &crate::sample::sample_project().compositions[0];
        let title = &comp.layers[0];
        assert!(!title.is_active_at(title.in_frame - 1));
        assert!(title.is_active_at(title.in_frame));
        assert!(!title.is_active_at(title.out_frame));
    }
}
