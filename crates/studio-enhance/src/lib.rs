//! Speech cleanup, like Adobe Podcast's Enhance Speech: takes a recording made in a noisy room
//! and makes the voice clear, even and podcast-loud. Everything runs on this computer; both
//! noise-removal models are built in.
//!
//! The steps, in order (each can be turned off):
//!
//! 1. **Hum**: mains hum (50 or 60 Hz, and its harmonics) is found and notched out.
//! 2. **Noise**: an AI model removes background sound (see `denoise`). `strength` mixes some
//!    of the original back in, for a more natural sound.
//! 3. **Voice EQ**: rumble below 80 Hz is cut, boxiness around 300 Hz eased, and presence
//!    (3–5 kHz) and air (10 kHz) lifted a little, as a broadcast voice chain does.
//! 4. **De-ess**: harsh "s" sounds are softened.
//! 5. **Even out**: a compressor narrows the gap between loud and quiet words, the whole is
//!    brought to -16 LUFS (the podcast and social-video standard), and a limiter keeps peaks
//!    under -1 dB.
//!
//! It works on whole recordings at 48 kHz, offline: the app renders a cleaned copy once and
//! plays that.

mod denoise;
mod dynamics;
mod filters;
mod hum;
mod loudness;
mod wav;

pub use denoise::{DenoiseError, Denoiser};
pub use loudness::integrated as integrated_loudness;
pub use wav::{read_float_wav, write_wav};

/// The sample rate everything here works at (both models need it).
pub const RATE: u32 = 48_000;
/// The loudness "Even out" aims for, in LUFS.
pub const TARGET_LUFS: f32 = -16.0;
/// The highest peak "Even out" allows, in dB below full scale.
const CEILING_DB: f32 = -1.0;

/// What to do to a recording.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// The noise remover, or `None` to keep the background.
    pub denoiser: Option<Denoiser>,
    /// How much of the cleaned sound to use, 0 to 1. Below 1, some of the original is mixed
    /// back in.
    pub strength: f32,
    pub remove_hum: bool,
    pub voice_eq: bool,
    pub de_ess: bool,
    pub even_out: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { denoiser: Some(Denoiser::Studio), strength: 0.9, remove_hum: true, voice_eq: true, de_ess: true, even_out: true }
    }
}

/// Cleans up `channels` (each a whole recording at `RATE`, all the same length). Channels that
/// are identical (mono recorded as stereo) are cleaned once. Calls `progress` with the share
/// done (0–1), and returns `Ok(None)` if `cancelled` says to stop.
pub fn enhance(channels: &[Vec<f32>], settings: Settings, progress: &(dyn Fn(f32) + Sync), cancelled: &(dyn Fn() -> bool + Sync)) -> Result<Option<Vec<Vec<f32>>>, DenoiseError> {
    let Some(length) = channels.iter().map(Vec::len).min() else { return Ok(Some(Vec::new())) };
    let mono = channels.windows(2).all(|pair| pair[0][..length] == pair[1][..length]);
    let mut working: Vec<Vec<f32>> = if mono { vec![channels[0][..length].to_vec()] } else { channels.iter().map(|channel| channel[..length].to_vec()).collect() };

    // 1. Hum, measured on the first channel and removed from all.
    if settings.remove_hum
        && let Some(base) = hum::detect(&working[0])
    {
        working.iter_mut().for_each(|channel| hum::remove(channel, base));
    }

    // 2. Noise: the slow part, so it gets almost all of the progress bar. Channels run side
    // by side.
    if let Some(denoiser) = settings.denoiser {
        let count = working.len() as f32;
        let shares: Vec<std::sync::Mutex<f32>> = working.iter().map(|_| std::sync::Mutex::new(0.0)).collect();
        let report = || {
            let done: f32 = shares.iter().map(|share| *share.lock().unwrap_or_else(|poisoned| poisoned.into_inner())).sum();
            progress(0.95 * done / count);
        };
        let results: Vec<Result<Option<Vec<f32>>, DenoiseError>> = std::thread::scope(|scope| {
            let jobs: Vec<_> = working
                .iter()
                .zip(&shares)
                .map(|(channel, share)| {
                    let report = &report;
                    scope.spawn(move || {
                        denoise::denoise(denoiser, channel, &|done| {
                            *share.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = done;
                            report();
                        }, cancelled)
                    })
                })
                .collect();
            jobs.into_iter().map(|job| job.join().unwrap_or_else(|_| Err(DenoiseError("the noise removal stopped unexpectedly".into())))).collect()
        });
        let strength = settings.strength.clamp(0.0, 1.0);
        for (channel, result) in working.iter_mut().zip(results) {
            let Some(cleaned) = result? else { return Ok(None) };
            for (sample, clean) in channel.iter_mut().zip(cleaned) {
                *sample = clean * strength + *sample * (1.0 - strength);
            }
        }
    }
    if cancelled() {
        return Ok(None);
    }

    // 3. and 4. Tone, per channel.
    for channel in &mut working {
        if settings.voice_eq {
            voice_eq(channel);
        }
        if settings.de_ess {
            dynamics::de_ess(channel);
        }
    }

    // 5. Level.
    if settings.even_out {
        dynamics::compress(&mut working, -26.0, 3.0);
        let views: Vec<&[f32]> = working.iter().map(Vec::as_slice).collect();
        // A mono voice counts once, as it would played in the middle of two speakers.
        if let Some(loudness) = loudness::integrated(&views) {
            let as_heard = if mono && channels.len() > 1 { loudness + 3.01 } else { loudness };
            dynamics::apply_gain(&mut working, (TARGET_LUFS - as_heard).clamp(-30.0, 30.0));
        }
    }
    // EQ and gain may push peaks past full scale; the limiter always has the last word.
    dynamics::limit(&mut working, CEILING_DB);
    progress(1.0);

    Ok(Some(if mono { vec![working[0].clone(); channels.len()] } else { working }))
}

/// A gentle broadcast voice EQ.
fn voice_eq(channel: &mut [f32]) {
    let rate = RATE as f32;
    for mut filter in [
        filters::Biquad::high_pass(80.0, rate, 0.707),
        filters::Biquad::peak(300.0, rate, 1.0, -2.0),
        filters::Biquad::peak(4_000.0, rate, 0.9, 2.5),
        filters::Biquad::high_shelf(10_000.0, rate, 0.707, 1.5),
    ] {
        filter.run_all(channel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_result_is_at_podcast_loudness_with_safe_peaks() {
        // Noise reduction itself is checked on real speech in `tests/real_speech.rs`.
        let quiet: Vec<f32> = denoise::tests::syllables(6.0).iter().map(|s| s * 0.1).collect();
        let settings = Settings { denoiser: Some(Denoiser::Light), ..Settings::default() };
        let cleaned = enhance(&[quiet.clone(), quiet], settings, &|_| {}, &|| false).unwrap().unwrap();
        assert_eq!(cleaned[0], cleaned[1], "a mono recording stays mono");
        let lufs = integrated_loudness(&[&cleaned[0], &cleaned[1]]).unwrap();
        assert!((lufs - TARGET_LUFS).abs() < 1.5, "{lufs} LUFS");
        let peak = cleaned[0].iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
        assert!(peak <= 10f32.powf(CEILING_DB / 20.0) + 1e-3, "peak {peak}");
    }

    #[test]
    fn everything_off_leaves_the_sound_as_it_was_but_safe() {
        let voice = denoise::tests::syllables(2.0);
        let off = Settings { denoiser: None, strength: 1.0, remove_hum: false, voice_eq: false, de_ess: false, even_out: false };
        let out = enhance(std::slice::from_ref(&voice), off, &|_| {}, &|| false).unwrap().unwrap();
        assert_eq!(out[0], voice);
    }
}
