//! A small picture of each saved project, shown beside it on the start screen's Recent list.
//!
//! The picture is whatever the viewer was showing when the project was last saved, scaled down
//! and kept as a PNG in Studio's settings folder (not in the project file, which stays plain
//! text). Each save writes a new file whose name includes the time, because pictures on screen
//! are remembered by their path: a new name is what makes the new picture show.

use std::path::{Path, PathBuf};

/// The longest side of a thumbnail, in pixels: twice the size it is drawn at, for sharp
/// pictures on high-density displays.
const MAX_SIDE: u32 = 320;

/// Where thumbnails are kept.
fn folder() -> PathBuf {
    crate::app_settings::folder().join("Thumbnails")
}

/// The start of every thumbnail file's name for the project at `project`: a short fingerprint
/// of its path, so a project's pictures can be found (and replaced) without a list.
fn prefix(project: &Path) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(project.to_string_lossy().as_bytes()).iter().take(10).map(|byte| format!("{byte:02x}")).collect()
}

/// The thumbnail files of the project at `project` (there is one, or briefly two while a new
/// one replaces the old).
fn files_of(project: &Path) -> Vec<PathBuf> {
    let prefix = prefix(project);
    let Ok(entries) = std::fs::read_dir(folder()) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).filter(|path| path.file_name().is_some_and(|name| name.to_string_lossy().starts_with(&prefix))).collect();
    files.sort();
    files
}

/// The picture to show for the project at `project`, if one has been saved: the newest.
pub fn find(project: &Path) -> Option<PathBuf> {
    files_of(project).pop()
}

/// Saves a thumbnail for the project at `project` from a picture of `width` × `height` BGRA
/// pixels (the viewer's layout), replacing its earlier one. `now` is seconds since 1970.
pub fn write(project: &Path, bgra: &[u8], width: u32, height: u32, now: u64) -> Result<(), String> {
    if width == 0 || height == 0 || bgra.len() < (width as usize) * (height as usize) * 4 {
        return Err("there is no picture to save".into());
    }
    // PNG files are RGBA; the viewer's pictures are BGRA.
    let mut rgba = bgra[..(width as usize) * (height as usize) * 4].to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let picture = image::RgbaImage::from_raw(width, height, rgba).ok_or("the picture is smaller than its size says")?;
    let scale = (MAX_SIDE as f32 / width.max(height) as f32).min(1.0);
    let small = image::imageops::thumbnail(&picture, ((width as f32 * scale).round() as u32).max(1), ((height as f32 * scale).round() as u32).max(1));
    let folder = folder();
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    let earlier = files_of(project);
    // The time is padded so that names sort oldest to newest.
    let path = folder.join(format!("{}-{now:012}.png", prefix(project)));
    small.save_with_format(&path, image::ImageFormat::Png).map_err(|error| error.to_string())?;
    for old in earlier.into_iter().filter(|old| *old != path) {
        let _ = std::fs::remove_file(old);
    }
    Ok(())
}

/// Removes the thumbnail of the project at `project`, when it leaves the recent list.
pub fn forget(project: &Path) {
    for file in files_of(project) {
        let _ = std::fs::remove_file(file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thumbnail_is_scaled_down_found_replaced_and_forgotten() {
        let project = std::env::temp_dir().join(format!("thumbnail-test-{}.studio", std::process::id()));
        forget(&project);
        assert_eq!(find(&project), None);
        // A 640 × 360 picture, all one blue (BGRA).
        let picture = [200u8, 40, 20, 255].repeat(640 * 360);
        write(&project, &picture, 640, 360, 1_000).unwrap();
        let first = find(&project).expect("the thumbnail was saved");
        let saved = image::open(&first).unwrap().to_rgba8();
        assert_eq!((saved.width(), saved.height()), (320, 180), "scaled to fit, keeping its shape");
        assert_eq!(saved.get_pixel(10, 10).0, [20, 40, 200, 255], "blue stays blue");
        // A later save replaces it under a new name.
        write(&project, &picture, 640, 360, 2_000).unwrap();
        let second = find(&project).unwrap();
        assert_ne!(first, second);
        assert!(!first.exists());
        // Another project's picture is its own.
        assert_eq!(find(&project.with_extension("other")), None);
        forget(&project);
        assert_eq!(find(&project), None);
        // A picture with too few pixels is refused.
        assert!(write(&project, &[0, 0, 0], 4, 4, 3_000).is_err());
    }
}
