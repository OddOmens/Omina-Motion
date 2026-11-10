//! Studio's drawn shapes as geometry both formats can hold: rectangles and ellipses as
//! themselves, and triangles, polygons, stars and pen paths as Bézier paths.
//!
//! The renderer draws the built-in shapes from distance fields (see `layer.wgsl`), and its
//! rounded corners come from shrinking the shape by the corner radius and growing it back. The
//! same is done here geometrically: the shrunk outline is offset outwards by the radius, which
//! turns each outward corner into a circular arc and leaves inward corners (a star's notches)
//! sharp, just as the shader does.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use studio_core::animation::Vec2;
use studio_core::model::{ShapeKind, ShapeStyle};
use studio_core::path::{PathPoint, ShapePath};

/// One outline, in a layer's own pixels.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Geometry {
    Rect { center: Vec2, size: Vec2, radius: f32 },
    Ellipse { center: Vec2, size: Vec2 },
    /// Handles are offsets from their points, as in `ShapePath`, but everything is in pixels.
    Path(ShapePath),
}

impl Geometry {
    /// The same outline with every coordinate multiplied by `factor` (per axis).
    pub fn scaled(&self, factor: Vec2) -> Self {
        let times = |v: Vec2| Vec2::new(v.x * factor.x, v.y * factor.y);
        match self {
            Self::Rect { center, size, radius } => Self::Rect { center: times(*center), size: times(*size), radius: radius * factor.x.min(factor.y) },
            Self::Ellipse { center, size } => Self::Ellipse { center: times(*center), size: times(*size) },
            Self::Path(path) => Self::Path(path.mapped(times)),
        }
    }
}

/// A shape layer's outline in a box of `size`, or `None` when there is nothing to draw (an
/// empty box, or a pen path that is missing or unusable).
pub(crate) fn shape(style: &ShapeStyle, path: Option<&ShapePath>, size: Vec2) -> Option<Geometry> {
    if !(size.x.is_finite() && size.y.is_finite()) || size.x <= 0.0 || size.y <= 0.0 {
        return None;
    }
    let half = size * 0.5;
    // The shader keeps the corner radius between zero and half the shorter side.
    let radius = if style.corner_radius.is_finite() { style.corner_radius.clamp(0.0, half.x.min(half.y)) } else { 0.0 };
    match style.kind {
        ShapeKind::Rectangle => Some(Geometry::Rect { center: half, size, radius }),
        ShapeKind::Ellipse => Some(Geometry::Ellipse { center: half, size }),
        ShapeKind::Triangle => Some(Geometry::Path(triangle(half, radius))),
        ShapeKind::Polygon | ShapeKind::Star => Some(Geometry::Path(regular(style, half, radius))),
        ShapeKind::Path => {
            let path = path.filter(|path| path.is_sound())?;
            Some(Geometry::Path(path.mapped(|v| Vec2::new(v.x * size.x, v.y * size.y))))
        }
    }
}

/// The triangle the shader draws: point up, base along the bottom of the box. Its corners are
/// rounded by pulling it towards its incenter by `radius` and growing it back out.
fn triangle(half: Vec2, radius: f32) -> ShapePath {
    let corners = [Vec2::new(half.x, 0.0), Vec2::new(half.x * 2.0, half.y * 2.0), Vec2::new(0.0, half.y * 2.0)];
    let lengths = [(corners[1] - corners[2]).length(), (corners[2] - corners[0]).length(), (corners[0] - corners[1]).length()];
    let perimeter = lengths.iter().sum::<f32>().max(1e-4);
    let incenter = (corners[0] * lengths[0] + corners[1] * lengths[1] + corners[2] * lengths[2]) * (1.0 / perimeter);
    let doubled_area = cross(corners[1] - corners[0], corners[2] - corners[0]).abs();
    let inradius = doubled_area / perimeter;
    let keep = (inradius - radius).max(0.0) / inradius.max(1e-4);
    let shrunk: Vec<Vec2> = corners.iter().map(|&corner| incenter + (corner - incenter) * keep).collect();
    rounded(&shrunk, radius, |v| v)
}

/// A regular polygon or star with a point on top, as the shader draws it: made in a circle
/// that fits the shorter side, then stretched to fill the box.
fn regular(style: &ShapeStyle, half: Vec2, radius: f32) -> ShapePath {
    let outer = half.x.min(half.y);
    let sides = style.points.clamp(3, studio_core::model::MAX_SHAPE_POINTS) as usize;
    // Points around the circle, clockwise from the top, in a square space centered on 0.
    let around = |angle: f32, distance: f32| Vec2::new(angle.sin() * distance, -angle.cos() * distance);
    let corners: Vec<Vec2> = if style.kind == ShapeKind::Polygon {
        // The polygon's edges move in by the radius (its apothem shrinks), so the corners do too.
        let apothem = (outer * (PI / sides as f32).cos() - radius).max(0.0);
        let reach = apothem / (PI / sides as f32).cos();
        (0..sides).map(|index| around(index as f32 * TAU / sides as f32, reach)).collect()
    } else {
        let reach = (outer - radius).max(0.0);
        let inner = reach * style.inner_radius.clamp(studio_core::model::INNER_RADIUS_RANGE.0, studio_core::model::INNER_RADIUS_RANGE.1);
        (0..sides * 2).map(|index| around(index as f32 * PI / sides as f32, if index % 2 == 0 { reach } else { inner })).collect()
    };
    // Stretched from the circle to the box, and moved to the box's middle.
    let stretch = Vec2::new(half.x / outer.max(1e-4), half.y / outer.max(1e-4));
    rounded(&corners, radius, |v| Vec2::new(half.x + v.x * stretch.x, half.y + v.y * stretch.y))
}

fn cross(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

fn unit(v: Vec2) -> Vec2 {
    let length = v.length();
    if length > 1e-6 { v * (1.0 / length) } else { Vec2::default() }
}

/// The closed polygon `corners` grown outwards by `radius`: outward corners become arcs of that
/// radius around them, inward corners stay sharp (where the grown edges meet). Every point is
/// then mapped by `place`, which may stretch the result.
fn rounded(corners: &[Vec2], radius: f32, place: impl Fn(Vec2) -> Vec2) -> ShapePath {
    let count = corners.len();
    let signed_area: f32 = (0..count).map(|index| cross(corners[index], corners[(index + 1) % count])).sum();
    // A shape shrunk to nothing grows back as a circle of the radius around where it was.
    if signed_area.abs() < 1e-6 {
        let center = corners.iter().fold(Vec2::default(), |sum, &corner| sum + corner) * (1.0 / count.max(1) as f32);
        return circle(center, radius, place);
    }
    let orientation = signed_area.signum();
    // The normal pointing out of the shape, for an edge running along `direction`.
    let outward = |direction: Vec2| {
        let d = unit(direction);
        Vec2::new(d.y, -d.x) * orientation
    };
    let mut points: Vec<PathPoint> = Vec::new();
    for index in 0..count {
        let (previous, corner, next) = (corners[(index + count - 1) % count], corners[index], corners[(index + 1) % count]);
        if radius <= 1e-4 {
            points.push(PathPoint::corner(corner));
            continue;
        }
        let (n1, n2) = (outward(corner - previous), outward(next - corner));
        let turns_out = cross(corner - previous, next - corner) * orientation > 0.0;
        if turns_out {
            points.extend(arc(corner, radius, n1, n2, orientation));
        } else {
            // Where the two grown edges meet.
            let along = n1 + n2;
            let meet = 1.0 + n1.x * n2.x + n1.y * n2.y;
            points.push(PathPoint::corner(corner + along * (radius / meet.max(1e-3))));
        }
    }
    let placed = ShapePath { points, closed: true, ..Default::default() };
    placed.mapped(place)
}

/// An arc of `radius` around `center`, from the direction `from` to the direction `to` (unit
/// vectors), turning the way `orientation` says, as cubic pieces of at most a quarter turn.
fn arc(center: Vec2, radius: f32, from: Vec2, to: Vec2, orientation: f32) -> Vec<PathPoint> {
    let start = from.y.atan2(from.x);
    let mut sweep = to.y.atan2(to.x) - start;
    // The short way round, in the shape's own turning direction.
    if orientation > 0.0 {
        while sweep < 0.0 {
            sweep += TAU;
        }
    } else {
        while sweep > 0.0 {
            sweep -= TAU;
        }
    }
    let pieces = (sweep.abs() / FRAC_PI_2).ceil().max(1.0) as usize;
    let step = sweep / pieces as f32;
    // The standard handle length for a cubic that follows a circular arc of `step`.
    let handle = 4.0 / 3.0 * (step / 4.0).tan() * radius;
    let at = |angle: f32| center + Vec2::new(angle.cos(), angle.sin()) * radius;
    let tangent = |angle: f32| Vec2::new(-angle.sin(), angle.cos()) * handle;
    (0..=pieces)
        .map(|index| {
            let angle = start + step * index as f32;
            let handle_in = if index == 0 { Vec2::default() } else { tangent(angle) * -1.0 };
            let handle_out = if index == pieces { Vec2::default() } else { tangent(angle) };
            PathPoint { at: at(angle), handle_in, handle_out }
        })
        .collect()
}

/// A closed circle of four quarter arcs, mapped by `place`.
fn circle(center: Vec2, radius: f32, place: impl Fn(Vec2) -> Vec2) -> ShapePath {
    let handle = 4.0 / 3.0 * (PI / 8.0).tan() * radius;
    let points = (0..4)
        .map(|index| {
            let angle = index as f32 * FRAC_PI_2;
            let (direction, tangent) = (Vec2::new(angle.cos(), angle.sin()), Vec2::new(-angle.sin(), angle.cos()));
            PathPoint { at: center + direction * radius, handle_in: tangent * -handle, handle_out: tangent * handle }
        })
        .collect();
    ShapePath { points, closed: true, ..Default::default() }.mapped(place)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec2, b: Vec2) -> bool {
        (a.x - b.x).abs() < 1e-2 && (a.y - b.y).abs() < 1e-2
    }

    fn style(kind: ShapeKind, radius: f32) -> ShapeStyle {
        ShapeStyle { corner_radius: radius, ..ShapeStyle::new(kind) }
    }

    #[test]
    fn rectangles_and_ellipses_fill_their_box() {
        let size = Vec2::new(200.0, 100.0);
        assert_eq!(shape(&style(ShapeKind::Rectangle, 500.0), None, size), Some(Geometry::Rect { center: Vec2::new(100.0, 50.0), size, radius: 50.0 }), "the radius is kept to half the short side");
        assert_eq!(shape(&style(ShapeKind::Ellipse, 0.0), None, size), Some(Geometry::Ellipse { center: Vec2::new(100.0, 50.0), size }));
        assert_eq!(shape(&style(ShapeKind::Ellipse, 0.0), None, Vec2::new(0.0, 10.0)), None);
    }

    #[test]
    fn a_sharp_triangle_points_up_from_the_bottom_of_its_box() {
        let Some(Geometry::Path(path)) = shape(&style(ShapeKind::Triangle, 0.0), None, Vec2::new(100.0, 80.0)) else { panic!() };
        let corners: Vec<Vec2> = path.points.iter().map(|point| point.at).collect();
        assert!(close(corners[0], Vec2::new(50.0, 0.0)) && close(corners[1], Vec2::new(100.0, 80.0)) && close(corners[2], Vec2::new(0.0, 80.0)), "{corners:?}");
        assert!(path.closed);
    }

    #[test]
    fn rounding_keeps_a_polygons_edges_where_they_were() {
        // A square-boxed hexagon: rounding its corners leaves its edge midpoints in place.
        let size = Vec2::new(200.0, 200.0);
        let Some(Geometry::Path(sharp)) = shape(&style(ShapeKind::Polygon, 0.0), None, size) else { panic!() };
        let Some(Geometry::Path(round)) = shape(&style(ShapeKind::Polygon, 20.0), None, size) else { panic!() };
        assert_eq!(sharp.points.len(), 6);
        assert!(round.points.len() > 6, "each corner became an arc");
        let farthest = |path: &ShapePath| path.points.iter().map(|point| (point.at - Vec2::new(100.0, 100.0)).length()).fold(0.0_f32, f32::max);
        assert!(farthest(&round) < farthest(&sharp), "the corners are cut back");
        // The straight edges between the arcs sit where the sharp edges were: their middles are
        // the apothem (center to edge middle) away from the center.
        let apothem = 100.0 * (PI / 6.0).cos();
        let count = round.points.len();
        let edge_middles: Vec<f32> = (0..count)
            .map(|index| (round.points[index], round.points[(index + 1) % count]))
            .filter(|(from, to)| from.handle_out == Vec2::default() && to.handle_in == Vec2::default())
            .map(|(from, to)| ((from.at + to.at) * 0.5 - Vec2::new(100.0, 100.0)).length())
            .collect();
        assert_eq!(edge_middles.len(), 6);
        assert!(edge_middles.iter().all(|distance| (distance - apothem).abs() < 0.05), "{edge_middles:?} vs {apothem}");
    }

    #[test]
    fn a_star_alternates_tips_and_notches_and_keeps_its_notches_sharp() {
        let mut star = style(ShapeKind::Star, 10.0);
        star.points = 5;
        let Some(Geometry::Path(path)) = shape(&star, None, Vec2::new(100.0, 100.0)) else { panic!() };
        let sharp = path.points.iter().filter(|point| point.handle_in == Vec2::default() && point.handle_out == Vec2::default()).count();
        assert_eq!(sharp, 5, "five sharp notches; the tips are arcs");
        // The tips still reach the circle around the box.
        let top = path.points.iter().map(|point| point.at.y).fold(f32::MAX, f32::min);
        assert!(top.abs() < 0.5, "top tip at {top}");
    }

    #[test]
    fn pen_paths_are_scaled_from_box_fractions_to_pixels() {
        let path = ShapePath { points: vec![PathPoint::corner(Vec2::new(0.0, 0.0)), PathPoint::smooth(Vec2::new(1.0, 0.5), Vec2::new(0.1, 0.0))], closed: false, ..Default::default() };
        let Some(Geometry::Path(placed)) = shape(&style(ShapeKind::Path, 0.0), Some(&path), Vec2::new(200.0, 100.0)) else { panic!() };
        assert_eq!(placed.points[1].at, Vec2::new(200.0, 50.0));
        assert!(close(placed.points[1].handle_out, Vec2::new(20.0, 0.0)));
        assert_eq!(shape(&style(ShapeKind::Path, 0.0), None, Vec2::new(10.0, 10.0)), None, "a pen shape without its path draws nothing");
    }
}
