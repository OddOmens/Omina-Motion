//! Exports a real composition and checks the movie: `colors.mp4` (half a second each of red,
//! green, blue and white) fills the frame, and `tone.wav` (a second of silence, then a tone)
//! plays from the start.

use std::path::{Path, PathBuf};

use studio_core::animation::Vec2;
use studio_core::edit::{self, CompositionSettings, FootageInfo};
use studio_core::model::{Id, Project, Rgba};
use studio_core::time::FrameRate;
use studio_export::{ExportFormat, ExportRequest, Outcome, Progress, export};
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

fn request(comp: Id, path: PathBuf, format: impl Into<ExportFormat>) -> ExportRequest {
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
fn exports_h265_with_sound() {
    exports_picture_and_sound(MovieFormat::H265, "comp-hevc.mp4");
}

#[test]
fn a_slower_frame_rate_keeps_the_length_and_the_timing() {
    let (project, comp) = project();
    let path = scratch("twelve.mp4");
    let request = ExportRequest { frame_rate: Some(FrameRate::new(12, 1)), ..request(comp, path.clone(), MovieFormat::H264) };
    let mut last = None;
    export(&project, &request, &mut |p| last = Some(p), &|| false).unwrap();
    assert_eq!(last, Some(Progress { done: 24, total: 24 }), "two seconds at 12 fps");
    let info = probe(&path).unwrap();
    assert_eq!((info.frame_rate, info.frame_count), (FrameRate::new(12, 1), 24));
    // Each color shows for half a second: six frames at 12 fps.
    assert_eq!(["red", "green", "blue", "white"].map(String::from), [3, 9, 15, 21].map(|f| color_at(&path, f).to_owned()));
    let sound = probe_audio(&path).unwrap();
    assert!((sound.duration_seconds - 2.0).abs() < 0.05, "the sound still lasts {} s", sound.duration_seconds);
    assert!(loudness(&path, 1.2, 1.8) > 0.05, "and the tone is still in the second second");
}

/// The frames of an exported GIF: each one's delay (hundredths of a second) and the color at its center.
fn gif_frames(path: &Path) -> Vec<(u16, [u8; 3])> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(std::fs::File::open(path).unwrap()).unwrap();
    let (width, height) = (usize::from(decoder.width()), usize::from(decoder.height()));
    let mut frames = Vec::new();
    while let Some(frame) = decoder.read_next_frame().unwrap() {
        let i = (height / 2 * width + width / 2) * 4;
        frames.push((frame.delay, [frame.buffer[i], frame.buffer[i + 1], frame.buffer[i + 2]]));
    }
    frames
}

#[test]
fn exports_a_looping_gif_that_keeps_time() {
    let (project, comp) = project();
    let path = scratch("comp.gif");
    let request = ExportRequest { frame_rate: Some(FrameRate::new(15, 1)), ..request(comp, path.clone(), ExportFormat::Gif) };
    export(&project, &request, &mut |_| {}, &|| false).unwrap();
    let frames = gif_frames(&path);
    assert_eq!(frames.len(), 30, "two seconds at 15 fps");
    assert_eq!(frames.iter().map(|(delay, _)| u32::from(*delay)).sum::<u32>(), 200, "and it lasts two seconds");
    let [red, _, blue] = frames[2].1;
    assert!(red > 160 && blue < 90, "starts red: {:?}", frames[2].1);
    let [red, _, blue] = frames[17].1;
    assert!(blue > 160 && red < 90, "blue after a second: {:?}", frames[17].1);
}

#[test]
fn a_gif_never_runs_faster_than_browsers_play() {
    let (project, comp) = project();
    let request = ExportRequest { frame_rate: Some(FrameRate::new(60, 1)), ..request(comp, scratch("fast.gif"), ExportFormat::Gif) };
    assert_eq!(request.output_rate(FrameRate::new(24, 1)), studio_export::GIF_MAX_RATE);
    let path = request.path.clone();
    export(&project, &request, &mut |_| {}, &|| false).unwrap();
    assert!(gif_frames(&path).iter().all(|(delay, _)| *delay >= 2));
}

#[test]
fn exports_a_numbered_png_sequence_into_its_own_folder() {
    let (project, comp) = project();
    let folder = scratch("Sequence");
    let _ = std::fs::remove_dir_all(&folder);
    let request = ExportRequest { frames: 0..12, ..request(comp, folder.clone(), ExportFormat::PngSequence) };
    export(&project, &request, &mut |_| {}, &|| false).unwrap();
    let mut names: Vec<String> = std::fs::read_dir(&folder).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names.len(), 12);
    assert_eq!((names[0].as_str(), names[11].as_str()), ("Sequence_00000.png", "Sequence_00011.png"));
    let picture = image::open(folder.join(&names[0])).unwrap().to_rgba8();
    assert_eq!(picture.dimensions(), (64, 36));
    let center = picture.get_pixel(32, 18).0;
    assert!(center[0] > 160 && center[2] < 90 && center[3] == 255, "red: {center:?}");
    // A second export to the same folder is refused rather than mixed in with the first.
    let error = export(&project, &request, &mut |_| {}, &|| false).unwrap_err();
    assert!(error.contains("already exists"), "{error}");
}

#[test]
fn a_cancelled_png_sequence_leaves_no_folder() {
    let (project, comp) = project();
    let folder = scratch("Cancelled Sequence");
    let done = std::cell::Cell::new(0);
    let outcome = export(&project, &request(comp, folder.clone(), ExportFormat::PngSequence), &mut |p| done.set(p.done), &|| done.get() >= 3).unwrap();
    assert_eq!(outcome, Outcome::Cancelled);
    assert!(!folder.exists() && !studio_export::temporary_path(&folder).exists());
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
