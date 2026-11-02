//! Exports a real composition and checks the movie: `colors.mp4` (half a second each of red,
//! green, blue and white) fills the frame, and `tone.wav` (a second of silence, then a tone)
//! plays from the start.

use std::path::{Path, PathBuf};

use studio_core::animation::Vec2;
use studio_core::edit::{self, CompositionSettings, FootageInfo};
use studio_core::model::{Id, Project, Rgba};
use studio_core::time::FrameRate;
use studio_export::{ExportRequest, Outcome, Progress, export};
use studio_media::{AudioDecoder, CHANNELS, MovieFormat, VideoDecoder, probe, probe_audio};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../studio-media/tests/fixtures").join(name)
}

fn scratch(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("studio-export-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    folder.join(name)
}

/// A 64 × 36, 24 fps, two-second composition with the color clip and the tone in it.
fn project() -> (Project, Id) {
    let mut project = Project::new(Vec::new(), Vec::new());
    let settings = CompositionSettings {
        name: "Export".into(),
        width: 64,
        height: 36,
        frame_rate: FrameRate::new(24, 1),
        duration_frames: 48,
        background: Rgba::hex(0x000000),
    };
    let comp = edit::new_composition(&mut project, &settings).unwrap();
    let clip = probe(&fixture("colors.mp4")).unwrap();
    let info = FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: clip.duration_seconds, frame_rate: clip.frame_rate, has_audio: false };
    let footage = edit::import_footage(&mut project, fixture("colors.mp4"), &info);
    edit::add_layer_from_item(&mut project, comp, footage, 0, 0).unwrap();
    let tone = edit::import_audio(&mut project, fixture("tone.wav"), 2.0);
    edit::add_layer_from_item(&mut project, comp, tone, 1, 0).unwrap();
    (project, comp)
}

fn request(comp: Id, path: PathBuf, format: MovieFormat) -> ExportRequest {
    ExportRequest::new(comp, 0..48, format, path)
}

fn color_at(path: &Path, index: i64) -> &'static str {
    let frame = VideoDecoder::open(path, 4096, false).unwrap().frame(index).unwrap();
    let i = ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
    match (frame.rgba[i] > 160, frame.rgba[i + 1] > 160, frame.rgba[i + 2] > 160) {
        (true, false, false) => "red",
        (false, true, false) => "green",
        (false, false, true) => "blue",
        (true, true, true) => "white",
        _ => "other",
    }
}

/// Loudness of the exported sound between `from` and `to` seconds.
fn loudness(path: &Path, from: f64, to: f64) -> f32 {
    let mut decoder = AudioDecoder::open(path, 48_000).unwrap();
    decoder.seek((from * 48_000.0) as i64).unwrap();
    let mut block = vec![0.0; ((to - from) * 48_000.0) as usize * CHANNELS];
    let read = decoder.read(&mut block).unwrap();
    let part = &block[..read * CHANNELS];
    (part.iter().map(|s| s * s).sum::<f32>() / part.len().max(1) as f32).sqrt()
}

fn exports_picture_and_sound(format: MovieFormat, name: &str) {
    let (project, comp) = project();
    let path = scratch(name);
    let mut reports = Vec::new();
    let outcome = export(&project, &request(comp, path.clone(), format), &mut |p| reports.push(p), &|| false).unwrap();
    assert_eq!(outcome, Outcome::Finished);
    assert_eq!(reports.last(), Some(&Progress { done: 48, total: 48 }));

    let info = probe(&path).unwrap();
    assert_eq!((info.width, info.height, info.frame_count), (64, 36, 48));
    assert_eq!(["red", "green", "blue", "white"].map(String::from), [6, 18, 30, 42].map(|f| color_at(&path, f).to_owned()));
    let sound = probe_audio(&path).unwrap();
    assert!((sound.duration_seconds - 2.0).abs() < 0.05, "{} s of sound", sound.duration_seconds);
    assert!(loudness(&path, 0.2, 0.8) < 0.01, "the first second is silent");
    assert!(loudness(&path, 1.2, 1.8) > 0.05, "the tone plays in the second second");
}

#[test]
fn exports_h264_with_sound() {
    exports_picture_and_sound(MovieFormat::H264, "comp.mp4");
}

#[test]
fn exports_prores_with_sound() {
    exports_picture_and_sound(MovieFormat::ProRes422Hq, "comp.mov");
}

#[test]
fn exports_just_the_work_area() {
    let (project, comp) = project();
    let path = scratch("work-area.mp4");
    let request = ExportRequest { frames: 24..48, ..request(comp, path.clone(), MovieFormat::H264) };
    export(&project, &request, &mut |_| {}, &|| false).unwrap();
    assert_eq!(probe(&path).unwrap().frame_count, 24);
    assert_eq!(color_at(&path, 0), "blue", "starts at the work area");
}

#[test]
fn a_half_size_export_has_half_the_pixels() {
    let (project, comp) = project();
    let path = scratch("half.mp4");
    let full = probe(&{
        let full = scratch("full-size.mp4");
        export(&project, &request(comp, full.clone(), MovieFormat::H264), &mut |_| {}, &|| false).unwrap();
        full
    })
    .unwrap();
    let request = ExportRequest { scale: 0.5, ..request(comp, path.clone(), MovieFormat::H264) };
    export(&project, &request, &mut |_| {}, &|| false).unwrap();
    let half = probe(&path).unwrap();
    assert_eq!((half.width, half.height), (request.output_size(full.width, full.height)));
    assert!(half.width < full.width);
    assert_eq!(color_at(&path, 0), "red", "the picture is the same, smaller");
}

#[test]
fn without_sound_there_is_no_sound_track() {
    let (project, comp) = project();
    let path = scratch("silent.mp4");
    let request = ExportRequest { include_audio: false, ..request(comp, path.clone(), MovieFormat::H264) };
    export(&project, &request, &mut |_| {}, &|| false).unwrap();
    assert!(!probe(&path).unwrap().has_audio);
}

#[test]
fn cancelling_leaves_no_file_behind() {
    let (project, comp) = project();
    let path = scratch("cancelled.mp4");
    let done = std::cell::Cell::new(0);
    let outcome = export(&project, &request(comp, path.clone(), MovieFormat::H264), &mut |p| done.set(p.done), &|| done.get() >= 5).unwrap();
    assert_eq!(outcome, Outcome::Cancelled);
    assert!(!path.exists());
    let folder = path.parent().unwrap();
    assert!(!std::fs::read_dir(folder).unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().contains("cancelled.mp4.exporting")));
}

#[test]
fn impossible_requests_are_refused_with_a_reason() {
    let (project, comp) = project();
    let empty = ExportRequest { frames: 30..30, ..request(comp, scratch("none.mp4"), MovieFormat::H264) };
    assert_eq!(export(&project, &empty, &mut |_| {}, &|| false).unwrap_err(), "There are no frames to export.");
}

#[test]
fn footage_that_cannot_be_read_stops_the_export_with_a_reason() {
    let (mut project, comp) = project();
    // Point the clip at a file that is gone: exporting must fail, not repeat a stale picture.
    let footage = project.items.iter_mut().find(|item| item.name == "colors.mp4").unwrap();
    footage.path = Some(scratch("missing-source.mp4"));
    let path = scratch("from-missing.mp4");
    let error = export(&project, &request(comp, path.clone(), MovieFormat::H264), &mut |_| {}, &|| false).unwrap_err();
    assert!(error.contains("could not be decoded"), "{error}");
    assert!(!path.exists());
}

/// A video taller than GPUs accept (a 64 × 9000 strip) exports instead of crashing the app.
#[test]
fn a_video_taller_than_the_gpu_limit_exports() {
    let clip = scratch("tall.mp4");
    let settings = studio_media::MovieSettings::new(MovieFormat::H264, 64, 9000, FrameRate::new(24, 1), None);
    let mut writer = studio_media::MovieWriter::create(&clip, settings).unwrap();
    let picture: Vec<u8> = [0u8, 0, 255, 255].iter().copied().cycle().take(64 * 9000 * 4).collect();
    for _ in 0..4 {
        writer.write_video(&picture, 64, 9000).unwrap();
    }
    writer.finish().unwrap();
    let mut project = Project::new(Vec::new(), Vec::new());
    let settings = CompositionSettings { name: "Tall".into(), width: 64, height: 36, frame_rate: FrameRate::new(24, 1), duration_frames: 4, background: Rgba::hex(0x000000) };
    let comp = edit::new_composition(&mut project, &settings).unwrap();
    let info = FootageInfo { size: Vec2::new(64.0, 9000.0), duration_seconds: 4.0 / 24.0, frame_rate: FrameRate::new(24, 1), has_audio: false };
    let footage = edit::import_footage(&mut project, clip, &info);
    edit::add_layer_from_item(&mut project, comp, footage, 0, 0).unwrap();
    let path = scratch("tall-out.mp4");
    export(&project, &ExportRequest::new(comp, 0..4, MovieFormat::H264, path.clone()), &mut |_| {}, &|| false).unwrap();
    assert!(path.exists());
}
