//! Proxies: small, all-intra copies of footage for instant scrubbing. Long-GOP formats
//! (H.264, HEVC) only store a full picture every second or more, so jumping to an
//! arbitrary frame means decoding everything since the last keyframe. A proxy stores
//! every frame as a keyframe (ProRes Proxy), so any frame is one short decode away.
//!
//! `ProxyManager` builds them one at a time on a background thread and keeps them in
//! the system cache folder, named after the source file's path, size and modification
//! time, so a changed source gets a new proxy.

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use ffmpeg_next as ffmpeg;
use ffmpeg::format::Pixel;
use ffmpeg::media::Type;
use ffmpeg::software::scaling;

use crate::decode::MediaError;

/// Proxy width in pixels. Enough for the viewer at typical sizes; full quality comes from the source.
pub const PROXY_WIDTH: u32 = 1280;

/// Transcodes `source` into an all-intra ProRes Proxy file at `output`, at most `width` wide.
/// Reports progress (0–1) as it goes and stops early, returning `Ok(false)`, if `cancelled` says so.
/// The file is written under a temporary name and only renamed into place when complete.
pub fn build_proxy(source: &Path, output: &Path, width: u32, progress: &dyn Fn(f32), cancelled: &dyn Fn() -> bool) -> Result<bool, MediaError> {
    let partial = output.with_extension("partial.mov");
    let built = encode_proxy(source, &partial, width, progress, cancelled);
    if !matches!(built, Ok(true)) {
        // Cancelled or failed: never leave a partial file in the cache.
        let _ = std::fs::remove_file(&partial);
        return built;
    }
    std::fs::rename(&partial, output).map_err(|error| {
        let _ = std::fs::remove_file(&partial);
        MediaError(format!("could not save the proxy: {error}"))
    })?;
    progress(1.0);
    Ok(true)
}

/// Writes the proxy to `partial`. Returns `Ok(false)` if cancelled.
fn encode_proxy(source: &Path, partial: &Path, width: u32, progress: &dyn Fn(f32), cancelled: &dyn Fn() -> bool) -> Result<bool, MediaError> {
    crate::decode::init()?;
    let info = crate::decode::probe(source)?;
    let mut input = ffmpeg::format::input(source)?;
    let in_stream = input.streams().best(Type::Video).ok_or_else(|| MediaError("the file has no video".into()))?;
    let in_index = in_stream.index();
    // Footage with transparency gets a ProRes 4444 proxy, which keeps it.
    let alpha = crate::decode::stream_has_alpha(&in_stream)?;
    let proxy_pixel = if alpha { Pixel::YUVA444P10LE } else { Pixel::YUV422P10LE };
    let in_time_base = in_stream.time_base();
    let mut decoder_context = ffmpeg::codec::context::Context::from_parameters(in_stream.parameters())?;
    decoder_context.set_threading(ffmpeg::codec::threading::Config::kind(ffmpeg::codec::threading::Type::Frame));
    let mut decoder = decoder_context.decoder().video()?;

    // Even dimensions, aspect ratio kept.
    let out_width = info.width.min(width.max(2)) & !1;
    let out_height = ((u64::from(info.height) * u64::from(out_width) / u64::from(info.width.max(1))) as u32).max(2) & !1;

    let mut output_context = ffmpeg::format::output(partial)?;
    let global_header = output_context.format().flags().contains(ffmpeg::format::Flags::GLOBAL_HEADER);
    let codec = ffmpeg::encoder::find_by_name("prores_ks").ok_or_else(|| MediaError("this FFmpeg has no ProRes encoder".into()))?;
    let mut out_stream = output_context.add_stream(codec)?;
    let out_index = out_stream.index();
    let mut encoder_context = ffmpeg::codec::context::Context::new_with_codec(codec);
    // Encode several frames at once on all cores (ProRes frames are independent of each other).
    encoder_context.set_threading(ffmpeg::codec::threading::Config::kind(ffmpeg::codec::threading::Type::Frame));
    let mut encoder = encoder_context.encoder().video()?;
    encoder.set_width(out_width);
    encoder.set_height(out_height);
    encoder.set_format(proxy_pixel);
    encoder.set_time_base(in_time_base);
    let rate = info.frame_rate;
    encoder.set_frame_rate(Some(ffmpeg::Rational::new(rate.numerator as i32, rate.denominator as i32)));
    if global_header {
        encoder.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
    }
    let mut options = ffmpeg::Dictionary::new();
    // Profile 0 is ProRes 422 Proxy; 4 is 4444.
    options.set("profile", if alpha { "4" } else { "0" });
    let mut encoder = encoder.open_with(options)?;
    out_stream.set_parameters(&encoder);
    out_stream.set_avg_frame_rate(ffmpeg::Rational::new(rate.numerator as i32, rate.denominator as i32));
    output_context.write_header()?;
    let out_time_base = output_context.stream(out_index).map(|stream| stream.time_base()).unwrap_or(in_time_base);

    let mut scaler: Option<(scaling::Context, Pixel, u32, u32)> = None;
    let mut done = 0_i64;
    let total = info.frame_count.max(1);

    // Moves finished packets from the encoder into the file.
    let write_packets = |encoder: &mut ffmpeg::encoder::Video, output_context: &mut ffmpeg::format::context::Output| -> Result<(), MediaError> {
        let mut packet = ffmpeg::Packet::empty();
        while encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(out_index);
            packet.rescale_ts(in_time_base, out_time_base);
            packet.write_interleaved(output_context)?;
        }
        Ok(())
    };

    // Decodes whatever the decoder has ready, scales it and sends it to the encoder.
    let mut encode_ready = |decoder: &mut ffmpeg::decoder::Video, encoder: &mut ffmpeg::encoder::Video, output_context: &mut ffmpeg::format::context::Output, done: &mut i64| -> Result<(), MediaError> {
        let mut decoded = ffmpeg::frame::Video::empty();
        while decoder.receive_frame(&mut decoded).is_ok() {
            let source_format = (decoded.format(), decoded.width(), decoded.height());
            if scaler.as_ref().is_none_or(|(_, f, w, h)| (*f, *w, *h) != source_format) {
                let context = scaling::Context::get(source_format.0, source_format.1, source_format.2, proxy_pixel, out_width, out_height, scaling::Flags::BILINEAR)?;
                scaler = Some((context, source_format.0, source_format.1, source_format.2));
            }
            let mut scaled = ffmpeg::frame::Video::empty();
            scaler.as_mut().expect("created above").0.run(&decoded, &mut scaled)?;
            scaled.set_pts(decoded.timestamp().or(decoded.pts()));
            encoder.send_frame(&scaled)?;
            write_packets(encoder, output_context)?;
            *done += 1;
        }
        Ok(())
    };

    for (stream, packet) in input.packets() {
        if cancelled() {
            return Ok(false);
        }
        if stream.index() != in_index {
            continue;
        }
        match decoder.send_packet(&packet) {
            // A damaged packet: skip it, as players do.
            Err(ffmpeg::Error::InvalidData) => continue,
            result => result?,
        }
        encode_ready(&mut decoder, &mut encoder, &mut output_context, &mut done)?;
        progress((done as f32 / total as f32).min(0.99));
    }
    decoder.send_eof()?;
    encode_ready(&mut decoder, &mut encoder, &mut output_context, &mut done)?;
    encoder.send_eof()?;
    write_packets(&mut encoder, &mut output_context)?;
    output_context.write_trailer()?;
    Ok(true)
}

/// Where proxies are kept: the platform's cache folder, under `Studio/Proxies`.
pub fn default_cache_folder() -> PathBuf {
    default_cache_root().join("Proxies")
}

/// The platform's cache folder for Studio, which holds the proxies and other rebuildable files.
pub fn default_cache_root() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home.map(|home| home.join("Library/Caches"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| home.map(|home| home.join(".cache")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("Studio")
}

/// The proxy file for `source`: named from its path, size and modification time, so editing the
/// source (or replacing it) produces a different name and a fresh proxy.
pub fn proxy_path(cache: &Path, source: &Path) -> PathBuf {
    let metadata = std::fs::metadata(source).ok();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    metadata.as_ref().map(std::fs::Metadata::len).hash(&mut hasher);
    metadata.and_then(|m| m.modified().ok()).hash(&mut hasher);
    PROXY_WIDTH.hash(&mut hasher);
    cache.join(format!("{:016x}.mov", hasher.finish()))
}

/// Where a clip's proxy stands.
#[derive(Clone, Debug, PartialEq)]
pub enum ProxyStatus {
    Queued,
    /// Building, with progress from 0 to 1.
    Building(f32),
    Ready(PathBuf),
    Failed(String),
}

type ChangeCallback = Box<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Queue {
    status: HashMap<PathBuf, ProxyStatus>,
    pending: VecDeque<PathBuf>,
    shutting_down: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    work: Condvar,
    on_change: Mutex<Option<ChangeCallback>>,
    /// The folder proxies are kept in; it can be changed while Studio runs.
    cache: Mutex<PathBuf>,
    /// The most the cache may hold, in bytes (`u64::MAX` for no limit).
    limit: AtomicU64,
}

/// The proxy files in `folder`, with their sizes and when they were last written.
fn cache_files(folder: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok().filter(|metadata| metadata.is_file())?;
            Some((entry.path(), metadata.len(), metadata.modified().unwrap_or(std::time::UNIX_EPOCH)))
        })
        .collect()
}

/// How many bytes the proxies in `folder` take up.
pub fn cache_size(folder: &Path) -> u64 {
    cache_files(folder).iter().map(|(_, size, _)| size).sum()
}

/// Deletes the oldest proxies in `folder` until it holds at most `limit` bytes, never touching
/// those in `keep` (in use, or being written). Returns how many bytes were freed.
pub fn trim_cache(folder: &Path, limit: u64, keep: &[PathBuf]) -> u64 {
    let mut files = cache_files(folder);
    let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
    files.sort_by_key(|(_, _, modified)| *modified);
    let mut freed = 0;
    for (path, size, _) in files {
        if total <= limit {
            break;
        }
        if keep.contains(&path) || std::fs::remove_file(&path).is_err() {
            continue;
        }
        total -= size;
        freed += size;
    }
    freed
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Keeps the cache within its limit, sparing the proxies this session uses.
    fn trim(&self) {
        let limit = self.limit.load(Ordering::Relaxed);
        if limit == u64::MAX {
            return;
        }
        let cache = self.folder();
        let in_use: Vec<PathBuf> = self.lock().status.keys().map(|source| proxy_path(&cache, source)).collect();
        trim_cache(&cache, limit, &in_use);
    }

    fn folder(&self) -> PathBuf {
        self.cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    fn changed(&self) {
        if let Some(callback) = self.on_change.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            callback();
        }
    }
}

/// Builds proxies in the background, one at a time, and reports their status.
pub struct ProxyManager {
    shared: Arc<Shared>,
}

impl ProxyManager {
    /// Keeps proxies in `cache` (created if missing).
    pub fn new(cache: PathBuf) -> Self {
        let shared = Arc::new(Shared { queue: Mutex::default(), work: Condvar::new(), on_change: Mutex::new(None), cache: Mutex::new(cache), limit: AtomicU64::new(u64::MAX) });
        let worker = shared.clone();
        if let Err(error) = std::thread::Builder::new().name("proxies".into()).spawn(move || build_loop(&worker)) {
            eprintln!("could not start the proxy builder: {error}");
        }
        Self { shared }
    }

    /// Called (from the builder thread) when any proxy's status changes.
    pub fn set_on_change(&self, callback: impl Fn() + Send + Sync + 'static) {
        *self.shared.on_change.lock().unwrap_or_else(|p| p.into_inner()) = Some(Box::new(callback));
    }

    /// Asks for a proxy of `source`. Already-built proxies are found in the cache immediately.
    pub fn request(&self, source: &Path) {
        let mut queue = self.shared.lock();
        if queue.status.contains_key(source) {
            return;
        }
        let existing = proxy_path(&self.shared.folder(), source);
        if existing.exists() {
            queue.status.insert(source.to_owned(), ProxyStatus::Ready(existing));
        } else {
            queue.status.insert(source.to_owned(), ProxyStatus::Queued);
            queue.pending.push_back(source.to_owned());
            self.shared.work.notify_all();
        }
        drop(queue);
        self.shared.changed();
    }

    pub fn status(&self, source: &Path) -> Option<ProxyStatus> {
        self.shared.lock().status.get(source).cloned()
    }

    /// The folder proxies are kept in.
    pub fn cache_folder(&self) -> PathBuf {
        self.shared.folder()
    }

    /// Keeps proxies in `folder` from now on. Proxies already made elsewhere are forgotten (and
    /// left where they are); footage asks for its proxy again with `request`.
    pub fn set_cache_folder(&self, folder: PathBuf) {
        *self.shared.cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = folder;
        self.shared.lock().status.retain(|_, status| matches!(status, ProxyStatus::Building(_) | ProxyStatus::Queued));
        self.shared.changed();
    }

    /// Sets the most the cache may hold, in bytes (`None` for no limit), and trims it now. Proxies
    /// in use this session are kept even if that leaves it over the limit.
    pub fn set_cache_limit(&self, limit: Option<u64>) {
        self.shared.limit.store(limit.unwrap_or(u64::MAX), Ordering::Relaxed);
        self.shared.trim();
    }

    /// Deletes every proxy not being built right now. Clips play from their original files
    /// until their proxies are asked for again (when a project is next opened, or media imported).
    /// Returns how many bytes were freed.
    pub fn clear_cache(&self) -> u64 {
        let cache = self.shared.folder();
        let building: Vec<PathBuf> = {
            let mut queue = self.shared.lock();
            let building = queue.status.iter().filter(|(_, status)| matches!(status, ProxyStatus::Building(_))).map(|(source, _)| proxy_path(&cache, source)).collect();
            queue.status.retain(|_, status| matches!(status, ProxyStatus::Building(_) | ProxyStatus::Queued));
            building
        };
        let freed = trim_cache(&cache, 0, &building);
        self.shared.changed();
        freed
    }

    /// The finished proxy for `source`, if there is one.
    pub fn ready(&self, source: &Path) -> Option<PathBuf> {
        match self.status(source)? {
            ProxyStatus::Ready(path) => Some(path),
            _ => None,
        }
    }
}

impl Drop for ProxyManager {
    fn drop(&mut self) {
        self.shared.lock().shutting_down = true;
        self.shared.work.notify_all();
    }
}

fn build_loop(shared: &Shared) {
    loop {
        let source = {
            let mut queue = shared.lock();
            loop {
                if queue.shutting_down {
                    return;
                }
                if let Some(next) = queue.pending.pop_front() {
                    break next;
                }
                queue = shared.work.wait(queue).unwrap_or_else(|p| p.into_inner());
            }
        };
        let cache = shared.folder();
        let output = proxy_path(&cache, &source);
        let set_status = |status: ProxyStatus| {
            shared.lock().status.insert(source.clone(), status);
            shared.changed();
        };
        set_status(ProxyStatus::Building(0.0));
        if let Err(error) = std::fs::create_dir_all(&cache) {
            set_status(ProxyStatus::Failed(format!("could not create the proxy folder: {error}")));
            continue;
        }
        // Report progress in whole percents, so the UI is not flooded with updates.
        let last_percent = Mutex::new(-1);
        let progress = |fraction: f32| {
            let percent = (fraction * 100.0) as i32;
            let mut last = last_percent.lock().unwrap_or_else(|p| p.into_inner());
            if percent != *last {
                *last = percent;
                set_status(ProxyStatus::Building(fraction));
            }
        };
        let cancelled = || shared.lock().shutting_down;
        match build_proxy(&source, &output, PROXY_WIDTH, &progress, &cancelled) {
            Ok(true) => {
                set_status(ProxyStatus::Ready(output));
                shared.trim();
            }
            Ok(false) => return,
            Err(error) => set_status(ProxyStatus::Failed(error.to_string())),
        }
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn the_cache_trims_oldest_first_and_spares_files_in_use() {
        let folder = std::env::temp_dir().join(format!("studio-proxy-trim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let file = |name: &str, bytes: usize| {
            let path = folder.join(name);
            std::fs::write(&path, vec![0u8; bytes]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
            path
        };
        let (old, in_use, newest) = (file("old.mov", 100), file("in-use.mov", 100), file("new.mov", 100));
        assert_eq!(cache_size(&folder), 300);
        assert_eq!(trim_cache(&folder, 150, std::slice::from_ref(&in_use)), 200, "the oldest unused went first, then the next");
        assert!(!old.exists() && in_use.exists() && !newest.exists());
        let _ = std::fs::remove_dir_all(&folder);
    }
}
