//! Painted strokes, as used by a Roto Brush style tool: green strokes over what to keep, red
//! strokes over what to remove. A stroke is drawn onto a mask as a thick line with round ends, and
//! sampled into points for the model.

use crate::Mask;

/// A painted stroke. `points` are where the brush went, as fractions (0–1) of the picture's width
/// and height; `radius` is the brush's radius as a fraction of the picture's width. `add` strokes
/// mark what to keep, the others what to remove.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub points: Vec<(f32, f32)>,
    pub radius: f32,
    pub add: bool,
}

/// The smallest brush radius, in pixels, so a very thin stroke still covers the pixels it passes.
const MIN_RADIUS_PIXELS: f32 = 0.5;

impl Stroke {
    /// The stroke's path in pixels of a `width × height` grid (skipping any point that isn't a
    /// number), and its radius in those pixels.
    fn in_pixels(&self, width: u32, height: u32) -> (Vec<(f32, f32)>, f32) {
        let path = self.points.iter().filter(|(x, y)| x.is_finite() && y.is_finite()).map(|&(x, y)| (x * width as f32, y * height as f32)).collect();
        let radius = if self.radius.is_finite() { self.radius * width as f32 } else { 0.0 };
        (path, radius.max(MIN_RADIUS_PIXELS))
    }

    /// Points spaced about `spacing` pixels apart along the stroke (its first point, then one
    /// every `spacing` along its path), as fractions of a `width × height` picture.
    pub(crate) fn sample(&self, width: u32, height: u32, spacing: f32) -> Vec<(f32, f32)> {
        let (path, _) = self.in_pixels(width, height);
        let Some(&first) = path.first() else { return Vec::new() };
        let spacing = spacing.max(1.0);
        let mut samples = vec![first];
        // How far along the path the next sample is due, counted from the current segment's start.
        let mut next_at = spacing;
        for pair in path.windows(2) {
            let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
            let length = (x1 - x0).hypot(y1 - y0);
            while next_at <= length {
                let t = next_at / length;
                samples.push((x0 + (x1 - x0) * t, y0 + (y1 - y0) * t));
                next_at += spacing;
            }
            next_at -= length;
        }
        samples.into_iter().map(|(x, y)| ((x / width as f32).clamp(0.0, 1.0), (y / height as f32).clamp(0.0, 1.0))).collect()
    }

    /// Paints the stroke onto `mask`: under an add stroke pixels become kept, under a remove
    /// stroke removed, with a one-pixel soft rim so the stroke's edge isn't jagged.
    pub(crate) fn paint(&self, mask: &mut Mask) {
        let (width, height) = (mask.width as usize, mask.height as usize);
        let (path, radius) = self.in_pixels(mask.width, mask.height);
        if path.is_empty() || width == 0 || height == 0 {
            return;
        }
        // A single point is a segment of no length: a round dab.
        let segments: Vec<((f32, f32), (f32, f32))> = if path.len() == 1 { vec![(path[0], path[0])] } else { path.windows(2).map(|pair| (pair[0], pair[1])).collect() };
        for (start, end) in segments {
            // Only the pixels near this segment can be touched.
            let reach = radius + 1.0;
            let columns = pixel_span(start.0.min(end.0) - reach, start.0.max(end.0) + reach, width);
            let rows = pixel_span(start.1.min(end.1) - reach, start.1.max(end.1) + reach, height);
            for y in rows {
                for x in columns.clone() {
                    let distance = distance_to_segment((x as f32 + 0.5, y as f32 + 0.5), start, end);
                    let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
                    if coverage <= 0.0 {
                        continue;
                    }
                    let index = y * width + x;
                    let painted = (coverage * 255.0).round() as u8;
                    mask.alpha[index] = if self.add { mask.alpha[index].max(painted) } else { mask.alpha[index].min(255 - painted) };
                }
            }
        }
    }
}

/// The pixel indices from `from` to `to` (in pixel coordinates), kept within `0..size`.
fn pixel_span(from: f32, to: f32, size: usize) -> std::ops::Range<usize> {
    let start = from.floor().max(0.0) as usize;
    let end = (to.ceil().max(0.0) as usize).min(size);
    start.min(end)..end
}

/// The distance from `point` to the line segment from `start` to `end`.
fn distance_to_segment(point: (f32, f32), start: (f32, f32), end: (f32, f32)) -> f32 {
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    let length_squared = dx * dx + dy * dy;
    let t = if length_squared > 0.0 { (((point.0 - start.0) * dx + (point.1 - start.1) * dy) / length_squared).clamp(0.0, 1.0) } else { 0.0 };
    (point.0 - (start.0 + dx * t)).hypot(point.1 - (start.1 + dy * t))
}

/// Paints all `strokes` onto `mask` in order, so a later stroke wins where strokes cross.
pub(crate) fn paint_strokes(mask: &mut Mask, strokes: &[Stroke]) {
    for stroke in strokes {
        stroke.paint(mask);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::rectangle;

    #[test]
    fn strokes_paint_thick_round_ended_lines() {
        let mut mask = rectangle(100, 50, 0..0, 0..0);
        // A horizontal stroke across the middle from x = 20 to 80, radius 5.3 pixels.
        let stroke = Stroke { points: vec![(0.2, 0.5), (0.8, 0.5)], radius: 0.053, add: true };
        stroke.paint(&mut mask);
        assert_eq!(mask.at(50, 25), 255, "on the line");
        assert_eq!(mask.at(50, 21), 255, "within the radius");
        assert_eq!(mask.at(50, 31), 0, "beyond the radius");
        assert_eq!(mask.at(15, 25), 255, "the round end reaches past the first point");
        assert_eq!(mask.at(15, 20), 0, "but is round, not square");
        assert_eq!(mask.at(10, 25), 0, "and stops");
        let rim = mask.at(50, 30);
        assert!(rim > 0 && rim < 255, "a soft one-pixel rim: {rim}");
    }

    #[test]
    fn remove_strokes_clear_and_later_strokes_win() {
        let mut mask = rectangle(100, 100, 0..100, 0..100);
        let remove = Stroke { points: vec![(0.5, 0.5)], radius: 0.1, add: false };
        let add = Stroke { points: vec![(0.5, 0.5)], radius: 0.03, add: true };
        paint_strokes(&mut mask, &[remove.clone(), add.clone()]);
        assert_eq!(mask.at(50, 50), 255, "the later add stroke wins in the middle");
        assert_eq!(mask.at(50, 57), 0, "the remove stroke clears around it");
        assert_eq!(mask.at(50, 70), 255, "outside both, untouched");
        let mut other_order = rectangle(100, 100, 0..100, 0..100);
        paint_strokes(&mut other_order, &[add, remove]);
        assert_eq!(other_order.at(50, 50), 0, "the later remove stroke wins");
    }

    #[test]
    fn strokes_without_usable_points_paint_nothing() {
        let mut mask = rectangle(10, 10, 0..0, 0..0);
        paint_strokes(&mut mask, &[Stroke { points: vec![], radius: 0.5, add: true }, Stroke { points: vec![(f32::NAN, 0.5)], radius: 0.5, add: true }]);
        assert!(mask.alpha.iter().all(|&a| a == 0));
        // Points off the picture paint only the part that overlaps it.
        paint_strokes(&mut mask, &[Stroke { points: vec![(-0.2, 0.5), (0.1, 0.5)], radius: 0.1, add: true }]);
        assert_eq!(mask.at(0, 5), 255);
        assert_eq!(mask.at(5, 5), 0);
    }

    #[test]
    fn samples_are_spread_evenly_along_the_stroke() {
        // 100 pixels long on a 200 × 100 picture, sampled every 25 pixels.
        let stroke = Stroke { points: vec![(0.25, 0.5), (0.5, 0.5), (0.5, 0.5), (0.75, 0.5)], radius: 0.01, add: true };
        let samples = stroke.sample(200, 100, 25.0);
        let xs: Vec<f32> = samples.iter().map(|&(x, _)| x * 200.0).collect();
        assert_eq!(xs.len(), 5, "{xs:?}");
        for (x, expected) in xs.iter().zip([50.0, 75.0, 100.0, 125.0, 150.0]) {
            assert!((x - expected).abs() < 1e-3, "{xs:?}");
        }
        assert_eq!(Stroke { points: vec![(0.3, 0.4)], radius: 0.01, add: true }.sample(200, 100, 25.0), vec![(0.3, 0.4)], "a dab is one point");
    }
}
