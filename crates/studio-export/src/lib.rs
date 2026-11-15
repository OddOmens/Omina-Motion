//! Exports a composition to a movie, an animated GIF, a PNG sequence or a vector animation.
//! For the frame-based formats, each frame is rendered on the GPU from full-quality source
//! frames (never proxies), the sound is mixed to match, and both are encoded together. The
//! vector formats (Lottie, dotLottie, animated SVG) are written whole by `studio_vector`; see
//! `vector`. The output can run at another frame rate than the
//! composition (see `format::source_frame`). The file is written under a temporary name and
//! only takes its real name once complete, so a cancelled or failed export never leaves a
//! broken file behind.

mod format;
mod sink;
mod vector;

use std::ops::Range;
use std::path::{Path, PathBuf};

use studio_core::audio::composition_audio;
use studio_core::model::{Id, Project};
use studio_core::time::FrameRate;
use studio_media::{CHANNELS, EncodeSpeed, ExactFrames, MixRange, Mixer, MovieSettings, VideoQuality};
use studio_render::Renderer;

pub use format::{ExportFormat, GIF_MAX_RATE, output_frame_count, source_frame};
pub use sink::discard;
use sink::Sink;

/// Sample rate of exported sound: the video standard.
pub const AUDIO_RATE: u32 = 48_000;
/// The largest frame the renderer draws.
pub const MAX_SIZE: u32 = 8192;

/// Straight-alpha BGRA pixels laid over black, as an opaque picture. The mix is done in linear
/// light, as the renderer composites, so a fade looks the same as it does over a black background
/// in the viewer.
fn over_black(bgra: &[u8]) -> Vec<u8> {
    let decode: Vec<f32> = (0..=255u8).map(|value| srgb_to_linear(f32::from(value) / 255.0)).collect();
    bgra.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| {
            let alpha = f32::from(pixel[3]) / 255.0;
            let channel = |value: u8| if pixel[3] == 255 { value } else { linear_to_srgb_byte(decode[usize::from(value)] * alpha) };
            [channel(pixel[0]), channel(pixel[1]), channel(pixel[2]), 255]
        })
        .collect()
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb_byte(linear: f32) -> u8 {
    let c = linear.clamp(0.0, 1.0);
    let srgb = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (srgb * 255.0).round() as u8
}

/// What to export, and where.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportRequest {
    pub composition: Id,
    /// Composition frames to export: the work area or the whole composition.
    pub frames: Range<i64>,
    pub format: ExportFormat,
    /// The output's frame rate, or `None` for the composition's own.
    pub frame_rate: Option<FrameRate>,
    /// Adds a sound track when the composition has audible layers.
    pub include_audio: bool,
    pub path: PathBuf,
    /// The movie's size as a fraction of the composition's (1 for full size).
    pub scale: f32,
    /// H.264 and H.265 only.
    pub quality: VideoQuality,
    /// H.264 and H.265 only.
    pub speed: EncodeSpeed,
    /// AAC sound bitrate (H.264 and H.265 movies), in kilobits per second.
    pub audio_kbps: u32,
}

impl ExportRequest {
    /// A full-size, high-quality export of `frames` with sound.
    pub fn new(composition: Id, frames: Range<i64>, format: impl Into<ExportFormat>, path: PathBuf) -> Self {
        Self {
            composition,
            frames,
            format: format.into(),
            frame_rate: None,
            include_audio: true,
            path,
            scale: 1.0,
            quality: VideoQuality::High,
            speed: EncodeSpeed::Balanced,
            audio_kbps: 320,
        }
    }

    /// The output's frame rate for a composition running at `composition`: the one chosen, or
    /// the composition's, slowed to what the format can play (GIFs top out at 50).
    pub fn output_rate(&self, composition: FrameRate) -> FrameRate {
        let rate = self.frame_rate.unwrap_or(composition);
        match self.format.max_frame_rate() {
            Some(max) if !self.format.allows_rate(rate) => max,
            _ => rate,
        }
    }

    /// How many frames the output has, for a composition running at `composition`.
    pub fn output_frame_count(&self, composition: FrameRate) -> i64 {
        output_frame_count(&self.frames, composition, self.output_rate(composition))
    }

    /// The movie's picture size for a composition of `width` × `height`: scaled, at least 16
    /// pixels a side, and even (both codecs need even sizes). Vector files keep the
    /// composition's own size, since they draw at any size.
    pub fn output_size(&self, width: u32, height: u32) -> (u32, u32) {
        if self.format.is_vector() {
            return (width, height);
        }
        let scale = if self.scale.is_finite() { self.scale.clamp(0.05, 1.0) } else { 1.0 };
        let side = |value: u32| (((value as f32 * scale).round() as u32).max(16) + 1) & !1;
        (side(width), side(height))
    }
}

/// How far an export has got, in frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub done: i64,
    pub total: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The file is written. `left_out` has a plain-English line for each kind of thing the
    /// format couldn't hold (video in a Lottie file, say); it is empty for the frame formats.
    Finished { left_out: Vec<String> },
    Cancelled,
}

/// Exports `request` from `project`. Calls `progress` after every frame and stops early if
/// `cancelled` says so.
pub fn export(
    project: &Project,
    request: &ExportRequest,
    progress: &mut dyn FnMut(Progress),
    cancelled: &dyn Fn() -> bool,
) -> Result<Outcome, String> {
    if request.format.is_vector() {
        return vector::export(project, request, progress, cancelled);
    }
    let composition = project.composition(request.composition).ok_or("The composition no longer exists.")?;
    if composition.width > MAX_SIZE || composition.height > MAX_SIZE {
        return Err(format!("Compositions larger than {MAX_SIZE} × {MAX_SIZE} pixels can’t be exported yet."));
    }
    let frames = request.frames.start.max(0)..request.frames.end.min(composition.duration_frames);
    if frames.is_empty() {
        return Err("There are no frames to export.".into());
    }
    let clips = if request.include_audio && request.format.has_sound() { composition_audio(project, composition) } else { Vec::new() };
    let rate = composition.frame_rate;
    let output_rate = request.output_rate(rate);
    let (width, height) = request.output_size(composition.width, composition.height);
    // Movies need a movie format for their settings; the others only read the size and rate.
    let movie_format = request.format.movie().unwrap_or(studio_media::MovieFormat::H264);
    let settings = MovieSettings {
        quality: request.quality,
        speed: request.speed,
        audio_kbps: request.audio_kbps,
        ..MovieSettings::new(movie_format, width, height, output_rate, (!clips.is_empty()).then_some(AUDIO_RATE))
    };
    let total = output_frame_count(&frames, rate, output_rate);

    let mut renderer = Renderer::new().map_err(|error| format!("The GPU could not be used: {error}"))?;
    let temporary = temporary_path(&request.path);
    let mut sink = Sink::create(request.format, &temporary, &request.path, settings, total)?;
    let result = (|| {
        let video = ExactFrames::new();
        // Sound for each output frame: from where the previous frame's sound ended up to the
        // moment this frame ends.
        let start_seconds = frames.start as f64 / rate.fps();
        let to_sample = |seconds: f64| (seconds * f64::from(AUDIO_RATE)).round() as i64;
        let output_end = |index: i64| start_seconds + index as f64 / output_rate.fps();
        let range = MixRange { start: to_sample(start_seconds), end: to_sample(frames.end as f64 / rate.fps()), looping: false };
        let mut mixer = Mixer::new(AUDIO_RATE, range, range.start);
        let mut mixed = range.start;
        let mut sound = Vec::new();

        for index in 0..total {
            if cancelled() {
                return Ok(Outcome::Cancelled);
            }
            let frame = source_frame(&frames, rate, output_rate, index);
            let picture = renderer
                .render(project, request.composition, frame, width, height, &video)
                .map_err(|error| format!("Frame {frame} could not be rendered: {error}"))?;
            // A source frame that could not be decoded would show the previous one instead: stop.
            if let Some((path, error)) = video.errors().into_iter().next() {
                return Err(format!("{} could not be decoded ({error}).", path.display()));
            }
            // Most formats have no transparency, so a transparent composition is laid over
            // black; ProRes 4444 and PNG keep it.
            let pixels = if request.format.has_alpha() { picture.bgra } else { over_black(&picture.bgra) };
            sink.write_video(&pixels, picture.width, picture.height).map_err(|error| format!("Frame {frame} could not be encoded: {error}"))?;
            if !clips.is_empty() {
                let until = to_sample(output_end(index + 1)).min(range.end).max(mixed);
                sound.resize((until - mixed) as usize * CHANNELS, 0.0);
                let written = mixer.mix(&clips, &mut sound);
                sink.write_audio(&sound[..written * CHANNELS]).map_err(|error| format!("The sound could not be encoded: {error}"))?;
                mixed = until;
            }
            progress(Progress { done: index + 1, total });
        }
        Ok(Outcome::Finished { left_out: Vec::new() })
    })();

    match result {
        Ok(Outcome::Finished { left_out }) => {
            let saved = sink.finish(&temporary, &request.path);
            if saved.is_err() {
                discard(&temporary);
            }
            saved.map(|()| Outcome::Finished { left_out })
        }
        other => {
            drop(sink);
            discard(&temporary);
            other
        }
    }
}

/// Saves one frame of a composition as a PNG picture at `path`, at the composition's full
/// size and with its transparency kept. Footage is decoded at full quality, as in an export.
pub fn save_frame(project: &Project, composition: Id, frame: i64, path: &Path) -> Result<(), String> {
    let settings = project.composition(composition).ok_or("The composition no longer exists.")?;
    if settings.width > MAX_SIZE || settings.height > MAX_SIZE {
        return Err(format!("Compositions larger than {MAX_SIZE} × {MAX_SIZE} pixels can’t be exported yet."));
    }
    let (width, height) = (settings.width, settings.height);
    let mut renderer = Renderer::new().map_err(|error| format!("The GPU could not be used: {error}"))?;
    let video = ExactFrames::new();
    let picture = renderer.render(project, composition, frame, width, height, &video).map_err(|error| format!("The frame could not be rendered: {error}"))?;
    if let Some((source, error)) = video.errors().into_iter().next() {
        return Err(format!("{} could not be decoded ({error}).", source.display()));
    }
    let picture = image::RgbaImage::from_raw(picture.width, picture.height, sink::to_rgba(&picture.bgra)).ok_or("The frame is smaller than its size says.")?;
    // Written under a temporary name first, so a picture that fails halfway leaves nothing behind.
    let temporary = temporary_path(path);
    let saved = picture
        .save_with_format(&temporary, image::ImageFormat::Png)
        .map_err(|error| error.to_string())
        .and_then(|()| std::fs::rename(&temporary, path).map_err(|error| error.to_string()));
    if saved.is_err() {
        discard(&temporary);
    }
    saved.map_err(|error| format!("The picture could not be saved: {error}"))
}

/// A hidden file next to the final one, so the rename at the end is a single, atomic step. An
/// app that crashes mid-export can find it here afterwards to clean it up.
pub fn temporary_path(path: &Path) -> PathBuf {
    let name = path.file_name().map_or_else(|| "export".into(), |name| name.to_string_lossy().into_owned());
    path.with_file_name(format!(".{name}.exporting"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_transparent_white_over_black_matches_the_renderer() {
        // The renderer draws 50% white over black as 188 (half the light, sRGB encoded).
        let flat = over_black(&[255, 255, 255, 128, 10, 20, 30, 255, 200, 100, 50, 0]);
        assert!(flat[..3].iter().all(|&c| c.abs_diff(188) <= 1), "{:?}", &flat[..3]);
        assert_eq!(&flat[4..8], &[10, 20, 30, 255], "opaque pixels are untouched");
        assert_eq!(&flat[8..12], &[0, 0, 0, 255], "transparent ones are black");
    }
}
