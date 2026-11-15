//! The compositor. For each visible layer, bottom to top: draw its source into a
//! composition-sized texture with its world transform (anti-aliased), run its effects,
//! then composite it onto everything below with its opacity, blend mode and track matte.
//! Adjustment layers apply their effects to everything below them instead.
//! Work happens in 16-bit float linear light; the result is converted to sRGB BGRA.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use studio_core::animation::Vec2;
use studio_core::effect_graph;
use studio_core::media::{VideoFrame, VideoSource};
use studio_core::model::{Id, Layer, LayerContent, LutSource, Paint, Project, Rgba, ShapeKind, ShapeStyle, TextStyle};
use studio_core::transform::{Affine, world_transform};
use wgpu::util::DeviceExt as _;

use crate::effects::{self, Pass};
use crate::text::TextRenderer;

const WORK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
const MEDIA_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const MSAA_SAMPLES: u32 = 4;
/// Text rasters kept in memory; older ones are dropped when the cache is full.
const TEXT_CACHE_LIMIT: usize = 64;
/// Pen path rasters kept in memory, trimmed the same way.
const PATH_CACHE_LIMIT: usize = 32;
/// The effect shader pass that keeps an image only where (or only outside where) a mask draws.
const MASK_PASS: u32 = 60;
/// The effect shader pass that mixes one more moment into a motion-blurred layer's average.
const MOTION_BLUR_PASS: u32 = 84;
/// Finishes an object matte's edge: moves it in or out and hardens it (see `effect.wgsl`).
const MATTE_EDGE_PASS: u32 = 85;
/// How far Shift Edge at 100% moves a hard edge, in output pixels before the layer's scale.
const SHIFT_EDGE_REACH: f32 = 6.0;

#[derive(Debug)]
pub struct RenderError(pub String);

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RenderError {}

/// A finished frame.
pub struct RenderedFrame {
    pub width: u32,
    pub height: u32,
    /// sRGB pixels in BGRA order, `width * 4` bytes per row.
    pub bgra: Vec<u8>,
    /// Measured text box sizes, so the editor can keep text layer bounds exact.
    pub text_sizes: Vec<(Id, Vec2)>,
    /// True if any footage in the frame was drawn from a low-resolution proxy.
    pub used_proxy: bool,
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// Textures for one output size, reused frame to frame.
struct Targets {
    width: u32,
    height: u32,
    /// Work textures, addressed by index so passes can ping-pong between them.
    work: Vec<Target>,
    msaa: Target,
    output: Target,
    readback: wgpu::Buffer,
    padded_row: u32,
}

/// Work texture roles. Each pair ping-pongs: a pass reads one and writes the other.
mod slot {
    pub const ACCUM_A: usize = 0;
    pub const ACCUM_B: usize = 1;
    pub const LAYER_A: usize = 2;
    pub const LAYER_B: usize = 3;
    pub const MATTE_A: usize = 4;
    pub const MATTE_B: usize = 5;
    pub const AUX_A: usize = 6;
    pub const AUX_B: usize = 7;
    pub const COUNT: usize = 8;
}

struct MediaTexture {
    view: wgpu::TextureView,
    size: Vec2,
}

/// A lookup table on the GPU: a 3D texture, its samples per side and the input range it covers.
struct LutTexture {
    view: wgpu::TextureView,
    size: u32,
    domain_min: [f32; 3],
    domain_max: [f32; 3],
}

/// A clip's current frame on the GPU, rewritten in place as playback moves.
struct VideoTexture {
    texture: MediaTexture,
    raw: wgpu::Texture,
    /// Which source frame the texture holds.
    index: i64,
}

/// What `layer.wgsl` fills a layer's rectangle with.
// A shape's style (with its gradients) makes `Shape` much the largest; sources only live for
// one draw call, so that costs nothing worth a box.
#[allow(clippy::large_enum_variant)]
/// One thing to draw: where (source pixels to composition pixels), what, and which rectangle
/// of source pixels.
type Drawn<'a> = (Affine, Source<'a>, (Vec2, Vec2));

// Sources live only while a frame is drawn; boxing the large shape variant would just add an
// allocation per shape drawn.
#[allow(clippy::large_enum_variant)]
enum Source<'a> {
    /// A premultiplied linear color.
    Solid([f32; 4]),
    /// A straight-alpha sRGB texture (text, images).
    Texture(&'a wgpu::TextureView),
    /// A texture drawn at an opacity (0–1), for caption fades.
    FadedTexture(&'a wgpu::TextureView, f32),
    /// Part of a texture, as fractions of it (left, top, width, height), drawn at an opacity
    /// (0–1): one letter of text set along a path, or one piece of animated text.
    TexturePart(&'a wgpu::TextureView, [f32; 4], f32),
    /// Media that is not decoded yet: a checker in this premultiplied linear color.
    Placeholder([f32; 4]),
    /// A shape filling a box: its origin and size, in the same pixels as the drawn rectangle
    /// (which can be larger, so a stroke centered on the edge is not cut off).
    Shape(ShapeStyle, Vec2, Vec2),
}

/// A paint as `layer.wgsl`'s `Paint` takes it, as floats in order: mode (1 solid, 2 linear,
/// 3 radial; 0 none) and stop count, eight premultiplied linear colors, eight stop locations,
/// and the gradient's from and to points (fractions of the box).
fn paint_uniform(paint: Option<Paint>) -> Vec<f32> {
    use studio_core::gradient::MAX_STOPS;
    let mut colors = [[0.0f32; 4]; MAX_STOPS];
    let mut stops = [1.0f32; MAX_STOPS];
    let (mode, count, points) = match paint {
        None => (0.0, 0, [0.0; 4]),
        Some(Paint::Solid(color)) => {
            colors[0] = linear(color);
            (1.0, 1, [0.0; 4])
        }
        Some(paint @ (Paint::Linear(gradient) | Paint::Radial(gradient))) => {
            for (index, stop) in gradient.stops().iter().enumerate() {
                colors[index] = linear(stop.color);
                stops[index] = stop.at;
            }
            let mode = if matches!(paint, Paint::Linear(_)) { 2.0 } else { 3.0 };
            (mode, gradient.stops().len(), [gradient.from.x, gradient.from.y, gradient.to.x, gradient.to.y])
        }
    };
    let mut floats = vec![mode, count as f32, 0.0, 0.0];
    floats.extend(colors.iter().flatten());
    floats.extend(stops);
    floats.extend(points);
    floats
}

/// Compositions nest at most this deep (a composition inside one that is inside it again
/// would otherwise never end).
const MAX_NESTING: u32 = 8;

/// The most nested compositions drawn for one frame. Depth alone doesn't bound the work: a
/// composition showing many copies of one that shows many copies, and so on, multiplies it.
const MAX_NESTED_RENDERS: u32 = 64;

/// The largest frame side drawn, in pixels (what GPUs accept everywhere).
const MAX_SIDE: u32 = 8192;

/// A caption layer's drawing for one frame, from its animation.
struct CaptionDraw {
    opacity: f32,
    scale: f32,
    background: Option<Rgba>,
}

/// Text set along a path, worked out for one frame: where each letter sits on its line (left
/// edge and width) and the baseline (see `TextRenderer::letters`), the path in composition
/// pixels, where the text starts on it, and how big the letters are drawn.
struct PathText {
    letters: Vec<(f32, f32)>,
    baseline: f32,
    walk: studio_core::path::Walk,
    start: f32,
    letter_scale: f32,
}

/// One piece of animated text for one frame (see `studio_core::text_animation`): the part of
/// the text's raster it is, in the text's own pixels (origin and size, padding included where
/// it is at an edge), and how it sits.
struct TextPiece {
    origin: Vec2,
    size: Vec2,
    pose: studio_core::text_animation::UnitPose,
}

/// Padding around caption text inside its background box, in layer pixels.
const CAPTION_PADDING: Vec2 = Vec2::new(28.0, 14.0);

/// The shape number `layer.wgsl` expects for each kind.
fn shape_code(kind: ShapeKind) -> f32 {
    match kind {
        ShapeKind::Rectangle => 0.0,
        ShapeKind::Ellipse => 1.0,
        ShapeKind::Triangle => 2.0,
        ShapeKind::Polygon => 3.0,
        ShapeKind::Star => 4.0,
        // Paths are drawn from a raster (see `path`), never by the shape shader.
        ShapeKind::Path => 0.0,
    }
}

pub struct Renderer {
    /// Whether the composition being composited mixes its layers in display colors rather
    /// than linear light (see `Composition::linear_blending`). Set per composition as it is
    /// drawn; nested compositions are drawn before, so each uses its own.
    display_blending: bool,
    /// Whether simple layers are drawn together (see `render`). Always on, except to check
    /// that drawing them one at a time gives the same picture.
    batching: bool,
    device: wgpu::Device,
    queue: wgpu::Queue,
    layer_pipeline: wgpu::RenderPipeline,
    layer_layout: wgpu::BindGroupLayout,
    composite_pipeline: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    effect_pipeline: wgpu::RenderPipeline,
    effect_layout: wgpu::BindGroupLayout,
    lut_pipeline: wgpu::RenderPipeline,
    lut_layout: wgpu::BindGroupLayout,
    output_pipeline: wgpu::RenderPipeline,
    output_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    blank: wgpu::TextureView,
    /// Work textures for the sizes recently rendered at, most recent last: the frame itself, and
    /// any nested compositions in it (each at its own size).
    targets: Vec<Targets>,
    text: TextRenderer,
    text_cache: HashMap<u64, MediaTexture>,
    /// How each caption layer on screen draws this frame (its text is in the text cache).
    captions: HashMap<Id, CaptionDraw>,
    /// Nested compositions on screen this frame, rendered, by the layer that shows them.
    nested: HashMap<Id, MediaTexture>,
    /// How deep the composition being rendered is nested (0 for the one asked for).
    depth: u32,
    /// Nested compositions still allowed this frame (see `MAX_NESTED_RENDERS`).
    nested_left: u32,
    /// Rasterized pen paths: the texture, and where it sits in the layer's own pixels.
    path_cache: HashMap<u64, (MediaTexture, Vec2)>,
    /// Which cached path raster each path layer on screen draws this frame.
    paths: HashMap<Id, u64>,
    /// Text set along a path this frame, by layer.
    path_text: HashMap<Id, PathText>,
    /// Text being animated piece by piece this frame, by layer.
    text_pieces: HashMap<Id, Vec<TextPiece>>,
    /// The background-removal mask each cut-out layer on screen uses, with the file it came
    /// from (so it is loaded again only when the frame's mask changes).
    cutouts: HashMap<Id, (PathBuf, MediaTexture)>,
    /// The measured text box for each cached raster.
    text_boxes: HashMap<u64, Vec2>,
    /// Loaded images (or `None` where loading failed), with the file's modification time when
    /// loaded: a file that changes on disk, or appears after failing, is loaded again.
    images: HashMap<PathBuf, (Option<std::time::SystemTime>, Option<MediaTexture>)>,
    /// Lookup tables for Apply LUT effects, with the file's modification time when loaded (`None`
    /// for built-in looks), or `None` where the table could not be read.
    luts: HashMap<LutSource, (Option<std::time::SystemTime>, Option<LutTexture>)>,
    /// The current picture of each footage layer. Keyed by layer, so two layers showing
    /// different moments of the same clip each have their own.
    videos: HashMap<Id, VideoTexture>,
    /// The largest texture side the GPU accepts; larger images and text are scaled down to fit.
    max_texture: u32,
    /// The last error the GPU reported. Without a handler wgpu would panic, which ends the app;
    /// instead the frame it happened in fails with this message.
    gpu_error: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

/// Converts straight-alpha sRGB pixels to premultiplied ones, still sRGB encoded: each color is
/// multiplied by its alpha in linear light, then encoded back. The texture's sRGB format decodes
/// them to premultiplied linear values before filtering, so semi-transparent edges blend
/// correctly instead of picking up the (often black) color of transparent neighbors.
fn premultiply(pixels: &[u8]) -> Vec<u8> {
    // Opaque pixels are unchanged, so most images and all video frames skip the math.
    if pixels.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 255) {
        return pixels.to_vec();
    }
    let to_linear: Vec<f32> = (0..=255).map(|value| srgb_to_linear(f32::from(value as u8) / 255.0)).collect();
    let encode = |linear: f32| {
        let c = linear.clamp(0.0, 1.0);
        let srgb = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
        (srgb * 255.0).round() as u8
    };
    let mut out = Vec::with_capacity(pixels.len());
    for pixel in pixels.as_chunks::<4>().0 {
        let alpha = f32::from(pixel[3]) / 255.0;
        if pixel[3] == 255 {
            out.extend_from_slice(pixel);
        } else if pixel[3] == 0 {
            // Fully transparent, as most of a shape's raster or a nested composition often is.
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            out.extend([0, 1, 2].map(|channel| encode(to_linear[usize::from(pixel[channel])] * alpha)));
            out.push(pixel[3]);
        }
    }
    out
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
}

/// A color as premultiplied linear RGBA for the shaders.
fn linear(color: Rgba) -> [f32; 4] {
    let a = color.a.clamp(0.0, 1.0);
    [srgb_to_linear(color.r) * a, srgb_to_linear(color.g) * a, srgb_to_linear(color.b) * a, a]
}

/// BGRA pixels as RGBA (the order `upload` takes).
fn swap_bgra_rgba(bgra: &[u8]) -> Vec<u8> {
    bgra.as_chunks::<4>().0.iter().flat_map(|pixel| [pixel[2], pixel[1], pixel[0], pixel[3]]).collect()
}

/// The largest factor by which a transform enlarges things, for choosing raster resolution.
/// What drawing a mask or track matte needs to know about the frame being rendered.
struct Shapers<'a> {
    project: &'a Project,
    composition: &'a studio_core::model::Composition,
    frame: i64,
    at: f64,
    time: f32,
    scale: f32,
    text_keys: &'a HashMap<Id, (u64, Vec2, Vec2)>,
}

/// A layer's opacity at frame `at`, from 0 to 1.
fn opacity_at(layer: &Layer, at: f64) -> f32 {
    layer.transform.as_ref().map_or(1.0, |t| (t.opacity.value_at(at) / 100.0).clamp(0.0, 1.0))
}

/// `layer`'s own rectangle in output pixels, its corners clockwise from the top left: where
/// effects tied to the layer's shape (Corner Pin) start from.
fn layer_box(layer: &Layer, world: Affine, scale: f32) -> [Vec2; 4] {
    let (width, height) = (layer.size.x, layer.size.y);
    [Vec2::new(0.0, 0.0), Vec2::new(width, 0.0), Vec2::new(width, height), Vec2::new(0.0, height)].map(|corner| world.apply(corner) * scale)
}

fn max_scale(transform: Affine) -> f32 {
    (transform.a.hypot(transform.b)).max(transform.c.hypot(transform.d))
}

/// The cache key for a pen path drawn at `raster_scale`: the path, its paints and its box.
fn path_key(path: &studio_core::path::ShapePath, style: &ShapeStyle, size: Vec2, raster_scale: f32) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{path:?}{style:?}").hash(&mut hasher);
    for value in [size.x, size.y, (raster_scale * 16.0).round()] {
        value.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

fn text_key(text: &str, style: &TextStyle, raster_scale: f32) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    style.font_family.hash(&mut hasher);
    style.font_weight.hash(&mut hasher);
    for value in [style.font_size, style.line_height, style.tracking, (raster_scale * 16.0).round()] {
        value.to_bits().hash(&mut hasher);
    }
    format!("{:?}{:?}{:?}", style.fill, style.stroke, style.align).hash(&mut hasher);
    hasher.finish()
}

/// Reads a `.cube` file, refusing ones too large to be a real lookup table.
pub fn read_cube(path: &std::path::Path) -> Result<studio_core::grade::Lut, String> {
    let bytes = std::fs::metadata(path).map_err(|error| error.to_string())?.len();
    if bytes > studio_core::grade::MAX_FILE_BYTES {
        return Err("it is too large to be a lookup table".into());
    }
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    studio_core::grade::parse_cube(&text)
}

impl Renderer {
    /// Creates a renderer on the default GPU. No window or display is needed.
    pub fn new() -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|error| RenderError(format!("no GPU available: {error}")))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("studio-render"),
            ..Default::default()
        }))
        .map_err(|error| RenderError(format!("could not open the GPU: {error}")))?;
        let max_texture = device.limits().max_texture_dimension_2d;
        let gpu_error = std::sync::Arc::new(std::sync::Mutex::new(None));
        let sink = gpu_error.clone();
        device.on_uncaptured_error(std::sync::Arc::new(move |error: wgpu::Error| {
            *sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error.to_string());
        }));

        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries })
        };
        let layer_layout = layout("layer", &[uniform_entry, texture_entry(1), sampler_entry(2)]);
        let composite_layout = layout("composite", &[uniform_entry, texture_entry(1), texture_entry(2), texture_entry(3)]);
        let effect_layout = layout("effect", &[uniform_entry, texture_entry(1), texture_entry(2), sampler_entry(3)]);
        let output_layout = layout("output", &[texture_entry(0)]);
        let table_entry = wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D3,
                multisampled: false,
            },
            count: None,
        };
        let lut_layout = layout("lut", &[uniform_entry, texture_entry(1), table_entry, sampler_entry(3)]);

        let pipeline = |label: &str, source: &str, layout: &wgpu::BindGroupLayout, format, samples, blend| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // Layers draw premultiplied color over a transparent target.
        let premultiplied = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let layer_pipeline = pipeline("layer", include_str!("shaders/layer.wgsl"), &layer_layout, WORK_FORMAT, MSAA_SAMPLES, premultiplied);
        let composite_pipeline = pipeline("composite", include_str!("shaders/composite.wgsl"), &composite_layout, WORK_FORMAT, 1, None);
        let effect_pipeline = pipeline("effect", include_str!("shaders/effect.wgsl"), &effect_layout, WORK_FORMAT, 1, None);
        let lut_pipeline = pipeline("lut", include_str!("shaders/lut.wgsl"), &lut_layout, WORK_FORMAT, 1, None);
        let output_pipeline = pipeline("output", include_str!("shaders/output.wgsl"), &output_layout, OUTPUT_FORMAT, 1, None);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let blank = device
            .create_texture_with_data(
                &queue,
                &wgpu::TextureDescriptor {
                    label: Some("blank"),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: MEDIA_FORMAT,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[0, 0, 0, 0],
            )
            .create_view(&Default::default());

        Ok(Self {
            batching: true,
            display_blending: false,
            device,
            queue,
            layer_pipeline,
            layer_layout,
            composite_pipeline,
            composite_layout,
            effect_pipeline,
            effect_layout,
            lut_pipeline,
            lut_layout,
            output_pipeline,
            output_layout,
            sampler,
            blank,
            targets: Vec::new(),
            text: TextRenderer::new(),
            text_cache: HashMap::new(),
            captions: HashMap::new(),
            nested: HashMap::new(),
            depth: 0,
            nested_left: MAX_NESTED_RENDERS,
            path_cache: HashMap::new(),
            paths: HashMap::new(),
            path_text: HashMap::new(),
            text_pieces: HashMap::new(),
            cutouts: HashMap::new(),
            text_boxes: HashMap::new(),
            images: HashMap::new(),
            luts: HashMap::new(),
            videos: HashMap::new(),
            max_texture,
            gpu_error,
        })
    }

    /// The text box size for a style, in source pixels (no GPU work).
    pub fn measure_text(&mut self, text: &str, style: &TextStyle) -> Vec2 {
        self.text.measure(text, style)
    }

    /// Where carets and selections go in `text` drawn with `style` (no GPU work).
    pub fn text_geometry(&mut self, text: &str, style: &TextStyle) -> crate::text::TextGeometry {
        self.text.geometry(text, style)
    }

    /// The index of a set of work textures `width` × `height`, made if needed. A few sizes are
    /// kept, so nested compositions don't make the frame's textures again every frame.
    fn ensure_targets(&mut self, width: u32, height: u32) -> usize {
        if let Some(index) = self.targets.iter().position(|t| t.width == width && t.height == height) {
            // Most recently used last, so the oldest goes first when there are too many.
            let set = self.targets.remove(index);
            self.targets.push(set);
            return self.targets.len() - 1;
        }
        let make = |label: &str, format, samples, usage| {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            Target { texture, view }
        };
        let work_usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let work = (0..slot::COUNT).map(|_| make("work", WORK_FORMAT, 1, work_usage)).collect();
        let msaa = make("msaa", WORK_FORMAT, MSAA_SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let output = make("output", OUTPUT_FORMAT, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
        // Buffer rows must be a multiple of 256 bytes.
        let padded_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        const KEPT_SIZES: usize = 4;
        if self.targets.len() >= KEPT_SIZES {
            self.targets.remove(0);
        }
        self.targets.push(Targets { width, height, work, msaa, output, readback, padded_row });
        self.targets.len() - 1
    }

    /// Draws simple layers together (the default) or one at a time; both give the same picture.
    pub fn set_batching(&mut self, on: bool) {
        self.batching = on;
    }

    fn uniform(&self, bytes: &[u8]) -> wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("uniform"),
            contents: bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        })
    }

    fn clear(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, color: [f32; 4]) {
        let [r, g, b, a] = color.map(f64::from);
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
    }

    /// Runs a fullscreen pass with `pipeline` and `bind_group` into `target`.
    fn fullscreen(&self, encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::RenderPipeline, bind_group: &wgpu::BindGroup, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("fullscreen"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Draws a layer source into work slot `target`. `rect` is the drawn rectangle in the layer's
    /// source pixels (origin, size); `world` maps source pixels to composition pixels.
    #[allow(clippy::too_many_arguments)]
    fn draw_source(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        target: usize,
        world: Affine,
        scale: f32,
        source: &Source,
        rect: (Vec2, Vec2),
    ) {
        self.draw_sources(encoder, targets, target, scale, &[(world, source, rect)]);
    }

    /// Draws several sources into work slot `target` in one pass, each over the ones before
    /// (a caption's text over its background box).
    fn draw_sources(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, target: usize, scale: f32, sources: &[(Affine, &Source, (Vec2, Vec2))]) {
        let opaque: Vec<(Affine, &Source, (Vec2, Vec2), f32)> = sources.iter().map(|&(world, source, rect)| (world, source, rect, 1.0)).collect();
        self.draw_faded_sources(encoder, targets, target, scale, &opaque);
    }

    /// The same, each source faded to its own opacity (0–1): several whole layers drawn in one
    /// pass (see `render`'s batching).
    fn draw_faded_sources(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, target: usize, scale: f32, sources: &[(Affine, &Source, (Vec2, Vec2), f32)]) {
        let groups: Vec<wgpu::BindGroup> = sources.iter().map(|(world, source, rect, opacity)| self.source_bind_group(targets, *world, scale, source, *rect, *opacity)).collect();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("layer"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &targets.msaa.view,
                depth_slice: None,
                resolve_target: Some(&targets.work[target].view),
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Discard },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.layer_pipeline);
        for group in &groups {
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..6, 0..1);
        }
    }

    /// The uniforms and textures for drawing one source with `layer.wgsl`.
    fn source_bind_group(&self, targets: &Targets, world: Affine, scale: f32, source: &Source, rect: (Vec2, Vec2), opacity: f32) -> wgpu::BindGroup {
        let (width, height) = (targets.width as f32, targets.height as f32);
        let (mode, color, view, cell) = match source {
            Source::Solid(color) => (0.0, *color, &self.blank, 0.0),
            Source::Texture(view) => (1.0, [1.0; 4], *view, 0.0),
            Source::FadedTexture(view, opacity) => (1.0, [*opacity; 4], *view, 0.0),
            Source::TexturePart(view, _, opacity) => (1.0, [*opacity; 4], *view, 0.0),
            Source::Placeholder(color) => (2.0, *color, &self.blank, 40.0),
            Source::Shape(..) => (3.0, [0.0; 4], &self.blank, 0.0),
        };
        let (origin, size) = rect;
        // For shapes: the outline, where its box sits in the drawn rectangle, and the paints.
        let (shape, shape_box, stroke_width, fill, stroke) = match source {
            Source::Shape(style, box_origin, box_size) => (
                [shape_code(style.kind), style.corner_radius, style.points as f32, style.inner_radius],
                [origin.x - box_origin.x, origin.y - box_origin.y, box_size.x, box_size.y],
                style.stroke.map_or(0.0, |stroke| stroke.width),
                paint_uniform(style.fill),
                paint_uniform(style.stroke.map(|stroke| stroke.paint)),
            ),
            // For textures, the part drawn: a slice, or all of it.
            Source::TexturePart(_, part, _) => ([0.0; 4], *part, 0.0, paint_uniform(None), paint_uniform(None)),
            Source::Texture(_) | Source::FadedTexture(..) => ([0.0; 4], [0.0, 0.0, 1.0, 1.0], 0.0, paint_uniform(None), paint_uniform(None)),
            _ => ([0.0; 4], [0.0; 4], 0.0, paint_uniform(None), paint_uniform(None)),
        };
        // Source pixels → composition pixels → output pixels → clip space (y up).
        let to_clip = Affine { a: 2.0 * scale / width, b: 0.0, c: 0.0, d: -2.0 * scale / height, tx: -1.0, ty: 1.0 };
        let matrix = Affine::translate(origin).then(world).then(to_clip);
        let mut uniform: Vec<f32> = vec![
            matrix.a, matrix.b, matrix.c, matrix.d, //
            matrix.tx, matrix.ty, size.x, size.y, //
            color[0], color[1], color[2], color[3], //
            mode, cell, stroke_width, opacity.clamp(0.0, 1.0),
        ];
        uniform.extend(shape);
        uniform.extend(shape_box);
        uniform.extend(fill);
        uniform.extend(stroke);
        let buffer = self.uniform(bytemuck::cast_slice(&uniform));
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layer"),
            layout: &self.layer_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        })
    }
}

impl Renderer {
    /// One effect shader pass from work slot `from` (plus `aux`) into slot `to`.
    #[allow(clippy::too_many_arguments)]
    fn effect_pass(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, id: u32, params: &[f32], time: f32, from: usize, aux: usize, to: usize) {
        let mut uniform = [0u8; 80];
        uniform[..4].copy_from_slice(&id.to_le_bytes());
        // Twelve parameters (unused ones zero), then the output size and the time.
        let mut floats = [0.0f32; 16];
        floats[..params.len().min(12)].copy_from_slice(&params[..params.len().min(12)]);
        floats[12..].copy_from_slice(&[targets.width as f32, targets.height as f32, time, 0.0]);
        uniform[16..].copy_from_slice(bytemuck::cast_slice(&floats));
        let buffer = self.uniform(&uniform);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("effect"),
            layout: &self.effect_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&targets.work[from].view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&targets.work[aux].view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.fullscreen(encoder, &self.effect_pipeline, &bind_group, &targets.work[to].view);
    }

    /// Looks the image in slot `from` up in `lut`, mixing in `intensity` (0–1) of the result, into `to`.
    fn lut_pass(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, lut: &LutTexture, intensity: f32, from: usize, to: usize) {
        let [a, b, c] = lut.domain_min;
        let [d, e, f] = lut.domain_max;
        let floats: [f32; 12] = [intensity, lut.size as f32, 0.0, 0.0, a, b, c, 0.0, d, e, f, 0.0];
        let buffer = self.uniform(bytemuck::cast_slice(&floats));
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lut"),
            layout: &self.lut_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&targets.work[from].view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&lut.view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.fullscreen(encoder, &self.lut_pipeline, &bind_group, &targets.work[to].view);
    }

    /// A two-pass Gaussian blur from `from` into `to`, using `spare` in between.
    fn blur(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, sigma: f32, from: usize, spare: usize, to: usize) {
        self.effect_pass(encoder, targets, 1, &[sigma, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0.0, from, from, spare);
        self.effect_pass(encoder, targets, 1, &[sigma, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0.0, spare, spare, to);
    }

    /// Runs one pass of `effect` from slot `from` into `to`, using the aux slots as scratch
    /// (and `to` itself between a glow's or shadow's steps). Returns false when the pass draws
    /// nothing, such as a LUT not chosen yet; `to` is then left as it was.
    #[allow(clippy::too_many_arguments)]
    fn run_pass(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, effect: &studio_core::model::Effect, pass: Pass, time: f32, from: usize, to: usize) -> bool {
        match pass {
            Pass::Shader { id, params } => self.effect_pass(encoder, targets, id, &params, time, from, from, to),
            Pass::Blur { sigma } => self.blur(encoder, targets, sigma, from, slot::AUX_A, to),
            Pass::Glow { threshold, sigma, intensity } => {
                self.effect_pass(encoder, targets, 30, &[threshold, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], time, from, from, slot::AUX_A);
                self.blur(encoder, targets, sigma, slot::AUX_A, to, slot::AUX_B);
                self.effect_pass(encoder, targets, 31, &[intensity, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], time, from, slot::AUX_B, to);
            }
            Pass::Unsharp { amount, sigma, threshold } => {
                // Blur into AUX_B (using `to` in between), then combine it with the original.
                self.blur(encoder, targets, sigma, from, to, slot::AUX_B);
                self.effect_pass(encoder, targets, 91, &[amount, threshold], time, from, slot::AUX_B, to);
            }
            Pass::Lut { intensity } => {
                let loaded = effect.lut.as_ref().and_then(|source| self.luts.get(source)).and_then(|(_, lut)| lut.as_ref());
                let Some(lut) = loaded.filter(|_| intensity > 0.0) else { return false };
                self.lut_pass(encoder, targets, lut, intensity, from, to);
            }
            Pass::Shadow { offset, opacity, sigma } => {
                self.effect_pass(encoder, targets, 32, &[offset[0], offset[1], opacity, 0.0, 0.0, 0.0, 0.0, 0.0], time, from, from, slot::AUX_A);
                self.blur(encoder, targets, sigma, slot::AUX_A, to, slot::AUX_B);
                self.effect_pass(encoder, targets, 33, &[0.0; 8], time, from, slot::AUX_B, to);
            }
        }
        true
    }

    /// Runs `layer`'s effects on the image in slot `image`, ping-ponging with `other` and using
    /// the aux slots as scratch. Returns the slot that holds the result. A layer edited as
    /// nodes runs its graph instead (see `apply_graph`); `world` places its object matte.
    #[allow(clippy::too_many_arguments)]
    fn apply_effects(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, layer: &Layer, world: Affine, frame: f64, time: f32, scale: f32, mut image: usize, mut other: usize) -> usize {
        if !layer.switches.effects_enabled {
            return image;
        }
        if layer.effect_graph.wired {
            return self.apply_graph(encoder, targets, layer, world, frame, time, scale, image, other);
        }
        let layer_box = layer_box(layer, world, scale);
        for effect in layer.effects.iter().filter(|effect| effect.enabled) {
            let Some(passes) = effects::passes(effect, frame, scale, layer_box) else { continue };
            for pass in passes {
                if self.run_pass(encoder, targets, effect, pass, time, image, other) {
                    std::mem::swap(&mut image, &mut other);
                }
            }
        }
        image
    }

    /// Runs a layer's node graph (see `studio_core::effect_graph::plan`) on the picture in slot
    /// `image`. Its registers live in the graph slots after the fixed ones (made beforehand by
    /// `ensure_graph_slots`); `other` is free meanwhile and receives the result. Returns the slot
    /// holding what the layer shows: transparent when nothing feeds the Output, and the
    /// unchanged picture for a graph with a loop in it.
    #[allow(clippy::too_many_arguments)]
    fn apply_graph(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, layer: &Layer, world: Affine, frame: f64, time: f32, scale: f32, image: usize, other: usize) -> usize {
        let Some(plan) = effect_graph::plan(layer) else { return image };
        if targets.work.len() < slot::COUNT + plan.registers.saturating_sub(1) {
            return image; // Not expected: slots are made for every plan before drawing.
        }
        let register = |r: effect_graph::Register| if r == 0 { image } else { slot::COUNT + r - 1 };
        let layer_box = layer_box(layer, world, scale);
        for step in &plan.steps {
            match *step {
                effect_graph::Step::Clear { out } => self.clear(encoder, &targets.work[register(out)].view, [0.0; 4]),
                effect_graph::Step::Matte { out } => self.draw_matte(encoder, targets, layer, world, scale, register(out)),
                effect_graph::Step::Effect { effect, input, mask, scratch, out } => {
                    let effect = &layer.effects[effect];
                    let target = register(scratch.unwrap_or(out));
                    // Steps after the first ping-pong between the target and `other`.
                    let mut current = register(input);
                    for pass in effects::passes(effect, frame, scale, layer_box).unwrap_or_default() {
                        let next = if current == target { other } else { target };
                        if self.run_pass(encoder, targets, effect, pass, time, current, next) {
                            current = next;
                        }
                    }
                    if current != target {
                        self.effect_pass(encoder, targets, 0, &[0.0; 8], time, current, current, target);
                    }
                    if let (Some(mask), Some(scratch)) = (mask, scratch) {
                        // The effect's result where the mask covers, the input elsewhere.
                        self.composite(encoder, targets, register(input), register(scratch), Some(register(mask)), [0, 1, 1], 1.0, register(out));
                    }
                }
                effect_graph::Step::Merge { background, foreground, mask, blend, opacity, out } => {
                    let matte_mode = if mask.is_some() { 1 } else { 0 };
                    self.composite(encoder, targets, register(background), register(foreground), mask.map(register), [blend as u32, matte_mode, 0], opacity, register(out));
                }
            }
        }
        match plan.output {
            Some(0) => image,
            Some(result) => {
                self.effect_pass(encoder, targets, 0, &[0.0; 8], time, register(result), register(result), other);
                other
            }
            None => {
                self.clear(encoder, &targets.work[other].view, [0.0; 4]);
                other
            }
        }
    }

    /// Draws `layer`'s object matte into slot `to`, stretched over the layer's box and with its
    /// edge finished: white where the object is, transparent elsewhere (and everywhere while no
    /// matte has been made). Uses the aux slots as scratch.
    fn draw_matte(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, layer: &Layer, world: Affine, scale: f32, to: usize) {
        let (Some((_, mask)), Some(cutout)) = (self.cutouts.get(&layer.id), layer.cutout.as_ref()) else {
            self.clear(encoder, &targets.work[to].view, [0.0; 4]);
            return;
        };
        let fit = Vec2::new(layer.size.x / mask.size.x.max(1.0), layer.size.y / mask.size.y.max(1.0));
        self.draw_source(encoder, targets, to, Affine::scale(fit).then(world), scale, &Source::Texture(&mask.view), (Vec2::default(), mask.size));
        self.finish_matte_edge(encoder, targets, &cutout.edges, world, scale, to);
    }

    /// Feathers, shifts and hardens the matte in `slot` (white with its coverage as alpha) by
    /// `edges`, in place, using the aux slots as scratch. Settings at their defaults do nothing.
    fn finish_matte_edge(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, edges: &studio_core::model::MatteEdges, world: Affine, scale: f32, slot: usize) {
        let (contrast, shift) = (edges.contrast / 100.0, edges.shift_edge / 100.0);
        if edges.feather <= 0.0 && contrast <= 0.0 && shift == 0.0 {
            return;
        }
        // Source pixels to output pixels: the layer's own scale (its x axis) times the output's.
        let layer_scale = world.apply_vector(Vec2::new(1.0, 0.0)).length().max(0.01);
        let feather = edges.feather * layer_scale * scale;
        let reach = shift.abs() * SHIFT_EDGE_REACH * scale.max(0.25);
        // A hard edge must be softened before it can be moved: the blur spreads it over the
        // distance it moves, and the edge pass then puts it at the new place.
        let sigma = (feather + reach) * 0.5;
        if sigma > 0.01 {
            self.blur(encoder, targets, sigma, slot, slot::AUX_A, slot::AUX_B);
        } else {
            self.effect_pass(encoder, targets, 0, &[0.0; 8], 0.0, slot, slot, slot::AUX_B);
        }
        // Shifting out (positive) lowers the level the edge sits at, so more of the soft edge counts.
        let center = 0.5 - shift * 0.45;
        let width = 1.0 - contrast * 0.98;
        self.effect_pass(encoder, targets, MATTE_EDGE_PASS, &[center, width, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0.0, slot::AUX_B, slot::AUX_B, slot);
    }

    /// Makes sure the work textures at `index` include enough graph slots for every node graph
    /// in `composition`, adding full-size textures as needed (they are kept for later frames).
    fn ensure_graph_slots(&mut self, index: usize, composition: &studio_core::model::Composition) {
        let needed = composition.layers.iter().filter_map(effect_graph::plan).map(|plan| plan.registers.saturating_sub(1)).max().unwrap_or(0);
        let (width, height, have) = {
            let targets = &self.targets[index];
            (targets.width, targets.height, targets.work.len())
        };
        for _ in have..slot::COUNT + needed {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("graph"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: WORK_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.targets[index].work.push(Target { texture, view });
        }
    }

    /// Composites `layer_slot` over the backdrop in `from`, writing `to`.
    #[allow(clippy::too_many_arguments)]
    fn composite(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, from: usize, layer_slot: usize, matte: Option<usize>, modes: [u32; 3], opacity: f32, to: usize) {
        let mut uniform = [0u8; 32];
        for (index, mode) in modes.iter().enumerate() {
            uniform[index * 4..index * 4 + 4].copy_from_slice(&mode.to_le_bytes());
        }
        uniform[12..16].copy_from_slice(&u32::from(self.display_blending).to_le_bytes());
        uniform[16..20].copy_from_slice(&opacity.to_le_bytes());
        let buffer = self.uniform(&uniform);
        let matte_view = matte.map_or(&self.blank, |slot| &targets.work[slot].view);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("composite"),
            layout: &self.composite_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&targets.work[from].view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&targets.work[layer_slot].view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(matte_view) },
            ],
        });
        self.fullscreen(encoder, &self.composite_pipeline, &bind_group, &targets.work[to].view);
    }

    /// Draws a layer that shapes others (a mask or a track matte) into a matte slot, with its
    /// effects and faded by its opacity, and returns the slot. It starts from an empty matte, so
    /// a layer that draws nothing (one that has ended) never leaves an earlier matte behind.
    fn draw_shaper(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, frame: &Shapers<'_>, shaper: &Layer) -> Option<usize> {
        let world = world_transform(frame.composition, shaper.id, frame.at)?;
        self.clear(encoder, &targets.work[slot::MATTE_A].view, [0.0; 4]);
        if shaper.is_active_at(frame.frame) {
            self.draw_layer(encoder, targets, frame.project, shaper, world, frame.scale, frame.text_keys, slot::MATTE_A);
        }
        let drawn = self.apply_effects(encoder, targets, shaper, world, frame.at, frame.time, frame.scale, slot::MATTE_A, slot::MATTE_B);
        Some(self.fade_matte(encoder, targets, drawn, opacity_at(shaper, frame.at)))
    }

    /// `layer`'s mask layer drawn into a matte slot, and 1 if it hides what it covers (0 if it
    /// shows it). An empty mask (one that has ended) shows nothing inside, so Show hides all.
    fn draw_mask(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, frame: &Shapers<'_>, layer: &Layer) -> Option<(usize, f32)> {
        let mask = layer.mask?;
        let drawn = self.draw_shaper(encoder, targets, frame, frame.composition.layer(mask.layer)?)?;
        Some((drawn, if mask.mode == studio_core::model::MaskMode::Hide { 1.0 } else { 0.0 }))
    }

    /// The track matte for the layer at stack position `index` (the layer above it), drawn into
    /// a matte slot, if it uses one.
    fn draw_track_matte(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, frame: &Shapers<'_>, index: usize, layer: &Layer) -> Option<usize> {
        if layer.track_matte == studio_core::model::TrackMatte::None {
            return None;
        }
        let above = frame.composition.layers.get(index.checked_sub(1)?)?;
        self.draw_shaper(encoder, targets, frame, above)
    }

    /// Where an adjustment layer applies: its box, narrowed by its mask and track matte like any
    /// layer. `adjusted` holds the adjusted picture (a layer slot) and is left alone. Returns the
    /// slot holding the coverage.
    #[allow(clippy::too_many_arguments)]
    fn adjustment_coverage(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, frame: &Shapers<'_>, index: usize, layer: &Layer, world: Affine, adjusted: usize) -> usize {
        // The coverage waits in the other layer slot: drawing a mask or matte uses the matte
        // slots, and effects on it use the aux slots.
        let coverage = if adjusted == slot::LAYER_A { slot::LAYER_B } else { slot::LAYER_A };
        let other_matte = |drawn: usize| if drawn == slot::MATTE_A { slot::MATTE_B } else { slot::MATTE_A };
        self.draw_source(encoder, targets, coverage, world, frame.scale, &Source::Solid([1.0; 4]), (Vec2::default(), layer.size));
        if let Some((drawn, hide)) = self.draw_mask(encoder, targets, frame, layer) {
            let masked = other_matte(drawn);
            self.effect_pass(encoder, targets, MASK_PASS, &[hide, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], frame.time, coverage, drawn, masked);
            // Back to the layer slot (effect 0 copies), clear of the track matte drawn next.
            self.effect_pass(encoder, targets, 0, &[0.0; 8], frame.time, masked, masked, coverage);
        }
        let Some(matte) = self.draw_track_matte(encoder, targets, frame, index, layer) else { return coverage };
        // The coverage laid over nothing through the matte: the coverage times the matte.
        let narrowed = other_matte(matte);
        self.clear(encoder, &targets.work[slot::AUX_A].view, [0.0; 4]);
        self.composite(encoder, targets, slot::AUX_A, coverage, Some(matte), [0, layer.track_matte as u32, 0], 1.0, narrowed);
        narrowed
    }

    /// A matte or mask drawn in `drawn` (one of the matte slots), faded by its layer's `opacity`
    /// as After Effects does. Returns the slot holding the result.
    fn fade_matte(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, drawn: usize, opacity: f32) -> usize {
        if opacity >= 1.0 {
            return drawn;
        }
        // Laid at `opacity` over nothing: the same picture with its coverage scaled.
        let faded = if drawn == slot::MATTE_A { slot::MATTE_B } else { slot::MATTE_A };
        self.clear(encoder, &targets.work[slot::AUX_A].view, [0.0; 4]);
        self.composite(encoder, targets, slot::AUX_A, drawn, None, [0, 0, 0], opacity, faded);
        faded
    }

    /// Makes sure the text raster for a layer exists at `raster_scale`, returning its cache key,
    /// its box in source pixels and the raster's padding in source pixels.
    fn prepare_text(&mut self, text: &str, style: &TextStyle, raster_scale: f32) -> (u64, Vec2, Vec2) {
        self.prepare_highlighted_text(text, style, None, raster_scale)
    }

    /// `prepare_text`, with a byte range of the text in another color.
    fn prepare_highlighted_text(&mut self, text: &str, style: &TextStyle, highlight: Option<(std::ops::Range<usize>, Rgba)>, raster_scale: f32) -> (u64, Vec2, Vec2) {
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            text_key(text, style, raster_scale).hash(&mut hasher);
            if let Some((range, color)) = &highlight {
                (range.start, range.end, color.to_rgba_u32()).hash(&mut hasher);
            }
            hasher.finish()
        };
        // The cache is trimmed after all of a frame's text is prepared (see `render`), never
        // here: a raster prepared earlier in this frame must still be there to draw.
        if !self.text_cache.contains_key(&key) {
            let image = self.text.rasterize_highlighted(text, style, highlight, raster_scale, self.max_texture);
            let view = self.upload(image.width, image.height, &image.pixels);
            // Sized by the scale actually drawn at, which can be lower than the one asked for.
            let drawn = Vec2::new(image.width as f32 / image.scale, image.height as f32 / image.scale);
            self.text_cache.insert(key, MediaTexture { view, size: drawn });
            let measured = image.size;
            // Remember the box with the raster, so it is not measured again.
            self.text_boxes.insert(key, measured);
        }
        let drawn = self.text_cache[&key].size;
        let measured = self.text_boxes[&key];
        // The raster is padded equally on all sides (for stroke); find that padding.
        let pad = (drawn - measured) * 0.5;
        (key, measured, pad)
    }

    /// Uploads straight-alpha sRGB pixels as a texture holding premultiplied colors (see
    /// `premultiply`), so the GPU's filtering blends edges correctly.
    fn upload(&self, width: u32, height: u32, pixels: &[u8]) -> wgpu::TextureView {
        let pixels = &premultiply(pixels);
        self.device
            .create_texture_with_data(
                &self.queue,
                &wgpu::TextureDescriptor {
                    label: Some("media"),
                    size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: MEDIA_FORMAT,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                pixels,
            )
            .create_view(&Default::default())
    }

    /// Uploads a footage layer's decoded frame, reusing its texture when the size is unchanged.
    /// Most video is opaque and needs no premultiplying; footage with transparency is
    /// premultiplied like images.
    fn prepare_video(&mut self, layer: Id, picture: &VideoFrame) {
        let size = Vec2::new(picture.width as f32, picture.height as f32);
        let reusable = self.videos.get(&layer).is_some_and(|video| video.texture.size == size);
        if reusable && self.videos[&layer].index == picture.index {
            return;
        }
        if !reusable {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("video"),
                size: wgpu::Extent3d { width: picture.width, height: picture.height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: MEDIA_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.videos.insert(layer, VideoTexture { texture: MediaTexture { view, size }, raw: texture, index: -1 });
        }
        let premultiplied;
        let pixels = if picture.has_alpha {
            premultiplied = premultiply(&picture.rgba);
            &premultiplied
        } else {
            &picture.rgba
        };
        let video = self.videos.get_mut(&layer).expect("inserted above");
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &video.raw, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(picture.width * 4), rows_per_image: Some(picture.height) },
            wgpu::Extent3d { width: picture.width, height: picture.height, depth_or_array_layers: 1 },
        );
        video.index = picture.index;
    }

    /// Loads the background-removal mask a cut-out layer uses at source frame `index`, unless it
    /// is already loaded. A mask that cannot be read (its cache was cleared) is left out, so the
    /// whole picture shows rather than none of it.
    fn prepare_cutout(&mut self, layer: &Layer, index: i64) {
        let Some(path) = layer.cutout.as_ref().and_then(|cutout| cutout.mask_path(index)) else {
            self.cutouts.remove(&layer.id);
            return;
        };
        if self.cutouts.get(&layer.id).is_some_and(|(loaded, _)| *loaded == path) {
            return;
        }
        match image::open(&path).map(|mask| mask.to_luma8()) {
            Ok(mask) => {
                let (width, height) = mask.dimensions();
                // White, as see-through as the mask says: only its alpha is used.
                let rgba: Vec<u8> = mask.as_raw().iter().flat_map(|&kept| [255, 255, 255, kept]).collect();
                let texture = MediaTexture { view: self.upload(width, height, &rgba), size: Vec2::new(width as f32, height as f32) };
                self.cutouts.insert(layer.id, (path, texture));
            }
            Err(_) => {
                self.cutouts.remove(&layer.id);
            }
        }
    }

    /// Makes sure the lookup table `source` is on the GPU: a built-in look is made once; a `.cube`
    /// file is read again when it changes on disk. Tables that can't be read are remembered (and
    /// draw nothing) until their file changes.
    fn prepare_lut(&mut self, source: &LutSource) {
        let modified = match source {
            LutSource::Look(_) => None,
            LutSource::File(path) => std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok(),
        };
        if self.luts.get(source).is_some_and(|(loaded_at, _)| *loaded_at == modified) {
            return;
        }
        let table = match source {
            LutSource::Look(name) => studio_core::grade::look(name),
            LutSource::File(path) => read_cube(path).ok(),
        };
        let texture = table.map(|table| self.upload_lut(&table));
        self.luts.insert(source.clone(), (modified, texture));
    }

    /// Uploads a lookup table as a 16-bit float 3D texture (red along x, green along y, blue along z).
    fn upload_lut(&self, table: &studio_core::grade::Lut) -> LutTexture {
        let size = table.size as u32;
        let mut texels: Vec<u16> = Vec::with_capacity(table.table.len() * 4);
        for [r, g, b] in &table.table {
            texels.extend([*r, *g, *b, 1.0].map(|value| half::f16::from_f32(value).to_bits()));
        }
        let view = self
            .device
            .create_texture_with_data(
                &self.queue,
                &wgpu::TextureDescriptor {
                    label: Some("lut"),
                    size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: size },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D3,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                bytemuck::cast_slice(&texels),
            )
            .create_view(&Default::default());
        LutTexture { view, size, domain_min: table.domain_min, domain_max: table.domain_max }
    }

    /// Loads an image file into a texture once. Failed loads are remembered so they are not retried every frame.
    fn prepare_image(&mut self, path: &PathBuf) -> bool {
        let modified = std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok();
        if self.images.get(path).is_none_or(|(loaded_at, _)| *loaded_at != modified) {
            let max = self.max_texture;
            // An SVG used as one picture layer is drawn from its file; the layer stretches the
            // drawing over its box, so it needn't be the SVG's stated size.
            let is_svg = path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("svg"));
            let loaded = if is_svg {
                crate::svg::rasterize(path, max).map(|image| MediaTexture { view: self.upload(image.width, image.height, &image.pixels), size: Vec2::new(image.width as f32, image.height as f32) })
            } else {
                image::open(path).ok().map(|image| {
                    // Larger than the GPU accepts: scale down to fit (the layer stretches it back).
                    let image = if image.width() > max || image.height() > max { image.resize(max, max, image::imageops::FilterType::Triangle) } else { image };
                    let rgba = image.to_rgba8();
                    let (width, height) = rgba.dimensions();
                    MediaTexture { view: self.upload(width, height, rgba.as_raw()), size: Vec2::new(width as f32, height as f32) }
                })
            };
            self.images.insert(path.clone(), (modified, loaded));
        }
        self.images[path].1.is_some()
    }

    /// Renders composition `comp` of `project` at `frame` into a `width` × `height` image.
    /// The composition is scaled to fit the output width. Footage frames come from `video`.
    pub fn render(
        &mut self,
        project: &Project,
        comp: Id,
        frame: i64,
        width: u32,
        height: u32,
        video: &dyn VideoSource,
    ) -> Result<RenderedFrame, RenderError> {
        let composition = project.composition(comp).ok_or_else(|| RenderError("the composition does not exist".into()))?;
        // Too large a frame is drawn smaller, keeping its shape (clamping each side alone would
        // squash it).
        let fit = (MAX_SIDE as f32 / width.max(height) as f32).min(1.0);
        let (width, height) = (((width as f32 * fit).round() as u32).clamp(1, MAX_SIDE), ((height as f32 * fit).round() as u32).clamp(1, MAX_SIDE));
        if self.depth == 0 {
            self.nested_left = MAX_NESTED_RENDERS;
        }
        let scale = width as f32 / composition.width as f32;
        let at = frame as f64;
        let at_time = at;
        let mut used_proxy_nested = false;
        let time = (frame as f64 / composition.frame_rate.fps()) as f32;

        // A layer with a track matte uses the layer above it as the matte; that layer is not drawn itself.
        let matte_sources: HashSet<Id> = composition
            .layers
            .windows(2)
            .filter(|pair| pair[1].track_matte != studio_core::model::TrackMatte::None)
            .map(|pair| pair[0].id)
            .collect();
        // Mask layers only shape the layers they mask; they are not drawn themselves.
        let mask_sources: HashSet<Id> = composition.layers.iter().filter_map(|layer| layer.mask).map(|mask| mask.layer).collect();

        // Nested compositions first: rendering one reuses the renderer's own buffers, so it must
        // happen before anything for this composition is prepared. Hidden ones are still needed
        // when they serve as a matte or mask, as any hidden layer can.
        let mut nested = HashMap::new();
        if self.depth < MAX_NESTING {
            let needed = |layer: &&Layer| layer.is_active_at(frame) && (layer.switches.visible || matte_sources.contains(&layer.id) || mask_sources.contains(&layer.id));
            for layer in composition.layers.iter().filter(needed) {
                let LayerContent::Composition { composition: inner_id } = layer.content else { continue };
                let Some(inner) = project.composition(inner_id) else { continue };
                // Its own time: from where the layer starts, at its own frame rate.
                let seconds = (frame - layer.start_frame) as f64 / composition.frame_rate.fps();
                let inner_frame = (seconds * inner.frame_rate.fps() + 1e-6).floor() as i64;
                if !(0..inner.duration_frames).contains(&inner_frame) {
                    continue;
                }
                // Drawn at the size it appears on screen, so it stays sharp when scaled up.
                let on_screen = world_transform(composition, layer.id, at_time).map_or(scale, |w| max_scale(w) * scale).clamp(0.01, 8.0);
                let (inner_width, inner_height) = (((inner.width as f32 * on_screen).round() as u32).clamp(1, MAX_SIDE), ((inner.height as f32 * on_screen).round() as u32).clamp(1, MAX_SIDE));
                if self.nested_left == 0 {
                    continue;
                }
                self.nested_left -= 1;
                self.depth += 1;
                let rendered = self.render(project, inner_id, inner_frame, inner_width, inner_height, video);
                self.depth -= 1;
                if let Ok(picture) = rendered {
                    used_proxy_nested |= picture.used_proxy;
                    let view = self.upload(picture.width, picture.height, &swap_bgra_rgba(&picture.bgra));
                    nested.insert(layer.id, MediaTexture { view, size: Vec2::new(picture.width as f32, picture.height as f32) });
                }
            }
        }
        self.nested = nested;

        // Prepare text rasters and images first (this needs `&mut self`), noting text box sizes.
        let mut text_sizes = Vec::new();
        let mut used_proxy = used_proxy_nested;
        let mut text_keys: HashMap<Id, (u64, Vec2, Vec2)> = HashMap::new();
        self.captions.clear();
        self.paths.clear();
        self.path_text.clear();
        self.text_pieces.clear();
        for layer in composition.layers.iter().filter(|layer| layer.is_active_at(frame)) {
            for source in layer.effects.iter().filter(|effect| effect.enabled).filter_map(|effect| effect.lut.as_ref()) {
                self.prepare_lut(source);
            }
            match &layer.content {
                LayerContent::Shape { path: Some(_), .. } => {
                    // The outline and style at this moment: a morphing shape changes them over time.
                    let (Some(path), Some(style)) = (layer.shape_path_at(at_time), layer.shape_style_at(at_time)) else { continue };
                    let (path, style) = (path.as_ref(), &style);
                    let raster_scale = world_transform(composition, layer.id, at_time).map_or(scale, |w| max_scale(w) * scale).max(0.05);
                    let key = path_key(path, style, layer.size, raster_scale);
                    if !self.path_cache.contains_key(&key)
                        && let Some(image) = crate::path::rasterize(path, style, layer.size, raster_scale, self.max_texture)
                    {
                        let view = self.upload(image.width, image.height, &image.pixels);
                        self.path_cache.insert(key, (MediaTexture { view, size: image.size }, image.origin));
                    }
                    if self.path_cache.contains_key(&key) {
                        self.paths.insert(layer.id, key);
                    }
                }
                LayerContent::Captions { words, style } => {
                    let fps = composition.frame_rate.fps();
                    let Some(shown) = studio_core::captions::frame_at(words, style, frame - layer.start_frame, fps) else { continue };
                    let text = shown.text();
                    // The spoken word's place in the line, to color it.
                    let mut at = 0;
                    let mut highlight = None;
                    for (word, spoken) in &shown.words {
                        if *spoken {
                            highlight = Some((at..at + word.len(), style.highlight));
                        }
                        at += word.len() + 1;
                    }
                    let raster_scale = world_transform(composition, layer.id, at_time).map_or(scale, |w| max_scale(w) * scale) * shown.scale;
                    let prepared = self.prepare_highlighted_text(&text, &style.text, highlight, raster_scale.max(0.05));
                    text_keys.insert(layer.id, prepared);
                    self.captions.insert(layer.id, CaptionDraw { opacity: shown.opacity, scale: shown.scale, background: style.background });
                }
                LayerContent::Text { text, style } => {
                    let raster_scale = world_transform(composition, layer.id, at_time).map_or(scale, |w| max_scale(w) * scale);
                    let prepared = self.prepare_text(text, style, raster_scale.max(0.05));
                    text_sizes.push((layer.id, prepared.1));
                    text_keys.insert(layer.id, prepared);
                    if let Some(on) = layer.text_path
                        && let Some(path_layer) = composition.layer(on.layer)
                        && let Some(path) = path_layer.shape_path_at(at_time)
                        && let Some(path_world) = world_transform(composition, on.layer, at_time)
                    {
                        let size = path_layer.size;
                        if let Some(walk) = studio_core::path::Walk::new(&path, |v| path_world.apply(Vec2::new(v.x * size.x, v.y * size.y))) {
                            let (letters, baseline) = self.text.letters(text, style);
                            // The letters' size: the text layer's own scale.
                            let letter_scale = layer.transform.as_ref().map_or(1.0, |transform| transform.scale.value_at(at_time).x / 100.0);
                            self.path_text.insert(layer.id, PathText { letters, baseline, walk, start: on.start, letter_scale });
                        }
                    } else if let Some(poses) = studio_core::text_animation::layer_poses(layer, text, style.font_size, at_time, composition.frame_rate) {
                        let pieces = self.text_pieces(text, style, &poses, prepared.1, prepared.2);
                        self.text_pieces.insert(layer.id, pieces);
                    }
                }
                LayerContent::Image { item } => {
                    if let Some(path) = project.item(*item).and_then(|item| item.path.clone()) {
                        self.prepare_image(&path);
                    }
                    self.prepare_cutout(layer, 0);
                }
                LayerContent::Footage { item } => {
                    let source = project.item(*item).and_then(|source| source.path.clone().zip(source.frame_rate));
                    if let Some((path, rate)) = source {
                        let index = layer.source_frame(frame, composition.frame_rate, rate);
                        // A picture larger than the GPU accepts is left out (the layer shows its
                        // placeholder); decoders keep frames within this size.
                        if let Some(picture) = video.frame(&path, index).filter(|picture| picture.width.max(picture.height) <= self.max_texture) {
                            used_proxy |= picture.from_proxy;
                            self.prepare_video(layer.id, &picture);
                        }
                        self.prepare_cutout(layer, index);
                    }
                }
                _ => {}
            }
        }

        // Keep caches bounded: text rasters beyond the limit and pictures of footage layers not
        // on screen are let go, but never anything this frame uses.
        if self.text_cache.len() > TEXT_CACHE_LIMIT {
            let in_use: HashSet<u64> = text_keys.values().map(|(key, ..)| *key).collect();
            self.text_cache.retain(|key, _| in_use.contains(key));
            self.text_boxes.retain(|key, _| in_use.contains(key));
        }
        if self.path_cache.len() > PATH_CACHE_LIMIT {
            let in_use: HashSet<u64> = self.paths.values().copied().collect();
            self.path_cache.retain(|key, _| in_use.contains(key));
        }
        let on_screen: HashSet<Id> = composition.layers.iter().filter(|layer| layer.is_active_at(frame)).map(|layer| layer.id).collect();
        self.videos.retain(|layer, _| on_screen.contains(layer));
        self.cutouts.retain(|layer, _| on_screen.contains(layer));
        // Images stay loaded while the project has them (reloading a still is slow), and are let
        // go once removed from it.
        let in_project: HashSet<&PathBuf> = project.items.iter().filter_map(|item| item.path.as_ref()).collect();
        self.images.retain(|path, _| in_project.contains(path));
        // Lookup tables likewise, while an effect in the project uses them.
        let luts_used: HashSet<&LutSource> = project.compositions.iter().flat_map(|comp| &comp.layers).flat_map(|layer| &layer.effects).filter_map(|effect| effect.lut.as_ref()).collect();
        self.luts.retain(|source, _| luts_used.contains(source));

        self.display_blending = !composition.linear_blending;
        let index = self.ensure_targets(width, height);
        self.ensure_graph_slots(index, composition);
        let targets = &self.targets[index];
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        let (mut accum, mut spare) = (slot::ACCUM_A, slot::ACCUM_B);
        self.clear(&mut encoder, &targets.work[accum].view, linear(composition.background));

        let any_solo = composition.layers.iter().any(|layer| layer.switches.solo);

        // Layers that only need laying over what is below (normal blending, no effects, matte,
        // mask or cut-out, drawn in one piece) are gathered and drawn together in one pass, then
        // laid over the picture once: "over" gives the same result whichever way it is grouped,
        // and big compositions of simple layers skip two full-frame passes per layer.
        let mut batch: Vec<(Affine, Source<'_>, (Vec2, Vec2), f32)> = Vec::new();
        let flush = |batch: &mut Vec<(Affine, Source<'_>, (Vec2, Vec2), f32)>, encoder: &mut wgpu::CommandEncoder, accum: &mut usize, spare: &mut usize| {
            if batch.is_empty() {
                return;
            }
            let drawn: Vec<(Affine, &Source, (Vec2, Vec2), f32)> = batch.iter().map(|(world, source, rect, opacity)| (*world, source, *rect, *opacity)).collect();
            self.draw_faded_sources(encoder, targets, slot::LAYER_A, scale, &drawn);
            self.composite(encoder, targets, *accum, slot::LAYER_A, None, [0, 0, 0], 1.0, *spare);
            std::mem::swap(accum, spare);
            batch.clear();
        };
        for (index, layer) in composition.layers.iter().enumerate().rev() {
            let shown = layer.switches.visible && layer.is_active_at(frame) && layer.content.has_picture() && (!any_solo || layer.switches.solo);
            if !shown || matte_sources.contains(&layer.id) || mask_sources.contains(&layer.id) {
                continue;
            }
            let Some(world) = world_transform(composition, layer.id, at) else { continue };
            let opacity = opacity_at(layer, at);

            let has_effects = layer.switches.effects_enabled && (layer.effect_graph.wired || effect_graph::picture_effects(layer).any(|effect| effect.enabled));
            let cut_out = layer.cutout.is_some() && self.cutouts.contains_key(&layer.id) && !effect_graph::uses_matte(layer);
            // A layer moving while the shutter is open is drawn several times and averaged, so
            // it can't join a batch; a still one (or one with its switch off) is drawn as usual.
            let shutter = studio_core::motion_blur::shutter_transforms(composition, layer, at);
            // (Adjustment layers change what is below them, so they are never simple.)
            let simple = !matches!(layer.content, LayerContent::Adjustment)
                && layer.blend_mode == studio_core::model::BlendMode::Normal
                && layer.track_matte == studio_core::model::TrackMatte::None
                && layer.mask.is_none()
                && !has_effects
                && !cut_out
                && shutter.is_none();
            // Batched layers are laid over each other in linear light, so a composition that
            // blends in display colors composites every layer on its own.
            if simple && self.batching && !self.display_blending && let Some(single) = self.single_source(project, layer, world, &text_keys) {
                if let Some((world, source, rect)) = single {
                    batch.push((world, source, rect, opacity));
                }
                continue;
            }
            flush(&mut batch, &mut encoder, &mut accum, &mut spare);

            let shapers = Shapers { project, composition, frame, at, time, scale, text_keys: &text_keys };
            if matches!(layer.content, LayerContent::Adjustment) {
                // Effects run on a copy of everything below, then blend back in where the layer covers.
                self.effect_pass(&mut encoder, targets, 0, &[0.0; 8], time, accum, accum, slot::LAYER_A);
                let adjusted = self.apply_effects(&mut encoder, targets, layer, world, at, time, scale, slot::LAYER_A, slot::LAYER_B);
                let coverage = self.adjustment_coverage(&mut encoder, targets, &shapers, index, layer, world, adjusted);
                self.composite(&mut encoder, targets, accum, adjusted, Some(coverage), [0, 0, 1], opacity, spare);
                std::mem::swap(&mut accum, &mut spare);
                continue;
            }

            match &shutter {
                Some(worlds) => self.draw_motion_blurred_layer(&mut encoder, targets, project, layer, worlds, scale, &text_keys, slot::LAYER_A),
                None => self.draw_layer(&mut encoder, targets, project, layer, world, scale, &text_keys, slot::LAYER_A),
            }
            // Background removal comes first, so effects (a glow, a shadow) follow the cut-out.
            // A node graph that uses the object matte itself gets the whole picture instead.
            let cutout = layer.cutout.as_ref().filter(|_| !effect_graph::uses_matte(layer));
            let (drawn, spare_layer) = match (self.cutouts.get(&layer.id), cutout) {
                (Some(_), Some(cutout)) => {
                    // The mask covers the whole picture, so it is stretched over the layer's box.
                    self.draw_matte(&mut encoder, targets, layer, world, scale, slot::MATTE_A);
                    let remove = if cutout.remove { 1.0 } else { 0.0 };
                    self.effect_pass(&mut encoder, targets, MASK_PASS, &[remove, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], time, slot::LAYER_A, slot::MATTE_A, slot::LAYER_B);
                    (slot::LAYER_B, slot::LAYER_A)
                }
                _ => (slot::LAYER_A, slot::LAYER_B),
            };
            let mut image = self.apply_effects(&mut encoder, targets, layer, world, at, time, scale, drawn, spare_layer);

            // A mask layer keeps the image only inside (or only outside) what the mask draws.
            if let Some((drawn, hide)) = self.draw_mask(&mut encoder, targets, &shapers, layer) {
                let other = if image == slot::LAYER_A { slot::LAYER_B } else { slot::LAYER_A };
                self.effect_pass(&mut encoder, targets, MASK_PASS, &[hide, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], time, image, drawn, other);
                image = other;
            }
            let matte = self.draw_track_matte(&mut encoder, targets, &shapers, index, layer);
            let blend = layer.blend_mode as u32;
            let matte_mode = if matte.is_some() { layer.track_matte as u32 } else { 0 };
            self.composite(&mut encoder, targets, accum, image, matte, [blend, matte_mode, 0], opacity, spare);
            std::mem::swap(&mut accum, &mut spare);
        }
        flush(&mut batch, &mut encoder, &mut accum, &mut spare);

        // Convert to sRGB and copy to the readback buffer.
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("output"),
            layout: &self.output_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&targets.work[accum].view) }],
        });
        self.fullscreen(&mut encoder, &self.output_pipeline, &bind_group, &targets.output.view);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &targets.output.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &targets.readback,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(targets.padded_row), rows_per_image: Some(height) },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);

        let slice = targets.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
            .map_err(|error| RenderError(format!("the GPU did not finish: {error}")))?;
        if let Some(error) = self.gpu_error.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take() {
            targets.readback.unmap();
            return Err(RenderError(format!("the GPU refused part of the frame: {error}")));
        }
        let mut bgra = Vec::with_capacity((width * height * 4) as usize);
        {
            let mapped = slice.get_mapped_range().map_err(|error| RenderError(format!("could not read the frame back: {error}")))?;
            for row in mapped.chunks(targets.padded_row as usize).take(height as usize) {
                bgra.extend_from_slice(&row[..(width * 4) as usize]);
            }
        }
        targets.readback.unmap();
        Ok(RenderedFrame { width, height, bgra, text_sizes, used_proxy })
    }

    /// Draws a text layer's letters along the pen path it is set on, each one turned to follow
    /// the path, with its baseline on it. Letters that run off the end of an open path are not
    /// drawn. The text layer's scale sets their size; its position and rotation are not used.
    fn draw_text_on_path(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, layer: &Layer, scale: f32, text_keys: &HashMap<Id, (u64, Vec2, Vec2)>, target: usize) {
        let (Some((key, measured, pad)), Some(on_path)) = (text_keys.get(&layer.id), self.path_text.get(&layer.id)) else { return };
        let PathText { letters, baseline, walk, start, letter_scale } = on_path;
        let (baseline, letter_scale) = (*baseline, *letter_scale);
        let texture = &self.text_cache[key];
        let total = texture.size.x.max(1.0);
        let mut sources = Vec::new();
        for (index, &(left, width)) in letters.iter().enumerate() {
            // The first and last letters take the raster's padding too, so nothing is cut off.
            let from = if index == 0 { -pad.x } else { left };
            let to = if index + 1 == letters.len() { measured.x + pad.x } else { left + width };
            let middle = left + width * 0.5;
            let Some((point, angle)) = walk.at(start + middle * letter_scale) else { continue };
            // Letter pixels, centered on the letter's middle at the baseline, turned onto the path.
            let world = Affine::translate(Vec2::new(-middle, -baseline))
                .then(Affine::scale(Vec2::new(letter_scale, letter_scale)))
                .then(Affine::rotate(angle.to_degrees()))
                .then(Affine::translate(point));
            let slice = Source::TexturePart(&texture.view, [(from + pad.x) / total, 0.0, (to - from) / total, 1.0], 1.0);
            sources.push((world, slice, (Vec2::new(from, -pad.y), Vec2::new(to - from, texture.size.y))));
        }
        let drawn: Vec<(Affine, &Source, (Vec2, Vec2))> = sources.iter().map(|(world, source, rect)| (*world, source, *rect)).collect();
        if !drawn.is_empty() {
            self.draw_sources(encoder, targets, target, scale, &drawn);
        }
    }

    /// Where each animated piece of `text` is in its raster (from the caret positions of its
    /// layout), with the pose `poses` gives it. Pieces at the start or end of a line, and on the
    /// first or last line, reach out over the raster's padding so strokes aren't cut off.
    fn text_pieces(&mut self, text: &str, style: &studio_core::model::TextStyle, poses: &[(std::ops::Range<usize>, studio_core::text_animation::UnitPose)], measured: Vec2, pad: Vec2) -> Vec<TextPiece> {
        let geometry = self.text.geometry(text, style);
        let last_line = geometry.lines.len().saturating_sub(1);
        let line_of = |offset: usize| geometry.lines.iter().rposition(|line| line.stops.first().is_some_and(|&(at, _)| at <= offset)).unwrap_or(0);
        poses
            .iter()
            .map(|(range, pose)| {
                let line_index = line_of(range.start);
                let line = &geometry.lines[line_index];
                let (start, _, _) = geometry.caret(range.start);
                let (end, _, _) = geometry.caret(range.end);
                let (mut left, mut right) = (start.min(end), start.max(end));
                // At the ends of its line, the piece takes the padding (and any space) there.
                let first_on_line = poses.iter().all(|(other, _)| line_of(other.start) != line_index || other.start >= range.start);
                let last_on_line = poses.iter().all(|(other, _)| line_of(other.start) != line_index || other.end <= range.end);
                if first_on_line {
                    left = -pad.x;
                }
                if last_on_line {
                    right = measured.x + pad.x;
                }
                let top = if line_index == 0 { -pad.y } else { line.top };
                let bottom = if line_index == last_line { measured.y + pad.y } else { line.top + line.height };
                TextPiece { origin: Vec2::new(left, top), size: Vec2::new((right - left).max(0.0), (bottom - top).max(0.0)), pose: *pose }
            })
            .collect()
    }

    /// Draws a text layer piece by piece, each moved, scaled, turned and faded about its own
    /// middle as its animation says.
    #[allow(clippy::too_many_arguments)]
    fn draw_text_pieces(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets, layer: &Layer, world: Affine, scale: f32, text_keys: &HashMap<Id, (u64, Vec2, Vec2)>, target: usize) {
        let (Some((key, measured, pad)), Some(pieces)) = (text_keys.get(&layer.id), self.text_pieces.get(&layer.id)) else { return };
        let texture = &self.text_cache[key];
        let total = Vec2::new(texture.size.x.max(1.0), texture.size.y.max(1.0));
        // Fit the text box to the layer's box, as when the text is drawn whole.
        let fit = Vec2::new(layer.size.x / measured.x.max(1.0), layer.size.y / measured.y.max(1.0));
        let mut sources = Vec::new();
        for piece in pieces.iter().filter(|piece| piece.pose.opacity > 0.001 && piece.pose.scale > 0.0001 && piece.size.x > 0.0) {
            let middle = piece.origin + piece.size * 0.5;
            let placed = Affine::translate(Vec2::new(-middle.x, -middle.y))
                .then(Affine::scale(Vec2::new(piece.pose.scale, piece.pose.scale)))
                .then(Affine::rotate(piece.pose.rotation))
                .then(Affine::translate(middle + piece.pose.offset))
                .then(Affine::scale(fit))
                .then(world);
            let part = [(piece.origin.x + pad.x) / total.x, (piece.origin.y + pad.y) / total.y, piece.size.x / total.x, piece.size.y / total.y];
            sources.push((placed, Source::TexturePart(&texture.view, part, piece.pose.opacity), (piece.origin, piece.size)));
        }
        let drawn: Vec<(Affine, &Source, (Vec2, Vec2))> = sources.iter().map(|(world, source, rect)| (*world, source, *rect)).collect();
        if !drawn.is_empty() {
            self.draw_sources(encoder, targets, target, scale, &drawn);
        }
    }

    /// Draws one layer's source (solid, text, image or a placeholder) into work slot `target`.
    #[allow(clippy::too_many_arguments)]
    fn draw_layer(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        project: &Project,
        layer: &Layer,
        world: Affine,
        scale: f32,
        text_keys: &HashMap<Id, (u64, Vec2, Vec2)>,
        target: usize,
    ) {
        if let Some(single) = self.single_source(project, layer, world, text_keys) {
            if let Some((world, source, rect)) = single {
                self.draw_source(encoder, targets, target, world, scale, &source, rect);
            }
            return;
        }
        match &layer.content {
            LayerContent::Captions { .. } => {
                let (Some((key, measured, pad)), Some(draw)) = (text_keys.get(&layer.id), self.captions.get(&layer.id)) else { return };
                let texture = &self.text_cache[key];
                // Centered in the layer's box, scaled about its center by the animation.
                let center = layer.size * 0.5;
                let around = |world: Affine| Affine::translate(Vec2::new(-center.x, -center.y)).then(Affine::scale(Vec2::new(draw.scale, draw.scale))).then(Affine::translate(center)).then(world);
                let world = around(world);
                let top_left = center - *measured * 0.5;
                let text = Source::FadedTexture(&texture.view, draw.opacity);
                let text_rect = (top_left - *pad, texture.size);
                match draw.background {
                    // A rounded box behind the text, padded around it.
                    Some(color) => {
                        let size = *measured + CAPTION_PADDING * 2.0;
                        let style = ShapeStyle { fill: Some(Paint::Solid(color.with_alpha(color.a * draw.opacity))), corner_radius: size.y.min(size.x) * 0.25, ..ShapeStyle::new(ShapeKind::Rectangle) };
                        let box_source = Source::Shape(style, top_left - CAPTION_PADDING, size);
                        self.draw_sources(encoder, targets, target, scale, &[(world, &box_source, (top_left - CAPTION_PADDING, size)), (world, &text, text_rect)]);
                    }
                    None => self.draw_source(encoder, targets, target, world, scale, &text, text_rect),
                }
            }
            LayerContent::Text { .. } if self.path_text.contains_key(&layer.id) => self.draw_text_on_path(encoder, targets, layer, scale, text_keys, target),
            LayerContent::Text { .. } if self.text_pieces.contains_key(&layer.id) => self.draw_text_pieces(encoder, targets, layer, world, scale, text_keys, target),
            _ => {}
        }
    }

    /// Draws a moving layer with motion blur into work slot `target`: once at each of `worlds`
    /// (its transforms across the shutter), averaged. Each moment is drawn into a spare slot and
    /// mixed into a running average that ping-pongs between `target` and the other layer slot,
    /// starting in whichever slot makes the last mix land in `target`.
    #[allow(clippy::too_many_arguments)]
    fn draw_motion_blurred_layer(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        project: &Project,
        layer: &Layer,
        worlds: &[Affine],
        scale: f32,
        text_keys: &HashMap<Id, (u64, Vec2, Vec2)>,
        target: usize,
    ) {
        let Some((first, rest)) = worlds.split_first() else { return };
        let other = if target == slot::LAYER_A { slot::LAYER_B } else { slot::LAYER_A };
        let (mut average, mut next) = if rest.len() % 2 == 0 { (target, other) } else { (other, target) };
        self.draw_layer(encoder, targets, project, layer, *first, scale, text_keys, average);
        for (index, world) in rest.iter().enumerate() {
            self.draw_layer(encoder, targets, project, layer, *world, scale, text_keys, slot::AUX_A);
            // Moment k (counting from 1) joins the average of the k − 1 before it at weight 1/k.
            let weight = 1.0 / (index + 2) as f32;
            self.effect_pass(encoder, targets, MOTION_BLUR_PASS, &[weight], 0.0, average, slot::AUX_A, next);
            std::mem::swap(&mut average, &mut next);
        }
    }

    /// What a layer drawn in one piece is drawn from: where (its world transform, adjusted to
    /// the source), the source, and the rectangle of source pixels drawn. `Some(None)` when there
    /// is nothing to draw (text not measured yet, an adjustment layer); `None` for layers drawn in
    /// several pieces (captions, text on a path or in animated pieces), which `draw_layer` draws.
    fn single_source<'a>(&'a self, project: &Project, layer: &Layer, world: Affine, text_keys: &HashMap<Id, (u64, Vec2, Vec2)>) -> Option<Option<Drawn<'a>>> {
        let placeholder = || Source::Placeholder(linear(layer.label.color()));
        let full = (Vec2::default(), layer.size);
        Some(match &layer.content {
            LayerContent::Solid { color } => Some((world, Source::Solid(linear(*color)), full)),
            LayerContent::Captions { .. } => return None,
            LayerContent::Shape { path: Some(_), .. } => self.paths.get(&layer.id).and_then(|key| self.path_cache.get(key)).map(|(texture, origin)| (world, Source::Texture(&texture.view), (*origin, texture.size))),
            LayerContent::Shape { style, path: None } => {
                // Grow the drawn rectangle by the stroke, which is centered on the edge.
                let grow = style.stroke.map_or(0.0, |stroke| stroke.width * 0.5 + 1.0);
                let rect = (Vec2::new(-grow, -grow), Vec2::new(layer.size.x + grow * 2.0, layer.size.y + grow * 2.0));
                Some((world, Source::Shape(*style, Vec2::default(), layer.size), rect))
            }
            LayerContent::Text { .. } if self.path_text.contains_key(&layer.id) || self.text_pieces.contains_key(&layer.id) => return None,
            LayerContent::Text { .. } => text_keys.get(&layer.id).map(|(key, measured, pad)| {
                let texture = &self.text_cache[key];
                // Fit the text box to the layer's box, so the anchor point stays where the user put it.
                let fit = Vec2::new(layer.size.x / measured.x.max(1.0), layer.size.y / measured.y.max(1.0));
                (Affine::scale(fit).then(world), Source::Texture(&texture.view), (Vec2::new(-pad.x, -pad.y), texture.size))
            }),
            LayerContent::Image { item } => {
                let loaded = project.item(*item).and_then(|item| item.path.as_ref()).and_then(|path| self.images.get(path)).and_then(|(_, texture)| texture.as_ref());
                Some(match loaded {
                    // Stretch the image to the layer's size (they match unless the file changed).
                    Some(image) => (Affine::scale(Vec2::new(layer.size.x / image.size.x.max(1.0), layer.size.y / image.size.y.max(1.0))).then(world), Source::Texture(&image.view), (Vec2::default(), image.size)),
                    None => (world, placeholder(), full),
                })
            }
            LayerContent::Footage { .. } => Some(match self.videos.get(&layer.id) {
                // Stretch the decoded picture (which may be scaled down for preview) over the layer.
                Some(video) => {
                    let fit = Vec2::new(layer.size.x / video.texture.size.x.max(1.0), layer.size.y / video.texture.size.y.max(1.0));
                    (Affine::scale(fit).then(world), Source::Texture(&video.texture.view), (Vec2::default(), video.texture.size))
                }
                // Still decoding (or the file is missing): show the placeholder.
                None => (world, placeholder(), full),
            }),
            // A nested composition, rendered before this one, stretched over the layer's box.
            LayerContent::Composition { .. } => self.nested.get(&layer.id).map(|picture| {
                let fit = Vec2::new(layer.size.x / picture.size.x.max(1.0), layer.size.y / picture.size.y.max(1.0));
                (Affine::scale(fit).then(world), Source::Texture(&picture.view), (Vec2::default(), picture.size))
            }),
            LayerContent::Adjustment | LayerContent::Audio { .. } => None,
        })
    }
}
