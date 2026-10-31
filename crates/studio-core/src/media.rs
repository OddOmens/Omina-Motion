//! What the renderer needs from decoded media, without depending on any decoder.
//! `studio-media` implements `VideoSource` with FFmpeg; tests can use simple stand-ins.

use std::path::Path;
use std::sync::Arc;

/// One decoded picture as opaque, straight-alpha sRGB RGBA bytes, `width * 4` per row.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    /// The source frame number this picture shows, counting from 0.
    pub index: i64,
    pub rgba: Vec<u8>,
    /// True if this picture came from a low-resolution proxy rather than the original file.
    pub from_proxy: bool,
}

/// Supplies decoded video frames to the renderer.
pub trait VideoSource {
    /// The picture for source frame `index` of the file at `path`, or the nearest one already
    /// decoded while the exact frame is still on its way. `None` if nothing is available yet.
    fn frame(&self, path: &Path, index: i64) -> Option<Arc<VideoFrame>>;
}

/// A source with no video, for rendering without a decoder (footage shows its placeholder).
pub struct NoVideo;

impl VideoSource for NoVideo {
    fn frame(&self, _: &Path, _: i64) -> Option<Arc<VideoFrame>> {
        None
    }
}
