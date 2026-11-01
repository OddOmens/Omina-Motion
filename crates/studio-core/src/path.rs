//! Pen paths: the outline of a custom shape layer, drawn point by point with the Pen tool.
//!
//! A path is a chain of cubic Bézier curves. Each point has two handles: the one going into it
//! (from the previous point) and the one coming out of it (towards the next point). A corner
//! point has both handles at zero; a smooth point has them pointing in opposite directions.
//!
//! Points are stored as fractions of the layer's box (0,0 is its top-left corner, 1,1 its
//! bottom-right), so the path stretches with the box when the layer is resized.

use crate::animation::Vec2;

/// One point on a path, with its handles as offsets from the point (in the same box fractions).
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PathPoint {
    pub at: Vec2,
    pub handle_in: Vec2,
    pub handle_out: Vec2,
}

impl PathPoint {
    /// A corner point (no handles).
    pub fn corner(at: Vec2) -> Self {
        Self { at, handle_in: Vec2::default(), handle_out: Vec2::default() }
    }

    /// A smooth point whose outgoing handle is `handle_out`; the incoming one mirrors it.
    pub fn smooth(at: Vec2, handle_out: Vec2) -> Self {
        Self { at, handle_in: Vec2::new(-handle_out.x, -handle_out.y), handle_out }
    }

    /// The point with every coordinate mapped by `f` (handles stay offsets from the point).
    fn map(self, f: impl Fn(Vec2) -> Vec2) -> Self {
        let at = f(self.at);
        Self { at, handle_in: f(self.at + self.handle_in) - at, handle_out: f(self.at + self.handle_out) - at }
    }
}

/// The outline of a custom shape.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShapePath {
    pub points: Vec<PathPoint>,
    /// Whether the last point joins back to the first. An open path is drawn with its stroke
    /// only; it is never filled.
    pub closed: bool,
}

/// The most points a path can have, so a damaged or hostile file cannot ask for millions.
pub const MAX_PATH_POINTS: usize = 4096;

/// One cubic Bézier segment: start, two control points, end.
pub type Segment = [Vec2; 4];

impl ShapePath {
    /// Whether there is enough to draw: two points, every coordinate a real number.
    pub fn is_sound(&self) -> bool {
        (2..=MAX_PATH_POINTS).contains(&self.points.len())
            && self.points.iter().all(|point| [point.at, point.handle_in, point.handle_out].iter().all(|v| v.x.is_finite() && v.y.is_finite()))
    }

    /// The path's curves, each from one point to the next (and back to the first when closed).
    pub fn segments(&self) -> Vec<Segment> {
        let count = self.points.len();
        let joins = if self.closed { count } else { count.saturating_sub(1) };
        (0..joins)
            .map(|index| {
                let (from, to) = (self.points[index], self.points[(index + 1) % count]);
                [from.at, from.at + from.handle_out, to.at + to.handle_in, to.at]
            })
            .collect()
    }

    /// The same path with every coordinate mapped by `f`.
    pub fn mapped(&self, f: impl Fn(Vec2) -> Vec2) -> Self {
        Self { points: self.points.iter().map(|point| point.map(&f)).collect(), closed: self.closed }
    }

    /// The smallest box holding the drawn curves (not the handles): its top-left and
    /// bottom-right corners. `None` for a path with no points.
    pub fn bounds(&self) -> Option<(Vec2, Vec2)> {
        let first = self.points.first()?.at;
        let (mut min, mut max) = (first, first);
        let mut include = |v: Vec2| {
            min = Vec2::new(min.x.min(v.x), min.y.min(v.y));
            max = Vec2::new(max.x.max(v.x), max.y.max(v.y));
        };
        for point in &self.points {
            include(point.at);
        }
        for segment in self.segments() {
            for t in curve_extremes(segment) {
                include(point_on(segment, t));
            }
        }
        Some((min, max))
    }
}

/// The point at `t` (0–1) along a segment.
pub fn point_on(segment: Segment, t: f32) -> Vec2 {
    let [p0, p1, p2, p3] = segment;
    let u = 1.0 - t;
    p0 * (u * u * u) + p1 * (3.0 * u * u * t) + p2 * (3.0 * u * t * t) + p3 * (t * t * t)
}

/// Where along a segment (0–1) the curve turns back in x or in y: the places its derivative is
/// zero, found by solving the derivative's quadratic for each axis.
fn curve_extremes(segment: Segment) -> Vec<f32> {
    let [p0, p1, p2, p3] = segment;
    let mut found = Vec::new();
    for axis in [|v: Vec2| v.x, |v: Vec2| v.y] {
        let (a0, a1, a2, a3) = (axis(p0), axis(p1), axis(p2), axis(p3));
        // The derivative is a·t² + b·t + c.
        let a = 3.0 * (-a0 + 3.0 * a1 - 3.0 * a2 + a3);
        let b = 6.0 * (a0 - 2.0 * a1 + a2);
        let c = 3.0 * (a1 - a0);
        if a.abs() < 1e-9 {
            if b.abs() > 1e-9 {
                found.push(-c / b);
            }
            continue;
        }
        let discriminant = b * b - 4.0 * a * c;
        if discriminant >= 0.0 {
            let root = discriminant.sqrt();
            found.extend([(-b + root) / (2.0 * a), (-b - root) / (2.0 * a)]);
        }
    }
    found.retain(|t| (0.0..=1.0).contains(t));
    found
}

/// Turns a path in pixels into a box and the path in fractions of it: returns the box's
/// top-left corner, its size (at least 1 pixel a side, so a straight line still has a box), and
/// the fitted path.
pub fn fit_to_box(path: &ShapePath) -> Option<(Vec2, Vec2, ShapePath)> {
    let (min, max) = path.bounds()?;
    let size = Vec2::new((max.x - min.x).max(1.0), (max.y - min.y).max(1.0));
    let fitted = path.mapped(|v| Vec2::new((v.x - min.x) / size.x, (v.y - min.y) / size.y));
    Some((min, size, fitted))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec2, b: Vec2) -> bool {
        (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3
    }

    #[test]
    fn a_closed_path_joins_its_last_point_to_its_first() {
        let corners = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)].map(|(x, y)| PathPoint::corner(Vec2::new(x, y)));
        let mut path = ShapePath { points: corners.to_vec(), closed: false };
        assert_eq!(path.segments().len(), 2);
        path.closed = true;
        assert_eq!(path.segments().len(), 3);
        assert_eq!(path.segments()[2][3], Vec2::new(0.0, 0.0));
    }

    #[test]
    fn bounds_follow_the_curve_not_the_handles() {
        // A smooth bump: the handles reach y = -40, but the curve itself only to y = -30.
        let path = ShapePath {
            points: vec![PathPoint { handle_out: Vec2::new(0.0, -40.0), ..PathPoint::corner(Vec2::new(0.0, 0.0)) }, PathPoint { handle_in: Vec2::new(0.0, -40.0), ..PathPoint::corner(Vec2::new(100.0, 0.0)) }],
            closed: false,
        };
        let (min, max) = path.bounds().unwrap();
        assert!(close(min, Vec2::new(0.0, -30.0)), "{min:?}");
        assert!(close(max, Vec2::new(100.0, 0.0)), "{max:?}");
    }

    #[test]
    fn fitting_maps_the_path_into_its_box() {
        let path = ShapePath { points: vec![PathPoint::corner(Vec2::new(50.0, 20.0)), PathPoint::smooth(Vec2::new(150.0, 120.0), Vec2::new(10.0, 0.0))], closed: false };
        let (origin, size, fitted) = fit_to_box(&path).unwrap();
        assert_eq!(origin, Vec2::new(50.0, 20.0));
        assert_eq!(size, Vec2::new(100.0, 100.0));
        assert!(close(fitted.points[1].at, Vec2::new(1.0, 1.0)));
        assert!(close(fitted.points[1].handle_out, Vec2::new(0.1, 0.0)), "handles scale with the box");
        // A straight horizontal line still gets a box one pixel tall.
        let flat = ShapePath { points: vec![PathPoint::corner(Vec2::new(0.0, 5.0)), PathPoint::corner(Vec2::new(30.0, 5.0))], closed: false };
        assert_eq!(fit_to_box(&flat).unwrap().1, Vec2::new(30.0, 1.0));
    }

    #[test]
    fn unusable_paths_are_caught() {
        assert!(!ShapePath { points: vec![PathPoint::default()], closed: false }.is_sound());
        let broken = ShapePath { points: vec![PathPoint::default(), PathPoint::corner(Vec2::new(f32::NAN, 0.0))], closed: true };
        assert!(!broken.is_sound());
    }
}
