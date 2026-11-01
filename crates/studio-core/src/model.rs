//! The project data model: project items, compositions, layers and their properties.

use crate::animation::{Animated, Vec2};
use crate::time::FrameRate;

/// The furthest a frame number can be from frame 0: about 1,400 years at 24 fps. Frames stay
/// well inside this, so time arithmetic (spans, shifts, sums) can never overflow; project
/// files with frames beyond it are refused.
pub const MAX_FRAME: i64 = 1 << 40;

/// Identifies any object in a project. Ids are unique within a project and never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Id(pub u64);

/// A color with straight (non-premultiplied) alpha, each channel 0.0–1.0.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    /// Builds an opaque color from 0xRRGGBB.
    pub const fn hex(rgb: u32) -> Self {
        Self {
            r: ((rgb >> 16) & 0xff) as f32 / 255.0,
            g: ((rgb >> 8) & 0xff) as f32 / 255.0,
            b: (rgb & 0xff) as f32 / 255.0,
            a: 1.0,
        }
    }

    /// Packs the color as 0xRRGGBBAA, the format GPUI's `rgba()` expects.
    pub fn to_rgba_u32(self) -> u32 {
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
        (channel(self.r) << 24) | (channel(self.g) << 16) | (channel(self.b) << 8) | channel(self.a)
    }
}

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
}

impl TextStyle {
    /// Approximate pixel size of `text` in this style, used for layer bounds until real
    /// text shaping is available. Assumes an average glyph width of 0.58 em.
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
}

impl ShapeKind {
    pub const ALL: [ShapeKind; 5] = [Self::Rectangle, Self::Ellipse, Self::Triangle, Self::Polygon, Self::Star];

    pub fn name(self) -> &'static str {
        match self {
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Triangle => "Triangle",
            Self::Polygon => "Polygon",
            Self::Star => "Star",
        }
    }

    /// Whether the shape has corners that can be rounded (everything except the ellipse).
    pub fn has_corners(self) -> bool {
        self != Self::Ellipse
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
    pub fill: Rgba,
    /// Radius of the rounded corners, in layer pixels. Ignored for ellipses.
    pub corner_radius: f32,
    /// Sides of a polygon, or points of a star (3 to `MAX_SHAPE_POINTS`).
    pub points: u32,
}

/// The most sides or points a polygon or star can have.
pub const MAX_SHAPE_POINTS: u32 = 64;

impl ShapeStyle {
    /// A shape of `kind` in the default fill, with square corners.
    pub fn new(kind: ShapeKind) -> Self {
        let points = if kind == ShapeKind::Star { 5 } else { 6 };
        Self { kind, fill: Rgba::hex(0x8b5cf6), corner_radius: 0.0, points }
    }

    /// The largest corner radius that makes sense in a box of `size`: half its shorter side.
    pub fn max_corner_radius(size: Vec2) -> f32 {
        size.x.min(size.y) * 0.5
    }
}

/// What a layer shows.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum LayerContent {
    Footage { item: Id },
    Image { item: Id },
    Audio { item: Id },
    Solid { color: Rgba },
    Text { text: String, style: TextStyle },
    Shape { style: ShapeStyle },
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
    pub parent: Option<Id>,
    pub switches: LayerSwitches,
    /// `None` for layers without a picture, such as audio.
    pub transform: Option<Transform>,
    pub effects: Vec<Effect>,
    pub has_audio: bool,
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
