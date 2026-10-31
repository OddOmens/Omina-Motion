//! Frame-based time. Time is always a whole frame number; frame rates are
//! exact fractions so rates such as 23.976 (24000/1001) never drift.

/// A frame rate as an exact fraction, e.g. 30000/1001 for 29.97 fps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FrameRate {
    pub numerator: u32,
    pub denominator: u32,
}

impl FrameRate {
    pub const fn new(numerator: u32, denominator: u32) -> Self {
        Self { numerator, denominator }
    }

    /// Frames per second as a decimal, for display and coarse conversions only.
    pub fn fps(self) -> f64 {
        f64::from(self.numerator) / f64::from(self.denominator)
    }

    /// The whole-number frame count timecode uses (30 for 29.97). Timecode counts nominal frames.
    pub fn timecode_base(self) -> i64 {
        self.fps().round() as i64
    }

    /// Converts a duration in seconds to the nearest whole number of frames.
    pub fn seconds_to_frames(self, seconds: f64) -> i64 {
        (seconds * self.fps()).round() as i64
    }

    /// Display label such as "24 fps" or "23.976 fps".
    pub fn label(self) -> String {
        let fps = self.fps();
        if fps.fract() == 0.0 { format!("{fps} fps") } else { format!("{fps:.3} fps") }
    }
}

/// A half-open range of frames, `start..end`.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FrameRange {
    pub start: f64,
    pub end: f64,
}

impl FrameRange {
    pub const fn new(start: f64, end: f64) -> Self {
        Self { start, end }
    }

    pub fn len(self) -> f64 {
        self.end - self.start
    }

    pub fn is_empty(self) -> bool {
        self.len() <= 0.0
    }

    /// Keeps the range inside `0..duration` and at least `min_len` long, preserving its length where possible.
    pub fn clamped(self, duration: f64, min_len: f64) -> Self {
        let len = self.len().clamp(min_len.min(duration), duration);
        let start = self.start.clamp(0.0, duration - len);
        Self::new(start, start + len)
    }
}

/// Formats a frame as After Effects-style non-drop-frame timecode, "H:MM:SS:FF".
pub fn format_timecode(frame: i64, rate: FrameRate) -> String {
    let base = rate.timecode_base();
    let sign = if frame < 0 { "-" } else { "" };
    let total = frame.abs();
    let frames = total % base;
    let total_seconds = total / base;
    let seconds = total_seconds % 60;
    let minutes = (total_seconds / 60) % 60;
    let hours = total_seconds / 3600;
    format!("{sign}{hours}:{minutes:02}:{seconds:02}:{frames:02}")
}

/// Picks a ruler label spacing (in frames) so labels are at least `min_spacing_px` apart.
/// Sub-second steps always divide one second evenly so labels stay on whole seconds.
pub fn ruler_tick_step(pixels_per_frame: f64, rate: FrameRate, min_spacing_px: f64) -> i64 {
    let base = rate.timecode_base();
    let frame_steps = [1, 2, 3, 4, 5, 6, 8, 10, 12, 15].into_iter().filter(|step| *step < base && base % step == 0);
    let second_steps = [1, 2, 5, 10, 30, 60, 300].into_iter().map(|seconds| seconds * base);
    let mut candidates = frame_steps.chain(second_steps);
    let last = 300 * base;
    candidates.find(|step| *step as f64 * pixels_per_frame >= min_spacing_px).unwrap_or(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FPS_24: FrameRate = FrameRate::new(24, 1);
    const NTSC: FrameRate = FrameRate::new(30000, 1001);

    #[test]
    fn timecode_rolls_over_units() {
        assert_eq!(format_timecode(0, FPS_24), "0:00:00:00");
        assert_eq!(format_timecode(23, FPS_24), "0:00:00:23");
        assert_eq!(format_timecode(24 * 61 + 5, FPS_24), "0:01:01:05");
        assert_eq!(format_timecode(24 * 3600, FPS_24), "1:00:00:00");
        assert_eq!(format_timecode(-25, FPS_24), "-0:00:01:01");
        assert_eq!(format_timecode(30, NTSC), "0:00:01:00");
    }

    #[test]
    fn frame_rate_labels() {
        assert_eq!(FPS_24.label(), "24 fps");
        assert_eq!(NTSC.label(), "29.970 fps");
    }

    #[test]
    fn ruler_steps_stay_on_whole_seconds() {
        for pixels_per_frame in 1..=70 {
            for rate in [FPS_24, NTSC] {
                let step = ruler_tick_step(f64::from(pixels_per_frame), rate, 70.0);
                let base = rate.timecode_base();
                let aligned = if step < base { base % step == 0 } else { step % base == 0 };
                assert!(aligned, "step {step} at {pixels_per_frame}px/frame");
            }
        }
        assert_eq!(ruler_tick_step(100.0, FPS_24, 70.0), 1);
        assert_eq!(ruler_tick_step(0.01, FPS_24, 70.0), 24 * 300);
    }

    #[test]
    fn ranges_clamp_inside_duration() {
        assert_eq!(FrameRange::new(-10.0, 50.0).clamped(240.0, 10.0), FrameRange::new(0.0, 60.0));
        assert_eq!(FrameRange::new(230.0, 300.0).clamped(240.0, 10.0), FrameRange::new(170.0, 240.0));
        assert_eq!(FrameRange::new(5.0, 6.0).clamped(240.0, 10.0), FrameRange::new(5.0, 15.0));
    }
}
