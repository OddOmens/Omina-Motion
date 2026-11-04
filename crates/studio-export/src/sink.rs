//! Where exported frames go: a movie (through FFmpeg), an animated GIF, or a folder of PNG
//! pictures. Each is written under a temporary name (see `temporary_path`) and only takes its
//! real name in `finish`; `discard` removes what was written of an export that stopped.

use std::fs::File;
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};

use studio_core::time::FrameRate;
use studio_media::{MovieSettings, MovieWriter};

use crate::format::ExportFormat;

/// How quickly GIF colors are picked for each frame, from 1 (best, very slow) to 30. 10 keeps
/// gradients smooth at a few frames a second for a 1080p GIF.
const GIF_COLOR_SPEED: i32 = 10;

pub(crate) enum Sink {
    Movie(MovieWriter),
    Gif(GifSink),
    Png(PngSink),
}

pub(crate) struct GifSink {
    encoder: gif::Encoder<BufWriter<File>>,
    rate: FrameRate,
    /// Frames written so far, to time the next one.
    written: i64,
}

pub(crate) struct PngSink {
    folder: PathBuf,
    /// The pictures' names: `stem_00000.png`.
    stem: String,
    digits: usize,
    written: i64,
}

impl Sink {
    /// Starts writing `format` at `temporary`. `movie` holds the picture size, frame rate and
    /// sound settings; `total` is how many frames will come (for numbering PNGs).
    pub(crate) fn create(format: ExportFormat, temporary: &Path, final_path: &Path, movie: MovieSettings, total: i64) -> Result<Self, String> {
        match format {
            ExportFormat::Movie(_) => MovieWriter::create(temporary, movie).map(Sink::Movie).map_err(|error| {
                // FFmpeg may have created the file before failing (an encoder refusing the size, say).
                let _ = std::fs::remove_file(temporary);
                format!("The movie could not be created: {error}")
            }),
            ExportFormat::Gif => {
                let (Ok(width), Ok(height)) = (u16::try_from(movie.width), u16::try_from(movie.height)) else {
                    return Err("A GIF can be at most 65,535 pixels a side.".into());
                };
                let file = File::create(temporary).map_err(|error| format!("The GIF could not be created: {error}"))?;
                let mut encoder = gif::Encoder::new(BufWriter::new(file), width, height, &[]).map_err(|error| format!("The GIF could not be created: {error}"))?;
                encoder.set_repeat(gif::Repeat::Infinite).map_err(|error| format!("The GIF could not be created: {error}"))?;
                Ok(Sink::Gif(GifSink { encoder, rate: movie.frame_rate, written: 0 }))
            }
            ExportFormat::PngSequence => {
                // The save panel only asks about replacing files, so an existing folder is
                // never overwritten.
                if final_path.exists() {
                    return Err(format!("“{}” already exists. Choose another name for the picture folder.", final_path.display()));
                }
                std::fs::create_dir_all(temporary).map_err(|error| format!("The picture folder could not be created: {error}"))?;
                let stem = final_path.file_name().map_or_else(|| "frame".to_owned(), |name| name.to_string_lossy().into_owned());
                let digits = total.max(1).to_string().len().max(5);
                Ok(Sink::Png(PngSink { folder: temporary.to_path_buf(), stem, digits, written: 0 }))
            }
        }
    }

    /// Adds the next frame: `width` × `height` straight-alpha BGRA pixels, already laid over
    /// black when the format has no transparency.
    pub(crate) fn write_video(&mut self, bgra: &[u8], width: u32, height: u32) -> Result<(), String> {
        match self {
            Sink::Movie(writer) => writer.write_video(bgra, width, height).map_err(|error| error.to_string()),
            Sink::Gif(sink) => {
                let mut rgba = to_rgba(bgra);
                let (Ok(width), Ok(height)) = (u16::try_from(width), u16::try_from(height)) else { return Err("the frame is too large for a GIF".into()) };
                let mut frame = gif::Frame::from_rgba_speed(width, height, &mut rgba, GIF_COLOR_SPEED);
                frame.delay = gif_delay(sink.rate, sink.written);
                sink.written += 1;
                sink.encoder.write_frame(&frame).map_err(|error| error.to_string())
            }
            Sink::Png(sink) => {
                let path = sink.folder.join(format!("{}_{:0width$}.png", sink.stem, sink.written, width = sink.digits));
                sink.written += 1;
                let picture = image::RgbaImage::from_raw(width, height, to_rgba(bgra)).ok_or("the frame is smaller than its size says")?;
                picture.save_with_format(&path, image::ImageFormat::Png).map_err(|error| error.to_string())
            }
        }
    }

    /// Adds interleaved stereo sound. Only movies carry sound; the others ignore it.
    pub(crate) fn write_audio(&mut self, samples: &[f32]) -> Result<(), String> {
        match self {
            Sink::Movie(writer) => writer.write_audio(samples).map_err(|error| error.to_string()),
            _ => Ok(()),
        }
    }

    /// Completes the file (or folder) and gives it its real name.
    pub(crate) fn finish(self, temporary: &Path, final_path: &Path) -> Result<(), String> {
        let written = match self {
            Sink::Movie(writer) => writer.finish().map_err(|error| format!("The movie could not be finished: {error}")),
            Sink::Gif(sink) => sink
                .encoder
                .into_inner()
                .map_err(|error| error.to_string())
                .and_then(|mut file| file.flush().map_err(|error| error.to_string()))
                .map_err(|error| format!("The GIF could not be finished: {error}")),
            Sink::Png(_) => Ok(()),
        };
        written.and_then(|()| std::fs::rename(temporary, final_path).map_err(|error| format!("The export could not be saved: {error}")))
    }
}

/// Removes what an export that stopped wrote at `temporary`: its file, or its picture folder.
pub fn discard(temporary: &Path) {
    if temporary.is_dir() {
        let _ = std::fs::remove_dir_all(temporary);
    } else {
        let _ = std::fs::remove_file(temporary);
    }
}

/// BGRA pixels as RGBA.
fn to_rgba(bgra: &[u8]) -> Vec<u8> {
    bgra.as_chunks::<4>().0.iter().flat_map(|&[blue, green, red, alpha]| [red, green, blue, alpha]).collect()
}

/// How long GIF frame `index` shows, in hundredths of a second. Each frame ends at its own
/// moment rounded to the nearest hundredth, so a 30 fps GIF alternates 3 and 4 hundredths and
/// keeps time overall instead of drifting.
fn gif_delay(rate: FrameRate, index: i64) -> u16 {
    let end = |frame: i64| (frame as f64 * 100.0 / rate.fps()).round() as i64;
    (end(index + 1) - end(index)).clamp(1, i64::from(u16::MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gif_frames_keep_time_at_rates_hundredths_dont_divide() {
        let rate = FrameRate::new(30, 1);
        let delays: Vec<u16> = (0..30).map(|index| gif_delay(rate, index)).collect();
        assert_eq!(delays.iter().map(|&d| u32::from(d)).sum::<u32>(), 100, "thirty frames last one second");
        assert!(delays.iter().all(|&d| d == 3 || d == 4), "{delays:?}");
        assert_eq!(gif_delay(FrameRate::new(50, 1), 7), 2);
    }

    #[test]
    fn colors_swap_to_rgba() {
        assert_eq!(to_rgba(&[1, 2, 3, 4, 5, 6, 7, 8]), [3, 2, 1, 4, 7, 6, 5, 8]);
    }
}
