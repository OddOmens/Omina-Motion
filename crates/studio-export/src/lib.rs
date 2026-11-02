//! Exports a composition to a movie file. Each frame is rendered on the GPU at the
//! composition's full size from full-quality source frames (never proxies), the sound is
//! mixed to match, and both are encoded together. The file is written under a temporary
//! name and only takes its real name once complete, so a cancelled or failed export never
//! leaves a broken movie behind.

use std::ops::Range;
use std::path::{Path, PathBuf};

use studio_core::audio::composition_audio;
use studio_core::model::{Id, Project};
use studio_media::{CHANNELS, EncodeSpeed, ExactFrames, MixRange, Mixer, MovieFormat, MovieSettings, MovieWriter, VideoQuality};
use studio_render::Renderer;

/// Sample rate of exported sound: the video standard.
pub const AUDIO_RATE: u32 = 48_000;
/// The largest frame the renderer draws.
pub const MAX_SIZE: u32 = 8192;

/// Straight-alpha BGRA pixels laid over black, as an opaque picture.
fn over_black(bgra: &[u8]) -> Vec<u8> {
    bgra.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| {
            let alpha = u16::from(pixel[3]);
            let channel = |value: u8| ((u16::from(value) * alpha + 127) / 255) as u8;
            [channel(pixel[0]), channel(pixel[1]), channel(pixel[2]), 255]
        })
        .collect()
}

/// What to export, and where.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportRequest {
    pub composition: Id,
    /// Composition frames to export: the work area or the whole composition.
    pub frames: Range<i64>,
    pub format: MovieFormat,
    /// Adds a sound track when the composition has audible layers.
    pub include_audio: bool,
    pub path: PathBuf,
    /// The movie's size as a fraction of the composition's (1 for full size).
    pub scale: f32,
    /// H.264 only.
    pub quality: VideoQuality,
    /// H.264 only.
    pub speed: EncodeSpeed,
    /// AAC sound bitrate (H.264 movies), in kilobits per second.
    pub audio_kbps: u32,
}

impl ExportRequest {
    /// A full-size, high-quality export of `frames` with sound.
    pub fn new(composition: Id, frames: Range<i64>, format: MovieFormat, path: PathBuf) -> Self {
        Self { composition, frames, format, include_audio: true, path, scale: 1.0, quality: VideoQuality::High, speed: EncodeSpeed::Balanced, audio_kbps: 320 }
    }

    /// The movie's picture size for a composition of `width` × `height`: scaled, at least 16
    /// pixels a side, and even (both codecs need even sizes).
    pub fn output_size(&self, width: u32, height: u32) -> (u32, u32) {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Finished,
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
    let composition = project.composition(request.composition).ok_or("The composition no longer exists.")?;
    if composition.width > MAX_SIZE || composition.height > MAX_SIZE {
        return Err(format!("Compositions larger than {MAX_SIZE} × {MAX_SIZE} pixels can’t be exported yet."));
    }
    let frames = request.frames.start.max(0)..request.frames.end.min(composition.duration_frames);
    if frames.is_empty() {
        return Err("There are no frames to export.".into());
    }
    let clips = if request.include_audio { composition_audio(project, composition) } else { Vec::new() };
    let rate = composition.frame_rate;
    let (width, height) = request.output_size(composition.width, composition.height);
    let settings = MovieSettings {
        quality: request.quality,
        speed: request.speed,
        audio_kbps: request.audio_kbps,
        ..MovieSettings::new(request.format, width, height, rate, (!clips.is_empty()).then_some(AUDIO_RATE))
    };

    let mut renderer = Renderer::new().map_err(|error| format!("The GPU could not be used: {error}"))?;
    let temporary = temporary_path(&request.path);
    let mut writer = MovieWriter::create(&temporary, settings).map_err(|error| {
        // FFmpeg may have created the file before failing (an encoder refusing the size, say).
        let _ = std::fs::remove_file(&temporary);
        format!("The movie could not be created: {error}")
    })?;
    let result = (|| {
        let video = ExactFrames::new();
        // Sound for each frame: from where the previous frame's sound ended up to this frame's end.
        let to_sample = |frame: i64| (frame as f64 / rate.fps() * f64::from(AUDIO_RATE)).round() as i64;
        let range = MixRange { start: to_sample(frames.start), end: to_sample(frames.end), looping: false };
        let mut mixer = Mixer::new(AUDIO_RATE, range, range.start);
        let mut mixed = to_sample(frames.start);
        let mut sound = Vec::new();
        let total = frames.end - frames.start;

        for frame in frames.clone() {
            if cancelled() {
                return Ok(Outcome::Cancelled);
            }
            let picture = renderer
                .render(project, request.composition, frame, width, height, &video)
                .map_err(|error| format!("Frame {frame} could not be rendered: {error}"))?;
            // A source frame that could not be decoded would show the previous one instead: stop.
            if let Some((path, error)) = video.errors().into_iter().next() {
                return Err(format!("{} could not be decoded ({error}).", path.display()));
            }
            // Most movies have no transparency, so a transparent composition is laid over black;
            // ProRes 4444 keeps it.
            let pixels = if request.format.has_alpha() { picture.bgra } else { over_black(&picture.bgra) };
            writer.write_video(&pixels, picture.width, picture.height).map_err(|error| format!("Frame {frame} could not be encoded: {error}"))?;
            if !clips.is_empty() {
                let until = to_sample(frame + 1);
                sound.resize((until - mixed) as usize * CHANNELS, 0.0);
                let written = mixer.mix(&clips, &mut sound);
                writer.write_audio(&sound[..written * CHANNELS]).map_err(|error| format!("The sound could not be encoded: {error}"))?;
                mixed = until;
            }
            progress(Progress { done: frame - frames.start + 1, total });
        }
        Ok(Outcome::Finished)
    })();

    match result {
        Ok(Outcome::Finished) => {
            let saved = writer
                .finish()
                .map_err(|error| format!("The movie could not be finished: {error}"))
                .and_then(|()| std::fs::rename(&temporary, &request.path).map_err(|error| format!("The movie could not be saved: {error}")));
            if saved.is_err() {
                let _ = std::fs::remove_file(&temporary);
            }
            saved.map(|()| Outcome::Finished)
        }
        other => {
            drop(writer);
            let _ = std::fs::remove_file(&temporary);
            other
        }
    }
}

/// A hidden file next to the final one, so the rename at the end is a single, atomic step.
fn temporary_path(path: &Path) -> PathBuf {
    let name = path.file_name().map_or_else(|| "export".into(), |name| name.to_string_lossy().into_owned());
    path.with_file_name(format!(".{name}.exporting"))
}
