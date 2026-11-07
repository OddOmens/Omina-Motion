//! Exports a Studio composition as a vector animation: Lottie (the Bodymovin JSON format that
//! web and app players read), dotLottie (the same JSON zipped with a manifest), or an animated
//! SVG (CSS keyframes) that plays in any browser.
//!
//! Both formats are written from one intermediate picture of the composition (`scene`): its
//! layers in drawing order, every animated value as keyframes with easings the formats share,
//! and every shape as Bézier geometry. Whatever Studio has that the formats can't hold (video,
//! most effects, captions) is left out and listed in `Exported::left_out`, so the person
//! exporting knows what to expect.
//!
//! Nothing here reads files or fonts. Image bytes and text outlines come from a `Resources`
//! implementation passed in by the caller (the app uses real fonts and image files; the tests
//! use stand-ins), so every export is a pure function of the project and is easy to test.

mod encode;
mod geometry;
pub mod lottie;
mod notes;
mod scene;
pub mod svg;

use std::ops::Range;

use studio_core::animation::Vec2;
use studio_core::model::{Id, ProjectItem, TextStyle};
use studio_core::path::ShapePath;
use studio_core::time::FrameRate;

pub use lottie::{dotlottie, lottie};
pub use svg::svg;

/// What to export: which composition, which of its frames, and at what frame rate.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorOptions {
    pub composition: Id,
    /// Composition frames to export (the work area, say). The animation starts at the first.
    pub frames: Range<i64>,
    /// The animation's frame rate, or `None` for the composition's own.
    pub frame_rate: Option<FrameRate>,
}

/// A finished export: the file's contents, and a plain-English line for each kind of thing
/// that was left out or simplified on the way (empty when everything made it across).
#[derive(Clone, Debug, PartialEq)]
pub struct Exported<T> {
    pub file: T,
    pub left_out: Vec<String>,
}

/// A picture file to embed, as the bytes of a PNG or JPEG.
#[derive(Clone, Debug, PartialEq)]
pub struct EmbeddedImage {
    /// "image/png" or "image/jpeg".
    pub mime: &'static str,
    pub bytes: Vec<u8>,
    /// The picture's size in pixels.
    pub width: u32,
    pub height: u32,
}

/// A text layer's letters as outlines, laid out exactly as Studio draws them. Coordinates are
/// in the text's own pixels: (0, 0) is the top-left of its box, y grows downwards.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextOutline {
    /// The text box the layout fills (Studio stretches it to the layer's box).
    pub size: Vec2,
    /// Each line's top and height.
    pub lines: Vec<(f32, f32)>,
    pub glyphs: Vec<GlyphOutline>,
}

/// One letter's outline and where it sits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphOutline {
    /// Byte offset in the text of the character it draws.
    pub start: usize,
    /// Which line it is on (an index into `TextOutline::lines`).
    pub line: usize,
    /// Its left edge and the width it takes on the line.
    pub left: f32,
    pub width: f32,
    /// Closed contours, in the text's pixels (not fractions of a box, unlike a pen path).
    pub contours: Vec<ShapePath>,
}

/// Where an export gets what isn't in the project itself.
pub trait Resources {
    /// The picture of an image item, ready to embed, or `None` if it can't be read.
    fn image(&mut self, item: &ProjectItem) -> Option<EmbeddedImage>;
    /// `text` in `style` as outlines, or `None` when fonts aren't available. Without outlines
    /// text is written as live text, which shows in whatever font the player has.
    fn text(&mut self, text: &str, style: &TextStyle) -> Option<TextOutline>;
}

/// Resources for an export with no fonts and no image files: text stays live text and image
/// layers are left out. Used by tests and anywhere fonts can't be loaded.
pub struct NoResources;

impl Resources for NoResources {
    fn image(&mut self, _item: &ProjectItem) -> Option<EmbeddedImage> {
        None
    }

    fn text(&mut self, _text: &str, _style: &TextStyle) -> Option<TextOutline> {
        None
    }
}
