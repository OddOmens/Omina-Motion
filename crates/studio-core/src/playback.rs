//! Playback timing. The playhead is always derived from wall-clock time since
//! playback started, never by adding a frame per screen refresh. That keeps the
//! true frame rate on any display; when the machine falls behind, frames are
//! skipped rather than slowed down.

use crate::time::{FrameRange, FrameRate};

/// Where playback begins: the playhead, or the work area start if the playhead is outside the work area.
pub fn playback_start_frame(current_frame: i64, work_area: FrameRange) -> i64 {
    let (start, end) = (work_area.start as i64, work_area.end as i64);
    if (start..end).contains(&current_frame) { current_frame } else { start }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaybackPosition {
    pub frame: i64,
    /// True when playback reached the end of the work area and is not looping.
    pub ended: bool,
}

/// The frame to show `elapsed_ms` after playback started at `start_frame`.
/// Playback stays inside the work area, looping to its start or stopping on its last frame.
pub fn playback_position(
    start_frame: i64,
    elapsed_ms: f64,
    rate: FrameRate,
    work_area: FrameRange,
    looping: bool,
) -> PlaybackPosition {
    // Multiply before dividing so fractional rates such as 24000/1001 stay exact.
    let elapsed_frames =
        (elapsed_ms.max(0.0) * f64::from(rate.numerator) / (1000.0 * f64::from(rate.denominator))).floor() as i64;
    let frame = start_frame + elapsed_frames;
    let (start, end) = (work_area.start as i64, work_area.end as i64);
    if frame < end {
        return PlaybackPosition { frame, ended: false };
    }
    if !looping {
        return PlaybackPosition { frame: end - 1, ended: true };
    }
    let len = (end - start).max(1);
    PlaybackPosition { frame: start + (frame - start) % len, ended: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FPS_24: FrameRate = FrameRate::new(24, 1);
    const WORK_AREA: FrameRange = FrameRange::new(24.0, 72.0);

    #[test]
    fn starts_inside_the_work_area() {
        assert_eq!(playback_start_frame(30, WORK_AREA), 30);
        assert_eq!(playback_start_frame(0, WORK_AREA), 24);
        assert_eq!(playback_start_frame(72, WORK_AREA), 24);
    }

    #[test]
    fn advances_at_the_frame_rate() {
        assert_eq!(playback_position(24, 0.0, FPS_24, WORK_AREA, true).frame, 24);
        assert_eq!(playback_position(24, 41.0, FPS_24, WORK_AREA, true).frame, 24);
        assert_eq!(playback_position(24, 42.0, FPS_24, WORK_AREA, true).frame, 25);
        assert_eq!(playback_position(24, 1000.0, FPS_24, WORK_AREA, true).frame, 48);
    }

    #[test]
    fn fractional_rates_stay_exact() {
        // 23.976 fps plays exactly 24000 frames in 1001 seconds.
        let rate = FrameRate::new(24000, 1001);
        let long = FrameRange::new(0.0, 100_000.0);
        assert_eq!(playback_position(0, 1_001_000.0, rate, long, false).frame, 24_000);
    }

    #[test]
    fn loops_or_stops_at_the_end() {
        let looped = playback_position(24, 2500.0, FPS_24, WORK_AREA, true);
        assert_eq!(looped, PlaybackPosition { frame: 36, ended: false });
        let stopped = playback_position(24, 5000.0, FPS_24, WORK_AREA, false);
        assert_eq!(stopped, PlaybackPosition { frame: 71, ended: true });
    }

    #[test]
    fn negative_elapsed_time_is_treated_as_zero() {
        assert_eq!(playback_position(30, -16.0, FPS_24, WORK_AREA, true).frame, 30);
    }
}
