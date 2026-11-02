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
        mask: None,
        cutout: None,
        text_path: None,
        parent: None,
        switches: LayerSwitches::default(),
        transform: Some(Transform::centered(size, position)),
        effects: Vec::new(),
        has_audio: false,
        audio: Default::default(),
        time: Default::default(),
        animation: Default::default(),
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
        markers: Vec::new(),
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
    layer.content = LayerContent::Shape { style: ShapeStyle { fill: Some(Paint::Solid(Rgba::hex(0xffffff))), corner_radius, ..ShapeStyle::new(kind) }, path: None };
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
        Some(std::sync::Arc::new(studio_core::media::VideoFrame { width: 8, height: 8, index, rgba: color.repeat(64), from_proxy: false, has_alpha: false }))
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
    layer.content = LayerContent::Shape { style, path: None };
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
            layer.content = LayerContent::Shape { style: ShapeStyle { fill, stroke, corner_radius: radius, inner_radius, ..ShapeStyle::new(kind) }, path: None };
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
        card.content = LayerContent::Shape { style: ShapeStyle { fill: Some(Paint::Linear { start: Rgba::hex(0xf97316), end: Rgba::hex(0x3b82f6), angle: 30.0 }), ..ShapeStyle::new(ShapeKind::Rectangle) }, path: None };
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

#[test]
fn captions_show_the_phrase_with_the_spoken_word_highlighted_over_their_box() {
    use studio_core::captions::{CaptionStyle, CaptionWord};
    let words = ["make", "it", "pop"].iter().enumerate().map(|(i, text)| CaptionWord { text: (*text).into(), start: i as i64 * 5, end: i as i64 * 5 + 5 }).collect();
    let style = CaptionStyle {
        text: TextStyle { font_size: 40.0, line_height: 48.0, font_weight: 800, fill: Some(Rgba::hex(0xffffff)), stroke: None, ..TextStyle::default() },
        highlight: Rgba::hex(0xff0000),
        background: Some(Rgba::hex(0x0000ff)),
        ..CaptionStyle::default()
    };
    let (width, height) = (400, 160);
    let mut layer = solid(2, 0, Vec2::new(width as f32, 120.0), Vec2::new(width as f32 / 2.0, height as f32 / 2.0));
    layer.content = LayerContent::Captions { words, style };
    let mut scene = project(vec![layer]);
    (scene.compositions[0].width, scene.compositions[0].height) = (width, height);
    let mut renderer = Renderer::new().unwrap();
    // Frame 7: "it" is being said.
    let frame = renderer.render(&scene, Id(1), 7, width, height, &studio_core::media::NoVideo).unwrap();
    let pixels: Vec<[u8; 3]> = (0..height).flat_map(|y| (0..width).map(move |x| (x, y))).map(|(x, y)| pixel(&frame, x, y)).collect();
    let count = |test: fn(&[u8; 3]) -> bool| pixels.iter().filter(|p| test(p)).count();
    assert!(count(|p| p[0] > 200 && p[1] < 60 && p[2] < 60) > 30, "the spoken word is red");
    assert!(count(|p| p[0] > 200 && p[1] > 200 && p[2] > 200) > 100, "the others are white");
    assert!(count(|p| p[2] > 200 && p[0] < 40) > 2000, "on a blue box");
    assert_eq!(pixel(&frame, 2, 2), [0, 0, 0], "outside the box is clear");
    if let Ok(path) = std::env::var("CAPTION_IMAGE") {
        let rgba: Vec<u8> = frame.bgra.chunks(4).flat_map(|p| [p[2], p[1], p[0], 255]).collect();
        image::save_buffer(path, &rgba, width, height, image::ColorType::Rgba8).unwrap();
    }
}

/// A 100 × 100 composition `id` with `layers` and a transparent background.
fn clear_comp(id: u64, layers: Vec<Layer>) -> Composition {
    Composition {
        id: Id(id),
        name: format!("comp {id}"),
        width: SIZE,
        height: SIZE,
        frame_rate: FrameRate::new(24, 1),
        duration_frames: 10,
        background: Rgba::TRANSPARENT,
        layers,
        markers: Vec::new(),
    }
}

/// A layer showing composition `inner`, filling the frame.
fn nested(id: u64, inner: u64) -> Layer {
    let mut layer = full(id, 0);
    layer.content = LayerContent::Composition { composition: Id(inner) };
    layer
}

#[test]
fn a_nested_composition_shows_its_layers() {
    let inner = clear_comp(20, vec![solid(21, 0xff0000, Vec2::new(40.0, 40.0), Vec2::new(30.0, 30.0))]);
    let outer = clear_comp(1, vec![nested(2, 20), full(3, 0x0000ff)]);
    let project = Project::new(Vec::new(), vec![outer, inner]);
    let frame = render(&project);
    assert!(close(pixel(&frame, 30, 30), [255, 0, 0]), "the inner red square shows: {:?}", pixel(&frame, 30, 30));
    assert!(close(pixel(&frame, 80, 80), [0, 0, 255]), "and the inner comp is see-through elsewhere: {:?}", pixel(&frame, 80, 80));
}

#[test]
fn a_transparent_background_stays_transparent() {
    let project = Project::new(Vec::new(), vec![clear_comp(1, vec![solid(2, 0xffffff, Vec2::new(20.0, 20.0), Vec2::new(50.0, 50.0))])]);
    let frame = render(&project);
    let alpha = |x: u32, y: u32| frame.bgra[((y * frame.width + x) * 4 + 3) as usize];
    assert_eq!(alpha(5, 5), 0, "nothing there");
    assert_eq!(alpha(50, 50), 255, "the solid");
}

#[test]
fn compositions_inside_each_other_stop_rather_than_hang() {
    let a = clear_comp(1, vec![nested(2, 20), solid(3, 0x00ff00, Vec2::new(10.0, 10.0), Vec2::new(50.0, 50.0))]);
    let b = clear_comp(20, vec![nested(21, 1)]);
    let project = Project::new(Vec::new(), vec![a, b]);
    let frame = render(&project);
    assert!(close(pixel(&frame, 50, 50), [0, 255, 0]));
}

/// A pen path layer: a closed triangle (or an open line when `closed` is false) filling an
/// 80 × 80 box in the middle of the frame, white, with an optional stroke.
fn path_layer(id: u64, closed: bool, stroke: Option<Stroke>) -> Layer {
    use studio_core::path::{PathPoint, ShapePath};
    let mut layer = solid(id, 0, Vec2::new(80.0, 80.0), Vec2::new(50.0, 50.0));
    let points = [(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(x, y)| PathPoint::corner(Vec2::new(x, y))).to_vec();
    let style = ShapeStyle { fill: Some(Paint::Solid(Rgba::hex(0xffffff))), stroke, ..ShapeStyle::new(ShapeKind::Path) };
    layer.content = LayerContent::Shape { style, path: Some(ShapePath { points, closed }) };
    layer
}

#[test]
fn pen_paths_fill_their_outline_and_open_ones_are_only_stroked() {
    let lit = |frame: &studio_render::RenderedFrame, x, y| pixel(frame, x, y)[0] > 200;
    let dark = |frame: &studio_render::RenderedFrame, x, y| pixel(frame, x, y)[0] < 40;
    // The triangle's tip is at (50, 10), its base from (10, 90) to (90, 90).
    let triangle = render(&project(vec![path_layer(2, true, None)]));
    assert!(lit(&triangle, 50, 70) && lit(&triangle, 15, 88), "filled inside");
    assert!(dark(&triangle, 15, 15) && dark(&triangle, 85, 15), "and empty outside");
    // Open, it is a red line along two sides, with nothing filled.
    let red = Stroke { paint: Paint::Solid(Rgba::hex(0xff0000)), width: 6.0 };
    let line = render(&project(vec![path_layer(2, false, Some(red))]));
    assert!(pixel(&line, 50, 70)[0] < 40, "nothing filled: {:?}", pixel(&line, 50, 70));
    assert!(close(pixel(&line, 30, 50), [0, 0, 0]), "the missing left side is not drawn");
    assert!(close(pixel(&line, 50, 90), [255, 0, 0]), "the base is");
    assert!(close(pixel(&line, 70, 50), [255, 0, 0]), "the right side is: {:?}", pixel(&line, 70, 50));
}

#[test]
fn a_mask_layer_shows_or_hides_the_layer_it_masks_and_is_not_drawn_itself() {
    use studio_core::model::{LayerMask, MaskMode};
    // A red full-frame solid, masked by the triangle above it.
    let masked = |mode| {
        let mut red = full(3, 0xff0000);
        red.mask = Some(LayerMask { layer: Id(2), mode });
        render(&project(vec![path_layer(2, true, None), red]))
    };
    let shown = masked(MaskMode::Show);
    assert!(close(pixel(&shown, 50, 70), [255, 0, 0]), "red inside the triangle, not the mask's white");
    assert!(close(pixel(&shown, 15, 15), [0, 0, 0]), "nothing outside it");
    let hidden = masked(MaskMode::Hide);
    assert!(close(pixel(&hidden, 50, 70), [0, 0, 0]), "a hole where the triangle is");
    assert!(close(pixel(&hidden, 15, 15), [255, 0, 0]), "red everywhere else");
}

#[test]
fn a_cut_out_shows_only_what_its_mask_keeps_or_only_what_it_removes() {
    use studio_core::model::{Cutout, CutoutClick, ProjectItem, ProjectItemKind};
    let folder = std::env::temp_dir().join(format!("studio-cutout-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let picture = folder.join("red.png");
    image::RgbaImage::from_pixel(50, 50, image::Rgba([255, 0, 0, 255])).save(&picture).unwrap();
    // The mask keeps the left half of the picture (at its own, smaller size).
    image::GrayImage::from_fn(20, 20, |x, _| image::Luma([if x < 10 { 255 } else { 0 }])).save(folder.join("0.png")).unwrap();
    let cut = |remove| {
        let mut layer = full(2, 0);
        layer.content = LayerContent::Image { item: Id(63) };
        let clicks = vec![CutoutClick { at: Vec2::new(0.25, 0.5), keep: true }];
        layer.cutout = Some(Cutout { frame: 0, clicks, remove, masks: Some(folder.clone()), frames: Some((0, 1)) });
        let mut project = project(vec![layer]);
        project.items.push(ProjectItem {
            id: Id(63),
            name: "red.png".into(),
            kind: ProjectItemKind::Image,
            label: LabelColor::Aqua,
            parent: None,
            size: Some(Vec2::new(50.0, 50.0)),
            duration_seconds: None,
            frame_rate: None,
            has_audio: false,
            color: None,
            path: Some(picture.clone()),
        });
        render(&project)
    };
    let kept = cut(false);
    assert!(close(pixel(&kept, 20, 50), [255, 0, 0]), "the kept half shows");
    assert!(close(pixel(&kept, 80, 50), [0, 0, 0]), "the rest is cut away");
    let removed = cut(true);
    assert!(close(pixel(&removed, 20, 50), [0, 0, 0]) && close(pixel(&removed, 80, 50), [255, 0, 0]), "or the other way round");
    std::fs::remove_dir_all(&folder).ok();
}

#[test]
fn text_set_on_a_path_follows_it() {
    use studio_core::model::TextPath;
    use studio_core::path::{PathPoint, ShapePath};
    // A straight path, hidden (it still carries the text), from `from` to `to`.
    let along = |from: (f32, f32), to: (f32, f32)| {
        let mut path = solid(2, 0, Vec2::new(80.0, 80.0), Vec2::new(50.0, 50.0));
        path.switches.visible = false;
        let points = vec![PathPoint::corner(Vec2::new(from.0, from.1)), PathPoint::corner(Vec2::new(to.0, to.1))];
        path.content = LayerContent::Shape { style: ShapeStyle::new(ShapeKind::Path), path: Some(ShapePath { points, closed: false }) };
        let style = TextStyle { font_family: "Helvetica".into(), font_size: 20.0, line_height: 24.0, fill: Some(Rgba::hex(0xffffff)), ..TextStyle::default() };
        let mut text = solid(3, 0, Vec2::new(60.0, 24.0), Vec2::new(50.0, 50.0));
        text.content = LayerContent::Text { text: "HHHH".into(), style };
        text.text_path = Some(TextPath { layer: Id(2), start: 0.0 });
        render(&project(vec![path, text]))
    };
    let lit = |frame: &studio_render::RenderedFrame, xs: std::ops::Range<u32>, ys: std::ops::Range<u32>| {
        ys.flat_map(|y| xs.clone().map(move |x| (x, y))).filter(|&(x, y)| pixel(frame, x, y)[0] > 128).count()
    };
    // Across the middle (the path runs y = 50, from x = 10): letters stand on it, above it.
    let flat = along((0.0, 0.5), (1.0, 0.5));
    assert!(lit(&flat, 10..70, 34..50) > 40, "letters above the line");
    assert_eq!(lit(&flat, 0..100, 55..100), 0, "nothing below it");
    // Down the middle (x = 50, from y = 10): the letters turn to run down it, their tops to the right.
    let down = along((0.5, 0.0), (0.5, 1.0));
    assert!(lit(&down, 50..66, 10..70) > 40, "letters right of the line");
    assert_eq!(lit(&down, 0..45, 0..100), 0, "nothing left of it");
}

/// Renders every template in one format at `TEMPLATE_SECONDS` (default 4) into a contact sheet
/// at `TEMPLATE_SHEET` (a PNG), ten across. `TEMPLATE_FORMAT` is wide, portrait or vertical.
/// Run with `TEMPLATE_SHEET=/path/sheet.png cargo test -p studio-render --test render template_sheet -- --ignored`.
#[test]
#[ignore = "writes an image for a visual check"]
fn template_sheet() {
    use studio_core::templates::{Format, TEMPLATES, build};
    let path = std::env::var("TEMPLATE_SHEET").expect("set TEMPLATE_SHEET to the PNG to write");
    let format = match std::env::var("TEMPLATE_FORMAT").as_deref() {
        Ok("portrait") => Format::Portrait,
        Ok("vertical") => Format::Vertical,
        _ => Format::Wide,
    };
    let seconds: f64 = std::env::var("TEMPLATE_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(4.0);
    let (cell_w, cell_h) = match format {
        Format::Wide => (320u32, 180u32),
        Format::Portrait => (216, 270),
        Format::Vertical => (180, 320),
    };
    // `TEMPLATE_FROM` starts partway through the list (to look at new ones).
    let from: usize = std::env::var("TEMPLATE_FROM").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let shown = &TEMPLATES[from.min(TEMPLATES.len())..];
    let columns = 10u32;
    let rows = (shown.len() as u32).div_ceil(columns);
    let mut sheet = image::RgbaImage::from_pixel(cell_w * columns, cell_h * rows, image::Rgba([40, 40, 40, 255]));
    let mut renderer = Renderer::new().unwrap();
    for (index, template) in shown.iter().enumerate() {
        let mut project = Project::new(Vec::new(), Vec::new());
        let comp = build(&mut project, template, format).unwrap();
        let frame = (seconds * 30.0) as i64;
        // Twice: the first render measures the text, as the app does, so boxes fit their words.
        let first = renderer.render(&project, comp, frame, cell_w * 2, cell_h * 2, &studio_core::media::NoVideo).unwrap();
        for (layer, size) in first.text_sizes {
            studio_core::edit::set_text_box(project.composition_mut(comp).unwrap(), layer, size);
        }
        let picture = renderer.render(&project, comp, frame, cell_w, cell_h, &studio_core::media::NoVideo).unwrap();
        let (x0, y0) = ((index as u32 % columns) * cell_w, (index as u32 / columns) * cell_h);
        for (i, pixel) in picture.bgra.chunks(4).enumerate() {
            let (x, y) = (i as u32 % cell_w, i as u32 / cell_w);
            // Transparent parts over a mid gray, so they read as see-through.
            let a = f32::from(pixel[3]) / 255.0;
            let over = |c: u8| (f32::from(c) * a + 60.0 * (1.0 - a)) as u8;
            sheet.put_pixel(x0 + x, y0 + y, image::Rgba([over(pixel[2]), over(pixel[1]), over(pixel[0]), 255]));
        }
    }
    sheet.save(path).unwrap();
}

#[test]
fn emoji_keep_their_own_colors() {
    let style = TextStyle { font_family: "Apple Color Emoji".into(), font_size: 60.0, line_height: 70.0, fill: Some(Rgba::hex(0xffffff)), ..TextStyle::default() };
    let mut text = solid(2, 0, Vec2::new(70.0, 70.0), Vec2::new(50.0, 50.0));
    text.content = LayerContent::Text { text: "🔥".into(), style };
    let frame = render(&project(vec![text]));
    // A flame is orange and red: plenty of pixels much redder than they are blue. A white
    // silhouette would have none.
    let fiery = (0..SIZE).flat_map(|y| (0..SIZE).map(move |x| (x, y))).filter(|&(x, y)| {
        let [r, _, b] = pixel(&frame, x, y);
        r > 180 && b < 90
    });
    assert!(fiery.count() > 40);
}

/// Renders every element on a dark square into a contact sheet at `ELEMENT_SHEET`.
#[test]
#[ignore = "writes an image for a visual check"]
fn element_sheet() {
    use studio_core::elements::{ELEMENTS, add};
    let path = std::env::var("ELEMENT_SHEET").expect("set ELEMENT_SHEET to the PNG to write");
    let (cell, columns) = (180u32, 9u32);
    let rows = (ELEMENTS.len() as u32).div_ceil(columns);
    let mut sheet = image::RgbaImage::from_pixel(cell * columns, cell * rows, image::Rgba([40, 40, 40, 255]));
    let mut renderer = Renderer::new().unwrap();
    for index in 0..ELEMENTS.len() {
        let mut composition = clear_comp(1, Vec::new());
        (composition.width, composition.height, composition.background) = (1080, 1080, Rgba::hex(0x3b3b44));
        composition.duration_frames = 10;
        let mut project = Project::new(Vec::new(), vec![composition]);
        add(&mut project, Id(1), index, Vec2::new(540.0, 540.0), 0);
        let first = renderer.render(&project, Id(1), 0, cell, cell, &studio_core::media::NoVideo).unwrap();
        for (layer, size) in first.text_sizes {
            studio_core::edit::set_text_box(project.composition_mut(Id(1)).unwrap(), layer, size);
        }
        let picture = renderer.render(&project, Id(1), 0, cell, cell, &studio_core::media::NoVideo).unwrap();
        let (x0, y0) = ((index as u32 % columns) * cell, (index as u32 / columns) * cell);
        for (i, pixel) in picture.bgra.chunks(4).enumerate() {
            sheet.put_pixel(x0 + i as u32 % cell, y0 + i as u32 / cell, image::Rgba([pixel[2], pixel[1], pixel[0], 255]));
        }
    }
    sheet.save(path).unwrap();
}

/// Imports an After Effects project or a Lottie file and renders each composition halfway
/// through into a contact sheet, to check an import by eye.
/// Run with `IMPORT_FILE=/path/project.aep IMPORT_SHEET=/path/sheet.png cargo test -p studio-render --test render import_sheet -- --ignored`.
#[test]
#[ignore = "needs a file to import; writes an image for a visual check"]
fn import_sheet() {
    use studio_core::import::{aep, build, lottie};
    let file = std::path::PathBuf::from(std::env::var("IMPORT_FILE").expect("set IMPORT_FILE"));
    let sheet_path = std::env::var("IMPORT_SHEET").expect("set IMPORT_SHEET to the PNG to write");
    let bytes = std::fs::read(&file).unwrap();
    let scene = if file.extension().is_some_and(|e| e == "aep") { aep::read(&bytes).unwrap() } else { lottie::read(std::str::from_utf8(&bytes).unwrap(), &file, &std::env::temp_dir()).unwrap() };
    let mut project = Project::new(Vec::new(), Vec::new());
    let imported = build(&mut project, &scene, "import", &|_| None).unwrap();
    let (cell_w, cell_h, columns) = (320u32, 180u32, 6u32);
    let shown: Vec<_> = imported.compositions.iter().take(36).collect();
    let rows = (shown.len() as u32).div_ceil(columns);
    let mut sheet = image::RgbaImage::from_pixel(cell_w * columns, cell_h * rows, image::Rgba([40, 40, 40, 255]));
    let mut renderer = Renderer::new().unwrap();
    for (index, &&comp) in shown.iter().enumerate() {
        let (frame, name) = project.composition(comp).map(|c| (c.duration_frames / 2, c.name.clone())).unwrap();
        let first = renderer.render(&project, comp, frame, cell_w * 2, cell_h * 2, &studio_core::media::NoVideo).unwrap();
        for (layer, size) in first.text_sizes {
            studio_core::edit::set_text_box(project.composition_mut(comp).unwrap(), layer, size);
        }
        let picture = renderer.render(&project, comp, frame, cell_w, cell_h, &studio_core::media::NoVideo).unwrap();
        let (x0, y0) = ((index as u32 % columns) * cell_w, (index as u32 / columns) * cell_h);
        for (i, pixel) in picture.bgra.chunks(4).enumerate() {
            let (x, y) = (i as u32 % cell_w, i as u32 / cell_w);
            let a = f32::from(pixel[3]) / 255.0;
            let over = |c: u8| (f32::from(c) * a + 60.0 * (1.0 - a)) as u8;
            sheet.put_pixel(x0 + x, y0 + y, image::Rgba([over(pixel[2]), over(pixel[1]), over(pixel[0]), 255]));
        }
        println!("{index}: {name}");
    }
    sheet.save(sheet_path).unwrap();
}

/// A white 20 × 20 square with a strong Glow, in the middle of the frame.
fn glowing_square(id: u64) -> Layer {
    let mut square = solid(id, 0xffffff, Vec2::new(20.0, 20.0), Vec2::new(50.0, 50.0));
    let mut glow = effects::instantiate(effects::find("Glow").unwrap(), Id(id + 100));
    glow.properties[0].value = Animated::constant(20.0);
    glow.properties[2].value = Animated::constant(2.0);
    square.effects.push(glow);
    square
}

#[test]
fn a_glow_inside_a_transparent_precomp_still_shows() {
    let direct = render(&project(vec![glowing_square(2)]));
    let inner = clear_comp(20, vec![glowing_square(21)]);
    let mut outer = project(vec![nested(2, 20)]);
    outer.compositions.push(inner);
    let nested = render(&outer);
    // Just outside the square, where only the halo reaches.
    let (halo, through) = (pixel(&direct, 38, 50), pixel(&nested, 38, 50));
    assert!(halo[0] > 60, "the halo shows when drawn directly: {halo:?}");
    assert!(close(through, halo), "and the same through a precomp: {through:?} vs {halo:?}");
}

#[test]
fn a_hidden_precomp_still_works_as_a_matte() {
    let inner = clear_comp(20, vec![solid(21, 0xffffff, Vec2::new(40.0, 40.0), Vec2::new(50.0, 50.0))]);
    let mut matte = nested(2, 20);
    matte.switches.visible = false;
    let mut blue = full(3, 0x0000ff);
    blue.track_matte = TrackMatte::Alpha;
    let mut project = project(vec![matte, blue]);
    project.compositions.push(inner);
    let frame = render(&project);
    assert!(close(pixel(&frame, 50, 50), [0, 0, 255]), "inside the matte: {:?}", pixel(&frame, 50, 50));
    assert!(close(pixel(&frame, 5, 5), [0, 0, 0]), "outside it: {:?}", pixel(&frame, 5, 5));
}

#[test]
fn a_mattes_opacity_fades_what_it_reveals() {
    let mut matte = full(2, 0xffffff);
    matte.transform.as_mut().unwrap().opacity = Animated::constant(0.0);
    let mut blue = full(3, 0x0000ff);
    blue.track_matte = TrackMatte::Alpha;
    let frame = render(&project(vec![matte, blue]));
    assert!(close(pixel(&frame, 50, 50), [0, 0, 0]), "a fully faded matte reveals nothing: {:?}", pixel(&frame, 50, 50));
}

/// A full-frame adjustment layer that turns what is below it gray.
fn gray_adjustment(id: u64) -> Layer {
    let mut adjustment = full(id, 0x000000);
    adjustment.content = LayerContent::Adjustment;
    adjustment.effects.push(effects::instantiate(effects::find("Tint").unwrap(), Id(id + 100)));
    adjustment
}

fn is_gray(color: [u8; 3]) -> bool {
    color[0] == color[1] && color[1] == color[2] && color[0] > 20
}

#[test]
fn an_adjustment_layer_applies_only_inside_its_track_matte() {
    // A 40 × 40 matte in the middle; the adjustment below it uses it.
    let matte = solid(2, 0xffffff, Vec2::new(40.0, 40.0), Vec2::new(50.0, 50.0));
    let mut adjustment = gray_adjustment(3);
    adjustment.track_matte = TrackMatte::Alpha;
    let frame = render(&project(vec![matte, adjustment, full(4, 0xff0000)]));
    assert!(is_gray(pixel(&frame, 50, 50)), "inside the matte: {:?}", pixel(&frame, 50, 50));
    assert!(close(pixel(&frame, 5, 5), [255, 0, 0]), "outside it, untouched: {:?}", pixel(&frame, 5, 5));
}

#[test]
fn an_adjustment_layers_mask_limits_where_it_applies() {
    let mask = solid(2, 0xffffff, Vec2::new(40.0, 40.0), Vec2::new(50.0, 50.0));
    let mut adjustment = gray_adjustment(3);
    adjustment.mask = Some(studio_core::model::LayerMask { layer: Id(2), mode: studio_core::model::MaskMode::Hide });
    let frame = render(&project(vec![mask, adjustment, full(4, 0xff0000)]));
    assert!(close(pixel(&frame, 50, 50), [255, 0, 0]), "hidden inside the mask: {:?}", pixel(&frame, 50, 50));
    assert!(is_gray(pixel(&frame, 5, 5)), "applied outside it: {:?}", pixel(&frame, 5, 5));
}

/// Every device mockup, upright and on its side, with a bright screen.
/// Run with `MOCKUP_SHEET=/path/sheet.png cargo test -p studio-render --test render mockup_sheet -- --ignored`
/// (`MOCKUP_FORMAT=wide|portrait|vertical`, `MOCKUP_FINISH=` a preset's position, `MOCKUP_ZOOM=` 1 to 4).
#[test]
#[ignore = "writes an image for a visual check"]
fn mockup_sheet() {
    use studio_core::mockups::{Device, Finish, MockupRequest, Orientation, build};
    use studio_core::templates::Format;
    let path = std::env::var("MOCKUP_SHEET").expect("set MOCKUP_SHEET to the PNG to write");
    let format = match std::env::var("MOCKUP_FORMAT").as_deref() {
        Ok("portrait") => Format::Portrait,
        Ok("vertical") => Format::Vertical,
        _ => Format::Wide,
    };
    let finish = Finish::presets()[std::env::var("MOCKUP_FINISH").ok().and_then(|s| s.parse().ok()).unwrap_or(0usize).min(3)].1;
    let zoom: u32 = std::env::var("MOCKUP_ZOOM").ok().and_then(|s| s.parse().ok()).unwrap_or(1).clamp(1, 4);
    let (cell_w, cell_h) = match format {
        Format::Wide => (480u32 * zoom, 270u32 * zoom),
        Format::Portrait => (320 * zoom, 400 * zoom),
        Format::Vertical => (270 * zoom, 480 * zoom),
    };
    let cases: Vec<(Device, Orientation)> = Device::ALL.iter().flat_map(|&device| [(device, Orientation::Portrait), (device, Orientation::Landscape)]).filter(|(device, orientation)| device.turns() || *orientation == Orientation::Portrait).collect();
    let mut sheet = image::RgbaImage::from_pixel(cell_w * cases.len() as u32, cell_h, image::Rgba([40, 40, 40, 255]));
    let mut renderer = Renderer::new().unwrap();
    for (index, &(device, orientation)) in cases.iter().enumerate() {
        let mut project = Project::new(Vec::new(), Vec::new());
        let screen_color = project.allocate_id();
        project.items.push(studio_core::model::ProjectItem {
            id: screen_color,
            name: "Screen".into(),
            kind: studio_core::model::ProjectItemKind::Solid,
            label: LabelColor::Red,
            parent: None,
            size: Some(Vec2::new(100.0, 100.0)),
            duration_seconds: None,
            frame_rate: None,
            has_audio: false,
            color: Some(Rgba::hex(0xff5a36)),
            path: None,
        });
        let made = build(&mut project, &MockupRequest { device, orientation, format, finish, footage: Some(screen_color), seconds: 10.0 }).unwrap();
        let picture = renderer.render(&project, made.composition, 0, cell_w, cell_h, &studio_core::media::NoVideo).unwrap();
        for (i, pixel) in picture.bgra.chunks(4).enumerate() {
            let (x, y) = (i as u32 % cell_w, i as u32 / cell_w);
            let a = f32::from(pixel[3]) / 255.0;
            let over = |c: u8| (f32::from(c) * a + 60.0 * (1.0 - a)) as u8;
            sheet.put_pixel(index as u32 * cell_w + x, y, image::Rgba([over(pixel[2]), over(pixel[1]), over(pixel[0]), 255]));
        }
    }
    sheet.save(path).unwrap();
}

#[test]
fn frames_past_the_size_limit_keep_their_shape() {
    let mut project = project(vec![full(2, 0xff0000)]);
    project.compositions[0].width = 5120;
    project.compositions[0].height = 2880;
    let mut renderer = Renderer::new().unwrap();
    let frame = renderer.render(&project, Id(1), 0, 10240, 5760, &studio_core::media::NoVideo).unwrap();
    assert_eq!((frame.width, frame.height), (8192, 4608), "scaled down as a whole, not squashed");
}

#[test]
fn many_copies_of_nested_compositions_still_draw_quickly() {
    // Eight compositions, each showing six copies of the next: millions of nested draws if
    // nothing limited them.
    let mut comps = Vec::new();
    for level in 0..8u64 {
        let id = 1 + level * 100;
        let layers = if level == 7 {
            vec![full(id + 1, 0x00ff00)]
        } else {
            (0..6).map(|copy| nested(id + 1 + copy, 1 + (level + 1) * 100)).collect()
        };
        comps.push(clear_comp(id, layers));
    }
    let project = Project::new(Vec::new(), comps);
    let started = std::time::Instant::now();
    let mut renderer = Renderer::new().unwrap();
    renderer.render(&project, Id(1), 0, SIZE, SIZE, &studio_core::media::NoVideo).unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "took {:?}", started.elapsed());
}
