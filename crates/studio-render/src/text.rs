//! Text layers: shaping and layout with cosmic-text (real fonts, weight, size, leading,
//! tracking and alignment), rasterized on the CPU into a straight-alpha sRGB image.
//! Stroke is drawn around the glyphs from a distance map, so it is evenly thick.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Weight};
use studio_core::animation::Vec2;
use studio_core::model::{Rgba, TextAlign, TextStyle};

/// A rasterized text layer, ready to upload as a texture.
pub struct TextImage {
    pub width: u32,
    pub height: u32,
    /// Straight (not premultiplied) sRGB RGBA, row by row.
    pub pixels: Vec<u8>,
    /// The text box in the layer's source pixels (before raster scaling).
    pub size: Vec2,
}

pub struct TextRenderer {
    fonts: FontSystem,
    glyphs: SwashCache,
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextRenderer {
    /// Loads the system's fonts. This takes a moment, so it is done once.
    pub fn new() -> Self {
        Self { fonts: FontSystem::new(), glyphs: SwashCache::new() }
    }

    /// Shapes `text` at `scale` × its font size. Returns the buffer and its box (unscaled).
    fn layout(&mut self, text: &str, style: &TextStyle, scale: f32) -> (Buffer, Vec2) {
        let metrics = Metrics::new(style.font_size * scale, style.line_height * scale);
        let mut buffer = Buffer::new(&mut self.fonts, metrics);
        let attrs = Attrs::new()
            .family(Family::Name(&style.font_family))
            .weight(Weight(style.font_weight))
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

    /// Rasterizes the text at `scale` output pixels per source pixel.
    pub fn rasterize(&mut self, text: &str, style: &TextStyle, scale: f32) -> TextImage {
        let scale = scale.clamp(0.05, 8.0);
        let (buffer, size) = self.layout(text, style, scale);
        // Leave room around the glyphs for the stroke.
        let stroke = style.stroke.map(|(color, width)| (color, width * scale));
        let pad = stroke.map_or(0.0, |(_, width)| width).ceil() as i32 + 1;
        let width = (size.x * scale).ceil() as i32 + pad * 2;
        let height = (size.y * scale).ceil() as i32 + pad * 2;
        let (width, height) = (width.max(1) as usize, height.max(1) as usize);

        // Glyph coverage, 0-1.
        let mut coverage = vec![0.0_f32; width * height];
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let physical = glyph.physical((pad as f32, pad as f32 + run.line_y), 1.0);
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
                        let alpha = match image.content {
                            SwashContent::Mask => image.data[index],
                            SwashContent::Color => image.data[index * 4 + 3],
                            SwashContent::SubpixelMask => image.data[index * 3],
                        };
                        let cell = &mut coverage[y as usize * width + x as usize];
                        *cell = cell.max(f32::from(alpha) / 255.0);
                    }
                }
            }
        }

        let stroke_coverage = stroke.map(|(_, stroke_width)| stroke_mask(&coverage, width, height, stroke_width.max(0.0)));
        let pixels = paint(&coverage, stroke_coverage.as_deref(), style.fill, stroke.map(|(color, _)| color));
        TextImage { width: width as u32, height: height as u32, pixels, size }
    }
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

/// Fill over stroke, as straight sRGB RGBA bytes.
fn paint(fill_coverage: &[f32], stroke_coverage: Option<&[f32]>, fill: Option<Rgba>, stroke: Option<Rgba>) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(fill_coverage.len() * 4);
    for (index, &coverage) in fill_coverage.iter().enumerate() {
        let fill_alpha = fill.map_or(0.0, |color| color.a * coverage);
        let stroke_alpha = match (stroke, stroke_coverage) {
            (Some(color), Some(mask)) => color.a * mask[index],
            _ => 0.0,
        };
        let alpha = fill_alpha + stroke_alpha * (1.0 - fill_alpha);
        let channel = |pick: fn(&Rgba) -> f32| {
            if alpha <= 0.0 {
                return 0.0;
            }
            let fill_part = fill.map_or(0.0, |c| pick(&c)) * fill_alpha;
            let stroke_part = stroke.map_or(0.0, |c| pick(&c)) * stroke_alpha * (1.0 - fill_alpha);
            (fill_part + stroke_part) / alpha
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
    fn text_rasterizes_with_ink_and_measures_by_content() {
        let mut text = TextRenderer::new();
        let short = text.measure("Hi", &style());
        let long = text.measure("Hello there", &style());
        assert!(long.x > short.x * 2.0, "{short:?} vs {long:?}");
        let image = text.rasterize("Hi", &style(), 1.0);
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
    fn stroke_adds_coverage_around_glyphs() {
        let mut text = TextRenderer::new();
        let plain = text.rasterize("O", &style(), 1.0);
        let stroked = text.rasterize("O", &TextStyle { stroke: Some((Rgba::hex(0xff0000), 6.0)), ..style() }, 1.0);
        let ink = |image: &TextImage| image.pixels.chunks(4).filter(|px| px[3] > 128).count();
        assert!(ink(&stroked) > ink(&plain) + 200);
        // Stroke pixels are red.
        assert!(stroked.pixels.chunks(4).any(|px| px[3] > 200 && px[0] > 200 && px[1] < 60));
    }
}
