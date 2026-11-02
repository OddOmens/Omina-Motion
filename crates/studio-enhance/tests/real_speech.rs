//! Cleans a real noisy recording and measures how much noise is left. It needs two files, so it
//! only runs when asked: a clean recording of speech and the same speech with noise mixed in
//! (both 32-bit float WAV at 48 kHz, lined up), and a path for the result:
//! `ENHANCE_CLEAN=clean.wav ENHANCE_NOISY=noisy.wav ENHANCE_OUT=out.wav cargo test -p studio-enhance --release --test real_speech -- --ignored --nocapture`

use std::path::Path;

use studio_enhance::{Denoiser, Settings, enhance, integrated_loudness, read_float_wav, write_wav};

/// Noise left in the pauses (where the clean speech is silent), relative to the level while
/// speaking, in dB. Lower is cleaner.
fn noise_below_speech(clean: &[f32], signal: &[f32]) -> f32 {
    // 20 ms windows: speaking where the clean recording is within 40 dB of its loudest.
    const WINDOW: usize = 960;
    let level = |part: &[f32]| (part.iter().map(|s| s * s).sum::<f32>() / part.len() as f32).sqrt();
    let loudest = clean.chunks(WINDOW).map(level).fold(0.0, f32::max);
    let (mut speech, mut pauses) = (Vec::new(), Vec::new());
    for (clean, signal) in clean.chunks(WINDOW).zip(signal.chunks(WINDOW)) {
        let clean_level = level(clean);
        if clean_level > loudest * 0.1 {
            speech.push(level(signal));
        } else if clean_level < loudest * 0.001 {
            pauses.push(level(signal));
        }
    }
    let mean = |values: &[f32]| (values.iter().map(|v| v * v).sum::<f32>() / values.len().max(1) as f32).sqrt();
    20.0 * (mean(&pauses) / mean(&speech)).log10()
}

#[test]
#[ignore = "needs recordings; set ENHANCE_CLEAN, ENHANCE_NOISY and ENHANCE_OUT"]
fn noise_comes_down_on_real_speech() {
    let (clean, _) = read_float_wav(Path::new(&std::env::var("ENHANCE_CLEAN").unwrap())).unwrap();
    let (noisy, rate) = read_float_wav(Path::new(&std::env::var("ENHANCE_NOISY").unwrap())).unwrap();
    assert_eq!(rate, 48_000);
    let before = noise_below_speech(&clean[0], &noisy[0]);
    // Both must clearly help. Levelling lifts the quiet gaps a little, so the check is with it
    // on, as people will use them; the numbers printed say how much each does.
    for (denoiser, at_least) in [(Denoiser::Light, 8.0), (Denoiser::Studio, 8.0)] {
        for even_out in [false, true] {
            let started = std::time::Instant::now();
            let settings = Settings { denoiser: Some(denoiser), strength: 1.0, even_out, ..Settings::default() };
            let cleaned = enhance(&noisy, settings, &|_| {}, &|| false).unwrap().unwrap();
            let seconds = noisy[0].len() as f32 / 48_000.0;
            let after = noise_below_speech(&clean[0], &cleaned[0]);
            let views: Vec<&[f32]> = cleaned.iter().map(Vec::as_slice).collect();
            println!(
                "{denoiser:?} (even out {even_out}): noise {before:.1} dB → {after:.1} dB below the voice; {:.1} LUFS; {seconds:.1} s of sound in {:?}",
                integrated_loudness(&views).unwrap_or(f32::NAN),
                started.elapsed()
            );
            if even_out {
                assert!(after < before - at_least, "{denoiser:?} takes the noise down by over {at_least} dB");
                let out = std::env::var("ENHANCE_OUT").unwrap().replace(".wav", &format!("-{denoiser:?}.wav"));
                write_wav(Path::new(&out), &cleaned, 48_000).unwrap();
            }
        }
    }
}
