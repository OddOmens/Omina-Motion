//! Finding the hard cuts in a video, so a clip can be split into its shots.
//!
//! Each frame is reduced to a small summary of its colors: for each quarter of the picture, how
//! its red, green and blue values are spread (a histogram). Two frames differ by how far those
//! spreads moved (the earth mover's distance: 0 for the same colors, 1 for black to white), so a
//! camera moving or someone walking changes it a little and a cut to another shot a lot. A fade
//! moves the colors a bit each frame and never jumps.
//!
//! A cut is a frame whose change from the one before stands well clear of the changes around it:
//! several times the usual change nearby (the median, which a second cut close by doesn't move)
//! and over a fixed floor. Measuring against what is around it is what keeps a fast pan or a
//! shaky camera, which change every frame a lot, from looking like cuts. Of cuts closer together
//! than the shortest shot allowed, only the clearest is kept.

use std::path::Path;

use crate::decode::{MediaError, VideoDecoder};

/// Frames are analyzed this wide: plenty to tell shots apart, and quick.
const WIDTH: u32 = 160;
/// The picture is split into this many regions across and down, so a cut between shots with
/// the same colors in different places still shows.
const GRID: usize = 2;
/// Histogram bins per color channel.
const BINS: usize = 16;
/// How many frames either side a frame's change is compared with.
const NEIGHBORS: usize = 8;
/// The least change that can be a cut, whatever is around it (a twentieth of the way from
/// black to white). Compression noise and slow movement stay far below it.
const MIN_JUMP: f32 = 0.05;
/// How many times the usual change nearby a cut must be, plus a little (`MARGIN`).
const RATIO: f32 = 3.0;
const MARGIN: f32 = 0.02;

/// A frame's colors in brief: for each region and channel, the cumulative share of its pixels at
/// or below each bin (the last bin, always 1, is left out).
#[derive(Clone, Debug, PartialEq)]
pub struct ColorSignature(Vec<f32>);

impl ColorSignature {
    /// The signature of an RGBA picture. Each value is shared between the two bins it falls
    /// between, so colors drifting slowly change the signature smoothly instead of in steps.
    pub fn of(rgba: &[u8], width: u32, height: u32) -> Self {
        let (width, height) = (width.max(1) as usize, height.max(1) as usize);
        let mut histograms = vec![0.0f32; GRID * GRID * 3 * BINS];
        let mut counts = [0u32; GRID * GRID];
        for (index, pixel) in rgba.as_chunks::<4>().0.iter().take(width * height).enumerate() {
            let (x, y) = (index % width, index / width);
            let region = (y * GRID / height) * GRID + x * GRID / width;
            counts[region] += 1;
            for (channel, &value) in pixel[..3].iter().enumerate() {
                let position = f32::from(value) / 255.0 * (BINS - 1) as f32;
                let low = (position.floor() as usize).min(BINS - 2);
                let upper_share = position - low as f32;
                let base = (region * 3 + channel) * BINS;
                histograms[base + low] += 1.0 - upper_share;
                histograms[base + low + 1] += upper_share;
            }
        }
        let mut cumulative = Vec::with_capacity(GRID * GRID * 3 * (BINS - 1));
        for (block, histogram) in histograms.as_chunks::<BINS>().0.iter().enumerate() {
            let total = counts[block / 3].max(1) as f32;
            let mut sum = 0.0;
            for &count in &histogram[..BINS - 1] {
                sum += count / total;
                cumulative.push(sum);
            }
        }
        Self(cumulative)
    }

    /// How different two frames' colors are, from 0 (the same) to 1 (black and white): how far
    /// each region's color spread moved, on average.
    pub fn difference(&self, other: &Self) -> f32 {
        let moved: f32 = self.0.iter().zip(&other.0).map(|(a, b)| (a - b).abs()).sum();
        moved / (GRID * GRID * 3 * (BINS - 1)) as f32
    }
}

/// Finds cuts in frames given one at a time, in order.
#[derive(Default)]
pub struct CutFinder {
    previous: Option<ColorSignature>,
    /// Each frame's change from the frame before (0 for the first).
    changes: Vec<f32>,
}

impl CutFinder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the next frame, as RGBA.
    pub fn push(&mut self, rgba: &[u8], width: u32, height: u32) {
        let signature = ColorSignature::of(rgba, width, height);
        self.changes.push(self.previous.as_ref().map_or(0.0, |previous| previous.difference(&signature)));
        self.previous = Some(signature);
    }

    /// Adds a frame that looks the same as the one before (one that couldn't be decoded).
    pub fn push_unchanged(&mut self) {
        self.changes.push(0.0);
    }

    /// The frames (counting from 0, in the order pushed) that start a new shot, no two closer
    /// than `min_shot` frames.
    pub fn cuts(&self, min_shot: usize) -> Vec<usize> {
        cuts_from_changes(&self.changes, min_shot)
    }
}

/// The frames that start a new shot, from each frame's change from the frame before (see
/// `CutFinder`). Split out from decoding so it can be tested on made-up numbers.
pub fn cuts_from_changes(changes: &[f32], min_shot: usize) -> Vec<usize> {
    // (frame, how far over its threshold it is), for every frame clear of what is around it.
    let mut candidates: Vec<(usize, f32)> = (1..changes.len())
        .filter_map(|index| {
            let mut nearby: Vec<f32> = changes[index.saturating_sub(NEIGHBORS).max(1)..index].iter().chain(changes.get(index + 1..(index + 1 + NEIGHBORS).min(changes.len())).unwrap_or(&[])).copied().collect();
            nearby.sort_by(f32::total_cmp);
            let usual = nearby.get(nearby.len() / 2).copied().unwrap_or(0.0);
            let threshold = MIN_JUMP.max(usual * RATIO + MARGIN);
            (changes[index] > threshold).then(|| (index, changes[index] / threshold))
        })
        .collect();
    // The clearest cuts first; any closer than a shot's length to one already kept is dropped.
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut kept: Vec<usize> = Vec::new();
    for (index, _) in candidates {
        if kept.iter().all(|&other| other.abs_diff(index) >= min_shot.max(1)) {
            kept.push(index);
        }
    }
    kept.sort_unstable();
    kept
}

/// Finds the cuts in source frames `frames` of the video at `path` (in playing order; usually
/// one after another). Returns positions in `frames` where a new shot starts, no two closer
/// than `min_shot`. `progress` is told how far along it is (0–1); a cancelled search finds
/// nothing. A frame that can't be decoded counts as unchanged, unless it is the first.
pub fn find_scene_cuts(path: &Path, frames: &[i64], min_shot: usize, progress: &dyn Fn(f32), cancelled: &dyn Fn() -> bool) -> Result<Vec<usize>, MediaError> {
    let mut decoder = VideoDecoder::open_fastest(path, WIDTH)?;
    let mut finder = CutFinder::new();
    for (position, &index) in frames.iter().enumerate() {
        if cancelled() {
            return Ok(Vec::new());
        }
        match decoder.frame(index) {
            Ok(frame) => finder.push(&frame.rgba, frame.width, frame.height),
            Err(error) if position == 0 => return Err(error),
            Err(_) => finder.push_unchanged(),
        }
        progress((position + 1) as f32 / frames.len() as f32);
    }
    Ok(finder.cuts(min_shot))
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 96;
    const H: u32 = 54;

    /// A picture whose pixel at (x, y) is `color(x, y)`.
    fn picture(color: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        (0..H).flat_map(|y| (0..W).map(move |x| (x, y))).flat_map(|(x, y)| {
            let [r, g, b] = color(x, y);
            [r, g, b, 255]
        }).collect()
    }

    fn flat(rgb: [u8; 3]) -> Vec<u8> {
        picture(|_, _| rgb)
    }

    /// A busy pattern of colored stripes and checks, shifted `offset` pixels to the left (a pan).
    fn textured(seed: u32, offset: u32) -> Vec<u8> {
        picture(move |x, y| {
            let x = x + offset;
            let check = ((x / 6 + y / 6) % 2) as u8;
            let stripe = ((x * (seed + 3) / 7) % 5) as u8;
            [40 + check * 120 + stripe * 15, (seed * 70 % 200) as u8 + stripe * 10, 200 - check * 90]
        })
    }

    /// A wide landscape (sky with a sun, ground) seen from `offset` pixels along: nothing in it
    /// repeats, so panning across it really changes what each part of the picture holds.
    fn landscape(offset: u32) -> Vec<u8> {
        picture(move |x, y| {
            let along = (x + offset) as f32;
            if ((along - 300.0).powi(2) + (y as f32 - 15.0).powi(2)).sqrt() < 12.0 {
                return [255, 230, 120];
            }
            if y < 30 {
                [(along * 0.3) as u8, (120.0 + 60.0 * (along / 40.0).sin()) as u8, 220]
            } else {
                [60, (90.0 + (along / 9.0).sin() * 40.0 + y as f32) as u8, (along * 0.2) as u8]
            }
        })
    }

    fn mix(a: &[u8], b: &[u8], amount: f32) -> Vec<u8> {
        a.iter().zip(b).map(|(&a, &b)| (f32::from(a) * (1.0 - amount) + f32::from(b) * amount).round() as u8).collect()
    }

    fn cuts(frames: &[Vec<u8>]) -> Vec<usize> {
        let mut finder = CutFinder::new();
        for frame in frames {
            finder.push(frame, W, H);
        }
        finder.cuts(6)
    }

    #[test]
    fn finds_cuts_between_flat_and_textured_shots_exactly() {
        let mut frames = Vec::new();
        frames.extend((0..20).map(|_| flat([200, 30, 30])));
        frames.extend((0..20).map(|step| textured(1, step)));
        frames.extend((0..20).map(|_| flat([20, 40, 220])));
        frames.extend((0..20).map(|step| textured(2, step * 2)));
        frames.extend((0..20).map(|step| textured(6, step)));
        frames.extend((0..20).map(|step| landscape(step * 2)));
        assert_eq!(cuts(&frames), vec![20, 40, 60, 80, 100]);
    }

    #[test]
    fn a_pan_or_a_shaky_camera_is_not_a_cut() {
        // A slow pan across a landscape, a whip pan (the sun swinging through the picture), then
        // the picture jumping about as a handheld camera does.
        let mut frames: Vec<Vec<u8>> = (0..60).map(|step| landscape(step * 3)).collect();
        frames.extend((0..30).map(|step| landscape(180 + step * 16)));
        frames.extend((0..40).map(|step| textured(3, [0, 7, 2, 9, 4, 11][step as usize % 6])));
        assert_eq!(cuts(&frames), vec![90], "only the cut from the landscape to the pattern");
    }

    #[test]
    fn fades_are_not_cuts() {
        let (black, white) = (flat([0, 0, 0]), flat([255, 255, 255]));
        let (a, b) = (textured(1, 0), textured(4, 0));
        let mut frames: Vec<Vec<u8>> = (0..10).map(|_| black.clone()).collect();
        // Fade up from black, a cross-fade between two shots, and a fade to white.
        frames.extend((0..=24).map(|step| mix(&black, &a, step as f32 / 24.0)));
        frames.extend((0..=24).map(|step| mix(&a, &b, step as f32 / 24.0)));
        frames.extend((0..=24).map(|step| mix(&b, &white, step as f32 / 24.0)));
        frames.extend((0..10).map(|_| white.clone()));
        assert_eq!(cuts(&frames), Vec::<usize>::new());
    }

    #[test]
    fn cuts_close_together_keep_the_clearest() {
        // A one-frame flash between shots, and two cuts four frames apart.
        let changes = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.9, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        assert_eq!(cuts_from_changes(&changes, 3), vec![9]);
        assert_eq!(cuts_from_changes(&changes, 1), vec![8, 9]);
        assert!(cuts_from_changes(&[], 3).is_empty() && cuts_from_changes(&[0.0], 3).is_empty());
        // A cut stands out against busy motion too, but motion alone never does.
        let busy: Vec<f32> = (0..30).map(|index| if index == 15 { 0.6 } else if index == 0 { 0.0 } else { 0.1 + (index % 3) as f32 * 0.02 }).collect();
        assert_eq!(cuts_from_changes(&busy, 3), vec![15]);
    }

    #[test]
    fn the_difference_runs_from_the_same_to_black_and_white() {
        let (black, white, gray) = (flat([0, 0, 0]), flat([255, 255, 255]), flat([128, 128, 128]));
        let signature = |rgba: &[u8]| ColorSignature::of(rgba, W, H);
        assert_eq!(signature(&black).difference(&signature(&black)), 0.0);
        assert!((signature(&black).difference(&signature(&white)) - 1.0).abs() < 1e-4);
        assert!((signature(&black).difference(&signature(&gray)) - 128.0 / 255.0).abs() < 1e-3, "flat colors differ by how far apart they are");
    }
}
