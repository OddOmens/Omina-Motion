//! Level control: a compressor that evens out loud and quiet words, a de-esser that tames
//! harsh "s" sounds, and a limiter that keeps peaks under the ceiling. All work on whole
//! recordings at 48 kHz, so the limiter can look ahead freely.

use studio_dsp::Biquad;

const RATE: f32 = 48_000.0;

/// How quickly a level follower moves towards a new level, per sample, for a time constant of
/// `ms` milliseconds.
fn coefficient(ms: f32) -> f32 {
    (-1.0 / (ms * 0.001 * RATE)).exp()
}

fn to_db(level: f32) -> f32 {
    20.0 * level.max(1e-9).log10()
}

fn from_db(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Compresses all channels together (so the stereo picture doesn't wander): above
/// `threshold_db`, every `ratio` dB of extra level comes out as 1 dB.
pub fn compress(channels: &mut [Vec<f32>], threshold_db: f32, ratio: f32) {
    let (attack, release) = (coefficient(8.0), coefficient(180.0));
    let length = channels.iter().map(Vec::len).min().unwrap_or(0);
    let mut envelope = 0.0f32;
    for i in 0..length {
        let peak = channels.iter().map(|channel| channel[i].abs()).fold(0.0, f32::max);
        let k = if peak > envelope { attack } else { release };
        envelope = k * envelope + (1.0 - k) * peak;
        let over = to_db(envelope) - threshold_db;
        if over > 0.0 {
            let gain = from_db(-over * (1.0 - 1.0 / ratio));
            for channel in channels.iter_mut() {
                channel[i] *= gain;
            }
        }
    }
}

/// Turns down the treble (above about 5 kHz) of one channel at moments when sibilance (the
/// 5–9 kHz band) is strong compared with the voice as a whole, leaving the rest alone.
///
/// The sound is split at 5 kHz with a Linkwitz-Riley crossover (two Butterworth filters in a
/// row on each side), whose halves add back to the original level exactly, so with no
/// sibilance the sound is unchanged.
pub fn de_ess(channel: &mut [f32]) {
    let mut detector = Biquad::band_pass(6_500.0, RATE, 0.9);
    let butterworth = std::f32::consts::FRAC_1_SQRT_2;
    let mut low = [Biquad::low_pass(5_000.0, RATE, butterworth); 2];
    let mut high = [Biquad::high_pass(5_000.0, RATE, butterworth); 2];
    let (attack, release) = (coefficient(1.0), coefficient(60.0));
    let (mut sibilance, mut whole) = (0.0f32, 0.0f32);
    for sample in channel.iter_mut() {
        let x = *sample;
        let band = detector.run(x).abs();
        let k = if band > sibilance { attack } else { release };
        sibilance = k * sibilance + (1.0 - k) * band;
        let k = if x.abs() > whole { attack } else { release };
        whole = k * whole + (1.0 - k) * x.abs();
        // Sibilance above 40% of the whole level is pulled back, at most by 10 dB.
        let limit = whole * 0.4;
        let gain = if sibilance > limit && sibilance > 1e-4 { (limit / sibilance).powf(0.6).max(from_db(-10.0)) } else { 1.0 };
        let below = low[0].run(x);
        let below = low[1].run(below);
        let above = high[0].run(x);
        let above = high[1].run(above);
        *sample = below + gain * above;
    }
}

/// Scales every channel by `gain_db`.
pub fn apply_gain(channels: &mut [Vec<f32>], gain_db: f32) {
    let gain = from_db(gain_db);
    for sample in channels.iter_mut().flatten() {
        *sample *= gain;
    }
}

/// Keeps every peak at or under `ceiling_db`, on all channels together. Gain comes down
/// smoothly just before a peak (the whole signal is known, so it can look ahead) and recovers
/// over about 80 ms after.
pub fn limit(channels: &mut [Vec<f32>], ceiling_db: f32) {
    const AHEAD: usize = 240; // 5 ms
    let ceiling = from_db(ceiling_db);
    let length = channels.iter().map(Vec::len).min().unwrap_or(0);
    if length == 0 {
        return;
    }
    // The gain each sample needs on its own.
    let needed: Vec<f32> = (0..length).map(|i| {
        let peak = channels.iter().map(|channel| channel[i].abs()).fold(0.0, f32::max);
        if peak > ceiling { ceiling / peak } else { 1.0 }
    }).collect();
    // The lowest need in the next `AHEAD` samples, so the gain is already down when a peak comes.
    let mut lowest = vec![1.0f32; length];
    let mut window = std::collections::VecDeque::<usize>::new();
    for i in (0..length).rev() {
        while window.back().is_some_and(|&j| needed[j] >= needed[i]) {
            window.pop_back();
        }
        window.push_back(i);
        while window.front().is_some_and(|&j| j > i + AHEAD) {
            window.pop_front();
        }
        lowest[i] = needed[*window.front().expect("just pushed")];
    }
    // Average over the previous `AHEAD` samples: a smooth ramp that is still low enough at
    // each peak (every value averaged there saw that peak). Then recover slowly.
    let release = coefficient(80.0);
    let mut running = 0.0f32;
    let mut gain = 1.0f32;
    for i in 0..length {
        running += lowest[i];
        if i >= AHEAD {
            running -= lowest[i - AHEAD];
        }
        let smooth = running / (i + 1).min(AHEAD) as f32;
        let target = smooth.min(lowest[i]);
        gain = if target < gain { target } else { release * gain + (1.0 - release) * target };
        for channel in channels.iter_mut() {
            channel[i] *= gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frequency: f32, level: f32, seconds: f32) -> Vec<f32> {
        (0..(seconds * RATE) as usize).map(|i| level * (2.0 * std::f32::consts::PI * frequency * i as f32 / RATE).sin()).collect()
    }

    #[test]
    fn the_limiter_keeps_peaks_under_the_ceiling() {
        let mut channels = vec![sine(200.0, 2.0, 0.5)];
        limit(&mut channels, -1.0);
        let peak = channels[0].iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
        assert!(peak <= from_db(-1.0) + 1e-4, "{peak}");
        // Quiet sound is untouched.
        let mut quiet = vec![sine(200.0, 0.1, 0.5)];
        let before = quiet.clone();
        limit(&mut quiet, -1.0);
        assert_eq!(quiet, before);
    }

    #[test]
    fn compression_narrows_the_gap_between_loud_and_quiet() {
        let mut channels = vec![[sine(300.0, 0.05, 0.5), sine(300.0, 0.8, 0.5)].concat()];
        compress(&mut channels, -24.0, 3.0);
        let level = |part: &[f32]| (part.iter().map(|s| s * s).sum::<f32>() / part.len() as f32).sqrt();
        let (quiet, loud) = (level(&channels[0][12_000..24_000]), level(&channels[0][36_000..48_000]));
        assert!(loud / quiet < 0.8 / 0.05 * 0.5, "the loud half came down: {}", loud / quiet);
    }

    #[test]
    fn de_essing_turns_down_harsh_treble_and_leaves_the_voice() {
        let level = |signal: &[f32]| (signal[24_000..].iter().map(|s| s * s).sum::<f32>() / (signal.len() - 24_000) as f32).sqrt();
        let mut hiss = sine(7_000.0, 0.5, 1.0);
        de_ess(&mut hiss);
        assert!(level(&hiss) < 0.5 * 0.707 * 0.75, "sibilance comes down: {}", level(&hiss));
        let mut voice = sine(250.0, 0.5, 1.0);
        let before = level(&voice);
        de_ess(&mut voice);
        assert!((level(&voice) - before).abs() < 0.01, "the voice stays");
    }
}
