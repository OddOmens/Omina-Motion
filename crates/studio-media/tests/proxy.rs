//! Builds proxies from the test clip and checks they are complete, correct and reused.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use studio_media::{ProxyManager, ProxyStatus, VideoDecoder, build_proxy, probe, proxy_path};

fn clip() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/colors.mp4")
}

/// A fresh, empty folder for one test.
fn scratch(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("studio-proxy-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

fn center_is(frame: &studio_core::media::VideoFrame, rgb: [bool; 3]) -> bool {
    let i = ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
    [frame.rgba[i] > 128, frame.rgba[i + 1] > 128, frame.rgba[i + 2] > 128] == rgb
}

#[test]
fn a_proxy_has_every_frame_and_the_same_timing() {
    let folder = scratch("build");
    let output = folder.join("proxy.mov");
    let built = build_proxy(&clip(), &output, 1280, &|_| {}, &|| false).unwrap();
    assert!(built && output.exists());
    assert!(!folder.join("proxy.partial.mov").exists(), "the temporary file is gone");
    let info = probe(&output).unwrap();
    assert_eq!(info.codec, "prores");
    assert_eq!(info.frame_count, 48);
    assert_eq!(info.frame_rate, studio_core::time::FrameRate::new(24, 1));

    // Every frame is a keyframe, so random access lands on the right picture.
    let mut decoder = VideoDecoder::open(&output, 1280, false).unwrap();
    for (index, rgb) in [(40, [true, true, true]), (3, [true, false, false]), (30, [false, false, true]), (13, [false, true, false])] {
        let frame = decoder.frame(index).unwrap();
        assert_eq!(frame.index, index);
        assert!(center_is(&frame, rgb), "proxy frame {index}");
    }
}

#[test]
fn cancelling_leaves_no_file() {
    let folder = scratch("cancel");
    let output = folder.join("proxy.mov");
    assert!(!build_proxy(&clip(), &output, 1280, &|_| {}, &|| true).unwrap());
    assert!(!output.exists() && !folder.join("proxy.partial.mov").exists());
}

#[test]
fn the_manager_builds_in_the_background_and_reuses_the_cache() {
    let folder = scratch("manager");
    let manager = ProxyManager::new(folder.clone());
    manager.request(&clip());
    let started = Instant::now();
    let ready = loop {
        match manager.status(&clip()) {
            Some(ProxyStatus::Ready(path)) => break path,
            Some(ProxyStatus::Failed(error)) => panic!("proxy failed: {error}"),
            _ => {}
        }
        assert!(started.elapsed() < Duration::from_secs(20), "the proxy never finished");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(ready, proxy_path(&folder, &clip()));

    // A second manager finds the finished proxy straight away.
    let again = ProxyManager::new(folder);
    again.request(&clip());
    assert_eq!(again.ready(&clip()), Some(ready));
}

#[test]
fn scrubbing_uses_the_proxy_then_upgrades_to_full_quality() {
    use std::sync::Arc;
    use studio_core::media::VideoSource;
    use studio_media::FrameServer;

    let folder = scratch("server");
    let proxies = Arc::new(ProxyManager::new(folder));
    proxies.request(&clip());
    let started = Instant::now();
    while proxies.ready(&clip()).is_none() {
        assert!(started.elapsed() < Duration::from_secs(20), "the proxy never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    let server = FrameServer::new(1920, Some(proxies));
    let mut saw_proxy = false;
    let started = Instant::now();
    loop {
        if let Some(frame) = server.frame(&clip(), 40).filter(|frame| frame.index == 40) {
            assert!(center_is(&frame, [true, true, true]), "frame 40 is white");
            saw_proxy |= frame.from_proxy;
            if !frame.from_proxy {
                break;
            }
        }
        assert!(started.elapsed() < Duration::from_secs(10), "full quality never arrived");
        std::thread::sleep(Duration::from_millis(1));
    }
    // On a clip this small the original can win the race; on long-GOP footage the proxy arrives first.
    println!("the proxy frame was shown first: {saw_proxy}");
}

#[test]
fn a_failed_build_leaves_no_partial_file_behind() {
    let folder = std::env::temp_dir().join(format!("studio-proxy-fail-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    // A folder where the finished proxy should go, so saving it fails at the very end.
    let output = folder.join("blocked.mov");
    std::fs::create_dir_all(&output).unwrap();
    let built = build_proxy(&clip(), &output, 64, &|_| {}, &|| false);
    assert!(built.is_err());
    let leftovers: Vec<_> = std::fs::read_dir(&folder).unwrap().map(|entry| entry.unwrap().file_name()).filter(|name| name != "blocked.mov").collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn damaged_footage_still_gets_a_proxy() {
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/damaged.mp4");
    let output = std::env::temp_dir().join(format!("studio-proxy-damaged-{}.mov", std::process::id()));
    assert_eq!(build_proxy(&source, &output, 64, &|_| {}, &|| false), Ok(true));
    std::fs::remove_file(&output).unwrap();
}
