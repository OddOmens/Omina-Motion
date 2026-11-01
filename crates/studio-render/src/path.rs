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
/// the same paint on the drawn shapes, so a path and a rectangle with one gradient match.
fn shader(paint: Paint, size: Vec2) -> Shader<'static> {
    let center = Point::from_xy(size.x * 0.5, size.y * 0.5);
    let stops = |a: Rgba, b: Rgba| vec![GradientStop::new(0.0, color(a)), GradientStop::new(1.0, color(b))];
    match paint {
        Paint::Solid(rgba) => Shader::SolidColor(color(rgba)),
        Paint::Linear { start, end, angle } => {
            // The gradient runs across the box along `angle`, reaching its far corners.
            let (sin, cos) = angle.to_radians().sin_cos();
            let reach = cos.abs() * size.x * 0.5 + sin.abs() * size.y * 0.5;
            let from = Point::from_xy(center.x - cos * reach, center.y - sin * reach);
            let to = Point::from_xy(center.x + cos * reach, center.y + sin * reach);
            LinearGradient::new(from, to, stops(start, end), SpreadMode::Pad, Transform::identity()).unwrap_or(Shader::SolidColor(color(start)))
        }
        Paint::Radial { inner, outer } => {
            // A unit circle stretched to the box, so the gradient is an ellipse like the box.
            let stretch = Transform::from_row(size.x.max(1e-3) * 0.5, 0.0, 0.0, size.y.max(1e-3) * 0.5, center.x, center.y);
            RadialGradient::new(Point::zero(), Point::zero(), 1.0, stops(inner, outer), SpreadMode::Pad, stretch).unwrap_or(Shader::SolidColor(color(inner)))
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
    let (width, height) = (((covered.x * scale).ceil() as u32).max(1), ((covered.y * scale).ceil() as u32).max(1));
    let mut pixmap = Pixmap::new(width, height)?;

    let mut builder = PathBuilder::new();
    let first = pixels_path.points[0].at;
    builder.move_to(first.x, first.y);
    for [_, control_a, control_b, end] in pixels_path.segments() {
        builder.cubic_to(control_a.x, control_a.y, control_b.x, control_b.y, end.x, end.y);
    }
    if path.closed {
        builder.close();
    }
    let outline = builder.finish()?;
    // Layer pixels → raster pixels.
    let to_raster = Transform::from_row(scale, 0.0, 0.0, scale, -origin.x * scale, -origin.y * scale);
    let paint_with = |paint: Paint| tiny_skia::Paint { shader: shader(paint, size), anti_alias: true, ..Default::default() };
    // Only a closed path has an inside to fill.
    if let (Some(fill), true) = (style.fill, path.closed) {
        pixmap.fill_path(&outline, &paint_with(fill), FillRule::Winding, to_raster, None);
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
