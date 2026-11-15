//! Shape morphing: one outline turning smoothly into another.
//!
//! A shape layer's outline can be keyframed (its `ShapeMorph`). Between two keyframes the two
//! outlines are first matched up, then blended point by point:
//!
//! 1. Both are cut into the same number of points. Every curve is split into pieces along its
//!    length, so the outline keeps exactly its shape (corners stay sharp) while gaining points.
//! 2. For closed outlines, the second one's starting point and direction are chosen so the
//!    points travel as little as possible (otherwise the shape twists or turns inside out).
//! 3. A closed outline morphing into an open one is opened at its first point first.
//!
//! Rectangles, ellipses, triangles, polygons and stars are drawn from their box, not from
//! points, so they are turned into an equivalent path (`outline`) before they morph.
//!
//! The same keyframes can carry the shape's style (fill, stroke, corner settings), so colors
//! and stroke widths blend along with the outline. "Morph To" (`morph_to`) builds such
//! keyframes from one shape layer into another.

use std::borrow::Cow;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::animation::{Animated, Blend, EASY_EASE, Easing, Keyframe, Lerp, Vec2};
use crate::gradient::{Gradient, GradientStop};
use crate::model::{Composition, Id, INNER_RADIUS_RANGE, Layer, LayerContent, MAX_SHAPE_POINTS, Paint, Rgba, ShapeKind, ShapeStyle, Stroke};
use crate::path::{MAX_PATH_POINTS, PathPoint, Segment, ShapePath, point_on};
use crate::edit::PathPart;

/// Keyframes that morph a shape layer's outline (and optionally its style) over time.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShapeMorph {
    /// The outline at each keyframe, in fractions of the layer's box. Points may reach past
    /// the box: the outline being morphed into can be larger than the layer's own.
    pub path: Animated<ShapePath>,
    /// The fill, stroke and corner settings at each keyframe. When animated, its static value
    /// mirrors the layer's own style. The editor keyframes a style edit at the playhead (see
    /// `edit_style_at`); an edit made through `edit::edit_shape` applies across the whole
    /// morph instead (see `sync_style_edit`).
    pub style: Animated<ShapeStyle>,
}

impl ShapeMorph {
    /// Whether any part of it changes over time.
    pub fn is_animated(&self) -> bool {
        self.path.is_animated() || self.style.is_animated()
    }

    /// Moves every keyframe by `delta` frames.
    pub fn shift(&mut self, delta: i64) {
        self.path.shift(delta);
        self.style.shift(delta);
    }

    /// The frames that have a keyframe of either the outline or the style, in order.
    pub fn keyframe_frames(&self) -> Vec<i64> {
        let mut frames = self.path.keyframe_frames();
        frames.extend(self.style.keyframe_frames());
        frames.sort_unstable();
        frames.dedup();
        frames
    }
}

/// The fewest points two outlines are cut into before they morph. Enough that a square
/// becoming a circle passes through smooth in-between shapes rather than lumpy ones.
pub const MORPH_POINTS: usize = 48;

/// The longest a Morph To can run, in frames, and the default length in seconds.
pub const MAX_MORPH_FRAMES: i64 = 100_000;
pub const DEFAULT_MORPH_SECONDS: f64 = 1.0;

// ---------------------------------------------------------------------------------------------
// Box shapes as paths
// ---------------------------------------------------------------------------------------------

/// The outline of a drawn box shape (rectangle, ellipse, triangle, polygon or star) in a box of
/// `size`, as a closed path in fractions of the box. Matches what the renderer draws, rounded
/// corners included. `None` for a pen path, whose outline is its own path.
pub fn outline(style: &ShapeStyle, size: Vec2) -> Option<ShapePath> {
    let half = size * 0.5;
    let radius = if style.kind.has_corners() { style.corner_radius.clamp(0.0, half.x.min(half.y)) } else { 0.0 };
    let points = match style.kind {
        ShapeKind::Path => return None,
        ShapeKind::Ellipse => ellipse(half),
        ShapeKind::Rectangle => rounded_rectangle(half, radius),
        ShapeKind::Triangle => {
            // Point up, base along the bottom; rounding shrinks it toward its incenter first.
            let corners = [Vec2::new(0.0, -half.y), Vec2::new(half.x, half.y), Vec2::new(-half.x, half.y)];
            let sides = [(corners[1] - corners[2]).length(), (corners[2] - corners[0]).length(), (corners[0] - corners[1]).length()];
            let perimeter: f32 = sides.iter().sum::<f32>().max(1e-4);
            let center = (corners[0] * sides[0] + corners[1] * sides[1] + corners[2] * sides[2]) * (1.0 / perimeter);
            let inradius = cross(corners[1] - corners[0], corners[2] - corners[0]).abs() / perimeter;
            let shrink = (inradius - radius).max(0.0) / inradius.max(1e-4);
            rounded_polygon(&corners.map(|corner| center + (corner - center) * shrink), radius)
        }
        ShapeKind::Polygon | ShapeKind::Star => {
            // Regular shapes are built in a circle of radius `r` with y pointing up, a point on
            // top, then stretched to the box (as the renderer does).
            let r = half.x.min(half.y).max(1e-4);
            let count = style.points.clamp(3, MAX_SHAPE_POINTS) as usize;
            let corners: Vec<Vec2> = if style.kind == ShapeKind::Polygon {
                let apothem = r * (PI / count as f32).cos();
                let circumradius = (apothem - radius).max(0.0) / (PI / count as f32).cos();
                (0..count).map(|index| from_top(index as f32 * TAU / count as f32, circumradius)).collect()
            } else {
                let outer = (r - radius).max(0.0);
                let inner = outer * style.inner_radius.clamp(INNER_RADIUS_RANGE.0, INNER_RADIUS_RANGE.1);
                (0..count * 2).map(|index| from_top(index as f32 * PI / count as f32, if index % 2 == 0 { outer } else { inner })).collect()
            };
            let stretch = |v: Vec2| Vec2::new(v.x * half.x / r, -v.y * half.y / r);
            rounded_polygon(&corners, radius).into_iter().map(|point| PathPoint { at: stretch(point.at), handle_in: stretch(point.handle_in), handle_out: stretch(point.handle_out) }).collect()
        }
    };
    // From the box's center to fractions of the box.
    let to_box = |v: Vec2| Vec2::new((v.x + half.x) / size.x.max(1e-4), (v.y + half.y) / size.y.max(1e-4));
    let to_box_offset = |v: Vec2| Vec2::new(v.x / size.x.max(1e-4), v.y / size.y.max(1e-4));
    let points = points.into_iter().map(|point| PathPoint { at: to_box(point.at), handle_in: to_box_offset(point.handle_in), handle_out: to_box_offset(point.handle_out) }).collect();
    Some(ShapePath { points, closed: true, ..Default::default() })
}

/// A rectangle with half size `half`, centered on the origin, its corners rounded by quarter
/// circles of `radius` (at most half its shorter side). Built directly rather than by rounding a
/// shrunk polygon, because a pill (radius half the height) or a circle shrinks to a line or a
/// point, which has no corners left to round. Clockwise on screen from the top-left corner.
fn rounded_rectangle(half: Vec2, radius: f32) -> Vec<PathPoint> {
    let inner = Vec2::new(half.x - radius, half.y - radius);
    let reach = radius * QUARTER_CIRCLE_HANDLE;
    // Each corner's circle center, and the angle (y pointing down) its quarter arc starts at.
    let corners = [
        (Vec2::new(-inner.x, -inner.y), PI),
        (Vec2::new(inner.x, -inner.y), PI * 1.5),
        (Vec2::new(inner.x, inner.y), 0.0),
        (Vec2::new(-inner.x, inner.y), FRAC_PI_2),
    ];
    let mut points: Vec<PathPoint> = Vec::new();
    for (center, start) in corners {
        let end = start + FRAC_PI_2;
        let on_circle = |angle: f32| center + Vec2::new(angle.cos(), angle.sin()) * radius;
        // Moving along the arc as the angle grows.
        let tangent = |angle: f32| Vec2::new(-angle.sin(), angle.cos()) * reach;
        points.push(PathPoint { at: on_circle(start), handle_in: Vec2::default(), handle_out: tangent(start) });
        points.push(PathPoint { at: on_circle(end), handle_in: tangent(end) * -1.0, handle_out: Vec2::default() });
    }
    merge_touching_points(points)
}

/// Joins neighboring points of a closed outline that sit in the same place (where one arc ends
/// and the next begins, with no straight side between them) into one point that keeps the
/// incoming handle of the first and the outgoing handle of the second.
fn merge_touching_points(points: Vec<PathPoint>) -> Vec<PathPoint> {
    const SAME_PLACE: f32 = 1e-4;
    let mut merged: Vec<PathPoint> = Vec::with_capacity(points.len());
    for point in points {
        match merged.last_mut() {
            Some(last) if (last.at - point.at).length() < SAME_PLACE => last.handle_out = point.handle_out,
            _ => merged.push(point),
        }
    }
    // The outline is closed, so its last point may touch its first.
    if merged.len() > 1 && (merged[merged.len() - 1].at - merged[0].at).length() < SAME_PLACE {
        let last = merged.pop().expect("there are at least two points");
        merged[0].handle_in = last.handle_in;
    }
    merged
}

/// The point `distance` from the center at `angle` clockwise from straight up (y pointing up).
fn from_top(angle: f32, distance: f32) -> Vec2 {
    Vec2::new(angle.sin() * distance, angle.cos() * distance)
}

/// How far a cubic's handles reach to draw a quarter circle of radius 1.
const QUARTER_CIRCLE_HANDLE: f32 = 0.552_284_8;

/// An ellipse filling a box with half size `half`, centered on the origin: four smooth points
/// (top, right, bottom, left).
fn ellipse(half: Vec2) -> Vec<PathPoint> {
    let (kx, ky) = (half.x * QUARTER_CIRCLE_HANDLE, half.y * QUARTER_CIRCLE_HANDLE);
    vec![
        PathPoint::smooth(Vec2::new(0.0, -half.y), Vec2::new(kx, 0.0)),
        PathPoint::smooth(Vec2::new(half.x, 0.0), Vec2::new(0.0, ky)),
        PathPoint::smooth(Vec2::new(0.0, half.y), Vec2::new(-kx, 0.0)),
        PathPoint::smooth(Vec2::new(-half.x, 0.0), Vec2::new(0.0, -ky)),
    ]
}

fn cross(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

fn dot(a: Vec2, b: Vec2) -> f32 {
    a.x * b.x + a.y * b.y
}

/// The vector scaled to length 1, or zero for a vector too short to have a direction.
fn unit(v: Vec2) -> Vec2 {
    let length = v.length();
    if length > 1e-6 { v * (1.0 / length) } else { Vec2::default() }
}

/// A polygon grown outward by `radius`: each outward-pointing corner becomes a circular arc of
/// that radius and each inward-pointing one a sharp corner. This is how the renderer rounds
/// shapes (shrink, then grow back), so a polygon shrunk by `radius` first gets rounded corners.
fn rounded_polygon(corners: &[Vec2], radius: f32) -> Vec<PathPoint> {
    let count = corners.len();
    let twice_area: f32 = (0..count).map(|index| cross(corners[index], corners[(index + 1) % count])).sum();
    // A polygon shrunk to a point (or a line) grows back into a circle.
    if radius > 0.0 && twice_area.abs() < 1e-6 {
        let center = corners.iter().fold(Vec2::default(), |sum, &corner| sum + corner) * (1.0 / count.max(1) as f32);
        return ellipse(Vec2::new(radius, radius)).into_iter().map(|point| PathPoint { at: point.at + center, ..point }).collect();
    }
    let turning = if twice_area >= 0.0 { 1.0 } else { -1.0 };
    // The side of an edge (running along `direction`) that faces out of the polygon.
    let outward = |direction: Vec2| Vec2::new(direction.y, -direction.x) * turning;
    let mut points = Vec::new();
    for index in 0..count {
        let corner = corners[index];
        let into = unit(corner - corners[(index + count - 1) % count]);
        let out_of = unit(corners[(index + 1) % count] - corner);
        if radius <= 0.0 || into == Vec2::default() || out_of == Vec2::default() {
            points.push(PathPoint::corner(corner));
            continue;
        }
        let (normal_in, normal_out) = (outward(into), outward(out_of));
        let bend = cross(into, out_of) * turning;
        if bend > 1e-6 {
            // An outward corner: a circular arc around it, in pieces of at most a quarter turn.
            let start = normal_in.y.atan2(normal_in.x);
            let sweep = cross(normal_in, normal_out).atan2(dot(normal_in, normal_out));
            let pieces = (sweep.abs() / FRAC_PI_2).ceil().max(1.0) as usize;
            let step = sweep / pieces as f32;
            let reach = radius * 4.0 / 3.0 * (step.abs() / 4.0).tan();
            for piece in 0..=pieces {
                let angle = start + step * piece as f32;
                let (sin, cos) = angle.sin_cos();
                let tangent = Vec2::new(-sin, cos) * (step.signum() * reach);
                points.push(PathPoint {
                    at: corner + Vec2::new(cos, sin) * radius,
                    handle_in: if piece == 0 { Vec2::default() } else { tangent * -1.0 },
                    handle_out: if piece == pieces { Vec2::default() } else { tangent },
                });
            }
        } else if bend < -1e-6 {
            // An inward corner stays sharp, where the two pushed-out edges meet.
            points.push(PathPoint::corner(corner + (normal_in + normal_out) * (radius / (1.0 + dot(normal_in, normal_out)).max(1e-3))));
        } else {
            points.push(PathPoint::corner(corner + normal_in * radius));
        }
    }
    points
}

// ---------------------------------------------------------------------------------------------
// Resampling and matching
// ---------------------------------------------------------------------------------------------

/// Splits a cubic at `t` (de Casteljau): the part before and the part after.
fn split(segment: Segment, t: f32) -> (Segment, Segment) {
    let [p0, p1, p2, p3] = segment;
    let (p01, p12, p23) = (p0.lerp(p1, t), p1.lerp(p2, t), p2.lerp(p3, t));
    let (p012, p123) = (p01.lerp(p12, t), p12.lerp(p23, t));
    let middle = p012.lerp(p123, t);
    ([p0, p01, p012, middle], [middle, p123, p23, p3])
}

/// Straight pieces used to measure a curve's length.
const MEASURE_STEPS: usize = 24;

/// The distance along a segment at each of `MEASURE_STEPS + 1` evenly spaced values of t.
fn measure(segment: Segment) -> Vec<f32> {
    let mut lengths = Vec::with_capacity(MEASURE_STEPS + 1);
    let mut total = 0.0;
    let mut previous = segment[0];
    lengths.push(0.0);
    for step in 1..=MEASURE_STEPS {
        let point = point_on(segment, step as f32 / MEASURE_STEPS as f32);
        total += (point - previous).length();
        previous = point;
        lengths.push(total);
    }
    lengths
}

/// The t (0–1) at which a segment measured as `lengths` has covered `fraction` of its length.
fn t_at_fraction(lengths: &[f32], fraction: f32) -> f32 {
    let total = *lengths.last().unwrap_or(&0.0);
    if total <= 0.0 {
        return fraction;
    }
    let target = fraction * total;
    let index = lengths.partition_point(|&length| length < target).clamp(1, lengths.len() - 1);
    let (before, after) = (lengths[index - 1], lengths[index]);
    let within = if after > before { (target - before) / (after - before) } else { 0.0 };
    (index as f32 - 1.0 + within) / MEASURE_STEPS as f32
}

/// Shares out `pieces` among segments of the given lengths: at least one each, the rest in
/// proportion to length (largest remainders first), so points end up evenly spread.
fn share_out(lengths: &[f32], pieces: usize) -> Vec<usize> {
    let count = lengths.len();
    let extra = pieces.saturating_sub(count);
    let total: f32 = lengths.iter().sum();
    let mut shares: Vec<usize> = vec![1; count];
    if extra == 0 || count == 0 {
        return shares;
    }
    let exact: Vec<f32> = lengths.iter().map(|&length| if total > 0.0 { extra as f32 * length / total } else { extra as f32 / count as f32 }).collect();
    let mut given = 0;
    for (share, &amount) in shares.iter_mut().zip(&exact) {
        *share += amount.floor() as usize;
        given += amount.floor() as usize;
    }
    let mut by_remainder: Vec<usize> = (0..count).collect();
    by_remainder.sort_by(|&a, &b| (exact[b] - exact[b].floor()).total_cmp(&(exact[a] - exact[a].floor())));
    for &index in by_remainder.iter().cycle().take(extra.saturating_sub(given)) {
        shares[index] += 1;
    }
    shares
}

/// The same outline cut into `count` points: each curve is split into pieces along its length
/// (longer curves get more), so the shape is exactly the same. A path that already has
/// `count` points or more comes back unchanged.
pub fn resample(path: &ShapePath, count: usize) -> ShapePath {
    let count = count.min(MAX_PATH_POINTS);
    if path.points.len() < 2 || count <= path.points.len() {
        return path.clone();
    }
    let segments = path.segments();
    let measured: Vec<Vec<f32>> = segments.iter().map(|&segment| measure(segment)).collect();
    let lengths: Vec<f32> = measured.iter().map(|lengths| *lengths.last().unwrap_or(&0.0)).collect();
    // A closed path has as many curves as points; an open one has one fewer.
    let pieces_wanted = if path.closed { count } else { count - 1 };
    let shares = share_out(&lengths, pieces_wanted);
    let mut pieces: Vec<Segment> = Vec::with_capacity(pieces_wanted);
    for ((segment, lengths), &share) in segments.iter().zip(&measured).zip(&shares) {
        let mut rest = *segment;
        let mut done = 0.0;
        for piece in 1..share {
            let t = t_at_fraction(lengths, piece as f32 / share as f32);
            // Where `t` falls within the part not yet cut off.
            let local = if done < 1.0 { ((t - done) / (1.0 - done)).clamp(0.0, 1.0) } else { 0.0 };
            let (before, after) = split(rest, local);
            pieces.push(before);
            rest = after;
            done = t;
        }
        pieces.push(rest);
    }
    let piece_count = pieces.len();
    let mut points: Vec<PathPoint> = (0..piece_count)
        .map(|index| {
            let piece = pieces[index];
            let handle_in = match (index, path.closed) {
                (0, false) => path.points[0].handle_in,
                (0, true) => pieces[piece_count - 1][2] - piece[0],
                _ => pieces[index - 1][2] - piece[0],
            };
            PathPoint { at: piece[0], handle_in, handle_out: piece[1] - piece[0] }
        })
        .collect();
    if !path.closed {
        let last_piece = pieces[piece_count - 1];
        let last = path.points[path.points.len() - 1];
        points.push(PathPoint { at: last_piece[3], handle_in: last_piece[2] - last_piece[3], handle_out: last.handle_out });
    }
    ShapePath { points, closed: path.closed, ..Default::default() }
}

/// The same outline drawn the other way round.
pub fn reversed(path: &ShapePath) -> ShapePath {
    let points = path.points.iter().rev().map(|point| PathPoint { at: point.at, handle_in: point.handle_out, handle_out: point.handle_in }).collect();
    ShapePath { points, closed: path.closed, ..Default::default() }
}

/// A closed outline as an open one that runs all the way round, ending where it starts; an
/// open path comes back unchanged.
fn opened(path: &ShapePath) -> ShapePath {
    if !path.closed || path.points.is_empty() {
        return path.clone();
    }
    let mut points = path.points.clone();
    let first = points[0];
    points[0].handle_in = Vec2::default();
    points.push(PathPoint { at: first.at, handle_in: first.handle_in, handle_out: Vec2::default() });
    ShapePath { points, closed: false, ..Default::default() }
}

/// How far apart two outlines with the same number of points are, point by point, with `to`
/// started `offset` points along. Checks at most about 64 points, so long paths stay quick.
fn travel(from: &[PathPoint], to: &[PathPoint], offset: usize) -> f32 {
    let count = from.len();
    let stride = (count / 64).max(1);
    (0..count).step_by(stride).map(|index| {
        let gap = from[index].at - to[(index + offset) % count].at;
        dot(gap, gap)
    })
    .sum()
}

/// `to` started at the point and run in the direction that make its points travel least on
/// the way from `from` (both must have the same number of points). An open path can only be
/// run either way; a closed one can also start anywhere.
fn aligned(from: &ShapePath, to: ShapePath) -> ShapePath {
    // The counts only differ when opening a closed outline took it past the point limit, so
    // `resample` could not match them; it is then blended as it is, rather than compared point
    // for point past the end of the shorter one.
    if from.points.len() != to.points.len() {
        return to;
    }
    let backwards = reversed(&to);
    let starts = if from.closed { from.points.len() } else { 1 };
    let mut best = (f32::INFINITY, false, 0);
    for (is_reversed, candidate) in [(false, &to), (true, &backwards)] {
        for offset in 0..starts {
            let cost = travel(&from.points, &candidate.points, offset);
            if cost < best.0 {
                best = (cost, is_reversed, offset);
            }
        }
    }
    let (_, is_reversed, offset) = best;
    let mut chosen = if is_reversed { backwards } else { to };
    chosen.points.rotate_left(offset);
    chosen
}

/// Two outlines made ready to blend point by point: the same number of points, both open or
/// both closed, and the second one started and run so its points travel least.
pub fn matched(from: &ShapePath, to: &ShapePath) -> (ShapePath, ShapePath) {
    let (from, to) = if from.closed == to.closed { (from.clone(), to.clone()) } else { (opened(from), opened(to)) };
    let count = from.points.len().max(to.points.len()).max(MORPH_POINTS);
    let (from, to) = (resample(&from, count), resample(&to, count));
    let to = aligned(&from, to);
    (from, to)
}

/// The outline `t` of the way (0–1, or past either end for curves that overshoot) from `from`
/// to `to`. An outline that is closed at one end and open at the other is closed for the
/// first half and open for the second.
pub fn morph(from: &ShapePath, to: &ShapePath, t: f32) -> ShapePath {
    if t == 0.0 || !to.is_sound() {
        return from.clone();
    }
    if t == 1.0 || !from.is_sound() {
        return to.clone();
    }
    let (start, end) = matched(from, to);
    let points = start
        .points
        .iter()
        .zip(&end.points)
        .map(|(a, b)| PathPoint { at: a.at.lerp(b.at, t), handle_in: a.handle_in.lerp(b.handle_in, t), handle_out: a.handle_out.lerp(b.handle_out, t) })
        .collect();
    ShapePath { points, closed: if t < 0.5 { from.closed } else { to.closed }, ..Default::default() }
}

impl Blend for ShapePath {
    fn blend(&self, other: &Self, t: f32) -> Self {
        morph(self, other, t)
    }
}

// ---------------------------------------------------------------------------------------------
// Blending styles
// ---------------------------------------------------------------------------------------------

impl Lerp for Rgba {
    fn lerp(self, other: Self, t: f32) -> Self {
        let channel = |a: f32, b: f32| a.lerp(b, t).clamp(0.0, 1.0);
        Rgba { r: channel(self.r, other.r), g: channel(self.g, other.g), b: channel(self.b, other.b), a: channel(self.a, other.a), link: if t < 0.5 { self.link } else { other.link } }
    }
}

/// `paint` as a gradient laid out like `like`: a solid color becomes a gradient of that one
/// color, so it can blend with a real gradient.
fn as_gradient(paint: Paint, like: Gradient) -> Gradient {
    match paint {
        Paint::Solid(color) => Gradient::new(&like.stops().iter().map(|stop| GradientStop { color, ..*stop }).collect::<Vec<_>>(), like.from, like.to),
        Paint::Linear(gradient) | Paint::Radial(gradient) => gradient,
    }
}

impl Lerp for Paint {
    fn lerp(self, other: Self, t: f32) -> Self {
        let (a, b) = match (self.gradient(), other.gradient()) {
            (None, None) => return Paint::Solid(self.colors()[0].lerp(other.colors()[0], t)),
            (Some(a), None) => (a, as_gradient(other, a)),
            (None, Some(b)) => (as_gradient(self, b), b),
            (Some(a), Some(b)) => (a, b),
        };
        // Stops at the denser gradient's locations, each color blended from both gradients there.
        let stops: Vec<GradientStop> = if a.stops().len() == b.stops().len() {
            a.stops().iter().zip(b.stops()).map(|(x, y)| GradientStop { at: x.at.lerp(y.at, t), color: x.color.lerp(y.color, t) }).collect()
        } else {
            let denser = if a.stops().len() > b.stops().len() { a } else { b };
            denser.stops().iter().map(|stop| GradientStop { at: stop.at, color: a.color_at(stop.at).lerp(b.color_at(stop.at), t) }).collect()
        };
        let gradient = Gradient::new(&stops, a.from.lerp(b.from, t), a.to.lerp(b.to, t));
        // A solid end takes the other end's kind of gradient; linear and radial swap halfway.
        let kind_from = match (self, other) {
            (Paint::Solid(_), _) => other,
            (_, Paint::Solid(_)) => self,
            _ if t < 0.5 => self,
            _ => other,
        };
        kind_from.with_gradient(gradient)
    }
}

/// `paint` with every color's opacity multiplied by `factor`.
fn faded(mut paint: Paint, factor: f32) -> Paint {
    for color in paint.colors_mut() {
        color.a *= factor.clamp(0.0, 1.0);
    }
    paint
}

/// A fill that may be off at either end: one that is off fades the other in or out.
fn blend_fill(from: Option<Paint>, to: Option<Paint>, t: f32) -> Option<Paint> {
    match (from, to) {
        (Some(a), Some(b)) => Some(a.lerp(b, t)),
        (Some(a), None) => Some(faded(a, 1.0 - t)),
        (None, Some(b)) => Some(faded(b, t)),
        (None, None) => None,
    }
}

/// A stroke that may be off at either end: one that is off fades the other in or out.
fn blend_stroke(from: Option<Stroke>, to: Option<Stroke>, t: f32) -> Option<Stroke> {
    match (from, to) {
        (Some(a), Some(b)) => Some(Stroke { paint: a.paint.lerp(b.paint, t), width: a.width.lerp(b.width, t).max(0.0) }),
        (Some(a), None) => Some(Stroke { paint: faded(a.paint, 1.0 - t), ..a }),
        (None, Some(b)) => Some(Stroke { paint: faded(b.paint, t), ..b }),
        (None, None) => None,
    }
}

impl Lerp for ShapeStyle {
    fn lerp(self, other: Self, t: f32) -> Self {
        let nearer = if t < 0.5 { self } else { other };
        ShapeStyle {
            kind: nearer.kind,
            fill: blend_fill(self.fill, other.fill, t),
            stroke: blend_stroke(self.stroke, other.stroke, t),
            corner_radius: self.corner_radius.lerp(other.corner_radius, t).max(0.0),
            points: (self.points as f32).lerp(other.points as f32, t).round().clamp(3.0, MAX_SHAPE_POINTS as f32) as u32,
            inner_radius: self.inner_radius.lerp(other.inner_radius, t).clamp(INNER_RADIUS_RANGE.0, INNER_RADIUS_RANGE.1),
            trim: crate::model::Trim {
                start: self.trim.start.lerp(other.trim.start, t),
                end: self.trim.end.lerp(other.trim.end, t),
                offset: self.trim.offset.lerp(other.trim.offset, t),
            },
        }
    }
}

// ---------------------------------------------------------------------------------------------
// What a shape layer draws
// ---------------------------------------------------------------------------------------------

impl Layer {
    /// The pen path a shape layer draws at `frame`: its morph's outline there while the outline
    /// is keyframed, otherwise its own path. `None` for drawn box shapes and other layers.
    pub fn shape_path_at(&self, frame: f64) -> Option<Cow<'_, ShapePath>> {
        let LayerContent::Shape { path, .. } = &self.content else { return None };
        match self.morph.as_ref().filter(|morph| morph.path.is_animated()) {
            Some(morph) if path.is_some() => Some(Cow::Owned(morph.path.value_at(frame))),
            _ => path.as_ref().map(Cow::Borrowed),
        }
    }

    /// The style a shape layer draws with at `frame`: its morph's while the style is keyframed,
    /// otherwise its own. `None` for layers that are not shapes.
    pub fn shape_style_at(&self, frame: f64) -> Option<ShapeStyle> {
        let LayerContent::Shape { style, .. } = &self.content else { return None };
        Some(self.morph.as_ref().filter(|morph| morph.style.is_animated()).map_or(*style, |morph| morph.style.value_at(frame)))
    }

    /// Whether the layer's outline is keyframed.
    pub fn is_morphing(&self) -> bool {
        self.morph.as_ref().is_some_and(|morph| morph.path.is_animated())
    }

    /// The size a shape is drawn at `frame`, in layer pixels. While the outline is keyframed
    /// this is the size of the outline there (which can differ from the layer's box, and is how
    /// a shape's size is animated); otherwise it is the layer's own size.
    pub fn shape_size_at(&self, frame: f64) -> Vec2 {
        if !self.is_morphing() {
            return self.size;
        }
        match self.shape_path_at(frame).and_then(|path| path.bounds()) {
            Some((min, max)) => Vec2::new((max.x - min.x) * self.size.x, (max.y - min.y) * self.size.y),
            None => self.size,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Edits
// ---------------------------------------------------------------------------------------------

/// Turns a drawn box shape into a pen path with the same outline (rounded corners included),
/// so its points can be edited and morphed. Returns whether the layer is now a path (true when
/// it already was one); false for other layers and locked ones.
pub fn convert_to_path(composition: &mut Composition, layer_id: Id) -> bool {
    let Some(layer) = composition.layer_mut(layer_id) else { return false };
    let size = layer.size;
    let locked = layer.switches.locked;
    let LayerContent::Shape { style, path } = &mut layer.content else { return false };
    if path.is_some() {
        return true;
    }
    if locked {
        return false;
    }
    let Some(converted) = outline(style, size) else { return false };
    *path = Some(converted);
    style.kind = ShapeKind::Path;
    style.corner_radius = 0.0;
    // Style keyframes describe the same shape, so they become a path with square corners too
    // (the rounding is now part of the outline).
    if let Some(morph) = layer.morph.as_mut() {
        morph.style.map_values(|mut keyed| {
            keyed.kind = ShapeKind::Path;
            keyed.corner_radius = 0.0;
            keyed
        });
    }
    true
}

/// A morph that does nothing yet: the layer's own path and style, not keyframed.
fn still_morph(path: ShapePath, style: ShapeStyle) -> ShapeMorph {
    ShapeMorph { path: Animated::constant(path), style: Animated::constant(style) }
}

/// Drops a layer's morph once nothing in it is keyframed any more.
fn tidy(layer: &mut Layer) {
    if layer.morph.as_ref().is_some_and(|morph| !morph.is_animated()) {
        layer.morph = None;
    }
}

/// Tidies a layer after outline keyframes were removed from outside this module (deleted in
/// the timeline, say). If none are left, the outline that remained becomes the layer's own
/// path and the morph is dropped once nothing else in it is keyframed. Call it only for a
/// layer whose outline was keyframed just before. Returns whether the animation ended.
pub fn settle_path_keyframes(layer: &mut Layer) -> bool {
    let Some(morph) = layer.morph.as_ref().filter(|morph| !morph.path.is_animated()) else { return false };
    let kept = morph.path.value.clone();
    if let LayerContent::Shape { path: Some(path), .. } = &mut layer.content {
        *path = kept;
    }
    tidy(layer);
    true
}

/// The Path stopwatch. Turning it on makes the layer a pen path (if it was a drawn shape) with
/// an outline keyframe at `frame`; turning it off keeps the outline it has at `frame` and drops
/// the outline keyframes. Returns whether anything changed.
pub fn toggle_path_animation(composition: &mut Composition, layer_id: Id, frame: i64) -> bool {
    if composition.layer(layer_id).is_none_or(|layer| layer.switches.locked) || !convert_to_path(composition, layer_id) {
        return false;
    }
    let Some(layer) = composition.layer_mut(layer_id) else { return false };
    let Some(current) = layer.shape_path_at(frame as f64).map(Cow::into_owned) else { return false };
    let LayerContent::Shape { style, path } = &mut layer.content else { return false };
    if layer.morph.as_ref().is_some_and(|morph| morph.path.is_animated()) {
        *path = Some(current.clone());
        if let Some(morph) = layer.morph.as_mut() {
            morph.path = Animated::constant(current);
        }
        tidy(layer);
    } else {
        let morph = layer.morph.get_or_insert_with(|| still_morph(current.clone(), *style));
        morph.path = Animated::keyframed(vec![Keyframe::linear(frame, current)]);
    }
    true
}

/// Adds an outline keyframe at `frame` holding the outline there, or removes the one already
/// there (the Path keyframe button). With no outline keyframes yet this starts the stopwatch.
/// Removing the last keyframe stops it and keeps that outline. Returns whether anything changed.
pub fn toggle_path_keyframe(composition: &mut Composition, layer_id: Id, frame: i64) -> bool {
    if !composition.layer(layer_id).is_some_and(Layer::is_morphing) {
        return toggle_path_animation(composition, layer_id, frame);
    }
    let Some(layer) = composition.layer_mut(layer_id) else { return false };
    if layer.switches.locked {
        return false;
    }
    let Some(morph) = layer.morph.as_mut() else { return false };
    morph.path.toggle_keyframe(frame);
    if !morph.path.is_animated() {
        let kept = morph.path.value.clone();
        if let LayerContent::Shape { path, .. } = &mut layer.content {
            *path = Some(kept);
        }
        tidy(layer);
    }
    true
}

/// Stops the morph's fill and stroke keyframes, keeping the style the layer has at `frame`.
pub fn stop_style_morph(composition: &mut Composition, layer_id: Id, frame: i64) -> bool {
    let Some(layer) = composition.layer_mut(layer_id) else { return false };
    let Some(kept) = layer.shape_style_at(frame as f64) else { return false };
    let Some(morph) = layer.morph.as_mut().filter(|morph| morph.style.is_animated()) else { return false };
    morph.style = Animated::constant(kept);
    if let LayerContent::Shape { style, .. } = &mut layer.content {
        *style = kept;
    }
    tidy(layer);
    true
}

/// The Fill & Stroke stopwatch. Turning it on keyframes the shape's style (fill, stroke and
/// stroke width, with its corner settings) at `frame`; turning it off keeps the style it has at
/// `frame` and drops the style keyframes. Returns whether anything changed.
pub fn toggle_style_animation(composition: &mut Composition, layer_id: Id, frame: i64) -> bool {
    let Some(layer) = composition.layer_mut(layer_id).filter(|layer| !layer.switches.locked) else { return false };
    if layer.morph.as_ref().is_some_and(|morph| morph.style.is_animated()) {
        return stop_style_morph(composition, layer_id, frame);
    }
    let LayerContent::Shape { style, path } = &layer.content else { return false };
    let (style, path) = (*style, path.clone().unwrap_or_default());
    // A drawn box shape has no outline of its own; its morph's outline is never keyframed, so
    // it is never drawn.
    let morph = layer.morph.get_or_insert_with(|| still_morph(path, style));
    morph.style = Animated::keyframed(vec![Keyframe::linear(frame, style)]);
    true
}

/// Adds a style keyframe at `frame` holding the style there, or removes the one already there
/// (the Fill & Stroke keyframe button). With no style keyframes yet this starts the stopwatch.
/// Removing the last keyframe stops it and keeps that style. Returns whether anything changed.
pub fn toggle_style_keyframe(composition: &mut Composition, layer_id: Id, frame: i64) -> bool {
    if !composition.layer(layer_id).and_then(|layer| layer.morph.as_ref()).is_some_and(|morph| morph.style.is_animated()) {
        return toggle_style_animation(composition, layer_id, frame);
    }
    let Some(layer) = composition.layer_mut(layer_id).filter(|layer| !layer.switches.locked) else { return false };
    if let Some(morph) = layer.morph.as_mut() {
        morph.style.toggle_keyframe(frame);
    }
    settle_style_keyframes(layer);
    true
}

/// Tidies a layer after style keyframes were removed (here, or deleted in the timeline). If
/// none are left, the style that remained becomes the layer's own and the morph is dropped
/// once nothing else in it is keyframed. Returns whether the animation ended.
pub fn settle_style_keyframes(layer: &mut Layer) -> bool {
    let Some(morph) = layer.morph.as_ref().filter(|morph| !morph.style.is_animated()) else { return false };
    let kept = morph.style.value;
    if let LayerContent::Shape { style, .. } = &mut layer.content {
        *style = kept;
    }
    tidy(layer);
    true
}

/// Changes a shape layer's style with `edit`, as `edit::edit_shape` does. While the style is
/// keyframed the edit starts from the style drawn at `frame` and is keyframed there, as any
/// animated property is keyframed when it is changed; the other keyframes are untouched.
/// Returns whether the layer is a shape.
pub fn edit_style_at(composition: &mut Composition, layer_id: Id, frame: i64, edit: impl FnOnce(&mut ShapeStyle)) -> bool {
    let Some(layer) = composition.layer_mut(layer_id) else { return false };
    if !layer.morph.as_ref().is_some_and(|morph| morph.style.is_animated()) {
        return crate::edit::edit_shape(composition, layer_id, edit);
    }
    let Some(drawn) = layer.shape_style_at(frame as f64) else { return false };
    if let LayerContent::Shape { style, .. } = &mut layer.content {
        *style = drawn;
    }
    if !crate::edit::edit_own_shape_style(layer, edit) {
        return false;
    }
    let LayerContent::Shape { style, .. } = &layer.content else { return false };
    let edited = *style;
    if let Some(morph) = layer.morph.as_mut() {
        morph.style.set_at(frame, edited);
        // The static value mirrors the layer's own style (see `ShapeMorph::style`).
        morph.style.value = edited;
    }
    true
}

/// Moves a point (or handle) of a keyframed outline at `frame` to `to`, in the layer's own
/// pixels, keyframing the outline there as any animated property is keyframed when changed.
/// The layer's box stays put (points may go past it), so the other keyframes are untouched.
/// Returns whether it changed.
pub fn move_morph_point(composition: &mut Composition, layer_id: Id, frame: i64, index: usize, part: PathPart, to: Vec2) -> bool {
    let Some(layer) = composition.layer_mut(layer_id) else { return false };
    let size = layer.size;
    if layer.switches.locked || !layer.is_morphing() || !(to.x.is_finite() && to.y.is_finite()) || size.x <= 0.0 || size.y <= 0.0 {
        return false;
    }
    let Some(mut path) = layer.shape_path_at(frame as f64).map(Cow::into_owned) else { return false };
    let Some(point) = path.points.get_mut(index) else { return false };
    let fraction = Vec2::new(to.x / size.x, to.y / size.y);
    match part {
        PathPart::Point => point.at = fraction,
        PathPart::HandleIn => point.handle_in = fraction - point.at,
        PathPart::HandleOut => point.handle_out = fraction - point.at,
    }
    let Some(morph) = layer.morph.as_mut() else { return false };
    morph.path.set_at(frame, path);
    true
}

/// Sets the size of a keyframed outline at `frame`, in layer pixels, keyframing the outline
/// there: this is how a shape's size is animated. The outline is stretched about its own
/// middle, so the shape stays where it is. Sides are kept from 1 to `MAX_SHAPE_SIZE` pixels; a
/// side the outline has no extent along (a perfectly flat line) is left alone. Returns whether
/// it changed.
pub fn set_morph_size(composition: &mut Composition, layer_id: Id, frame: i64, size: Vec2) -> bool {
    let Some(layer) = composition.layer_mut(layer_id) else { return false };
    let layer_size = layer.size;
    if layer.switches.locked || !layer.is_morphing() || layer_size.x <= 0.0 || layer_size.y <= 0.0 {
        return false;
    }
    let Some(path) = layer.shape_path_at(frame as f64).map(Cow::into_owned) else { return false };
    let Some((min, max)) = path.bounds() else { return false };
    let middle = (min + max) * 0.5;
    // How much to stretch one axis: the wanted side over the side it has now, both in pixels.
    let factor = |wanted: f32, extent: f32, side: f32| {
        let now = extent * side;
        if !wanted.is_finite() || now < 1.0e-3 { 1.0 } else { wanted.clamp(1.0, crate::edit::MAX_SHAPE_SIZE) / now }
    };
    let (fx, fy) = (factor(size.x, max.x - min.x, layer_size.x), factor(size.y, max.y - min.y, layer_size.y));
    if fx == 1.0 && fy == 1.0 {
        return false;
    }
    let resized = path.mapped(|point| Vec2::new(middle.x + (point.x - middle.x) * fx, middle.y + (point.y - middle.y) * fy));
    let Some(morph) = layer.morph.as_mut() else { return false };
    morph.path.set_at(frame, resized);
    true
}

/// Keeps a morph's style keyframes in step with an edit to the layer's own style. Each part
/// of the style that the edit changed (fill, stroke color, stroke width, corners) is set the
/// same on every keyframe; the parts it left alone keep morphing. Called after any shape style
/// edit (see `edit::edit_shape`).
pub fn sync_style_edit(layer: &mut Layer) {
    let LayerContent::Shape { style, .. } = &layer.content else { return };
    let edited = *style;
    let Some(morph) = layer.morph.as_mut().filter(|morph| morph.style.is_animated()) else { return };
    let before = morph.style.value;
    if before == edited {
        return;
    }
    let stroke_paint = |style: &ShapeStyle| style.stroke.map(|stroke| stroke.paint);
    let stroke_width = |style: &ShapeStyle| style.stroke.map(|stroke| stroke.width);
    morph.style.map_values(|mut keyed| {
        if edited.fill != before.fill {
            keyed.fill = edited.fill;
        }
        match (edited.stroke, before.stroke.is_some()) {
            // The stroke turned off, or on: the same everywhere.
            (None, true) | (Some(_), false) => keyed.stroke = edited.stroke,
            (Some(stroke), true) => {
                let mut keyed_stroke = keyed.stroke.unwrap_or(stroke);
                if stroke_paint(&edited) != stroke_paint(&before) {
                    keyed_stroke.paint = stroke.paint;
                }
                if stroke_width(&edited) != stroke_width(&before) {
                    keyed_stroke.width = stroke.width;
                }
                keyed.stroke = Some(keyed_stroke);
            }
            (None, false) => {}
        }
        if edited.corner_radius != before.corner_radius {
            keyed.corner_radius = edited.corner_radius;
        }
        if edited.points != before.points {
            keyed.points = edited.points;
        }
        if edited.inner_radius != before.inner_radius {
            keyed.inner_radius = edited.inner_radius;
        }
        keyed
    });
    // `map_values` changed the mirror too; it should be exactly the edited style.
    morph.style.value = edited;
}

/// Why a Morph To could not be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MorphError {
    /// Both layers must be shapes (drawn shapes or pen paths) with a transform.
    NotShapes,
    /// A layer cannot morph into itself.
    SameLayer,
    /// The layer that would change is locked.
    Locked,
    /// There is no room after `start` in the composition for the morph to run.
    NoRoom,
}

impl std::fmt::Display for MorphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotShapes => "Morph To needs two shape layers.",
            Self::SameLayer => "Pick a different layer to morph into.",
            Self::Locked => "The layer is locked.",
            Self::NoRoom => "There is no time left in the composition after the playhead.",
        })
    }
}

/// Whether `from` can be morphed into `to`: two different shape layers with transforms, the
/// first one not locked.
pub fn can_morph(composition: &Composition, from: Id, to: Id) -> Result<(), MorphError> {
    let shape = |id: Id| composition.layer(id).filter(|layer| matches!(layer.content, LayerContent::Shape { .. }) && layer.transform.is_some());
    let (Some(source), Some(_)) = (shape(from), shape(to)) else { return Err(MorphError::NotShapes) };
    if from == to {
        return Err(MorphError::SameLayer);
    }
    if source.switches.locked {
        return Err(MorphError::Locked);
    }
    Ok(())
}

/// Morph To: keyframes `from` to turn into `to` between `start` and `start + frames` (cut short
/// at the end of the composition). The outline morphs into `to`'s, the fill and stroke into its
/// colors and widths, and position, scale, rotation and opacity into its values, all with
/// `easing`. `to` is then hidden, since `from` ends up looking just like it. A drawn box shape
/// becomes a pen path first. Returns the frame the morph ends on.
///
/// Both layers' transforms are read in their own parent's space, so layers with different
/// parents may not meet exactly.
pub fn morph_to(composition: &mut Composition, from: Id, to: Id, start: i64, frames: i64, easing: Easing) -> Result<i64, MorphError> {
    can_morph(composition, from, to)?;
    let end = (start + frames.clamp(1, MAX_MORPH_FRAMES)).min(composition.duration_frames - 1);
    if end <= start {
        return Err(MorphError::NoRoom);
    }
    let (start_at, end_at) = (start as f64, end as f64);

    // What `to` looks like when the morph ends: its outline and style, in its own box.
    let target = composition.layer(to).ok_or(MorphError::NotShapes)?;
    let target_style = target.shape_style_at(end_at).ok_or(MorphError::NotShapes)?;
    let target_path = match target.shape_path_at(end_at) {
        Some(path) => path.into_owned(),
        None => outline(&target_style, target.size).ok_or(MorphError::NotShapes)?,
    };
    let target_size = target.size;
    let target_transform = target.transform.clone().ok_or(MorphError::NotShapes)?;

    if !convert_to_path(composition, from) {
        return Err(MorphError::NotShapes);
    }
    let layer = composition.layer_mut(from).ok_or(MorphError::NotShapes)?;
    let size = layer.size;
    let start_path = layer.shape_path_at(start_at).ok_or(MorphError::NotShapes)?.into_owned();
    let start_style = layer.shape_style_at(start_at).ok_or(MorphError::NotShapes)?;
    let LayerContent::Shape { style: own_style, path: own_path } = &layer.content else { return Err(MorphError::NotShapes) };
    let (own_style, own_path) = (*own_style, own_path.clone().ok_or(MorphError::NotShapes)?);
    let transform = layer.transform.as_mut().ok_or(MorphError::NotShapes)?;

    // `to`'s outline in `from`'s box. At the end `from` takes `to`'s position, scale and
    // rotation, so a point must sit as far from `from`'s anchor as it does from `to`'s.
    let (own_anchor, target_anchor) = (transform.anchor_point.value_at(end_at), target_transform.anchor_point.value_at(end_at));
    let into_own_box = |fraction: Vec2| {
        let pixels = Vec2::new(fraction.x * target_size.x, fraction.y * target_size.y) - target_anchor + own_anchor;
        Vec2::new(pixels.x / size.x.max(1e-4), pixels.y / size.y.max(1e-4))
    };
    let end_path = target_path.mapped(into_own_box);
    // Gradients are placed in fractions of the box too.
    let mut end_style = ShapeStyle { kind: ShapeKind::Path, corner_radius: 0.0, ..target_style };
    for paint in end_style.fill.iter_mut().chain(end_style.stroke.as_mut().map(|stroke| &mut stroke.paint)) {
        if let Some(gradient) = paint.gradient() {
            let mut placed = gradient;
            (placed.from, placed.to) = (into_own_box(gradient.from), into_own_box(gradient.to));
            *paint = paint.with_gradient(placed);
        }
    }

    // Two keyframes for each property, eased from the first.
    fn key<T: Blend>(animated: &mut Animated<T>, (start, from): (i64, T), (end, to): (i64, T), easing: Easing) {
        if animated.is_animated() {
            animated.set_at(start, from);
            animated.set_at(end, to);
        } else {
            *animated = Animated::keyframed(vec![Keyframe::linear(start, from), Keyframe::linear(end, to)]);
        }
        animated.set_easing(start, easing);
    }
    let position = (transform.position.value_at(start_at), target_transform.position.value_at(end_at));
    let scale = (transform.scale.value_at(start_at), target_transform.scale.value_at(end_at));
    let rotation = (transform.rotation.value_at(start_at), target_transform.rotation.value_at(end_at));
    let opacity = (transform.opacity.value_at(start_at), target_transform.opacity.value_at(end_at));
    key(&mut transform.position, (start, position.0), (end, position.1), easing);
    key(&mut transform.scale, (start, scale.0), (end, scale.1), easing);
    key(&mut transform.rotation, (start, rotation.0), (end, rotation.1), easing);
    key(&mut transform.opacity, (start, opacity.0), (end, opacity.1), easing);

    let morph = layer.morph.get_or_insert_with(|| still_morph(own_path, own_style));
    key(&mut morph.path, (start, start_path), (end, end_path), easing);
    key(&mut morph.style, (start, start_style), (end, end_style), easing);
    // The style's static value mirrors the layer's own style (see `sync_style_edit`).
    morph.style.value = own_style;
    // The layer must still be showing when the morph ends.
    layer.out_frame = layer.out_frame.max(end + 1);

    if let Some(target) = composition.layer_mut(to) {
        target.switches.visible = false;
    }
    Ok(end)
}

/// Morph To's default easing: After Effects' Easy Ease.
pub fn default_easing() -> Easing {
    Easing::Curve(EASY_EASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Paint;

    fn close(a: Vec2, b: Vec2, within: f32) -> bool {
        (a.x - b.x).abs() < within && (a.y - b.y).abs() < within
    }

    fn square() -> ShapePath {
        let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(x, y)| PathPoint::corner(Vec2::new(x, y)));
        ShapePath { points: corners.to_vec(), closed: true, ..Default::default() }
    }

    fn circle() -> ShapePath {
        outline(&ShapeStyle::new(ShapeKind::Ellipse), Vec2::new(100.0, 100.0)).unwrap()
    }

    /// Points sampled densely along a path, to compare two outlines' shapes.
    fn trace(path: &ShapePath) -> Vec<Vec2> {
        path.segments().into_iter().flat_map(|segment| (0..=200).map(move |step| point_on(segment, step as f32 / 200.0))).collect()
    }

    /// Whether every point of `a` lies on (within `within` of) the outline traced from `b`.
    fn same_outline(a: &ShapePath, b: &ShapePath, within: f32) -> bool {
        let traced = trace(b);
        trace(a).iter().all(|point| traced.iter().any(|other| (*point - *other).length() < within))
    }

    #[test]
    fn outlines_at_the_point_limit_morph_without_crashing() {
        // A closed outline with the most points a path may have, morphed into an open line:
        // opening it adds a point, past what resampling can match.
        let points = (0..MAX_PATH_POINTS).map(|index| {
            let angle = index as f32 / MAX_PATH_POINTS as f32 * std::f32::consts::TAU;
            PathPoint { at: Vec2::new(angle.cos(), angle.sin()), handle_in: Vec2::default(), handle_out: Vec2::default() }
        });
        let circle = ShapePath { points: points.collect(), closed: true, ..Default::default() };
        let corner = |at: Vec2| PathPoint { at, handle_in: Vec2::default(), handle_out: Vec2::default() };
        let line = ShapePath { points: vec![corner(Vec2::new(0.0, 0.0)), corner(Vec2::new(1.0, 1.0))], closed: false, ..Default::default() };
        assert!(circle.is_sound() && line.is_sound());
        let halfway = morph(&circle, &line, 0.5);
        assert!(halfway.is_sound());
    }

    #[test]
    fn resampling_adds_points_without_changing_the_shape() {
        let resampled = resample(&circle(), 50);
        assert_eq!(resampled.points.len(), 50);
        assert!(resampled.closed);
        assert!(same_outline(&resampled, &circle(), 0.01) && same_outline(&circle(), &resampled, 0.01));
        // A square keeps its corners exactly, and the new points spread evenly: 3 more on each side.
        let square_16 = resample(&square(), 16);
        for corner in square().points {
            assert!(square_16.points.iter().any(|point| close(point.at, corner.at, 1e-5)), "corner {:?} kept", corner.at);
        }
        let on_top: Vec<f32> = square_16.points.iter().filter(|point| point.at.y.abs() < 1e-5).map(|point| point.at.x).collect();
        assert_eq!(on_top.len(), 5, "the top edge's two corners and three new points");
        // An open path keeps its two ends.
        let line = ShapePath { points: vec![PathPoint::corner(Vec2::new(0.0, 0.0)), PathPoint::corner(Vec2::new(10.0, 0.0))], closed: false, ..Default::default() };
        let line_5 = resample(&line, 5);
        // (Evenly spaced along the line, to within the precision lengths are measured to.)
        for (point, x) in line_5.points.iter().zip([0.0, 2.5, 5.0, 7.5, 10.0]) {
            assert!(close(point.at, Vec2::new(x, 0.0), 0.01), "{:?} near {x}", point.at);
        }
        // Asking for fewer points than there are changes nothing.
        assert_eq!(resample(&square(), 3), square());
    }

    #[test]
    fn matching_gives_equal_counts_and_the_shortest_travel() {
        let (a, b) = matched(&square(), &circle());
        assert_eq!(a.points.len(), b.points.len());
        assert_eq!(a.points.len(), MORPH_POINTS);
        assert!(same_outline(&b, &circle(), 0.01), "the circle keeps its shape");

        // The same square started at a different corner and drawn the other way round lines up
        // with the original exactly: nothing has to move.
        let mut turned = reversed(&square());
        turned.points.rotate_left(2);
        let (a, b) = matched(&square(), &turned);
        for (x, y) in a.points.iter().zip(&b.points) {
            assert!(close(x.at, y.at, 1e-5), "{:?} vs {:?}", x.at, y.at);
        }
        // An open line drawn backwards is turned round.
        let line = ShapePath { points: vec![PathPoint::corner(Vec2::new(0.0, 0.0)), PathPoint::corner(Vec2::new(10.0, 0.0))], closed: false, ..Default::default() };
        let (a, b) = matched(&line, &reversed(&line));
        assert!(close(a.points[0].at, b.points[0].at, 1e-5));
    }

    #[test]
    fn halfway_through_a_morph_the_shape_is_in_between() {
        // A small centered square (corners at 0.25 and 0.75 of the box) morphing into the box itself.
        let small = square().mapped(|v| Vec2::new(0.25 + v.x * 0.5, 0.25 + v.y * 0.5));
        let halfway = morph(&small, &square(), 0.5);
        let (min, max) = halfway.bounds().unwrap();
        assert!(close(min, Vec2::new(0.125, 0.125), 1e-4) && close(max, Vec2::new(0.875, 0.875), 1e-4), "{min:?} {max:?}");
        // Square to circle: halfway, the corner has come in toward the circle but not all the way.
        let box_square = square().mapped(|v| v * 100.0).mapped(|v| Vec2::new(v.x / 100.0, v.y / 100.0));
        let halfway = morph(&box_square, &circle(), 0.5);
        let corner_reach = halfway.points.iter().map(|point| (point.at - Vec2::new(0.5, 0.5)).length()).fold(0.0, f32::max);
        let circle_radius = 0.5;
        let square_corner = 0.5 * 2f32.sqrt();
        assert!(corner_reach > circle_radius + 0.01 && corner_reach < square_corner - 0.01, "{corner_reach}");
        // The ends are exactly the keyframes' outlines.
        assert_eq!(morph(&box_square, &circle(), 0.0), box_square);
        assert_eq!(morph(&box_square, &circle(), 1.0), circle());
    }

    #[test]
    fn closed_into_open_opens_halfway() {
        let line = ShapePath { points: vec![PathPoint::corner(Vec2::new(0.0, 0.5)), PathPoint::corner(Vec2::new(1.0, 0.5))], closed: false, ..Default::default() };
        assert!(morph(&square(), &line, 0.25).closed);
        let late = morph(&square(), &line, 0.75);
        assert!(!late.closed && late.is_sound());
    }

    #[test]
    fn box_shapes_become_matching_paths() {
        let size = Vec2::new(200.0, 100.0);
        for kind in ShapeKind::ALL {
            let mut style = ShapeStyle::new(kind);
            for radius in [0.0, 20.0] {
                style.corner_radius = radius;
                let path = outline(&style, size).unwrap();
                assert!(path.is_sound() && path.closed, "{kind:?}");
                let (min, max) = path.bounds().unwrap();
                // Every shape stays inside its box; with sharp corners (or no corners), it reaches
                // the top. A rounded tip sits lower, as the renderer draws it.
                assert!(min.x >= -1e-4 && min.y >= -1e-4 && max.x <= 1.0001 && max.y <= 1.0001, "{kind:?} r{radius}: {min:?} {max:?}");
                if radius == 0.0 || matches!(kind, ShapeKind::Rectangle | ShapeKind::Ellipse) {
                    assert!(min.y.abs() < 1e-3, "{kind:?} r{radius} reaches the top: {min:?}");
                }
            }
        }
        // A rounded rectangle has quarter circles in its corners: the curve passes the corner at
        // radius·(1 - 1/√2) in from each side, as a rounded corner does.
        let style = ShapeStyle { corner_radius: 20.0, ..ShapeStyle::new(ShapeKind::Rectangle) };
        let path = outline(&style, size).unwrap().mapped(|v| Vec2::new(v.x * size.x, v.y * size.y));
        let inset = 20.0 * (1.0 - std::f32::consts::FRAC_1_SQRT_2);
        assert!(trace(&path).iter().any(|point| close(*point, Vec2::new(inset, inset), 0.3)), "rounded top-left corner");
        // Squared corners stay square.
        let sharp = outline(&ShapeStyle::new(ShapeKind::Rectangle), size).unwrap();
        assert_eq!(sharp.points.len(), 4);
        assert!(close(sharp.points[0].at, Vec2::new(0.0, 0.0), 1e-6));
        assert!(outline(&ShapeStyle::new(ShapeKind::Path), size).is_none());
    }

    #[test]
    fn a_pill_and_a_fully_rounded_square_keep_their_shape() {
        // A pill: the corner radius is half the height, so the sides have no straight part left.
        let size = Vec2::new(300.0, 100.0);
        let style = ShapeStyle { corner_radius: 50.0, ..ShapeStyle::new(ShapeKind::Rectangle) };
        let pill = outline(&style, size).unwrap();
        assert!(pill.is_sound() && pill.closed);
        let (min, max) = pill.bounds().unwrap();
        assert!(close(min, Vec2::new(0.0, 0.0), 1e-3) && close(max, Vec2::new(1.0, 1.0), 1e-3), "fills its box, not a circle in the middle: {min:?} {max:?}");
        let in_pixels = pill.mapped(|v| Vec2::new(v.x * size.x, v.y * size.y));
        // Its ends are half circles around (50, 50) and (250, 50).
        for point in trace(&in_pixels) {
            let center = Vec2::new(point.x.clamp(50.0, 250.0), 50.0);
            assert!(((point - center).length() - 50.0).abs() < 0.5, "{point:?} is on the pill's outline");
        }
        // Where two quarter arcs meet with no side between them they make one smooth point.
        assert_eq!(pill.points.len(), 6);
        // A square rounded all the way is a circle of four smooth points.
        let square = Vec2::new(100.0, 100.0);
        let round = outline(&style, square).unwrap();
        assert_eq!(round.points.len(), 4);
        assert!(same_outline(&round, &circle(), 0.01) && same_outline(&circle(), &round, 0.01));
    }

    #[test]
    fn styles_blend_colors_and_fade_missing_paint() {
        let red = ShapeStyle { fill: Some(Paint::Solid(Rgba::hex(0xff0000))), ..ShapeStyle::new(ShapeKind::Path) };
        let blue = ShapeStyle { fill: Some(Paint::Solid(Rgba::hex(0x0000ff))), stroke: Some(Stroke { paint: Paint::Solid(Rgba::hex(0xffffff)), width: 10.0 }), ..red };
        let halfway = red.lerp(blue, 0.5);
        let Some(Paint::Solid(fill)) = halfway.fill else { panic!("a solid fill") };
        assert!((fill.r - 0.5).abs() < 1e-5 && (fill.b - 0.5).abs() < 1e-5);
        let stroke = halfway.stroke.expect("the stroke fades in");
        assert_eq!(stroke.width, 10.0);
        assert!((stroke.paint.colors()[0].a - 0.5).abs() < 1e-5);
        // A solid color into a gradient passes through gradients of the two.
        let gradient = Paint::linear(Rgba::hex(0x000000), Rgba::hex(0xffffff), 0.0);
        let mixed = Paint::Solid(Rgba::hex(0xff0000)).lerp(gradient, 0.5);
        assert!(matches!(mixed, Paint::Linear(_)));
        assert_eq!(mixed.colors().len(), 2);
    }

    fn sample_with_shapes() -> (Composition, Id, Id) {
        let mut project = crate::sample::sample_project();
        let comp = project.compositions[0].id;
        let square = crate::edit::add_shape_layer(&mut project, comp, ShapeStyle::new(ShapeKind::Rectangle), Vec2::new(100.0, 100.0), Vec2::new(300.0, 300.0), 0).unwrap();
        let star = crate::edit::add_shape_layer(&mut project, comp, ShapeStyle::new(ShapeKind::Star), Vec2::new(200.0, 200.0), Vec2::new(900.0, 500.0), 0).unwrap();
        let composition = project.composition_mut(comp).unwrap();
        crate::edit::edit_shape(composition, star, |style| style.fill = Some(Paint::Solid(Rgba::hex(0x00ff00))));
        composition.layer_mut(star).unwrap().transform.as_mut().unwrap().rotation = Animated::constant(30.0);
        (composition.clone(), square, star)
    }

    #[test]
    fn morph_to_keyframes_the_first_layer_into_the_second() {
        let (mut composition, square, star) = sample_with_shapes();
        let end = morph_to(&mut composition, square, star, 10, 20, default_easing()).unwrap();
        assert_eq!(end, 30);
        let layer = composition.layer(square).unwrap();
        assert!(layer.is_morphing());
        assert!(!composition.layer(star).unwrap().switches.visible, "the target is hidden");
        let transform = layer.transform.as_ref().unwrap();
        assert_eq!(transform.position.value_at(30.0), Vec2::new(900.0, 500.0));
        assert_eq!(transform.rotation.value_at(30.0), 30.0);
        assert_eq!(transform.position.value_at(10.0), Vec2::new(300.0, 300.0));
        // At the end the square's outline, in composition pixels, is the star's.
        let place = |layer: &Layer, frame: f64| {
            let world = crate::transform::world_transform(&composition, layer.id, frame).unwrap();
            let size = layer.size;
            let path = layer.shape_path_at(frame).map(Cow::into_owned).unwrap_or_else(|| outline(&layer.shape_style_at(frame).unwrap(), size).unwrap());
            path.mapped(|v| world.apply(Vec2::new(v.x * size.x, v.y * size.y)))
        };
        let morphed = place(layer, 30.0);
        let target = place(composition.layer(star).unwrap(), 30.0);
        assert!(same_outline(&morphed, &target, 0.5) && same_outline(&target, &morphed, 0.5), "lands on the star");
        // And its fill has turned the star's green, passing through a blend on the way.
        let Some(Paint::Solid(end_fill)) = layer.shape_style_at(30.0).unwrap().fill else { panic!() };
        assert_eq!(end_fill.g, 1.0);
        let Some(Paint::Solid(middle_fill)) = layer.shape_style_at(20.0).unwrap().fill else { panic!() };
        assert!(middle_fill.g > 0.0 && middle_fill.g < 1.0);
        // The starting outline is still the square.
        let start = place(layer, 10.0);
        let (min, max) = start.bounds().unwrap();
        assert!(close(min, Vec2::new(250.0, 250.0), 0.01) && close(max, Vec2::new(350.0, 350.0), 0.01), "{min:?} {max:?}");
    }

    #[test]
    fn morph_to_refuses_what_it_cannot_do() {
        let (mut composition, square, star) = sample_with_shapes();
        assert_eq!(can_morph(&composition, square, square), Err(MorphError::SameLayer));
        let footage = composition.layers.iter().find(|layer| !matches!(layer.content, LayerContent::Shape { .. })).unwrap().id;
        assert_eq!(can_morph(&composition, square, footage), Err(MorphError::NotShapes));
        let last = composition.duration_frames - 1;
        assert_eq!(morph_to(&mut composition, square, star, last, 10, default_easing()), Err(MorphError::NoRoom));
        composition.layer_mut(square).unwrap().switches.locked = true;
        assert_eq!(can_morph(&composition, square, star), Err(MorphError::Locked));
    }

    #[test]
    fn the_layers_rectangle_follows_a_keyframed_outline_past_its_box() {
        use crate::transform::{layer_box, layer_contains};
        let (mut composition, square, _) = sample_with_shapes();
        assert!(toggle_path_animation(&mut composition, square, 0));
        // Pull a point well outside the box at a later keyframe.
        assert!(move_morph_point(&mut composition, square, 20, 0, PathPart::Point, Vec2::new(-50.0, -30.0)));
        let layer = composition.layer(square).unwrap();
        assert_eq!(layer.size, Vec2::new(100.0, 100.0), "the layer's own box is untouched");
        let (min, max) = layer_box(layer, 20.0);
        assert!(min.x <= -50.0 + 1e-3 && min.y <= -30.0 + 1e-3, "the rectangle reaches the moved point: {min:?}");
        assert!(close(max, Vec2::new(100.0, 100.0), 1e-3));
        // At the first keyframe the outline, and so the rectangle, is as it was.
        let (min, max) = layer_box(layer, 0.0);
        assert!(close(min, Vec2::new(0.0, 0.0), 1e-3) && close(max, Vec2::new(100.0, 100.0), 1e-3));
        // A click on the part past the box lands on the layer only where the outline reaches.
        let world = crate::transform::world_transform(&composition, square, 20.0).unwrap();
        let outside = world.apply(Vec2::new(-40.0, -25.0));
        assert!(layer_contains(&composition, square, outside, 20.0));
        assert!(!layer_contains(&composition, square, outside, 0.0));
    }

    #[test]
    fn size_is_animated_by_resizing_the_outline_at_a_keyframe() {
        let (mut composition, square, _) = sample_with_shapes();
        // A shape that isn't animated has no outline keyframes to resize.
        assert!(!set_morph_size(&mut composition, square, 20, Vec2::new(200.0, 50.0)));
        assert!(toggle_path_animation(&mut composition, square, 0));
        assert!(set_morph_size(&mut composition, square, 20, Vec2::new(200.0, 50.0)));
        let layer = composition.layer(square).unwrap();
        assert_eq!(layer.morph.as_ref().unwrap().path.keyframe_frames(), vec![0, 20]);
        let size_at = |frame: f64| layer.shape_size_at(frame);
        assert!(close(size_at(0.0), Vec2::new(100.0, 100.0), 1e-3), "the first keyframe keeps its size");
        assert!(close(size_at(20.0), Vec2::new(200.0, 50.0), 1e-3));
        assert!(close(size_at(10.0), Vec2::new(150.0, 75.0), 1e-3), "and the size blends between them");
        // It grows about its own middle, so the shape stays where it is.
        let (min, max) = layer.shape_path_at(20.0).unwrap().bounds().unwrap();
        assert!(close((min + max) * 0.5, Vec2::new(0.5, 0.5), 1e-5));
        assert_eq!(layer.size, Vec2::new(100.0, 100.0), "the layer's own box is untouched");
        // The same size again changes nothing, and nonsense is refused.
        assert!(!set_morph_size(&mut composition, square, 20, Vec2::new(200.0, 50.0)));
        assert!(!set_morph_size(&mut composition, square, 20, Vec2::new(f32::NAN, f32::NAN)));
    }

    #[test]
    fn the_path_stopwatch_and_keyframes() {
        let (mut composition, square, _) = sample_with_shapes();
        assert!(toggle_path_animation(&mut composition, square, 0));
        let layer = composition.layer(square).unwrap();
        assert!(layer.is_morphing());
        assert!(matches!(&layer.content, LayerContent::Shape { style, path: Some(_) } if style.kind == ShapeKind::Path), "now a path");
        // Moving a point at frame 20 keyframes it there; frame 0 keeps the square.
        assert!(move_morph_point(&mut composition, square, 20, 0, PathPart::Point, Vec2::new(-50.0, -50.0)));
        let layer = composition.layer(square).unwrap();
        assert_eq!(layer.morph.as_ref().unwrap().path.keyframe_frames(), vec![0, 20]);
        assert!(close(layer.shape_path_at(20.0).unwrap().points[0].at, Vec2::new(-0.5, -0.5), 1e-5));
        assert!(close(layer.shape_path_at(0.0).unwrap().points[0].at, Vec2::new(0.0, 0.0), 1e-5));
        // Removing both keyframes stops the morph, keeping the last outline.
        assert!(toggle_path_keyframe(&mut composition, square, 0));
        assert!(toggle_path_keyframe(&mut composition, square, 20));
        let layer = composition.layer(square).unwrap();
        assert!(layer.morph.is_none());
        assert!(matches!(&layer.content, LayerContent::Shape { path: Some(path), .. } if close(path.points[0].at, Vec2::new(-0.5, -0.5), 1e-5)));
    }

    #[test]
    fn style_edits_apply_across_the_morph() {
        let (mut composition, square, star) = sample_with_shapes();
        morph_to(&mut composition, square, star, 0, 20, Easing::Linear).unwrap();
        // Turning a stroke on gives every keyframe that stroke; the fill keeps morphing.
        let stroke = Stroke { paint: Paint::Solid(Rgba::hex(0x123456)), width: 6.0 };
        crate::edit::edit_shape(&mut composition, square, |style| style.stroke = Some(stroke));
        let layer = composition.layer(square).unwrap();
        for frame in [0.0, 10.0, 20.0] {
            assert_eq!(layer.shape_style_at(frame).unwrap().stroke, Some(stroke));
        }
        assert_ne!(layer.shape_style_at(0.0).unwrap().fill, layer.shape_style_at(20.0).unwrap().fill, "the fill still morphs");
    }

    #[test]
    fn opening_a_file_checks_its_morphs() {
        let mut project = crate::sample::sample_project();
        let comp = project.compositions[0].id;
        let square = crate::edit::add_shape_layer(&mut project, comp, ShapeStyle::new(ShapeKind::Rectangle), Vec2::new(100.0, 100.0), Vec2::new(300.0, 300.0), 0).unwrap();
        let star = crate::edit::add_shape_layer(&mut project, comp, ShapeStyle::new(ShapeKind::Star), Vec2::new(200.0, 200.0), Vec2::new(900.0, 500.0), 0).unwrap();
        morph_to(project.composition_mut(comp).unwrap(), square, star, 0, 20, default_easing()).unwrap();
        assert_eq!(crate::project_file::check(&project), Ok(()));
        // A keyframe outline with one point can't be drawn.
        let mut broken = project.clone();
        let morph = broken.composition_mut(comp).unwrap().layer_mut(square).unwrap().morph.as_mut().unwrap();
        morph.path.set_at(20, ShapePath { points: vec![PathPoint::default()], closed: true, ..Default::default() });
        assert!(crate::project_file::check(&broken).unwrap_err().contains("broken morph"));
        // Nor can a drawn box shape carry one.
        let mut misplaced = project.clone();
        let layer = misplaced.composition_mut(comp).unwrap().layer_mut(square).unwrap();
        layer.content = LayerContent::Shape { style: ShapeStyle::new(ShapeKind::Rectangle), path: None };
        assert!(crate::project_file::check(&misplaced).is_err());
    }

    #[test]
    fn morph_keyframes_survive_saving() {
        let (mut composition, square, star) = sample_with_shapes();
        morph_to(&mut composition, square, star, 0, 20, default_easing()).unwrap();
        let layer = composition.layer(square).unwrap();
        let saved = serde_json::to_string(layer).unwrap();
        let loaded: Layer = serde_json::from_str(&saved).unwrap();
        assert_eq!(&loaded, layer);
    }
}
