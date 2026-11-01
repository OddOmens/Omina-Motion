//! Filmstrip thumbnails and waveforms from real files: `colors.mp4` (64x36, 2 s at 24 fps: half
//! a second each of red, green, blue, white, with sound), `tone.wav` (one second of silence, then
//! one second of a 440 Hz tone, no picture), `bframes.ts` and `late_audio.mp4` (see `robust.rs`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use studio_core::media::VideoFrame;
use studio_media::{PeakCache, Peaks, Thumbnailer, audio_peaks};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

/// The color at the center of a frame, roughly: which channel dominates, or white.
fn color_of(frame: &VideoFrame) -> &'static str {
    let i = ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
    match (frame.rgba[i] > 128, frame.rgba[i + 1] > 128, frame.rgba[i + 2] > 128) {
        (true, true, true) => "white",
        (true, false, false) => "red",
        (false, true, false) => "green",
        (false, false, true) => "blue",
        _ => "other",
    }
}

/// Asks for the thumbnail at `seconds` until the exact one (not a stand-in) has arrived.
fn wait_for_thumbnail(thumbnailer: &Thumbnailer, path: &Path, seconds: f64, color: &str) -> Arc<VideoFrame> {
    let started = Instant::now();
    loop {
        if let Some(frame) = thumbnailer.thumbnail(path, seconds).filter(|frame| color_of(frame) == color) {
            return frame;
        }
        assert!(started.elapsed() < Duration::from_secs(5), "no {color} thumbnail at {seconds} s");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn thumbnails_show_the_picture_at_the_requested_time_and_height() {
    let thumbnailer = Thumbnailer::new(18);
    let readies = Arc::new(AtomicUsize::new(0));
    let counter = readies.clone();
    thumbnailer.set_on_ready(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let clip = fixture("colors.mp4");
    let red = wait_for_thumbnail(&thumbnailer, &clip, 0.25, "red");
    assert_eq!((red.width, red.height), (32, 18), "keeps the 16:9 shape");
    assert!(readies.load(Ordering::SeqCst) >= 1, "the timeline was told to redraw");

    // While the blue one is decoding, the nearest ready thumbnail stands in for it.
    let stand_in = thumbnailer.thumbnail(&clip, 1.25).expect("a stand-in");
    assert_eq!(color_of(&stand_in), "red");
    let blue = wait_for_thumbnail(&thumbnailer, &clip, 1.25, "blue");
    assert_eq!(blue.height, 18);

    // Nearby times share the decoded quarter-second thumbnail.
    let same = thumbnailer.thumbnail(&clip, 1.3).unwrap();
    assert!(Arc::ptr_eq(&same, &blue));
    wait_for_thumbnail(&thumbnailer, &clip, 0.75, "green");
    wait_for_thumbnail(&thumbnailer, &clip, 1.9, "white");
}

#[test]
fn thumbnails_of_odd_sizes_are_exactly_the_requested_height() {
    // 36 tall to 25: the decoder's even-size rounding alone would give 24.
    let thumbnailer = Thumbnailer::new(25);
    let red = wait_for_thumbnail(&thumbnailer, &fixture("colors.mp4"), 0.0, "red");
    assert_eq!((red.width, red.height), (44, 25));
    // Transport streams that start late are counted from their first picture.
    let thumbnailer = Thumbnailer::new(18);
    let path = fixture("bframes.ts");
    let started = Instant::now();
    let frame = loop {
        if let Some(frame) = thumbnailer.thumbnail(&path, 2.0) {
            break frame;
        }
        assert!(started.elapsed() < Duration::from_secs(5), "no thumbnail from bframes.ts");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!((frame.height, frame.index), (18, 48));
}

#[test]
fn files_without_a_picture_give_no_thumbnail_and_are_not_retried() {
    let thumbnailer = Thumbnailer::new(18);
    let readies = Arc::new(AtomicUsize::new(0));
    let counter = readies.clone();
    thumbnailer.set_on_ready(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    for path in [fixture("tone.wav"), PathBuf::from("/no/such/clip.mp4")] {
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(300) {
            assert!(thumbnailer.thumbnail(&path, 0.5).is_none());
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    assert_eq!(readies.load(Ordering::SeqCst), 0);
    // The worker is still free for other files.
    wait_for_thumbnail(&thumbnailer, &fixture("colors.mp4"), 0.0, "red");
}

/// The loudest bucket of `peaks` between `from` and `to` seconds.
fn loudest(peaks: &Peaks, from: f64, to: f64) -> f32 {
    let bucket = |seconds: f64| (seconds * f64::from(peaks.per_second)) as usize;
    peaks.values[bucket(from)..bucket(to).min(peaks.values.len())].iter().copied().fold(0.0, f32::max)
}

#[test]
fn waveforms_follow_the_sound() {
    let peaks = audio_peaks(&fixture("tone.wav"), 100).unwrap();
    assert_eq!(peaks.per_second, 100);
    assert!((peaks.values.len() as i64 - 200).abs() <= 2, "{} buckets", peaks.values.len());
    assert!(loudest(&peaks, 0.0, 0.95) < 0.01, "sound in the silence: {}", loudest(&peaks, 0.0, 0.95));
    // The tone is quiet (about 0.09 at its peak once spread over two channels), but steady.
    assert!(peaks.values[105..195].iter().all(|value| *value > 0.05), "silent buckets in the tone: {:?}", &peaks.values[100..]);
    assert!(peaks.values.iter().all(|value| (0.0..=1.0).contains(value)));

    // A sound track starting after the picture is placed by the picture's start.
    let late = audio_peaks(&fixture("late_audio.mp4"), 50).unwrap();
    assert!(loudest(&late, 0.0, 0.4) < 0.01, "sound before its start: {}", loudest(&late, 0.0, 0.4));
    assert!(loudest(&late, 0.5, 0.9) > 0.05, "no sound after its start");
}

#[test]
fn the_peak_cache_computes_in_the_background_and_remembers_failures() {
    let cache = PeakCache::new();
    let readies = Arc::new(AtomicUsize::new(0));
    let counter = readies.clone();
    cache.set_on_ready(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let tone = fixture("tone.wav");
    let started = Instant::now();
    let peaks = loop {
        if let Some(peaks) = cache.peaks(&tone) {
            break peaks;
        }
        assert!(started.elapsed() < Duration::from_secs(5), "no waveform arrived");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(*peaks, audio_peaks(&tone, 100).unwrap());
    assert!(Arc::ptr_eq(&peaks, &cache.peaks(&tone).unwrap()), "computed once");

    // A missing file fails once and stays failed, without being computed again.
    let missing = PathBuf::from("/no/such/sound.wav");
    assert!(cache.peaks(&missing).is_none());
    let started = Instant::now();
    while readies.load(Ordering::SeqCst) < 2 {
        assert!(started.elapsed() < Duration::from_secs(5), "the failure was never recorded");
        std::thread::sleep(Duration::from_millis(5));
    }
    for _ in 0..20 {
        assert!(cache.peaks(&missing).is_none());
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(readies.load(Ordering::SeqCst), 2, "the failed file was not tried again");
}

#[test]
fn still_images_get_a_thumbnail() {
    // A green PNG and JPEG, as Image layers use: FFmpeg reads them as one-frame videos.
    for name in ["still.png", "still.jpg"] {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
        let thumbnailer = studio_media::Thumbnailer::new(18);
        let started = std::time::Instant::now();
        let picture = loop {
            if let Some(picture) = thumbnailer.thumbnail(&path, 0.0) {
                break picture;
            }
            assert!(started.elapsed() < std::time::Duration::from_secs(5), "{name}: no thumbnail");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(picture.height, 18, "{name}");
        let i = ((picture.height / 2 * picture.width + picture.width / 2) * 4) as usize;
        assert!(picture.rgba[i + 1] > 200 && picture.rgba[i] < 60, "{name}: green");
    }
}
