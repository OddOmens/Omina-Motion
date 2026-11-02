//! The two noise removers, run over a whole channel at 48 kHz, with their built-in delay taken
//! out so the cleaned sound lines up with the picture exactly.
//!
//! - **Light**: RNNoise (Xiph), a small network that turns down noisy frequency bands. Very
//!   fast; good on steady noise such as fans, hiss and hum.
//! - **Studio**: DeepFilterNet3, a larger network that also rebuilds the fine detail of the
//!   voice. Handles traffic, keyboards, dishes and rooms full of people much better, at a few
//!   percent of one processor core per second of sound.

use ndarray::Array2;

/// Which network removes the noise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denoiser {
    Light,
    Studio,
}

/// Samples per frame for both networks: 10 ms at 48 kHz.
const FRAME: usize = 480;

/// Something that went wrong, in words for the person using the app.
#[derive(Clone, Debug, PartialEq)]
pub struct DenoiseError(pub String);

/// Removes noise from one channel. Calls `progress` with the share done (0–1) now and then,
/// and stops with `Ok(None)` when `cancelled` says so.
pub fn denoise(denoiser: Denoiser, channel: &[f32], progress: &dyn Fn(f32), cancelled: &dyn Fn() -> bool) -> Result<Option<Vec<f32>>, DenoiseError> {
    match denoiser {
        Denoiser::Light => Ok(light(channel, progress, cancelled)),
        Denoiser::Studio => studio(channel, progress, cancelled),
    }
}

/// Runs `step` over `channel` in frames of `FRAME`, padded at the end so the last of the sound
/// comes out too, and drops the network's `delay` from the start.
fn by_frames(channel: &[f32], delay: usize, progress: &dyn Fn(f32), cancelled: &dyn Fn() -> bool, mut step: impl FnMut(&[f32], &mut [f32])) -> Option<Vec<f32>> {
    let total = channel.len() + delay;
    let frames = total.div_ceil(FRAME);
    let mut padded = channel.to_vec();
    padded.resize(frames * FRAME, 0.0);
    let mut output = vec![0.0; frames * FRAME];
    for (index, (input, out)) in padded.as_chunks::<FRAME>().0.iter().zip(output.as_chunks_mut::<FRAME>().0.iter_mut()).enumerate() {
        // Checking every 100 frames (a second of sound) is often enough.
        if index % 100 == 0 {
            if cancelled() {
                return None;
            }
            progress(index as f32 / frames as f32);
        }
        step(input, out);
    }
    output.drain(..delay.min(output.len()));
    output.truncate(channel.len());
    Some(output)
}

fn light(channel: &[f32], progress: &dyn Fn(f32), cancelled: &dyn Fn() -> bool) -> Option<Vec<f32>> {
    // RNNoise works in 16-bit sample units.
    const SCALE: f32 = 32_768.0;
    let mut state = nnnoiseless::DenoiseState::new();
    let mut scaled = [0.0f32; FRAME];
    by_frames(channel, LIGHT_DELAY, progress, cancelled, |input, out| {
        for (scaled, &sample) in scaled.iter_mut().zip(input) {
            *scaled = sample * SCALE;
        }
        state.process_frame(out, &scaled);
        for sample in out.iter_mut() {
            *sample /= SCALE;
        }
    })
}

/// RNNoise's output lags its input by one frame (its analysis window overlaps the frame before).
const LIGHT_DELAY: usize = FRAME;

fn failed(error: impl std::fmt::Display) -> DenoiseError {
    DenoiseError(format!("the noise removal model failed: {error}"))
}

fn studio(channel: &[f32], progress: &dyn Fn(f32), cancelled: &dyn Fn() -> bool) -> Result<Option<Vec<f32>>, DenoiseError> {
    use deep_filter_rt::tract::{DfParams, DfTract, RuntimeParams};
    let mut model = DfTract::new(DfParams::default(), &RuntimeParams::default_with_ch(1)).map_err(failed)?;
    if model.hop_size != FRAME || model.sr != 48_000 {
        return Err(DenoiseError("the noise removal model expects a different sound format".into()));
    }
    let delay = model.delay();
    let mut noisy = Array2::<f32>::zeros((1, FRAME));
    let mut enhanced = Array2::<f32>::zeros((1, FRAME));
    let mut error = None;
    let cleaned = by_frames(channel, delay, progress, cancelled, |input, out| {
        if error.is_some() {
            return;
        }
        noisy.row_mut(0).iter_mut().zip(input).for_each(|(slot, &sample)| *slot = sample);
        match model.process(noisy.view(), enhanced.view_mut()) {
            Ok(_) => out.iter_mut().zip(enhanced.row(0)).for_each(|(slot, &sample)| *slot = sample),
            Err(failure) => error = Some(failed(failure)),
        }
    });
    match error {
        Some(error) => Err(error),
        None => Ok(cleaned),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A speech-like test sound: syllables of a few harmonics, with pauses, at 48 kHz.
    pub fn syllables(seconds: f32) -> Vec<f32> {
        (0..(seconds * 48_000.0) as usize)
            .map(|i| {
                let t = i as f32 / 48_000.0;
                let syllable = (t * 4.0).fract();
                let envelope = if syllable < 0.6 { (syllable / 0.6 * std::f32::consts::PI).sin() } else { 0.0 };
                let pitch = 140.0 + 30.0 * (t * 1.3).sin();
                let voice: f32 = (1..8).map(|h| (2.0 * std::f32::consts::PI * pitch * h as f32 * t).sin() / h as f32).sum();
                0.25 * envelope * voice
            })
            .collect()
    }

    /// The shift (in samples, within ±`reach`) at which `output` best matches `input`.
    fn lag(input: &[f32], output: &[f32], reach: i64) -> i64 {
        (-reach..=reach)
            .max_by(|&a, &b| {
                let score = |shift: i64| -> f32 {
                    (24_000..input.len() - 24_000).map(|i| input[i] * output[(i as i64 + shift) as usize]).sum()
                };
                score(a).total_cmp(&score(b))
            })
            .unwrap()
    }

    #[test]
    fn both_noise_removers_keep_the_sound_in_step() {
        let clean = syllables(3.0);
        for denoiser in [Denoiser::Light, Denoiser::Studio] {
            let out = denoise(denoiser, &clean, &|_| {}, &|| false).unwrap().unwrap();
            assert_eq!(out.len(), clean.len());
            // Within a tenth of a millisecond (filtering shifts phase a little).
            let shift = lag(&clean, &out, 1_000);
            assert!(shift.abs() <= 5, "{denoiser:?} lines up with the original: {shift} samples off");
        }
    }

    #[test]
    fn cancelling_stops_early() {
        assert_eq!(denoise(Denoiser::Light, &syllables(2.0), &|_| {}, &|| true).unwrap(), None);
    }
}
