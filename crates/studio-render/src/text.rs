//! Text layers: shaping and layout with cosmic-text (real fonts, weight, size, leading,
//! tracking and alignment), rasterized on the CPU into a straight-alpha sRGB image.
//! The stroke follows the letters' own outlines (with its position, corners and dashes);
//! color glyphs (emoji), which have none, get an even outline from a distance map instead.

use std::collections::HashMap;

use cosmic_text::{Attrs, Buffer, Command, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Weight};
use studio_core::animation::Vec2;
use studio_core::model::{Rgba, StrokeCap, StrokeJoin, StrokePosition, TextAlign, TextStrokeOptions, TextStyle};
use studio_core::path::{PathPoint, ShapePath};
use tiny_skia::{FillRule, PathBuilder, Pixmap, Transform};

/// A rasterized text layer, ready to upload as a texture.
pub struct TextImage {
    pub width: u32,
    pub height: u32,
    /// Straight (not premultiplied) sRGB RGBA, row by row.
    pub pixels: Vec<u8>,
    /// The text box in the layer's source pixels (before raster scaling).
    pub size: Vec2,
    /// The scale the raster was actually drawn at: the one asked for, unless that would have
    /// made it too big (over 8× or larger than `max_size` pixels).
    pub scale: f32,
}

/// Where a text layer's characters sit, for drawing a caret and selection over it and for
/// finding the character under a click (see `TextRenderer::geometry`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextGeometry {
    pub lines: Vec<TextLine>,
}

/// One line of text: its top and height, and the caret position between its characters, as
/// (byte offset in the whole text, x).
#[derive(Clone, Debug, PartialEq)]
pub struct TextLine {
    pub top: f32,
    pub height: f32,
    pub stops: Vec<(usize, f32)>,
}

impl TextGeometry {
    /// The caret before byte `offset`: its x, and the top and height of its line.
    pub fn caret(&self, offset: usize) -> (f32, f32, f32) {
        // The last line whose first stop is at or before the offset holds it.
        let line = self.lines.iter().rev().find(|line| line.stops.first().is_some_and(|&(at, _)| at <= offset)).unwrap_or(&self.lines[0]);
        let x = line.stops.iter().rev().find(|&&(at, _)| at <= offset).or(line.stops.first()).map_or(0.0, |&(_, x)| x);
        (x, line.top, line.height)
    }

    /// The rectangles a selection of bytes `range` covers, one per line: (left, top, width,
    /// height).
    pub fn highlight(&self, range: std::ops::Range<usize>) -> Vec<(f32, f32, f32, f32)> {
        self.lines
            .iter()
            .filter_map(|line| {
                let inside: Vec<f32> = line.stops.iter().filter(|&&(at, _)| at >= range.start && at <= range.end).map(|&(_, x)| x).collect();
                let (left, right) = inside.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
                (inside.len() >= 2 && right > left).then_some((left, line.top, right - left, line.height))
            })
            .collect()
    }

    /// The byte offset of the caret stop nearest `(x, y)`: on the line at that height (the
    /// first or last line when above or below the text).
    pub fn hit(&self, x: f32, y: f32) -> usize {
        let line = self.lines.iter().find(|line| y < line.top + line.height).unwrap_or_else(|| self.lines.last().expect("there is always a line"));
        line.stops.iter().min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs())).map_or(0, |&(at, _)| at)
    }
}

/// A text layer's letters as outlines (see `TextRenderer::outlines`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextOutlines {
    /// The text box, as `measure` gives it.
    pub size: Vec2,
    /// Each line's top and height.
    pub lines: Vec<(f32, f32)>,
    pub glyphs: Vec<GlyphPath>,
}

/// One letter: the byte offset in the text of the character it draws, its line, its left edge
/// and the width it takes, and its outline as closed contours in the text's pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphPath {
    pub start: usize,
    pub line: usize,
    pub left: f32,
    pub width: f32,
    pub contours: Vec<ShapePath>,
}

/// Whether the face at `index` of a font file is a variable font whose weight axis reaches
/// `weight`, judged as cosmic-text judges it when choosing a face. (Some older Apple fonts
/// measure that axis on a scale of their own, so their range rarely covers 100 to 900.)
fn weight_axis_covers(data: &[u8], index: u32, weight: u16) -> bool {
    use cosmic_text::skrifa::{MetadataProvider, Tag};
    let Ok(font) = cosmic_text::skrifa::FontRef::from_index(data, index) else { return false };
    font.axes().get_by_tag(Tag::new(b"wght")).is_some_and(|axis| (axis.min_value()..=axis.max_value()).contains(&f32::from(weight)))
}

/// How many characters the face at `index` of a font file has shapes for.
fn characters_covered(data: &[u8], index: u32) -> usize {
    use cosmic_text::skrifa::MetadataProvider;
    cosmic_text::skrifa::FontRef::from_index(data, index).map_or(0, |font| font.charmap().mappings().count())
}

/// Loads the installed fonts, leaving out faces that would hide a family's real face. Some
/// families install an ornament face under the same name, weight and style as their regular
/// face (Hoefler Text Ornaments, for one). A layout can only ever pick one face for a given
/// family, weight and style, and if it picks the ornaments, ordinary text comes out as
/// ornaments and the system font. So of faces that are asked for the same way, only the one
/// covering the most characters is kept. (Omina Edit loads its fonts the same way.)
fn load_fonts() -> FontSystem {
    let mut fonts = FontSystem::new();
    let db = fonts.db_mut();
    // Faces grouped by how a layout asks for them: family, weight, style and width.
    let mut alike: HashMap<(String, u16, String, u16), Vec<cosmic_text::fontdb::ID>> = HashMap::new();
    for face in db.faces() {
        let Some((family, _)) = face.families.first() else { continue };
        alike.entry((family.clone(), face.weight.0, format!("{:?}", face.style), face.stretch.to_number())).or_default().push(face.id);
    }
    for ids in alike.into_values().filter(|ids| ids.len() > 1) {
        let coverage: Vec<usize> = ids.iter().map(|id| db.with_face_data(*id, characters_covered).unwrap_or(0)).collect();
        let fullest = (0..ids.len()).max_by_key(|at| coverage[*at]).unwrap_or(0);
        for (at, id) in ids.iter().enumerate() {
            if at != fullest {
                db.remove_face(*id);
            }
        }
    }
    fonts
}

/// A glyph's outline commands (y up, from its origin) as closed contours in the text's pixels
/// (y down), placed at `origin`. Quadratic curves become the equivalent cubic ones.
fn contours(commands: &[Command], origin: Vec2) -> Vec<ShapePath> {
    let place = |x: f32, y: f32| Vec2::new(origin.x + x, origin.y - y);
    let mut contours: Vec<ShapePath> = Vec::new();
    let mut current: Vec<PathPoint> = Vec::new();
    fn finish(points: &mut Vec<PathPoint>, contours: &mut Vec<ShapePath>) {
        // A contour that ends back on its first point joins it there.
        if points.len() > 2 && (points[0].at - points[points.len() - 1].at).length() < 1e-3 {
            let last = points.pop().expect("more than two points");
            points[0].handle_in = last.handle_in;
        }
        if points.len() > 1 {
            contours.push(ShapePath { points: std::mem::take(points), closed: true, ..Default::default() });
        }
        points.clear();
    }
    for command in commands {
        match *command {
            Command::MoveTo(to) => {
                finish(&mut current, &mut contours);
                current.push(PathPoint::corner(place(to.x, to.y)));
            }
            Command::LineTo(to) => current.push(PathPoint::corner(place(to.x, to.y))),
            Command::CurveTo(a, b, to) => {
                let end = place(to.x, to.y);
                if let Some(last) = current.last_mut() {
                    last.handle_out = place(a.x, a.y) - last.at;
                }
                current.push(PathPoint { at: end, handle_in: place(b.x, b.y) - end, handle_out: Vec2::default() });
            }
            Command::QuadTo(control, to) => {
                let (control, end) = (place(control.x, control.y), place(to.x, to.y));
                if let Some(last) = current.last_mut() {
                    last.handle_out = (control - last.at) * (2.0 / 3.0);
                }
                current.push(PathPoint { at: end, handle_in: (control - end) * (2.0 / 3.0), handle_out: Vec2::default() });
            }
            Command::Close => finish(&mut current, &mut contours),
        }
    }
    finish(&mut current, &mut contours);
    contours
}

pub struct TextRenderer {
    fonts: FontSystem,
    glyphs: SwashCache,
    /// The weight each (family, asked-for weight) is drawn at, so fonts are only looked through
    /// once for each.
    drawn_weights: HashMap<(String, u16), u16>,
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextRenderer {
    /// Loads the system's fonts. This takes a moment, so it is done once.
    pub fn new() -> Self {
        Self { fonts: load_fonts(), glyphs: SwashCache::new(), drawn_weights: HashMap::new() }
    }

    /// The weights (100 to 900) the installed family `family` comes in, lightest first. A
    /// variable font reports only the weights of its named faces. Empty if the family isn't
    /// installed.
    pub fn weights(&self, family: &str) -> Vec<u16> {
        let mut weights: Vec<u16> = self.fonts.db().faces().filter(|face| face.families.iter().any(|(name, _)| name == family)).map(|face| face.weight.0).collect();
        weights.sort_unstable();
        weights.dedup();
        weights
    }

    /// The weight `family` is drawn at when `wanted` is asked for: `wanted` itself when the family
    /// has it (or is a variable font whose weight axis reaches it), otherwise the
    /// family's nearest weight, the heavier one on a tie. Without this, asking for a weight a
    /// family doesn't come in (bold, in a font made only in regular) would make the layout pass
    /// the family over and draw the text in the system font instead.
    pub fn nearest_weight(&mut self, family: &str, wanted: u16) -> u16 {
        let wanted = wanted.clamp(100, 900);
        let key = (family.to_owned(), wanted);
        if let Some(weight) = self.drawn_weights.get(&key) {
            return *weight;
        }
        let db = self.fonts.db();
        let faces: Vec<_> = db.faces().filter(|face| face.families.iter().any(|(name, _)| name == family)).collect();
        let weight = if faces.is_empty() || faces.iter().any(|face| face.weight.0 == wanted) || faces.iter().any(|face| db.with_face_data(face.id, |data, index| weight_axis_covers(data, index, wanted)) == Some(true)) {
            wanted
        } else {
            faces.iter().map(|face| face.weight.0).min_by_key(|weight| (weight.abs_diff(wanted), std::cmp::Reverse(*weight))).unwrap_or(wanted)
        };
        self.drawn_weights.insert(key, weight);
        weight
    }

    /// Shapes `text` at `scale` × its font size. Returns the buffer and its box (unscaled).
    fn layout(&mut self, text: &str, style: &TextStyle, scale: f32) -> (Buffer, Vec2) {
        // Kept in the editor's ranges even for a style that slipped past them: a line height of 0
        // stops the layout, and a negative one never finishes.
        let (font_size, line_height) = (style.font_size.clamp(1.0, 2000.0), style.line_height.clamp(1.0, 4000.0));
        let metrics = Metrics::new(font_size * scale, line_height * scale);
        let weight = self.nearest_weight(&style.font_family, style.font_weight);
        let mut buffer = Buffer::new(&mut self.fonts, metrics);
        let attrs = Attrs::new()
            .family(Family::Name(&style.font_family))
            .weight(Weight(weight))
            // Tracking is in thousandths of an em; cosmic-text letter spacing is in ems.
            .letter_spacing(style.tracking / 1000.0);
        let align = match style.align {
            TextAlign::Left => cosmic_text::Align::Left,
            TextAlign::Center => cosmic_text::Align::Center,
            TextAlign::Right => cosmic_text::Align::Right,
        };
        let shown = if text.is_empty() { " " } else { text };
        {
            let mut borrowed = buffer.borrow_with(&mut self.fonts);
            borrowed.set_size(None, None);
            borrowed.set_text(shown, &attrs, Shaping::Advanced, Some(align));
            borrowed.shape_until_scroll(false);
        }
        let mut width = 0.0_f32;
        let mut lines = 0;
        for run in buffer.layout_runs() {
            width = width.max(run.line_w);
            lines += 1;
        }
        // Alignment needs a box as wide as the widest line; lay out again with that width.
        buffer.borrow_with(&mut self.fonts).set_size(Some(width.max(1.0)), None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let height = lines.max(1) as f32 * style.line_height * scale;
        (buffer, Vec2::new(width.max(1.0) / scale, height / scale))
    }

    /// The text box size in source pixels, used to keep layer bounds accurate.
    pub fn measure(&mut self, text: &str, style: &TextStyle) -> Vec2 {
        self.layout(text, style, 1.0).1
    }

    /// Where each letter of the text's first line sits, in source pixels: its left edge and its
    /// width (the space it takes, so the letters tile the line), and how far down the line its
    /// baseline is. For setting text along a path.
    pub fn letters(&mut self, text: &str, style: &TextStyle) -> (Vec<(f32, f32)>, f32) {
        let (buffer, _) = self.layout(text, style, 1.0);
        let Some(run) = buffer.layout_runs().next() else { return (Vec::new(), 0.0) };
        (run.glyphs.iter().map(|glyph| (glyph.x, glyph.w)).collect(), run.line_y)
    }

    /// The text's letters as outlines, laid out exactly as `rasterize` draws them, for exporting
    /// text as vector shapes. Positions are in source pixels in the text's own box.
    pub fn outlines(&mut self, text: &str, style: &TextStyle) -> TextOutlines {
        let (buffer, size) = self.layout(text, style, 1.0);
        // Glyph offsets count from the start of their line (split at line breaks, as the layout does).
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
        let mut lines = Vec::new();
        let mut glyphs = Vec::new();
        for run in buffer.layout_runs() {
            let line = lines.len();
            lines.push((run.line_top, run.line_height));
            let line_start = line_starts.get(run.line_i).copied().unwrap_or(0);
            for glyph in run.glyphs {
                let physical = glyph.physical((0.0, run.line_y), 1.0);
                // Where the glyph's origin sits on its baseline, as `physical` places it (only the
                // vertical position is snapped to a whole pixel).
                let origin = Vec2::new(glyph.x + glyph.font_size * glyph.x_offset, (run.line_y + glyph.y - glyph.font_size * glyph.y_offset).trunc());
                let contours = self.glyphs.get_outline_commands(&mut self.fonts, physical.cache_key).map(|commands| contours(commands, origin)).unwrap_or_default();
                glyphs.push(GlyphPath { start: line_start + glyph.start, line, left: glyph.x, width: glyph.w, contours });
            }
        }
        TextOutlines { size, lines, glyphs }
    }

    /// Where a caret and selection go in `text` as drawn with `style`, for editing it in place.
    /// Positions are in the layer's source pixels, the same box the text is drawn in.
    pub fn geometry(&mut self, text: &str, style: &TextStyle) -> TextGeometry {
        let (buffer, _) = self.layout(text, style, 1.0);
        // Byte offset in `text` where each line (split at line breaks, as the layout does) starts.
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
        let mut lines = Vec::new();
        for run in buffer.layout_runs() {
            let start = line_starts.get(run.line_i).copied().unwrap_or(text.len());
            let line_text = &text[start..text[start..].find('\n').map_or(text.len(), |end| start + end)];
            // A caret stop at every character boundary of the line, end included.
            let stops = line_text
                .char_indices()
                .map(|(at, _)| at)
                .chain(std::iter::once(line_text.len()))
                .map(|at| (start + at, run.cursor_position(&cosmic_text::Cursor::new(run.line_i, at)).unwrap_or(0.0)))
                .collect();
            lines.push(TextLine { top: run.line_top, height: run.line_height, stops });
        }
        if lines.is_empty() {
            lines.push(TextLine { top: 0.0, height: style.line_height, stops: vec![(0, 0.0)] });
        }
        TextGeometry { lines }
    }

    /// Rasterizes the text at `scale` output pixels per source pixel.
    /// Draws `text` at `scale`, never larger than `max_size` pixels on a side (the GPU's
    /// texture limit); very large text is drawn at a lower scale and stretched.
    pub fn rasterize(&mut self, text: &str, style: &TextStyle, scale: f32, max_size: u32) -> TextImage {
        self.rasterize_highlighted(text, style, None, scale, max_size)
    }

    /// Like `rasterize`, with the characters in a byte range of `text` filled in another color
    /// (a caption's spoken word).
    pub fn rasterize_highlighted(&mut self, text: &str, style: &TextStyle, highlight: Option<(std::ops::Range<usize>, Rgba)>, scale: f32, max_size: u32) -> TextImage {
        let mut scale = scale.clamp(0.05, 8.0);
        let mut laid_out = self.raster_layout(text, style, scale);
        let biggest = laid_out.2.max(laid_out.3);
        if biggest > max_size as usize {
            // Shrink to fit, with a little room for the padding not scaling exactly.
            scale *= max_size as f32 / biggest as f32 * 0.98;
            laid_out = self.raster_layout(text, style, scale);
        }
        let (buffer, size, width, height, pad, stroke) = laid_out;
        let (width, height) = (width.min(max_size as usize), height.min(max_size as usize));

        // Glyph coverage, 0-1, and the part of it that is highlighted.
        let mut coverage = vec![0.0_f32; width * height];
        let mut highlighted = vec![0.0_f32; if highlight.is_some() { width * height } else { 0 }];
        // Color glyphs (emoji) keep their own colors: kept apart, drawn over the rest at the end.
        let mut colored: Vec<[u8; 4]> = Vec::new();
        // The letters' outlines, gathered only when there is a stroke to draw along them.
        let mut outline = PathBuilder::new();
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let lit = highlight.as_ref().is_some_and(|(range, _)| range.contains(&glyph.start));
                let physical = glyph.physical((pad as f32, pad as f32 + run.line_y), 1.0);
                if stroke.is_some() && let Some(commands) = self.glyphs.get_outline_commands(&mut self.fonts, physical.cache_key) {
                    let origin = Vec2::new(pad as f32 + glyph.x + glyph.font_size * glyph.x_offset, pad as f32 + run.line_y + glyph.y - glyph.font_size * glyph.y_offset);
                    add_glyph_outline(&mut outline, commands, origin);
                }
                let Some(image) = self.glyphs.get_image(&mut self.fonts, physical.cache_key).as_ref() else { continue };
                let (left, top) = (physical.x + image.placement.left, physical.y - image.placement.top);
                let (glyph_w, glyph_h) = (image.placement.width as i32, image.placement.height as i32);
                for gy in 0..glyph_h {
                    for gx in 0..glyph_w {
                        let (x, y) = (left + gx, top + gy);
                        if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                            continue;
                        }
                        let index = (gy * glyph_w + gx) as usize;
                        let at = y as usize * width + x as usize;
                        if image.content == SwashContent::Color {
                            if colored.is_empty() {
                                colored = vec![[0; 4]; width * height];
                            }
                            let pixel = &image.data[index * 4..index * 4 + 4];
                            if pixel[3] > colored[at][3] {
                                colored[at] = [pixel[0], pixel[1], pixel[2], pixel[3]];
                            }
                            continue;
                        }
                        let alpha = match image.content {
                            SwashContent::Mask | SwashContent::Color => image.data[index],
                            SwashContent::SubpixelMask => image.data[index * 3],
                        };
                        let value = f32::from(alpha) / 255.0;
                        coverage[at] = coverage[at].max(value);
                        if lit {
                            highlighted[at] = highlighted[at].max(value);
                        }
                    }
                }
            }
        }

        let options = style.stroke_options;
        let stroke_coverage = stroke.map(|(_, stroke_width)| {
            let stroke_width = stroke_width.max(0.0);
            let mut mask = match outline.finish() {
                Some(path) => outline_stroke(&path, width, height, stroke_width, &options, scale),
                None => vec![0.0; width * height],
            };
            // Color glyphs have no outline to follow; they get an even ring instead, as far out
            // as the stroke reaches (a sticker outline).
            if !colored.is_empty() {
                let emoji: Vec<f32> = colored.iter().map(|pixel| f32::from(pixel[3]) / 255.0).collect();
                let reach = match options.position {
                    StrokePosition::Inside => 0.0,
                    StrokePosition::Center => stroke_width * 0.5,
                    StrokePosition::Outside => stroke_width,
                };
                for (cover, ring) in mask.iter_mut().zip(stroke_mask(&emoji, width, height, reach)) {
                    *cover = cover.max(ring);
                }
            }
            // An outside stroke under an opaque fill also runs under the letters, so no seam
            // shows where the soft edges of the two meet.
            if options.position == StrokePosition::Outside && !options.over_fill && style.fill.is_some_and(|fill| fill.a >= 0.999) {
                for (cover, &letter) in mask.iter_mut().zip(&coverage) {
                    *cover = cover.max(letter);
                }
            }
            mask
        });
        let lit = highlight.map(|(_, color)| (highlighted.as_slice(), color));
        let mut pixels = paint(&coverage, stroke_coverage.as_deref(), style.fill, stroke.map(|(color, _)| color), options.over_fill, lit);
        over(&mut pixels, &colored);
        TextImage { width: width as u32, height: height as u32, pixels, size, scale }
    }

    /// Lays text out at `scale`: the buffer, the text box, the raster size with padding for
    /// the stroke, the padding, and the stroke at raster scale.
    #[allow(clippy::type_complexity)]
    fn raster_layout(&mut self, text: &str, style: &TextStyle, scale: f32) -> (Buffer, Vec2, usize, usize, i32, Option<(Rgba, f32)>) {
        let (buffer, size) = self.layout(text, style, scale);
        // Leave room around the glyphs for the stroke.
        let stroke = style.stroke.map(|(color, width)| (color, width * scale));
        let pad = stroke.map_or(0.0, |(_, width)| style.stroke_options.reach(width)).ceil() as i32 + 1;
        let width = (size.x * scale).ceil() as i32 + pad * 2;
        let height = (size.y * scale).ceil() as i32 + pad * 2;
        (buffer, size, width.max(1) as usize, height.max(1) as usize, pad, stroke)
    }
}

/// Adds one glyph's outline to `path`, with the glyph's origin (on its baseline) at `origin`.
/// Font outlines point up; the image's rows go down, so the outline is flipped.
fn add_glyph_outline(path: &mut PathBuilder, commands: &[Command], origin: Vec2) {
    let at = |x: f32, y: f32| (origin.x + x, origin.y - y);
    for command in commands {
        match *command {
            Command::MoveTo(to) => {
                let (x, y) = at(to.x, to.y);
                path.move_to(x, y);
            }
            Command::LineTo(to) => {
                let (x, y) = at(to.x, to.y);
                path.line_to(x, y);
            }
            Command::QuadTo(control, to) => {
                let ((cx, cy), (x, y)) = (at(control.x, control.y), at(to.x, to.y));
                path.quad_to(cx, cy, x, y);
            }
            Command::CurveTo(first, second, to) => {
                let ((ax, ay), (bx, by), (x, y)) = (at(first.x, first.y), at(second.x, second.y), at(to.x, to.y));
                path.cubic_to(ax, ay, bx, by, x, y);
            }
            Command::Close => path.close(),
        }
    }
}

/// Coverage (0-1) of a stroke `stroke_width` pixels wide along the letters' `outline`, in a
/// `width` × `height` image: centered on the outline, or kept to its inside or outside, with
/// the options' corners, ends and dashes (dash lengths are multiplied by `scale`).
fn outline_stroke(outline: &tiny_skia::Path, width: usize, height: usize, stroke_width: f32, options: &TextStrokeOptions, scale: f32) -> Vec<f32> {
    let Some(mut pixmap) = Pixmap::new(width as u32, height as u32) else { return vec![0.0; width * height] };
    let one_sided = options.position != StrokePosition::Center;
    let dash = options.dashed().then(|| tiny_skia::StrokeDash::new(vec![options.dash * scale, options.gap * scale], 0.0)).flatten();
    let stroke = tiny_skia::Stroke {
        // One-sided strokes are drawn twice as wide, then cut to one side of the outline.
        width: if one_sided { stroke_width * 2.0 } else { stroke_width },
        miter_limit: options.miter_limit.max(1.0),
        line_cap: match options.cap {
            StrokeCap::Butt => tiny_skia::LineCap::Butt,
            StrokeCap::Round => tiny_skia::LineCap::Round,
            StrokeCap::Square => tiny_skia::LineCap::Square,
        },
        line_join: match options.join {
            StrokeJoin::Miter => tiny_skia::LineJoin::Miter,
            StrokeJoin::Round => tiny_skia::LineJoin::Round,
            StrokeJoin::Bevel => tiny_skia::LineJoin::Bevel,
        },
        dash,
    };
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(255, 255, 255, 255);
    paint.anti_alias = true;
    let side = one_sided
        .then(|| {
            let mut mask = tiny_skia::Mask::new(width as u32, height as u32)?;
            mask.fill_path(outline, FillRule::Winding, true, Transform::identity());
            if options.position == StrokePosition::Outside {
                mask.invert();
            }
            Some(mask)
        })
        .flatten();
    pixmap.stroke_path(outline, &paint, &stroke, Transform::identity(), side.as_ref());
    pixmap.data().chunks_exact(4).map(|pixel| f32::from(pixel[3]) / 255.0).collect()
}

/// Coverage of a stroke `radius` pixels wide around `coverage`, from a two-pass chamfer distance map.
fn stroke_mask(coverage: &[f32], width: usize, height: usize, radius: f32) -> Vec<f32> {
    const FAR: f32 = 1.0e6;
    let mut distance: Vec<f32> = coverage.iter().map(|&c| if c >= 0.5 { 0.0 } else { FAR }).collect();
    let (straight, diagonal) = (1.0, std::f32::consts::SQRT_2);
    let at = |x: usize, y: usize| y * width + x;
    for y in 0..height {
        for x in 0..width {
            let mut best = distance[at(x, y)];
            if x > 0 {
                best = best.min(distance[at(x - 1, y)] + straight);
            }
            if y > 0 {
                best = best.min(distance[at(x, y - 1)] + straight);
                if x > 0 {
                    best = best.min(distance[at(x - 1, y - 1)] + diagonal);
                }
                if x + 1 < width {
                    best = best.min(distance[at(x + 1, y - 1)] + diagonal);
                }
            }
            distance[at(x, y)] = best;
        }
    }
    for y in (0..height).rev() {
        for x in (0..width).rev() {
            let mut best = distance[at(x, y)];
            if x + 1 < width {
                best = best.min(distance[at(x + 1, y)] + straight);
            }
            if y + 1 < height {
                best = best.min(distance[at(x, y + 1)] + straight);
                if x + 1 < width {
                    best = best.min(distance[at(x + 1, y + 1)] + diagonal);
                }
                if x > 0 {
                    best = best.min(distance[at(x - 1, y + 1)] + diagonal);
                }
            }
            distance[at(x, y)] = best;
        }
    }
    distance.iter().zip(coverage).map(|(&d, &c)| (radius + 0.5 - d).clamp(0.0, 1.0).max(c)).collect()
}

/// Lays straight-alpha `top` pixels over straight-alpha `pixels` (both RGBA), in place.
fn over(pixels: &mut [u8], top: &[[u8; 4]]) {
    for (pixel, source) in pixels.chunks_mut(4).zip(top) {
        let alpha = f32::from(source[3]) / 255.0;
        if alpha <= 0.0 {
            continue;
        }
        let below = f32::from(pixel[3]) / 255.0;
        let out = alpha + below * (1.0 - alpha);
        for channel in 0..3 {
            let mixed = (f32::from(source[channel]) * alpha + f32::from(pixel[channel]) * below * (1.0 - alpha)) / out.max(1e-6);
            pixel[channel] = mixed.round().clamp(0.0, 255.0) as u8;
        }
        pixel[3] = (out * 255.0).round() as u8;
    }
}

/// The fill and stroke together, the fill over the stroke unless `stroke_over_fill`, as
/// straight sRGB RGBA bytes.
fn paint(fill_coverage: &[f32], stroke_coverage: Option<&[f32]>, fill: Option<Rgba>, stroke: Option<Rgba>, stroke_over_fill: bool, highlight: Option<(&[f32], Rgba)>) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(fill_coverage.len() * 4);
    for (index, &coverage) in fill_coverage.iter().enumerate() {
        // Highlighted glyphs take the highlight's color instead of the fill.
        let fill = match highlight {
            Some((lit, color)) if lit[index] > 0.0 => Some(color),
            _ => fill,
        };
        let fill_alpha = fill.map_or(0.0, |color| color.a * coverage);
        let stroke_alpha = match (stroke, stroke_coverage) {
            (Some(color), Some(mask)) => color.a * mask[index],
            _ => 0.0,
        };
        let ((top, top_alpha), (bottom, bottom_alpha)) = if stroke_over_fill { ((stroke, stroke_alpha), (fill, fill_alpha)) } else { ((fill, fill_alpha), (stroke, stroke_alpha)) };
        let alpha = top_alpha + bottom_alpha * (1.0 - top_alpha);
        let channel = |pick: fn(&Rgba) -> f32| {
            if alpha <= 0.0 {
                return 0.0;
            }
            let top_part = top.map_or(0.0, |c| pick(&c)) * top_alpha;
            let bottom_part = bottom.map_or(0.0, |c| pick(&c)) * bottom_alpha * (1.0 - top_alpha);
            (top_part + bottom_part) / alpha
        };
        let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        pixels.extend_from_slice(&[byte(channel(|c| c.r)), byte(channel(|c| c.g)), byte(channel(|c| c.b)), byte(alpha)]);
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> TextStyle {
        TextStyle { font_family: "Helvetica".into(), ..TextStyle::default() }
    }

    #[test]
    fn a_weight_the_font_lacks_draws_its_nearest_weight_not_another_font() {
        let mut text = TextRenderer::new();
        // Any installed family made only in regular that draws Latin letters itself; with none,
        // there is nothing to check.
        let mut families: Vec<String> = text.fonts.db().faces().filter_map(|face| face.families.first().map(|(name, _)| name.clone())).collect();
        families.sort();
        families.dedup();
        let in_family = |text: &mut TextRenderer, family: &str, weight: u16| {
            let style = TextStyle { font_family: family.to_owned(), font_weight: weight, ..TextStyle::default() };
            let (buffer, _) = text.layout("Tide", &style, 1.0);
            let ids: Vec<_> = buffer.layout_runs().flat_map(|run| run.glyphs.iter().map(|glyph| glyph.font_id).collect::<Vec<_>>()).collect();
            !ids.is_empty() && ids.iter().all(|id| text.fonts.db().face(*id).is_some_and(|face| face.families.iter().any(|(name, _)| name == family)))
        };
        let single = families.into_iter().find(|family| {
            let weights: Vec<u16> = text.fonts.db().faces().filter(|face| face.families.iter().any(|(name, _)| name == family)).map(|face| face.weight.0).collect();
            weights.iter().all(|weight| *weight == 400) && !weights.is_empty() && in_family(&mut text, family, 400)
        });
        let Some(family) = single else { return };
        assert_eq!(text.nearest_weight(&family, 700), 400);
        assert!(in_family(&mut text, &family, 700), "bold {family} is drawn in {family}, not the system font");
    }

    #[test]
    fn every_installed_font_with_the_letters_draws_them_itself() {
        // Fonts with quirks this covers, where installed: Skia, an old Apple variable font with
        // a weight scale of its own, and Hoefler Text, whose ornament face shares its name.
        let mut text = TextRenderer::new();
        let mut families: Vec<String> = text.fonts.db().faces().filter_map(|face| face.families.first().map(|(name, _)| name.clone())).collect();
        families.sort();
        families.dedup();
        let mut wrong = Vec::new();
        for family in families {
            let has_letters = text.fonts.db().faces().filter(|face| face.families.iter().any(|(name, _)| *name == family)).any(|face| {
                text.fonts.db().with_face_data(face.id, |data, index| {
                    use cosmic_text::skrifa::MetadataProvider;
                    cosmic_text::skrifa::FontRef::from_index(data, index).is_ok_and(|font| "Tide".chars().all(|letter| font.charmap().map(letter).is_some()))
                }) == Some(true)
            });
            if !has_letters {
                continue;
            }
            for weight in [400, 700] {
                let style = TextStyle { font_family: family.clone(), font_weight: weight, ..TextStyle::default() };
                let (buffer, _) = text.layout("Tide", &style, 1.0);
                let font_ids: Vec<_> = buffer.layout_runs().flat_map(|run| run.glyphs.iter().map(|glyph| glyph.font_id).collect::<Vec<_>>()).collect();
                if let Some(other) = font_ids.iter().filter_map(|id| text.fonts.db().face(*id)).find(|face| !face.families.iter().any(|(name, _)| *name == family)) {
                    wrong.push(format!("{family} at {weight}: drawn in {}", other.post_script_name));
                }
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    #[test]
    fn text_rasterizes_with_ink_and_measures_by_content() {
        let mut text = TextRenderer::new();
        let short = text.measure("Hi", &style());
        let long = text.measure("Hello there", &style());
        assert!(long.x > short.x * 2.0, "{short:?} vs {long:?}");
        let image = text.rasterize("Hi", &style(), 1.0, 8192);
        let ink = image.pixels.chunks(4).filter(|px| px[3] > 128).count();
        assert!(ink > 100, "glyphs were drawn ({ink} solid pixels)");
    }

    #[test]
    fn tracking_widens_text_and_two_lines_double_height() {
        let mut text = TextRenderer::new();
        let plain = text.measure("SPACED", &style());
        let tracked = text.measure("SPACED", &TextStyle { tracking: 200.0, ..style() });
        assert!(tracked.x > plain.x + 50.0, "{plain:?} vs {tracked:?}");
        let two = text.measure("A\nB", &style());
        assert!((two.y - plain.y * 2.0).abs() < 1.0);
    }

    #[test]
    fn a_highlighted_word_takes_its_own_color() {
        let mut text = TextRenderer::new();
        let style = TextStyle { fill: Some(Rgba::hex(0xffffff)), ..style() };
        let image = text.rasterize_highlighted("AB CD", &style, Some((3..5, Rgba::hex(0xff0000))), 1.0, 8192);
        let reddish = |pixel: &[u8]| pixel[3] > 200 && pixel[0] > 200 && pixel[1] < 60;
        let whitish = |pixel: &[u8]| pixel[3] > 200 && pixel[0] > 200 && pixel[1] > 200;
        let columns = image.width as usize;
        let mut left_red = 0;
        let mut right_red = 0;
        let mut left_white = 0;
        for (i, pixel) in image.pixels.chunks(4).enumerate() {
            let x = i % columns;
            if x < columns / 3 {
                left_red += usize::from(reddish(pixel));
                left_white += usize::from(whitish(pixel));
            } else if x > columns * 2 / 3 {
                right_red += usize::from(reddish(pixel));
            }
        }
        assert!(left_white > 20 && left_red == 0, "the first word stays white");
        assert!(right_red > 20, "the second word is red");
    }

    #[test]
    fn the_caret_follows_the_letters_and_clicks_find_them() {
        let mut text = TextRenderer::new();
        let size = text.measure("Hello", &style());
        let geometry = text.geometry("Hello", &style());
        let (start, top, height) = geometry.caret(0);
        let (end, ..) = geometry.caret(5);
        assert!(start.abs() < 1.0 && (end - size.x).abs() < 1.0, "from the left edge ({start}) to the right ({end} of {})", size.x);
        assert!(top.abs() < 0.5 && (height - style().line_height).abs() < 0.5);
        let (middle, ..) = geometry.caret(2);
        assert!(middle > start && middle < end);
        assert_eq!(geometry.hit(middle + 0.5, 5.0), 2, "a click finds the gap nearest it");
        assert_eq!(geometry.hit(-50.0, 5.0), 0);
        assert_eq!(geometry.hit(size.x + 50.0, 5.0), 5);
        let rows = geometry.highlight(1..4);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].0 > start && rows[0].0 + rows[0].2 < end);
    }

    #[test]
    fn a_second_line_has_its_own_carets() {
        let mut text = TextRenderer::new();
        let geometry = text.geometry("AB\nCD", &style());
        assert_eq!(geometry.lines.len(), 2);
        let (_, first_top, height) = geometry.caret(1);
        let (_, second_top, _) = geometry.caret(4);
        assert!((second_top - first_top - height).abs() < 0.5, "the next line is one line lower");
        assert_eq!(geometry.hit(0.0, height * 1.5), 3, "a click on the second line's start");
        assert_eq!(geometry.highlight(1..5).len(), 2, "a selection across the break covers both lines");
        // Empty text still has a place for the caret.
        assert_eq!(text.geometry("", &style()).caret(0).2, style().line_height);
    }

    #[test]
    fn outlined_text_without_a_fill_is_hollow() {
        let mut text = TextRenderer::new();
        let outlined = text.rasterize("O", &TextStyle { fill: None, stroke: Some((Rgba::hex(0xff0000), 4.0)), ..style() }, 1.0, 8192);
        let filled = text.rasterize("O", &TextStyle { stroke: Some((Rgba::hex(0xff0000), 4.0)), ..style() }, 1.0, 8192);
        let ink = |image: &TextImage| image.pixels.chunks(4).filter(|px| px[3] > 128).count();
        assert!(ink(&outlined) > 100, "the ring is drawn");
        assert!(ink(&outlined) < ink(&filled) * 3 / 4, "but not the letter inside it: {} vs {}", ink(&outlined), ink(&filled));
    }

    /// A heavy black "O" with a red stroke `width` wide and `options`.
    fn outlined_o(width: f32, options: TextStrokeOptions) -> TextStyle {
        TextStyle { font_size: 120.0, line_height: 144.0, fill: Some(Rgba::hex(0x000000)), stroke: Some((Rgba::hex(0xff0000), width)), stroke_options: options, ..style() }
    }

    /// Counts solid pixels that are mostly red, and mostly black.
    fn red_and_black(image: &TextImage) -> (usize, usize) {
        let red = image.pixels.chunks(4).filter(|px| px[3] > 200 && px[0] > 200 && px[1] < 60).count();
        let black = image.pixels.chunks(4).filter(|px| px[3] > 200 && px[0] < 40).count();
        (red, black)
    }

    #[test]
    fn an_inside_stroke_stays_within_the_letters() {
        let mut text = TextRenderer::new();
        let plain = text.rasterize("O", &TextStyle { stroke: None, ..outlined_o(0.0, TextStrokeOptions::default()) }, 1.0, 8192);
        let inside = text.rasterize("O", &outlined_o(6.0, TextStrokeOptions { position: StrokePosition::Inside, over_fill: true, ..TextStrokeOptions::default() }), 1.0, 8192);
        let ink = |image: &TextImage| image.pixels.chunks(4).filter(|px| px[3] > 128).count();
        let (red, _) = red_and_black(&inside);
        assert!(red > 300, "the stroke is drawn ({red} red pixels)");
        let (plain_ink, inside_ink) = (ink(&plain), ink(&inside));
        assert!(inside_ink.abs_diff(plain_ink) * 20 < plain_ink, "the letters keep their size (give or take their soft edges): {inside_ink} vs {plain_ink}");
    }

    #[test]
    fn a_stroke_over_the_fill_covers_part_of_the_letters() {
        let mut text = TextRenderer::new();
        let centered = TextStrokeOptions { position: StrokePosition::Center, ..TextStrokeOptions::default() };
        let (under_red, under_black) = red_and_black(&text.rasterize("O", &outlined_o(8.0, centered), 1.0, 8192));
        let (over_red, over_black) = red_and_black(&text.rasterize("O", &outlined_o(8.0, TextStrokeOptions { over_fill: true, ..centered }), 1.0, 8192));
        assert!(over_red > under_red + 100, "more of the stroke shows: {over_red} vs {under_red}");
        assert!(over_black + 100 < under_black, "less of the fill shows: {over_black} vs {under_black}");
    }

    #[test]
    fn a_dashed_stroke_leaves_gaps() {
        let mut text = TextRenderer::new();
        let hollow = |options| TextStyle { fill: None, ..outlined_o(4.0, options) };
        let (solid, _) = red_and_black(&text.rasterize("O", &hollow(TextStrokeOptions::default()), 1.0, 8192));
        let (dashed, _) = red_and_black(&text.rasterize("O", &hollow(TextStrokeOptions { dash: 6.0, gap: 6.0, ..TextStrokeOptions::default() }), 1.0, 8192));
        assert!(dashed * 10 < solid * 7, "gaps leave out part of the line: {dashed} vs {solid}");
    }

    #[test]
    fn sharp_corners_reach_further_than_round_ones() {
        let mut text = TextRenderer::new();
        let hollow = |join| TextStyle { fill: None, ..outlined_o(10.0, TextStrokeOptions { join, ..TextStrokeOptions::default() }) };
        let (round, _) = red_and_black(&text.rasterize("V", &hollow(StrokeJoin::Round), 1.0, 8192));
        let (miter, _) = red_and_black(&text.rasterize("V", &hollow(StrokeJoin::Miter), 1.0, 8192));
        assert!(miter > round, "the V's points stick out: {miter} vs {round}");
    }

    #[test]
    fn stroke_adds_coverage_around_glyphs() {
        let mut text = TextRenderer::new();
        let plain = text.rasterize("O", &style(), 1.0, 8192);
        let stroked = text.rasterize("O", &TextStyle { stroke: Some((Rgba::hex(0xff0000), 6.0)), ..style() }, 1.0, 8192);
        let ink = |image: &TextImage| image.pixels.chunks(4).filter(|px| px[3] > 128).count();
        assert!(ink(&stroked) > ink(&plain) + 200);
        // Stroke pixels are red.
        assert!(stroked.pixels.chunks(4).any(|px| px[3] > 200 && px[0] > 200 && px[1] < 60));
    }
}
