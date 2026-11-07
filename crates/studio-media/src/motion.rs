//! The tracking engine behind Track Motion and Stabilize, working on gray pictures already in
//! memory (no decoding here, so it is tested on made-up frames).
//!
//! A small square of picture (a "patch") is looked for in the next frame by normalized
//! cross-correlation, which scores how alike two patches are regardless of overall brightness
//! and contrast. Each frame is kept as a pyramid of ever smaller copies: the search starts on
//! the smallest copy, where a wide search is cheap, and is narrowed on each larger one. The
//! best whole-pixel match is then refined to a fraction of a pixel. The match score doubles as
//! a confidence, so a point that disappears is reported as lost instead of drifting off.
//!
//! For stabilizing, many patches spread over the frame are followed at once and their
//! movements combined with medians, which ignore the few that land on something moving by
//! itself, giving the camera's shift and turn from one frame to the next.

use studio_core::animation::Vec2;

/// A picture in shades of gray, one `f32` per pixel (0–255), row by row.
#[derive(Clone, Debug)]
pub struct GrayFrame {
    width: usize,
    height: usize,
    pixels: Vec<f32>,
}

impl GrayFrame {
    /// A frame from its pixels. `None` if the frame is empty or `pixels` is the wrong length.
    pub fn new(width: usize, height: usize, pixels: Vec<f32>) -> Option<Self> {
        (width > 0 && height > 0 && pixels.len() == width * height).then_some(Self { width, height, pixels })
    }

    /// A gray copy of an RGBA picture (the decoder's output), weighting the channels by how
    /// bright they look (BT.601 luma). `None` if `rgba` doesn't hold `width` × `height` pixels.
    pub fn from_rgba(rgba: &[u8], width: u32, height: u32) -> Option<Self> {
        let pixels = rgba.as_chunks::<4>().0.iter().map(|pixel| 0.299 * f32::from(pixel[0]) + 0.587 * f32::from(pixel[1]) + 0.114 * f32::from(pixel[2])).collect();
        Self::new(width as usize, height as usize, pixels)
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// The pixel at (`x`, `y`); outside the frame, the nearest edge pixel.
    fn at(&self, x: i32, y: i32) -> f32 {
        let x = x.clamp(0, self.width as i32 - 1) as usize;
        let y = y.clamp(0, self.height as i32 - 1) as usize;
        self.pixels[y * self.width + x]
    }

    /// The picture at a point between pixels, blended from the four around it, so patches can
    /// be cut out at fractional positions without rounding the track to whole pixels.
    fn sample(&self, x: f32, y: f32) -> f32 {
        let (left, top) = (x.floor(), y.floor());
        let (fx, fy) = (x - left, y - top);
        let (left, top) = (left as i32, top as i32);
        let upper = self.at(left, top) * (1.0 - fx) + self.at(left + 1, top) * fx;
        let lower = self.at(left, top + 1) * (1.0 - fx) + self.at(left + 1, top + 1) * fx;
        upper * (1.0 - fy) + lower * fy
    }

    /// The frame at half the width and height, each pixel the average of the four it covers.
    fn half(&self) -> Self {
        let (width, height) = (self.width.div_ceil(2), self.height.div_ceil(2));
        let pixels = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x as i32 * 2, y as i32 * 2)))
            .map(|(x, y)| (self.at(x, y) + self.at(x + 1, y) + self.at(x, y + 1) + self.at(x + 1, y + 1)) * 0.25)
            .collect();
        Self { width, height, pixels }
    }
}

/// Most pyramid levels kept: the full frame and three smaller copies (down to an eighth).
const MAX_LEVELS: usize = 4;
/// Halving stops before a copy gets narrower than this, where too little detail is left.
const MIN_LEVEL_SIDE: usize = 24;

/// A frame and its smaller copies, the full frame first.
#[derive(Clone, Debug)]
pub struct Pyramid {
    levels: Vec<GrayFrame>,
}

impl Pyramid {
    pub fn new(frame: GrayFrame) -> Self {
        let mut levels = vec![frame];
        while levels.len() < MAX_LEVELS {
            let last = &levels[levels.len() - 1];
            if last.width / 2 < MIN_LEVEL_SIDE || last.height / 2 < MIN_LEVEL_SIDE {
                break;
            }
            let smaller = last.half();
            levels.push(smaller);
        }
        Self { levels }
    }

    /// The full-size frame.
    pub fn frame(&self) -> &GrayFrame {
        &self.levels[0]
    }
}

/// A patch ready for matching: its pixels with their average taken away, and their spread.
#[derive(Clone, Debug)]
struct Template {
    radius: i32,
    values: Vec<f32>,
    /// The square root of the sum of the squared values (zero for a flat patch).
    norm: f32,
}

impl Template {
    /// The square of side `2 × radius + 1` centered on (`x`, `y`) of `frame`.
    fn cut(frame: &GrayFrame, x: f32, y: f32, radius: i32) -> Self {
        let values: Vec<f32> = (-radius..=radius).flat_map(|dy| (-radius..=radius).map(move |dx| (dx, dy))).map(|(dx, dy)| frame.sample(x + dx as f32, y + dy as f32)).collect();
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        let values: Vec<f32> = values.into_iter().map(|value| value - mean).collect();
        let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
        Self { radius, values, norm }
    }

    /// How alike the patch is to the square centered on whole pixel (`x`, `y`) of `frame`:
    /// 1 for the same picture (even brighter or with more contrast), 0 for unrelated, down to
    /// -1 for a negative. A flat square, which has nothing to compare, scores 0.
    fn score(&self, frame: &GrayFrame, x: i32, y: i32) -> f32 {
        let (mut sum, mut squares, mut cross) = (0.0f32, 0.0f32, 0.0f32);
        let mut values = self.values.iter();
        for dy in -self.radius..=self.radius {
            for dx in -self.radius..=self.radius {
                let pixel = frame.at(x + dx, y + dy);
                sum += pixel;
                squares += pixel * pixel;
                // The template's values already average zero, so the square's own average
                // drops out of this sum and needn't be taken away first.
                cross += values.next().copied().unwrap_or(0.0) * pixel;
            }
        }
        let count = self.values.len() as f32;
        let spread = (squares - sum * sum / count).max(0.0).sqrt();
        if self.norm < 1e-3 || spread < 1e-3 { 0.0 } else { (cross / (self.norm * spread)).clamp(-1.0, 1.0) }
    }
}

/// Patches with less variation than this (the standard deviation of their gray values, out of
/// 255) are too flat to follow: noise would decide where they match.
const MIN_DETAIL: f32 = 3.0;
/// A patch is cut from smaller pyramid levels only while it is at least this many pixels across
/// from its center, so the coarse search still has something to compare.
const MIN_LEVEL_RADIUS: i32 = 3;

/// A patch to look for in later frames, cut from every pyramid level it is big enough for.
#[derive(Clone, Debug)]
pub struct PatchTemplate {
    levels: Vec<Template>,
}

impl PatchTemplate {
    /// The patch of `radius` pixels around `center` (full-size pixels) of `pyramid`. `None`
    /// where the picture is too flat to follow, such as a clear sky.
    pub fn new(pyramid: &Pyramid, center: Vec2, radius: i32) -> Option<Self> {
        let full = Template::cut(pyramid.frame(), center.x, center.y, radius.max(1));
        let deviation = full.norm / (full.values.len() as f32).sqrt();
        if deviation < MIN_DETAIL {
            return None;
        }
        let mut levels = vec![full];
        for (level, frame) in pyramid.levels.iter().enumerate().skip(1) {
            let scale = (1 << level) as f32;
            let level_radius = radius >> level;
            if level_radius < MIN_LEVEL_RADIUS {
                break;
            }
            levels.push(Template::cut(frame, center.x / scale, center.y / scale, level_radius));
        }
        Some(Self { levels })
    }
}

/// Where a patch was found, in full-size pixels, and how sure the match is: its score, from 0
/// (nothing like it) to 1 (identical).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Found {
    pub position: Vec2,
    pub confidence: f32,
}

/// Looks for `template` in `pyramid` within `search` full-size pixels of `around` (both ways).
pub fn find_patch(template: &PatchTemplate, pyramid: &Pyramid, around: Vec2, search: i32) -> Found {
    let top = template.levels.len().min(pyramid.levels.len()) - 1;
    // A wide, exhaustive search on the smallest level...
    let scale = (1 << top) as f32;
    let reach = (search as f32 / scale).ceil() as i32;
    let start = ((around.x / scale).round() as i32, (around.y / scale).round() as i32);
    let mut best = best_in(&template.levels[top], &pyramid.levels[top], start, reach);
    // ...narrowed level by level: each step doubles the position and checks around it, which
    // covers the half-pixel uncertainty of the coarser level with room to spare.
    for level in (0..top).rev() {
        best = best_in(&template.levels[level], &pyramid.levels[level], (best.0 * 2, best.1 * 2), 2);
    }
    let (x, y, score) = best;
    let full = &template.levels[0];
    let frame = &pyramid.levels[0];
    let offset_x = peak_offset(full.score(frame, x - 1, y), score, full.score(frame, x + 1, y));
    let offset_y = peak_offset(full.score(frame, x, y - 1), score, full.score(frame, x, y + 1));
    Found { position: Vec2::new(x as f32 + offset_x, y as f32 + offset_y), confidence: score.max(0.0) }
}

/// The best-scoring whole-pixel position within `reach` of `center`, and its score.
fn best_in(template: &Template, frame: &GrayFrame, center: (i32, i32), reach: i32) -> (i32, i32, f32) {
    let mut best = (center.0, center.1, f32::MIN);
    for y in center.1 - reach..=center.1 + reach {
        for x in center.0 - reach..=center.0 + reach {
            let score = template.score(frame, x, y);
            if score > best.2 {
                best = (x, y, score);
            }
        }
    }
    best
}

/// Where the true peak lies between whole pixels, from the scores just before, at and just
/// after the best one: the top of the parabola through the three, within half a pixel.
fn peak_offset(before: f32, at: f32, after: f32) -> f32 {
    let curvature = before - 2.0 * at + after;
    if curvature >= -1e-6 {
        return 0.0; // Not a peak (flat or a ridge): stay on the whole pixel.
    }
    (0.5 * (before - after) / curvature).clamp(-0.5, 0.5)
}

/// Below this confidence a match is not trusted: the point is hidden or gone.
pub const LOST_CONFIDENCE: f32 = 0.5;
/// Above this confidence the patch is refreshed from the new frame, so the track keeps up with
/// things that turn or change shape slowly. A weaker match keeps the old patch, so the point
/// can be picked up again after something passes in front of it.
const REFRESH_CONFIDENCE: f32 = 0.8;

/// How near (in pixels) to the latest match the starting patch is looked for, to correct drift.
const DRIFT_CHECK_RADIUS: i32 = 2;

/// How far a point tracker looks, in full-size pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackSettings {
    /// Half the side of the square followed.
    pub patch_radius: i32,
    /// How far from where it is expected the point is looked for, each way.
    pub search_radius: i32,
}

impl Default for TrackSettings {
    /// Suited to frames about 640 pixels wide.
    fn default() -> Self {
        Self { patch_radius: 12, search_radius: 48 }
    }
}

/// Follows one point from frame to frame.
#[derive(Clone, Debug)]
pub struct PointTracker {
    /// The patch as it looked in the latest confidently matched frame.
    template: PatchTemplate,
    /// The patch as it looked where tracking started. While it still matches, its position is
    /// preferred, so tiny errors from refreshing the patch each frame can't add up to drift.
    reference: PatchTemplate,
    position: Vec2,
    /// How far the point moved over the last trusted frame, so the search starts where it is
    /// heading rather than where it was.
    velocity: Vec2,
    settings: TrackSettings,
}

impl PointTracker {
    /// Starts at `start` (full-size pixels) of `pyramid`. `None` where the picture is too flat
    /// there to follow.
    pub fn new(pyramid: &Pyramid, start: Vec2, settings: TrackSettings) -> Option<Self> {
        let template = PatchTemplate::new(pyramid, start, settings.patch_radius)?;
        Some(Self { reference: template.clone(), template, position: start, velocity: Vec2::default(), settings })
    }

    /// Finds the point in the next frame. A confident match moves the tracker there; an
    /// unsure one leaves it coasting along its last direction.
    pub fn step(&mut self, pyramid: &Pyramid) -> Found {
        let expected = self.position + self.velocity;
        let mut found = find_patch(&self.template, pyramid, expected, self.settings.search_radius);
        if found.confidence < LOST_CONFIDENCE {
            self.position = expected;
            return found;
        }
        // Checked against the starting patch close to where the latest one matched.
        let anchored = find_patch(&self.reference, pyramid, found.position, DRIFT_CHECK_RADIUS);
        if anchored.confidence >= REFRESH_CONFIDENCE {
            found.position = anchored.position;
        }
        self.velocity = found.position - self.position;
        self.position = found.position;
        if found.confidence >= REFRESH_CONFIDENCE
            && let Some(fresh) = PatchTemplate::new(pyramid, found.position, self.settings.patch_radius)
        {
            self.template = fresh;
        }
        found
    }
}

/// Frames in a row a point may be unsure for (something passing in front) before it counts as
/// lost. If it is found again within them, the frames between are filled in along a line.
pub const LOST_AFTER_FRAMES: usize = 4;

/// The result of following a point: its position in each frame it was found in, in the order
/// the frames were given, and the first frame it was lost at, if it was.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Followed {
    pub points: Vec<(i64, Vec2)>,
    pub lost_at: Option<i64>,
}

/// Follows a point through frames given one at a time, deciding when it is lost.
pub struct PointFollower {
    tracker: PointTracker,
    followed: Followed,
    /// Frames since the last confident match, waiting to be filled in or given up on.
    unsure: Vec<i64>,
}

impl PointFollower {
    /// Starts at `start` (full-size pixels) of frame `index`. `None` where the picture is too
    /// flat to follow.
    pub fn new(index: i64, pyramid: &Pyramid, start: Vec2, settings: TrackSettings) -> Option<Self> {
        let tracker = PointTracker::new(pyramid, start, settings)?;
        Some(Self { tracker, followed: Followed { points: vec![(index, start)], lost_at: None }, unsure: Vec::new() })
    }

    /// Looks for the point in frame `index`, the next in order. Returns `false` once the point
    /// is lost, after which further frames are ignored.
    pub fn push(&mut self, index: i64, pyramid: &Pyramid) -> bool {
        if self.followed.lost_at.is_some() {
            return false;
        }
        let found = self.tracker.step(pyramid);
        if found.confidence < LOST_CONFIDENCE {
            self.unsure.push(index);
            if self.unsure.len() > LOST_AFTER_FRAMES {
                self.followed.lost_at = self.unsure.first().copied();
                return false;
            }
            return true;
        }
        // Found again after a few unsure frames: put those on the line between.
        let last = self.followed.points.last().map_or(found.position, |&(_, point)| point);
        let gap = self.unsure.len() as f32 + 1.0;
        for (step, frame) in self.unsure.drain(..).enumerate() {
            let along = (step as f32 + 1.0) / gap;
            self.followed.points.push((frame, last + (found.position - last) * along));
        }
        self.followed.points.push((index, found.position));
        true
    }

    /// The frames followed. Frames still unsure at the end count as lost.
    pub fn finish(mut self) -> Followed {
        if self.followed.lost_at.is_none() {
            self.followed.lost_at = self.unsure.first().copied();
        }
        self.followed
    }
}

/// How the camera moved from one frame to the next: the picture turned by `rotation` radians
/// (clockwise, as y points down) about the frame's center, then shifted by `shift` pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameMotion {
    pub shift: Vec2,
    pub rotation: f32,
}

/// Patches spread over the frame for measuring its motion: this many across and down.
const GRID: (usize, usize) = (6, 4);
/// The patches are kept this far (a fraction of the width and height) from the frame's edges,
/// which come and go as the camera moves.
const GRID_MARGIN: f32 = 0.12;
/// Settings for the patches that measure the whole frame's motion: camera shake is small from
/// one frame to the next, so the search is narrower than for a tracked point.
const MOTION_SETTINGS: TrackSettings = TrackSettings { patch_radius: 12, search_radius: 32 };
/// Only matches at least this sure count toward the frame's motion.
const MOTION_CONFIDENCE: f32 = 0.8;
/// Fewest patches that must be found to trust a frame's motion.
const MIN_MOTION_POINTS: usize = 3;
/// Point pairs closer than this (a fraction of the frame's smaller side) are left out of the
/// rotation estimate: a pixel of error turns a short pair a lot.
const MIN_PAIR_SPAN: f32 = 0.2;

/// How the picture moved from `previous` to `next`, or `None` if too little of it could be
/// followed (a flat or blurred frame, or a cut to another shot).
pub fn global_motion(previous: &Pyramid, next: &Pyramid) -> Option<FrameMotion> {
    let frame = previous.frame();
    let (width, height) = (frame.width as f32, frame.height as f32);
    let center = Vec2::new(width * 0.5, height * 0.5);
    // Where each grid patch was, and where it went.
    let mut pairs: Vec<(Vec2, Vec2)> = Vec::new();
    for row in 0..GRID.1 {
        for column in 0..GRID.0 {
            let fraction = |index: usize, count: usize| GRID_MARGIN + (1.0 - 2.0 * GRID_MARGIN) * (index as f32 + 0.5) / count as f32;
            let at = Vec2::new(fraction(column, GRID.0) * width, fraction(row, GRID.1) * height);
            let Some(template) = PatchTemplate::new(previous, at, MOTION_SETTINGS.patch_radius) else { continue };
            let found = find_patch(&template, next, at, MOTION_SETTINGS.search_radius);
            if found.confidence >= MOTION_CONFIDENCE {
                pairs.push((at, found.position));
            }
        }
    }
    if pairs.len() < MIN_MOTION_POINTS {
        return None;
    }
    // The turn: how much the line between each well-separated pair of points turned. Lines
    // are unaffected by a shift, so this is measured first.
    let min_span = MIN_PAIR_SPAN * width.min(height);
    let mut turns = Vec::new();
    for (index, &(from_a, to_a)) in pairs.iter().enumerate() {
        for &(from_b, to_b) in &pairs[index + 1..] {
            let (before, after) = (from_b - from_a, to_b - to_a);
            if before.length() < min_span {
                continue;
            }
            let turn = after.y.atan2(after.x) - before.y.atan2(before.x);
            // Wrapped into -π..π, so a line pointing just past straight left doesn't jump a full turn.
            turns.push((turn + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI);
        }
    }
    let rotation = median(&mut turns).unwrap_or(0.0);
    // The shift: where each point went beyond where the turn alone would have put it.
    let (sin, cos) = rotation.sin_cos();
    let turned = |point: Vec2| {
        let offset = point - center;
        center + Vec2::new(cos * offset.x - sin * offset.y, sin * offset.x + cos * offset.y)
    };
    let mut xs: Vec<f32> = pairs.iter().map(|&(from, to)| to.x - turned(from).x).collect();
    let mut ys: Vec<f32> = pairs.iter().map(|&(from, to)| to.y - turned(from).y).collect();
    Some(FrameMotion { shift: Vec2::new(median(&mut xs)?, median(&mut ys)?), rotation })
}

/// The middle value (the average of the middle two for an even count), or `None` for none.
/// Unlike an average, a few wild values (patches on something moving by itself) don't sway it.
fn median(values: &mut [f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f32::total_cmp);
    let middle = values.len() / 2;
    Some(if values.len().is_multiple_of(2) { (values[middle - 1] + values[middle]) * 0.5 } else { values[middle] })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A smooth, detailed made-up picture (bumpy like foliage or gravel), defined everywhere so
    /// it can be drawn shifted or turned by any amount, including fractions of a pixel.
    fn texture(x: f32, y: f32) -> f32 {
        // A random value at each corner of a 6-pixel lattice, blended between corners.
        let corner = |cx: i32, cy: i32| {
            let hash = (cx.wrapping_mul(374_761_393) ^ cy.wrapping_mul(668_265_263)).wrapping_mul(1_274_126_177);
            ((hash >> 8) & 0xff) as f32
        };
        let (gx, gy) = (x / 6.0, y / 6.0);
        let (left, top) = (gx.floor(), gy.floor());
        let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
        let (fx, fy) = (smooth(gx - left), smooth(gy - top));
        let (left, top) = (left as i32, top as i32);
        let upper = corner(left, top) * (1.0 - fx) + corner(left + 1, top) * fx;
        let lower = corner(left, top + 1) * (1.0 - fx) + corner(left + 1, top + 1) * fx;
        upper * (1.0 - fy) + lower * fy
    }

    /// A 240 × 160 frame drawn by `picture` at each pixel.
    fn frame_of(picture: impl Fn(f32, f32) -> f32) -> Pyramid {
        let (width, height) = (240, 160);
        let pixels = (0..height).flat_map(|y| (0..width).map(move |x| (x as f32, y as f32))).map(|(x, y)| picture(x, y)).collect();
        Pyramid::new(GrayFrame::new(width, height, pixels).unwrap())
    }

    /// The texture moved right by `dx` and down by `dy`.
    fn shifted(dx: f32, dy: f32) -> Pyramid {
        frame_of(|x, y| texture(x - dx, y - dy))
    }

    fn close(a: Vec2, b: Vec2, tolerance: f32) -> bool {
        (a - b).length() <= tolerance
    }

    #[test]
    fn a_patch_is_found_where_it_moved_to_within_a_fraction_of_a_pixel() {
        let before = shifted(0.0, 0.0);
        let start = Vec2::new(100.0, 70.0);
        let template = PatchTemplate::new(&before, start, 12).unwrap();
        for (dx, dy) in [(3.0, -2.0), (3.4, -2.7), (-17.25, 9.5), (30.6, 0.3)] {
            let found = find_patch(&template, &shifted(dx, dy), start, 48);
            let expected = start + Vec2::new(dx, dy);
            assert!(close(found.position, expected, 0.15), "moved ({dx}, {dy}): found {:?}, expected {expected:?}", found.position);
            assert!(found.confidence > 0.95, "{}", found.confidence);
        }
    }

    #[test]
    fn a_point_moving_steadily_is_followed_without_drifting() {
        let velocity = Vec2::new(2.3, -1.1);
        let start = Vec2::new(60.0, 100.0);
        let mut follower = PointFollower::new(0, &shifted(0.0, 0.0), start, TrackSettings::default()).unwrap();
        for index in 1..30 {
            let moved = velocity * index as f32;
            assert!(follower.push(index, &shifted(moved.x, moved.y)), "lost at frame {index}");
        }
        let followed = follower.finish();
        assert_eq!(followed.lost_at, None);
        assert_eq!(followed.points.len(), 30);
        for &(index, point) in &followed.points {
            let expected = start + velocity * index as f32;
            assert!(close(point, expected, 0.3), "frame {index}: {point:?} vs {expected:?}");
        }
    }

    #[test]
    fn matching_ignores_brightness_and_contrast() {
        let before = shifted(0.0, 0.0);
        let start = Vec2::new(120.0, 80.0);
        let template = PatchTemplate::new(&before, start, 12).unwrap();
        let brighter = frame_of(|x, y| texture(x - 4.0, y) * 0.6 + 70.0);
        let found = find_patch(&template, &brighter, start, 48);
        assert!(close(found.position, start + Vec2::new(4.0, 0.0), 0.15), "{:?}", found.position);
        assert!(found.confidence > 0.95);
    }

    #[test]
    fn a_point_that_disappears_is_reported_lost_not_followed_somewhere_else() {
        let start = Vec2::new(120.0, 80.0);
        let mut follower = PointFollower::new(0, &shifted(0.0, 0.0), start, TrackSettings::default()).unwrap();
        assert!(follower.push(1, &shifted(1.0, 0.0)));
        // From frame 2 on, the picture is something else entirely.
        let elsewhere = frame_of(|x, y| texture(x * 0.7 + 913.0, y * 1.3 + 457.0));
        let mut frame = 2;
        while follower.push(frame, &elsewhere) {
            frame += 1;
            assert!(frame < 20, "never lost");
        }
        let followed = follower.finish();
        assert_eq!(followed.lost_at, Some(2));
        assert_eq!(followed.points.iter().map(|(index, _)| *index).collect::<Vec<_>>(), vec![0, 1], "no positions for the frames it was gone");
        // The match itself scores low.
        let mut tracker = PointTracker::new(&shifted(0.0, 0.0), start, TrackSettings::default()).unwrap();
        assert!(tracker.step(&elsewhere).confidence < LOST_CONFIDENCE);
    }

    #[test]
    fn a_point_hidden_for_a_moment_is_picked_up_again_and_the_gap_filled() {
        let start = Vec2::new(120.0, 80.0);
        let mut follower = PointFollower::new(0, &shifted(0.0, 0.0), start, TrackSettings::default()).unwrap();
        let flat = frame_of(|_, _| 128.0);
        assert!(follower.push(1, &shifted(2.0, 0.0)));
        assert!(follower.push(2, &flat));
        assert!(follower.push(3, &flat));
        assert!(follower.push(4, &shifted(8.0, 0.0)));
        let followed = follower.finish();
        assert_eq!(followed.lost_at, None);
        let at = |frame: i64| followed.points.iter().find(|(index, _)| *index == frame).unwrap().1;
        assert!(close(at(4), start + Vec2::new(8.0, 0.0), 0.2), "{:?}", at(4));
        assert!(close(at(3), start + Vec2::new(6.0, 0.0), 0.3), "filled in along the line: {:?}", at(3));
    }

    #[test]
    fn flat_picture_has_nothing_to_follow() {
        let flat = frame_of(|_, _| 90.0);
        assert!(PatchTemplate::new(&flat, Vec2::new(100.0, 80.0), 12).is_none());
        assert!(PointTracker::new(&flat, Vec2::new(100.0, 80.0), TrackSettings::default()).is_none());
        assert_eq!(global_motion(&flat, &flat), None);
    }

    #[test]
    fn the_whole_frames_shift_and_turn_are_measured() {
        let center = Vec2::new(120.0, 80.0);
        let (shift, degrees) = (Vec2::new(4.5, -3.0), 1.5f32);
        // Each pixel of the moved frame shows what was at the matching point of the first:
        // undo the shift, then turn back about the center.
        let (sin, cos) = (-degrees.to_radians()).sin_cos();
        let moved = frame_of(|x, y| {
            let (ox, oy) = (x - shift.x - center.x, y - shift.y - center.y);
            texture(center.x + cos * ox - sin * oy, center.y + sin * ox + cos * oy)
        });
        let motion = global_motion(&shifted(0.0, 0.0), &moved).unwrap();
        assert!((motion.rotation.to_degrees() - degrees).abs() < 0.1, "turned {}°", motion.rotation.to_degrees());
        assert!(close(motion.shift, shift, 0.3), "shifted {:?}", motion.shift);
    }

    #[test]
    fn something_moving_by_itself_does_not_sway_the_frames_motion() {
        // The background shifts 2 px right; a block covering a quarter of the frame races left.
        let moved = frame_of(|x, y| if x < 80.0 && y < 60.0 { texture(x + 15.0, y) } else { texture(x - 2.0, y) });
        let motion = global_motion(&shifted(0.0, 0.0), &moved).unwrap();
        assert!(close(motion.shift, Vec2::new(2.0, 0.0), 0.3), "{:?}", motion.shift);
        assert!(motion.rotation.abs().to_degrees() < 0.2);
    }

    #[test]
    fn the_median_ignores_outliers() {
        assert_eq!(median(&mut [1.0, 100.0, 2.0, 3.0, -50.0]), Some(2.0));
        assert_eq!(median(&mut [4.0, 2.0]), Some(3.0));
        assert_eq!(median(&mut []), None);
    }

    #[test]
    fn a_pixel_buffer_of_the_wrong_size_is_refused() {
        assert!(GrayFrame::new(4, 4, vec![0.0; 15]).is_none());
        assert!(GrayFrame::new(0, 4, Vec::new()).is_none());
        assert!(GrayFrame::from_rgba(&[0; 4 * 6], 2, 3).is_some());
        assert!(GrayFrame::from_rgba(&[0; 4 * 5], 2, 3).is_none());
    }

    /// A frame of soft noise with a bright cross at (`cx`, `cy`).
    fn cross_at(cx: f32, cy: f32) -> Pyramid {
        frame_of(|x, y| {
            let noise = ((x as i32 * 73 + y as i32 * 151) % 17) as f32 * 2.0;
            let cross = (x - cx).abs() <= 1.0 && (y - cy).abs() <= 6.0 || (y - cy).abs() <= 1.0 && (x - cx).abs() <= 6.0;
            if cross { 230.0 } else { 40.0 + noise }
        })
    }

    #[test]
    fn a_moving_cross_is_found_where_it_went() {
        let template = PatchTemplate::new(&cross_at(60.0, 50.0), Vec2::new(60.0, 50.0), 15).unwrap();
        let found = find_patch(&template, &cross_at(73.0, 44.0), Vec2::new(60.0, 50.0), 40);
        assert!(close(found.position, Vec2::new(73.0, 44.0), 0.2), "{:?}", found.position);
        assert!(found.confidence > 0.8, "{}", found.confidence);
    }
}
