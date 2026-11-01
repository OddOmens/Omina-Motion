//! Studio's GPU compositor. It renders a composition frame with wgpu (Metal, Vulkan
//! or DirectX 12) without any window, so the editor, export and a headless render
//! node all share it. See `renderer` for the pipeline.

pub mod effects;
mod path;
mod renderer;
pub mod text;

pub use renderer::{RenderError, RenderedFrame, Renderer};
