//! Telling FFmpeg's scaler which YUV formula a video uses. Without this the scaler assumes the
//! standard-definition formula (BT.601) for every file, so HD footage (BT.709) comes out with its
//! colors shifted. `ffmpeg-next` has no safe wrapper for `sws_setColorspaceDetails`, so this
//! module calls FFmpeg directly; like `hardware`, every `unsafe` block states why it is sound.
#![allow(unsafe_code)]

use ffmpeg_next as ffmpeg;
use ffmpeg::ffi;
use ffmpeg::format::Pixel;
use ffmpeg::software::scaling;
use ffmpeg::util::color::{Range, Space};

/// The YUV formula a picture uses. Files that do not say are treated as players treat them:
/// HD and larger (720 lines or more) as BT.709, smaller as BT.601.
pub fn matrix(space: Space, height: u32) -> Space {
    match space {
        Space::BT709 | Space::FCC | Space::BT470BG | Space::SMPTE170M | Space::SMPTE240M | Space::BT2020NCL | Space::BT2020CL => space,
        _ if height >= 720 => Space::BT709,
        _ => Space::SMPTE170M,
    }
}

/// Whether a picture uses the full 0–255 range rather than the video range (16–235). The older
/// "J" pixel formats are always full range, whatever the file says.
pub fn is_full_range(format: Pixel, range: Range) -> bool {
    range == Range::JPEG || matches!(format, Pixel::YUVJ420P | Pixel::YUVJ422P | Pixel::YUVJ444P | Pixel::YUVJ440P | Pixel::YUVJ411P)
}

/// Sets `scaler` to read its YUV source with `matrix` and the given range. A YUV target keeps the
/// same formula and gets `target_full_range` (pass `true` for an RGB target). If FFmpeg refuses (for a source that is not YUV, where there is no formula to apply),
/// the scaler keeps its defaults.
pub fn set_source_colors(scaler: &mut scaling::Context, matrix: Space, source_full_range: bool, target_full_range: bool) {
    let space: ffi::AVColorSpace = matrix.into();
    // SAFETY: `sws_getCoefficients` returns a pointer to one of FFmpeg's static tables (a default
    // one for unknown values), valid for the whole program. `scaler` is a valid, initialized
    // context that we hold exclusively; FFmpeg copies the tables rather than keeping the pointers.
    // The last three arguments are brightness 0 and contrast and saturation 1.0 in 16.16 fixed point.
    unsafe {
        let table = ffi::sws_getCoefficients(space as i32);
        ffi::sws_setColorspaceDetails(scaler.as_mut_ptr(), table, i32::from(source_full_range), table, i32::from(target_full_range), 0, 1 << 16, 1 << 16);
    }
}
