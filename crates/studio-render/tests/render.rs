//! Renders small compositions on the GPU and checks real pixel colors.

use studio_core::animation::{Animated, Vec2};
use studio_core::effects;
use studio_core::model::{
    BlendMode, Composition, Id, LabelColor, Layer, LayerContent, LayerSwitches, Project, Rgba, TextStyle, TrackMatte,
    Transform,
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
