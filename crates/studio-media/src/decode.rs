//! Probing media files and decoding video frames with FFmpeg.
//!
//! Decoding uses the GPU's video decoder when available (see `hardware`), and FFmpeg's
//! multi-threaded software decoders otherwise.

use std::path::{Path, PathBuf};

use ffmpeg_next as ffmpeg;
use ffmpeg::format::Pixel;
use ffmpeg::media::Type;
use ffmpeg::software::scaling;
use studio_core::media::VideoFrame;
use studio_core::time::FrameRate;

#[derive(Debug, Clone, PartialEq)]
pub struct MediaError(pub String);

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MediaError {}

impl From<ffmpeg::Error> for MediaError {
    fn from(error: ffmpeg::Error) -> Self {
        Self(error.to_string())
    }
}

/// Facts about a media file, read without decoding it.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaInfo {
    pub width: u32,
    pub height: u32,
    pub duration_seconds: f64,
    pub frame_rate: FrameRate,
    pub frame_count: i64,
    pub has_audio: bool,
    pub codec: String,
}

/// Initializes FFmpeg once. Safe to call repeatedly.
pub(crate) fn init() -> Result<(), MediaError> {
    static INIT: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    INIT.get_or_init(|| {
        ffmpeg::init().map_err(|error| error.to_string())?;
        // FFmpeg's own log goes to stderr; only show real errors.
        ffmpeg::log::set_level(ffmpeg::log::Level::Error);
        Ok(())
    })
    .clone()
    .map_err(MediaError)
}

/// A rational frame rate as an exact fraction, falling back to 24 fps if the file has none.
fn frame_rate_of(rate: ffmpeg::Rational) -> FrameRate {
    let (numerator, denominator) = (rate.numerator(), rate.denominator());
    if numerator > 0 && denominator > 0 { FrameRate::new(numerator as u32, denominator as u32) } else { FrameRate::new(24, 1) }
}

/// Reads a file's video size, length, frame rate and whether it has audio.
/// Whether a video stream's pixels carry transparency (four components, or two for gray).
pub(crate) fn stream_has_alpha(stream: &ffmpeg::Stream<'_>) -> Result<bool, MediaError> {
    let decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())?.decoder().video()?;
    Ok(matches!(decoder.format().descriptor().map(|format| format.nb_components()), Some(2 | 4)))
}

pub fn probe(path: &Path) -> Result<MediaInfo, MediaError> {
    init()?;
    let input = ffmpeg::format::input(path)?;
    let stream = input.streams().best(Type::Video).ok_or_else(|| MediaError("the file has no video".into()))?;
    let parameters = ffmpeg::codec::context::Context::from_parameters(stream.parameters())?;
    let decoder = parameters.decoder().video()?;
    let frame_rate = frame_rate_of(stream.avg_frame_rate());
    // Prefer the stream's own duration; fall back to the container's (microseconds).
    let duration_seconds = if stream.duration() > 0 {
        stream.duration() as f64 * f64::from(stream.time_base())
    } else {
        input.duration().max(0) as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE)
    };
    let frame_count = if stream.frames() > 0 { stream.frames() } else { frame_rate.seconds_to_frames(duration_seconds) };
    Ok(MediaInfo {
        width: decoder.width(),
        height: decoder.height(),
        duration_seconds,
        frame_rate,
        frame_count: frame_count.max(1),
        has_audio: input.streams().best(Type::Audio).is_some(),
        codec: decoder.codec().map_or_else(|| "unknown".to_owned(), |codec| codec.name().to_owned()),
    })
}

/// Decodes frames of one video file, converting them to RGBA at most `max_width` pixels wide.
pub struct VideoDecoder {
    path: PathBuf,
    input: ffmpeg::format::context::Input,
    stream_index: usize,
    decoder: ffmpeg::decoder::Video,
    /// Converts decoded pictures to RGBA; rebuilt if the decoded format changes (hardware frames
    /// arrive as NV12 or P010 only after download).
    scaler: Option<(scaling::Context, Pixel, u32, u32)>,
    output_size: (u32, u32),
    /// Kept alive while the decoder uses it.
    hardware: Option<crate::hardware::HardwareDevice>,
    /// Seconds per unit of the stream's timestamps.
    time_base: f64,
    start_seconds: f64,
    frame_rate: FrameRate,
    frame_count: i64,
    /// The source frame the next decoded picture will be, if decoding is moving forward from a known spot.
    next_index: Option<i64>,
    /// The file's final picture, once decoding has reached the end: shown for any later frame,
    /// since files can claim more frames than they hold.
    final_picture: Option<VideoFrame>,
}

/// Jumps shorter than this decode forward instead of seeking, which is faster for small skips.
const FORWARD_DECODE_LIMIT: i64 = 24;

impl VideoDecoder {
    /// Opens `path`, decoding at most `max_width` wide. With `use_hardware`, the GPU's video
    /// decoder is used if the platform has one for this codec.
    pub fn open(path: &Path, max_width: u32, use_hardware: bool) -> Result<Self, MediaError> {
        init()?;
        let input = ffmpeg::format::input(path)?;
        let stream = input.streams().best(Type::Video).ok_or_else(|| MediaError("the file has no video".into()))?;
        let stream_index = stream.index();
        let mut context = ffmpeg::codec::context::Context::from_parameters(stream.parameters())?;
        // GPU decoders hand back pictures without alpha, so files with transparency are decoded
        // in software.
        let use_hardware = use_hardware && !stream_has_alpha(&stream)?;
        let hardware = if use_hardware { crate::hardware::attach(&mut context) } else { None };
        if hardware.is_none() {
            // Software decoding: use all cores.
            context.set_threading(ffmpeg::codec::threading::Config::kind(ffmpeg::codec::threading::Type::Frame));
        }
        let decoder = context.decoder().video()?;
        let (width, height) = (decoder.width(), decoder.height());
        if width == 0 || height == 0 {
            return Err(MediaError("the video has no picture size".into()));
        }
        // Keep the aspect ratio; round to even sizes, which scalers prefer.
        let out_width = width.min(max_width.max(2)) & !1;
        let out_height = ((u64::from(height) * u64::from(out_width) / u64::from(width)) as u32).max(2) & !1;
        let time_base = f64::from(stream.time_base());
        let start_seconds = if stream.start_time() == ffmpeg::ffi::AV_NOPTS_VALUE { 0.0 } else { stream.start_time() as f64 * time_base };
        let info = probe(path)?;
        Ok(Self {
            path: path.to_owned(),
            input,
            stream_index,
            decoder,
            scaler: None,
            output_size: (out_width, out_height),
            hardware,
            time_base,
            start_seconds,
            frame_rate: info.frame_rate,
            frame_count: info.frame_count,
            next_index: Some(0),
            final_picture: None,
        })
    }

    /// Opens `path` with whichever decoder is faster for this file: software, or the GPU's video
    /// decoder if the platform has one. Each is timed on the first few frames; which wins depends
    /// on the codec, resolution and bitrate (multi-core software decoding beats the hardware on
    /// light streams; hardware tends to win on heavy ones), so it is measured rather than assumed.
    pub fn open_fastest(path: &Path, max_width: u32) -> Result<Self, MediaError> {
        let software = Self::open(path, max_width, false)?;
        let Ok(hardware) = Self::open(path, max_width, true) else { return Ok(software) };
        if hardware.hardware.is_none() {
            return Ok(software);
        }
        let sample = |mut decoder: Self| -> Result<(Self, f64), MediaError> {
            let frames = decoder.frame_count.min(8);
            let started = std::time::Instant::now();
            for index in 0..frames {
                decoder.frame(index)?;
            }
            Ok((decoder, started.elapsed().as_secs_f64()))
        };
        let (software, software_time) = sample(software)?;
        match sample(hardware) {
            Ok((hardware, hardware_time)) if hardware_time < software_time => Ok(hardware),
            _ => Ok(software),
        }
    }

    pub fn frame_count(&self) -> i64 {
        self.frame_count
    }

    /// Which decoder is in use: the hardware backend's name, or "software".
    pub fn backend(&self) -> &'static str {
        self.hardware.as_ref().map_or("software", |device| device.name)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The source frame number of a decoded picture, from its timestamp.
    fn index_of(&self, frame: &ffmpeg::frame::Video) -> Option<i64> {
        let pts = frame.timestamp().or(frame.pts())?;
        let seconds = pts as f64 * self.time_base - self.start_seconds;
        Some((seconds * self.frame_rate.fps()).round() as i64)
    }

    /// Decodes the next picture in stream order. `Ok(None)` at the end of the file.
    fn decode_next(&mut self) -> Result<Option<ffmpeg::frame::Video>, MediaError> {
        let mut decoded = ffmpeg::frame::Video::empty();
        loop {
            if self.decoder.receive_frame(&mut decoded).is_ok() {
                return Ok(Some(decoded));
            }
            // Feed the decoder another packet of our stream, or flush it at the end.
            let mut fed = false;
            for (stream, packet) in self.input.packets() {
                if stream.index() == self.stream_index {
                    match self.decoder.send_packet(&packet) {
                        // A damaged packet: skip it and keep going, as players do.
                        Err(ffmpeg::Error::InvalidData) => continue,
                        result => result?,
                    }
                    fed = true;
                    break;
                }
            }
            if !fed {
                // End of file: drain what the decoder still holds.
                let _ = self.decoder.send_eof();
                return Ok(self.decoder.receive_frame(&mut decoded).is_ok().then_some(decoded));
            }
        }
    }

    /// Moves to `seconds` into the stream (from its start), landing at or before it.
    fn seek_to(&mut self, seconds: f64) -> Result<(), MediaError> {
        let target = ((self.start_seconds + seconds.max(0.0)) * f64::from(ffmpeg::ffi::AV_TIME_BASE)) as i64;
        // Land on the keyframe at or before the target (FFmpeg uses the range end as the latest allowed point).
        self.input.seek(target, ..target)?;
        self.decoder.flush();
        self.next_index = None;
        Ok(())
    }

    /// Seeks so that decoding starts at or before source frame `index`, and returns the first
    /// picture decoded there (or `None` past the end). Some formats (MPEG-TS) seek to a byte
    /// position rather than a keyframe, and the decoder then skips ahead to the next keyframe,
    /// past the target; in that case this steps back further (1 s, 2 s, 4 s, ...) and tries again.
    fn seek_at_or_before(&mut self, index: i64) -> Result<Option<(ffmpeg::frame::Video, i64)>, MediaError> {
        let seconds = index as f64 / self.frame_rate.fps();
        let mut back = 0.0;
        loop {
            // Once stepping back reaches the start, read from the very beginning of the file:
            // even a seek to time 0 can land after the first keyframe in these formats.
            let at_start = seconds - back <= 0.0;
            if at_start {
                self.rewind()?;
            } else {
                self.seek_to(seconds - back)?;
            }
            if let Some(first) = self.decode_next()? {
                let first_index = self.index_of(&first).unwrap_or(index);
                if first_index <= index || at_start {
                    return Ok(Some((first, first_index)));
                }
            } else if at_start {
                return Ok(None);
            }
            back = if back == 0.0 { 1.0 } else { back * 2.0 };
        }
    }

    /// Goes back to the very beginning of the file by opening it again.
    fn rewind(&mut self) -> Result<(), MediaError> {
        self.input = ffmpeg::format::input(&self.path)?;
        self.decoder.flush();
        self.next_index = None;
        Ok(())
    }

    /// The picture for source frame `index` (clamped to the clip). Reading frames in order is cheap;
    /// a jump seeks to the nearest earlier keyframe and decodes forward from there.
    pub fn frame(&mut self, index: i64) -> Result<VideoFrame, MediaError> {
        self.frame_unless(index, &|| false).map(|frame| frame.expect("never cancelled"))
    }

    /// Like `frame`, but checks `cancelled` after each decoded picture and gives up (returning
    /// `Ok(None)`) once it says so, for example because the viewer has moved elsewhere.
    pub fn frame_unless(&mut self, index: i64, cancelled: &dyn Fn() -> bool) -> Result<Option<VideoFrame>, MediaError> {
        let index = index.clamp(0, self.frame_count - 1);
        // Past the end already found: the final picture, without decoding again.
        if let Some(last) = self.final_picture.as_ref().filter(|last| index >= last.index) {
            return Ok(Some(VideoFrame { index, ..last.clone() }));
        }
        let close_ahead = self.next_index.is_some_and(|next| index >= next && index - next <= FORWARD_DECODE_LIMIT);
        let mut pending = if close_ahead { None } else { self.seek_at_or_before(index)? };
        let mut last = None;
        loop {
            let (decoded, decoded_index) = match pending.take() {
                Some(first) => first,
                None => match self.decode_next()? {
                    Some(decoded) => {
                        let decoded_index = self.index_of(&decoded).unwrap_or(index);
                        (decoded, decoded_index)
                    }
                    None => break,
                },
            };
            self.next_index = Some(decoded_index + 1);
            if decoded_index >= index {
                return self.convert(decoded, decoded_index).map(Some);
            }
            if cancelled() {
                return Ok(None);
            }
            last = Some((decoded, decoded_index));
        }
        // The end of the file came first: the file has fewer frames than it claimed.
        let Some((decoded, decoded_index)) = last else {
            // Nothing left to decode from here (the decoder had already reached the end): look again from a seek.
            if close_ahead {
                self.next_index = None;
                return self.frame_unless(index, cancelled);
            }
            return Err(MediaError(format!("frame {index} could not be decoded")));
        };
        let picture = self.convert(decoded, decoded_index)?;
        self.final_picture = Some(picture.clone());
        Ok(Some(VideoFrame { index, ..picture }))
    }

    /// Converts a decoded picture to RGBA. Hardware pictures are downloaded first; this happens only
    /// for pictures that are shown, never for the in-between frames a seek decodes.
    fn convert(&mut self, decoded: ffmpeg::frame::Video, index: i64) -> Result<VideoFrame, MediaError> {
        let decoded = crate::hardware::download(decoded)?;
        let (width, height) = self.output_size;
        let source = (decoded.format(), decoded.width(), decoded.height());
        let stale = self.scaler.as_ref().is_none_or(|(_, format, w, h)| (*format, *w, *h) != source);
        if stale {
            let scaler = scaling::Context::get(source.0, source.1, source.2, Pixel::RGBA, width, height, scaling::Flags::BILINEAR)?;
            self.scaler = Some((scaler, source.0, source.1, source.2));
        }
        let (scaler, ..) = self.scaler.as_mut().expect("created above");
        let mut rgba = ffmpeg::frame::Video::empty();
        scaler.run(&decoded, &mut rgba)?;
        let stride = rgba.stride(0);
        let row = width as usize * 4;
        let mut pixels = Vec::with_capacity(row * height as usize);
        for y in 0..height as usize {
            pixels.extend_from_slice(&rgba.data(0)[y * stride..y * stride + row]);
        }
        // Pixel formats with an alpha component have four components (or two, for gray).
        let has_alpha = matches!(source.0.descriptor().map(|format| format.nb_components()), Some(2 | 4));
        Ok(VideoFrame { width, height, index, rgba: pixels, from_proxy: false, has_alpha })
    }
}
