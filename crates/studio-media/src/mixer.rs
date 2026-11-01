//! Mixes a composition's audio clips into one stereo stream, block by block, following the
//! timeline from a start point and wrapping around a loop range like playback does. It does
//! no I/O of its own beyond the decoders, so it can be tested without a sound card.
//!
//! Along the way it plays clips at their speed (by decoding them at a shifted rate, which
//! changes pitch like a tape), turns down background noise with each clip's noise gate, and
//! ducks clips (music, say) while the clip they duck under (a voice) is speaking.

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
    /// The rate the decoder delivers at: the mixer's, divided by the clip's speed.
    rate: u32,
    /// `None` until the decoder is positioned; any jump (a loop, a moved layer) seeks again.
    next_frame: Option<i64>,
    /// The noise gate's state: the level it follows, and the gain it applies.
    gate_level: f32,
    gate_gain: f32,
}

/// A voice (the clip ducked under) counts as speaking above this level (RMS, about -34 dB).
const SPEAKING: f32 = 0.02;
/// How quickly ducking turns down when the voice starts, and back up after it stops, in seconds.
const DUCK_ATTACK: f64 = 0.06;
const DUCK_RELEASE: f64 = 0.4;
/// The noise gate: how quickly it opens and closes, and how quickly its level follower falls.
const GATE_OPEN: f32 = 0.004;
const GATE_CLOSE: f32 = 0.12;
const GATE_FOLLOW: f32 = 0.08;

/// A per-sample smoothing factor for a time constant of `seconds` at `rate`.
fn smoothing(seconds: f32, rate: u32) -> f32 {
    (-1.0 / (seconds * rate as f32).max(1.0)).exp()
}

pub struct Mixer {
    sample_rate: u32,
    range: MixRange,
    /// The timeline sample frame the next mixed block starts at.
    position: i64,
    voices: HashMap<Id, Voice>,
    /// Scratch space for one clip's samples before they are added to the mix.
    scratch: Vec<f32>,
    /// The gain each ducked clip was at by the end of the last block.
    ducking: HashMap<Id, f32>,
}

impl Mixer {
    /// A mixer at `sample_rate`, starting at timeline frame `start` inside `range`.
    pub fn new(sample_rate: u32, range: MixRange, start: i64) -> Self {
        let position = start.clamp(range.start, (range.end - 1).max(range.start));
        Self { sample_rate, range, position, voices: HashMap::new(), scratch: Vec::new(), ducking: HashMap::new() }
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
            // Clips that duck go last, once the levels of the clips they duck under are known.
            let mut levels: HashMap<Id, f32> = HashMap::new();
            for clip in clips.iter().filter(|clip| clip.duck_under.is_none()) {
                let level = self.add_clip(clip, self.position, block, (1.0, 1.0));
                levels.insert(clip.layer, level);
            }
            let block_seconds = span as f64 / f64::from(self.sample_rate);
            for clip in clips.iter().filter(|clip| clip.duck_under.is_some()) {
                let speaking = clip.duck_under.and_then(|under| levels.get(&under)).is_some_and(|&level| level > SPEAKING);
                let target = if speaking { 10f32.powf(-clip.duck_db / 20.0) } else { 1.0 };
                let before = self.ducking.get(&clip.layer).copied().unwrap_or(1.0);
                let time = if target < before { DUCK_ATTACK } else { DUCK_RELEASE };
                let after = before + (target - before) * (1.0 - (-block_seconds / time).exp()) as f32;
                self.ducking.insert(clip.layer, after);
                let level = self.add_clip(clip, self.position, block, (before, after));
                levels.insert(clip.layer, level);
            }
            self.position += span as i64;
            written += span;
        }
        for sample in &mut out[..written * CHANNELS] {
            *sample = sample.clamp(-1.0, 1.0);
        }
        written
    }

    /// Adds the part of `clip` heard during the block starting at timeline frame `from`, with
    /// a ducking gain going from `duck.0` to `duck.1` across the block. Returns how loud the
    /// clip was in the block (RMS), for clips ducking under it.
    fn add_clip(&mut self, clip: &AudioClip, from: i64, block: &mut [f32], duck: (f32, f32)) -> f32 {
        let to = from + (block.len() / CHANNELS) as i64;
        let (start, end) = (self.frame_at(clip.start_seconds).max(from), self.frame_at(clip.end_seconds).min(to));
        if start >= end {
            return 0.0;
        }
        let offset = self.frame_at(clip.offset_seconds);
        let sample_rate = self.sample_rate;
        // At another speed, the file is decoded at a shifted rate: twice as fast is half the
        // rate, so each output sample takes up twice as much of the file.
        let rate = ((f64::from(sample_rate) / clip.speed.clamp(0.05, 20.0)).round() as u32).clamp(1_000, 768_000);
        if self.voices.get(&clip.layer).is_some_and(|voice| voice.rate != rate) {
            self.voices.remove(&clip.layer);
        }
        let voice = self.voices.entry(clip.layer).or_insert_with(|| Voice {
            // A file that cannot be opened stays silent rather than stopping the mix.
            decoder: AudioDecoder::open(&clip.path, rate).ok(),
            rate,
            next_frame: None,
            gate_level: 0.0,
            gate_gain: 1.0,
        });
        let Some(decoder) = voice.decoder.as_mut() else { return 0.0 };
        if voice.next_frame != Some(start) && decoder.seek(start - offset).is_err() {
            voice.decoder = None;
            return 0.0;
        }
        let frames = (end - start) as usize;
        self.scratch.resize(frames * CHANNELS, 0.0);
        let read = decoder.read(&mut self.scratch).unwrap_or(0);
        // Past the end of the file `read` comes up short; the rest of the clip is silence.
        voice.next_frame = Some(end);
        let samples = &mut self.scratch[..read * CHANNELS];
        if clip.noise_gate > 0.0 {
            gate(samples, clip.noise_gate, sample_rate, &mut voice.gate_level, &mut voice.gate_gain);
        }
        let into = &mut block[(start - from) as usize * CHANNELS..];
        let fading = clip.fade_in > 0.0 || clip.fade_out > 0.0;
        let span = (to - from).max(1) as f32;
        let mut energy = 0.0f32;
        for (index, (mixed, sample)) in into.iter_mut().zip(samples.iter()).enumerate() {
            let frame = start + (index / CHANNELS) as i64;
            // Fades change the level sample by sample; without them it is one gain.
            let level = if fading { clip.level_at(frame as f64 / f64::from(sample_rate)) } else { clip.gain };
            let ducked = duck.0 + (duck.1 - duck.0) * ((frame - from) as f32 / span);
            let value = sample * level * ducked;
            energy += value * value;
            *mixed += value;
        }
        (energy / samples.len().max(1) as f32).sqrt()
    }
}

/// A noise gate on interleaved stereo `samples`: where the sound falls below a threshold (set
/// by `amount`, 0–100: higher turns down louder background), it is turned down, more the
/// quieter it is, so hiss and room noise between words drop away while speech is untouched.
/// `level` and `gain` carry the gate's state from one block to the next.
fn gate(samples: &mut [f32], amount: f32, rate: u32, level: &mut f32, gain: &mut f32) {
    let threshold = 10f32.powf((-62.0 + amount.clamp(0.0, 100.0) * 0.36) / 20.0);
    let (follow, open, close) = (smoothing(GATE_FOLLOW, rate), smoothing(GATE_OPEN, rate), smoothing(GATE_CLOSE, rate));
    for frame in samples.chunks_mut(CHANNELS) {
        let peak = frame.iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        *level = if peak > *level { peak } else { *level * follow };
        let target = if *level >= threshold { 1.0 } else { (*level / threshold).powi(2) };
        let factor = if target > *gain { open } else { close };
        *gain = target + (*gain - target) * factor;
        for sample in frame {
            *sample *= *gain;
        }
    }
}

/// The noise gate on its own, for tests of what it does to sound.
#[doc(hidden)]
pub fn gate_for_tests(samples: &mut [f32], amount: f32, rate: u32) {
    let (mut level, mut gain) = (0.0, 1.0);
    gate(samples, amount, rate, &mut level, &mut gain);
}
