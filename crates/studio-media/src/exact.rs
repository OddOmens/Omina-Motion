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
    /// The first frame of each file that could not be decoded, and why. An export must not
    /// quietly repeat an earlier frame in its place.
    failed_frames: Mutex<HashMap<PathBuf, (i64, String)>>,
}

impl ExactFrames {
    pub fn new() -> Self {
        Self::default()
    }

    /// Files that could not be opened, or had a frame that could not be decoded, and why.
    pub fn errors(&self) -> Vec<(PathBuf, String)> {
        let decoders = self.decoders.lock().unwrap_or_else(|p| p.into_inner());
        let mut errors: Vec<(PathBuf, String)> =
            decoders.iter().filter_map(|(path, decoder)| decoder.as_ref().err().map(|error| (path.clone(), error.clone()))).collect();
        let failed = self.failed_frames.lock().unwrap_or_else(|p| p.into_inner());
        errors.extend(failed.iter().map(|(path, (index, error))| (path.clone(), format!("frame {index}: {error}"))));
        errors
    }
}

impl VideoSource for ExactFrames {
    fn frame(&self, path: &Path, index: i64) -> Option<Arc<VideoFrame>> {
        let mut decoders = self.decoders.lock().unwrap_or_else(|p| p.into_inner());
        let decoder = decoders
            .entry(path.to_owned())
            .or_insert_with(|| VideoDecoder::open_fastest(path, MAX_WIDTH).map_err(|error| error.to_string()));
        match decoder.as_mut().ok()?.frame(index) {
            Ok(frame) => Some(Arc::new(frame)),
            Err(error) => {
                self.failed_frames.lock().unwrap_or_else(|p| p.into_inner()).entry(path.to_owned()).or_insert((index, error.to_string()));
                None
            }
        }
    }
}
