//! The audio effects a clip can have, run on its sound block by block as the mix plays (and
//! as it exports). Each effect keeps its own memory (filter history, level followers, an echo
//! buffer, a reverb's rooms of delays) from one block to the next, and takes its settings fresh for every block, so
//! keyframed settings change smoothly while sound plays.
//!
//! The settings come as a list of numbers in the order Studio's effect catalog lists them
//! (see `studio_core::effects`, the "Audio" category); the names here must match it.

use crate::filters::Biquad;

/// The mixer works in interleaved stereo: left, right, left, right, ...
const CHANNELS: usize = 2;

/// The longest echo, in seconds (the echo buffer's size).
pub const ECHO_MAX_SECONDS: f32 = 2.0;

/// The name of every audio effect, as the catalog names it (kept in step with `Kind::new`).
const NAMES: &[&str] = &["EQ", "Compressor", "Limiter", "De-esser", "Hum Remover", "Gain", "Pan", "Echo", "Reverb"];

/// One audio effect on one clip, with the memory it carries between blocks.
pub struct AudioEffect {
    kind: Kind,
    /// The sample rate the memory was set up for; a change starts it afresh.
    rate: u32,
}

/// What an effect is, with its memory.
enum Kind {
    Eq(Eq),
    Compressor { envelope: f32 },
    Limiter { gain: f32 },
    DeEsser([DeEsser; CHANNELS]),
    HumRemover(Hum),
    Gain,
    Pan,
    Echo(Echo),
    Reverb(Box<Reverb>),
}

/// The EQ's filters for each channel, and the settings they were tuned for.
struct Eq {
    filters: [[Biquad; 5]; CHANNELS],
    tuned: Option<[f32; 6]>,
}

/// The de-esser's detector, crossover and level followers, for one channel.
struct DeEsser {
    detector: Biquad,
    low: [Biquad; 2],
    high: [Biquad; 2],
    sibilance: f32,
    whole: f32,
}

/// The hum remover's notches (one per harmonic) for each channel, and what they were tuned for.
struct Hum {
    notches: [Vec<Biquad>; CHANNELS],
    tuned: Option<(f32, usize)>,
}

/// A Freeverb-style reverb (Jezar Wakefield's public-domain design): for each channel, eight
/// comb filters in parallel, each a delay line whose output is fed back through a gentle treble
/// cut, stand in for a room's many reflections; four all-pass filters in series then smear them
/// into a dense tail. The right channel's delays are slightly longer, which makes the tail wide.
struct Reverb {
    channels: [ReverbChannel; CHANNELS],
}

struct ReverbChannel {
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
}

/// A feedback delay line with a one-pole treble cut in its loop (the "damping").
struct Comb {
    buffer: Vec<f32>,
    index: usize,
    /// The treble cut's memory.
    filtered: f32,
}

/// A delay line mixed with its own input so that every frequency passes at the same level, only
/// spread out in time.
struct Allpass {
    buffer: Vec<f32>,
    index: usize,
}

/// The comb and all-pass delay lengths, in samples at 44.1 kHz (Freeverb's tuning; scaled for
/// other rates), and how much longer the right channel's are.
const COMB_LENGTHS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_LENGTHS: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;

impl Comb {
    fn run(&mut self, input: f32, feedback: f32, damping: f32) -> f32 {
        let output = self.buffer[self.index];
        self.filtered = output * (1.0 - damping) + self.filtered * damping;
        self.buffer[self.index] = input + self.filtered * feedback;
        self.index = (self.index + 1) % self.buffer.len();
        output
    }
}

impl Allpass {
    fn run(&mut self, input: f32) -> f32 {
        let delayed = self.buffer[self.index];
        self.buffer[self.index] = input + delayed * 0.5;
        self.index = (self.index + 1) % self.buffer.len();
        delayed - input
    }
}

impl Reverb {
    fn new(rate: u32) -> Self {
        let scaled = |length: usize| ((length as f64 * rate as f64 / 44_100.0) as usize).max(1);
        let channel = |spread: usize| ReverbChannel {
            combs: COMB_LENGTHS.iter().map(|&length| Comb { buffer: vec![0.0; scaled(length + spread)], index: 0, filtered: 0.0 }).collect(),
            allpasses: ALLPASS_LENGTHS.iter().map(|&length| Allpass { buffer: vec![0.0; scaled(length + spread)], index: 0 }).collect(),
        };
        Self { channels: [channel(0), channel(STEREO_SPREAD)] }
    }

    /// Settings: room size (% — longer ringing), damping (% — the tail's treble fades faster),
    /// mix (% of the room heard in place of the original sound).
    fn process(&mut self, settings: &[f32], samples: &mut [f32]) {
        let room = (setting(settings, 0, 50.0) / 100.0).clamp(0.0, 1.0);
        let damping = (setting(settings, 1, 50.0) / 100.0).clamp(0.0, 1.0) * 0.4;
        let mix = (setting(settings, 2, 25.0) / 100.0).clamp(0.0, 1.0);
        // The room keeps listening even at 0% mix, so turning the mix up later doesn't start
        // from silence. Both channels hear the sum of the input, as in Freeverb; its gains
        // (0.015 in, 3 out) are kept, which puts the tail near the level of the original.
        let feedback = 0.7 + 0.28 * room;
        for frame in samples.chunks_mut(CHANNELS) {
            let input = frame.iter().sum::<f32>() * 0.015;
            for (sample, channel) in frame.iter_mut().zip(&mut self.channels) {
                let mut tail = channel.combs.iter_mut().map(|comb| comb.run(input, feedback, damping)).sum::<f32>();
                for allpass in &mut channel.allpasses {
                    tail = allpass.run(tail);
                }
                *sample = *sample * (1.0 - mix) + tail * 3.0 * mix;
            }
        }
    }
}

/// The echo's buffer of past sound (interleaved stereo) and where the next sample goes.
struct Echo {
    buffer: Vec<f32>,
    write: usize,
}

fn from_db(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn to_db(level: f32) -> f32 {
    20.0 * level.max(1e-9).log10()
}

/// How far a level follower moves towards a new level per sample, for a time constant of `ms`.
fn follow(ms: f32, rate: u32) -> f32 {
    (-1.0 / (ms.max(0.01) * 0.001 * rate as f32)).exp()
}

/// Setting `index`, or `default` when the list is too short or it isn't a number.
fn setting(settings: &[f32], index: usize, default: f32) -> f32 {
    settings.get(index).copied().filter(|value| value.is_finite()).unwrap_or(default)
}

impl AudioEffect {
    /// The effect called `name` (as the catalog names it), or `None` for a name that isn't an
    /// audio effect.
    pub fn new(name: &str, rate: u32) -> Option<Self> {
        Some(Self { kind: Kind::new(name, rate)?, rate })
    }

    /// Whether `name` is one of the audio effects. The mixer asks this for every block, so it
    /// checks the name instead of building the effect (and its echo or reverb buffers).
    pub fn exists(name: &str) -> bool {
        NAMES.contains(&name)
    }

    /// Runs the effect with `settings` on `samples` (interleaved stereo at `rate`), in place.
    pub fn process(&mut self, settings: &[f32], rate: u32, samples: &mut [f32]) {
        if rate != self.rate {
            if let Some(kind) = Kind::new(self.kind.name(), rate) {
                self.kind = kind;
            }
            self.rate = rate;
        }
        match &mut self.kind {
            Kind::Eq(eq) => eq.process(settings, rate, samples),
            Kind::Compressor { envelope } => compress(settings, rate, envelope, samples),
            Kind::Limiter { gain } => limit(settings, rate, gain, samples),
            Kind::DeEsser(channels) => de_ess(settings, rate, channels, samples),
            Kind::HumRemover(hum) => hum.process(settings, rate, samples),
            Kind::Gain => {
                let gain = from_db(setting(settings, 0, 0.0).clamp(-96.0, 48.0));
                samples.iter_mut().for_each(|sample| *sample *= gain);
            }
            Kind::Pan => pan(settings, samples),
            Kind::Echo(echo) => echo.process(settings, rate, samples),
            Kind::Reverb(reverb) => reverb.process(settings, samples),
        }
    }
}

impl Kind {
    fn new(name: &str, rate: u32) -> Option<Self> {
        let rate_f = rate as f32;
        Some(match name {
            "EQ" => Kind::Eq(Eq { filters: [[Biquad::pass(); 5]; CHANNELS], tuned: None }),
            "Compressor" => Kind::Compressor { envelope: 0.0 },
            "Limiter" => Kind::Limiter { gain: 1.0 },
            "De-esser" => {
                let butterworth = std::f32::consts::FRAC_1_SQRT_2;
                let one = || DeEsser {
                    detector: Biquad::band_pass(6_500.0, rate_f, 0.9),
                    low: [Biquad::low_pass(5_000.0, rate_f, butterworth); 2],
                    high: [Biquad::high_pass(5_000.0, rate_f, butterworth); 2],
                    sibilance: 0.0,
                    whole: 0.0,
                };
                Kind::DeEsser([one(), one()])
            }
            "Hum Remover" => Kind::HumRemover(Hum { notches: [Vec::new(), Vec::new()], tuned: None }),
            "Gain" => Kind::Gain,
            "Pan" => Kind::Pan,
            "Echo" => Kind::Echo(Echo { buffer: vec![0.0; (ECHO_MAX_SECONDS * rate_f) as usize * CHANNELS + CHANNELS], write: 0 }),
            "Reverb" => Kind::Reverb(Box::new(Reverb::new(rate))),
            _ => return None,
        })
    }

    fn name(&self) -> &'static str {
        match self {
            Kind::Eq(_) => "EQ",
            Kind::Compressor { .. } => "Compressor",
            Kind::Limiter { .. } => "Limiter",
            Kind::DeEsser(_) => "De-esser",
            Kind::HumRemover(_) => "Hum Remover",
            Kind::Gain => "Gain",
            Kind::Pan => "Pan",
            Kind::Echo(_) => "Echo",
            Kind::Reverb(_) => "Reverb",
        }
    }
}

impl Eq {
    /// Settings: low cut (Hz), low shelf gain (dB, at 120 Hz), mid frequency (Hz), mid gain
    /// (dB), high shelf gain (dB, at 8 kHz), high cut (Hz). A low cut at 20 Hz and a high cut at
    /// 20 kHz are off.
    fn process(&mut self, settings: &[f32], rate: u32, samples: &mut [f32]) {
        let wanted = [
            setting(settings, 0, 20.0),
            setting(settings, 1, 0.0),
            setting(settings, 2, 1000.0),
            setting(settings, 3, 0.0),
            setting(settings, 4, 0.0),
            setting(settings, 5, 20_000.0),
        ];
        if self.tuned != Some(wanted) {
            let rate = rate as f32;
            let [low_cut, low_gain, mid, mid_gain, high_gain, high_cut] = wanted;
            let butterworth = std::f32::consts::FRAC_1_SQRT_2;
            let shapes = [
                if low_cut > 20.5 { Biquad::high_pass(low_cut, rate, butterworth) } else { Biquad::pass() },
                Biquad::low_shelf(120.0, rate, butterworth, low_gain),
                Biquad::peak(mid.clamp(20.0, 20_000.0), rate, 1.0, mid_gain),
                Biquad::high_shelf(8_000.0, rate, butterworth, high_gain),
                if high_cut < 19_999.0 { Biquad::low_pass(high_cut, rate, butterworth) } else { Biquad::pass() },
            ];
            for channel in &mut self.filters {
                for (filter, shape) in channel.iter_mut().zip(shapes) {
                    filter.retune(shape);
                }
            }
            self.tuned = Some(wanted);
        }
        for frame in samples.chunks_mut(CHANNELS) {
            for (sample, filters) in frame.iter_mut().zip(&mut self.filters) {
                *sample = filters.iter_mut().fold(*sample, |value, filter| filter.run(value));
            }
        }
    }
}

/// Settings: threshold (dB), ratio, attack (ms), release (ms), makeup gain (dB). Both channels
/// are turned down together, so the stereo picture holds still.
fn compress(settings: &[f32], rate: u32, envelope: &mut f32, samples: &mut [f32]) {
    let threshold = setting(settings, 0, -18.0);
    let ratio = setting(settings, 1, 4.0).max(1.0);
    let (attack, release) = (follow(setting(settings, 2, 10.0), rate), follow(setting(settings, 3, 150.0), rate));
    let makeup = from_db(setting(settings, 4, 0.0));
    for frame in samples.chunks_mut(CHANNELS) {
        let peak = frame.iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        let k = if peak > *envelope { attack } else { release };
        *envelope = k * *envelope + (1.0 - k) * peak;
        let over = to_db(*envelope) - threshold;
        let gain = if over > 0.0 { from_db(-over * (1.0 - 1.0 / ratio)) } else { 1.0 } * makeup;
        frame.iter_mut().for_each(|sample| *sample *= gain);
    }
}

/// Settings: ceiling (dB), release (ms). The gain drops at once to keep each peak under the
/// ceiling and recovers over the release time.
fn limit(settings: &[f32], rate: u32, gain: &mut f32, samples: &mut [f32]) {
    let ceiling = from_db(setting(settings, 0, -1.0).min(0.0));
    let release = follow(setting(settings, 1, 80.0), rate);
    for frame in samples.chunks_mut(CHANNELS) {
        let peak = frame.iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        let needed = if peak > ceiling { ceiling / peak } else { 1.0 };
        *gain = if needed < *gain { needed } else { needed + (*gain - needed) * release };
        frame.iter_mut().for_each(|sample| *sample *= *gain);
    }
}

/// Settings: amount (%). Turns down the treble (above about 5 kHz) at moments when sibilance
/// (the 5–9 kHz band) is strong compared with the voice as a whole; more amount pulls it back
/// sooner and further. The crossover's halves add back to the original exactly, so with no
/// sibilance the sound is unchanged.
fn de_ess(settings: &[f32], rate: u32, channels: &mut [DeEsser; CHANNELS], samples: &mut [f32]) {
    let amount = (setting(settings, 0, 50.0) / 100.0).clamp(0.0, 1.0);
    if amount == 0.0 {
        return;
    }
    // At 100%, sibilance above 25% of the whole is pulled back by up to 18 dB.
    let share = 0.6 - 0.35 * amount;
    let floor = from_db(-18.0 * amount);
    let (attack, release) = (follow(1.0, rate), follow(60.0, rate));
    for frame in samples.chunks_mut(CHANNELS) {
        for (sample, state) in frame.iter_mut().zip(channels.iter_mut()) {
            let x = *sample;
            let band = state.detector.run(x).abs();
            let k = if band > state.sibilance { attack } else { release };
            state.sibilance = k * state.sibilance + (1.0 - k) * band;
            let k = if x.abs() > state.whole { attack } else { release };
            state.whole = k * state.whole + (1.0 - k) * x.abs();
            let limit = state.whole * share;
            let gain = if state.sibilance > limit && state.sibilance > 1e-4 { (limit / state.sibilance).powf(0.6).max(floor) } else { 1.0 };
            let below = state.low[0].run(x);
            let below = state.low[1].run(below);
            let above = state.high[0].run(x);
            let above = state.high[1].run(above);
            *sample = below + gain * above;
        }
    }
}

impl Hum {
    /// Settings: mains frequency (Hz: 50 in most of the world, 60 in the Americas), harmonics
    /// (how many multiples of it are removed too).
    fn process(&mut self, settings: &[f32], rate: u32, samples: &mut [f32]) {
        let mains = if setting(settings, 0, 60.0) < 55.0 { 50.0 } else { 60.0 };
        let harmonics = setting(settings, 1, 4.0).round().clamp(1.0, 8.0) as usize;
        if self.tuned != Some((mains, harmonics)) {
            let nyquist = rate as f32 / 2.0;
            let notches: Vec<Biquad> = (1..=harmonics)
                .map(|n| mains * n as f32)
                .filter(|&frequency| frequency < nyquist * 0.9)
                .map(|frequency| Biquad::notch(frequency, rate as f32, (frequency / 2.0).min(60.0)))
                .collect();
            self.notches = [notches.clone(), notches];
            self.tuned = Some((mains, harmonics));
        }
        for frame in samples.chunks_mut(CHANNELS) {
            for (sample, notches) in frame.iter_mut().zip(&mut self.notches) {
                *sample = notches.iter_mut().fold(*sample, |value, notch| notch.run(value));
            }
        }
    }
}

/// Settings: pan (-100 is all left, 100 all right). Constant power: the sound is as loud
/// panned as it is in the middle, where it is unchanged.
fn pan(settings: &[f32], samples: &mut [f32]) {
    let position = (setting(settings, 0, 0.0) / 100.0).clamp(-1.0, 1.0);
    let angle = (position + 1.0) * std::f32::consts::FRAC_PI_4;
    let (left, right) = (angle.cos() * std::f32::consts::SQRT_2, angle.sin() * std::f32::consts::SQRT_2);
    for frame in samples.chunks_mut(CHANNELS) {
        frame[0] *= left;
        if let Some(sample) = frame.get_mut(1) {
            *sample *= right;
        }
    }
}

impl Echo {
    /// Settings: time (ms between repeats), feedback (% of each repeat fed into the next), mix
    /// (% of the echo heard with the original).
    fn process(&mut self, settings: &[f32], rate: u32, samples: &mut [f32]) {
        let frames = self.buffer.len() / CHANNELS;
        let delay = ((setting(settings, 0, 250.0).max(1.0) * 0.001 * rate as f32) as usize).clamp(1, frames - 1);
        let feedback = (setting(settings, 1, 30.0) / 100.0).clamp(0.0, 0.95);
        let mix = (setting(settings, 2, 30.0) / 100.0).clamp(0.0, 1.0);
        for frame in samples.chunks_mut(CHANNELS) {
            let read = (self.write + frames - delay) % frames;
            for (channel, sample) in frame.iter_mut().enumerate() {
                let echoed = self.buffer[read * CHANNELS + channel];
                self.buffer[self.write * CHANNELS + channel] = *sample + echoed * feedback;
                *sample += echoed * mix;
            }
            self.write = (self.write + 1) % frames;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// One second of a stereo sine at `frequency` and amplitude `level`.
    fn sine(frequency: f32, level: f32) -> Vec<f32> {
        (0..RATE as usize).flat_map(|i| {
            let value = level * (2.0 * std::f32::consts::PI * frequency * i as f32 / RATE as f32).sin();
            [value, value]
        }).collect()
    }

    /// The RMS level of the second half of a stereo signal's channel `channel`, as a ratio to a
    /// full-scale sine (so a sine at amplitude `a` measures `a`).
    fn level(samples: &[f32], channel: usize) -> f32 {
        let tail: Vec<f32> = samples.chunks(CHANNELS).skip(samples.len() / CHANNELS / 2).map(|frame| frame[channel]).collect();
        (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32 * 2.0).sqrt()
    }

    /// Runs effect `name` with `settings` on `signal`, in blocks of 512 frames as the mixer does.
    fn run(name: &str, settings: &[f32], mut signal: Vec<f32>) -> Vec<f32> {
        let mut effect = AudioEffect::new(name, RATE).unwrap();
        for block in signal.chunks_mut(512 * CHANNELS) {
            effect.process(settings, RATE, block);
        }
        signal
    }

    #[test]
    fn only_known_names_are_audio_effects() {
        for name in ["EQ", "Compressor", "Limiter", "De-esser", "Hum Remover", "Gain", "Pan", "Echo", "Reverb"] {
            assert!(AudioEffect::exists(name), "{name}");
            let effect = AudioEffect::new(name, RATE).unwrap_or_else(|| panic!("{name} can be made"));
            assert_eq!(effect.kind.name(), name);
        }
        assert!(!AudioEffect::exists("Gaussian Blur"));
    }

    #[test]
    fn the_eq_cuts_and_boosts_its_bands() {
        let flat = [20.0, 0.0, 1000.0, 0.0, 0.0, 20_000.0];
        assert!((level(&run("EQ", &flat, sine(1000.0, 0.5)), 0) - 0.5).abs() < 0.01, "flat changes nothing");
        assert!(level(&run("EQ", &[200.0, 0.0, 1000.0, 0.0, 0.0, 20_000.0], sine(50.0, 0.5)), 0) < 0.05, "low cut");
        assert!(level(&run("EQ", &[20.0, 0.0, 1000.0, 0.0, 0.0, 2000.0], sine(10_000.0, 0.5)), 0) < 0.05, "high cut");
        let boosted = level(&run("EQ", &[20.0, 0.0, 1000.0, 6.0, 0.0, 20_000.0], sine(1000.0, 0.25)), 0);
        assert!((boosted - 0.5).abs() < 0.02, "+6 dB at the mid frequency doubles it: {boosted}");
    }

    #[test]
    fn the_compressor_evens_out_loud_sound_and_leaves_quiet_sound() {
        let settings = [-20.0, 4.0, 5.0, 100.0, 0.0];
        let loud = level(&run("Compressor", &settings, sine(200.0, 0.9)), 0);
        assert!(loud < 0.5, "a loud sine is turned down: {loud}");
        let quiet = level(&run("Compressor", &settings, sine(200.0, 0.02)), 0);
        assert!((quiet - 0.02).abs() < 0.002, "under the threshold nothing changes: {quiet}");
        let made_up = level(&run("Compressor", &[0.0, 4.0, 5.0, 100.0, 6.0], sine(200.0, 0.1)), 0);
        assert!((made_up - 0.2).abs() < 0.01, "makeup gain: {made_up}");
    }

    #[test]
    fn the_limiter_keeps_peaks_under_the_ceiling() {
        let out = run("Limiter", &[-6.0, 50.0], sine(200.0, 1.0));
        let peak = out.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
        assert!(peak <= from_db(-6.0) + 1e-4, "{peak}");
    }

    #[test]
    fn the_de_esser_tames_sibilance_but_not_the_voice() {
        let voice = level(&run("De-esser", &[100.0], sine(300.0, 0.3)), 0);
        assert!((voice - 0.3).abs() < 0.02, "a low voice is left alone: {voice}");
        let hiss = level(&run("De-esser", &[100.0], sine(7000.0, 0.3)), 0);
        assert!(hiss < 0.2, "a strong 7 kHz hiss is pulled back: {hiss}");
        let off = level(&run("De-esser", &[0.0], sine(7000.0, 0.3)), 0);
        assert!((off - 0.3).abs() < 0.01, "at 0% nothing changes: {off}");
    }

    #[test]
    fn the_hum_remover_takes_out_mains_hum_and_its_harmonics() {
        assert!(level(&run("Hum Remover", &[60.0, 4.0], sine(60.0, 0.5)), 0) < 0.05);
        assert!(level(&run("Hum Remover", &[60.0, 4.0], sine(180.0, 0.5)), 0) < 0.05, "the third harmonic");
        assert!(level(&run("Hum Remover", &[50.0, 4.0], sine(100.0, 0.5)), 0) < 0.05, "50 Hz mains");
        assert!((level(&run("Hum Remover", &[60.0, 4.0], sine(1000.0, 0.5)), 0) - 0.5).abs() < 0.02, "the rest stays");
    }

    #[test]
    fn gain_and_pan_set_the_level_and_place_the_sound() {
        assert!((level(&run("Gain", &[6.0], sine(440.0, 0.25)), 0) - 0.5).abs() < 0.01);
        let centered = run("Pan", &[0.0], sine(440.0, 0.5));
        assert!((level(&centered, 0) - 0.5).abs() < 0.01 && (level(&centered, 1) - 0.5).abs() < 0.01, "the middle is unchanged");
        let left = run("Pan", &[-100.0], sine(440.0, 0.5));
        assert!(level(&left, 1) < 0.001 && level(&left, 0) > 0.6, "all left");
    }

    #[test]
    fn the_echo_repeats_sound_after_its_time() {
        // A single click, then silence: the echo hears it again 100 ms later.
        let mut signal = vec![0.0; RATE as usize * CHANNELS];
        signal[0] = 1.0;
        signal[1] = 1.0;
        let out = run("Echo", &[100.0, 50.0, 50.0], signal);
        let at = |seconds: f32| out[(seconds * RATE as f32) as usize * CHANNELS];
        assert!((at(0.1) - 0.5).abs() < 1e-4, "first repeat at the mix: {}", at(0.1));
        assert!((at(0.2) - 0.25).abs() < 1e-4, "second repeat, fed back: {}", at(0.2));
        assert_eq!(at(0.05), 0.0);
    }

    /// A single stereo click at the start of `seconds` of silence.
    fn click(seconds: f32) -> Vec<f32> {
        let mut signal = vec![0.0; (seconds * RATE as f32) as usize * CHANNELS];
        signal[0] = 1.0;
        signal[1] = 1.0;
        signal
    }

    /// The RMS level of channel `channel` between `start` and `end` seconds.
    fn level_between(samples: &[f32], channel: usize, start: f32, end: f32) -> f32 {
        let frames = samples.chunks(CHANNELS).skip((start * RATE as f32) as usize).take(((end - start) * RATE as f32) as usize);
        let values: Vec<f32> = frames.map(|frame| frame[channel]).collect();
        (values.iter().map(|s| s * s).sum::<f32>() / values.len().max(1) as f32).sqrt()
    }

    #[test]
    fn the_reverb_leaves_a_decaying_tail_after_a_click() {
        let out = run("Reverb", &[50.0, 50.0, 100.0], click(2.0));
        let early = level_between(&out, 0, 0.05, 0.25);
        let late = level_between(&out, 0, 1.2, 1.4);
        assert!(early > 1e-4, "the room answers the click: {early}");
        assert!(late < early * 0.5, "and dies away: {early} then {late}");
        assert!(out.iter().all(|sample| sample.is_finite() && sample.abs() < 1.0), "the tail stays bounded");
        // The two channels' rooms differ slightly, which is what makes the tail wide.
        assert!(out.chunks(CHANNELS).any(|frame| (frame[0] - frame[1]).abs() > 1e-6));
    }

    #[test]
    fn a_bigger_room_rings_longer() {
        let small = level_between(&run("Reverb", &[10.0, 50.0, 100.0], click(3.0)), 0, 1.0, 1.5);
        let large = level_between(&run("Reverb", &[95.0, 50.0, 100.0], click(3.0)), 0, 1.0, 1.5);
        assert!(large > small * 4.0, "{small} vs {large}");
    }

    #[test]
    fn damping_darkens_the_reverb_tail() {
        // A burst of 8 kHz, then silence: how much of it is still ringing a little later.
        let mut signal = sine(8_000.0, 0.5);
        signal[(RATE as usize / 10) * CHANNELS..].iter_mut().for_each(|sample| *sample = 0.0);
        let bright = level_between(&run("Reverb", &[70.0, 0.0, 100.0], signal.clone()), 0, 0.3, 0.6);
        let dark = level_between(&run("Reverb", &[70.0, 100.0, 100.0], signal), 0, 0.3, 0.6);
        assert!(dark < bright * 0.5, "{bright} vs {dark}");
    }

    #[test]
    fn the_reverb_mix_blends_the_room_with_the_original() {
        let dry = run("Reverb", &[50.0, 50.0, 0.0], sine(440.0, 0.5));
        assert_eq!(dry, sine(440.0, 0.5), "at 0% the sound is unchanged");
        let half = level(&run("Reverb", &[50.0, 50.0, 50.0], sine(440.0, 0.5)), 0);
        assert!(half > 0.1 && half < 0.6, "half the original plus half the room: {half}");
        // The room keeps listening at 0%, so raising the mix mid-sound has a tail ready.
        let mut effect = AudioEffect::new("Reverb", RATE).unwrap();
        let mut first = click(0.5);
        effect.process(&[50.0, 50.0, 0.0], RATE, &mut first);
        let mut second = vec![0.0; RATE as usize / 4 * CHANNELS];
        effect.process(&[50.0, 50.0, 100.0], RATE, &mut second);
        assert!(level_between(&second, 0, 0.0, 0.2) > 1e-5);
    }
}
