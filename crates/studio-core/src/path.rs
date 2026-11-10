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
    /// More outlines that are part of the same shape, as in an SVG compound path: they are
    /// filled together with this one, so an outline inside another (the inside of an "O")
    /// cuts a hole. Each is closed or open on its own; their own `compound` is unused.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compound: Vec<ShapePath>,
    /// How overlapping outlines fill: by the even-odd rule (each overlap flips between inside
    /// and outside) rather than the non-zero rule (the default, as in SVG).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub even_odd: bool,
}

/// The most extra outlines one compound path can have.
pub const MAX_COMPOUND_OUTLINES: usize = 1024;

/// The most points a path can have, so a damaged or hostile file cannot ask for millions.
pub const MAX_PATH_POINTS: usize = 4096;

/// One cubic Bézier segment: start, two control points, end.
pub type Segment = [Vec2; 4];

impl ShapePath {
    /// Whether there is enough to draw: two points, every coordinate a real number.
    pub fn is_sound(&self) -> bool {
        let finite = |path: &ShapePath| path.points.iter().all(|point| [point.at, point.handle_in, point.handle_out].iter().all(|v| v.x.is_finite() && v.y.is_finite()));
        (2..=MAX_PATH_POINTS).contains(&self.points.len())
            && finite(self)
            && self.compound.len() <= MAX_COMPOUND_OUTLINES
            && self.compound.iter().all(|outline| outline.points.len() <= MAX_PATH_POINTS && finite(outline))
    }

    /// This outline and its compound ones, in drawing order.
    pub fn outlines(&self) -> impl Iterator<Item = &ShapePath> {
        std::iter::once(self).chain(&self.compound)
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
        let map_one = |path: &ShapePath| Self { points: path.points.iter().map(|point| point.map(&f)).collect(), closed: path.closed, compound: Vec::new(), even_odd: path.even_odd };
        Self { compound: self.compound.iter().map(map_one).collect(), ..map_one(self) }
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
        // Every outline counts, the compound ones too.
        for outline in self.outlines() {
            for point in &outline.points {
                include(point.at);
            }
            for segment in outline.segments() {
                for t in curve_extremes(segment) {
                    include(point_on(segment, t));
                }
            }
        }
        Some((min, max))
    }
}

/// A path flattened into short straight pieces, measured along its length, for placing things
/// (such as letters) a given distance along it.
#[derive(Clone, Debug, PartialEq)]
pub struct Walk {
    points: Vec<Vec2>,
    /// The distance along the path at each point.
    distances: Vec<f32>,
    closed: bool,
}

/// Straight pieces per curve: enough that letters follow even tight curves smoothly.
const WALK_STEPS: usize = 48;

impl Walk {
    /// Flattens `path`, with every coordinate mapped by `place` (into composition pixels, say).
    pub fn new(path: &ShapePath, place: impl Fn(Vec2) -> Vec2) -> Option<Self> {
        let segments = path.segments();
        let first = segments.first()?;
        let mut points = vec![place(first[0])];
        for segment in &segments {
            points.extend((1..=WALK_STEPS).map(|step| place(point_on(*segment, step as f32 / WALK_STEPS as f32))));
        }
        let mut distances = Vec::with_capacity(points.len());
        let mut total = 0.0;
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                total += (*point - points[index - 1]).length();
            }
            distances.push(total);
        }
        Some(Self { points, distances, closed: path.closed })
    }

    /// The path's length.
    pub fn length(&self) -> f32 {
        self.distances.last().copied().unwrap_or(0.0)
    }

    /// The point `distance` along the path and the direction it runs there (radians, 0 to the
    /// right, clockwise on screen). A closed path wraps around; an open one has nothing before
    /// its start or past its end.
    pub fn at(&self, distance: f32) -> Option<(Vec2, f32)> {
        let length = self.length();
        if length <= 0.0 || !distance.is_finite() {
            return None;
        }
        let distance = if self.closed { distance.rem_euclid(length) } else { distance };
        if !(0.0..=length).contains(&distance) {
            return None;
        }
        let index = self.distances.partition_point(|&reached| reached < distance).clamp(1, self.points.len() - 1);
        let (from, to) = (self.points[index - 1], self.points[index]);
        let piece = self.distances[index] - self.distances[index - 1];
        let t = if piece > 0.0 { (distance - self.distances[index - 1]) / piece } else { 0.0 };
        let direction = to - from;
        Some((from + direction * t, direction.y.atan2(direction.x)))
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
        let mut path = ShapePath { points: corners.to_vec(), closed: false, ..Default::default() };
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
            ..Default::default()
        };
        let (min, max) = path.bounds().unwrap();
        assert!(close(min, Vec2::new(0.0, -30.0)), "{min:?}");
        assert!(close(max, Vec2::new(100.0, 0.0)), "{max:?}");
    }

    #[test]
    fn fitting_maps_the_path_into_its_box() {
        let path = ShapePath { points: vec![PathPoint::corner(Vec2::new(50.0, 20.0)), PathPoint::smooth(Vec2::new(150.0, 120.0), Vec2::new(10.0, 0.0))], closed: false, ..Default::default() };
        let (origin, size, fitted) = fit_to_box(&path).unwrap();
        assert_eq!(origin, Vec2::new(50.0, 20.0));
        assert_eq!(size, Vec2::new(100.0, 100.0));
        assert!(close(fitted.points[1].at, Vec2::new(1.0, 1.0)));
        assert!(close(fitted.points[1].handle_out, Vec2::new(0.1, 0.0)), "handles scale with the box");
        // A straight horizontal line still gets a box one pixel tall.
        let flat = ShapePath { points: vec![PathPoint::corner(Vec2::new(0.0, 5.0)), PathPoint::corner(Vec2::new(30.0, 5.0))], closed: false, ..Default::default() };
        assert_eq!(fit_to_box(&flat).unwrap().1, Vec2::new(30.0, 1.0));
    }

    #[test]
    fn walking_a_path_finds_points_and_directions_along_it() {
        // An open L: 100 right, then 100 down.
        let corners = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)].map(|(x, y)| PathPoint::corner(Vec2::new(x, y)));
        let path = ShapePath { points: corners.to_vec(), closed: false, ..Default::default() };
        let walk = Walk::new(&path, |v| v).unwrap();
        assert!((walk.length() - 200.0).abs() < 1e-3);
        let (point, angle) = walk.at(50.0).unwrap();
        assert!(close(point, Vec2::new(50.0, 0.0)) && angle.abs() < 1e-3, "along the top, heading right");
        let (point, angle) = walk.at(150.0).unwrap();
        assert!(close(point, Vec2::new(100.0, 50.0)) && (angle - std::f32::consts::FRAC_PI_2).abs() < 1e-3, "down the side, heading down");
        assert!(walk.at(250.0).is_none() && walk.at(-1.0).is_none(), "nothing off the ends of an open path");
        // Closed, it comes back up the diagonal and wraps around.
        let closed = Walk::new(&ShapePath { closed: true, ..path }, |v| v).unwrap();
        assert!(closed.at(closed.length() + 50.0).is_some_and(|(point, _)| close(point, Vec2::new(50.0, 0.0))));
    }

    #[test]
    fn unusable_paths_are_caught() {
        assert!(!ShapePath { points: vec![PathPoint::default()], closed: false, ..Default::default() }.is_sound());
        let broken = ShapePath { points: vec![PathPoint::default(), PathPoint::corner(Vec2::new(f32::NAN, 0.0))], closed: true, ..Default::default() };
        assert!(!broken.is_sound());
    }
}
