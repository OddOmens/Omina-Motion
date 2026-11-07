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
        morph: None,
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
        motion_blur: Default::default(),
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

/// A full-frame solid of `color` with the catalog effect `name`, its parameters set to `values`
/// (by index; the rest at their defaults), rendered; returns the middle pixel.
fn graded(color: u32, name: &str, values: &[(usize, f32)]) -> [u8; 3] {
    let mut layer = full(2, color);
    let mut effect = effects::instantiate(effects::find(name).unwrap(), Id(9));
    for &(index, value) in values {
        effect.properties[index].value = Animated::constant(value);
    }
    layer.effects.push(effect);
    pixel(&render(&project(vec![layer])), 50, 50)
}

#[test]
fn basic_correction_balances_exposes_and_saturates() {
    let gray = 0x808080;
    assert!(close(graded(gray, "Basic Correction", &[]), [128, 128, 128]), "the defaults change nothing");
    let warm = graded(gray, "Basic Correction", &[(0, 60.0)]);
    assert!(warm[0] > warm[2] + 20, "Temperature warms: {warm:?}");
    let magenta = graded(gray, "Basic Correction", &[(1, 60.0)]);
    assert!(magenta[1] + 15 < magenta[0], "Tint goes toward magenta: {magenta:?}");
    assert!(graded(gray, "Basic Correction", &[(2, 1.0)])[0] > 160, "one stop brighter");
    let flat = graded(0xcc3322, "Basic Correction", &[(8, 0.0)]);
    assert!(flat[0] == flat[1] && flat[1] == flat[2], "no saturation is gray: {flat:?}");
    assert!(graded(0x303030, "Basic Correction", &[(5, 100.0)])[0] > 0x30 + 15, "Shadows lifts dark tones");
    assert!(graded(0xd0d0d0, "Basic Correction", &[(4, -100.0)])[0] < 0xd0 - 15, "Highlights pulls bright tones down");
}

#[test]
fn color_wheels_tint_shadows_and_highlights_apart() {
    assert!(close(graded(0x808080, "Color Wheels", &[]), [128, 128, 128]));
    // Highlights pushed toward blue turn a light gray blue, and leave black alone.
    let light = graded(0xd0d0d0, "Color Wheels", &[(6, 240.0), (7, 100.0)]);
    assert!(light[2] > light[0] + 20, "{light:?}");
    assert!(close(graded(0x000000, "Color Wheels", &[(6, 240.0), (7, 100.0)]), [0, 0, 0]));
    // Lifting the shadows brightens black.
    assert!(graded(0x000000, "Color Wheels", &[(2, 60.0)])[0] > 20);
}

#[test]
fn levels_and_the_tone_curve_remap_brightness() {
    assert!(close(graded(0x808080, "Levels", &[(1, 128.0)]), [255, 255, 255]), "input white at middle gray");
    assert!(close(graded(0xffffff, "Levels", &[(4, 200.0)]), [200, 200, 200]), "output white");
    assert!(close(graded(0x808080, "Tone Curve", &[]), [128, 128, 128]));
    assert!(graded(0x303030, "Tone Curve", &[(3, 100.0)])[0] > 0x30 + 10, "Shadows lifts the darkest band");
    assert!(close(graded(0xffffff, "Tone Curve", &[(0, -100.0)]), [255, 255, 255]), "white stays white");
}

/// A full-frame red solid under an adjustment layer whose Apply LUT uses `lut` at `intensity` %.
fn lut_frame(lut: Option<studio_core::model::LutSource>, intensity: f32) -> [u8; 3] {
    let mut grade = full(2, 0x000000);
    grade.content = LayerContent::Adjustment;
    let mut effect = effects::instantiate(effects::find("Apply LUT").unwrap(), Id(9));
    effect.properties[0].value = Animated::constant(intensity);
    effect.lut = lut;
    grade.effects.push(effect);
    pixel(&render(&project(vec![grade, full(3, 0xcc3322)])), 50, 50)
}

#[test]
fn luts_grade_through_adjustment_layers() {
    use studio_core::model::LutSource;
    let original = [0xcc, 0x33, 0x22];
    assert!(close(lut_frame(None, 100.0), original), "no table chosen draws the picture unchanged");
    let noir = lut_frame(Some(LutSource::Look("Noir".into())), 100.0);
    assert!(noir[0] == noir[1] && noir[1] == noir[2], "Noir is black and white: {noir:?}");
    assert!(close(lut_frame(Some(LutSource::Look("Noir".into())), 0.0), original), "0% intensity");

    // A .cube file that swaps red and blue, written to disk and read back like a user's LUT.
    let folder = std::env::temp_dir().join(format!("studio-lut-test-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("Swap.cube");
    let swap = studio_core::grade::Lut::from_fn("Swap", 17, |[r, g, b]| [b, g, r]);
    std::fs::write(&path, swap.to_cube()).unwrap();
    let swapped = lut_frame(Some(LutSource::File(path.clone())), 100.0);
    assert!(close(swapped, [0x22, 0x33, 0xcc]), "red and blue swapped: {swapped:?}");
    // A missing or broken file draws the picture as it is instead of failing the frame.
    std::fs::write(&path, "not a lut").unwrap();
    assert!(close(lut_frame(Some(LutSource::File(path)), 100.0), original));
    assert!(close(lut_frame(Some(LutSource::File(folder.join("Gone.cube"))), 100.0), original));
    let _ = std::fs::remove_dir_all(folder);
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
        let mut template = effects::instantiate(definition, Id(9));
        // Apply LUT draws nothing until a table is chosen.
        if definition.name == "Apply LUT" {
            template.lut = Some(studio_core::model::LutSource::Look("Noir".into()));
        }
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
    let linear = styled_shape(ShapeStyle { fill: Some(Paint::linear(red, blue, 0.0)), ..ShapeStyle::new(ShapeKind::Rectangle) });
    let (left, right) = (pixel(&linear, 12, 50), pixel(&linear, 88, 50));
    assert!(left[0] > 200 && left[2] < 60 && right[2] > 200 && right[0] < 60, "red on the left, blue on the right: {left:?} {right:?}");
    let down = styled_shape(ShapeStyle { fill: Some(Paint::linear(red, blue, 90.0)), ..ShapeStyle::new(ShapeKind::Rectangle) });
    assert!(pixel(&down, 50, 12)[0] > 200 && pixel(&down, 50, 88)[2] > 200, "90° runs top to bottom");
    let radial = styled_shape(ShapeStyle { fill: Some(Paint::radial(red, blue)), ..ShapeStyle::new(ShapeKind::Ellipse) });
    let (center, edge) = (pixel(&radial, 50, 50), pixel(&radial, 50, 13));
    assert!(center[0] > 200 && center[2] < 40 && edge[2] > edge[0], "red in the middle, bluer at the edge: {center:?} {edge:?}");
}

#[test]
fn gradient_stops_and_points_place_the_colors() {
    use studio_core::gradient::{Gradient, GradientStop};
    let (red, green, blue) = (Rgba::hex(0xff0000), Rgba::hex(0x00ff00), Rgba::hex(0x0000ff));
    // Red, green half way, blue; running over the left half of the box only.
    let stops = [GradientStop { at: 0.0, color: red }, GradientStop { at: 0.5, color: green }, GradientStop { at: 1.0, color: blue }];
    let gradient = Gradient::new(&stops, Vec2::new(0.0, 0.5), Vec2::new(0.5, 0.5));
    let check = |frame: &studio_render::RenderedFrame, what: &str| {
        // The 80-pixel box spans x 10–90: the gradient ends at x 50.
        let (start, middle, past) = (pixel(frame, 11, 50), pixel(frame, 30, 50), pixel(frame, 75, 50));
        assert!(start[0] > 200 && start[1] < 100, "{what}: red at the start: {start:?}");
        assert!(middle[1] > 200 && middle[0] < 60 && middle[2] < 60, "{what}: green a quarter across: {middle:?}");
        assert!(past[2] > 200 && past[0] < 60 && past[1] < 60, "{what}: blue past the end: {past:?}");
    };
    let style = ShapeStyle { fill: Some(Paint::Linear(gradient)), ..ShapeStyle::new(ShapeKind::Rectangle) };
    check(&styled_shape(style), "drawn shape");
    // The same paint on a drawn path (filled by tiny-skia) puts the colors in the same places.
    let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(x, y)| studio_core::path::PathPoint::corner(Vec2::new(x, y)));
    let path = studio_core::path::ShapePath { points: corners.to_vec(), closed: true };
    let mut layer = solid(2, 0, Vec2::new(80.0, 80.0), Vec2::new(50.0, 50.0));
    layer.content = LayerContent::Shape { style: ShapeStyle { kind: ShapeKind::Path, ..style }, path: Some(path) };
    check(&render(&project(vec![layer])), "path");
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
    let gradient = Paint::linear(Rgba::hex(0xf472b6), Rgba::hex(0x60a5fa), 45.0);
    let radial = Paint::radial(Rgba::hex(0xfde047), Rgba::hex(0xea580c));
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

/// A shape layer of `kind` filled with `fill`, `size` across, centered on `center`.
fn shape(id: u64, kind: ShapeKind, fill: Paint, size: Vec2, center: Vec2) -> Layer {
    let mut layer = solid(id, 0, size, center);
    layer.content = LayerContent::Shape { style: ShapeStyle { fill: Some(fill), ..ShapeStyle::new(kind) }, path: None };
    layer
}

/// Saves every catalog effect at its defaults, one labelled cell each, to `EFFECT_SHEET` as a PNG.
/// The test picture is a composition with a colorful gradient card, a green-screen patch with a
/// face in front of it, and a star on a transparent surround, so moves, colors, keys and
/// outlines all show. Transitions are drawn half done, and a few effects are shown again at
/// other settings to check their direction and reach.
/// Run with `EFFECT_SHEET=/path/effects.png cargo test -p studio-render --test render effect_sheet -- --ignored`.
#[test]
#[ignore = "writes an image for a visual check"]
fn effect_sheet() {
    let path = std::env::var("EFFECT_SHEET").expect("set EFFECT_SHEET to the PNG to write");
    let (cell, columns) = (180u32, 8u32);
    let size = cell as f32;
    // Each cell: its label, the effect, and parameters changed from the defaults. The first cell
    // is the picture with no effect, for comparison.
    type Cell<'a> = (String, &'a str, Vec<(usize, f32)>);
    let mut cells: Vec<Cell> = vec![("(no effect)".into(), "", vec![])];
    cells.extend(effects::CATEGORIES.iter().flat_map(|category| category.effects).map(|effect| (effect.name.to_owned(), effect.name, vec![])));
    cells.push(("Crop 20 10 15 5, feather 8".into(), "Crop", vec![(0, 20.0), (1, 10.0), (2, 15.0), (3, 5.0), (4, 8.0)]));
    cells.push(("Offset 25%, 10%".into(), "Offset", vec![(0, 25.0), (1, 10.0)]));
    cells.push(("Mirror 90°".into(), "Mirror", vec![(0, 90.0)]));
    cells.push(("Motion Tile mirrored".into(), "Motion Tile", vec![(2, 1.0)]));
    cells.push(("Stroke 10 px".into(), "Stroke", vec![(0, 10.0)]));
    cells.push(("Key Cleaner 6 px".into(), "Key Cleaner", vec![(0, 6.0)]));
    cells.push(("Median 6 px".into(), "Median", vec![(0, 6.0)]));
    cells.push(("Iris Wipe 80%, feather 20".into(), "Iris Wipe", vec![(0, 80.0), (1, 20.0)]));
    cells.push(("Gradient Wipe 80%".into(), "Gradient Wipe", vec![(0, 80.0)]));
    cells.push(("Blinds 30%, 45°".into(), "Venetian Blinds", vec![(0, 30.0), (1, 45.0)]));
    let rows = (cells.len() as u32).div_ceil(columns);
    let mut sheet = vec![0u8; (cell * columns * cell * rows * 4) as usize];
    let mut renderer = Renderer::new().unwrap();
    let card = shape(21, ShapeKind::Rectangle, Paint::linear(Rgba::hex(0xf97316), Rgba::hex(0x3b82f6), 30.0), Vec2::new(105.0, 95.0), Vec2::new(62.0, 62.0));
    let screen = shape(22, ShapeKind::Rectangle, Paint::Solid(Rgba::hex(0x00b140)), Vec2::new(62.0, 80.0), Vec2::new(138.0, 60.0));
    let face = shape(23, ShapeKind::Ellipse, Paint::radial(Rgba::hex(0xf1c6a6), Rgba::hex(0xa8694a)), Vec2::new(30.0, 38.0), Vec2::new(138.0, 60.0));
    let star = shape(24, ShapeKind::Star, Paint::Solid(Rgba::hex(0xfacc15)), Vec2::new(46.0, 46.0), Vec2::new(120.0, 128.0));
    let mut picture = clear_comp(20, vec![star, face, screen, card]);
    (picture.width, picture.height) = (cell, cell);
    for (index, (text, name, values)) in cells.iter().enumerate() {
        let mut layer = nested(2, 20);
        layer.size = Vec2::new(size, size);
        layer.transform = Some(Transform::centered(layer.size, Vec2::new(size / 2.0, size / 2.0)));
        if !name.is_empty() {
            let mut effect = effects::instantiate(effects::find(name).unwrap(), Id(9));
            for property in &mut effect.properties {
                if property.name == "Transition Completion" {
                    property.value = Animated::constant(50.0);
                }
            }
            for &(parameter, value) in values {
                effect.properties[parameter].value = Animated::constant(value);
            }
            if *name == "Apply LUT" {
                effect.lut = studio_core::grade::looks().next().map(|look| studio_core::model::LutSource::Look(look.to_owned()));
            }
            layer.effects.push(effect);
        }
        // Text is drawn to fill its box, so the box is sized roughly to the label.
        let width = (text.chars().count() as f32 * 6.5).min(size - 6.0);
        let mut label = solid(4, 0, Vec2::new(width, 16.0), Vec2::new(size / 2.0, size - 11.0));
        label.content = LayerContent::Text { text: text.clone(), style: TextStyle { font_size: 12.0, line_height: 16.0, fill: Some(Rgba::hex(0xffffff)), ..TextStyle::default() } };
        let backdrop = solid(3, 0x2a2a30, Vec2::new(size, size), Vec2::new(size / 2.0, size / 2.0));
        let mut scene = project(vec![label, layer, backdrop]);
        (scene.compositions[0].width, scene.compositions[0].height) = (cell, cell);
        scene.compositions.push(picture.clone());
        let frame = renderer.render(&scene, Id(1), 5, cell, cell, &studio_core::media::NoVideo).unwrap();
        let (ox, oy) = ((index as u32 % columns) * cell, (index as u32 / columns) * cell);
        for y in 0..cell {
            for x in 0..cell {
                let from = ((y * cell + x) * 4) as usize;
                let to = (((oy + y) * cell * columns + ox + x) * 4) as usize;
                // A one-pixel gap between cells keeps neighbouring effects apart.
                let edge = x == cell - 1 || y == cell - 1;
                let color = if edge { [0, 0, 0, 255] } else { [frame.bgra[from + 2], frame.bgra[from + 1], frame.bgra[from], 255] };
                sheet[to..to + 4].copy_from_slice(&color);
            }
        }
    }
    image::save_buffer(path, &sheet, cell * columns, cell * rows, image::ColorType::Rgba8).unwrap();
}

/// Renders a picture through each built-in look and a few grading settings, on an adjustment
/// layer as the whole-video grade does, into one sheet (the first cell is ungraded).
#[test]
#[ignore = "writes an image for a visual check"]
fn grade_sheet() {
    use studio_core::model::LutSource;
    let path = std::env::var("GRADE_SHEET").expect("set GRADE_SHEET to the PNG to write");
    // Each cell: the LUT (for Apply LUT), the effect, and the parameters set on it.
    type Grade<'a> = (Option<LutSource>, &'a str, Vec<(usize, f32)>);
    let mut grades: Vec<Grade> = vec![(None, "", vec![])];
    grades.extend(studio_core::grade::looks().map(|look| (Some(LutSource::Look(look.to_owned())), "Apply LUT", vec![])));
    grades.push((None, "Basic Correction", vec![(0, 50.0), (3, 30.0), (8, 130.0)]));
    grades.push((None, "Color Wheels", vec![(0, 200.0), (1, 60.0), (6, 30.0), (7, 50.0)]));
    grades.push((None, "Tone Curve", vec![(0, -60.0), (3, 60.0)]));
    grades.push((None, "Levels", vec![(0, 30.0), (1, 220.0), (2, 1.2)]));
    let (cell, columns) = (200u32, 6u32);
    let rows = (grades.len() as u32).div_ceil(columns);
    let mut sheet = vec![0u8; (cell * columns * cell * rows * 4) as usize];
    let mut renderer = Renderer::new().unwrap();
    let size = cell as f32;
    for (index, (lut, name, values)) in grades.iter().enumerate() {
        let mut sky = solid(5, 0, Vec2::new(size, size * 0.55), Vec2::new(size / 2.0, size * 0.275));
        sky.content = LayerContent::Shape { style: ShapeStyle { fill: Some(Paint::linear(Rgba::hex(0x5fa8e8), Rgba::hex(0xf7c08a), 180.0)), ..ShapeStyle::new(ShapeKind::Rectangle) }, path: None };
        let mut ground = solid(6, 0, Vec2::new(size, size * 0.45), Vec2::new(size / 2.0, size * 0.775));
        ground.content = LayerContent::Shape { style: ShapeStyle { fill: Some(Paint::linear(Rgba::hex(0x4c7a3a), Rgba::hex(0x1d2a17), 180.0)), ..ShapeStyle::new(ShapeKind::Rectangle) }, path: None };
        let mut face = solid(7, 0, Vec2::new(size * 0.3, size * 0.3), Vec2::new(size * 0.5, size * 0.55));
        face.content = LayerContent::Shape { style: ShapeStyle { fill: Some(Paint::radial(Rgba::hex(0xf1c6a6), Rgba::hex(0xa8694a))), ..ShapeStyle::new(ShapeKind::Ellipse) }, path: None };
        let mut grade = full(2, 0);
        grade.content = LayerContent::Adjustment;
        grade.size = Vec2::new(size, size);
        grade.transform = Some(Transform::centered(grade.size, Vec2::new(size / 2.0, size / 2.0)));
        if !name.is_empty() {
            let mut effect = effects::instantiate(effects::find(name).unwrap(), Id(9));
            effect.lut = lut.clone();
            for &(parameter, value) in values {
                effect.properties[parameter].value = Animated::constant(value);
            }
            grade.effects.push(effect);
        }
        let mut scene = project(vec![grade, face, ground, sky]);
        (scene.compositions[0].width, scene.compositions[0].height) = (cell, cell);
        let frame = renderer.render(&scene, Id(1), 0, cell, cell, &studio_core::media::NoVideo).unwrap();
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
    let words = ["make", "it", "pop"].iter().enumerate().map(|(i, text)| CaptionWord::new(*text, i as i64 * 5, i as i64 * 5 + 5)).collect();
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
        motion_blur: Default::default(),
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

/// Renders chosen templates across their running time, one row of frames each, into
/// `TEMPLATE_STRIP` (a PNG), to check how they move. `TEMPLATE_PICK` lists template numbers
/// (such as "0,5,10"), `TEMPLATE_FORMAT` is wide, portrait or vertical, and `TEMPLATE_TIMES`
/// lists the seconds to show (a negative one counts back from the end).
/// Run with `TEMPLATE_STRIP=/path/strip.png TEMPLATE_PICK=0,5 cargo test -p studio-render --test render template_strip -- --ignored`.
#[test]
#[ignore = "writes an image for a visual check"]
fn template_strip() {
    use studio_core::templates::{Format, TEMPLATES, build};
    let path = std::env::var("TEMPLATE_STRIP").expect("set TEMPLATE_STRIP to the PNG to write");
    let numbers = |name: &str, fallback: &str| -> Vec<f64> { std::env::var(name).unwrap_or_else(|_| fallback.to_owned()).split(',').filter_map(|part| part.trim().parse().ok()).collect() };
    let picks: Vec<usize> = numbers("TEMPLATE_PICK", "0").into_iter().map(|number| number as usize).filter(|&index| index < TEMPLATES.len()).collect();
    let times = numbers("TEMPLATE_TIMES", "0.4,0.9,1.5,2.5,4,-0.3");
    let format = match std::env::var("TEMPLATE_FORMAT").as_deref() {
        Ok("portrait") => Format::Portrait,
        Ok("wide") => Format::Wide,
        _ => Format::Vertical,
    };
    let (cell_w, cell_h) = match format {
        Format::Wide => (384u32, 216u32),
        Format::Portrait => (216, 270),
        Format::Vertical => (216, 384),
    };
    let columns = times.len() as u32;
    let mut sheet = image::RgbaImage::from_pixel(cell_w * columns, cell_h * picks.len() as u32, image::Rgba([40, 40, 40, 255]));
    let mut renderer = Renderer::new().unwrap();
    for (row, &index) in picks.iter().enumerate() {
        let mut project = Project::new(Vec::new(), Vec::new());
        let comp = build(&mut project, &TEMPLATES[index], format).unwrap();
        // With TEMPLATE_FILL set to a picture, every placeholder gets it, as a drop would.
        if let Ok(fill) = std::env::var("TEMPLATE_FILL") {
            let size = image::image_dimensions(&fill).map(|(w, h)| Vec2::new(w as f32, h as f32)).unwrap();
            let picture = studio_core::edit::import_image(&mut project, fill.into(), size);
            let placeholders: Vec<_> = project.composition(comp).unwrap().layers.iter().filter(|layer| studio_core::slots::is_placeholder(layer)).map(|layer| layer.id).collect();
            for layer in placeholders {
                let slot = studio_core::slots::Slot { comp, layer, kind: studio_core::slots::SlotKind::Placeholder, root: layer, outline: [Vec2::default(); 4] };
                studio_core::slots::fill(&mut project, &slot, picture).unwrap();
            }
        }
        let duration = project.composition(comp).unwrap().duration_frames;
        for (column, &seconds) in times.iter().enumerate() {
            let frame = if seconds < 0.0 { duration + (seconds * 30.0) as i64 } else { (seconds * 30.0) as i64 }.clamp(0, duration - 1);
            // Twice: the first render measures the text, as the app does.
            let first = renderer.render(&project, comp, frame, cell_w * 2, cell_h * 2, &studio_core::media::NoVideo).unwrap();
            for (layer, size) in first.text_sizes {
                studio_core::edit::set_text_box(project.composition_mut(comp).unwrap(), layer, size);
            }
            let picture = renderer.render(&project, comp, frame, cell_w, cell_h, &studio_core::media::NoVideo).unwrap();
            let (x0, y0) = (column as u32 * cell_w, row as u32 * cell_h);
            for (i, pixel) in picture.bgra.chunks(4).enumerate() {
                let a = f32::from(pixel[3]) / 255.0;
                let over = |c: u8| (f32::from(c) * a + 60.0 * (1.0 - a)) as u8;
                sheet.put_pixel(x0 + i as u32 % cell_w, y0 + i as u32 / cell_w, image::Rgba([over(pixel[2]), over(pixel[1]), over(pixel[0]), 255]));
            }
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

/// Every device mockup, upright and on its side, with a bright screen, each fitted into a cell.
/// Run with `MOCKUP_SHEET=/path/sheet.png cargo test -p studio-render --test render mockup_sheet -- --ignored`
/// (`MOCKUP_FINISH=` which of each device's finishes, `MOCKUP_ZOOM=` 1 to 4).
#[test]
#[ignore = "writes an image for a visual check"]
fn mockup_sheet() {
    use studio_core::mockups::{Device, MockupRequest, Orientation, build};
    let path = std::env::var("MOCKUP_SHEET").expect("set MOCKUP_SHEET to the PNG to write");
    let pick: usize = std::env::var("MOCKUP_FINISH").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let zoom: u32 = std::env::var("MOCKUP_ZOOM").ok().and_then(|s| s.parse().ok()).unwrap_or(1).clamp(1, 4);
    let (cell_w, cell_h) = (360u32 * zoom, 360u32 * zoom);
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
        // The device's own finishes, the one asked for (or its last).
        let finishes = device.finishes();
        let finish = finishes[pick.min(finishes.len() - 1)].1;
        let made = build(&mut project, &MockupRequest { device, orientation, finish, footage: Some(screen_color), seconds: 10.0 }).unwrap();
        let mockup = project.composition(made.composition).unwrap();
        let fit = cell_w as f32 / mockup.width.max(mockup.height) as f32;
        let (w, h) = ((mockup.width as f32 * fit) as u32, (mockup.height as f32 * fit) as u32);
        let picture = renderer.render(&project, made.composition, 0, w, h, &studio_core::media::NoVideo).unwrap();
        let (w, h) = (picture.width, picture.height);
        for (i, pixel) in picture.bgra.chunks(4).enumerate() {
            let (x, y) = (i as u32 % w + (cell_w - w) / 2, i as u32 / w + (cell_h - h) / 2);
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

// Text animators ---------------------------------------------------------------------------------

fn render_at(project: &Project, frame: i64) -> studio_render::RenderedFrame {
    let mut renderer = Renderer::new().expect("a GPU is available");
    renderer.render(project, Id(1), frame, SIZE, SIZE, &studio_core::media::NoVideo).expect("the frame renders")
}

/// How much light the frame has in columns `columns`, and the average height it sits at.
fn light(frame: &studio_render::RenderedFrame, columns: std::ops::Range<u32>) -> (f32, f32) {
    let (mut total, mut weighted) = (0.0, 0.0);
    for y in 0..SIZE {
        for x in columns.clone() {
            let value = f32::from(pixel(frame, x, y)[0]);
            total += value;
            weighted += value * y as f32;
        }
    }
    (total, if total > 0.0 { weighted / total } else { 0.0 })
}

#[test]
fn text_comes_in_a_character_at_a_time_and_ends_as_drawn() {
    use studio_core::text_animation::{TextAnimator, TextEase, TextMotion, TextUnit};
    let still = text_layer(2, "AB", Vec2::new(80.0, 48.0), Vec2::new(50.0, 50.0));
    let mut animated = still.clone();
    // Ten frames at 24 fps; the two letters fade in one after the other over the first six.
    animated.animation.text_in = Some(TextAnimator { motion: TextMotion::Fade, unit: TextUnit::Character, ease: TextEase::Linear, seconds: 0.25, overlap: 0.0, ..TextAnimator::default() });
    let (still, animated) = (project(vec![still]), project(vec![animated]));
    let whole = render_at(&still, 8);
    assert_eq!(light(&render_at(&animated, 0), 0..SIZE).0, 0.0, "nothing shows at the in point");
    // A quarter of the way in, the first letter is half in and the second hasn't started.
    let early = render_at(&animated, 1);
    let (left, right) = (light(&early, 0..50).0, light(&early, 50..SIZE).0);
    assert!(left > 0.0 && right == 0.0, "left {left}, right {right}");
    assert!(left < light(&whole, 0..50).0 * 0.8, "and only partly");
    // Once it is over, the text is exactly as drawn without it.
    assert_eq!(render_at(&animated, 8).bgra, whole.bgra);
}

#[test]
fn rising_text_starts_below_its_place() {
    use studio_core::text_animation::{TextAnimator, TextEase, TextMotion, TextUnit};
    let still = text_layer(2, "AB", Vec2::new(80.0, 48.0), Vec2::new(50.0, 50.0));
    let mut animated = still.clone();
    animated.animation.text_in = Some(TextAnimator { motion: TextMotion::Rise, unit: TextUnit::Word, ease: TextEase::Linear, seconds: 0.4, overlap: 1.0, ..TextAnimator::default() });
    let home = light(&render_at(&project(vec![still]), 9), 0..SIZE).1;
    let rising = light(&render_at(&project(vec![animated]), 4), 0..SIZE).1;
    assert!(rising > home + 3.0, "lower on the way in: {rising} vs {home}");
}

/// A red square morphing into a green circle to its right (Morph To): it starts as the square,
/// ends as the circle where the circle was (the circle itself hidden), and is a blend of the
/// two halfway.
#[test]
fn a_shape_morphs_into_another() {
    let square = shape(2, ShapeKind::Rectangle, Paint::Solid(Rgba::hex(0xff0000)), Vec2::new(30.0, 30.0), Vec2::new(25.0, 50.0));
    let circle = shape(3, ShapeKind::Ellipse, Paint::Solid(Rgba::hex(0x00ff00)), Vec2::new(30.0, 30.0), Vec2::new(75.0, 50.0));
    let mut scene = project(vec![square, circle]);
    let end = studio_core::morph::morph_to(&mut scene.compositions[0], Id(2), Id(3), 0, 8, studio_core::animation::Easing::Linear).unwrap();
    assert_eq!(end, 8);
    let mut renderer = Renderer::new().expect("a GPU is available");
    let mut at = |frame| renderer.render(&scene, Id(1), frame, SIZE, SIZE, &studio_core::media::NoVideo).expect("the frame renders");
    let start = at(0);
    assert!(close(pixel(&start, 25, 50), [255, 0, 0]), "the square to start with");
    assert!(close(pixel(&start, 12, 37), [255, 0, 0]), "with its corners");
    assert!(close(pixel(&start, 75, 50), [0, 0, 0]), "the circle is hidden");
    let middle = at(4);
    let [r, g, _] = pixel(&middle, 50, 50);
    assert!(r > 60 && g > 60, "a blend of red and green halfway, where the two meet: {r} {g}");
    let finish = at(8);
    assert!(close(pixel(&finish, 75, 50), [0, 255, 0]), "the circle at the end");
    assert!(close(pixel(&finish, 62, 37), [0, 0, 0]), "with no square corners left");
    assert!(close(pixel(&finish, 25, 50), [0, 0, 0]), "and nothing where the square was");
}

/// Saves frames across three morphs (square into circle, star into a rounded rectangle, and a
/// triangle into an open wave line) side by side to `MORPH_SHEET` as a PNG for a visual check.
/// Run with `MORPH_SHEET=/path/morphs.png cargo test -p studio-render --test render morph_sheet -- --ignored`.
#[test]
#[ignore = "writes an image for a visual check"]
fn morph_sheet() {
    use studio_core::path::{PathPoint, ShapePath};
    let path = std::env::var("MORPH_SHEET").expect("set MORPH_SHEET to the PNG to write");
    let (cell, steps) = (200u32, 7i64);
    let row_center = |row: u32| 100.0 + row as f32 * cell as f32;
    let mut layers = Vec::new();
    // Square → circle, also turning from violet to orange.
    layers.push(shape(10, ShapeKind::Rectangle, Paint::Solid(Rgba::hex(0x8b5cf6)), Vec2::new(120.0, 120.0), Vec2::new(100.0, row_center(0))));
    layers.push(shape(11, ShapeKind::Ellipse, Paint::Solid(Rgba::hex(0xf97316)), Vec2::new(150.0, 110.0), Vec2::new(100.0, row_center(0))));
    // Star → rounded rectangle, rotated, with a stroke fading in.
    layers.push(shape(20, ShapeKind::Star, Paint::linear(Rgba::hex(0xf472b6), Rgba::hex(0x60a5fa), 0.0), Vec2::new(150.0, 150.0), Vec2::new(100.0, row_center(1))));
    let mut rounded = shape(21, ShapeKind::Rectangle, Paint::Solid(Rgba::hex(0x22c55e)), Vec2::new(140.0, 90.0), Vec2::new(100.0, row_center(1)));
    if let LayerContent::Shape { style, .. } = &mut rounded.content {
        style.corner_radius = 30.0;
        style.stroke = Some(Stroke { paint: Paint::Solid(Rgba::hex(0xffffff)), width: 6.0 });
    }
    rounded.transform.as_mut().unwrap().rotation = Animated::constant(20.0);
    layers.push(rounded);
    // Triangle → an open wave (closed into open: filled for the first half, a line after).
    layers.push(shape(30, ShapeKind::Triangle, Paint::Solid(Rgba::hex(0xeab308)), Vec2::new(130.0, 120.0), Vec2::new(100.0, row_center(2))));
    let wave = ShapePath {
        points: vec![
            PathPoint::smooth(Vec2::new(0.0, 0.5), Vec2::new(0.15, -0.6)),
            PathPoint::smooth(Vec2::new(0.5, 0.5), Vec2::new(0.15, 0.6)),
            PathPoint::smooth(Vec2::new(1.0, 0.5), Vec2::new(0.15, -0.6)),
        ],
        closed: false,
    };
    let mut line = solid(31, 0, Vec2::new(160.0, 80.0), Vec2::new(100.0, row_center(2)));
    line.content = LayerContent::Shape {
        style: ShapeStyle { fill: None, stroke: Some(Stroke { paint: Paint::Solid(Rgba::hex(0x38bdf8)), width: 8.0 }), ..ShapeStyle::new(ShapeKind::Path) },
        path: Some(wave),
    };
    layers.push(line);

    let mut scene = project(layers);
    let composition = &mut scene.compositions[0];
    (composition.width, composition.height, composition.duration_frames) = (cell, cell * 3, steps);
    for (from, to) in [(10, 11), (20, 21), (30, 31)] {
        studio_core::morph::morph_to(composition, Id(from), Id(to), 0, steps - 1, studio_core::morph::default_easing()).unwrap();
    }
    let (width, height) = (cell * steps as u32, cell * 3);
    let mut sheet = vec![0u8; (width * height * 4) as usize];
    let mut renderer = Renderer::new().unwrap();
    for frame in 0..steps {
        let picture = renderer.render(&scene, Id(1), frame, cell, cell * 3, &studio_core::media::NoVideo).unwrap();
        for (index, pixel) in picture.bgra.chunks(4).enumerate() {
            let (x, y) = (index as u32 % cell + frame as u32 * cell, index as u32 / cell);
            let at = ((y * width + x) * 4) as usize;
            sheet[at..at + 4].copy_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
    }
    image::save_buffer(path, &sheet, width, height, image::ColorType::Rgba8).unwrap();
}

/// Saves frames across a Magic Move between two small compositions to `MAGIC_MOVE_SHEET` as a
/// PNG: a square named "Card" morphs into a star, a triangle named "Badge" (the same in both)
/// moves and turns, a circle only in the first fades out and a bar only in the second fades in.
/// Run with `MAGIC_MOVE_SHEET=/path/magic.png cargo test -p studio-render --test render magic_move_sheet -- --ignored`.
#[test]
#[ignore = "writes an image for a visual check"]
fn magic_move_sheet() {
    use studio_core::edit::{CompositionSettings, add_shape_layer, new_composition};
    let path = std::env::var("MAGIC_MOVE_SHEET").expect("set MAGIC_MOVE_SHEET to the PNG to write");
    let (width, height) = (320u32, 180u32);
    let mut scene = Project::new(Vec::new(), Vec::new());
    let settings = |name: &str| CompositionSettings { name: name.into(), width, height, frame_rate: FrameRate::new(30, 1), duration_frames: 30, background: Rgba::hex(0x101018) };
    let intro = new_composition(&mut scene, &settings("Intro")).unwrap();
    let main = new_composition(&mut scene, &settings("Main")).unwrap();
    let style = |kind, color| ShapeStyle { fill: Some(Paint::Solid(Rgba::hex(color))), ..ShapeStyle::new(kind) };
    let shapes = [
        (intro, "Card", style(ShapeKind::Rectangle, 0x8b5cf6), Vec2::new(70.0, 70.0), Vec2::new(70.0, 90.0), 0.0),
        (intro, "Spot", style(ShapeKind::Ellipse, 0xf97316), Vec2::new(40.0, 40.0), Vec2::new(270.0, 40.0), 0.0),
        (intro, "Badge", style(ShapeKind::Triangle, 0xeab308), Vec2::new(40.0, 36.0), Vec2::new(160.0, 140.0), 0.0),
        (main, "Card", style(ShapeKind::Star, 0xef4444), Vec2::new(110.0, 110.0), Vec2::new(240.0, 80.0), 0.0),
        (main, "Badge", style(ShapeKind::Triangle, 0xeab308), Vec2::new(40.0, 36.0), Vec2::new(60.0, 40.0), 90.0),
        (main, "Bar", style(ShapeKind::Rectangle, 0x22c55e), Vec2::new(280.0, 20.0), Vec2::new(160.0, 160.0), 0.0),
    ];
    for (comp, name, style, size, center, rotation) in shapes {
        let id = add_shape_layer(&mut scene, comp, style, size, center, 0).unwrap();
        let layer = scene.composition_mut(comp).unwrap().layer_mut(id).unwrap();
        layer.name = name.into();
        layer.transform.as_mut().unwrap().rotation = Animated::constant(rotation);
    }
    let made = studio_core::magic_move::magic_move(&mut scene, intro, main, 20, studio_core::morph::default_easing()).unwrap();
    let frames = [29i64, 32, 35, 38, 41, 44, 47, 50];
    let columns = 4u32;
    let rows = frames.len() as u32 / columns;
    let mut sheet = vec![0u8; (width * columns * height * rows * 4) as usize];
    let mut renderer = Renderer::new().unwrap();
    for (cell, &frame) in frames.iter().enumerate() {
        let picture = renderer.render(&scene, made.composition, frame, width, height, &studio_core::media::NoVideo).unwrap();
        let (x0, y0) = (cell as u32 % columns * width, cell as u32 / columns * height);
        for (index, pixel) in picture.bgra.chunks(4).enumerate() {
            let (x, y) = (x0 + index as u32 % width, y0 + index as u32 / width);
            let at = ((y * width * columns + x) * 4) as usize;
            sheet[at..at + 4].copy_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
    }
    image::save_buffer(path, &sheet, width * columns, height * rows, image::ColorType::Rgba8).unwrap();
}

/// Where a big composition's render time goes: by layer kind and by output size. Run with
/// `cargo test -p studio-render --release --test render render_cost -- --ignored --nocapture`.
#[test]
#[ignore = "benchmark; run on demand"]
fn render_cost() {
    use studio_core::model::{ShapeKind, ShapeStyle};
    let build = |kind: &str, count: u32| {
        let mut project = Project::new(Vec::new(), Vec::new());
        let settings = studio_core::edit::CompositionSettings { name: "Bench".into(), width: 1920, height: 1080, frame_rate: studio_core::time::FrameRate::new(30, 1), duration_frames: 300, background: Rgba::hex(0x000000) };
        let comp = studio_core::edit::new_composition(&mut project, &settings).unwrap();
        for index in 0..count {
            let at = Vec2::new(100.0 + (index % 20) as f32 * 85.0, 80.0 + (index / 20) as f32 * 38.0);
            match kind {
                "text" => studio_core::edit::add_text_layer(&mut project, comp, &format!("Line {index}"), at, 0),
                "shape" => studio_core::edit::add_shape_layer(&mut project, comp, ShapeStyle::new(ShapeKind::Rectangle), Vec2::new(60.0, 30.0), at, 0),
                _ => studio_core::edit::add_background(&mut project, comp, Rgba::hex(0x223344)),
            };
        }
        (project, comp)
    };
    let mut renderer = Renderer::new().unwrap();
    for kind in ["text", "shape", "solid"] {
        for count in [10, 100, 200] {
            let (project, comp) = build(kind, count);
            for (width, height) in [(960, 540), (1920, 1080)] {
                renderer.render(&project, comp, 10, width, height, &studio_core::media::NoVideo).unwrap();
                let started = std::time::Instant::now();
                for frame in 0..10 {
                    renderer.render(&project, comp, 11 + frame, width, height, &studio_core::media::NoVideo).unwrap();
                }
                println!("{kind:>6} × {count:>3} at {width}×{height}: {:>7.2} ms", started.elapsed().as_secs_f64() * 100.0);
            }
        }
    }
}

#[test]
fn simple_layers_drawn_together_look_the_same_as_one_at_a_time() {
    use studio_core::animation::Animated;
    use studio_core::model::{ShapeKind, ShapeStyle};
    // Overlapping solids and shapes at part opacity, a text layer, and a blend-mode layer in the
    // middle (which isn't simple, so it splits the run in two).
    let mut layers = Vec::new();
    for index in 0..8u64 {
        let mut layer = solid(10 + index, [0xff3366, 0x33ccff, 0xffcc00, 0x66ff66][index as usize % 4], Vec2::new(40.0, 30.0), Vec2::new(25.0 + index as f32 * 7.0, 30.0 + index as f32 * 5.0));
        if index % 2 == 1 {
            layer.content = LayerContent::Shape { style: ShapeStyle::new(if index % 4 == 1 { ShapeKind::Ellipse } else { ShapeKind::Star }), path: None };
        }
        if let Some(transform) = layer.transform.as_mut() {
            transform.opacity = Animated::constant(35.0 + index as f32 * 8.0);
            transform.rotation = Animated::constant(index as f32 * 11.0);
        }
        if index == 4 {
            layer.blend_mode = studio_core::model::BlendMode::Screen;
        }
        layers.push(layer);
    }
    let mut text = solid(30, 0xffffff, Vec2::new(60.0, 20.0), Vec2::new(50.0, 70.0));
    text.content = LayerContent::Text { text: "Hi".into(), style: TextStyle { font_size: 18.0, line_height: 20.0, ..TextStyle::default() } };
    layers.push(text);
    let project = project(layers);
    let mut renderer = Renderer::new().expect("a GPU is available");
    let together = renderer.render(&project, Id(1), 0, SIZE, SIZE, &studio_core::media::NoVideo).unwrap();
    renderer.set_batching(false);
    let apart = renderer.render(&project, Id(1), 0, SIZE, SIZE, &studio_core::media::NoVideo).unwrap();
    let largest = together.bgra.iter().zip(&apart.bgra).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    assert!(largest <= 1, "they differ by up to {largest} levels");
    assert!(together.bgra.chunks(4).any(|pixel| pixel[..3] != [0, 0, 0]), "something was drawn");
}

/// A full-frame solid of `color` with a Keyer whose parameters are set to `values` (by index;
/// the rest at their defaults), on a transparent composition. Returns the middle pixel's color
/// and alpha.
fn keyed(color: u32, values: &[(usize, f32)]) -> ([u8; 3], u8) {
    let mut layer = full(2, color);
    let mut keyer = effects::instantiate(effects::find("Keyer").unwrap(), Id(9));
    for &(index, value) in values {
        keyer.properties[index].value = Animated::constant(value);
    }
    layer.effects.push(keyer);
    let frame = render(&Project::new(Vec::new(), vec![clear_comp(1, vec![layer])]));
    (pixel(&frame, 50, 50), frame.bgra[((50 * frame.width + 50) * 4 + 3) as usize])
}

#[test]
fn the_keyer_removes_the_screen_color_and_keeps_skin_and_gray() {
    assert_eq!(keyed(0x00ff00, &[]).1, 0, "pure green is keyed out");
    // The same screen in shadow and in a hot spot still keys, as the match ignores brightness.
    assert_eq!(keyed(0x008000, &[]).1, 0, "darker green");
    assert_eq!(keyed(0x60ff60, &[(3, 50.0)]).1, 0, "a lighter, washed-out green at a wider tolerance");
    // Skin and gray stay solid and keep their color (no green to take out of them).
    for color in [0xd9a07a, 0x8d5524, 0x808080, 0xffffff] {
        let (rgb, alpha) = keyed(color, &[]);
        assert_eq!(alpha, 255, "{color:06x} stays opaque");
        let expected = [(color >> 16) as u8, (color >> 8) as u8, color as u8];
        assert!(close(rgb, expected), "{color:06x} keeps its color: {rgb:?}");
    }
    // A blue screen keys with the key color set to blue, and leaves green alone then.
    let blue = [(0, 0.0), (1, 0.0), (2, 255.0)];
    assert_eq!(keyed(0x0000ff, &blue).1, 0, "pure blue against a blue key");
    assert_eq!(keyed(0x00ff00, &blue).1, 255, "green against a blue key");
}

#[test]
fn the_keyers_softness_leaves_edge_colors_partly_transparent() {
    // Half screen green, half gray: the color of a pixel on a subject's edge.
    let edge = 0x40bf40;
    assert_eq!(keyed(edge, &[(4, 0.0)]).1, 255, "with no softness the edge is all kept");
    let (_, soft) = keyed(edge, &[(4, 30.0)]);
    assert!((20..235).contains(&soft), "softness makes the edge partly see-through: {soft}");
    let (_, softer) = keyed(edge, &[(4, 60.0)]);
    assert!(softer < soft, "more softness, more see-through: {softer} vs {soft}");
}

#[test]
fn the_keyers_spill_suppression_takes_the_green_cast_off_edges() {
    // A light gray picking up green light from the screen: kept, but tinted.
    let tinted = 0x90c090;
    let (cast, alpha) = keyed(tinted, &[(5, 0.0)]);
    assert_eq!(alpha, 255);
    assert!(cast[1] > cast[0] + 40, "without suppression the green stays: {cast:?}");
    let (cleaned, alpha) = keyed(tinted, &[(5, 100.0)]);
    assert_eq!(alpha, 255, "suppression changes color, not what is kept");
    assert!(cleaned[1] <= cleaned[0] + 3, "full suppression brings green down to red and blue: {cleaned:?}");
    let (half, _) = keyed(tinted, &[(5, 50.0)]);
    assert!(half[1] < cast[1] && half[1] > cleaned[1], "half suppression is in between: {half:?}");
}

#[test]
fn the_keyers_choke_pulls_the_edge_in() {
    // A gray square on a green screen, keyed by an adjustment layer over both.
    let render_with_choke = |choke: f32| {
        let mut keyer_layer = full(2, 0);
        keyer_layer.content = LayerContent::Adjustment;
        let mut keyer = effects::instantiate(effects::find("Keyer").unwrap(), Id(9));
        keyer.properties[6].value = Animated::constant(choke);
        keyer_layer.effects.push(keyer);
        let square = solid(3, 0x808080, Vec2::new(40.0, 40.0), Vec2::new(50.0, 50.0));
        render(&Project::new(Vec::new(), vec![clear_comp(1, vec![keyer_layer, square, full(4, 0x00ff00)])]))
    };
    let alpha = |frame: &studio_render::RenderedFrame, x: u32, y: u32| frame.bgra[((y * frame.width + x) * 4 + 3) as usize];
    let plain = render_with_choke(0.0);
    assert_eq!((alpha(&plain, 10, 10), alpha(&plain, 32, 50), alpha(&plain, 50, 50)), (0, 255, 255), "the screen goes, the square stays");
    let choked = render_with_choke(5.0);
    assert_eq!(alpha(&choked, 32, 50), 0, "2 px inside the edge is pulled in by a 5 px choke");
    assert_eq!(alpha(&choked, 50, 50), 255, "the middle stays solid");
}

/// A 10 × 10 white square crossing the frame left to right at 40 px a frame (x = 10 at frame 0,
/// 50 at frame 1, 90 at frame 2), with its Motion Blur switch set to `blur`.
fn moving_square(blur: bool) -> Layer {
    use studio_core::animation::Keyframe;
    let mut square = solid(2, 0xffffff, Vec2::new(10.0, 10.0), Vec2::new(10.0, 50.0));
    square.transform.as_mut().unwrap().position = Animated::keyframed(vec![Keyframe::linear(0, Vec2::new(10.0, 50.0)), Keyframe::linear(2, Vec2::new(90.0, 50.0))]);
    square.switches.motion_blur = blur;
    square
}

#[test]
fn motion_blur_smears_a_moving_layer_along_its_path() {
    let sharp = render_at(&project(vec![moving_square(false)]), 1);
    let blurred = render_at(&project(vec![moving_square(true)]), 1);
    let red = |frame: &studio_render::RenderedFrame, x: u32| pixel(frame, x, 50)[0];
    // Without blur: solid from x = 45 to 55, nothing else on the row.
    assert_eq!((red(&sharp, 50), red(&sharp, 40), red(&sharp, 60)), (255, 0, 0));
    // A 180° shutter is open for half a frame, so the square sweeps 20 px (its center from 40
    // to 60): from 45 to 55 it is covered half the time, toward the swept ends less, and past
    // them not at all.
    assert!((150..230).contains(&red(&blurred, 50)), "half covered in the middle: {}", red(&blurred, 50));
    assert!(red(&blurred, 45).abs_diff(red(&blurred, 50)) <= 8, "evenly covered across the square's own width");
    for x in [38, 41, 59, 62] {
        assert!((20..175).contains(&red(&blurred, x)), "partly covered at {x}: {}", red(&blurred, x));
    }
    assert!(red(&blurred, 37) < red(&blurred, 40) && red(&blurred, 40) < red(&blurred, 43), "fading toward the ends of the path");
    assert_eq!((red(&blurred, 30), red(&blurred, 70)), (0, 0), "nothing beyond the swept path");
    // Above and below the path stays clear: the smear runs along the motion only.
    assert_eq!(pixel(&blurred, 50, 40), [0, 0, 0]);

    // A wider shutter smears further.
    let mut wide = project(vec![moving_square(true)]);
    wide.compositions[0].motion_blur.shutter_angle = 360.0;
    assert!(red(&render_at(&wide, 1), 33) > 0, "a 360° shutter sweeps a whole frame's move");
}

#[test]
fn motion_blur_changes_nothing_on_still_layers_or_with_the_switch_off() {
    // Held still (after its last keyframe), the switch on draws exactly what it does off.
    let still_on = render_at(&project(vec![moving_square(true)]), 5);
    let still_off = render_at(&project(vec![moving_square(false)]), 5);
    assert_eq!(still_on.bgra, still_off.bgra);
    // Moving with the switch off is the plain frame: the same as a still square in that place.
    let moving_off = render_at(&project(vec![moving_square(false)]), 1);
    let placed = render_at(&project(vec![solid(2, 0xffffff, Vec2::new(10.0, 10.0), Vec2::new(50.0, 50.0))]), 1);
    assert_eq!(moving_off.bgra, placed.bgra);
    // A closed shutter blurs nothing either.
    let mut closed = project(vec![moving_square(true)]);
    closed.compositions[0].motion_blur.shutter_angle = 0.0;
    assert_eq!(render_at(&closed, 1).bgra, moving_off.bgra);
}

#[test]
fn a_motion_blurred_layer_composites_like_any_other() {
    // Over another layer, at half opacity: the blur is drawn first, then faded and laid over.
    let mut square = moving_square(true);
    square.transform.as_mut().unwrap().opacity = Animated::constant(50.0);
    let frame = render_at(&project(vec![square, full(3, 0x0000ff)]), 1);
    let middle = pixel(&frame, 50, 50);
    assert!(middle[0] > 60 && middle[0] < 200 && middle[2] > 150, "a faded smear over blue: {middle:?}");
    assert!(close(pixel(&frame, 20, 50), [0, 0, 255]), "the blue shows outside the path");
}
