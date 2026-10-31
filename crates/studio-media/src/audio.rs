//! Decoding audio with FFmpeg into the one format the mixer works in: interleaved stereo
//! 32-bit floats at the sound card's sample rate, whatever the file's own format is.

use std::collections::VecDeque;
use std::path::Path;

use ffmpeg_next as ffmpeg;
use ffmpeg::format::Sample;
use ffmpeg::format::sample::Type as SampleLayout;
use ffmpeg::media::Type;
use ffmpeg::software::resampling;
use ffmpeg::{ChannelLayout, frame};

use crate::decode::{MediaError, init};

/// The mixer always works in stereo: left, right, left, right, ...
pub const CHANNELS: usize = 2;

/// Facts about a file's sound, read without decoding it.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioInfo {
    pub duration_seconds: f64,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Reads the length and format of a file's main audio stream.
pub fn probe_audio(path: &Path) -> Result<AudioInfo, MediaError> {
    init()?;
    let input = ffmpeg::format::input(path)?;
    let stream = input.streams().best(Type::Audio).ok_or_else(|| MediaError("the file has no audio".into()))?;
    let decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())?.decoder().audio()?;
    // Prefer the stream's own duration; fall back to the container's (microseconds).
    let duration_seconds = if stream.duration() > 0 {
        stream.duration() as f64 * f64::from(stream.time_base())
    } else {
        input.duration().max(0) as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE)
    };
    Ok(AudioInfo { duration_seconds, sample_rate: decoder.rate(), channels: decoder.channels() })
}

/// The resampler and the input format it was built for. Rebuilt if the decoded format changes.
struct Resampler {
    context: resampling::Context,
    input: (Sample, ChannelLayout, u32),
}

/// Decodes one file's main audio stream as stereo floats at `sample_rate`, reading forward
/// from any point. Positions count output sample frames (one left and one right sample)
/// from the start of the file.
pub struct AudioDecoder {
    input: ffmpeg::format::context::Input,
    stream_index: usize,
    decoder: ffmpeg::decoder::Audio,
    resampler: Option<Resampler>,
    sample_rate: u32,
    /// Seconds per unit of the stream's timestamps, and the stream's first timestamp in seconds.
    time_base: f64,
    start_seconds: f64,
    /// Decoded, resampled samples not read yet (interleaved stereo).
    pending: VecDeque<f32>,
    /// The output frame the first pending sample belongs to.
    position: i64,
    /// After a seek: the frame reading should start at. Decoded audio before it is dropped.
    seek_target: Option<i64>,
    ended: bool,
}

impl AudioDecoder {
    pub fn open(path: &Path, sample_rate: u32) -> Result<Self, MediaError> {
        init()?;
        let input = ffmpeg::format::input(path)?;
        let stream = input.streams().best(Type::Audio).ok_or_else(|| MediaError("the file has no audio".into()))?;
        let stream_index = stream.index();
        let decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())?.decoder().audio()?;
        let time_base = f64::from(stream.time_base());
        let start_seconds = if stream.start_time() == ffmpeg::ffi::AV_NOPTS_VALUE { 0.0 } else { stream.start_time() as f64 * time_base };
        Ok(Self {
            input,
            stream_index,
            decoder,
            resampler: None,
            sample_rate,
            time_base,
            start_seconds,
            pending: VecDeque::new(),
            position: 0,
            seek_target: None,
            ended: false,
        })
    }

    /// The output frame the next `read` starts at.
    pub fn position(&self) -> i64 {
        self.position
    }

    /// Moves to `frame` (output sample frames from the start of the file). Sample-accurate:
    /// FFmpeg lands on a packet at or before the spot, and the audio in between is dropped.
    pub fn seek(&mut self, frame: i64) -> Result<(), MediaError> {
        let frame = frame.max(0);
        let seconds = self.start_seconds + frame as f64 / f64::from(self.sample_rate);
        let target = (seconds * f64::from(ffmpeg::ffi::AV_TIME_BASE)) as i64;
        self.input.seek(target, ..target)?;
        self.decoder.flush();
        // A resampler holds a few samples of the old position; start afresh.
        self.resampler = None;
        self.pending.clear();
        self.position = frame;
        self.seek_target = Some(frame);
        self.ended = false;
        Ok(())
    }

    /// Fills `out` (interleaved stereo) from the current position and returns how many frames
    /// were written. Fewer than requested only at the end of the file.
    pub fn read(&mut self, out: &mut [f32]) -> Result<usize, MediaError> {
        let wanted = out.len() / CHANNELS;
        while self.pending.len() < wanted * CHANNELS && !self.ended {
            self.decode_more()?;
        }
        let frames = wanted.min(self.pending.len() / CHANNELS);
        for (slot, sample) in out.iter_mut().zip(self.pending.drain(..frames * CHANNELS)) {
            *slot = sample;
        }
        self.position += frames as i64;
        Ok(frames)
    }

    /// Decodes the next block of audio into `pending`, or marks the end of the file.
    fn decode_more(&mut self) -> Result<(), MediaError> {
        let Some(decoded) = self.decode_next()? else {
            self.ended = true;
            return Ok(());
        };
        let starts_at = self.frame_of(&decoded);
        let samples = self.resample(decoded)?;
        match self.seek_target {
            // The first audio after a seek: line it up with the frame that was asked for.
            Some(target) => {
                let Some(starts_at) = starts_at else {
                    // No timestamp to line up with; take the audio as it comes.
                    self.seek_target = None;
                    self.pending.extend(samples);
                    return Ok(());
                };
                let end = starts_at + (samples.len() / CHANNELS) as i64;
                if end <= target {
                    return Ok(()); // Entirely before the target: drop it and keep decoding.
                }
                self.seek_target = None;
                if starts_at > target {
                    // The file has a gap (or starts late): fill up to the audio with silence.
                    self.pending.extend(std::iter::repeat_n(0.0, (starts_at - target) as usize * CHANNELS));
                    self.pending.extend(samples);
                } else {
                    self.pending.extend(&samples[(target - starts_at) as usize * CHANNELS..]);
                }
            }
            None => self.pending.extend(samples),
        }
        Ok(())
    }

    /// The output frame a decoded block starts at, from its timestamp.
    fn frame_of(&self, decoded: &frame::Audio) -> Option<i64> {
        let pts = decoded.timestamp().or(decoded.pts())?;
        let seconds = pts as f64 * self.time_base - self.start_seconds;
        Some((seconds * f64::from(self.sample_rate)).round() as i64)
    }

    /// Decodes the next block of audio in stream order. `Ok(None)` at the end of the file.
    fn decode_next(&mut self) -> Result<Option<frame::Audio>, MediaError> {
        let mut decoded = frame::Audio::empty();
        loop {
            if self.decoder.receive_frame(&mut decoded).is_ok() {
                return Ok(Some(decoded));
            }
            // Feed the decoder another packet of our stream, or flush it at the end.
            let mut fed = false;
            for (stream, packet) in self.input.packets() {
                if stream.index() == self.stream_index {
                    self.decoder.send_packet(&packet)?;
                    fed = true;
                    break;
                }
            }
            if !fed {
                let _ = self.decoder.send_eof();
                return Ok(self.decoder.receive_frame(&mut decoded).is_ok().then_some(decoded));
            }
        }
    }

    /// Converts a decoded block to interleaved stereo floats at the output rate.
    fn resample(&mut self, mut decoded: frame::Audio) -> Result<Vec<f32>, MediaError> {
        // Some files (many WAVs) leave the channel layout unset; assume the standard one for the
        // channel count. It is set on the frame itself because the resampler checks it.
        let mut layout = decoded.channel_layout();
        if layout.is_empty() {
            layout = ChannelLayout::default(i32::from(decoded.channels()));
            decoded.set_channel_layout(layout);
        }
        let input = (decoded.format(), layout, decoded.rate());
        if self.resampler.as_ref().is_none_or(|resampler| resampler.input != input) {
            let output = Sample::F32(SampleLayout::Packed);
            let context = resampling::Context::get(input.0, input.1, input.2, output, ChannelLayout::STEREO, self.sample_rate)?;
            self.resampler = Some(Resampler { context, input });
        }
        let resampler = self.resampler.as_mut().expect("created above");

        // Room for every input sample at the new rate, plus what the resampler held back last time.
        let capacity = decoded.samples() * self.sample_rate as usize / decoded.rate().max(1) as usize + 256;
        let mut converted = frame::Audio::new(Sample::F32(SampleLayout::Packed), capacity, ChannelLayout::STEREO);
        resampler.context.run(&decoded, &mut converted)?;
        let bytes = &converted.data(0)[..converted.samples() * CHANNELS * size_of::<f32>()];
        Ok(bytes.as_chunks::<4>().0.iter().map(|b| f32::from_ne_bytes(*b)).collect())
    }
}
