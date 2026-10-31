//! Simulates scrubbing: jumps to random frames and measures how long until each one is on screen,
//! without a proxy and with one. `cargo run -p studio-media --release --example scrub_speed -- clip.mp4`

use std::sync::Arc;
use std::time::{Duration, Instant};

use studio_core::media::VideoSource;
use studio_media::{FrameServer, ProxyManager, probe};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::PathBuf::from(std::env::args().nth(1).ok_or("usage: scrub_speed <clip>")?);
    let frames = probe(&path)?.frame_count;
    let cache = std::env::temp_dir().join("studio-scrub-speed");
    let proxies = Arc::new(ProxyManager::new(cache));
    proxies.request(&path);
    while proxies.ready(&path).is_none() {
        std::thread::sleep(Duration::from_millis(20));
    }
    for (label, server) in [("original only", FrameServer::new(1920, None)), ("with proxy", FrameServer::new(1920, Some(proxies.clone())))] {
        // Let the decoders open and pick a backend before timing.
        while server.frame(&path, 0).is_none_or(|f| f.index != 0) {
            std::thread::sleep(Duration::from_millis(1));
        }
        let jumps = 15;
        let mut total = Duration::ZERO;
        for step in 1..=jumps {
            let target = (step * 7919) % frames;
            let started = Instant::now();
            while server.frame(&path, target).is_none_or(|f| f.index != target) {
                std::thread::sleep(Duration::from_micros(200));
            }
            total += started.elapsed();
        }
        println!("{label}: {:.1} ms until the frame is on screen, per jump", total.as_secs_f64() * 1000.0 / jumps as f64);
    }
    Ok(())
}
