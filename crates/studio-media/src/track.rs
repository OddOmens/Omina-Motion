//! Motion tracking: following one point of a video from frame to frame, so text or graphics
//! can be pinned to it. A small square of picture around the point is looked for in the next
//! frame, near where it was, by best match (normalized cross-correlation, so changes in
//! brightness don't throw it). The square is refreshed from each frame as it goes, so the
//! track keeps up with things that turn or change shape slowly.

use std::path::Path;

use studio_core::animation::Vec2;

use crate::decode::{MediaError, VideoDecoder};

/// Frames are tracked at this width: detailed enough, and quick.
const WIDTH: u32 = 640;
/// The square followed, and how far it is looked for each frame, in pixels at `WIDTH`.
const PATCH: i32 = 15; // half the side
const SEARCH: i32 = 40;

/// A frame in gray, for matching.
struct Gray {
    width: i32,
    height: i32,
    pixels: Vec<f32>,
}

impl Gray {
    fn from_rgba(rgba: &[u8], width: u32, height: u32) -> Self {
        let pixels = rgba.chunks(4).map(|pixel| 0.299 * f32::from(pixel[0]) + 0.587 * f32::from(pixel[1]) + 0.114 * f32::from(pixel[2])).collect();
        Self { width: width as i32, height: height as i32, pixels }
    }

    fn at(&self, x: i32, y: i32) -> f32 {
        self.pixels[(y.clamp(0, self.height - 1) * self.width + x.clamp(0, self.width - 1)) as usize]
    }

    /// The square around (`x`, `y`), with its average taken away (for normalized matching).
    fn patch(&self, x: i32, y: i32) -> Vec<f32> {
        let values: Vec<f32> = (-PATCH..=PATCH).flat_map(|dy| (-PATCH..=PATCH).map(move |dx| (dx, dy))).map(|(dx, dy)| self.at(x + dx, y + dy)).collect();
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        values.into_iter().map(|value| value - mean).collect()
    }
}

/// How alike two mean-removed patches are: 1 for the same, down to -1.
fn likeness(a: &[f32], b: &[f32]) -> f32 {
    let (mut cross, mut a2, mut b2) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        cross += x * y;
        a2 += x * x;
        b2 += y * y;
    }
    if a2 <= 1e-6 || b2 <= 1e-6 { 0.0 } else { cross / (a2 * b2).sqrt() }
}

/// Where the template is found in `frame`, searching around (`x`, `y`).
fn find(frame: &Gray, template: &[f32], x: i32, y: i32) -> (i32, i32, f32) {
    let mut best = (x, y, f32::MIN);
    // Every other pixel first, then the pixels around the best of those.
    for dy in (-SEARCH..=SEARCH).step_by(2) {
        for dx in (-SEARCH..=SEARCH).step_by(2) {
            let score = likeness(template, &frame.patch(x + dx, y + dy));
            if score > best.2 {
                best = (x + dx, y + dy, score);
            }
        }
    }
    let (cx, cy, _) = best;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let score = likeness(template, &frame.patch(cx + dx, cy + dy));
            if score > best.2 {
                best = (cx + dx, cy + dy, score);
            }
        }
    }
    best
}

/// Follows the point `start` (fractions of the picture's width and height) from source frame
/// `from` through to `to` (exclusive), and back to `back_to`. Returns the point in every frame
/// of `back_to..to`, in order, as fractions. `progress` is told how far along it is (0–1).
pub fn track_point(path: &Path, start: Vec2, from: i64, back_to: i64, to: i64, progress: &dyn Fn(f32), cancelled: &dyn Fn() -> bool) -> Result<Vec<(i64, Vec2)>, MediaError> {
    let mut decoder = VideoDecoder::open(path, WIDTH, true)?;
    let gray = |decoder: &mut VideoDecoder, index: i64| -> Result<Gray, MediaError> {
        let frame = decoder.frame(index)?;
        Ok(Gray::from_rgba(&frame.rgba, frame.width, frame.height))
    };
    let first = gray(&mut decoder, from)?;
    let to_pixels = |point: Vec2, frame: &Gray| ((point.x * frame.width as f32).round() as i32, (point.y * frame.height as f32).round() as i32);
    let to_fraction = |x: i32, y: i32, frame: &Gray| Vec2::new(x as f32 / frame.width as f32, y as f32 / frame.height as f32);
    let (sx, sy) = to_pixels(start, &first);
    let total = ((to - from) + (from - back_to)).max(1) as f32;
    let mut done = 0.0;
    let mut forward = vec![(from, start)];
    // One direction at a time, each from the clicked frame.
    for (range, direction) in [((from + 1)..to, 1), (back_to..from, -1)] {
        let mut template = first.patch(sx, sy);
        let (mut x, mut y) = (sx, sy);
        let frames: Vec<i64> = if direction > 0 { range.collect() } else { range.rev().collect() };
        for index in frames {
            if cancelled() {
                return Ok(Vec::new());
            }
            let frame = gray(&mut decoder, index)?;
            let (nx, ny, score) = find(&frame, &template, x, y);
            // A poor match (something passing in front) keeps the old template, so the track
            // can pick the point up again once it is clear.
            if score > 0.6 {
                template = frame.patch(nx, ny);
            }
            (x, y) = (nx, ny);
            forward.push((index, to_fraction(x, y, &frame)));
            done += 1.0;
            progress(done / total);
        }
    }
    forward.sort_by_key(|(index, _)| *index);
    Ok(forward)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame of soft noise with a bright cross at (`cx`, `cy`).
    fn frame(cx: i32, cy: i32) -> Gray {
        let (width, height) = (160, 120);
        let pixels = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y): (i32, i32)| {
                let noise = (((x * 73 + y * 151) % 17) as f32) * 2.0;
                let cross = (x - cx).abs() <= 1 && (y - cy).abs() <= 6 || (y - cy).abs() <= 1 && (x - cx).abs() <= 6;
                if cross { 230.0 } else { 40.0 + noise }
            })
            .collect();
        Gray { width, height, pixels }
    }

    #[test]
    fn a_moving_cross_is_found_where_it_went() {
        let before = frame(60, 50);
        let after = frame(73, 44);
        let (x, y, score) = find(&after, &before.patch(60, 50), 60, 50);
        assert_eq!((x, y), (73, 44));
        assert!(score > 0.8, "{score}");
    }

    #[test]
    fn matching_ignores_overall_brightness() {
        let a = vec![1.0, -1.0, 2.0, -2.0];
        let b: Vec<f32> = a.iter().map(|value| value * 3.0).collect();
        assert!((likeness(&a, &b) - 1.0).abs() < 1e-6);
    }
}

/// Tracks the middle of a test pattern square moving across a dark frame (from (150, 180),
/// 150 px a second right and 20 down, at 12 fps, in 640 × 360):
/// `TRACK_CLIP=<clip> cargo test -p studio-media tracks_a_real_clip -- --ignored`.
#[test]
#[ignore = "needs a clip; set TRACK_CLIP"]
fn tracks_a_real_clip() {
    let path = std::env::var("TRACK_CLIP").unwrap();
    let start = Vec2::new(150.0 / 640.0, 180.0 / 360.0);
    let points = track_point(Path::new(&path), start, 0, 0, 24, &|_| {}, &|| false).unwrap();
    assert_eq!(points.len(), 24);
    for (index, point) in points {
        let seconds = index as f32 / 12.0;
        let expected = Vec2::new((150.0 + seconds * 150.0) / 640.0, (180.0 + seconds * 20.0) / 360.0);
        assert!((point.x - expected.x).abs() < 0.01 && (point.y - expected.y).abs() < 0.015, "frame {index}: {point:?} vs {expected:?}");
    }
}
