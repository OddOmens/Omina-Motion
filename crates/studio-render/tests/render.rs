//! Renders small compositions on the GPU and checks real pixel colors.

use studio_core::animation::{Animated, Vec2};
use studio_core::effects;
use studio_core::model::{
    BlendMode, Composition, Id, LabelColor, Layer, LayerContent, LayerSwitches, Project, Paint, Rgba, ShapeKind, ShapeStyle, Stroke, TextStyle,
    TrackMatte, Transform,
};
use studio_core::sample::{ids, sample_project};
use studio_core::time::FrameRate;
use studio_render::Renderer;

const SIZE: u32 = 100;

fn solid(id: u64, color: u32, size: Vec2, position: Vec2) -> Layer {
    Layer {
        id: Id(id),
        name: format!("solid {id}"),
        label: LabelColor::Red,
        content: LayerContent::Solid { color: Rgba::hex(color) },
        size,
        in_frame: 0,
        out_frame: 10,
        start_frame: 0,
        track: 0,
        blend_mode: BlendMode::Normal,
        track_matte: TrackMatte::None,
        parent: None,
        switches: LayerSwitches::default(),
        transform: Some(Transform::centered(size, position)),
        effects: Vec::new(),
        has_audio: false,
    }
}

fn full(id: u64, color: u32) -> Layer {
    solid(id, color, Vec2::new(100.0, 100.0), Vec2::new(50.0, 50.0))
}

/// A 100 × 100 composition with a black background and `layers` (top first).
fn project(layers: Vec<Layer>) -> Project {
    let composition = Composition {
        id: Id(1),
        name: "test".into(),
        width: SIZE,
        height: SIZE,
        frame_rate: FrameRate::new(24, 1),
        duration_frames: 10,
        background: Rgba::hex(0x000000),
        layers,
    };
    Project::new(Vec::new(), vec![composition])
}

/// BGRA of the pixel at (x, y), returned as [r, g, b].
fn pixel(frame: &studio_render::RenderedFrame, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * frame.width + x) * 4) as usize;
    [frame.bgra[i + 2], frame.bgra[i + 1], frame.bgra[i]]
}

fn render(project: &Project) -> studio_render::RenderedFrame {
    let mut renderer = Renderer::new().expect("a GPU is available");
    renderer.render(project, Id(1), 0, SIZE, SIZE, &studio_core::media::NoVideo).expect("the frame renders")
}

fn close(actual: [u8; 3], expected: [u8; 3]) -> bool {
    actual.iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= 3)
}

#[test]
fn a_solid_fills_its_area_and_the_background_shows_elsewhere() {
    let frame = render(&project(vec![solid(2, 0xff0000, Vec2::new(40.0, 40.0), Vec2::new(50.0, 50.0))]));
    assert_eq!((frame.width, frame.height, frame.bgra.len()), (SIZE, SIZE, (SIZE * SIZE * 4) as usize));
    assert!(close(pixel(&frame, 50, 50), [255, 0, 0]));
    assert!(close(pixel(&frame, 5, 5), [0, 0, 0]));
}

#[test]
fn opacity_mixes_in_linear_light() {
    let mut white = full(2, 0xffffff);
    white.transform.as_mut().unwrap().opacity = Animated::constant(50.0);
    // 50% of linear white is 0.5 linear, which is about 188 in sRGB.
    assert!(close(pixel(&render(&project(vec![white])), 50, 50), [188, 188, 188]));
}

#[test]
fn rotation_turns_the_layer() {
    let mut bar = solid(2, 0xffffff, Vec2::new(80.0, 10.0), Vec2::new(50.0, 50.0));
    let flat = render(&project(vec![bar.clone()]));
    assert!(close(pixel(&flat, 50, 20), [0, 0, 0]) && close(pixel(&flat, 20, 50), [255, 255, 255]));
    bar.transform.as_mut().unwrap().rotation = Animated::constant(90.0);
    let turned = render(&project(vec![bar]));
    assert!(close(pixel(&turned, 50, 20), [255, 255, 255]), "the bar now runs vertically");
    assert!(close(pixel(&turned, 20, 50), [0, 0, 0]));
}

#[test]
fn children_follow_their_parent() {
    let mut parent = solid(2, 0x000000, Vec2::new(10.0, 10.0), Vec2::new(50.0, 50.0));
    parent.switches.visible = false;
    let child = solid(3, 0x00ff00, Vec2::new(10.0, 10.0), Vec2::new(80.0, 50.0));
    let mut project = project(vec![child, parent]);
    let comp = project.composition_mut(Id(1)).unwrap();
    studio_core::edit::set_parent(comp, Id(3), Some(Id(2)), 0);
    assert!(close(pixel(&render(&project), 80, 50), [0, 255, 0]), "parenting does not move the child");
    // Rotating the parent 90° about (50, 50) swings the child from (80, 50) to (50, 80).
    let comp = project.composition_mut(Id(1)).unwrap();
    comp.layer_mut(Id(2)).unwrap().transform.as_mut().unwrap().rotation = Animated::constant(90.0);
    let frame = render(&project);
    assert!(close(pixel(&frame, 50, 80), [0, 255, 0]));
    assert!(close(pixel(&frame, 80, 50), [0, 0, 0]));
}

#[test]
fn blend_modes_change_the_result() {
    let check = |mode: BlendMode, expected: [u8; 3]| {
        let mut top = full(2, 0xff8000);
        top.blend_mode = mode;
        let result = pixel(&render(&project(vec![top, full(3, 0x808080)])), 50, 50);
        assert!(close(result, expected), "{mode:?}: got {result:?}, expected {expected:?}");
    };
    // Expected values computed in linear light: sRGB 0x80 is 0.2158 linear.
    check(BlendMode::Normal, [255, 128, 0]);
    check(BlendMode::Multiply, [128, 61, 0]); // 0.2158 × 0.2158 = 0.0466 → 61
    check(BlendMode::Add, [255, 176, 128]); // 0.2158 + 0.2158 = 0.4316 → 176
    check(BlendMode::Screen, [255, 167, 128]); // 1 − 0.7842² = 0.385 → 167
    check(BlendMode::Difference, [229, 0, 128]); // |1 − 0.2158| = 0.784 → 229
    for mode in BlendMode::ALL {
        let mut top = full(2, 0xff8000);
        top.blend_mode = mode;
        render(&project(vec![top, full(3, 0x808080)]));
    }
}

#[test]
fn track_mattes_limit_the_layer_to_the_matte() {
    let matte = solid(2, 0xffffff, Vec2::new(20.0, 20.0), Vec2::new(50.0, 50.0));
    let mut fill = full(3, 0x0000ff);
    fill.track_matte = TrackMatte::Alpha;
    let frame = render(&project(vec![matte.clone(), fill.clone()]));
    assert!(close(pixel(&frame, 50, 50), [0, 0, 255]), "inside the matte the layer shows");
    assert!(close(pixel(&frame, 10, 10), [0, 0, 0]), "outside it does not, and the matte itself is hidden");
    fill.track_matte = TrackMatte::AlphaInverted;
    let inverted = render(&project(vec![matte, fill]));
    assert!(close(pixel(&inverted, 50, 50), [0, 0, 0]) && close(pixel(&inverted, 10, 10), [0, 0, 255]));
}

#[test]
fn solo_hides_every_other_layer() {
    let top = full(2, 0xff0000);
    let mut bottom = full(3, 0x00ff00);
    bottom.switches.solo = true;
    assert!(close(pixel(&render(&project(vec![top, bottom])), 50, 50), [0, 255, 0]));
}

#[test]
fn adjustment_layers_affect_everything_below() {
    let mut adjustment = full(2, 0x000000);
    adjustment.content = LayerContent::Adjustment;
    adjustment.effects.push(effects::instantiate(effects::find("Tint").unwrap(), Id(9)));
    let result = pixel(&render(&project(vec![adjustment, full(3, 0xff0000)])), 50, 50);
    assert!(result[0] == result[1] && result[1] == result[2] && result[0] > 20, "red became gray: {result:?}");
}

#[test]
fn gaussian_blur_softens_edges() {
    let mut square = solid(2, 0xffffff, Vec2::new(40.0, 40.0), Vec2::new(50.0, 50.0));
    let sharp = render(&project(vec![square.clone()]));
    square.effects.push(effects::instantiate(effects::find("Gaussian Blur").unwrap(), Id(9)));
    let blurred = render(&project(vec![square]));
    let edge = |frame| pixel(frame, 30, 50)[0];
    assert!(edge(&sharp) > 250 || edge(&sharp) < 5, "sharp edge");
    assert!((40..230).contains(&edge(&blurred)), "the edge is soft after blurring: {}", edge(&blurred));
}

#[test]
fn every_rendered_effect_runs_on_the_gpu() {
    for definition in effects::CATEGORIES.iter().flat_map(|category| category.effects) {
        if !studio_render::effects::is_rendered(definition.name) {
            continue;
        }
        let mut layer = solid(2, 0x3366cc, Vec2::new(60.0, 60.0), Vec2::new(50.0, 50.0));
        layer.effects.push(effects::instantiate(definition, Id(9)));
        // Any wgpu validation error panics, failing the test with the effect's name in the log.
        let frame = render(&project(vec![layer]));
        assert_eq!(frame.bgra.len(), (SIZE * SIZE * 4) as usize, "{}", definition.name);
    }
}

/// Values worth trying for an effect parameter: its default, then points across its range,
/// so effects that do nothing at their defaults (a 0% wipe, say) are still exercised.
fn trial_values(property: &studio_core::model::EffectProperty) -> Vec<f32> {
    let default = property.value.value_at(0.0);
    let (min, max) = (property.info.min.max(-200.0), property.info.max.min(200.0));
    vec![default, min + (max - min) * 0.3, min + (max - min) * 0.7]
}

#[test]
fn every_rendered_effect_changes_the_picture() {
    let mut renderer = Renderer::new().expect("a GPU is available");
    // A two-tone layer, so effects that only move or mix colours have something to show.
    let mut base = solid(2, 0x3366cc, Vec2::new(60.0, 60.0), Vec2::new(50.0, 50.0));
    base.content = LayerContent::Text { text: "Ab".into(), style: TextStyle { font_size: 40.0, line_height: 48.0, fill: Some(Rgba::hex(0xff9933)), ..TextStyle::default() } };
    let plain = renderer.render(&project(vec![base.clone(), full(3, 0x204060)]), Id(1), 0, SIZE, SIZE, &studio_core::media::NoVideo).unwrap();
    let mut idle = Vec::new();
    for definition in effects::CATEGORIES.iter().flat_map(|category| category.effects) {
        if !studio_render::effects::is_rendered(definition.name) {
            continue;
        }
        let template = effects::instantiate(definition, Id(9));
        let changed = (0..3).any(|trial| {
            let mut effect = template.clone();
            for property in &mut effect.properties {
                property.value = Animated::constant(trial_values(property)[trial]);
            }
            let mut layer = base.clone();
            layer.effects.push(effect);
            let frame = renderer.render(&project(vec![layer, full(3, 0x204060)]), Id(1), 3, SIZE, SIZE, &studio_core::media::NoVideo).unwrap();
            frame.bgra != plain.bgra
        });
        if !changed {
            idle.push(definition.name);
        }
    }
    assert!(idle.is_empty(), "these effects left the picture unchanged: {idle:?}");
}

/// Each effect must keep a 1080p frame well inside real time on this machine's GPU.
#[test]
fn every_rendered_effect_is_fast_at_1080p() {
    let mut renderer = Renderer::new().expect("a GPU is available");
    let mut slow = Vec::new();
    for definition in effects::CATEGORIES.iter().flat_map(|category| category.effects) {
        if !studio_render::effects::is_rendered(definition.name) {
            continue;
        }
        let mut layer = solid(2, 0x3366cc, Vec2::new(1600.0, 900.0), Vec2::new(960.0, 540.0));
        layer.effects.push(effects::instantiate(definition, Id(9)));
        let mut scene = project(vec![layer, solid(3, 0x204060, Vec2::new(1920.0, 1080.0), Vec2::new(960.0, 540.0))]);
        let composition = &mut scene.compositions[0];
        (composition.width, composition.height) = (1920, 1080);
        // The first frame compiles pipelines; time the ones after it.
        renderer.render(&scene, Id(1), 0, 1920, 1080, &studio_core::media::NoVideo).unwrap();
        let start = std::time::Instant::now();
        for frame in 1..6 {
            renderer.render(&scene, Id(1), frame, 1920, 1080, &studio_core::media::NoVideo).unwrap();
        }
        let each = start.elapsed().as_secs_f64() * 1000.0 / 5.0;
        println!("{:<24} {each:6.1} ms", definition.name);
        if each > 40.0 {
            slow.push((definition.name, each));
        }
    }
    assert!(slow.is_empty(), "too slow for playback at 1080p: {slow:?}");
}

/// An 80 × 80 white shape in the middle of the frame, over black.
fn shape_frame(kind: ShapeKind, corner_radius: f32) -> studio_render::RenderedFrame {
    let mut layer = solid(2, 0, Vec2::new(80.0, 80.0), Vec2::new(50.0, 50.0));
    layer.content = LayerContent::Shape { style: ShapeStyle { fill: Some(Paint::Solid(Rgba::hex(0xffffff))), corner_radius, ..ShapeStyle::new(kind) } };
    render(&project(vec![layer]))
}

#[test]
fn shapes_fill_their_outline() {
    let lit = |frame: &studio_render::RenderedFrame, x, y| pixel(frame, x, y)[0] > 200;
    let dark = |frame: &studio_render::RenderedFrame, x, y| pixel(frame, x, y)[0] < 40;
    // The box runs from 10 to 90 on both axes.
    let rectangle = shape_frame(ShapeKind::Rectangle, 0.0);
    assert!(lit(&rectangle, 50, 50) && lit(&rectangle, 12, 12), "a plain rectangle fills its corners");
    assert!(dark(&rectangle, 5, 5), "and nothing outside");
    let ellipse = shape_frame(ShapeKind::Ellipse, 0.0);
    assert!(lit(&ellipse, 50, 50) && lit(&ellipse, 50, 12) && dark(&ellipse, 14, 14), "an ellipse leaves the corners empty");
    let triangle = shape_frame(ShapeKind::Triangle, 0.0);
    assert!(lit(&triangle, 50, 80) && lit(&triangle, 14, 87) && dark(&triangle, 14, 14) && dark(&triangle, 86, 14), "a triangle points up");
    for kind in [ShapeKind::Polygon, ShapeKind::Star] {
        let frame = shape_frame(kind, 0.0);
        assert!(lit(&frame, 50, 50) && dark(&frame, 12, 12), "{kind:?} is filled in the middle only");
    }
    // A star's notches are empty where a polygon of the same size is filled.
    let (polygon, star) = (shape_frame(ShapeKind::Polygon, 0.0), shape_frame(ShapeKind::Star, 0.0));
    let filled = |frame: &studio_render::RenderedFrame| (0..SIZE).flat_map(|y| (0..SIZE).map(move |x| (x, y))).filter(|&(x, y)| pixel(frame, x, y)[0] > 128).count();
    assert!(filled(&star) < filled(&polygon) * 3 / 4, "the star is notched: {} vs {}", filled(&star), filled(&polygon));
}

#[test]
fn corner_radius_rounds_every_shape_but_the_ellipse() {
    let rounded = shape_frame(ShapeKind::Rectangle, 30.0);
    assert!(pixel(&rounded, 12, 12)[0] < 40, "the corner is cut away");
    assert!(pixel(&rounded, 50, 12)[0] > 200, "the straight edge is not");
    let coverage = |frame: &studio_render::RenderedFrame| (0..SIZE).flat_map(|y| (0..SIZE).map(move |x| (x, y))).map(|(x, y)| u32::from(pixel(frame, x, y)[0])).sum::<u32>();
    for kind in [ShapeKind::Triangle, ShapeKind::Polygon] {
        let (sharp, soft) = (shape_frame(kind, 0.0), shape_frame(kind, 12.0));
        assert!(coverage(&soft) < coverage(&sharp), "rounding {kind:?} trims its corners");
        assert!(coverage(&soft) > coverage(&sharp) / 2, "without shrinking {kind:?} away");
    }
    // A star's points are rounded off, reaching as far but blunter: more of the rows just under
    // the top point are filled.
    let (sharp, soft) = (shape_frame(ShapeKind::Star, 0.0), shape_frame(ShapeKind::Star, 12.0));
    let tip = |frame: &studio_render::RenderedFrame| (10..18).flat_map(|y| (30..70).map(move |x| (x, y))).map(|(x, y)| u32::from(pixel(frame, x, y)[0])).sum::<u32>();
    assert!(tip(&soft) > tip(&sharp), "the top point is blunter: {} vs {}", tip(&soft), tip(&sharp));
    assert!(pixel(&soft, 50, 8)[0] < 40, "and reaches no further than the box");
    assert_eq!(shape_frame(ShapeKind::Ellipse, 30.0).bgra, shape_frame(ShapeKind::Ellipse, 0.0).bgra, "an ellipse has no corners");
}

#[test]
fn text_draws_glyphs_in_its_fill_color_and_reports_its_size() {
    let style = TextStyle { font_family: "Helvetica".into(), font_size: 40.0, line_height: 48.0, fill: Some(Rgba::hex(0xffff00)), ..TextStyle::default() };
    let size = Vec2::new(80.0, 48.0);
    let mut text = solid(2, 0, size, Vec2::new(50.0, 50.0));
    text.content = LayerContent::Text { text: "Hi".into(), style };
    let frame = render(&project(vec![text]));
    let yellow = (0..SIZE).flat_map(|y| (0..SIZE).map(move |x| (x, y))).filter(|&(x, y)| close(pixel(&frame, x, y), [255, 255, 0])).count();
    assert!(yellow > 50, "yellow glyph pixels: {yellow}");
    assert_eq!(frame.text_sizes.len(), 1);
    assert!(frame.text_sizes[0].1.x > 20.0);
}

#[test]
fn the_sample_project_renders_every_frame() {
    let project = sample_project();
    let mut renderer = Renderer::new().unwrap();
    for frame in (0..240).step_by(24) {
        let rendered = renderer.render(&project, ids::MAIN_COMP, frame, 384, 216, &studio_core::media::NoVideo).unwrap();
        assert_eq!(rendered.bgra.len(), 384 * 216 * 4);
    }
}


#[test]
fn glow_adds_light_without_eating_the_backdrop() {
    // A gray layer whose glow spills onto a gray backdrop.
    let mut card = solid(2, 0xb19478, Vec2::new(40.0, 40.0), Vec2::new(50.0, 50.0));
    let mut glow = effects::instantiate(effects::find("Glow").unwrap(), Id(9));
    glow.properties[0].value = Animated::constant(20.0);
    glow.properties[2].value = Animated::constant(2.0);
    card.effects.push(glow);
    let frame = render(&project(vec![card, full(3, 0x808080)]));
    let center = pixel(&frame, 50, 50);
    let beside = pixel(&frame, 50, 25);
    assert!(center.iter().zip([177, 148, 120]).all(|(c, plain)| *c >= plain), "glow only brightens: {center:?}");
    assert!(beside.iter().all(|c| *c >= 128), "the backdrop next to it is lit, not darkened: {beside:?}");
}

/// A video source whose frame N is solid red for even N and blue for odd N; it records what was asked.
struct StripedVideo {
    asked: std::cell::RefCell<Vec<i64>>,
}

impl studio_core::media::VideoSource for StripedVideo {
    fn frame(&self, _: &std::path::Path, index: i64) -> Option<std::sync::Arc<studio_core::media::VideoFrame>> {
        self.asked.borrow_mut().push(index);
        let color: [u8; 4] = if index % 2 == 0 { [255, 0, 0, 255] } else { [0, 0, 255, 255] };
        Some(std::sync::Arc::new(studio_core::media::VideoFrame { width: 8, height: 8, index, rgba: color.repeat(64), from_proxy: false }))
    }
}

#[test]
fn footage_shows_the_source_frame_for_the_current_time() {
    use studio_core::model::{ProjectItem, ProjectItemKind};
    let mut clip = full(2, 0);
    clip.content = LayerContent::Footage { item: Id(50) };
    clip.in_frame = 2;
    clip.start_frame = 2;
    let mut project = project(vec![clip]);
    // A 48 fps clip in a 24 fps composition: each comp frame advances two source frames.
    project.items.push(ProjectItem {
        id: Id(50),
        name: "clip.mov".into(),
        kind: ProjectItemKind::Footage,
        label: LabelColor::Aqua,
        parent: None,
        size: Some(Vec2::new(100.0, 100.0)),
        duration_seconds: Some(10.0),
        frame_rate: Some(FrameRate::new(48, 1)),
        has_audio: false,
        color: None,
        path: Some("/clip.mov".into()),
    });
    let video = StripedVideo { asked: Default::default() };
    let mut renderer = Renderer::new().unwrap();
    // Comp frame 5 is 3 frames into the layer: 0.125 s → source frame 6 (red).
    let first = renderer.render(&project, Id(1), 5, SIZE, SIZE, &video).unwrap();
    assert!(close(pixel(&first, 50, 50), [255, 0, 0]));
    // At a matching 24 fps the same moment is source frame 3 (odd → blue).
    project.items[0].frame_rate = Some(FrameRate::new(24, 1));
    let second = renderer.render(&project, Id(1), 5, SIZE, SIZE, &video).unwrap();
    assert!(close(pixel(&second, 50, 50), [0, 0, 255]));
    // Slipped so the source starts 4 frames earlier: the same moment shows source frame 7.
    project.compositions[0].layers[0].start_frame = -2;
    renderer.render(&project, Id(1), 5, SIZE, SIZE, &video).unwrap();
    assert_eq!(*video.asked.borrow(), vec![6, 3, 7]);
}

// Regression tests for bugs found in review --------------------------------------------------

fn text_layer(id: u64, text: &str, size: Vec2, position: Vec2) -> Layer {
    let style = TextStyle { font_family: "Helvetica".into(), font_size: 40.0, line_height: 48.0, fill: Some(Rgba::hex(0xffffff)), ..TextStyle::default() };
    let mut layer = solid(id, 0, size, position);
    layer.content = LayerContent::Text { text: text.into(), style };
    layer
}

#[test]
fn a_full_text_cache_never_drops_a_raster_the_frame_still_needs() {
    let mut renderer = Renderer::new().unwrap();
    // Fill the cache to one short of its limit (64), one text per frame.
    for n in 0..63 {
        let project = project(vec![text_layer(2, &format!("text {n}"), Vec2::new(80.0, 48.0), Vec2::new(50.0, 50.0))]);
        renderer.render(&project, Id(1), 0, SIZE, SIZE, &studio_core::media::NoVideo).unwrap();
    }
    // Then a frame with two texts the cache hasn't seen: both must draw.
    let two = project(vec![
        text_layer(2, "new A", Vec2::new(80.0, 48.0), Vec2::new(50.0, 25.0)),
        text_layer(3, "new B", Vec2::new(80.0, 48.0), Vec2::new(50.0, 75.0)),
    ]);
    let frame = renderer.render(&two, Id(1), 0, SIZE, SIZE, &studio_core::media::NoVideo).unwrap();
    assert_eq!(frame.text_sizes.len(), 2);
}

#[test]
fn text_larger_than_the_gpu_limit_draws_instead_of_crashing() {
    // 200 px text at 4000% is far past the 8192-pixel texture limit.
    let mut big = text_layer(2, "Studio Studio", Vec2::new(80.0, 48.0), Vec2::new(50.0, 50.0));
    big.transform.as_mut().unwrap().scale = Animated::constant(Vec2::new(4000.0, 4000.0));
    if let LayerContent::Text { style, .. } = &mut big.content {
        style.font_size = 200.0;
    }
    let mut renderer = Renderer::new().unwrap();
    renderer.render(&project(vec![big]), Id(1), 0, SIZE, SIZE, &studio_core::media::NoVideo).unwrap();
}

#[test]
fn text_scaled_past_eight_times_is_drawn_at_its_full_size() {
    // Small text, so even at 1600% it fits in the frame. Raster scales stop at 8×, but the drawn
    // text must keep growing: twice the scale is about four times the ink.
    let ink = |scale: f32| {
        let mut layer = text_layer(2, "I", Vec2::new(3.0, 5.0), Vec2::new(50.0, 50.0));
        if let LayerContent::Text { style, .. } = &mut layer.content {
            (style.font_size, style.line_height) = (4.0, 5.0);
        }
        layer.transform.as_mut().unwrap().scale = Animated::constant(Vec2::new(scale, scale));
        let frame = render(&project(vec![layer]));
        (0..SIZE).flat_map(|y| (0..SIZE).map(move |x| (x, y))).filter(|&(x, y)| pixel(&frame, x, y)[0] > 128).count() as f32
    };
    let (at_8, at_16) = (ink(800.0), ink(1600.0));
    assert!(at_16 > at_8 * 3.0, "ink at 1600%: {at_16}, at 800%: {at_8}");
}

#[test]
fn images_larger_than_the_gpu_limit_are_scaled_down_to_fit() {
    use studio_core::model::{ProjectItem, ProjectItemKind};
    let path = std::env::temp_dir().join(format!("studio-wide-{}.png", std::process::id()));
    image::RgbaImage::from_pixel(9000, 2, image::Rgba([0, 255, 0, 255])).save(&path).unwrap();
    let mut layer = full(2, 0);
    layer.content = LayerContent::Image { item: Id(60) };
    let mut project = project(vec![layer]);
    project.items.push(ProjectItem {
        id: Id(60),
        name: "wide.png".into(),
        kind: ProjectItemKind::Image,
        label: LabelColor::Aqua,
        parent: None,
        size: Some(Vec2::new(100.0, 100.0)),
        duration_seconds: None,
        frame_rate: None,
        has_audio: false,
        color: None,
        path: Some(path.clone()),
    });
    let frame = render(&project);
    std::fs::remove_file(&path).unwrap();
    assert!(close(pixel(&frame, 50, 50), [0, 255, 0]));
}

#[test]
fn two_layers_of_one_clip_each_show_their_own_moment() {
    use studio_core::model::{ProjectItem, ProjectItemKind};
    // Left half shows source frame 0 (red); right half is slipped one frame later (blue).
    let mut left = solid(2, 0, Vec2::new(50.0, 100.0), Vec2::new(25.0, 50.0));
    let mut right = solid(3, 0, Vec2::new(50.0, 100.0), Vec2::new(75.0, 50.0));
    left.content = LayerContent::Footage { item: Id(50) };
    right.content = LayerContent::Footage { item: Id(50) };
    right.start_frame = -1;
    let mut project = project(vec![left, right]);
    project.items.push(ProjectItem {
        id: Id(50),
        name: "clip.mov".into(),
        kind: ProjectItemKind::Footage,
        label: LabelColor::Aqua,
        parent: None,
        size: Some(Vec2::new(50.0, 100.0)),
        duration_seconds: Some(10.0),
        frame_rate: Some(FrameRate::new(24, 1)),
        has_audio: false,
        color: None,
        path: Some("/clip.mov".into()),
    });
    let video = StripedVideo { asked: Default::default() };
    let frame = Renderer::new().unwrap().render(&project, Id(1), 0, SIZE, SIZE, &video).unwrap();
    assert!(close(pixel(&frame, 25, 50), [255, 0, 0]), "left: {:?}", pixel(&frame, 25, 50));
    assert!(close(pixel(&frame, 75, 50), [0, 0, 255]), "right: {:?}", pixel(&frame, 75, 50));
}

#[test]
fn a_matte_that_draws_nothing_hides_its_layer_instead_of_reusing_another_matte() {
    // Top first: an ended solid used as A's matte, A (blue), a small matte M2, B (red).
    let mut ended = full(2, 0xffffff);
    (ended.in_frame, ended.out_frame) = (5, 10);
    let mut a = full(3, 0x0000ff);
    a.track_matte = TrackMatte::Alpha;
    let m2 = solid(4, 0xffffff, Vec2::new(20.0, 20.0), Vec2::new(50.0, 50.0));
    let mut b = full(5, 0xff0000);
    b.track_matte = TrackMatte::Alpha;
    let frame = render(&project(vec![ended, a, m2, b]));
    // At the center only B shows through its matte; A's matte has ended, so A is hidden.
    assert!(close(pixel(&frame, 50, 50), [255, 0, 0]), "center: {:?}", pixel(&frame, 50, 50));
    assert!(close(pixel(&frame, 10, 10), [0, 0, 0]), "outside both mattes: {:?}", pixel(&frame, 10, 10));
}

#[test]
fn semi_transparent_edges_are_filtered_without_dark_fringes() {
    use studio_core::model::{ProjectItem, ProjectItemKind};
    // A 2×1 image, opaque white and fully transparent, stretched over the frame: halfway
    // across, the color is half-covered white, not darkened by the transparent pixel's black.
    let path = std::env::temp_dir().join(format!("studio-edge-{}.png", std::process::id()));
    let mut edge = image::RgbaImage::new(2, 1);
    edge.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
    edge.put_pixel(1, 0, image::Rgba([0, 0, 0, 0]));
    edge.save(&path).unwrap();
    let mut layer = full(2, 0);
    layer.content = LayerContent::Image { item: Id(61) };
    let mut project = project(vec![layer]);
    project.items.push(ProjectItem {
        id: Id(61),
        name: "edge.png".into(),
        kind: ProjectItemKind::Image,
        label: LabelColor::Aqua,
        parent: None,
        size: Some(Vec2::new(100.0, 100.0)),
        duration_seconds: None,
        frame_rate: None,
        has_audio: false,
        color: None,
        path: Some(path.clone()),
    });
    let frame = render(&project);
    std::fs::remove_file(&path).unwrap();
    // Linear 0.5 coverage of white over black is about 188 in sRGB.
    let middle = pixel(&frame, 50, 50);
    assert!(middle[0] > 175, "midpoint {middle:?}");
}

#[test]
fn an_image_changed_on_disk_is_shown_as_it_is_now() {
    use studio_core::model::{ProjectItem, ProjectItemKind};
    let path = std::env::temp_dir().join(format!("studio-changing-{}.png", std::process::id()));
    let mut layer = full(2, 0);
    layer.content = LayerContent::Image { item: Id(62) };
    let mut project = project(vec![layer]);
    project.items.push(ProjectItem {
        id: Id(62),
        name: "changing.png".into(),
        kind: ProjectItemKind::Image,
        label: LabelColor::Aqua,
        parent: None,
        size: Some(Vec2::new(100.0, 100.0)),
        duration_seconds: None,
        frame_rate: None,
        has_audio: false,
        color: None,
        path: Some(path.clone()),
    });
    let mut renderer = Renderer::new().unwrap();
    let draw = |renderer: &mut Renderer| pixel(&renderer.render(&project, Id(1), 0, SIZE, SIZE, &studio_core::media::NoVideo).unwrap(), 50, 50);
    // Missing at first: the placeholder. Once the file appears, it shows; when it is edited, the edit shows.
    let _ = std::fs::remove_file(&path);
    assert!(!close(draw(&mut renderer), [0, 255, 0]));
    image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 255, 0, 255])).save(&path).unwrap();
    assert!(close(draw(&mut renderer), [0, 255, 0]), "the file appeared");
    // Make sure the modification time moves on, even on coarse file systems.
    std::thread::sleep(std::time::Duration::from_millis(20));
    image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 255, 255])).save(&path).unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(1)).unwrap();
    assert!(close(draw(&mut renderer), [0, 0, 255]), "the edit shows");
    std::fs::remove_file(&path).unwrap();
}

/// Draws `style` in an 80 × 80 box in the middle of the frame, over black.
fn styled_shape(style: ShapeStyle) -> studio_render::RenderedFrame {
    let mut layer = solid(2, 0, Vec2::new(80.0, 80.0), Vec2::new(50.0, 50.0));
    layer.content = LayerContent::Shape { style };
    render(&project(vec![layer]))
}

#[test]
fn gradients_run_across_the_box() {
    let (red, blue) = (Rgba::hex(0xff0000), Rgba::hex(0x0000ff));
    let linear = styled_shape(ShapeStyle { fill: Some(Paint::Linear { start: red, end: blue, angle: 0.0 }), ..ShapeStyle::new(ShapeKind::Rectangle) });
    let (left, right) = (pixel(&linear, 12, 50), pixel(&linear, 88, 50));
    assert!(left[0] > 200 && left[2] < 60 && right[2] > 200 && right[0] < 60, "red on the left, blue on the right: {left:?} {right:?}");
    let down = styled_shape(ShapeStyle { fill: Some(Paint::Linear { start: red, end: blue, angle: 90.0 }), ..ShapeStyle::new(ShapeKind::Rectangle) });
    assert!(pixel(&down, 50, 12)[0] > 200 && pixel(&down, 50, 88)[2] > 200, "90° runs top to bottom");
    let radial = styled_shape(ShapeStyle { fill: Some(Paint::Radial { inner: red, outer: blue }), ..ShapeStyle::new(ShapeKind::Ellipse) });
    let (center, edge) = (pixel(&radial, 50, 50), pixel(&radial, 50, 13));
    assert!(center[0] > 200 && center[2] < 40 && edge[2] > edge[0], "red in the middle, bluer at the edge: {center:?} {edge:?}");
}

#[test]
fn a_stroke_outlines_the_shape_centered_on_its_edge() {
    let style = ShapeStyle {
        fill: None,
        stroke: Some(Stroke { paint: Paint::Solid(Rgba::hex(0xffffff)), width: 8.0 }),
        ..ShapeStyle::new(ShapeKind::Rectangle)
    };
    let frame = styled_shape(style);
    // The box edge is at x = 10; the stroke covers 6..14.
    assert!(pixel(&frame, 8, 50)[0] > 200, "outside the edge");
    assert!(pixel(&frame, 12, 50)[0] > 200, "inside the edge");
    assert!(pixel(&frame, 50, 50)[0] < 20, "no fill, so the middle is empty");
    assert!(pixel(&frame, 3, 50)[0] < 20, "and nothing beyond the stroke");
}

#[test]
fn a_stars_inner_radius_sets_how_deep_its_notches_go() {
    let star = |inner_radius| styled_shape(ShapeStyle { fill: Some(Paint::Solid(Rgba::hex(0xffffff))), inner_radius, ..ShapeStyle::new(ShapeKind::Star) });
    let filled = |frame: &studio_render::RenderedFrame| (0..SIZE).flat_map(|y| (0..SIZE).map(move |x| (x, y))).filter(|&(x, y)| pixel(frame, x, y)[0] > 128).count();
    let (thin, fat) = (filled(&star(0.25)), filled(&star(0.8)));
    assert!(thin * 2 < fat, "a small inner radius makes a spiky star: {thin} vs {fat}");
}

/// Saves every shape (sharp, rounded, then with gradients, strokes and inner radii) to
/// `SHAPE_SHEET` as a PNG for a visual check.
/// Run with `SHAPE_SHEET=/path/shapes.png cargo test -p studio-render --test render shape_sheet -- --ignored`.
#[test]
#[ignore = "writes an image for a visual check"]
fn shape_sheet() {
    let path = std::env::var("SHAPE_SHEET").expect("set SHAPE_SHEET to the PNG to write");
    let (cell, gap) = (200.0, 20.0);
    let violet = Paint::Solid(Rgba::hex(0x8b5cf6));
    let gradient = Paint::Linear { start: Rgba::hex(0xf472b6), end: Rgba::hex(0x60a5fa), angle: 45.0 };
    let radial = Paint::Radial { inner: Rgba::hex(0xfde047), outer: Rgba::hex(0xea580c) };
    let outline = Stroke { paint: Paint::Solid(Rgba::hex(0xffffff)), width: 10.0 };
    let rows: [(f32, Option<Paint>, Option<Stroke>, f32); 4] = [(0.0, Some(violet), None, 0.5), (30.0, Some(violet), None, 0.5), (12.0, Some(gradient), Some(outline), 0.35), (0.0, Some(radial), None, 0.75)];
    let mut layers = Vec::new();
    for (row, (radius, fill, stroke, inner_radius)) in rows.into_iter().enumerate() {
        for (column, kind) in ShapeKind::ALL.into_iter().enumerate() {
            let center = Vec2::new(gap + cell * 0.5 + column as f32 * (cell + gap), gap + cell * 0.5 + row as f32 * (cell + gap));
            let mut layer = solid((10 + row * 5 + column) as u64, 0, Vec2::new(cell, cell * if column == 0 { 0.7 } else { 1.0 }), center);
            layer.content = LayerContent::Shape { style: ShapeStyle { fill, stroke, corner_radius: radius, inner_radius, ..ShapeStyle::new(kind) } };
            layers.push(layer);
        }
    }
    let (width, height) = (5 * 220 + 20, 4 * 220 + 20);
    let mut scene = project(layers);
    (scene.compositions[0].width, scene.compositions[0].height) = (width, height);
    let mut renderer = Renderer::new().unwrap();
    let frame = renderer.render(&scene, Id(1), 0, width, height, &studio_core::media::NoVideo).unwrap();
    let rgba: Vec<u8> = frame.bgra.chunks(4).flat_map(|p| [p[2], p[1], p[0], 255]).collect();
    image::save_buffer(path, &rgba, width, height, image::ColorType::Rgba8).unwrap();
}

/// Saves the newer effects, each on a gradient card with text, to `EFFECT_SHEET` as a PNG.
/// Run with `EFFECT_SHEET=/path/effects.png cargo test -p studio-render --test render effect_sheet -- --ignored`.
#[test]
#[ignore = "writes an image for a visual check"]
fn effect_sheet() {
    let path = std::env::var("EFFECT_SHEET").expect("set EFFECT_SHEET to the PNG to write");
    let names = ["Black & White", "Invert", "Vignette", "Letterbox", "Light Leak", "Strobe", "Wiggle", "Camera Shake", "Glitch", "RGB Split", "VHS", "Scanlines"];
    let (cell, columns) = (240u32, 4u32);
    let rows = (names.len() as u32).div_ceil(columns);
    let mut sheet = vec![0u8; (cell * columns * cell * rows * 4) as usize];
    let mut renderer = Renderer::new().unwrap();
    for (index, name) in names.iter().enumerate() {
        let mut card = solid(2, 0, Vec2::new(cell as f32, cell as f32 * 0.75), Vec2::new(cell as f32 / 2.0, cell as f32 / 2.0));
        card.content = LayerContent::Shape { style: ShapeStyle { fill: Some(Paint::Linear { start: Rgba::hex(0xf97316), end: Rgba::hex(0x3b82f6), angle: 30.0 }), ..ShapeStyle::new(ShapeKind::Rectangle) } };
        card.effects.push(effects::instantiate(effects::find(name).unwrap(), Id(9)));
        let mut text = solid(4, 0, Vec2::new(150.0, 60.0), Vec2::new(cell as f32 / 2.0, cell as f32 / 2.0));
        text.content = LayerContent::Text { text: "Sale".into(), style: TextStyle { font_size: 52.0, line_height: 60.0, fill: Some(Rgba::hex(0xffffff)), ..TextStyle::default() } };
        text.effects.push(effects::instantiate(effects::find(name).unwrap(), Id(10)));
        let mut scene = project(vec![text, card, full(3, 0x202024)]);
        (scene.compositions[0].width, scene.compositions[0].height) = (cell, cell);
        let frame = renderer.render(&scene, Id(1), 5, cell, cell, &studio_core::media::NoVideo).unwrap();
        let (ox, oy) = ((index as u32 % columns) * cell, (index as u32 / columns) * cell);
        for y in 0..cell {
            for x in 0..cell {
                let from = ((y * cell + x) * 4) as usize;
                let to = (((oy + y) * cell * columns + ox + x) * 4) as usize;
                sheet[to..to + 4].copy_from_slice(&[frame.bgra[from + 2], frame.bgra[from + 1], frame.bgra[from], 255]);
            }
        }
    }
    image::save_buffer(path, &sheet, cell * columns, cell * rows, image::ColorType::Rgba8).unwrap();
}
