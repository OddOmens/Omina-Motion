//! Mains hum: the 50 Hz (Europe, most of Asia) or 60 Hz (the Americas) buzz that cables and
//! lights put into recordings, with its harmonics. It is found by measuring, and removed with
//! very narrow notches that leave the voice alone.

use studio_dsp::Biquad;

/// The hum's base frequency in `channel`, if there is clearly one: 50 or 60 Hz standing well
/// above the frequencies next to it.
pub fn detect(channel: &[f32]) -> Option<f32> {
    // A minute is plenty to measure, and keeps this quick on long recordings.
    let sample = &channel[..channel.len().min(48_000 * 60)];
    // The base frequency against its neighbours a few hertz away. Voices hardly reach down to
    // 50–60 Hz, so a clear peak there is hum.
    let strength = |base: f32| power(sample, base) / ((power(sample, base - 7.0) + power(sample, base + 7.0)) / 2.0).max(1e-12);
    let (fifty, sixty) = (strength(50.0), strength(60.0));
    let (base, ratio) = if fifty > sixty { (50.0, fifty) } else { (60.0, sixty) };
    (ratio > 4.0).then_some(base)
}

/// The power of `signal` at one frequency (the Goertzel algorithm: a single bin of a Fourier
/// transform, cheaply).
fn power(signal: &[f32], frequency: f32) -> f32 {
    let coefficient = 2.0 * (2.0 * std::f32::consts::PI * frequency / 48_000.0).cos();
    let (mut previous, mut before) = (0.0f32, 0.0f32);
    for &x in signal {
        let current = x + coefficient * previous - before;
        (before, previous) = (previous, current);
    }
    (previous * previous + before * before - coefficient * previous * before) / signal.len().max(1) as f32
}

/// Notches out `base` and its harmonics up to 1 kHz: about 2 Hz wide at the base, widening a
/// little higher up (harmonics drift more), never so sharp that the filter rings.
pub fn remove(channel: &mut [f32], base: f32) {
    let mut harmonic = base;
    while harmonic < 1_000.0 {
        Biquad::notch(harmonic, 48_000.0, (harmonic / 2.0).min(60.0)).run_all(channel);
        harmonic += base;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hum_is_found_and_removed() {
        let voice = crate::denoise::tests::syllables(3.0);
        let hum: Vec<f32> = (0..voice.len()).map(|i| {
            let t = i as f32 / 48_000.0;
            0.05 * (2.0 * std::f32::consts::PI * 60.0 * t).sin() + 0.03 * (2.0 * std::f32::consts::PI * 180.0 * t).sin()
        }).collect();
        let mut humming: Vec<f32> = voice.iter().zip(&hum).map(|(v, h)| v + h).collect();
        assert_eq!(detect(&humming), Some(60.0));
        assert_eq!(detect(&voice), None, "a voice alone has no hum");
        let before = power(&humming[24_000..], 60.0);
        remove(&mut humming, 60.0);
        assert!(power(&humming[24_000..], 60.0) < before / 100.0, "60 Hz is 20 dB down");
    }
}
