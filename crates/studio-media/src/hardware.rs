//! Hardware video decoding through FFmpeg: VideoToolbox on macOS, VAAPI on Linux and
//! D3D11 on Windows. `ffmpeg-next` has no safe wrapper for this, so this module calls
//! FFmpeg directly. It is the only `unsafe` code in Studio; every block states why it
//! is sound. Anything that fails here falls back to software decoding.
#![allow(unsafe_code)]

use std::ptr;

use ffmpeg_next as ffmpeg;
use ffmpeg::ffi;

/// The hardware decoders to try on this platform, best first, with the pixel format they produce.
fn candidates() -> &'static [(ffi::AVHWDeviceType, ffi::AVPixelFormat, &'static str)] {
    #[cfg(target_os = "macos")]
    return &[(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX, ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX, "VideoToolbox")];
    #[cfg(target_os = "linux")]
    return &[(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI, ffi::AVPixelFormat::AV_PIX_FMT_VAAPI, "VAAPI")];
    #[cfg(target_os = "windows")]
    return &[(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA, ffi::AVPixelFormat::AV_PIX_FMT_D3D11, "D3D11")];
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return &[];
}

/// A hardware decoding device attached to a codec context. Releases the device when dropped.
pub struct HardwareDevice {
    device: *mut ffi::AVBufferRef,
    pub name: &'static str,
}

impl Drop for HardwareDevice {
    fn drop(&mut self) {
        // SAFETY: `device` came from a successful `av_hwdevice_ctx_create` and is released exactly
        // once, here. The codec context holds its own reference, released when the context is freed.
        unsafe { ffi::av_buffer_unref(&mut self.device) };
    }
}

/// FFmpeg asks which pixel format to decode into. Pick the hardware format stored in the context's
/// `opaque` field if the decoder offers it; otherwise let FFmpeg choose a software format.
unsafe extern "C" fn choose_format(context: *mut ffi::AVCodecContext, offered: *const ffi::AVPixelFormat) -> ffi::AVPixelFormat {
    // SAFETY: FFmpeg passes a valid context and a list of formats ending in AV_PIX_FMT_NONE.
    // `opaque` holds the wanted format as an integer, set in `attach` before the decoder opened.
    unsafe {
        let wanted = (*context).opaque as usize as i32;
        let mut entry = offered;
        while *entry != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
            if *entry as i32 == wanted {
                return *entry;
            }
            entry = entry.add(1);
        }
        ffi::avcodec_default_get_format(context, offered)
    }
}

/// Tries to attach a hardware decoder to `context`, which must not be opened yet.
/// Returns `None` (leaving the context untouched) if no hardware decoder is available.
pub fn attach(context: &mut ffmpeg::codec::context::Context) -> Option<HardwareDevice> {
    for &(device_type, pixel_format, name) in candidates() {
        let mut device: *mut ffi::AVBufferRef = ptr::null_mut();
        // SAFETY: `device` is a valid out-pointer; null device name and options mean "default".
        let created = unsafe { ffi::av_hwdevice_ctx_create(&mut device, device_type, ptr::null(), ptr::null_mut(), 0) };
        if created < 0 || device.is_null() {
            continue;
        }
        // SAFETY: the context is not opened yet, so FFmpeg reads these fields when it opens.
        // `av_buffer_ref` gives the context its own reference to the device.
        unsafe {
            let raw = context.as_mut_ptr();
            (*raw).hw_device_ctx = ffi::av_buffer_ref(device);
            (*raw).opaque = pixel_format as i32 as usize as *mut std::ffi::c_void;
            (*raw).get_format = Some(choose_format);
        }
        return Some(HardwareDevice { device, name });
    }
    None
}

/// Copies a hardware frame into ordinary memory so it can be converted to RGBA.
/// Returns the frame unchanged if it is already in memory.
pub fn download(frame: ffmpeg::frame::Video) -> Result<ffmpeg::frame::Video, ffmpeg::Error> {
    // SAFETY: reading `hw_frames_ctx` of a valid frame; it is non-null only for hardware frames.
    let on_gpu = unsafe { !(*frame.as_ptr()).hw_frames_ctx.is_null() };
    if !on_gpu {
        return Ok(frame);
    }
    let mut memory = ffmpeg::frame::Video::empty();
    // SAFETY: both frames are valid; FFmpeg allocates `memory`'s buffers in a format the hardware
    // supports for downloads, then copies timestamps and other properties across.
    unsafe {
        let copied = ffi::av_hwframe_transfer_data(memory.as_mut_ptr(), frame.as_ptr(), 0);
        if copied < 0 {
            return Err(ffmpeg::Error::from(copied));
        }
        let properties = ffi::av_frame_copy_props(memory.as_mut_ptr(), frame.as_ptr());
        if properties < 0 {
            return Err(ffmpeg::Error::from(properties));
        }
    }
    Ok(memory)
}
