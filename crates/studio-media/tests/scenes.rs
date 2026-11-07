//! Scene-cut detection on real movie files: shots with known cuts, a pan and a fade are
//! encoded, then decoded and searched as Split at Scene Cuts does.

use std::path::{Path, PathBuf};

use studio_core::time::FrameRate;
use studio_media::{MovieFormat, MovieSettings, MovieWriter, find_scene_cuts};

const WIDTH: u32 = 192;
const HEIGHT: u32 = 108;

fn scratch(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("studio-scenes-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    folder.join(name)
}

/// A BGRA picture whose pixel at (x, y) has the RGB color `color(x, y)`.
fn picture(color: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    (0..HEIGHT).flat_map(|y| (0..WIDTH).map(move |x| (x, y))).flat_map(|(x, y)| {
        let [r, g, b] = color(x, y);
        [b, g, r, 255]
    }).collect()
}

/// A wide landscape seen from `offset` pixels along, for panning across.
fn landscape(offset: u32) -> Vec<u8> {
    picture(move |x, y| {
        let along = (x + offset) as f32;
        if y < HEIGHT / 2 { [(along * 0.3) as u8, (120.0 + 60.0 * (along / 40.0).sin()) as u8, 220] } else { [60, (90.0 + (along / 9.0).sin() * 40.0) as u8 + (y / 2) as u8, (along * 0.2) as u8] }
    })
}

/// Colored checks and stripes, slightly different for each `seed`.
fn pattern(seed: u32) -> Vec<u8> {
    picture(move |x, y| {
        let check = ((x / 12 + y / 12) % 2) as u8;
        [40 + check * 150, (seed * 70 % 200) as u8 + ((x / 5) % 4) as u8 * 12, 200 - check * 120]
    })
}

fn mix(a: &[u8], b: &[u8], amount: f32) -> Vec<u8> {
    a.iter().zip(b).map(|(&a, &b)| (f32::from(a) * (1.0 - amount) + f32::from(b) * amount).round() as u8).collect()
}

/// Writes the test movie: a pan (frames 0–29), a pattern (30–53), a flat red shot that fades
/// into another pattern (54–81), and that pattern (82–99). Cuts are at 30 and 54 only.
fn write_shots(path: &Path, format: MovieFormat) {
    let mut writer = MovieWriter::create(path, MovieSettings::new(format, WIDTH, HEIGHT, FrameRate::new(24, 1), None)).unwrap();
    let (red, second) = (picture(|_, _| [210, 40, 40]), pattern(3));
    let mut frames: Vec<Vec<u8>> = (0..30).map(|step| landscape(step * 4)).collect();
    frames.extend((0..24).map(|_| pattern(1)));
    frames.extend((0..16).map(|_| red.clone()));
    frames.extend((0..12).map(|step| mix(&red, &second, (step + 1) as f32 / 12.0)));
    frames.extend((0..18).map(|_| second.clone()));
    for frame in &frames {
        writer.write_video(frame, WIDTH, HEIGHT).unwrap();
    }
    writer.finish().unwrap();
}

#[test]
fn finds_the_cuts_in_an_encoded_movie() {
    for (format, name) in [(MovieFormat::H264, "shots.mp4"), (MovieFormat::ProRes422, "shots.mov")] {
        let path = scratch(name);
        write_shots(&path, format);
        let frames: Vec<i64> = (0..100).collect();
        let cuts = find_scene_cuts(&path, &frames, 6, &|_| {}, &|| false).unwrap();
        assert_eq!(cuts, vec![30, 54], "{format:?}");
        // Part of the movie, as a trimmed clip would ask for: positions are within that part.
        let part: Vec<i64> = (40..100).collect();
        assert_eq!(find_scene_cuts(&path, &part, 6, &|_| {}, &|| false).unwrap(), vec![14], "{format:?}");
        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn cancelling_finds_nothing_and_a_missing_file_fails() {
    let path = scratch("cancel.mp4");
    write_shots(&path, MovieFormat::H264);
    let frames: Vec<i64> = (0..100).collect();
    assert!(find_scene_cuts(&path, &frames, 6, &|_| {}, &|| true).unwrap().is_empty());
    assert!(find_scene_cuts(&scratch("missing.mp4"), &frames, 6, &|_| {}, &|| false).is_err());
    std::fs::remove_file(&path).ok();
}
