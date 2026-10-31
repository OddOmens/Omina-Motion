//! Writes real movie files and reads them back with the decoder.

use std::path::{Path, PathBuf};

use studio_core::time::FrameRate;
use studio_media::{AudioDecoder, CHANNELS, MovieFormat, MovieSettings, MovieWriter, VideoDecoder, probe, probe_audio};

fn scratch(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("studio-encode-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    folder.join(name)
}

/// A solid BGRA image.
fn solid(width: u32, height: u32, bgra: [u8; 4]) -> Vec<u8> {
    bgra.iter().copied().cycle().take((width * height * 4) as usize).collect()
}

/// Writes one second at 24 fps: red for half a second, then blue, with a 440 Hz tone.
fn write_movie(path: &Path, format: MovieFormat, width: u32, height: u32) {
    let settings = MovieSettings { format, width, height, frame_rate: FrameRate::new(24, 1), audio_rate: Some(48_000) };
    let mut writer = MovieWriter::create(path, settings).unwrap();
    let (red, blue) = (solid(width, height, [0, 0, 255, 255]), solid(width, height, [255, 0, 0, 255]));
    let tone: Vec<f32> = (0..48_000).flat_map(|i| {
        let sample = (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5;
        [sample, sample]
    }).collect();
    for frame in 0..24 {
        writer.write_video(if frame < 12 { &red } else { &blue }, width, height).unwrap();
        // Sound in blocks that do not line up with the codec's frame size.
        writer.write_audio(&tone[frame * 2000 * CHANNELS..(frame + 1) * 2000 * CHANNELS]).unwrap();
    }
    writer.finish().unwrap();
}

/// Which channel dominates at the center of decoded frame `index`.
fn color_at(path: &Path, index: i64) -> &'static str {
    let frame = VideoDecoder::open(path, 4096, false).unwrap().frame(index).unwrap();
    let i = ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
    match (frame.rgba[i] > 160, frame.rgba[i + 2] > 160) {
        (true, false) => "red",
        (false, true) => "blue",
        _ => "other",
    }
}

fn check_movie(path: &Path, codec: &str) {
    let info = probe(path).unwrap();
    assert_eq!(info.codec, codec);
    assert_eq!((info.width, info.height, info.frame_count), (64, 36, 24));
    assert_eq!(info.frame_rate, FrameRate::new(24, 1));
    assert!(info.has_audio);
    assert_eq!(color_at(path, 3), "red");
    assert_eq!(color_at(path, 20), "blue");

    let audio = probe_audio(path).unwrap();
    assert_eq!((audio.sample_rate, audio.channels), (48_000, 2));
    assert!((audio.duration_seconds - 1.0).abs() < 0.05, "{} s of sound", audio.duration_seconds);
    let mut decoder = AudioDecoder::open(path, 48_000).unwrap();
    decoder.seek(12_000).unwrap();
    let mut block = vec![0.0; 4800 * CHANNELS];
    decoder.read(&mut block).unwrap();
    let loudness = (block.iter().map(|s| s * s).sum::<f32>() / block.len() as f32).sqrt();
    assert!((loudness - 0.35).abs() < 0.05, "the tone survives encoding (rms {loudness})");
}

#[test]
fn writes_h264_with_aac_sound() {
    let path = scratch("out.mp4");
    write_movie(&path, MovieFormat::H264, 64, 36);
    check_movie(&path, "h264");
}

#[test]
fn writes_prores_with_pcm_sound() {
    let path = scratch("out.mov");
    write_movie(&path, MovieFormat::ProRes422Hq, 64, 36);
    check_movie(&path, "prores");
}

#[test]
fn odd_sizes_are_rounded_up_to_even() {
    let path = scratch("odd.mp4");
    write_movie(&path, MovieFormat::H264, 63, 35);
    let info = probe(&path).unwrap();
    assert_eq!((info.width, info.height), (64, 36));
}
