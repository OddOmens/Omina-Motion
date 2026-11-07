//! Motion tracking on clips: following one point of a video from frame to frame, so text or
//! graphics can be pinned to it (Track Motion), and measuring how the whole picture moves, so
//! shaky footage can be steadied (Stabilize). This file decodes the frames; the matching
//! itself is in `motion.rs`.

use std::path::Path;

use studio_core::animation::Vec2;

use crate::decode::{MediaError, VideoDecoder};
use crate::motion::{FrameMotion, GrayFrame, PointFollower, Pyramid, TrackSettings, global_motion};

/// Frames are tracked at this width: detailed enough, and quick.
const WIDTH: u32 = 640;

/// A point followed through a clip. Points are fractions of the picture's width and height.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackedPoint {
    /// The point in each source frame it was found in, in order of frame.
    pub points: Vec<(i64, Vec2)>,
    /// Going back from the clicked frame, the frame where the point was lost, if it was.
    pub lost_before: Option<i64>,
    /// Going forward from the clicked frame, the frame where the point was lost, if it was.
    pub lost_after: Option<i64>,
}

/// Decodes source frame `index` and turns it gray, ready for matching.
fn pyramid_at(decoder: &mut VideoDecoder, index: i64) -> Result<Pyramid, MediaError> {
    let frame = decoder.frame(index)?;
    let gray = GrayFrame::from_rgba(&frame.rgba, frame.width, frame.height).ok_or_else(|| MediaError(format!("frame {index} came back the wrong size")))?;
    Ok(Pyramid::new(gray))
}

/// The error for a job stopped part way (the caller ignores it, having asked to stop).
fn cancelled_error() -> MediaError {
    MediaError("Cancelled".into())
}

/// Follows the point `start` (fractions of the picture's width and height) from source frame
/// `from` forward to `to` (exclusive), and backward to `back_to`. Each way stops where the
/// point is lost. `progress` is told how far along it is (0–1).
pub fn track_point(path: &Path, start: Vec2, from: i64, back_to: i64, to: i64, progress: &dyn Fn(f32), cancelled: &dyn Fn() -> bool) -> Result<TrackedPoint, MediaError> {
    if !(back_to <= from && from < to) {
        return Err(MediaError(format!("frames {back_to}..{to} don't include frame {from}")));
    }
    if !(0.0..=1.0).contains(&start.x) || !(0.0..=1.0).contains(&start.y) {
        return Err(MediaError("the point to follow is outside the picture".into()));
    }
    let mut decoder = VideoDecoder::open(path, WIDTH, true)?;
    let first = pyramid_at(&mut decoder, from)?;
    let size = Vec2::new(first.frame().width() as f32, first.frame().height() as f32);
    let start_pixels = Vec2::new(start.x * size.x, start.y * size.y);
    let total = ((to - from - 1) + (from - back_to)).max(1) as f32;
    let mut done = 0.0;
    let mut result = TrackedPoint::default();
    // One direction at a time, each from the clicked frame.
    let directions: [Vec<i64>; 2] = [((from + 1)..to).collect(), (back_to..from).rev().collect()];
    for (direction, frames) in directions.iter().enumerate() {
        let mut follower = PointFollower::new(from, &first, start_pixels, TrackSettings::default())
            .ok_or_else(|| MediaError("There's no detail at that point to follow. Pick a spot with clear edges or texture.".into()))?;
        for &index in frames {
            if cancelled() {
                return Err(cancelled_error());
            }
            let still_found = follower.push(index, &pyramid_at(&mut decoder, index)?);
            done += 1.0;
            progress(done / total);
            if !still_found {
                break;
            }
        }
        let followed = follower.finish();
        let as_fraction = |&(index, point): &(i64, Vec2)| (index, Vec2::new(point.x / size.x, point.y / size.y));
        // Both directions start with the clicked frame: keep it once.
        let skip = usize::from(direction == 1);
        result.points.extend(followed.points.iter().skip(skip).map(as_fraction));
        if direction == 0 {
            result.lost_after = followed.lost_at;
        } else {
            result.lost_before = followed.lost_at;
        }
    }
    result.points.sort_by_key(|(index, _)| *index);
    Ok(result)
}

/// How the picture of a clip moves from frame to frame, measured on frames `width` × `height`.
#[derive(Clone, Debug, PartialEq)]
pub struct MotionAnalysis {
    pub width: u32,
    pub height: u32,
    /// The first source frame measured.
    pub first_frame: i64,
    /// Entry `i` is the motion from frame `first_frame + i` to the next, in pixels at `width`;
    /// `None` where it couldn't be measured (a cut, a blank or blurred frame).
    pub steps: Vec<Option<FrameMotion>>,
}

/// Measures the camera's motion across source frames `from..to` of the clip at `path`.
/// `progress` is told how far along it is (0–1).
pub fn measure_motion(path: &Path, from: i64, to: i64, progress: &dyn Fn(f32), cancelled: &dyn Fn() -> bool) -> Result<MotionAnalysis, MediaError> {
    if to - from < 2 {
        return Err(MediaError("the clip needs at least two frames to steady".into()));
    }
    let mut decoder = VideoDecoder::open(path, WIDTH, true)?;
    let mut previous = pyramid_at(&mut decoder, from)?;
    let (width, height) = (previous.frame().width() as u32, previous.frame().height() as u32);
    let mut steps = Vec::with_capacity((to - from - 1) as usize);
    for index in (from + 1)..to {
        if cancelled() {
            return Err(cancelled_error());
        }
        let next = pyramid_at(&mut decoder, index)?;
        steps.push(global_motion(&previous, &next));
        previous = next;
        progress((index - from) as f32 / (to - from - 1) as f32);
    }
    Ok(MotionAnalysis { width, height, first_frame: from, steps })
}

/// Tracks the middle of a test pattern square moving across a dark frame (from (150, 180),
/// 150 px a second right and 20 down, at 12 fps, in 640 × 360):
/// `TRACK_CLIP=<clip> cargo test -p studio-media tracks_a_real_clip -- --ignored`.
#[test]
#[ignore = "needs a clip; set TRACK_CLIP"]
fn tracks_a_real_clip() {
    let path = std::env::var("TRACK_CLIP").unwrap();
    let start = Vec2::new(150.0 / 640.0, 180.0 / 360.0);
    let tracked = track_point(Path::new(&path), start, 0, 0, 24, &|_| {}, &|| false).unwrap();
    assert_eq!(tracked.points.len(), 24);
    for (index, point) in tracked.points {
        let seconds = index as f32 / 12.0;
        let expected = Vec2::new((150.0 + seconds * 150.0) / 640.0, (180.0 + seconds * 20.0) / 360.0);
        assert!((point.x - expected.x).abs() < 0.01 && (point.y - expected.y).abs() < 0.015, "frame {index}: {point:?} vs {expected:?}");
    }
}
