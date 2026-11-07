//! Exporting to the vector formats (Lottie, dotLottie and animated SVG). The file is worked out
//! whole by `studio_vector` from the project; this side supplies what that needs from the
//! machine (the system's fonts, to outline text exactly as Studio lays it out, and the image
//! files to embed) and writes the result the same safe way as the other formats.

use std::path::Path;

use studio_core::model::{Project, ProjectItem, TextStyle};
use studio_render::text::TextRenderer;
use studio_vector::{EmbeddedImage, GlyphOutline, Resources, TextOutline, VectorOptions};

use crate::{ExportFormat, ExportRequest, Outcome, Progress, discard, temporary_path};

/// Fonts and pictures from this machine. The fonts are loaded the first time text needs them.
struct MachineResources {
    text: Option<TextRenderer>,
}

impl Resources for MachineResources {
    fn image(&mut self, item: &ProjectItem) -> Option<EmbeddedImage> {
        let bytes = std::fs::read(item.path.as_ref()?).ok()?;
        // Players read PNG and JPEG; Studio's pictures are one or the other.
        let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            "image/jpeg"
        } else {
            return None;
        };
        let size = item.size?;
        Some(EmbeddedImage { mime, bytes, width: size.x.round().max(1.0) as u32, height: size.y.round().max(1.0) as u32 })
    }

    fn text(&mut self, text: &str, style: &TextStyle) -> Option<TextOutline> {
        let renderer = self.text.get_or_insert_with(TextRenderer::new);
        let outlines = renderer.outlines(text, style);
        let glyphs = outlines.glyphs.into_iter().map(|glyph| GlyphOutline { start: glyph.start, line: glyph.line, left: glyph.left, width: glyph.width, contours: glyph.contours }).collect();
        Some(TextOutline { size: outlines.size, lines: outlines.lines, glyphs })
    }
}

/// Writes `request` (a vector format) from `project`. Progress jumps to done when the file is
/// written; the work is quick and can only be cancelled before it starts writing.
pub(crate) fn export(project: &Project, request: &ExportRequest, progress: &mut dyn FnMut(Progress), cancelled: &dyn Fn() -> bool) -> Result<Outcome, String> {
    let composition = project.composition(request.composition).ok_or("The composition no longer exists.")?;
    let total = request.output_frame_count(composition.frame_rate);
    let options = VectorOptions { composition: request.composition, frames: request.frames.clone(), frame_rate: request.frame_rate };
    let mut resources = MachineResources { text: None };
    if cancelled() {
        return Ok(Outcome::Cancelled);
    }
    let (bytes, left_out) = match request.format {
        ExportFormat::Lottie => {
            let exported = studio_vector::lottie(project, &options, &mut resources)?;
            (exported.file.into_bytes(), exported.left_out)
        }
        ExportFormat::DotLottie => {
            let exported = studio_vector::dotlottie(project, &options, &mut resources)?;
            (exported.file, exported.left_out)
        }
        ExportFormat::AnimatedSvg => {
            let exported = studio_vector::svg(project, &options, &mut resources)?;
            (exported.file.into_bytes(), exported.left_out)
        }
        other => return Err(format!("{} isn’t a vector format.", other.label())),
    };
    if cancelled() {
        return Ok(Outcome::Cancelled);
    }
    write_whole(&request.path, &bytes)?;
    progress(Progress { done: total, total });
    Ok(Outcome::Finished { left_out })
}

/// Writes `bytes` under a temporary name next to `path`, then gives it its real name, so a
/// failed write never leaves half a file under that name.
fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = temporary_path(path);
    if let Err(error) = std::fs::write(&temporary, bytes) {
        discard(&temporary);
        return Err(format!("The file could not be written: {error}"));
    }
    std::fs::rename(&temporary, path).map_err(|error| {
        discard(&temporary);
        format!("The file could not be saved: {error}")
    })
}
