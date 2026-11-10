//! Exports real compositions (a hand-made one and Studio's built-in templates) as Lottie,
//! dotLottie and SVG, and checks what comes out.

mod schema;

use serde_json::Value;
use studio_core::animation::{Animated, CubicBezier, Easing, Keyframe, Vec2};
use studio_core::edit::{self, CompositionSettings};
use studio_core::model::{Id, LayerMask, MaskMode, Paint, Project, ProjectItem, Rgba, ShapeKind, ShapeStyle, Stroke, TextStyle, TrackMatte};
use studio_core::path::{PathPoint, ShapePath};
use studio_core::time::FrameRate;
use studio_vector::{EmbeddedImage, GlyphOutline, NoResources, Resources, TextOutline, VectorOptions, dotlottie, lottie, svg};

/// Stand-in fonts and pictures: each letter a box half an em wide, every picture a 2 × 2 PNG.
/// Emoji have no outline, as color glyphs from a real font don't.
struct FakeResources;

impl Resources for FakeResources {
    fn image(&mut self, _item: &ProjectItem) -> Option<EmbeddedImage> {
        Some(EmbeddedImage { mime: "image/png", bytes: vec![0x89, b'P', b'N', b'G'], width: 2, height: 2 })
    }

    fn text(&mut self, text: &str, style: &TextStyle) -> Option<TextOutline> {
        let em = style.font_size;
        let advance = em * 0.6;
        let mut glyphs = Vec::new();
        let mut lines = Vec::new();
        let mut widest = 0.0_f32;
        let mut line_start = 0;
        for (line_index, line) in text.split('\n').enumerate() {
            let top = line_index as f32 * style.line_height;
            lines.push((top, style.line_height));
            for (column, (at, c)) in line.char_indices().enumerate() {
                let left = column as f32 * advance;
                let contours = if c.is_whitespace() || u32::from(c) >= 0x1F000 {
                    Vec::new()
                } else {
                    let corners = [(left, top + 0.2 * em), (left + 0.5 * em, top + 0.2 * em), (left + 0.5 * em, top + em), (left, top + em)];
                    vec![ShapePath { points: corners.iter().map(|&(x, y)| PathPoint::corner(Vec2::new(x, y))).collect(), closed: true, ..Default::default() }]
                };
                glyphs.push(GlyphOutline { start: line_start + at, line: line_index, left, width: advance, contours });
            }
            widest = widest.max(line.chars().count() as f32 * advance);
            line_start += line.len() + 1;
        }
        Some(TextOutline { size: Vec2::new(widest.max(1.0), lines.len() as f32 * style.line_height), lines, glyphs })
    }
}

const EASE: CubicBezier = CubicBezier::new(0.16, 1.0, 0.3, 1.0);

/// A 400 × 300, 30 fps, two-second composition holding one rectangle that slides across with
/// an ease-out curve and blinks off with a hold keyframe.
fn keyframed_rectangle() -> (Project, Id, Id) {
    let mut project = Project::new(Vec::new(), Vec::new());
    let settings = CompositionSettings { name: "Slide".into(), width: 400, height: 300, frame_rate: FrameRate::new(30, 1), duration_frames: 60, background: Rgba::TRANSPARENT };
    let comp = edit::new_composition(&mut project, &settings).unwrap();
    let style = ShapeStyle { fill: Some(Paint::Solid(Rgba::hex(0xff3366))), corner_radius: 8.0, ..ShapeStyle::new(ShapeKind::Rectangle) };
    let layer = edit::add_shape_layer(&mut project, comp, style, Vec2::new(100.0, 50.0), Vec2::new(200.0, 150.0), 0).unwrap();
    let composition = project.composition_mut(comp).unwrap();
    let shape = composition.layer_mut(layer).unwrap();
    shape.out_frame = 60;
    let transform = shape.transform.as_mut().unwrap();
    transform.position = Animated::keyframed(vec![Keyframe { frame: 0, value: Vec2::new(50.0, 150.0), easing: Easing::Curve(EASE) }, Keyframe::linear(30, Vec2::new(350.0, 150.0))]);
    transform.opacity = Animated::keyframed(vec![Keyframe { frame: 40, value: 100.0, easing: Easing::Hold }, Keyframe::linear(50, 0.0)]);
    (project, comp, layer)
}

fn options(comp: Id, frames: std::ops::Range<i64>) -> VectorOptions {
    VectorOptions { composition: comp, frames, frame_rate: None }
}

fn parse(json: &str) -> Value {
    serde_json::from_str(json).expect("valid JSON")
}

#[test]
fn a_keyframed_shape_becomes_a_shape_layer_with_its_easing() {
    let (project, comp, _) = keyframed_rectangle();
    let exported = lottie(&project, &options(comp, 0..60), &mut NoResources).unwrap();
    assert!(exported.left_out.is_empty(), "nothing lost: {:?}", exported.left_out);
    let json = parse(&exported.file);
    assert_eq!((json["w"].as_i64(), json["h"].as_i64(), json["fr"].as_f64(), json["ip"].as_f64(), json["op"].as_f64()), (Some(400), Some(300), Some(30.0), Some(0.0), Some(60.0)));
    let layers = json["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 1, "a transparent background adds no layer");
    let layer = &layers[0];
    assert_eq!(layer["ty"], 4, "a shape layer");
    assert_eq!((layer["ip"].as_f64(), layer["op"].as_f64()), (Some(0.0), Some(60.0)));

    // Position: two keyframes; the first carries the curve as its out and in tangents.
    let position = &layer["ks"]["p"];
    assert_eq!(position["a"], 1);
    let keys = position["k"].as_array().unwrap();
    assert_eq!(keys.len(), 2);
    assert_eq!((keys[0]["t"].as_f64(), keys[1]["t"].as_f64()), (Some(0.0), Some(30.0)));
    assert_eq!(keys[0]["s"], serde_json::json!([50, 150]));
    assert_eq!(keys[0]["o"]["x"], serde_json::json!([0.16, 0.16]));
    assert_eq!(keys[0]["o"]["y"], serde_json::json!([1, 1]));
    assert_eq!(keys[0]["i"]["x"], serde_json::json!([0.3, 0.3]));
    assert_eq!(keys[0]["i"]["y"], serde_json::json!([1, 1]));
    assert!(keys[1].get("o").is_none(), "the last keyframe has no segment to ease");

    // Opacity: a hold keyframe, then off.
    let opacity = layer["ks"]["o"]["k"].as_array().unwrap();
    assert_eq!(opacity[0]["h"], 1);
    assert_eq!(opacity[0]["s"], serde_json::json!([100]));
    assert_eq!(opacity[1]["s"], serde_json::json!([0]));

    // The anchor stays in the box's middle, so the rectangle turns about its center.
    assert_eq!(layer["ks"]["a"], serde_json::json!({ "a": 0, "k": [50, 25] }));

    // Shapes: a group holding the rounded rectangle, its fill and the group transform.
    let group = &layer["shapes"][0];
    assert_eq!(group["ty"], "gr");
    let items: Vec<&str> = group["it"].as_array().unwrap().iter().map(|item| item["ty"].as_str().unwrap()).collect();
    assert_eq!(items, ["rc", "fl", "tr"]);
    let rect = &group["it"][0];
    assert_eq!((rect["p"]["k"].clone(), rect["s"]["k"].clone(), rect["r"]["k"].clone()), (serde_json::json!([50, 25]), serde_json::json!([100, 50]), serde_json::json!(8)));
    let fill = &group["it"][1];
    assert_eq!(fill["c"]["k"], serde_json::json!([1, 0.2, 0.4, 1]));
    schema::check(&exported.file);
}

#[test]
fn studios_own_lottie_importer_reads_the_export_back_as_the_same_animation() {
    let (mut project, comp, layer) = keyframed_rectangle();
    // An overshooting curve too, whose handles leave 0–1.
    let back_out = Easing::Curve(CubicBezier::new(0.34, 1.56, 0.64, 1.0));
    project.composition_mut(comp).unwrap().layer_mut(layer).unwrap().transform.as_mut().unwrap().rotation = Animated::keyframed(vec![Keyframe { frame: 5, value: 0.0, easing: back_out }, Keyframe::linear(45, 180.0)]);
    let exported = lottie(&project, &options(comp, 0..60), &mut NoResources).unwrap().file;

    let folder = std::env::temp_dir().join(format!("studio-vector-round-trip-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let scene = studio_core::import::lottie::read(&exported, &folder.join("Slide.json"), &folder).expect("Studio reads its own Lottie");
    let mut back = Project::new(Vec::new(), Vec::new());
    let imported = studio_core::import::build(&mut back, &scene, "Slide", &|_| None).unwrap();
    std::fs::remove_dir_all(&folder).ok();

    let original_comp = project.composition(comp).unwrap();
    let copy_comp = back.composition(imported.compositions[0]).unwrap();
    assert_eq!((copy_comp.width, copy_comp.height, copy_comp.frame_rate, copy_comp.duration_frames), (400, 300, FrameRate::new(30, 1), 60));
    let original = original_comp.layer(layer).unwrap();
    let copy = copy_comp.layers.iter().find(|candidate| candidate.name == original.name).expect("the layer came back");
    assert_eq!((copy.in_frame, copy.out_frame), (original.in_frame, original.out_frame));
    // The layer sits and moves the same on every frame: its box's corners line up.
    for frame in 0..60 {
        let at = frame as f64;
        let (a, b) = (original.transform.as_ref().unwrap(), copy.transform.as_ref().unwrap());
        let position = (a.position.value_at(at), b.position.value_at(at));
        assert!((position.0 - position.1).length() < 0.05, "position at {frame}: {position:?}");
        let rotation = (a.rotation.value_at(at), b.rotation.value_at(at));
        assert!((rotation.0 - rotation.1).abs() < 0.05, "rotation at {frame}: {rotation:?}");
        let opacity = (a.opacity.value_at(at), b.opacity.value_at(at));
        assert!((opacity.0 - opacity.1).abs() < 0.05, "opacity at {frame}: {opacity:?}");
    }
}

#[test]
fn the_work_area_starts_the_animation_and_frame_rates_convert() {
    let (project, comp, _) = keyframed_rectangle();
    let at_sixty = VectorOptions { frame_rate: Some(FrameRate::new(60, 1)), ..options(comp, 10..40) };
    let json = parse(&lottie(&project, &at_sixty, &mut NoResources).unwrap().file);
    assert_eq!((json["fr"].as_f64(), json["op"].as_f64()), (Some(60.0), Some(60.0)), "30 frames at 30 fps are 60 at 60 fps");
    let layer = &json["layers"][0];
    // Frame 10 of the composition is frame 0 of the animation; times double at 60 fps.
    assert_eq!(layer["ip"].as_f64(), Some(-20.0));
    let keys = layer["ks"]["p"]["k"].as_array().unwrap();
    assert_eq!((keys[0]["t"].as_f64(), keys[1]["t"].as_f64()), (Some(-20.0), Some(40.0)));
}

#[test]
fn elastic_easing_is_sampled_and_noted() {
    let (mut project, comp, layer) = keyframed_rectangle();
    let transform = project.composition_mut(comp).unwrap().layer_mut(layer).unwrap().transform.as_mut().unwrap();
    transform.scale = Animated::keyframed(vec![Keyframe { frame: 0, value: Vec2::new(0.0, 0.0), easing: Easing::Elastic(studio_core::animation::EaseMode::Out) }, Keyframe::linear(20, Vec2::new(100.0, 100.0))]);
    let exported = lottie(&project, &options(comp, 0..60), &mut NoResources).unwrap();
    let json = parse(&exported.file);
    let keys = json["layers"][0]["ks"]["s"]["k"].as_array().unwrap();
    assert_eq!(keys.len(), 21, "a keyframe on every frame of the elastic segment");
    assert!(keys.iter().any(|key| key["s"][0].as_f64().unwrap() > 100.0), "it still overshoots");
    assert!(exported.left_out.iter().any(|line| line.starts_with("Elastic and bounce easing")), "{:?}", exported.left_out);
    schema::check(&exported.file);
}

#[test]
fn mattes_masks_parents_and_gradients_map_to_lottie() {
    let (mut project, comp, rectangle) = keyframed_rectangle();
    // A circle above the rectangle, used as its track matte.
    let circle = edit::add_shape_layer(&mut project, comp, ShapeStyle::new(ShapeKind::Ellipse), Vec2::new(80.0, 80.0), Vec2::new(200.0, 150.0), 0).unwrap();
    // A star with a gradient and a stroke, masked by a pen shape, parented to a null.
    let star_style = ShapeStyle {
        fill: Some(Paint::radial(Rgba::hex(0xffffff), Rgba::hex(0x0000ff))),
        stroke: Some(Stroke { paint: Paint::Solid(Rgba::hex(0x000000)), width: 4.0 }),
        corner_radius: 6.0,
        ..ShapeStyle::new(ShapeKind::Star)
    };
    let star = edit::add_shape_layer(&mut project, comp, star_style, Vec2::new(120.0, 60.0), Vec2::new(100.0, 100.0), 0).unwrap();
    let pen = ShapePath { points: vec![PathPoint::corner(Vec2::new(0.0, 0.0)), PathPoint::smooth(Vec2::new(100.0, 50.0), Vec2::new(20.0, 0.0)), PathPoint::corner(Vec2::new(0.0, 100.0))], closed: true, ..Default::default() };
    let mask = edit::add_path_layer(&mut project, comp, &pen, ShapeStyle::new(ShapeKind::Path), 0).unwrap();
    let null = edit::add_null_layer(&mut project, comp, 0).unwrap();
    let composition = project.composition_mut(comp).unwrap();
    // Stacking (top first): null, mask, star, circle, rectangle.
    let order = [null, mask, star, circle, rectangle];
    composition.layers.sort_by_key(|layer| order.iter().position(|id| *id == layer.id));
    composition.layer_mut(rectangle).unwrap().track_matte = TrackMatte::AlphaInverted;
    composition.layer_mut(star).unwrap().mask = Some(LayerMask { layer: mask, mode: MaskMode::Show });
    composition.layer_mut(star).unwrap().parent = Some(null);
    composition.layer_mut(null).unwrap().transform.as_mut().unwrap().rotation = Animated::keyframed(vec![Keyframe::linear(0, 0.0), Keyframe::linear(59, 90.0)]);
    for layer in &mut composition.layers {
        layer.out_frame = 60;
    }

    let exported = lottie(&project, &options(comp, 0..60), &mut NoResources).unwrap();
    let animation = schema::check(&exported.file);
    let json = parse(&exported.file);
    let layers = json["layers"].as_array().unwrap();
    let names: Vec<&str> = layers.iter().map(|layer| layer["nm"].as_str().unwrap()).collect();
    let null_name = project.composition(comp).unwrap().layer(null).unwrap().name.clone();
    // The null's stand-in first, then each matte directly above the layer it cuts out.
    assert_eq!(names[0], format!("{null_name} (parent)"));
    let star_at = layers.iter().position(|layer| layer["tt"] == 1).expect("the star is masked (alpha matte)");
    assert_eq!(layers[star_at - 1]["td"], 1, "the mask's copy sits above it as a matte");
    assert_eq!(layers[star_at]["parent"], 1, "parented to the null's stand-in");
    let rect_at = layers.iter().position(|layer| layer["tt"] == 2).expect("the rectangle has an inverted alpha matte");
    assert_eq!(layers[rect_at - 1]["shapes"][0]["it"][0]["ty"], "el", "the circle is its matte");
    assert_eq!(animation.layers.iter().filter(|layer| layer.td == Some(1)).count(), 2);

    // The star: a path with rounded tips, a stroke on top, and its radial fill in a stretched
    // unit square (its box isn't square).
    let star_group = &layers[star_at]["shapes"][0]["it"];
    let stroke_part = &star_group[0]["it"];
    assert_eq!(stroke_part[0]["ty"], "sh");
    assert_eq!(stroke_part[1]["ty"], "st");
    assert_eq!(stroke_part[1]["lj"], 2, "round joins, as the renderer's distance field draws them");
    let fill_part = &star_group[1]["it"];
    assert_eq!(fill_part[1]["ty"], "gf");
    assert_eq!(fill_part[1]["t"], 2);
    assert_eq!(fill_part[2]["s"]["k"], serde_json::json!([12000, 6000]), "stretched to the 120 × 60 box");
    // The null turns, and the stand-in carries that.
    assert_eq!(layers[0]["ty"], 3);
    assert_eq!(layers[0]["ks"]["r"]["a"], 1);
}

#[test]
fn text_is_outlined_and_text_animators_are_baked_per_piece() {
    use studio_core::text_animation::{TextAnimator, TextMotion, TextUnit};
    let (mut project, comp, _) = keyframed_rectangle();
    let text = edit::add_text_layer(&mut project, comp, "Hi there", Vec2::new(200.0, 100.0), 0).unwrap();
    let layer = project.composition_mut(comp).unwrap().layer_mut(text).unwrap();
    layer.out_frame = 60;
    layer.animation.text_in = Some(TextAnimator { motion: TextMotion::Pop, unit: TextUnit::Word, seconds: 0.5, ..TextAnimator::default() });

    let exported = lottie(&project, &options(comp, 0..60), &mut FakeResources).unwrap();
    schema::check(&exported.file);
    assert!(exported.left_out.iter().any(|line| line.starts_with("Text animations were turned into keyframes")), "{:?}", exported.left_out);
    let json = parse(&exported.file);
    let text_layer = json["layers"].as_array().unwrap().iter().find(|layer| layer["nm"] == "Hi there" || layer["shapes"][0]["nm"] == "Hi there").expect("the text layer").clone();
    assert_eq!(text_layer["ty"], 4, "outlined text is a shape layer");
    // One group per word, each scaling up from nothing about its middle.
    let words: Vec<&Value> = text_layer["shapes"][0]["it"].as_array().unwrap().iter().filter(|item| item["ty"] == "gr").collect();
    assert_eq!(words.iter().map(|word| word["nm"].as_str().unwrap()).collect::<Vec<_>>(), ["Hi", "there"]);
    let transform = words[0]["it"].as_array().unwrap().last().unwrap();
    assert_eq!(transform["ty"], "tr");
    let scale = transform["s"]["k"].as_array().unwrap();
    assert_eq!(scale[0]["s"], serde_json::json!([0, 0]), "starts from nothing");
    assert_eq!(scale.last().unwrap()["s"], serde_json::json!([100, 100]), "ends full size");
    assert_eq!(scale.last().unwrap()["t"].as_f64(), Some(15.0), "half a second at 30 fps");

    // Without fonts, the text stays live text and says so.
    let live = lottie(&project, &options(comp, 0..60), &mut NoResources).unwrap();
    schema::check(&live.file);
    let json = parse(&live.file);
    assert!(json["layers"].as_array().unwrap().iter().any(|layer| layer["ty"] == 5));
    assert_eq!(json["fonts"]["list"][0]["fFamily"], "Inter");
    assert!(live.left_out.iter().any(|line| line.starts_with("Text animations were left out")));
}

#[test]
fn outlined_text_without_a_fill_stays_hollow() {
    let (mut project, comp, _) = keyframed_rectangle();
    let text = edit::add_text_layer(&mut project, comp, "O", Vec2::new(200.0, 100.0), 0).unwrap();
    let layer = project.composition_mut(comp).unwrap().layer_mut(text).unwrap();
    layer.out_frame = 60;
    if let studio_core::model::LayerContent::Text { style, .. } = &mut layer.content {
        style.fill = None;
        style.stroke = Some((Rgba::hex(0xff0000), 4.0));
    }
    // Outlined letters: a stroke and no fill, in both formats.
    let file = svg(&project, &options(comp, 0..60), &mut FakeResources).unwrap().file;
    assert!(!file.contains("fill=\"#ff0000\""), "the letters aren't filled with the outline's color");
    assert!(file.contains("stroke=\"#ff0000\""));
    let json = parse(&lottie(&project, &options(comp, 0..60), &mut FakeResources).unwrap().file);
    let letters = json["layers"].as_array().unwrap().iter().find(|layer| layer["nm"] == "O").expect("the text layer").to_string();
    assert!(letters.contains("\"ty\":\"st\"") && !letters.contains("\"ty\":\"fl\""), "a stroke and no fill");
    // Live text, without fonts: no fill color, so players leave the letters empty.
    let json = parse(&lottie(&project, &options(comp, 0..60), &mut NoResources).unwrap().file);
    let document = json["layers"].as_array().unwrap().iter().find(|layer| layer["ty"] == 5).unwrap()["t"]["d"]["k"][0]["s"].clone();
    assert!(document.get("fc").is_none() && document.get("sc").is_some(), "{document}");
    let file = svg(&project, &options(comp, 0..60), &mut NoResources).unwrap().file;
    assert!(file.contains("fill=\"none\""));
}

#[test]
fn emoji_and_shape_morphs_are_listed_as_left_out() {
    let (mut project, comp, _) = keyframed_rectangle();
    let text = edit::add_text_layer(&mut project, comp, "Party 🎉", Vec2::new(200.0, 100.0), 0).unwrap();
    project.composition_mut(comp).unwrap().layer_mut(text).unwrap().out_frame = 60;
    let circle = edit::add_shape_layer(&mut project, comp, ShapeStyle::new(ShapeKind::Ellipse), Vec2::new(80.0, 80.0), Vec2::new(100.0, 100.0), 0).unwrap();
    let star = edit::add_shape_layer(&mut project, comp, ShapeStyle::new(ShapeKind::Star), Vec2::new(80.0, 80.0), Vec2::new(300.0, 100.0), 0).unwrap();
    studio_core::morph::morph_to(project.composition_mut(comp).unwrap(), circle, star, 0, 30, Easing::Linear).unwrap();
    for left_out in [lottie(&project, &options(comp, 0..60), &mut FakeResources).unwrap().left_out, svg(&project, &options(comp, 0..60), &mut FakeResources).unwrap().left_out] {
        assert!(left_out.iter().any(|line| line.starts_with("Emoji and other color characters were left out") && line.contains("Party")), "{left_out:?}");
        assert!(left_out.iter().any(|line| line.starts_with("Shape morphs were left out")), "{left_out:?}");
    }
    // Ordinary text loses nothing.
    let (plain, comp, _) = keyframed_rectangle();
    let mut plain = plain;
    let text = edit::add_text_layer(&mut plain, comp, "Hi there", Vec2::new(200.0, 100.0), 0).unwrap();
    plain.composition_mut(comp).unwrap().layer_mut(text).unwrap().out_frame = 60;
    assert!(lottie(&plain, &options(comp, 0..60), &mut FakeResources).unwrap().left_out.is_empty());
}

#[test]
fn pictures_are_embedded_and_stretched_to_their_box_once() {
    let (mut project, comp, _) = keyframed_rectangle();
    // The stand-in picture is 2 × 2 pixels; the layer's box is 200 × 100.
    let item = edit::import_image(&mut project, "/pictures/logo.png".into(), Vec2::new(200.0, 100.0));
    let layer = edit::add_layer_from_item(&mut project, comp, item, 0, 0).unwrap();
    project.composition_mut(comp).unwrap().layer_mut(layer).unwrap().out_frame = 60;

    let exported = lottie(&project, &options(comp, 0..60), &mut FakeResources).unwrap();
    schema::check(&exported.file);
    let json = parse(&exported.file);
    assert_eq!(json["assets"][0]["p"], "data:image/png;base64,iVBORw==", "embedded as base64");
    let image = json["layers"].as_array().unwrap().iter().find(|layer| layer["ty"] == 2).unwrap();
    // Lottie draws the picture at its own size, so the layer scales it into the box.
    assert_eq!(image["ks"]["s"]["k"], serde_json::json!([10000, 5000]));
    assert_eq!(image["ks"]["a"]["k"], serde_json::json!([1, 1]));

    let file = svg(&project, &options(comp, 0..60), &mut FakeResources).unwrap().file;
    assert_eq!(file.matches("scale(100 50)").count(), 1, "stretched once, inside the layer");
    assert!(file.contains("translate(-100 -50)"), "the layer keeps its own anchor point");
}

#[test]
fn video_effects_and_sound_are_listed_as_left_out() {
    let (mut project, comp, rectangle) = keyframed_rectangle();
    edit::add_effect(&mut project, comp, rectangle, "Gaussian Blur");
    let info = edit::FootageInfo { size: Vec2::new(1920.0, 1080.0), duration_seconds: 2.0, frame_rate: FrameRate::new(30, 1), has_audio: true };
    let clip = edit::import_footage(&mut project, "/clips/Intro.mov".into(), &info);
    edit::add_layer_from_item(&mut project, comp, clip, 0, 0);
    for exported in [lottie(&project, &options(comp, 0..60), &mut NoResources).unwrap().left_out, svg(&project, &options(comp, 0..60), &mut NoResources).unwrap().left_out] {
        assert!(exported.iter().any(|line| line.starts_with("Video layers were skipped") && line.contains("Intro")), "{exported:?}");
        assert!(exported.iter().any(|line| line.starts_with("Effects were left out") && line.contains("Gaussian Blur")), "{exported:?}");
    }
}

#[test]
fn a_dotlottie_is_a_smaller_zip_of_the_animation() {
    let (project, comp, _) = keyframed_rectangle();
    let archive = dotlottie(&project, &options(comp, 0..60), &mut NoResources).unwrap().file;
    assert_eq!(&archive[..4], b"PK\x03\x04", "a zip file");
    let plain = lottie(&project, &options(comp, 0..60), &mut NoResources).unwrap().file;
    assert!(archive.len() < plain.len(), "compressed: {} vs {} bytes", archive.len(), plain.len());
}

#[test]
fn the_svg_animates_with_css_keyframes_keeping_the_curves() {
    let (project, comp, _) = keyframed_rectangle();
    let exported = svg(&project, &options(comp, 0..60), &mut NoResources).unwrap();
    let file = &exported.file;
    assert!(file.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"400\" height=\"300\" viewBox=\"0 0 400 300\">"));
    well_formed(file);
    assert!(file.contains("animation-timing-function:cubic-bezier(0.16,1,0.3,1)"), "the ease-out curve is kept");
    assert!(file.contains("transform:translate(50px,150px)") && file.contains("transform:translate(350px,150px)"));
    assert!(file.contains("animation-timing-function:steps(1,end)"), "the hold");
    assert!(file.contains("2s linear infinite both"), "two seconds, looping");
    assert!(file.contains("<rect x=\"0\" y=\"0\" width=\"100\" height=\"50\" rx=\"8\" fill=\"#ff3366\" stroke=\"none\"/>"), "{file}");
}

#[test]
fn templates_export_in_every_format() {
    use studio_core::templates::{Format, TEMPLATES, build};
    // Every template, in the vertical format (the others only move things about), with
    // stand-in fonts and pictures (so text is outlined and text animators are baked) and without.
    for (index, template) in TEMPLATES.iter().enumerate() {
        let mut project = Project::new(Vec::new(), Vec::new());
        let comp = build(&mut project, template, Format::Vertical).unwrap();
        let duration = project.composition(comp).unwrap().duration_frames;
        let request = options(comp, 0..duration);
        let with_fonts = lottie(&project, &request, &mut FakeResources).unwrap_or_else(|error| panic!("template {index} {}: {error}", template.name));
        schema::check(&with_fonts.file);
        schema::check(&lottie(&project, &request, &mut NoResources).unwrap().file);
        assert!(!dotlottie(&project, &request, &mut NoResources).unwrap().file.is_empty());
        well_formed(&svg(&project, &request, &mut FakeResources).unwrap().file);
    }
}

/// Checks every tag in `xml` is closed in order (enough to catch a writer slip; the content has
/// no comments, CDATA or processing instructions).
fn well_formed(xml: &str) {
    let mut open: Vec<&str> = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find('<') {
        let end = rest[start..].find('>').expect("every tag ends") + start;
        let tag = &rest[start + 1..end];
        if let Some(name) = tag.strip_prefix('/') {
            assert_eq!(open.pop(), Some(name), "closing </{name}>");
        } else {
            // No attribute may appear twice in a tag.
            let names: Vec<&str> = tag.split('"').step_by(2).filter_map(|part| part.trim().strip_suffix('=')).map(|part| part.rsplit(' ').next().unwrap()).collect();
            let unique: std::collections::HashSet<&&str> = names.iter().collect();
            assert_eq!(unique.len(), names.len(), "repeated attribute in <{tag}>");
            if !tag.ends_with('/') {
                open.push(tag.split([' ', '\n']).next().unwrap());
            }
        }
        rest = &rest[end + 1..];
    }
    assert!(open.is_empty(), "unclosed: {open:?}");
}
