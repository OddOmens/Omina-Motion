//! Second-order ("biquad") filters from Robert Bristow-Johnson's Audio EQ Cookbook: the
//! building block of the EQ, the hum notches and the de-esser's detector.

use std::f64::consts::PI;

/// One biquad filter with its own memory, for one channel. It computes in 64-bit floats:
/// narrow filters (the hum notches) put their poles so close to instability that 32-bit
/// rounding turns into audible noise.
#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    // The last two inputs and outputs.
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    pub(crate) fn from_coefficients(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
    }

    /// The angle per sample and the bandwidth term for `frequency` (Hz) at `rate` with quality `q`.
    fn omega(frequency: f32, rate: f32, q: f32) -> (f64, f64, f64) {
        let w = 2.0 * PI * f64::from((frequency / rate).clamp(1e-5, 0.49));
        (w.cos(), w.sin(), w.sin() / (2.0 * f64::from(q)))
    }

    /// Lets through what is above `frequency`.
    pub fn high_pass(frequency: f32, rate: f32, q: f32) -> Self {
        let (cos, _, alpha) = Self::omega(frequency, rate, q);
        Self::from_coefficients((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
    }

    /// Lets through what is below `frequency`.
    pub fn low_pass(frequency: f32, rate: f32, q: f32) -> Self {
        let (cos, _, alpha) = Self::omega(frequency, rate, q);
        Self::from_coefficients((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
    }

    /// Lets through what is around `frequency`, with a peak of 1.
    pub fn band_pass(frequency: f32, rate: f32, q: f32) -> Self {
        let (cos, _, alpha) = Self::omega(frequency, rate, q);
        Self::from_coefficients(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
    }

    /// Removes a narrow band around `frequency` (for mains hum).
    pub fn notch(frequency: f32, rate: f32, q: f32) -> Self {
        let (cos, _, alpha) = Self::omega(frequency, rate, q);
        Self::from_coefficients(1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
    }

    /// Raises (or with a negative `gain_db`, lowers) the band around `frequency`.
    pub fn peak(frequency: f32, rate: f32, q: f32, gain_db: f32) -> Self {
        let (cos, _, alpha) = Self::omega(frequency, rate, q);
        let a = 10f64.powf(f64::from(gain_db) / 40.0);
        Self::from_coefficients(1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a)
    }

    /// Raises everything above about `frequency` by `gain_db`.
    pub fn high_shelf(frequency: f32, rate: f32, q: f32, gain_db: f32) -> Self {
        let (cos, sin, _) = Self::omega(frequency, rate, q);
        let a = 10f64.powf(f64::from(gain_db) / 40.0);
        let alpha = sin / (2.0 * f64::from(q));
        let root = 2.0 * a.sqrt() * alpha;
        Self::from_coefficients(
            a * ((a + 1.0) + (a - 1.0) * cos + root),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
            a * ((a + 1.0) + (a - 1.0) * cos - root),
            (a + 1.0) - (a - 1.0) * cos + root,
            2.0 * ((a - 1.0) - (a + 1.0) * cos),
            (a + 1.0) - (a - 1.0) * cos - root,
        )
    }

    /// Filters one sample.
    pub fn run(&mut self, x: f32) -> f32 {
        let x = f64::from(x);
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        (self.x2, self.x1, self.y2, self.y1) = (self.x1, x, self.y1, y);
        y as f32
    }

    /// Filters a whole signal in place.
    pub fn run_all(&mut self, samples: &mut [f32]) {
        for sample in samples {
            *sample = self.run(*sample);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The level of a sine at `frequency` after `filter`, as a ratio of what went in.
    fn response(mut filter: Biquad, frequency: f32) -> f32 {
        let rate = 48_000.0;
        let mut signal: Vec<f32> = (0..48_000).map(|i| (2.0 * std::f32::consts::PI * frequency * i as f32 / rate).sin()).collect();
        filter.run_all(&mut signal);
        let tail = &signal[24_000..];
        (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32 * 2.0).sqrt()
    }

    #[test]
    fn filters_shape_the_spectrum_as_named() {
        let rate = 48_000.0;
        assert!(response(Biquad::high_pass(100.0, rate, 0.707), 30.0) < 0.15);
        assert!(response(Biquad::high_pass(100.0, rate, 0.707), 1000.0) > 0.98);
        assert!(response(Biquad::notch(60.0, rate, 30.0), 60.0) < 0.05);
        assert!(response(Biquad::notch(60.0, rate, 30.0), 200.0) > 0.98);
        let boosted = response(Biquad::peak(3000.0, rate, 1.0, 6.0), 3000.0);
        assert!((boosted - 2.0).abs() < 0.05, "+6 dB is twice the level: {boosted}");
        assert!((response(Biquad::high_shelf(5000.0, rate, 0.707, 6.0), 15_000.0) - 2.0).abs() < 0.1);
    }
}
