//! Awkward real-world files, each made with ffmpeg to reproduce a bug found in review:
//!
//! - `bframes.ts`: MPEG-TS, 4 s at 24 fps (96 frames), a keyframe every 12 frames, B-frames,
//!   starting at 1.48 s. Seeking in TS lands on byte positions that are not keyframes.
//! - `longaudio.mkv`: 1 s of video (24 frames) with 2 s of audio, so the file's length (and the
//!   frame count worked out from it) is too long for the video.
//! - `damaged.mp4`: 4 s at 24 fps with corrupted bytes in the middle; players decode through it.
//! - `late_audio.mp4`: 1 s of video and a tone whose stream starts 0.44 s after the video's.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use studio_media::{AudioDecoder, CHANNELS, VideoDecoder};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn checksum(frame: &studio_core::media::VideoFrame) -> u64 {
    let mut hasher = DefaultHasher::new();
    frame.rgba.hash(&mut hasher);
    hasher.finish()
}

/// Every frame read in order from the start: the reference that seeks must match.
fn in_order(path: &Path, frames: i64) -> Vec<u64> {
    let mut decoder = VideoDecoder::open(path, 4096, false).unwrap();
    (0..frames).map(|index| checksum(&decoder.frame(index).unwrap())).collect()
}

#[test]
fn seeks_in_transport_streams_land_on_the_requested_frame() {
    let path = fixture("bframes.ts");
    let reference = in_order(&path, 96);
    let mut decoder = VideoDecoder::open(&path, 4096, false).unwrap();
    // Jumps in both directions, including into the last group of pictures.
    for index in [47, 0, 58, 95, 30, 83, 13, 90, 1] {
        let frame = decoder.frame(index).unwrap_or_else(|error| panic!("frame {index}: {error}"));
        assert_eq!(frame.index, index, "seeking to {index} returned frame {}", frame.index);
        assert_eq!(checksum(&frame), reference[index as usize], "frame {index} shows the wrong picture");
    }
}

#[test]
fn frames_past_the_real_end_show_the_last_picture() {
    let path = fixture("longaudio.mkv");
    let mut decoder = VideoDecoder::open(&path, 4096, false).unwrap();
    let last = (0..24).map(|index| decoder.frame(index).unwrap()).last().unwrap();
    // The file claims more frames than its video has; asking past the end is not an error.
    for index in [24, 30, decoder.frame_count() - 1] {
        let frame = decoder.frame(index).unwrap_or_else(|error| panic!("frame {index}: {error}"));
        assert_eq!(checksum(&frame), checksum(&last), "frame {index} should repeat the last picture");
    }
}

#[test]
fn damaged_packets_are_skipped_like_players_do() {
    let path = fixture("damaged.mp4");
    let mut decoder = VideoDecoder::open(&path, 4096, false).unwrap();
    let failures: Vec<i64> = (0..decoder.frame_count()).filter(|&index| decoder.frame(index).is_err()).collect();
    assert!(failures.is_empty(), "frames that failed to decode: {failures:?}");
}

#[test]
fn sound_lines_up_with_the_picture_when_its_stream_starts_later() {
    let mut decoder = AudioDecoder::open(&fixture("late_audio.mp4"), 48_000).unwrap();
    let mut all = vec![0.0; 48_000 * CHANNELS];
    let read = decoder.read(&mut all).unwrap();
    let loudness = |from: f64, to: f64| {
        let part = &all[(from * 48_000.0) as usize * CHANNELS..((to * 48_000.0) as usize).min(read) * CHANNELS];
        (part.iter().map(|s| s * s).sum::<f32>() / part.len().max(1) as f32).sqrt()
    };
    // The tone starts 0.44 s after the picture: silence before it, sound after.
    assert!(loudness(0.0, 0.4) < 0.001, "sound before its start: {}", loudness(0.0, 0.4));
    assert!(loudness(0.5, 0.9) > 0.05, "no sound after its start: {}", loudness(0.5, 0.9));
}


#[test]
fn compressed_sound_after_a_seek_matches_reading_straight_through() {
    // `tone.m4a`: 2 s of a 440 Hz tone as AAC, whose frames depend on the ones before them.
    let path = fixture("tone.m4a");
    let mut straight = AudioDecoder::open(&path, 48_000).unwrap();
    let mut all = vec![0.0; 96_000 * CHANNELS];
    let read = straight.read(&mut all).unwrap();
    // Seek to spots inside the file (as a loop or a clip start does) and compare what follows.
    for start in [24_000_usize, 48_000, 61_234] {
        let mut sought = AudioDecoder::open(&path, 48_000).unwrap();
        sought.seek(start as i64).unwrap();
        let mut block = vec![0.0; 2_000 * CHANNELS];
        let got = sought.read(&mut block).unwrap();
        assert!(start + got <= read);
        let worst = block[..got * CHANNELS]
            .iter()
            .zip(&all[start * CHANNELS..(start + got) * CHANNELS])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(worst < 0.01, "after seeking to {start}, the sound differs by up to {worst} (a click)");
    }
}
