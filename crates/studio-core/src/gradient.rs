//! Gradients: color stops along a line laid over a layer's box, as in After Effects' and
//! Premiere's gradient editors. A gradient holds two to `MAX_STOPS` stops, kept in order of
//! their location, and two points in fractions of the box (0,0 top left, 1,1 bottom right;
//! they may lie outside it). A linear gradient runs from `from` to `to`; a radial one is
//! centered on `from` and reaches `to`.
//!
//! Gradients are `Copy` (like the rest of `Paint`), so the stops live in a fixed-size array.

use crate::animation::Vec2;
use crate::model::Rgba;

/// The most color stops a gradient can hold.
pub const MAX_STOPS: usize = 8;

/// One color at a place along a gradient.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GradientStop {
    /// Where along the gradient, 0 (its start) to 1 (its end).
    pub at: f32,
    pub color: Rgba,
}

/// Color stops and where the gradient runs. See the module notes.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(from = "GradientFile", into = "GradientFile")]
pub struct Gradient {
    stops: [GradientStop; MAX_STOPS],
    count: usize,
    pub from: Vec2,
    pub to: Vec2,
}

/// How a gradient is saved: its stops as a plain list.
#[derive(serde::Serialize, serde::Deserialize)]
struct GradientFile {
    stops: Vec<GradientStop>,
    from: Vec2,
    to: Vec2,
}

impl From<GradientFile> for Gradient {
    fn from(file: GradientFile) -> Self {
        Self::new(&file.stops, file.from, file.to)
    }
}

impl From<Gradient> for GradientFile {
    fn from(gradient: Gradient) -> Self {
        Self { stops: gradient.stops().to_vec(), from: gradient.from, to: gradient.to }
    }
}

/// Where a linear gradient starts and ends by default: across the box, left to right.
pub const LINEAR_FROM: Vec2 = Vec2 { x: 0.0, y: 0.5 };
pub const LINEAR_TO: Vec2 = Vec2 { x: 1.0, y: 0.5 };
/// A radial gradient's default center and edge: from the middle out to the box's sides.
pub const RADIAL_FROM: Vec2 = Vec2 { x: 0.5, y: 0.5 };
pub const RADIAL_TO: Vec2 = Vec2 { x: 1.0, y: 0.5 };

impl Gradient {
    /// A gradient from `stops` (sorted here, locations kept within 0–1, at most `MAX_STOPS`
    /// used). Fewer than two stops are padded out so there is always a start and an end.
    pub fn new(stops: &[GradientStop], from: Vec2, to: Vec2) -> Self {
        let fallback = GradientStop { at: 0.0, color: Rgba::hex(0x000000) };
        let mut gradient = Self { stops: [fallback; MAX_STOPS], count: 0, from: finite_or(from, LINEAR_FROM), to: finite_or(to, LINEAR_TO) };
        for stop in stops.iter().take(MAX_STOPS) {
            gradient.stops[gradient.count] = GradientStop { at: clamp_location(stop.at), color: stop.color };
            gradient.count += 1;
        }
        // Always a start and an end.
        while gradient.count < 2 {
            let color = gradient.stops().last().map_or(fallback.color, |stop| stop.color);
            gradient.stops[gradient.count] = GradientStop { at: if gradient.count == 0 { 0.0 } else { 1.0 }, color };
            gradient.count += 1;
        }
        gradient.sort();
        gradient
    }

    /// Two colors, from one end to the other, between `from` and `to`.
    pub fn two(start: Rgba, end: Rgba, from: Vec2, to: Vec2) -> Self {
        Self::new(&[GradientStop { at: 0.0, color: start }, GradientStop { at: 1.0, color: end }], from, to)
    }

    /// A two-color linear gradient running across the box at `angle` degrees (0 left to right,
    /// 90 top to bottom), from edge to edge. Exact for the four straight directions; diagonal
    /// ones run corner to corner.
    pub fn linear_at_angle(start: Rgba, end: Rgba, angle: f32) -> Self {
        let (sin, cos) = angle.to_radians().sin_cos();
        // How far from the center the gradient reaches, in fractions of a square box.
        let reach = (cos.abs() + sin.abs()) * 0.5;
        let from = Vec2::new(0.5 - cos * reach, 0.5 - sin * reach);
        let to = Vec2::new(0.5 + cos * reach, 0.5 + sin * reach);
        Self::two(start, end, from, to)
    }

    /// Whether its points are numbers the renderer can use (stops are always kept in range).
    pub fn is_sound(&self) -> bool {
        [self.from.x, self.from.y, self.to.x, self.to.y].iter().all(|value| value.is_finite())
    }

    /// The same gradient with any unusable point put back where gradients start out.
    pub fn sanitized(self) -> Self {
        Self { from: finite_or(self.from, LINEAR_FROM), to: finite_or(self.to, LINEAR_TO), ..self }
    }

    /// The stops, in order of location.
    pub fn stops(&self) -> &[GradientStop] {
        &self.stops[..self.count]
    }

    /// The stops' colors, to change in place (their order and locations stay).
    pub fn colors_mut(&mut self) -> impl Iterator<Item = &mut Rgba> {
        self.stops[..self.count].iter_mut().map(|stop| &mut stop.color)
    }

    /// The gradient with stop `index`'s color replaced.
    pub fn with_color(mut self, index: usize, color: Rgba) -> Self {
        if let Some(stop) = self.stops[..self.count].get_mut(index) {
            stop.color = color;
        }
        self
    }

    /// The color at `at` along the gradient (0–1), blended between the stops around it.
    pub fn color_at(&self, at: f32) -> Rgba {
        let stops = self.stops();
        let at = clamp_location(at);
        let first = stops[0];
        if at <= first.at {
            return first.color;
        }
        for pair in stops.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if at <= b.at {
                let span = (b.at - a.at).max(f32::EPSILON);
                return mix(a.color, b.color, (at - a.at) / span);
            }
        }
        stops[stops.len() - 1].color
    }

    /// Adds a stop at `at` in the color the gradient already has there. Returns its index, or
    /// `None` when the gradient is full.
    pub fn add_stop(&mut self, at: f32) -> Option<usize> {
        if self.count >= MAX_STOPS {
            return None;
        }
        let at = clamp_location(at);
        let color = self.color_at(at);
        self.stops[self.count] = GradientStop { at, color };
        self.count += 1;
        self.sort();
        // The new stop is the last one at that location after sorting (ties keep their order).
        self.stops().iter().rposition(|stop| stop.at == at && stop.color == color)
    }

    /// Removes stop `index`. A gradient keeps at least two stops. Returns whether it went.
    pub fn remove_stop(&mut self, index: usize) -> bool {
        if self.count <= 2 || index >= self.count {
            return false;
        }
        self.stops.copy_within(index + 1..self.count, index);
        self.count -= 1;
        true
    }

    /// Moves stop `index` to `at` (0–1). Stops stay in order, so it may pass others; returns
    /// its index afterwards.
    pub fn move_stop(&mut self, index: usize, at: f32) -> usize {
        let Some(stop) = self.stops[..self.count].get_mut(index) else { return index };
        stop.at = clamp_location(at);
        let moved = *stop;
        self.sort();
        // The same stop, found again: the nearest index holding its location and color.
        (0..self.count).filter(|&i| self.stops[i] == moved).min_by_key(|&i| i.abs_diff(index)).unwrap_or(index)
    }

    /// The direction from `from` to `to` on a box of `size` pixels, in degrees (0 left to
    /// right, 90 top to bottom), 0–360.
    pub fn angle(&self, size: Vec2) -> f32 {
        let (dx, dy) = ((self.to.x - self.from.x) * size.x, (self.to.y - self.from.y) * size.y);
        if dx == 0.0 && dy == 0.0 {
            return 0.0;
        }
        dy.atan2(dx).to_degrees().rem_euclid(360.0)
    }

    /// The gradient turned to `angle` degrees on a box of `size` pixels, about the middle of its
    /// line and keeping the line's length on screen.
    pub fn with_angle(self, angle: f32, size: Vec2) -> Self {
        if !angle.is_finite() || size.x <= 0.0 || size.y <= 0.0 {
            return self;
        }
        let middle = Vec2::new((self.from.x + self.to.x) * 0.5 * size.x, (self.from.y + self.to.y) * 0.5 * size.y);
        let length = ((self.to.x - self.from.x) * size.x).hypot((self.to.y - self.from.y) * size.y).max(1.0);
        let (sin, cos) = angle.to_radians().sin_cos();
        let half = Vec2::new(cos * length * 0.5, sin * length * 0.5);
        let to_fraction = |x: f32, y: f32| Vec2::new(x / size.x, y / size.y);
        Self { from: to_fraction(middle.x - half.x, middle.y - half.y), to: to_fraction(middle.x + half.x, middle.y + half.y), ..self }
    }

    /// The stops in the opposite order, so the gradient runs the other way.
    pub fn reversed(mut self) -> Self {
        for stop in &mut self.stops[..self.count] {
            stop.at = 1.0 - stop.at;
        }
        self.stops[..self.count].reverse();
        self
    }

    fn sort(&mut self) {
        // A stable sort, so stops at the same place keep their order.
        self.stops[..self.count].sort_by(|a, b| a.at.total_cmp(&b.at));
    }
}

fn clamp_location(at: f32) -> f32 {
    if at.is_finite() { at.clamp(0.0, 1.0) } else { 0.0 }
}

fn finite_or(point: Vec2, fallback: Vec2) -> Vec2 {
    if point.x.is_finite() && point.y.is_finite() { point } else { fallback }
}

/// Blends two straight-alpha colors, `t` of the way from `a` to `b`. The blend is a new color,
/// linked to no library color.
fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    Rgba { r: lerp(a.r, b.r), g: lerp(a.g, b.g), b: lerp(a.b, b.b), a: lerp(a.a, b.a), link: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop(at: f32, hex: u32) -> GradientStop {
        GradientStop { at, color: Rgba::hex(hex) }
    }

    #[test]
    fn stops_are_kept_in_order_and_padded_to_two() {
        let gradient = Gradient::new(&[stop(0.8, 0xff0000), stop(0.2, 0x00ff00)], LINEAR_FROM, LINEAR_TO);
        assert_eq!(gradient.stops().iter().map(|s| s.at).collect::<Vec<_>>(), vec![0.2, 0.8]);
        let single = Gradient::new(&[stop(0.3, 0xff0000)], LINEAR_FROM, LINEAR_TO);
        assert_eq!(single.stops().len(), 2);
    }

    #[test]
    fn adding_a_stop_takes_the_color_already_there() {
        let mut gradient = Gradient::two(Rgba::hex(0x000000), Rgba::hex(0xffffff), LINEAR_FROM, LINEAR_TO);
        let index = gradient.add_stop(0.5).unwrap();
        assert_eq!(index, 1, "between the two ends");
        let middle = gradient.stops()[1].color;
        assert!((middle.r - 0.5).abs() < 0.01, "half way to white: {middle:?}");
    }

    #[test]
    fn a_full_gradient_takes_no_more_stops_and_keeps_two() {
        let mut gradient = Gradient::two(Rgba::hex(0x000000), Rgba::hex(0xffffff), LINEAR_FROM, LINEAR_TO);
        for at in 1..=6 {
            assert!(gradient.add_stop(at as f32 / 10.0).is_some());
        }
        assert_eq!(gradient.add_stop(0.95), None, "eight is the most");
        while gradient.stops().len() > 2 {
            assert!(gradient.remove_stop(1));
        }
        assert!(!gradient.remove_stop(0), "the last two stay");
    }

    #[test]
    fn moving_a_stop_past_another_reorders_them() {
        let mut gradient = Gradient::new(&[stop(0.0, 0x000000), stop(0.3, 0xff0000), stop(1.0, 0xffffff)], LINEAR_FROM, LINEAR_TO);
        let index = gradient.move_stop(1, 1.0);
        assert_eq!(gradient.stops()[index].color, Rgba::hex(0xff0000));
        assert_eq!(gradient.stops()[index].at, 1.0);
        let index = gradient.move_stop(0, 0.6);
        assert_eq!(index, 0, "black is still first at 0.6 (red and white are at 1)");
    }

    #[test]
    fn angles_become_points_across_the_box() {
        let across = Gradient::linear_at_angle(Rgba::hex(0), Rgba::hex(0xffffff), 0.0);
        assert!((across.from.x - 0.0).abs() < 1e-5 && (across.to.x - 1.0).abs() < 1e-5 && (across.from.y - 0.5).abs() < 1e-5);
        let down = Gradient::linear_at_angle(Rgba::hex(0), Rgba::hex(0xffffff), 90.0);
        assert!((down.from.y - 0.0).abs() < 1e-5 && (down.to.y - 1.0).abs() < 1e-5 && (down.from.x - 0.5).abs() < 1e-5);
        let diagonal = Gradient::linear_at_angle(Rgba::hex(0), Rgba::hex(0xffffff), 45.0);
        assert!((diagonal.from.x).abs() < 1e-5 && (diagonal.to.y - 1.0).abs() < 1e-5, "corner to corner: {diagonal:?}");
    }

    #[test]
    fn turning_keeps_the_middle_and_length_on_screen() {
        let size = Vec2::new(200.0, 100.0);
        let gradient = Gradient::two(Rgba::hex(0), Rgba::hex(0xffffff), LINEAR_FROM, LINEAR_TO);
        assert!((gradient.angle(size) - 0.0).abs() < 1e-4);
        let turned = gradient.with_angle(90.0, size);
        assert!((turned.angle(size) - 90.0).abs() < 1e-3, "{}", turned.angle(size));
        // 200 pixels long, now vertical on a 100-pixel-tall box: from -50% to 150%.
        assert!((turned.from.x - 0.5).abs() < 1e-4 && (turned.from.y + 0.5).abs() < 1e-4 && (turned.to.y - 1.5).abs() < 1e-4, "{turned:?}");
    }

    #[test]
    fn reversing_swaps_the_ends() {
        let gradient = Gradient::new(&[stop(0.0, 0x000000), stop(0.25, 0xff0000), stop(1.0, 0xffffff)], LINEAR_FROM, LINEAR_TO).reversed();
        assert_eq!(gradient.stops()[0].color, Rgba::hex(0xffffff));
        assert_eq!(gradient.stops()[1].at, 0.75);
    }

    #[test]
    fn saved_gradients_load_back_the_same() {
        let gradient = Gradient::new(&[stop(0.0, 0x000000), stop(0.4, 0xff0000), stop(1.0, 0xffffff)], Vec2::new(0.1, 0.2), Vec2::new(0.9, 0.7));
        let json = serde_json::to_string(&gradient).unwrap();
        assert_eq!(serde_json::from_str::<Gradient>(&json).unwrap(), gradient);
    }
}
