//! Mixes a composition's audio clips into one stereo stream, block by block, following the
//! timeline from a start point and wrapping around a loop range like playback does. It does
//! no I/O of its own beyond the decoders, so it can be tested without a sound card.

use std::collections::HashMap;

use studio_core::audio::AudioClip;
use studio_core::model::Id;

use crate::audio::{AudioDecoder, CHANNELS};

/// Where the mixer plays: a range of timeline sample frames, played once or looped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixRange {
    /// The first sample frame and the one after the last.
    pub start: i64,
    pub end: i64,
    pub looping: bool,
}

/// A clip's decoder and where on the timeline its next sample plays.
struct Voice {
    decoder: Option<AudioDecoder>,
    /// `None` until the decoder is positioned; any jump (a loop, a moved layer) seeks again.
    next_frame: Option<i64>,
}

pub struct Mixer {
    sample_rate: u32,
    range: MixRange,
    /// The timeline sample frame the next mixed block starts at.
    position: i64,
    voices: HashMap<Id, Voice>,
    /// Scratch space for one clip's samples before they are added to the mix.
    scratch: Vec<f32>,
}

impl Mixer {
    /// A mixer at `sample_rate`, starting at timeline frame `start` inside `range`.
    pub fn new(sample_rate: u32, range: MixRange, start: i64) -> Self {
        let position = start.clamp(range.start, (range.end - 1).max(range.start));
        Self { sample_rate, range, position, voices: HashMap::new(), scratch: Vec::new() }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Converts timeline seconds to this mixer's sample frames.
    pub fn frame_at(&self, seconds: f64) -> i64 {
        (seconds * f64::from(self.sample_rate)).round() as i64
    }

    /// Mixes the next stretch of timeline into `out` (interleaved stereo) and returns how many
    /// frames were written. Stretches with no clip are silence. Fewer frames than `out` holds
    /// means playback reached the end of a range that does not loop.
    pub fn mix(&mut self, clips: &[AudioClip], out: &mut [f32]) -> usize {
        out.fill(0.0);
        // Forget clips that are gone, such as deleted or muted layers.
        self.voices.retain(|layer, _| clips.iter().any(|clip| clip.layer == *layer));
        let total = out.len() / CHANNELS;
        let mut written = 0;
        while written < total {
            if self.position >= self.range.end {
                if !self.range.looping {
                    break;
                }
                self.position = self.range.start;
            }
            // Mix up to the end of the range, where playback wraps or stops.
            let span = (total - written).min((self.range.end - self.position).max(0) as usize);
            if span == 0 {
                break;
            }
            let block = &mut out[written * CHANNELS..(written + span) * CHANNELS];
            for clip in clips {
                self.add_clip(clip, self.position, block);
            }
            self.position += span as i64;
            written += span;
        }
        for sample in &mut out[..written * CHANNELS] {
            *sample = sample.clamp(-1.0, 1.0);
        }
        written
    }

    /// Adds the part of `clip` heard during the block starting at timeline frame `from`.
    fn add_clip(&mut self, clip: &AudioClip, from: i64, block: &mut [f32]) {
        let to = from + (block.len() / CHANNELS) as i64;
        let (start, end) = (self.frame_at(clip.start_seconds).max(from), self.frame_at(clip.end_seconds).min(to));
        if start >= end {
            return;
        }
        let offset = self.frame_at(clip.offset_seconds);
        let sample_rate = self.sample_rate;
        let voice = self.voices.entry(clip.layer).or_insert_with(|| Voice {
            // A file that cannot be opened stays silent rather than stopping the mix.
            decoder: AudioDecoder::open(&clip.path, sample_rate).ok(),
            next_frame: None,
        });
        let Some(decoder) = voice.decoder.as_mut() else { return };
        if voice.next_frame != Some(start) && decoder.seek(start - offset).is_err() {
            voice.decoder = None;
            return;
        }
        let frames = (end - start) as usize;
        self.scratch.resize(frames * CHANNELS, 0.0);
        let read = decoder.read(&mut self.scratch).unwrap_or(0);
        // Past the end of the file `read` comes up short; the rest of the clip is silence.
        voice.next_frame = Some(end);
        let into = &mut block[(start - from) as usize * CHANNELS..];
        let fading = clip.fade_in > 0.0 || clip.fade_out > 0.0;
        for (index, (mixed, sample)) in into.iter_mut().zip(&self.scratch[..read * CHANNELS]).enumerate() {
            // Fades change the level sample by sample; without them it is one gain.
            let level = if fading { clip.level_at((start + (index / CHANNELS) as i64) as f64 / f64::from(sample_rate)) } else { clip.gain };
            *mixed += sample * level;
        }
    }
}
