//! Drawing an SVG file as a picture, for SVGs used as one layer (rather than broken into shape
//! layers). Everything SVG can hold comes through, embedded photos and text included, because
//! the whole file is drawn by the `resvg` library the way a browser would draw it.

use std::path::Path;

/// The longest side an SVG is drawn at, in pixels. Drawings are made larger than their stated
/// size (up to `MAX_ENLARGEMENT` times) so they stay sharp when the layer is scaled up.
const TARGET_SIDE: f32 = 3840.0;
const MAX_ENLARGEMENT: f32 = 8.0;

/// An SVG file drawn to pixels: straight (not premultiplied) RGBA, row by row.
pub struct SvgImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Draws the SVG file at `path`, no larger than `max_side` pixels a side. Returns `None` when
/// the file can't be read or isn't a drawable SVG.
pub fn rasterize(path: &Path, max_side: u32) -> Option<SvgImage> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut options = resvg::usvg::Options { resources_dir: path.parent().map(Path::to_path_buf), ..Default::default() };
    // Finding the computer's fonts takes a moment, so it is only done for drawings with text.
    if text.contains("<text") {
        options.fontdb_mut().load_system_fonts();
    }
    let tree = resvg::usvg::Tree::from_str(&text, &options).ok()?;
    let (width, height) = (tree.size().width(), tree.size().height());
    let longest = width.max(height);
    if !(longest.is_finite() && longest >= 1.0) {
        return None;
    }
    let scale = (TARGET_SIDE.min(max_side as f32) / longest).min(MAX_ENLARGEMENT);
    let side = |length: f32| ((length * scale).round() as u32).clamp(1, max_side);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(side(width), side(height))?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let pixels = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let straight = pixel.demultiply();
            [straight.red(), straight.green(), straight.blue(), straight.alpha()]
        })
        .collect();
    Some(SvgImage { width: pixmap.width(), height: pixmap.height(), pixels })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_svg_file_is_drawn_larger_than_its_stated_size_and_keeps_its_colors() {
        let folder = std::env::temp_dir().join(format!("studio-svg-raster-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("dot.svg");
        std::fs::write(&file, r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="10" height="10" fill="#ff0000"/></svg>"##).unwrap();
        let image = rasterize(&file, 8192).unwrap();
        assert_eq!((image.width, image.height), (160, 80), "eight times its stated size");
        let pixel = |x: u32, y: u32| {
            let at = ((y * image.width + x) * 4) as usize;
            [image.pixels[at], image.pixels[at + 1], image.pixels[at + 2], image.pixels[at + 3]]
        };
        assert_eq!(pixel(40, 40), [255, 0, 0, 255], "the red square");
        assert_eq!(pixel(120, 40)[3], 0, "nothing drawn beside it");
        // Never larger than the GPU accepts.
        let small = rasterize(&file, 64).unwrap();
        assert_eq!((small.width, small.height), (64, 32));
        assert!(rasterize(&folder.join("missing.svg"), 8192).is_none());
        std::fs::remove_dir_all(&folder).ok();
    }
}
