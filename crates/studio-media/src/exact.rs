//! A video source for export: every request decodes exactly the frame asked for, at the
//! source's full size, waiting for it. (The viewer's `FrameServer` instead answers at once
//! with the nearest frame it has, which is right for scrubbing but not for a final render.)
//! Export reads frames in order, which is the decoder's fast path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use studio_core::media::{VideoFrame, VideoSource};

use crate::decode::VideoDecoder;

/// The largest picture decoded; matches the renderer's largest frame.
const MAX_WIDTH: u32 = 8192;

#[derive(Default)]
pub struct ExactFrames {
    /// One decoder per file, or why it could not be opened (so it is not retried every frame).
    decoders: Mutex<HashMap<PathBuf, Result<VideoDecoder, String>>>,
}

impl ExactFrames {
    pub fn new() -> Self {
        Self::default()
    }

    /// Files that could not be decoded, and why.
    pub fn errors(&self) -> Vec<(PathBuf, String)> {
        let decoders = self.decoders.lock().unwrap_or_else(|p| p.into_inner());
        decoders.iter().filter_map(|(path, decoder)| decoder.as_ref().err().map(|error| (path.clone(), error.clone()))).collect()
    }
}

impl VideoSource for ExactFrames {
    fn frame(&self, path: &Path, index: i64) -> Option<Arc<VideoFrame>> {
        let mut decoders = self.decoders.lock().unwrap_or_else(|p| p.into_inner());
        let decoder = decoders
            .entry(path.to_owned())
            .or_insert_with(|| VideoDecoder::open_fastest(path, MAX_WIDTH).map_err(|error| error.to_string()));
        decoder.as_mut().ok()?.frame(index).ok().map(Arc::new)
    }
}
