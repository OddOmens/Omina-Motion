//! Writes movie files: rendered frames and mixed sound in, H.264 or H.265 (.mp4), ProRes
//! (.mov) or WebM (.webm) out. Frames must arrive in order; sound may arrive in any block size.
//!
//! - **H.264** uses x264 at constant quality (CRF 18, visually lossless for most material)
//!   with AAC sound at 320 kb/s, and moves the index to the front so files start playing
//!   before they are fully downloaded. Without x264, the system's hardware encoder is used.
//! - **H.265 (HEVC)** is the same idea at about half the file size, through x265 when FFmpeg
//!   has it and the system's hardware encoder otherwise. It is tagged `hvc1`, which Apple's
//!   players and the Photos app need to play it.
//! - **ProRes 422 HQ** is 10-bit and intra-only, for finishing and further editing, with
//!   uncompressed 24-bit sound.
//! - **WebM** is VP9 (libvpx) at constant quality with Opus sound, and keeps transparency in an
//!   alpha plane: small see-through clips for websites and web apps.
//!
//! Both are tagged BT.709, matching how frames are converted (see `yuv`).

use std::path::Path;

use ffmpeg_next as ffmpeg;
use ffmpeg::format::Pixel;
use ffmpeg::format::sample::Type as SampleLayout;
use ffmpeg::util::color;
use ffmpeg::{ChannelLayout, Rational, codec, encoder, frame};
use studio_core::time::FrameRate;

use crate::audio::CHANNELS;
use crate::decode::{MediaError, init};
use crate::yuv::{Plane, YuvLayout, bgra_to_yuv};

/// The kinds of movie Studio exports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovieFormat {
    H264,
    /// H.265, also called HEVC: smaller files than H.264 at the same quality.
    H265,
    /// Apple ProRes 422 Proxy, LT, standard and HQ: the same 10-bit 4:2:2 picture at rising data rates.
    ProResProxy,
    ProResLt,
    ProRes422,
    ProRes422Hq,
    /// Apple ProRes 4444: full color detail and an alpha channel, so transparent areas stay
    /// transparent (renders of single layers, graphics for other apps).
    ProRes4444,
    /// VP9 video and Opus sound in a WebM file, keeping transparency: for the web.
    WebM,
}

impl MovieFormat {
    pub const ALL: [Self; 8] = [Self::H264, Self::H265, Self::WebM, Self::ProRes4444, Self::ProRes422Hq, Self::ProRes422, Self::ProResLt, Self::ProResProxy];

    pub const fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264 (MP4)",
            Self::H265 => "H.265 / HEVC (MP4)",
            Self::ProResProxy => "ProRes 422 Proxy (MOV)",
            Self::ProResLt => "ProRes 422 LT (MOV)",
            Self::ProRes422 => "ProRes 422 (MOV)",
            Self::ProRes422Hq => "ProRes 422 HQ (MOV)",
            Self::ProRes4444 => "ProRes 4444 with transparency (MOV)",
            Self::WebM => "WebM · VP9 with transparency (WEBM)",
        }
    }

    /// The file name extension, without the dot.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::H264 | Self::H265 => "mp4",
            Self::WebM => "webm",
            _ => "mov",
        }
    }

    /// Whether it is a ProRes flavor (which has no quality or speed setting: each flavor is a
    /// fixed data rate, and its sound is uncompressed).
    pub const fn is_prores(self) -> bool {
        !matches!(self, Self::H264 | Self::H265 | Self::WebM)
    }

    /// `prores_ks`'s profile number for a ProRes flavor.
    const fn prores_profile(self) -> &'static str {
        match self {
            Self::ProResProxy => "0",
            Self::ProResLt => "1",
            Self::ProRes422 => "2",
            Self::ProRes4444 => "4",
            _ => "3",
        }
    }

    fn container(self) -> &'static str {
        self.extension()
    }

    /// Whether this build's FFmpeg has the encoders the format needs (WebM needs libvpx and
    /// libopus, which an FFmpeg built without them lacks), so it can be offered.
    pub fn can_encode(self) -> bool {
        if init().is_err() {
            return false;
        }
        let has = |name: &str| encoder::find_by_name(name).is_some();
        match self {
            Self::H264 => has("libx264") || has("h264_videotoolbox"),
            Self::H265 => has("libx265") || has("hevc_videotoolbox"),
            Self::WebM => has("libvpx-vp9") && has("libopus"),
            _ => has("prores_ks"),
        }
    }

    /// Whether the movie keeps the picture's transparency.
    pub const fn has_alpha(self) -> bool {
        matches!(self, Self::ProRes4444 | Self::WebM)
    }

    fn layout(self) -> YuvLayout {
        match self {
            Self::H264 | Self::H265 | Self::WebM => YuvLayout::Yuv420p8,
            Self::ProRes4444 => YuvLayout::Yuv444p10,
            _ => YuvLayout::Yuv422p10,
        }
    }

    fn pixel(self) -> Pixel {
        match self {
            Self::H264 | Self::H265 => Pixel::YUV420P,
            Self::WebM => Pixel::YUVA420P,
            Self::ProRes4444 => Pixel::YUVA444P10LE,
            _ => Pixel::YUV422P10LE,
        }
    }
}

/// How much picture detail H.264 keeps, against file size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoQuality {
    /// Small files for quick reviews.
    Draft,
    /// Good for streaming and social posts.
    Good,
    /// Close to the source; the default.
    High,
    /// Visually lossless, for masters and further editing.
    Maximum,
    /// A fixed data rate, in kilobits per second (for platforms with a bitrate spec).
    Bitrate(u32),
}

impl VideoQuality {
    pub const PRESETS: [Self; 4] = [Self::Draft, Self::Good, Self::High, Self::Maximum];

    pub fn label(self) -> String {
        match self {
            Self::Draft => "Draft".into(),
            Self::Good => "Good".into(),
            Self::High => "High".into(),
            Self::Maximum => "Maximum".into(),
            Self::Bitrate(kbps) => format!("{} Mbps", f64::from(kbps) / 1000.0),
        }
    }

    /// x264's constant-quality number (lower is better).
    fn crf(self) -> Option<&'static str> {
        match self {
            Self::Draft => Some("28"),
            Self::Good => Some("23"),
            Self::High => Some("18"),
            Self::Maximum => Some("14"),
            Self::Bitrate(_) => None,
        }
    }

    /// x265's constant-quality number. Its scale runs about six steps behind x264's for the
    /// same look, which is where H.265's smaller files come from.
    fn x265_crf(self) -> Option<&'static str> {
        match self {
            Self::Draft => Some("32"),
            Self::Good => Some("28"),
            Self::High => Some("23"),
            Self::Maximum => Some("19"),
            Self::Bitrate(_) => None,
        }
    }

    /// VP9's constant-quality number (0–63, lower is better). VP9 needs higher numbers than x264
    /// for the same look.
    fn vp9_crf(self) -> Option<&'static str> {
        match self {
            Self::Draft => Some("42"),
            Self::Good => Some("36"),
            Self::High => Some("31"),
            Self::Maximum => Some("24"),
            Self::Bitrate(_) => None,
        }
    }

    /// Bits per pixel for encoders without a constant-quality mode (the hardware one).
    fn bits_per_pixel(self) -> f64 {
        match self {
            Self::Draft => 0.05,
            Self::Good => 0.1,
            Self::High => 0.15,
            Self::Maximum => 0.3,
            Self::Bitrate(_) => 0.15,
        }
    }
}

/// How long H.264 spends finding a smaller file at the same quality.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodeSpeed {
    Fast,
    Balanced,
    /// Slower, for a smaller file.
    Small,
}

impl EncodeSpeed {
    pub const ALL: [Self; 3] = [Self::Fast, Self::Balanced, Self::Small];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Fast => "Faster export",
            Self::Balanced => "Balanced",
            Self::Small => "Smaller file",
        }
    }

    const fn preset(self) -> &'static str {
        match self {
            Self::Fast => "veryfast",
            Self::Balanced => "medium",
            Self::Small => "slow",
        }
    }

    /// libvpx's `cpu-used`: how much quality it trades for speed (higher is faster).
    const fn vp9_cpu_used(self) -> &'static str {
        match self {
            Self::Fast => "5",
            Self::Balanced => "3",
            Self::Small => "1",
        }
    }
}

/// What a movie holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovieSettings {
    pub format: MovieFormat,
    /// The picture size. Odd sizes are rounded up to even ones (both codecs need even sizes),
    /// repeating the last column or row.
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    /// The sound's sample rate, or `None` for a movie without sound.
    pub audio_rate: Option<u32>,
    /// H.264, H.265 and WebM only.
    pub quality: VideoQuality,
    /// H.264, H.265 and WebM only.
    pub speed: EncodeSpeed,
    /// Compressed sound (AAC in H.264 and H.265 movies, Opus in WebM), in kilobits per second.
    /// ProRes sound is uncompressed.
    pub audio_kbps: u32,
}

impl MovieSettings {
    /// A movie at high quality, balanced speed and 320 kbps sound; change what you need.
    pub fn new(format: MovieFormat, width: u32, height: u32, frame_rate: FrameRate, audio_rate: Option<u32>) -> Self {
        Self { format, width, height, frame_rate, audio_rate, quality: VideoQuality::High, speed: EncodeSpeed::Balanced, audio_kbps: 320 }
    }
}

/// One encoded stream: its encoder, where it goes in the file, and its time units.
struct Track<E> {
    encoder: E,
    stream: usize,
    encoder_time_base: Rational,
    stream_time_base: Rational,
    /// Timestamp of the next frame, in the encoder's time units.
    next_pts: i64,
}

/// How sound samples are stored for each format's audio codec.
#[derive(Clone, Copy)]
enum SampleStore {
    /// 32-bit floats, one plane per channel (AAC).
    PlanarFloat,
    /// 32-bit floats, interleaved (Opus).
    PackedFloat,
    /// 24-bit samples in 32-bit integers, interleaved (PCM).
    Packed24,
}

struct AudioTrack {
    track: Track<encoder::audio::Encoder>,
    store: SampleStore,
    /// Samples per encoded frame.
    frame_size: usize,
    /// Interleaved stereo samples not yet encoded (less than one frame's worth).
    pending: Vec<f32>,
}

pub struct MovieWriter {
    output: ffmpeg::format::context::Output,
    format: MovieFormat,
    size: (u32, u32),
    video: Track<encoder::video::Encoder>,
    audio: Option<AudioTrack>,
}

impl MovieWriter {
    /// Creates the file at `path` (whatever its name, the container follows `settings.format`).
    pub fn create(path: &Path, settings: MovieSettings) -> Result<Self, MediaError> {
        init()?;
        if settings.width == 0 || settings.height == 0 {
            return Err(MediaError("the picture has no size".into()));
        }
        let size = ((settings.width + 1) & !1, (settings.height + 1) & !1);
        let mut output = ffmpeg::format::output_as(path, settings.format.container())?;
        let global_header = output.format().flags().contains(ffmpeg::format::Flags::GLOBAL_HEADER);

        let video = open_video(&mut output, settings, size, global_header)?;
        let audio = match settings.audio_rate {
            Some(rate) => Some(open_audio(&mut output, settings.format, rate, settings.audio_kbps, global_header)?),
            None => None,
        };

        let mut options = ffmpeg::Dictionary::new();
        if settings.format.extension() == "mp4" {
            // Put the index first, so the file can play while it downloads.
            options.set("movflags", "+faststart");
        }
        output.write_header_with(options)?;
        // The muxer may change stream time bases when it writes the header.
        let mut writer = Self { output, format: settings.format, size, video, audio };
        writer.video.stream_time_base = writer.stream_time_base(writer.video.stream);
        if let Some(audio) = writer.audio.as_mut().map(|audio| audio.track.stream) {
            let time_base = writer.stream_time_base(audio);
            writer.audio.as_mut().expect("checked above").track.stream_time_base = time_base;
        }
        Ok(writer)
    }

    fn stream_time_base(&self, index: usize) -> Rational {
        self.output.stream(index).map_or(Rational::new(1, 90_000), |stream| stream.time_base())
    }

    /// Encodes the next frame from a `width × height` BGRA image.
    pub fn write_video(&mut self, bgra: &[u8], width: u32, height: u32) -> Result<(), MediaError> {
        if bgra.len() < width as usize * height as usize * 4 || width == 0 || height == 0 {
            return Err(MediaError("the frame is smaller than its size says".into()));
        }
        // A fresh frame each time: encoders working on several frames at once may still be
        // reading the previous one.
        let (out_width, out_height) = self.size;
        let mut picture = frame::Video::new(self.format.pixel(), out_width, out_height);
        // Convert into tightly packed planes, then copy each into the frame's own (padded) rows:
        // the frame only lends out one plane at a time.
        let layout = self.format.layout();
        let (luma, chroma) = layout.plane_sizes(out_width as usize, out_height as usize);
        let (mut y, mut u, mut v) = (vec![0; luma.0 * luma.1], vec![0; chroma.0 * chroma.1], vec![0; chroma.0 * chroma.1]);
        let planes = [Plane { data: &mut y, stride: luma.0 }, Plane { data: &mut u, stride: chroma.0 }, Plane { data: &mut v, stride: chroma.0 }];
        let source = (width as usize, height as usize);
        let out = (out_width as usize, out_height as usize);
        bgra_to_yuv(bgra, source, out, layout, planes);
        // With transparency, alpha goes in a fourth plane. Colors are stored straight (not
        // multiplied by their alpha), as the renderer hands them over.
        let alpha = self.format.has_alpha().then(|| crate::yuv::alpha_plane(bgra, source, out, layout));
        let mut packed_planes = vec![(&y, luma), (&u, chroma), (&v, chroma)];
        if let Some(alpha) = &alpha {
            packed_planes.push((alpha, luma));
        }
        for (index, (packed, (row_bytes, rows))) in packed_planes.into_iter().enumerate() {
            let stride = picture.stride(index);
            let data = picture.data_mut(index);
            for row in 0..rows {
                data[row * stride..row * stride + row_bytes].copy_from_slice(&packed[row * row_bytes..(row + 1) * row_bytes]);
            }
        }
        tag_bt709(&mut picture);
        picture.set_pts(Some(self.video.next_pts));
        self.video.next_pts += 1;
        self.video.encoder.send_frame(&picture)?;
        drain_video(&mut self.video, &mut self.output)
    }

    /// Adds sound: interleaved stereo samples at the movie's sample rate. Ignored for movies
    /// without sound.
    pub fn write_audio(&mut self, samples: &[f32]) -> Result<(), MediaError> {
        let Some(audio) = self.audio.as_mut() else { return Ok(()) };
        audio.pending.extend_from_slice(samples);
        let block = audio.frame_size * CHANNELS;
        while audio.pending.len() >= block {
            let rest = audio.pending.split_off(block);
            let full = std::mem::replace(&mut audio.pending, rest);
            encode_audio(audio, &full, &mut self.output)?;
        }
        Ok(())
    }

    /// Encodes what is still buffered and completes the file.
    pub fn finish(mut self) -> Result<(), MediaError> {
        if let Some(audio) = self.audio.as_mut() {
            let rest = std::mem::take(&mut audio.pending);
            if !rest.is_empty() {
                encode_audio(audio, &rest, &mut self.output)?;
            }
            audio.track.encoder.send_eof()?;
            drain_audio(&mut audio.track, &mut self.output)?;
        }
        self.video.encoder.send_eof()?;
        drain_video(&mut self.video, &mut self.output)?;
        self.output.write_trailer()?;
        Ok(())
    }
}

fn tag_bt709(picture: &mut frame::Video) {
    picture.set_color_space(color::Space::BT709);
    picture.set_color_range(color::Range::MPEG);
    picture.set_color_primaries(color::Primaries::BT709);
    picture.set_color_transfer_characteristic(color::TransferCharacteristic::BT709);
}

fn open_video(
    output: &mut ffmpeg::format::context::Output,
    settings: MovieSettings,
    (width, height): (u32, u32),
    global_header: bool,
) -> Result<Track<encoder::video::Encoder>, MediaError> {
    let (codec, mut options) = match settings.format {
        MovieFormat::H264 => {
            let mut options = ffmpeg::Dictionary::new();
            match encoder::find_by_name("libx264") {
                Some(codec) => {
                    // A fixed bitrate is set on the encoder below; otherwise constant quality.
                    if let Some(crf) = settings.quality.crf() {
                        options.set("crf", crf);
                    }
                    options.set("preset", settings.speed.preset());
                    options.set("profile", "high");
                    (codec, options)
                }
                None => {
                    let codec = encoder::find_by_name("h264_videotoolbox")
                        .ok_or_else(|| MediaError("this FFmpeg has no H.264 encoder (install it with x264)".into()))?;
                    options.set("profile", "high");
                    (codec, options)
                }
            }
        }
        MovieFormat::H265 => {
            let mut options = ffmpeg::Dictionary::new();
            match encoder::find_by_name("libx265") {
                Some(codec) => {
                    if let Some(crf) = settings.quality.x265_crf() {
                        options.set("crf", crf);
                    }
                    options.set("preset", settings.speed.preset());
                    // x265 otherwise prints a page of settings for every export.
                    options.set("x265-params", "log-level=error");
                    (codec, options)
                }
                None => {
                    let codec = encoder::find_by_name("hevc_videotoolbox")
                        .ok_or_else(|| MediaError("this FFmpeg has no H.265 encoder".into()))?;
                    options.set("profile", "main");
                    (codec, options)
                }
            }
        }
        MovieFormat::WebM => {
            let codec = encoder::find_by_name("libvpx-vp9").ok_or_else(|| MediaError("this FFmpeg has no VP9 encoder (it needs libvpx)".into()))?;
            let mut options = ffmpeg::Dictionary::new();
            // Constant quality needs the target bitrate at zero; a fixed bitrate is set below.
            if let Some(crf) = settings.quality.vp9_crf() {
                options.set("crf", crf);
                options.set("b", "0");
            }
            options.set("deadline", "good");
            options.set("cpu-used", settings.speed.vp9_cpu_used());
            // Rows of each frame encode in parallel.
            options.set("row-mt", "1");
            // libvpx can't look ahead with an alternate reference frame while it keeps alpha.
            options.set("auto-alt-ref", "0");
            (codec, options)
        }
        _ => {
            let codec = encoder::find_by_name("prores_ks").ok_or_else(|| MediaError("this FFmpeg has no ProRes encoder".into()))?;
            let mut options = ffmpeg::Dictionary::new();
            options.set("profile", settings.format.prores_profile());
            options.set("vendor", "apl0"); // Marks the file as Apple-compatible ProRes.
            (codec, options)
        }
    };
    let mut stream = output.add_stream(codec)?;
    let index = stream.index();
    let mut context = codec::context::Context::new_with_codec(codec);
    context.set_threading(codec::threading::Config::kind(codec::threading::Type::Frame));
    let mut video = context.encoder().video()?;
    let rate = settings.frame_rate;
    let fps = Rational::new(rate.numerator as i32, rate.denominator as i32);
    let time_base = Rational::new(rate.denominator as i32, rate.numerator as i32);
    video.set_width(width);
    video.set_height(height);
    video.set_format(settings.format.pixel());
    video.set_time_base(time_base);
    video.set_frame_rate(Some(fps));
    video.set_colorspace(color::Space::BT709);
    video.set_color_range(color::Range::MPEG);
    video.set_color_primaries(color::Primaries::BT709);
    video.set_color_transfer_characteristic(color::TransferCharacteristic::BT709);
    if let VideoQuality::Bitrate(kbps) = settings.quality
        && !settings.format.is_prores()
    {
        video.set_bit_rate(kbps as usize * 1000);
    } else if codec.name() == "h264_videotoolbox" || codec.name() == "hevc_videotoolbox" {
        // The hardware encoder has no constant-quality mode here, so the quality picks a bitrate.
        let bits = f64::from(width) * f64::from(height) * rate.fps() * settings.quality.bits_per_pixel();
        video.set_bit_rate(bits as usize);
    }
    if global_header {
        video.set_flags(codec::Flags::GLOBAL_HEADER);
    }
    let encoder = video.open_with(std::mem::take(&mut options))?;
    stream.set_parameters(&encoder);
    if settings.format == MovieFormat::H265 {
        tag_as_hvc1(&mut stream);
    }
    stream.set_avg_frame_rate(fps);
    stream.set_time_base(time_base);
    Ok(Track { encoder, stream: index, encoder_time_base: time_base, stream_time_base: time_base, next_pts: 0 })
}

/// Marks an H.265 stream `hvc1` rather than FFmpeg's default `hev1`: both hold the same video,
/// but QuickTime, Safari and Photos only play `hvc1`. `ffmpeg-next` has no safe way to set a
/// stream's tag, so this one field is written directly.
#[allow(unsafe_code)]
fn tag_as_hvc1(stream: &mut ffmpeg::format::stream::StreamMut) {
    // SAFETY: the stream was just added to the open output, so FFmpeg has allocated both it and
    // its codec parameters; a plain integer field is written before the header is.
    unsafe {
        let parameters = (*stream.as_mut_ptr()).codecpar;
        (*parameters).codec_tag = u32::from_le_bytes(*b"hvc1");
    }
}

fn open_audio(output: &mut ffmpeg::format::context::Output, format: MovieFormat, rate: u32, kbps: u32, global_header: bool) -> Result<AudioTrack, MediaError> {
    let (name, sample, store) = if format == MovieFormat::WebM {
        ("libopus", ffmpeg::format::Sample::F32(SampleLayout::Packed), SampleStore::PackedFloat)
    } else if format.is_prores() {
        ("pcm_s24le", ffmpeg::format::Sample::I32(SampleLayout::Packed), SampleStore::Packed24)
    } else {
        ("aac", ffmpeg::format::Sample::F32(SampleLayout::Planar), SampleStore::PlanarFloat)
    };
    let codec = encoder::find_by_name(name).ok_or_else(|| MediaError(format!("this FFmpeg has no {name} encoder")))?;
    let mut stream = output.add_stream(codec)?;
    let index = stream.index();
    let mut audio = codec::context::Context::new_with_codec(codec).encoder().audio()?;
    let time_base = Rational::new(1, rate as i32);
    audio.set_rate(rate as i32);
    audio.set_channel_layout(ChannelLayout::STEREO);
    audio.set_channels(CHANNELS as i32);
    audio.set_format(sample);
    audio.set_time_base(time_base);
    match store {
        SampleStore::PlanarFloat => audio.set_bit_rate(kbps.clamp(64, 512) as usize * 1000),
        // Opus tops out at 256 kb/s for stereo, and sounds transparent well below that.
        SampleStore::PackedFloat => audio.set_bit_rate(kbps.clamp(64, 256) as usize * 1000),
        SampleStore::Packed24 => {}
    }
    if global_header {
        audio.set_flags(codec::Flags::GLOBAL_HEADER);
    }
    let encoder = audio.open_as(codec)?;
    stream.set_parameters(&encoder);
    stream.set_time_base(time_base);
    // PCM takes any block size; AAC and Opus need exactly their frame size.
    let frame_size = match encoder.frame_size() {
        0 => 1024,
        size => size as usize,
    };
    Ok(AudioTrack {
        track: Track { encoder, stream: index, encoder_time_base: time_base, stream_time_base: time_base, next_pts: 0 },
        store,
        frame_size,
        pending: Vec::new(),
    })
}

/// Encodes one block of interleaved stereo samples (at most one frame's worth).
fn encode_audio(audio: &mut AudioTrack, samples: &[f32], output: &mut ffmpeg::format::context::Output) -> Result<(), MediaError> {
    let frames = samples.len() / CHANNELS;
    let (format, rate) = (audio.track.encoder.format(), audio.track.encoder.rate());
    let mut block = frame::Audio::new(format, frames, ChannelLayout::STEREO);
    block.set_rate(rate);
    match audio.store {
        SampleStore::PlanarFloat => {
            for channel in 0..CHANNELS {
                let plane = block.data_mut(channel);
                for (index, sample) in samples.iter().skip(channel).step_by(CHANNELS).enumerate() {
                    plane[index * 4..index * 4 + 4].copy_from_slice(&sample.clamp(-1.0, 1.0).to_ne_bytes());
                }
            }
        }
        SampleStore::PackedFloat => {
            let plane = block.data_mut(0);
            for (index, sample) in samples.iter().enumerate() {
                plane[index * 4..index * 4 + 4].copy_from_slice(&sample.clamp(-1.0, 1.0).to_ne_bytes());
            }
        }
        SampleStore::Packed24 => {
            let plane = block.data_mut(0);
            for (index, sample) in samples.iter().enumerate() {
                // 24-bit precision, stored in the top bits of a 32-bit sample.
                let value = ((sample.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32) << 8;
                plane[index * 4..index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
            }
        }
    }
    block.set_pts(Some(audio.track.next_pts));
    audio.track.next_pts += frames as i64;
    audio.track.encoder.send_frame(&block)?;
    drain_audio(&mut audio.track, output)
}

/// Moves finished packets from an encoder into the file.
fn drain(
    encoder: &mut encoder::encoder::Encoder,
    (stream, from, to): (usize, Rational, Rational),
    output: &mut ffmpeg::format::context::Output,
) -> Result<(), MediaError> {
    let mut packet = ffmpeg::Packet::empty();
    while encoder.receive_packet(&mut packet).is_ok() {
        packet.set_stream(stream);
        packet.rescale_ts(from, to);
        packet.write_interleaved(output)?;
    }
    Ok(())
}

fn drain_video(track: &mut Track<encoder::video::Encoder>, output: &mut ffmpeg::format::context::Output) -> Result<(), MediaError> {
    drain(&mut track.encoder, (track.stream, track.encoder_time_base, track.stream_time_base), output)
}

fn drain_audio(track: &mut Track<encoder::audio::Encoder>, output: &mut ffmpeg::format::context::Output) -> Result<(), MediaError> {
    drain(&mut track.encoder, (track.stream, track.encoder_time_base, track.stream_time_base), output)
}
