//! Studio's media layer: probing files and decoding video with FFmpeg, and a frame
//! server that decodes on background threads so playback and scrubbing never wait
//! on the decoder. It implements `studio_core::media::VideoSource` for the renderer.
//! It also decodes and mixes audio, plays it through the sound card, and writes movie files,
//! and makes the timeline's filmstrip thumbnails and waveforms in the background.

mod audio;
mod beats;
mod decode;
mod encode;
mod exact;
mod hardware;
mod mixer;
mod peaks;
mod player;
mod proxy;
mod server;
mod thumbnails;
mod track;
mod yuv;

pub use audio::{AudioDecoder, AudioInfo, CHANNELS, probe_audio};
pub use beats::{beats_from_energy, detect_beats};
pub use mixer::{MixRange, Mixer, gate_for_tests};
pub use player::{AudioOutput, AudioPlayer, AudioTimeline};
pub use decode::{MediaError, MediaInfo, VideoDecoder, probe};
pub use encode::{EncodeSpeed, MovieFormat, MovieSettings, MovieWriter, VideoQuality};
pub use exact::ExactFrames;
pub use proxy::{PROXY_WIDTH, ProxyManager, ProxyStatus, build_proxy, cache_size, default_cache_folder, default_cache_root, proxy_path};
pub use peaks::{PeakCache, Peaks, audio_peaks};
pub use server::FrameServer;
pub use thumbnails::Thumbnailer;
pub use track::track_point;
