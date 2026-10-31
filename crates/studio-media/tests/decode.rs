//! Decodes a real clip (2 s at 24 fps: half a second each of red, green, blue, white).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use studio_core::media::VideoSource;
use studio_core::time::FrameRate;
use studio_media::{FrameServer, VideoDecoder, probe};

fn clip() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/colors.mp4")
}

/// The color at the center of a frame, roughly: which channel dominates, or white.
fn color_of(frame: &studio_core::media::VideoFrame) -> &'static str {
    let i = ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
    match (frame.rgba[i] > 128, frame.rgba[i + 1] > 128, frame.rgba[i + 2] > 128) {
        (true, true, true) => "white",
        (true, false, false) => "red",
        (false, true, false) => "green",
        (false, false, true) => "blue",
        _ => "other",
    }
}

#[test]
fn probing_reads_size_length_rate_and_audio() {
    let info = probe(&clip()).unwrap();
    assert_eq!((info.width, info.height), (64, 36));
    assert_eq!(info.frame_rate, FrameRate::new(24, 1));
    assert_eq!(info.frame_count, 48);
    assert!((info.duration_seconds - 2.0).abs() < 0.05, "{}", info.duration_seconds);
    assert!(info.has_audio);
    assert_eq!(info.codec, "h264");
}

#[test]
fn frames_decode_in_order_and_after_jumps() {
    let mut decoder = VideoDecoder::open(&clip(), 1920, false).unwrap();
    // In order.
    for (index, color) in [(0, "red"), (1, "red"), (13, "green"), (30, "blue")] {
        let frame = decoder.frame(index).unwrap();
        assert_eq!((frame.index, color_of(&frame)), (index, color), "frame {index}");
    }
    // Backwards and across the keyframe at frame 24: these seek.
    for (index, color) in [(5, "red"), (40, "white"), (12, "green"), (47, "white"), (0, "red")] {
        let frame = decoder.frame(index).unwrap();
        assert_eq!((frame.index, color_of(&frame)), (index, color), "frame {index}");
    }
    // Past the end shows the last frame.
    assert_eq!(decoder.frame(500).unwrap().index, 47);
}

#[test]
fn frames_scale_down_to_the_requested_width() {
    let mut decoder = VideoDecoder::open(&clip(), 32, false).unwrap();
    let frame = decoder.frame(0).unwrap();
    assert_eq!((frame.width, frame.height), (32, 18));
    assert_eq!(frame.rgba.len(), 32 * 18 * 4);
}

#[test]
fn missing_files_report_an_error() {
    assert!(probe(std::path::Path::new("/nonexistent.mov")).is_err());
    let server = FrameServer::new(1920, None);
    let missing = PathBuf::from("/nonexistent.mov");
    assert!(server.frame(&missing, 0).is_none());
    let started = Instant::now();
    while server.error(&missing).is_none() && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(server.error(&missing).is_some());
}

#[test]
fn the_server_decodes_in_the_background_and_reports_readiness() {
    let server = FrameServer::new(1920, None);
    let ready = Arc::new(AtomicUsize::new(0));
    let counter = ready.clone();
    server.set_on_ready(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let path = clip();
    // The first request starts decoding and returns nothing yet.
    assert!(server.frame(&path, 30).is_none());
    let started = Instant::now();
    let frame = loop {
        if let Some(frame) = server.frame(&path, 30).filter(|frame| frame.index == 30) {
            break frame;
        }
        assert!(started.elapsed() < Duration::from_secs(5), "frame 30 never arrived");
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(color_of(&frame), "blue");
    assert!(ready.load(Ordering::SeqCst) >= 1);

    // While a new frame is decoding, the nearest one is shown instead of nothing.
    let nearest = server.frame(&path, 31).expect("a nearby frame is shown");
    assert!((30..=31).contains(&nearest.index));
}

#[test]
fn playing_decodes_ahead() {
    let server = FrameServer::new(1920, None);
    let path = clip();
    server.set_playing(true);
    server.frame(&path, 0);
    // Give the worker time to fill its look-ahead, then step forward without waiting.
    let started = Instant::now();
    while server.frame(&path, 10).is_none_or(|frame| frame.index != 10) && started.elapsed() < Duration::from_secs(5) {
        // Keep asking for frame 0 so frame 10 can only come from decoding ahead.
        server.frame(&path, 0);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(server.frame(&path, 10).map(|frame| frame.index), Some(10), "frame 10 was decoded ahead of the playhead");
}

#[test]
fn hardware_decoding_matches_software() {
    let mut software = VideoDecoder::open(&clip(), 1920, false).unwrap();
    let mut hardware = VideoDecoder::open(&clip(), 1920, true).unwrap();
    assert_eq!(software.backend(), "software");
    println!("hardware decoder in use: {}", hardware.backend());
    for (index, color) in [(3, "red"), (20, "green"), (30, "blue"), (47, "white"), (1, "red")] {
        let (a, b) = (software.frame(index).unwrap(), hardware.frame(index).unwrap());
        assert_eq!((a.index, color_of(&a)), (index, color));
        assert_eq!((b.index, color_of(&b)), (index, color), "{} decoder, frame {index}", hardware.backend());
    }
}

#[test]
fn a_cancelled_decode_gives_up() {
    let mut decoder = VideoDecoder::open(&clip(), 1920, false).unwrap();
    // Frame 20 sits 20 frames after the keyframe at 0, so this decodes and checks along the way.
    decoder.frame(0).unwrap();
    assert!(decoder.frame_unless(20, &|| true).unwrap().is_none());
    // The decoder still works afterwards.
    assert_eq!(decoder.frame(20).unwrap().index, 20);
}

#[test]
fn the_fastest_decoder_still_decodes_correctly() {
    let mut decoder = VideoDecoder::open_fastest(&clip(), 1920).unwrap();
    println!("fastest decoder for the test clip: {}", decoder.backend());
    for (index, color) in [(0, "red"), (30, "blue"), (12, "green")] {
        assert_eq!(color_of(&decoder.frame(index).unwrap()), color, "{} decoder, frame {index}", decoder.backend());
    }
}
