//! What an export can be written as: a movie (H.264, H.265 or a ProRes flavor), an animated
//! GIF, a numbered sequence of PNG pictures, or a vector animation (Lottie, dotLottie or an
//! animated SVG). Also how an export at another frame rate than its composition's picks which
//! composition frame each output frame shows.

use std::ops::Range;

use studio_core::time::FrameRate;
use studio_media::MovieFormat;

/// The kinds of file an export can write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Movie(MovieFormat),
    /// A looping animated GIF, for chats and web pages. 256 colors a frame and no sound.
    Gif,
    /// One PNG picture per frame, with transparency, in a folder of their own: for other apps
    /// that build their own animation from frames.
    PngSequence,
    /// Lottie (Bodymovin JSON): shapes, text and keyframes for web and app animation players.
    Lottie,
    /// The same Lottie animation zipped with a manifest, as LottieFiles' players prefer.
    DotLottie,
    /// An SVG that animates itself with CSS, for web pages.
    AnimatedSvg,
}

/// GIF frame times are counted in hundredths of a second, and browsers slow down frames
/// shorter than two hundredths, so a GIF plays at most 50 frames a second.
pub const GIF_MAX_RATE: FrameRate = FrameRate::new(50, 1);

impl ExportFormat {
    /// Every format, in the order the Export dialog lists them.
    pub const ALL: [Self; 13] = [
        Self::Movie(MovieFormat::H264),
        Self::Movie(MovieFormat::H265),
        Self::Movie(MovieFormat::WebM),
        Self::Movie(MovieFormat::ProRes4444),
        Self::Movie(MovieFormat::ProRes422Hq),
        Self::Movie(MovieFormat::ProRes422),
        Self::Movie(MovieFormat::ProResLt),
        Self::Movie(MovieFormat::ProResProxy),
        Self::Gif,
        Self::PngSequence,
        Self::Lottie,
        Self::DotLottie,
        Self::AnimatedSvg,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Movie(format) => format.label(),
            Self::Gif => "Animated GIF",
            Self::PngSequence => "PNG sequence",
            Self::Lottie => "Lottie (.json)",
            Self::DotLottie => "dotLottie (.lottie)",
            Self::AnimatedSvg => "Animated SVG (.svg)",
        }
    }

    /// The file name extension, without the dot. A PNG sequence is a folder, so it has none.
    pub const fn extension(self) -> Option<&'static str> {
        match self {
            Self::Movie(format) => Some(format.extension()),
            Self::Gif => Some("gif"),
            Self::PngSequence => None,
            Self::Lottie => Some("json"),
            Self::DotLottie => Some("lottie"),
            Self::AnimatedSvg => Some("svg"),
        }
    }

    pub const fn movie(self) -> Option<MovieFormat> {
        match self {
            Self::Movie(format) => Some(format),
            _ => None,
        }
    }

    /// Whether see-through parts of the picture stay see-through. Otherwise they are laid over
    /// black. (GIF could keep fully clear pixels, but its hard edges look worse than black.)
    pub const fn has_alpha(self) -> bool {
        match self {
            Self::Movie(format) => format.has_alpha(),
            Self::Gif => false,
            Self::PngSequence | Self::Lottie | Self::DotLottie | Self::AnimatedSvg => true,
        }
    }

    /// Whether the file holds shapes and keyframes rather than rendered frames. Vector files
    /// draw at any size, so the size setting doesn't apply, and they hold no video or sound.
    pub const fn is_vector(self) -> bool {
        matches!(self, Self::Lottie | Self::DotLottie | Self::AnimatedSvg)
    }

    /// Whether the file can carry sound: only movies can.
    pub const fn has_sound(self) -> bool {
        matches!(self, Self::Movie(_))
    }

    /// Whether the quality and encoding speed settings apply (H.264, H.265 and WebM; every
    /// other format has a fixed quality).
    pub const fn has_quality(self) -> bool {
        matches!(self, Self::Movie(format) if !format.is_prores())
    }

    /// Whether the sound is compressed (AAC, or Opus in WebM), so its bitrate can be chosen.
    /// ProRes sound is uncompressed.
    pub const fn has_sound_bitrate(self) -> bool {
        self.has_quality()
    }

    /// The fastest frame rate the format plays at, if it has a limit.
    pub const fn max_frame_rate(self) -> Option<FrameRate> {
        match self {
            Self::Gif => Some(GIF_MAX_RATE),
            _ => None,
        }
    }

    /// Whether the format can play `rate`.
    pub fn allows_rate(self, rate: FrameRate) -> bool {
        self.max_frame_rate().is_none_or(|max| rate.fps() <= max.fps() + 1e-9)
    }
}

impl From<MovieFormat> for ExportFormat {
    fn from(format: MovieFormat) -> Self {
        Self::Movie(format)
    }
}

/// How many frames an export of composition `frames` at `output` frames a second has: the same
/// length of time, counted in output frames (at least one when there is anything to export).
pub fn output_frame_count(frames: &Range<i64>, composition: FrameRate, output: FrameRate) -> i64 {
    let length = i128::from((frames.end - frames.start).max(0));
    if length == 0 {
        return 0;
    }
    // length × (composition seconds per frame) × (output frames per second), rounded.
    let numerator = length * i128::from(composition.denominator) * i128::from(output.numerator);
    let denominator = i128::from(composition.numerator) * i128::from(output.denominator);
    (((2 * numerator + denominator) / (2 * denominator)) as i64).max(1)
}

/// The composition frame that output frame `index` shows: the one on screen at the moment that
/// output frame starts. Faster output rates show some frames twice; slower ones skip some.
pub fn source_frame(frames: &Range<i64>, composition: FrameRate, output: FrameRate, index: i64) -> i64 {
    // index × (output seconds per frame) × (composition frames per second), rounded down.
    let numerator = i128::from(index) * i128::from(output.denominator) * i128::from(composition.numerator);
    let denominator = i128::from(output.numerator) * i128::from(composition.denominator);
    (frames.start + (numerator / denominator) as i64).min(frames.end - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_rate_shows_every_frame_once() {
        let rate = FrameRate::new(30, 1);
        assert_eq!(output_frame_count(&(10..40), rate, rate), 30);
        let shown: Vec<i64> = (0..30).map(|index| source_frame(&(10..40), rate, rate, index)).collect();
        assert_eq!(shown, (10..40).collect::<Vec<_>>());
    }

    #[test]
    fn half_the_rate_skips_every_other_frame() {
        let (comp, out) = (FrameRate::new(30, 1), FrameRate::new(15, 1));
        assert_eq!(output_frame_count(&(0..30), comp, out), 15);
        assert_eq!((0..4).map(|index| source_frame(&(0..30), comp, out, index)).collect::<Vec<_>>(), [0, 2, 4, 6]);
    }

    #[test]
    fn a_faster_rate_repeats_frames_and_keeps_the_length() {
        let (comp, out) = (FrameRate::new(24, 1), FrameRate::new(60, 1));
        assert_eq!(output_frame_count(&(0..24), comp, out), 60, "one second either way");
        assert_eq!((0..6).map(|index| source_frame(&(0..24), comp, out, index)).collect::<Vec<_>>(), [0, 0, 0, 1, 1, 2]);
        assert_eq!(source_frame(&(0..24), comp, out, 59), 23, "never past the last frame");
    }

    #[test]
    fn drop_frame_rates_line_up_with_whole_ones() {
        // Ten seconds at 30 fps is 299.7 frames at 29.97, which rounds to 300.
        assert_eq!(output_frame_count(&(0..300), FrameRate::new(30, 1), FrameRate::new(30000, 1001)), 300);
        assert_eq!(output_frame_count(&(0..0), FrameRate::new(30, 1), FrameRate::new(24, 1)), 0);
        assert_eq!(output_frame_count(&(0..1), FrameRate::new(60, 1), FrameRate::new(12, 1)), 1, "never nothing");
    }

    #[test]
    fn gif_is_limited_to_fifty_frames_a_second() {
        assert!(ExportFormat::Gif.allows_rate(FrameRate::new(50, 1)));
        assert!(!ExportFormat::Gif.allows_rate(FrameRate::new(60, 1)));
        assert!(ExportFormat::Movie(MovieFormat::H264).allows_rate(FrameRate::new(120, 1)));
    }

    #[test]
    fn vector_formats_keep_transparency_and_carry_no_sound() {
        for format in [ExportFormat::Lottie, ExportFormat::DotLottie, ExportFormat::AnimatedSvg] {
            assert!(format.is_vector() && format.has_alpha() && !format.has_sound() && !format.has_quality());
        }
        assert_eq!(ExportFormat::Lottie.extension(), Some("json"));
        assert_eq!(ExportFormat::DotLottie.extension(), Some("lottie"));
        assert_eq!(ExportFormat::AnimatedSvg.extension(), Some("svg"));
        assert!(!ExportFormat::Gif.is_vector());
    }
}
