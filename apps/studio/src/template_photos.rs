//! The sample photos templates start out with: a few portraits, products, scenes and one
//! abstract picture, built into the app. A project refers to pictures by file, so the first
//! time they are needed they are written to "Template Photos" in Studio's settings folder,
//! and templates use those files.
//!
//! The pictures come from Unsplash (unsplash.com) under its free license.

use std::path::Path;
use std::sync::OnceLock;

use studio_core::animation::Vec2;
use studio_core::templates::{PhotoKind, SamplePhoto};

/// One built-in photo: what it shows, its file name, and the file itself.
struct BuiltIn {
    kind: PhotoKind,
    name: &'static str,
    bytes: &'static [u8],
}

macro_rules! built_in {
    ($kind:ident, $name:literal) => {
        BuiltIn { kind: PhotoKind::$kind, name: $name, bytes: include_bytes!(concat!("../../../assets/template-photos/", $name)) }
    };
}

const BUILT_IN: [BuiltIn; 11] = [
    built_in!(Portrait, "portrait-1.jpg"),
    built_in!(Portrait, "portrait-2.jpg"),
    built_in!(Portrait, "portrait-3.jpg"),
    built_in!(Portrait, "portrait-4.jpg"),
    built_in!(Product, "product-1.jpg"),
    built_in!(Product, "product-2.jpg"),
    built_in!(Scene, "scene-1.jpg"),
    built_in!(Scene, "scene-2.jpg"),
    built_in!(Scene, "scene-3.jpg"),
    built_in!(Scene, "scene-4.jpg"),
    built_in!(Art, "art-1.jpg"),
];

/// Writes one built-in photo into `folder` unless a file of the same length is already there
/// (so a photo changed by an update replaces the old one), and reads its size in pixels.
fn unpack(photo: &BuiltIn, folder: &Path) -> Result<SamplePhoto, String> {
    let path = folder.join(photo.name);
    let current = std::fs::metadata(&path).is_ok_and(|file| file.len() == photo.bytes.len() as u64);
    if !current {
        std::fs::write(&path, photo.bytes).map_err(|error| format!("could not write {}: {error}", path.display()))?;
    }
    let (width, height) = image::image_dimensions(&path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    Ok(SamplePhoto { kind: photo.kind, path, size: Vec2::new(width as f32, height as f32) })
}

/// The built-in photos as files, ready to fill templates' photo spots. They are unpacked once
/// per run. A photo that can't be written (a full or read-only disk) is left out and noted in
/// the log, and the spots it would have filled stay empty, as they do without any photos.
pub fn photos() -> &'static [SamplePhoto] {
    static PHOTOS: OnceLock<Vec<SamplePhoto>> = OnceLock::new();
    PHOTOS.get_or_init(|| {
        let folder = crate::app_settings::folder().join("Template Photos");
        if let Err(error) = std::fs::create_dir_all(&folder) {
            eprintln!("Template photos: could not create {}: {error}", folder.display());
            return Vec::new();
        }
        BUILT_IN
            .iter()
            .filter_map(|photo| unpack(photo, &folder).map_err(|error| eprintln!("Template photos: {error}")).ok())
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_photo_unpacks_with_its_size() {
        let folder = std::env::temp_dir().join(format!("studio-template-photos-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        for photo in &BUILT_IN {
            let unpacked = unpack(photo, &folder).unwrap();
            assert!(unpacked.size.x >= 800.0 && unpacked.size.y >= 800.0, "{} is {:?}", photo.name, unpacked.size);
            // A second time finds the file in place and leaves it alone.
            assert_eq!(unpack(photo, &folder).unwrap(), unpacked);
        }
        for kind in [PhotoKind::Portrait, PhotoKind::Product, PhotoKind::Scene, PhotoKind::Art] {
            assert!(BUILT_IN.iter().any(|photo| photo.kind == kind), "no {kind:?} photo");
        }
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
