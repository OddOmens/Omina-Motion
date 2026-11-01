//! Decodes and mixes real audio: `tone.wav` is 2 s of 16 kHz mono, one second of silence and
//! then one second of a 440 Hz tone, so timing can be checked by where the sound starts.

use std::path::PathBuf;

use studio_core::audio::AudioClip;
use studio_core::model::Id;
use studio_media::{AudioDecoder, CHANNELS, MixRange, Mixer, probe_audio};

/// The mixer's rate in these tests: different from the file's, so resampling is exercised.
const RATE: u32 = 48_000;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

/// Root-mean-square loudness of frames `range` of interleaved stereo `samples`.
fn loudness(samples: &[f32], range: std::ops::Range<usize>) -> f32 {
    let part = &samples[range.start * CHANNELS..range.end * CHANNELS];
    (part.iter().map(|s| s * s).sum::<f32>() / part.len().max(1) as f32).sqrt()
}

/// Reads everything a decoder has left.
fn read_all(decoder: &mut AudioDecoder) -> Vec<f32> {
    let mut all = Vec::new();
    let mut block = vec![0.0; 1000 * CHANNELS];
    loop {
        let frames = decoder.read(&mut block).unwrap();
        all.extend_from_slice(&block[..frames * CHANNELS]);
        if frames < 1000 {
            return all;
        }
    }
}

fn tone_clip(layer: u64, offset_seconds: f64) -> AudioClip {
    AudioClip {
        layer: Id(layer),
        path: fixture("tone.wav"),
        offset_seconds,
        start_seconds: offset_seconds,
        end_seconds: offset_seconds + 2.0,
        gain: 1.0,
        fade_in: 0.0,
        fade_out: 0.0,
    }
}

#[test]
fn probing_reads_length_rate_and_channels() {
    let info = probe_audio(&fixture("tone.wav")).unwrap();
    assert_eq!((info.sample_rate, info.channels), (16_000, 1));
    assert!((info.duration_seconds - 2.0).abs() < 0.01);
    // Video files with a sound track work too.
    assert!(probe_audio(&fixture("colors.mp4")).is_ok());
}

#[test]
fn decodes_the_whole_file_as_stereo_at_the_mixer_rate() {
    let mut decoder = AudioDecoder::open(&fixture("tone.wav"), RATE).unwrap();
    let all = read_all(&mut decoder);
    let frames = all.len() / CHANNELS;
    assert!((frames as i64 - 2 * i64::from(RATE)).abs() < 100, "{frames} frames");
    assert!(loudness(&all, 0..47_000) < 0.001, "the first second is silent");
    assert!(loudness(&all, 49_000..95_000) > 0.05, "the second second has the tone");
    // Mono reaches both speakers equally.
    let i = 60_000 * CHANNELS;
    assert_eq!(all[i], all[i + 1]);
}

#[test]
fn seeking_is_sample_accurate() {
    let mut decoder = AudioDecoder::open(&fixture("tone.wav"), RATE).unwrap();
    let sequential = read_all(&mut decoder);
    // Land 10 ms before the tone starts: 480 frames of silence, then sound.
    decoder.seek(47_520).unwrap();
    assert_eq!(decoder.position(), 47_520);
    let mut block = vec![0.0; 2000 * CHANNELS];
    assert_eq!(decoder.read(&mut block).unwrap(), 2000);
    assert!(loudness(&block, 0..440) < 0.001);
    assert!(loudness(&block, 520..2000) > 0.05);
    // And it matches reading from the start, past the resampler's few warm-up samples.
    let from_start = &sequential[(47_520 + 600) * CHANNELS..(47_520 + 2000) * CHANNELS];
    let worst = from_start.iter().zip(&block[600 * CHANNELS..]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
    assert!(worst < 0.01, "seeked audio differs by {worst}");
}

#[test]
fn decodes_the_sound_of_a_video_file() {
    let mut decoder = AudioDecoder::open(&fixture("colors.mp4"), RATE).unwrap();
    let frames = read_all(&mut decoder).len() / CHANNELS;
    assert!((frames as i64 - 2 * i64::from(RATE)).abs() < 2400, "{frames} frames");
}

#[test]
fn clips_play_at_their_place_on_the_timeline() {
    // The clip starts at 0.5 s, so its tone is heard from 1.5 s to 2.5 s; 3 s are played once.
    let clips = [tone_clip(1, 0.5)];
    let mut mixer = Mixer::new(RATE, MixRange { start: 0, end: 3 * 48_000, looping: false }, 0);
    let mut out = vec![0.0; 4 * 48_000 * CHANNELS];
    assert_eq!(mixer.mix(&clips, &mut out), 3 * 48_000, "stops at the end of the range");
    assert!(loudness(&out, 0..71_000) < 0.001);
    assert!(loudness(&out, 73_000..119_000) > 0.05);
    assert!(loudness(&out, 121_000..144_000) < 0.001);
}

#[test]
fn looping_wraps_back_to_the_range_start() {
    // Loop 1.6 s – 2.0 s, all inside the tone: every block of the loop must sound.
    let clips = [tone_clip(1, 0.0)];
    let range = MixRange { start: 76_800, end: 96_000, looping: true };
    let mut mixer = Mixer::new(RATE, range, 76_800);
    let mut out = vec![0.0; 48_000 * CHANNELS];
    assert_eq!(mixer.mix(&clips, &mut out), 48_000, "a loop never runs out");
    for start in (0..47_000).step_by(1000) {
        assert!(loudness(&out, start..start + 1000) > 0.05, "silent at frame {start}");
    }
}

#[test]
fn clips_add_up_and_follow_their_gain() {
    let mut single = Mixer::new(RATE, MixRange { start: 0, end: 96_000, looping: false }, 72_000);
    let mut one = vec![0.0; 4800 * CHANNELS];
    single.mix(&[tone_clip(1, 0.0)], &mut one);

    let mut double = Mixer::new(RATE, MixRange { start: 0, end: 96_000, looping: false }, 72_000);
    let mut two = vec![0.0; 4800 * CHANNELS];
    double.mix(&[tone_clip(1, 0.0), tone_clip(2, 0.0)], &mut two);
    let ratio = loudness(&two, 0..4800) / loudness(&one, 0..4800);
    assert!((ratio - 2.0).abs() < 0.01, "two copies are {ratio}× as loud");

    let quiet = AudioClip { gain: 0.0, ..tone_clip(1, 0.0) };
    let mut muted = Mixer::new(RATE, MixRange { start: 0, end: 96_000, looping: false }, 72_000);
    muted.mix(&[quiet], &mut one);
    assert!(loudness(&one, 0..4800) == 0.0);
}

#[test]
fn fades_rise_from_silence_and_fall_back_to_it() {
    // Half a second of fade at each end of the two-second tone.
    let faded = AudioClip { fade_in: 0.5, fade_out: 0.5, ..tone_clip(1, 0.0) };
    let mut mixer = Mixer::new(RATE, MixRange { start: 0, end: 96_000, looping: false }, 0);
    let mut out = vec![0.0; 96_000 * CHANNELS];
    mixer.mix(&[faded], &mut out);
    let (start, middle, end) = (loudness(&out, 0..2400), loudness(&out, 46_000..50_800), loudness(&out, 93_600..96_000));
    assert!(start < middle * 0.1, "nearly silent at the start: {start} vs {middle}");
    assert!(end < middle * 0.1, "and at the end: {end}");
    assert!(loudness(&out, 12_000..12_480) < middle * 0.6, "half way up a quarter second in");
}

#[test]
fn a_missing_file_is_silence_not_a_failure() {
    let missing = AudioClip { path: fixture("missing.wav"), ..tone_clip(1, 0.0) };
    let mut mixer = Mixer::new(RATE, MixRange { start: 0, end: 96_000, looping: false }, 0);
    let mut out = vec![1.0; 4800 * CHANNELS];
    assert_eq!(mixer.mix(&[missing], &mut out), 4800);
    assert!(out.iter().all(|sample| *sample == 0.0));
}
