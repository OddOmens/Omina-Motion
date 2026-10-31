//! Studio's media layer: probing files and decoding video with FFmpeg, and a frame
//! server that decodes on background threads so playback and scrubbing never wait
//! on the decoder. It implements `studio_core::media::VideoSource` for the renderer.
//! It also decodes and mixes audio, plays it through the sound card, and writes movie files.

mod audio;
mod decode;
mod encode;
mod exact;
mod hardware;
mod mixer;
mod player;
mod proxy;
mod server;
mod yuv;

pub use audio::{AudioDecoder, AudioInfo, CHANNELS, probe_audio};
pub use mixer::{MixRange, Mixer};
pub use player::{AudioOutput, AudioPlayer, AudioTimeline};
pub use decode::{MediaError, MediaInfo, VideoDecoder, probe};
pub use encode::{MovieFormat, MovieSettings, MovieWriter};
pub use exact::ExactFrames;
pub use proxy::{PROXY_WIDTH, ProxyManager, ProxyStatus, build_proxy, default_cache_folder, proxy_path};
pub use server::FrameServer;
