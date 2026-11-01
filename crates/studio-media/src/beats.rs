//! Finding the beats in music, for beat markers. The sound is decoded at a low rate, reduced
//! to an onset curve (how sharply the loudness rises at each moment), and the tempo is the
//! beat spacing at which that curve best repeats. The beats are then laid on the strongest
//! onsets at that spacing, each nudged onto the nearest hit so a drifting tempo still lines up.

use std::path::Path;

use crate::audio::{AudioDecoder, CHANNELS};
use crate::decode::MediaError;

/// The rate sound is analyzed at: plenty for drums and bass, quick to decode.
const RATE: u32 = 11_025;
/// Samples per onset step (about 23 ms).
const HOP: usize = 256;
/// The tempos looked for, in beats per minute.
const TEMPO_RANGE: (f64, f64) = (70.0, 180.0);

/// The beats in `path`'s sound, as seconds from its start.
pub fn detect_beats(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<Vec<f64>, MediaError> {
    let mut decoder = AudioDecoder::open(path, RATE)?;
    let mut block = vec![0.0f32; HOP * CHANNELS];
    let mut energy = Vec::new();
    loop {
        if cancelled() {
            return Ok(Vec::new());
        }
        let read = decoder.read(&mut block)?;
        if read == 0 {
            break;
        }
        let sum: f32 = block[..read * CHANNELS].iter().map(|sample| sample * sample).sum();
        energy.push(sum / (read * CHANNELS) as f32);
        if read < HOP {
            break;
        }
    }
    Ok(beats_from_energy(&energy, f64::from(RATE) / HOP as f64))
}

/// The beats from loudness steps (`steps_per_second` of them), as seconds. Split out from
/// decoding so it can be tested on made-up sound.
pub fn beats_from_energy(energy: &[f32], steps_per_second: f64) -> Vec<f64> {
    // The onset curve: rises in loudness (on a log scale, so quiet and loud passages count alike).
    let log: Vec<f32> = energy.iter().map(|value| (value + 1e-9).ln()).collect();
    let onsets: Vec<f32> = log.windows(2).map(|pair| (pair[1] - pair[0]).max(0.0)).collect();
    if onsets.len() < 8 {
        return Vec::new();
    }
    // The tempo: the beat spacing (in steps) where the onset curve best matches itself, with a
    // gentle preference for tempos near 120, where most music sits.
    let shortest = (60.0 / TEMPO_RANGE.1 * steps_per_second).floor() as usize;
    let longest = ((60.0 / TEMPO_RANGE.0 * steps_per_second).ceil() as usize).min(onsets.len() / 2);
    if shortest == 0 || longest <= shortest {
        return Vec::new();
    }
    let score = |lag: usize| -> f64 {
        let matched: f64 = onsets.iter().zip(&onsets[lag..]).map(|(a, b)| f64::from(a * b)).sum();
        let bpm = 60.0 * steps_per_second / lag as f64;
        let preference = (-((bpm / 120.0).log2()).powi(2) / 0.5).exp();
        matched / (onsets.len() - lag) as f64 * preference
    };
    let Some(period) = (shortest..=longest).max_by(|a, b| score(*a).total_cmp(&score(*b))) else { return Vec::new() };
    // Where the beats fall: the offset within one period that catches the most onset.
    let phase = (0..period)
        .max_by(|&a, &b| {
            let caught = |offset: usize| -> f32 { onsets.iter().skip(offset).step_by(period).sum() };
            caught(a).total_cmp(&caught(b))
        })
        .unwrap_or(0);
    // Lay the beats out, each moved onto the strongest onset within a few steps of where the
    // tempo puts it, and carry on from there (so the grid follows small tempo changes).
    let reach = (period / 6).max(1);
    let mut beats = Vec::new();
    let mut at = phase;
    while at < onsets.len() {
        // The first beat may be up to half a beat from where the whole-step spacing put it.
        let reach = if beats.is_empty() { period / 2 } else { reach };
        let from = at.saturating_sub(reach);
        let to = (at + reach).min(onsets.len() - 1);
        let peak = (from..=to).max_by(|&a, &b| onsets[a].total_cmp(&onsets[b])).unwrap_or(at);
        let chosen = if onsets[peak] > 0.0 { peak } else { at };
        // The onset at step `i` is the rise into step `i + 1`.
        beats.push((chosen + 1) as f64 / steps_per_second);
        at = chosen + period;
    }
    beats
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loudness for a click track at `bpm`, starting `offset` seconds in, with a quiet hiss.
    fn click_track(bpm: f64, offset: f64, seconds: f64, steps_per_second: f64) -> Vec<f32> {
        let steps = (seconds * steps_per_second) as usize;
        let beat = 60.0 / bpm;
        (0..steps)
            .map(|step| {
                let time = step as f64 / steps_per_second;
                let since = (time - offset).rem_euclid(beat);
                // A sharp hit decaying over ~80 ms, over steady hiss.
                let hit = if time >= offset { (-since / 0.08).exp() as f32 } else { 0.0 };
                0.001 + hit * 0.5
            })
            .collect()
    }

    #[test]
    fn a_click_track_gives_its_beats() {
        let rate = f64::from(RATE) / HOP as f64;
        for bpm in [90.0, 120.0, 150.0] {
            let beats = beats_from_energy(&click_track(bpm, 0.3, 12.0, rate), rate);
            let spacing = 60.0 / bpm;
            assert!(beats.len() as f64 > 12.0 / spacing * 0.8, "{bpm} BPM: found {} beats", beats.len());
            // Every beat sits on a click, within a step.
            for &beat in &beats {
                let off = ((beat - 0.3) / spacing - ((beat - 0.3) / spacing).round()).abs() * spacing;
                assert!(off < 2.0 / rate, "{bpm} BPM: beat at {beat:.3} s is {off:.3} s off the grid");
            }
        }
    }

    #[test]
    fn silence_has_no_beats_and_short_sound_is_left_alone() {
        assert!(beats_from_energy(&[0.0; 4], 43.0).is_empty());
        let flat = vec![0.01f32; 500];
        let beats = beats_from_energy(&flat, 43.0);
        assert!(beats.len() <= 500, "{}", beats.len());
    }
}

/// Runs on a real file: `BEATS_FILE=<a 128 BPM click track starting at 0.25 s> cargo test -p studio-media beats_in_a_file -- --ignored`.
#[test]
#[ignore = "needs a click track; set BEATS_FILE"]
fn beats_in_a_file() {
    let path = std::env::var("BEATS_FILE").unwrap();
    let beats = detect_beats(Path::new(&path), &|| false).unwrap();
    let spacing = 60.0 / 128.0;
    println!("{} beats: {:?}", beats.len(), &beats[..beats.len().min(6)]);
    assert!(beats.len() >= 20);
    for beat in beats {
        let off = ((beat - 0.25) / spacing - ((beat - 0.25) / spacing).round()).abs() * spacing;
        assert!(off < 0.05, "beat at {beat:.3} is {off:.3} s off");
    }
}
