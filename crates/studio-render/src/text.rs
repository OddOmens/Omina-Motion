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
    /// The scale the raster was actually drawn at: the one asked for, unless that would have
    /// made it too big (over 8× or larger than `max_size` pixels).
    pub scale: f32,
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
        // Kept in the editor's ranges even for a style that slipped past them: a line height of 0
        // stops the layout, and a negative one never finishes.
        let (font_size, line_height) = (style.font_size.clamp(1.0, 2000.0), style.line_height.clamp(1.0, 4000.0));
        let metrics = Metrics::new(font_size * scale, line_height * scale);
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

    /// Where each letter of the text's first line sits, in source pixels: its left edge and its
    /// width (the space it takes, so the letters tile the line), and how far down the line its
    /// baseline is. For setting text along a path.
    pub fn letters(&mut self, text: &str, style: &TextStyle) -> (Vec<(f32, f32)>, f32) {
        let (buffer, _) = self.layout(text, style, 1.0);
        let Some(run) = buffer.layout_runs().next() else { return (Vec::new(), 0.0) };
        (run.glyphs.iter().map(|glyph| (glyph.x, glyph.w)).collect(), run.line_y)
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
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let lit = highlight.as_ref().is_some_and(|(range, _)| range.contains(&glyph.start));
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

        // A stroke goes around color glyphs too (a sticker outline).
        let shape: Vec<f32> = if colored.is_empty() { coverage.clone() } else { coverage.iter().zip(&colored).map(|(&c, pixel)| c.max(f32::from(pixel[3]) / 255.0)).collect() };
        let stroke_coverage = stroke.map(|(_, stroke_width)| stroke_mask(&shape, width, height, stroke_width.max(0.0)));
        let lit = highlight.map(|(_, color)| (highlighted.as_slice(), color));
        let mut pixels = paint(&coverage, stroke_coverage.as_deref(), style.fill, stroke.map(|(color, _)| color), lit);
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
        let pad = stroke.map_or(0.0, |(_, width)| width).ceil() as i32 + 1;
        let width = (size.x * scale).ceil() as i32 + pad * 2;
        let height = (size.y * scale).ceil() as i32 + pad * 2;
        (buffer, size, width.max(1) as usize, height.max(1) as usize, pad, stroke)
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

/// Fill over stroke, as straight sRGB RGBA bytes.
fn paint(fill_coverage: &[f32], stroke_coverage: Option<&[f32]>, fill: Option<Rgba>, stroke: Option<Rgba>, highlight: Option<(&[f32], Rgba)>) -> Vec<u8> {
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
