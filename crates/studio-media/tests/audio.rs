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
    AudioClip::plain(Id(layer), fixture("tone.wav"), offset_seconds, offset_seconds, offset_seconds + 2.0)
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
fn panning_turns_down_the_other_side_only() {
    // The tone starts a second in, so mix from 1.5 s.
    let side_levels = |pan: f32| {
        let clip = AudioClip { pan, ..tone_clip(1, 0.0) };
        let mut mixer = Mixer::new(RATE, MixRange { start: 0, end: 96_000, looping: false }, 72_000);
        let mut block = vec![0.0; 4800 * CHANNELS];
        mixer.mix(&[clip], &mut block);
        let side = |offset: usize| (block.iter().skip(offset).step_by(CHANNELS).map(|sample| sample * sample).sum::<f32>() / 4800.0).sqrt();
        (side(0), side(1))
    };
    let (left, right) = side_levels(0.0);
    assert!(left > 0.05 && (left - right).abs() < 1e-6, "centered sound is equal on both sides");
    let (panned_left, panned_right) = side_levels(-1.0);
    assert!((panned_left - left).abs() < 1e-6, "the side panned toward keeps its level");
    assert_eq!(panned_right, 0.0, "the other side is silent");
    let (half_left, half_right) = side_levels(0.5);
    assert!((half_right - right).abs() < 1e-6 && (half_left - left * 0.5).abs() < 1e-4);
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

#[test]
fn double_speed_plays_the_tone_half_a_second_in() {
    let fast = AudioClip { speed: 2.0, end_seconds: 1.0, ..tone_clip(1, 0.0) };
    let mut mixer = Mixer::new(RATE, MixRange { start: 0, end: 48_000, looping: false }, 0);
    let mut out = vec![0.0; 48_000 * CHANNELS];
    mixer.mix(&[fast], &mut out);
    assert!(loudness(&out, 1000..20_000) < 0.01, "the silent second goes by in half a second");
    assert!(loudness(&out, 28_000..46_000) > 0.05, "then the tone");
}

#[test]
fn ducking_turns_a_clip_down_while_the_other_sounds() {
    // The "music" plays the tone from the start; the "voice" is the same file, so it is silent
    // for a second and then sounds.
    let voice = tone_clip(1, 0.0);
    let music = AudioClip { duck_under: Some(Id(1)), duck_db: 20.0, ..tone_clip(2, 0.0) };
    let music_alone = tone_clip(2, 0.0);
    let mut ducked = Mixer::new(RATE, MixRange { start: 0, end: 96_000, looping: false }, 0);
    let mut out = vec![0.0; 96_000 * CHANNELS];
    // Small blocks, as the sound card asks for.
    for block in out.chunks_mut(1024 * CHANNELS) {
        ducked.mix(&[voice.clone(), music.clone()], block);
    }
    let mut plain = Mixer::new(RATE, MixRange { start: 0, end: 96_000, looping: false }, 0);
    let mut both = vec![0.0; 96_000 * CHANNELS];
    for block in both.chunks_mut(1024 * CHANNELS) {
        plain.mix(&[voice.clone(), music_alone.clone()], block);
    }
    // While the voice sounds, the mix is the voice plus a much quieter copy: well under double.
    let (with, without) = (loudness(&out, 72_000..90_000), loudness(&both, 72_000..90_000));
    assert!(with < without * 0.7, "ducked {with} vs {without}");
}

#[test]
fn the_noise_gate_leaves_a_loud_tone_alone() {
    let gated = AudioClip { noise_gate: 60.0, ..tone_clip(1, 0.0) };
    let mut mixer = Mixer::new(RATE, MixRange { start: 0, end: 96_000, looping: false }, 0);
    let mut out = vec![0.0; 96_000 * CHANNELS];
    mixer.mix(&[gated], &mut out);
    let mut plain = Mixer::new(RATE, MixRange { start: 0, end: 96_000, looping: false }, 0);
    let mut reference = vec![0.0; 96_000 * CHANNELS];
    plain.mix(&[tone_clip(1, 0.0)], &mut reference);
    let ratio = loudness(&out, 60_000..90_000) / loudness(&reference, 60_000..90_000);
    assert!(ratio > 0.97, "the tone passes: {ratio}");
}

#[test]
fn the_noise_gate_turns_quiet_hiss_down() {
    // Quiet noise (about -50 dB) alone: the gate should cut most of it.
    let mut samples: Vec<f32> = (0..48_000 * CHANNELS).map(|index| (((index * 7919) % 1000) as f32 / 1000.0 - 0.5) * 0.006).collect();
    let before = loudness(&samples, 10_000..40_000);
    studio_media::gate_for_tests(&mut samples, 60.0, RATE);
    let after = loudness(&samples, 10_000..40_000);
    assert!(after < before * 0.2, "hiss {before} → {after}");
}

/// `tone_clip` with the catalog audio effect `name`, its settings set to `values` (by index;
/// the rest at their defaults), and its keyframes read on the timeline's own clock at 30 fps.
fn with_effect(name: &str, values: &[(usize, f32)]) -> AudioClip {
    let mut effect = studio_core::effects::instantiate(studio_core::effects::find(name).unwrap(), Id(90));
    for &(index, value) in values {
        effect.properties[index].value = studio_core::animation::Animated::constant(value);
    }
    AudioClip { effects: vec![studio_core::audio::ClipEffect { effect, shift_seconds: 0.0, fps: 30.0 }], ..tone_clip(1, 0.0) }
}

/// The loudness of the tone (the second half of `tone.wav`) mixed through `clip`.
fn tone_loudness(clip: AudioClip) -> f32 {
    let mut mixer = Mixer::new(RATE, MixRange { start: 0, end: 96_000, looping: false }, 0);
    let mut out = vec![0.0; 96_000 * CHANNELS];
    // Small blocks, as playback mixes them, so effects carry their memory across blocks.
    for block in out.chunks_mut(1024 * CHANNELS) {
        mixer.mix(std::slice::from_ref(&clip), block);
    }
    loudness(&out, 60_000..90_000)
}

#[test]
fn audio_effects_change_the_sound_as_it_mixes() {
    let plain = tone_loudness(tone_clip(1, 0.0));
    let quieter = tone_loudness(with_effect("Gain", &[(0, -12.0)]));
    assert!((quieter / plain - 0.251).abs() < 0.01, "-12 dB is a quarter: {}", quieter / plain);
    // The tone is 440 Hz: a low cut at 1 kHz takes it away, a high cut at 2 kHz leaves it.
    assert!(tone_loudness(with_effect("EQ", &[(0, 1000.0)])) < plain * 0.3, "low cut");
    assert!((tone_loudness(with_effect("EQ", &[(5, 2000.0)])) / plain - 1.0).abs() < 0.03, "high cut above the tone");
    // A sine's peak is √2 times its loudness; the limiter keeps the peak at its -30 dB ceiling.
    let limited = tone_loudness(with_effect("Limiter", &[(0, -30.0)]));
    let ceiling = 10f32.powf(-30.0 / 20.0);
    assert!((limited * std::f32::consts::SQRT_2 - ceiling).abs() < ceiling * 0.1, "the peak sits at the ceiling: {limited}");
}

#[test]
fn keyframed_audio_effect_settings_change_while_it_plays() {
    // Gain falls from 0 dB at frame 30 (1 s, where the tone starts) to -40 dB at frame 60 (2 s).
    let mut clip = with_effect("Gain", &[]);
    clip.effects[0].effect.properties[0].value = studio_core::animation::Animated::keyframed(vec![
        studio_core::animation::Keyframe::linear(30, 0.0),
        studio_core::animation::Keyframe::linear(60, -40.0),
    ]);
    let mut mixer = Mixer::new(RATE, MixRange { start: 0, end: 96_000, looping: false }, 0);
    let mut out = vec![0.0; 96_000 * CHANNELS];
    for block in out.chunks_mut(1024 * CHANNELS) {
        mixer.mix(std::slice::from_ref(&clip), block);
    }
    let (early, late) = (loudness(&out, 50_000..56_000), loudness(&out, 86_000..92_000));
    assert!(late < early * 0.2, "it fades as the keyframes say: {early} then {late}");
}
