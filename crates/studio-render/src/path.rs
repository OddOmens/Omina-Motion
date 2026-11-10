//! Draws pen paths (custom shapes) on the CPU with tiny-skia. Arbitrary Bézier outlines are hard
//! to draw exactly in a fragment shader, so each path is rasterized once at the size it appears
//! on screen and then drawn like any other texture. The renderer keeps the result until the path,
//! its paints or its on-screen size change.

use studio_core::animation::Vec2;
use studio_core::model::{Paint, Rgba, ShapeStyle};
use studio_core::path::ShapePath;
use tiny_skia::{FillRule, GradientStop, LineCap, LineJoin, LinearGradient, PathBuilder, Pixmap, Point, RadialGradient, Shader, SpreadMode, Transform};

/// A rasterized path: straight-alpha sRGB RGBA pixels, and the rectangle they cover in the
/// layer's own pixels (which reaches past the layer's box by the stroke).
pub struct PathImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub origin: Vec2,
    pub size: Vec2,
}

fn color(rgba: Rgba) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba(rgba.r.clamp(0.0, 1.0), rgba.g.clamp(0.0, 1.0), rgba.b.clamp(0.0, 1.0), rgba.a.clamp(0.0, 1.0)).unwrap_or(tiny_skia::Color::TRANSPARENT)
}

/// A paint as a tiny-skia shader, laid out over a box of `size` the way `layer.wgsl` lays out
/// the same paint on the drawn shapes, so a path and a rectangle with one gradient match: a
/// linear gradient runs between its points in pixels, and a radial one is measured in
/// fractions of the box (so its circle stretches with the box, as an ellipse).
fn shader(paint: Paint, size: Vec2) -> Shader<'static> {
    let Some(gradient) = paint.gradient() else {
        return match paint {
            Paint::Solid(rgba) => Shader::SolidColor(color(rgba)),
            _ => Shader::SolidColor(tiny_skia::Color::TRANSPARENT),
        };
    };
    let stops: Vec<GradientStop> = gradient.stops().iter().map(|stop| GradientStop::new(stop.at, color(stop.color))).collect();
    let first = color(gradient.stops()[0].color);
    match paint {
        Paint::Radial(_) => {
            let center = Point::from_xy(gradient.from.x, gradient.from.y);
            let radius = ((gradient.to.x - gradient.from.x).hypot(gradient.to.y - gradient.from.y)).max(1e-4);
            let stretch = Transform::from_scale(size.x.max(1e-3), size.y.max(1e-3));
            RadialGradient::new(center, center, radius, stops, SpreadMode::Pad, stretch).unwrap_or(Shader::SolidColor(first))
        }
        _ => {
            let from = Point::from_xy(gradient.from.x * size.x, gradient.from.y * size.y);
            let to = Point::from_xy(gradient.to.x * size.x, gradient.to.y * size.y);
            LinearGradient::new(from, to, stops, SpreadMode::Pad, Transform::identity()).unwrap_or(Shader::SolidColor(first))
        }
    }
}

/// Draws `path` (in fractions of a box of `size` layer pixels) with `style`'s fill and stroke,
/// at `scale` output pixels per layer pixel, no larger than `max_side` pixels a side. Returns
/// `None` when there is nothing to draw.
pub fn rasterize(path: &ShapePath, style: &ShapeStyle, size: Vec2, scale: f32, max_side: u32) -> Option<PathImage> {
    if !path.is_sound() {
        return None;
    }
    let pixels_path = path.mapped(|v| Vec2::new(v.x * size.x, v.y * size.y));
    let (min, max) = pixels_path.bounds()?;
    // Room for the stroke, which is centered on the outline, and for its mitred corners.
    let stroke_width = style.stroke.map_or(0.0, |stroke| stroke.width);
    let margin = stroke_width * 2.0 + 2.0;
    let origin = Vec2::new(min.x - margin, min.y - margin);
    let covered = Vec2::new(max.x - min.x + margin * 2.0, max.y - min.y + margin * 2.0);
    // Never larger than the GPU accepts: scale down to fit (it is stretched back when drawn).
    let scale = scale.min(max_side as f32 / covered.x.max(covered.y)).max(1e-3);
    // The floor on `scale` keeps tiny paths drawable; the sides are still never above the limit.
    let side = |length: f32| ((length * scale).ceil() as u32).clamp(1, max_side);
    let (width, height) = (side(covered.x), side(covered.y));
    let mut pixmap = Pixmap::new(width, height)?;

    // Every outline goes into one shape (a compound path's too), so they fill together and an
    // outline inside another can cut a hole. Only closed outlines are filled.
    let build = |closed_only: bool| {
        let mut builder = PathBuilder::new();
        for outline in pixels_path.outlines().filter(|outline| outline.points.len() >= 2 && (!closed_only || outline.closed)) {
            let first = outline.points[0].at;
            builder.move_to(first.x, first.y);
            for [_, control_a, control_b, end] in outline.segments() {
                builder.cubic_to(control_a.x, control_a.y, control_b.x, control_b.y, end.x, end.y);
            }
            if outline.closed {
                builder.close();
            }
        }
        builder.finish()
    };
    let outline = build(false)?;
    // Layer pixels → raster pixels.
    let to_raster = Transform::from_row(scale, 0.0, 0.0, scale, -origin.x * scale, -origin.y * scale);
    let paint_with = |paint: Paint| tiny_skia::Paint { shader: shader(paint, size), anti_alias: true, ..Default::default() };
    let rule = if path.even_odd { FillRule::EvenOdd } else { FillRule::Winding };
    if let (Some(fill), Some(filled)) = (style.fill, build(true)) {
        pixmap.fill_path(&filled, &paint_with(fill), rule, to_raster, None);
    }
    if let Some(stroke) = style.stroke.filter(|stroke| stroke.width > 0.0) {
        let line = tiny_skia::Stroke { width: stroke.width, line_join: LineJoin::Miter, line_cap: if path.closed { LineCap::Butt } else { LineCap::Round }, ..Default::default() };
        pixmap.stroke_path(&outline, &paint_with(stroke.paint), &line, to_raster, None);
    }
    let pixels = pixmap.pixels().iter().flat_map(|pixel| {
        let straight = pixel.demultiply();
        [straight.red(), straight.green(), straight.blue(), straight.alpha()]
    });
    // The rectangle the whole raster covers (a touch more than `covered`, from rounding up).
    let size = Vec2::new(width as f32 / scale, height as f32 / scale);
    Some(PathImage { width, height, pixels: pixels.collect(), origin, size })
}
