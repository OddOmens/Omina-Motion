//! Plays audio through the default sound output and prints how much has been heard against
//! wall-clock time, to check the playback clock on real hardware. With no file it plays
//! silence (the clock still runs), so it can be run without making a sound.
//!
//! cargo run -p studio-media --release --example audio_clock -- [file or ""] [seconds]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use studio_core::audio::AudioClip;
use studio_core::model::Id;
use studio_media::{AudioOutput, AudioTimeline, probe_audio};

fn main() {
    let mut args = std::env::args().skip(1);
    let file = args.next().filter(|arg| !arg.is_empty()).map(PathBuf::from);
    let seconds: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let clips = match &file {
        Some(path) => {
            let info = probe_audio(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            vec![AudioClip { layer: Id(1), path: path.clone(), offset_seconds: 0.0, start_seconds: 0.0, end_seconds: info.duration_seconds, gain: 1.0 }]
        }
        None => Vec::new(),
    };

    let output = AudioOutput::new();
    let timeline = AudioTimeline { clips, start_seconds: 0.0, range_seconds: (0.0, 3600.0), looping: false };
    // Press play twice: the first includes opening the device, the second is a normal start.
    // Playback holds its first frame until sound actually comes out.
    let mut player = None;
    let mut started = Instant::now();
    for attempt in ["first play (device opening)", "next play"] {
        drop(player.take());
        let pressed = Instant::now();
        let current = output.play(timeline.clone());
        while current.heard().is_zero() && pressed.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_micros(200));
        }
        started = Instant::now();
        println!("{attempt}: first sound after {:.1} ms", (started - pressed).as_secs_f64() * 1000.0);
        player = Some(current);
    }
    let player = player.expect("played above");
    if let Some(error) = player.error() {
        println!("no sound: {error}");
    }

    let mut worst: f64 = 0.0;
    while started.elapsed().as_secs_f64() < seconds {
        std::thread::sleep(Duration::from_millis(250));
        let (heard, wall) = (player.heard().as_secs_f64(), started.elapsed().as_secs_f64());
        worst = worst.max((heard - wall).abs());
        println!("wall {wall:7.3} s   heard {heard:7.3} s   difference {:+6.1} ms", (heard - wall) * 1000.0);
    }
    println!("largest difference from the wall clock: {:.1} ms", worst * 1000.0);
}
