//! Headless UI tests: the real editor window rendered offscreen and driven with
//! simulated clicks, drags and keys. They check that each control is wired to
//! the behavior it promises, without touching the real mouse or screen.

use gpui_kit::base::Root;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AppContext as _, ElementId, Entity, MouseButton, Pixels, Point, TestAppContext, VisualTestContext, point, px, size,
};
use studio_core::animation::{Easing, Vec2};
use studio_core::model::{Id, LayerContent};
use studio_core::property::{PropertyPath, PropertyValue, TransformProperty};

use crate::panels::fields::FieldTarget;
use crate::state::KeyframeRef;
use crate::workspace::Workspace;

const TITLE: Id = Id(100);
const ROTATION: PropertyPath = PropertyPath::Transform(TransformProperty::Rotation);
const POSITION: PropertyPath = PropertyPath::Transform(TransformProperty::Position);
const OPACITY: PropertyPath = PropertyPath::Transform(TransformProperty::Opacity);

/// Opens the editor in a 1600 × 960 offscreen window and renders the first frames.
fn open(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply(crate::theme::ThemeName::Dark, crate::theme::Accent::Violet, cx);
        crate::workspace::bind_keys(cx);
    });
    let mut workspace = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    visual.simulate_resize(size(px(1600.0), px(960.0)));
    // Measured bounds settle over two frames.
    for _ in 0..3 {
        visual.update(|window, cx| window.render_frame(cx));
    }
    (workspace.expect("workspace was built"), visual)
}

/// Draws a frame the way the running app does, honoring cached views. (`render_frame` forces a
/// full refresh, which ignores caching, so it cannot measure or test caching.)
fn draw_frame(visual: &mut VisualTestContext) {
    visual.update(|window, cx| window.draw(cx).clear(cx));
}

fn render(visual: &mut VisualTestContext) {
    for _ in 0..2 {
        visual.update(|window, cx| window.render_frame(cx));
    }
}

fn center_of(visual: &mut VisualTestContext, id: impl Into<ElementId>) -> Point<Pixels> {
    let id = id.into();
    visual.update(|window, _| window.find(id).bounds().center())
}

fn field_id(target: FieldTarget) -> ElementId {
    ElementId::Name(format!("field:{target:?}").into())
}

/// Where a composition point is drawn on screen, from the viewer's measured frame.
fn screen_point(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, comp: Vec2) -> Point<Pixels> {
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        let frame = ws.ui.frame_bounds.get();
        let scale = frame.width / ws.state.composition().width as f32;
        point(px(frame.left + comp.x * scale), px(frame.top + comp.y * scale))
    })
}

fn title_value(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, path: PropertyPath, frame: i64) -> PropertyValue {
    visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().property(path, frame).unwrap().value)
}

#[gpui_kit::test]
fn clicking_a_layer_in_the_viewer_selects_it_and_dragging_moves_it(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.set_current_frame(60)));
    render(visual);

    let from = screen_point(&workspace, visual, Vec2::new(960.0, 540.0));
    let to = point(from.x + px(50.0), from.y);
    visual.update(|window, cx| window.drag(from, to, cx));
    render(visual);

    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.selected_layers, vec![TITLE]));
    let PropertyValue::Pair(position) = title_value(&workspace, visual, POSITION, 60) else { panic!("position is 2D") };
    assert!(position.x > 1000.0, "the title moved right, now at {position:?}");
    let keys = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().property(POSITION, 0).unwrap().keyframes);
    assert!(keys.contains(&60), "moving an animated position adds a keyframe at the playhead");
}

#[gpui_kit::test]
fn property_fields_scrub_and_accept_typed_values(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.select_layer(TITLE, false)));
    render(visual);

    let rotation = FieldTarget::Property { layer: TITLE, path: ROTATION, component: 0 };
    let start = center_of(visual, field_id(rotation));
    visual.update(|window, cx| window.drag(start, point(start.x + px(40.0), start.y), cx));
    render(visual);
    let PropertyValue::Scalar(scrubbed) = title_value(&workspace, visual, ROTATION, 0) else { panic!() };
    assert!(scrubbed > 10.0, "dragging right increases rotation, got {scrubbed}");

    visual.update(|window, cx| window.click(field_id(rotation), cx));
    render(visual);
    visual.update(|window, cx| {
        window.input("90", cx);
        window.press("enter", cx);
    });
    render(visual);
    assert_eq!(title_value(&workspace, visual, ROTATION, 0), PropertyValue::Scalar(90.0));
}

#[gpui_kit::test]
fn opening_a_field_and_leaving_it_unchanged_changes_nothing(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    // Between keyframes, where the animated position is not a round number.
    visual.update(|_, cx| workspace.update(cx, |ws, _| {
        ws.state.select_layer(TITLE, false);
        ws.state.set_current_frame(45);
    }));
    render(visual);
    let keys = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().property(POSITION, 0).unwrap().keyframes);
    let (before, keys_before) = (title_value(&workspace, visual, POSITION, 45), keys(visual));
    let x = FieldTarget::Property { layer: TITLE, path: POSITION, component: 0 };
    visual.update(|window, cx| window.click(field_id(x), cx));
    render(visual);
    visual.update(|window, cx| window.press("enter", cx));
    render(visual);
    assert_eq!(title_value(&workspace, visual, POSITION, 45), before, "the exact value is kept, not the rounded one shown");
    assert_eq!(keys(visual), keys_before, "no keyframe was added");
    assert!(!has_unsaved_changes(&workspace, visual));
}

#[gpui_kit::test]
fn the_first_and_last_frame_buttons_match_home_and_end(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    // A trimmed work area, so its ends differ from the composition's.
    visual.update(|_, cx| workspace.update(cx, |ws, _| {
        ws.state.work_area = studio_core::time::FrameRange::new(30.0, 90.0);
        ws.state.set_current_frame(60);
    }));
    render(visual);
    let frame = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.current_frame);
    let last = visual.update(|_, cx| workspace.read(cx).state.composition().duration_frames - 1);
    visual.update(|window, cx| window.click("last-frame", cx));
    assert_eq!(frame(visual), last);
    visual.update(|window, cx| window.click("first-frame", cx));
    assert_eq!(frame(visual), 0);
    visual.update(|window, cx| window.press("end", cx));
    assert_eq!(frame(visual), last);
    visual.update(|window, cx| window.click("tracks-start", cx));
    assert_eq!(frame(visual), 0);
}

#[gpui_kit::test]
fn the_stopwatch_turns_animation_on(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.state.select_layer(TITLE, false);
            ws.state.set_current_frame(30);
        })
    });
    render(visual);
    visual.update(|window, cx| window.click(ElementId::Name(format!("stopwatch:{ROTATION:?}:{}", TITLE.0).into()), cx));
    render(visual);
    let keys = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().property(ROTATION, 0).unwrap().keyframes);
    assert_eq!(keys, vec![30]);
}

#[gpui_kit::test]
fn delete_removes_selected_keyframes_then_layers(cx: &mut TestAppContext) {
    let (workspace, visual) = open_title_keyframes(cx);
    // Click the opacity keyframe at frame 36 under the title clip, then press Delete.
    visual.update(|window, cx| window.click(ElementId::Name(format!("clip-kf:{}:{OPACITY:?}:36", TITLE.0).into()), cx));
    render(visual);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        assert_eq!(state.selected_keyframes, vec![KeyframeRef { layer: TITLE, path: OPACITY, frame: 36 }]);
    });
    visual.update(|window, cx| window.press("backspace", cx));
    render(visual);
    let keys = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().property(OPACITY, 0).unwrap().keyframes);
    assert_eq!(keys, vec![24, 156, 168]);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.select_layer(TITLE, false); cx.notify(); }));
    visual.update(|window, cx| window.press("backspace", cx));
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).state.composition().layer(TITLE).is_none(), "the layer is deleted next"));
}

#[gpui_kit::test]
fn the_graph_editor_applies_easing_families_to_selected_keyframes(cx: &mut TestAppContext) {
    use studio_core::animation::{EASING_FAMILIES, EaseMode};
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| {
        workspace.update(cx, |ws, _| ws.state.select_keyframe(KeyframeRef { layer: TITLE, path: OPACITY, frame: 24 }, false))
    });
    render(visual);
    visual.update(|window, cx| window.click("easings", cx));
    render(visual);
    let easing = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.selection_easing());
    // Expo, out form.
    visual.update(|window, cx| window.click(ElementId::Name("easing:Expo:Out".into()), cx));
    render(visual);
    let expo = EASING_FAMILIES.iter().find(|(name, _)| *name == "Expo").unwrap().1;
    assert_eq!(easing(visual), Some(Easing::Curve(expo[1])));
    // Bounce, both ends.
    visual.update(|window, cx| window.click(ElementId::Name("easing:Bounce:Both".into()), cx));
    render(visual);
    assert_eq!(easing(visual), Some(Easing::Bounce(EaseMode::Both)));
    // Linear and Hold rows.
    visual.update(|window, cx| window.click(("easing-row", 1_usize), cx));
    render(visual);
    assert_eq!(easing(visual), Some(Easing::Hold));
}

#[gpui_kit::test]
fn dragging_a_graph_handle_makes_a_custom_curve(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| {
        workspace.update(cx, |ws, _| ws.state.select_keyframe(KeyframeRef { layer: TITLE, path: OPACITY, frame: 24 }, false))
    });
    render(visual);
    visual.update(|window, cx| window.click("easings", cx));
    render(visual);
    visual.update(|window, cx| window.click(ElementId::Name("easing:Quad:Both".into()), cx));
    render(visual);
    let handle = center_of(visual, ("graph-handle", 0_usize));
    visual.update(|window, cx| window.drag(handle, point(handle.x + px(40.0), handle.y - px(60.0)), cx));
    render(visual);
    let Some(Easing::Curve(curve)) = visual.update(|_, cx| workspace.read(cx).state.selection_easing()) else { panic!("a curve") };
    assert!(curve.x1 > 0.5 && curve.y1 > 0.2, "the first handle moved right and up: {curve:?}");
}

#[gpui_kit::test]
fn clicking_away_closes_the_graph_editor(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.click("easings", cx));
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).ui.graph_editor.is_some()));
    let elsewhere = center_of(visual, "project-items");
    visual.update(|window, cx| {
        window.pointer_down(elsewhere, MouseButton::Left, cx);
        window.pointer_up(elsewhere, MouseButton::Left, cx);
    });
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).ui.graph_editor.is_none()));
}

#[gpui_kit::test]
fn effect_controls_edit_effect_parameters(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let a_cam = Id(104);
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.select_layer(a_cam, false)));
    render(visual);
    visual.update(|window, cx| window.click(ElementId::Name("tab:Properties:1".into()), cx));
    render(visual);
    let blur = FieldTarget::Property { layer: a_cam, path: PropertyPath::Effect { effect: Id(202), parameter: 0 }, component: 0 };
    visual.update(|window, cx| window.click(field_id(blur), cx));
    render(visual);
    visual.update(|window, cx| {
        window.input("25", cx);
        window.press("enter", cx);
    });
    render(visual);
    let blurriness = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        state.composition().layer(a_cam).unwrap().property(PropertyPath::Effect { effect: Id(202), parameter: 0 }, state.current_frame).unwrap().value
    });
    // Blurriness is animated, so the typed value lands as a keyframe at the playhead (frame 0).
    assert_eq!(blurriness, PropertyValue::Scalar(25.0));
}

#[gpui_kit::test]
fn the_type_tool_places_text_and_typing_sets_it(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.press("cmd-t", cx));
    render(visual);
    let at = screen_point(&workspace, visual, Vec2::new(400.0, 300.0));
    visual.update(|window, cx| window.click_at("viewer", at - window.find("viewer").bounds().origin, cx));
    render(visual);
    visual.update(|window, cx| {
        window.input("Hello", cx);
        window.press("enter", cx);
    });
    render(visual);
    visual.update(|_, cx| {
        let comp = workspace.read(cx).state.composition();
        let LayerContent::Text { text, .. } = &comp.layers[0].content else { panic!("a text layer is on top") };
        assert_eq!(text, "Hello");
        assert_eq!(comp.layers[0].name, "Hello");
    });
}

#[gpui_kit::test]
fn typing_in_a_field_never_triggers_editor_shortcuts(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.select_layer(TITLE, false)));
    render(visual);
    visual.update(|window, cx| window.click(ElementId::Name("source-text".into()), cx));
    render(visual);
    // Real key presses, not text input: these keys are also editor shortcuts (S snapping,
    // V selection tool, Space play, Alt+[ trim), which must not fire while typing.
    let (_, in_before, _) = layer_timing(&workspace, visual, TITLE);
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.set_current_frame(60)));
    visual.update(|window, cx| {
        window.press("cmd-a", cx);
        for key in ["s", "space", "v", "alt-["] {
            window.press(key, cx);
        }
        window.press("enter", cx);
    });
    render(visual);
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        assert!(!ws.state.is_playing(), "space typed into a field stays in the field");
        assert!(ws.state.snapping, "S typed into a field does not toggle snapping");
        assert_eq!(ws.state.composition().layer(TITLE).unwrap().in_frame, in_before, "Alt+[ typed into a field does not trim");
        assert!(ws.state.text_of(TITLE).unwrap().0.starts_with("s v"), "the keys were typed: {:?}", ws.state.text_of(TITLE).unwrap().0);
    });
    // Outside a field, Space plays.
    visual.update(|window, cx| window.press("space", cx));
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).state.is_playing()));
}

/// Clicks the first element whose accessible label is `label` (dialog buttons have no stable id).
fn click_label(visual: &mut VisualTestContext, label: &str) {
    visual.update(|window, cx| {
        let target = window.find_by_label(label).bounds().center();
        window.pointer_down(target, MouseButton::Left, cx);
        window.pointer_up(target, MouseButton::Left, cx);
    });
}

#[gpui_kit::test]
fn deleting_a_project_item_asks_first_and_removes_its_layers(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    // Select A-cam_01.mov in the Project panel, then press Delete there.
    visual.update(|window, cx| window.click(("project-item", 4_usize), cx));
    render(visual);
    visual.update(|window, cx| window.press("backspace", cx));
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).state.project.item(Id(4)).is_some(), "nothing is deleted before confirming"));

    click_label(visual, "Delete");
    render(visual);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        assert!(state.project.item(Id(4)).is_none());
        assert!(state.composition().layer(Id(104)).is_none(), "the layer using the clip is removed too");
    });
}

#[gpui_kit::test]
fn new_composition_dialog_makes_a_custom_size_comp(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.press("cmd-n", cx));
    render(visual);
    visual.update(|window, cx| window.click("comp-preset", cx));
    render(visual);
    click_label(visual, "Social Vertical 9:16 · 1080×1920 · 30 fps");
    render(visual);
    click_label(visual, "OK");
    render(visual);
    visual.update(|_, cx| {
        let comp = workspace.read(cx).state.composition();
        assert_eq!((comp.width, comp.height), (1080, 1920));
        assert_eq!(comp.name, "Comp 2");
    });
}

#[gpui_kit::test]
fn corner_handles_scale_the_selected_layer(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let solid = Id(105);
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.select_layer(solid, false)));
    render(visual);
    let corner = center_of(visual, ElementId::Name(format!("scale-handle:{}:BottomRight", solid.0).into()));
    visual.update(|window, cx| window.drag(corner, point(corner.x - px(100.0), corner.y - px(100.0)), cx));
    render(visual);
    let scale = visual.update(|_, cx| workspace.read(cx).state.composition().layer(solid).unwrap().property(PropertyPath::Transform(TransformProperty::Scale), 0).unwrap().value);
    let PropertyValue::Pair(scale) = scale else { panic!() };
    assert!(scale.x < 100.0 && scale.y < 100.0, "dragging the corner inward shrinks the layer: {scale:?}");
}

#[gpui_kit::test]
fn align_buttons_move_the_selection(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, _| {
        ws.state.select_layer(TITLE, false);
        ws.state.set_current_frame(100);
    }));
    render(visual);
    // Align Left (to the composition) puts the title's left edge at x = 0.
    visual.update(|window, cx| window.click(("align", 0_usize), cx));
    render(visual);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        let bounds = studio_core::align::layer_bounds(state.composition(), TITLE, 100).unwrap();
        assert!(bounds.min.x.abs() < 0.01, "left edge at {}", bounds.min.x);
    });
}

/// Twirls the title open to show its transform properties in the timeline.

#[gpui_kit::test]
fn pressing_a_keyframe_row_then_f9_eases_every_segment(cx: &mut TestAppContext) {
    let (workspace, visual) = open_title_keyframes(cx);
    let row = ElementId::Name(format!("clip-prop:{}:{OPACITY:?}", TITLE.0).into());
    let bounds = bounds_of(visual, row.clone());
    visual.update(|window, cx| window.click_with_options(row, gpui_kit::test::ClickOptions::new().with_offset(point(bounds.size.width - px(30.0), px(6.0))), cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.selected_keyframes.len(), 4));
    visual.update(|window, cx| window.press("f9", cx));
    render(visual);
    visual.update(|_, cx| {
        let easings = workspace.read(cx).state.composition().layer(TITLE).unwrap().keyframe_easings(OPACITY);
        let ease = Easing::Curve(studio_core::animation::EASY_EASE);
        assert!(easings[..3].iter().all(|(_, easing)| *easing == ease), "every segment between them is eased: {easings:?}");
    });
}

#[gpui_kit::test]
fn typing_a_value_on_a_selected_keyframe_edits_all_selected(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.state.select_keyframe(KeyframeRef { layer: TITLE, path: OPACITY, frame: 36 }, false);
            ws.state.select_keyframe(KeyframeRef { layer: TITLE, path: OPACITY, frame: 156 }, true);
            ws.state.set_current_frame(36);
        })
    });
    render(visual);
    let field = FieldTarget::Property { layer: TITLE, path: OPACITY, component: 0 };
    visual.update(|window, cx| window.click(field_id(field), cx));
    render(visual);
    visual.update(|window, cx| {
        window.input("40", cx);
        window.press("enter", cx);
    });
    render(visual);
    assert_eq!(title_value(&workspace, visual, OPACITY, 36), PropertyValue::Scalar(40.0));
    assert_eq!(title_value(&workspace, visual, OPACITY, 156), PropertyValue::Scalar(40.0));
}

#[gpui_kit::test]
fn the_new_composition_button_opens_the_dialog(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.click("new-comp", cx));
    render(visual);
    visual.update(|window, cx| window.click("comp-preset", cx));
    render(visual);
    click_label(visual, "Social Square 1:1 · 1080×1080 · 30 fps");
    render(visual);
    // The background menu sets the new composition's background.
    visual.update(|window, cx| window.click("comp-background", cx));
    render(visual);
    click_label(visual, "Midnight Blue");
    render(visual);
    click_label(visual, "OK");
    render(visual);
    visual.update(|_, cx| {
        let comp = workspace.read(cx).state.composition();
        assert_eq!((comp.width, comp.height), (1080, 1080));
        assert_eq!(comp.frame_rate, studio_core::time::FrameRate::new(30, 1));
        assert_eq!(comp.background.to_rgba_u32(), studio_core::model::Rgba::hex(0x0d1b2a).to_rgba_u32());
    });
}

/// Measures frame cost. Playback frames come from the playback clock (only timed panels
/// re-render); edit frames notify the workspace (every panel re-renders). Run with
/// `cargo test -p studio frame_time -- --ignored --nocapture` (add `--release` for optimized numbers).
#[gpui_kit::test]
#[ignore = "benchmark; run on demand"]
fn frame_time_during_playback(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let frames: u32 = std::env::var("BENCH_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(120);
    for (label, selected) in [("nothing selected", None), ("text layer selected", Some(TITLE))] {
        visual.update(|_, cx| {
            workspace.update(cx, |ws, cx| {
                ws.state.clear_selection();
                if let Some(layer) = selected {
                    ws.state.select_layer(layer, false);
                }
                ws.ui.timeline.expanded_clips.insert(TITLE);
                cx.notify();
            })
        });
        render(visual);

        visual.update(|_, cx| workspace.update(cx, |ws, cx| {
            ws.toggle_playback(cx);
        }));
        render(visual);
        let before = visual.update(|_, cx| workspace.read(cx).state.current_frame);
        let start = std::time::Instant::now();
        for _ in 0..frames {
            draw_frame(visual);
        }
        let playback = start.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
        let after = visual.update(|_, cx| workspace.read(cx).state.current_frame);
        visual.update(|_, cx| workspace.update(cx, |ws, cx| ws.toggle_playback(cx)));
        assert_ne!(before, after, "the playhead advanced during playback");

        let start = std::time::Instant::now();
        for frame in 0..frames {
            visual.update(|_, cx| workspace.update(cx, |ws, cx| {
                ws.state.current_frame = i64::from(frame % 240);
                cx.notify();
            }));
            draw_frame(visual);
        }
        let edits = start.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
        println!("frame time ({label}): playback {playback:.2} ms, edit {edits:.2} ms");
    }
}

#[gpui_kit::test]
fn the_properties_panel_sets_blend_mode_and_parent(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.select_layer(TITLE, false); cx.notify(); }));
    render(visual);
    visual.update(|window, cx| window.click("layer-blend", cx));
    render(visual);
    click_label(visual, "Multiply");
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.composition().layer(TITLE).unwrap().blend_mode, studio_core::model::BlendMode::Multiply));
    visual.update(|window, cx| window.click("layer-solo", cx));
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).state.composition().layer(TITLE).unwrap().switches.solo));
}

#[gpui_kit::test]
fn playback_rerenders_only_the_panels_that_show_time(cx: &mut TestAppContext) {
    use crate::panels::views::PanelKind;
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| ws.toggle_playback(cx)));
    render(visual);
    let before = visual.update(|_, cx| workspace.read(cx).panel_render_counts(cx));
    for _ in 0..30 {
        draw_frame(visual);
    }
    let after = visual.update(|_, cx| workspace.read(cx).panel_render_counts(cx));
    let rendered = |kind: PanelKind| after[kind as usize] - before[kind as usize];
    for kind in [PanelKind::Toolbar, PanelKind::Project, PanelKind::Effects, PanelKind::Libraries, PanelKind::Align] {
        assert_eq!(rendered(kind), 0, "{kind:?} should stay cached during playback");
    }
    for kind in [PanelKind::Viewer, PanelKind::Timeline, PanelKind::Properties] {
        assert!(rendered(kind) >= 20, "{kind:?} should refresh while playing, rendered {}", rendered(kind));
    }
}

fn has_unsaved_changes(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> bool {
    visual.update(|_, cx| workspace.read(cx).state.has_unsaved_changes())
}

#[gpui_kit::test]
fn a_fresh_editor_has_nothing_to_undo_or_save(cx: &mut TestAppContext) {
    // Fitting text boxes to their text on the first frames must not count as an edit.
    let (workspace, visual) = open(cx);
    render(visual);
    visual.update(|_, cx| assert!(!workspace.read(cx).state.can_undo()));
    assert!(!has_unsaved_changes(&workspace, visual));
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.window_title, "Untitled — Studio"));
}

#[gpui_kit::test]
fn dragging_a_layer_undoes_in_one_step_and_redoes(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.set_current_frame(60)));
    render(visual);
    let before = title_value(&workspace, visual, POSITION, 60);

    let from = screen_point(&workspace, visual, Vec2::new(960.0, 540.0));
    visual.update(|window, cx| window.drag(from, point(from.x + px(50.0), from.y + px(20.0)), cx));
    render(visual);
    let moved = title_value(&workspace, visual, POSITION, 60);
    assert_ne!(moved, before);
    assert!(has_unsaved_changes(&workspace, visual));
    visual.update(|_, cx| assert!(workspace.read(cx).ui.window_title.contains('•'), "the title bar shows unsaved changes"));

    visual.update(|window, cx| window.press("cmd-z", cx));
    render(visual);
    assert_eq!(title_value(&workspace, visual, POSITION, 60), before, "one undo reverts the whole drag");
    visual.update(|_, cx| assert!(!workspace.read(cx).state.can_undo()));
    assert!(!has_unsaved_changes(&workspace, visual), "back to how it was");

    visual.update(|window, cx| window.press("cmd-shift-z", cx));
    render(visual);
    assert_eq!(title_value(&workspace, visual, POSITION, 60), moved);
}

#[gpui_kit::test]
fn scrubbing_a_field_undoes_in_one_step(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.select_layer(TITLE, false)));
    render(visual);
    let before = title_value(&workspace, visual, ROTATION, 0);
    let rotation = FieldTarget::Property { layer: TITLE, path: ROTATION, component: 0 };
    let start = center_of(visual, field_id(rotation));
    visual.update(|window, cx| window.drag(start, point(start.x + px(40.0), start.y), cx));
    render(visual);
    assert_ne!(title_value(&workspace, visual, ROTATION, 0), before);
    visual.update(|window, cx| window.press("cmd-z", cx));
    render(visual);
    assert_eq!(title_value(&workspace, visual, ROTATION, 0), before);
}

#[gpui_kit::test]
fn undo_brings_back_a_deleted_layer(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.select_layer(TITLE, false)));
    render(visual);
    visual.update(|window, cx| window.press("backspace", cx));
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).state.composition().layer(TITLE).is_none()));
    visual.update(|window, cx| window.press("cmd-z", cx));
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).state.composition().layer(TITLE).is_some()));
}

/// A fresh, empty folder for one test's files.
fn scratch_folder(name: &str) -> std::path::PathBuf {
    let folder = std::env::temp_dir().join(format!("studio-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// Rotates the title a little: a quick edit that leaves unsaved changes.
fn make_an_edit(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    visual.update(|_, cx| {
        workspace.update(cx, |ws, cx| {
            ws.state.set_property_component(TITLE, ROTATION, 0, 33.0);
            cx.notify();
        })
    });
    render(visual);
}

#[gpui_kit::test]
fn saving_asks_where_the_first_time_then_saves_in_place(cx: &mut TestAppContext) {
    let folder = scratch_folder("save");
    let (workspace, visual) = open(cx);
    make_an_edit(&workspace, visual);

    visual.update(|window, cx| window.press("cmd-s", cx));
    visual.run_until_parked();
    assert!(visual.did_prompt_for_new_path(), "an untitled project asks where to save");
    let chosen = folder.join("Promo");
    visual.simulate_new_path_selection(|_| Some(chosen.clone()));
    visual.run_until_parked();
    render(visual);

    let saved = folder.join("Promo.studio");
    assert!(saved.exists(), "the extension is added");
    assert!(!has_unsaved_changes(&workspace, visual));
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.window_title, "Promo — Studio"));

    // The second save writes straight to the same file.
    make_an_edit(&workspace, visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.set_property_component(TITLE, ROTATION, 0, 50.0); cx.notify(); }));
    visual.update(|window, cx| window.press("cmd-s", cx));
    visual.run_until_parked();
    assert!(!visual.did_prompt_for_new_path());
    assert!(std::fs::read_to_string(&saved).unwrap().contains("50.0"));
    std::fs::remove_dir_all(&folder).unwrap();
}

#[gpui_kit::test]
fn opening_with_unsaved_changes_asks_first(cx: &mut TestAppContext) {
    let folder = scratch_folder("open");
    let (workspace, visual) = open(cx);
    // A project on disk to open: the sample with the title rotated to 33°.
    make_an_edit(&workspace, visual);
    let file = folder.join("Other.studio");
    let text = visual.update(|_, cx| workspace.read(cx).state.project_text().unwrap());
    std::fs::write(&file, text).unwrap();
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.set_property_component(TITLE, ROTATION, 0, 70.0); cx.notify(); }));
    render(visual);

    // Cancel keeps everything as it is.
    visual.update(|window, cx| window.press("cmd-o", cx));
    visual.run_until_parked();
    assert!(visual.has_pending_prompt());
    visual.simulate_prompt_answer("Cancel");
    visual.run_until_parked();
    assert!(!visual.did_prompt_for_paths());
    assert_eq!(title_value(&workspace, visual, ROTATION, 0), PropertyValue::Scalar(70.0));

    // Don't Save goes on to open the other project.
    visual.update(|window, cx| window.press("cmd-o", cx));
    visual.run_until_parked();
    visual.simulate_prompt_answer("Don’t Save");
    visual.run_until_parked();
    assert!(visual.did_prompt_for_paths());
    visual.simulate_path_prompt_response(|_| Some(vec![file.clone()]));
    visual.run_until_parked();
    render(visual);
    assert_eq!(title_value(&workspace, visual, ROTATION, 0), PropertyValue::Scalar(33.0));
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.window_title, "Other — Studio"));
    std::fs::remove_dir_all(&folder).unwrap();
}

#[gpui_kit::test]
fn opening_a_damaged_file_explains_and_keeps_the_project(cx: &mut TestAppContext) {
    let folder = scratch_folder("damaged");
    let file = folder.join("Broken.studio");
    std::fs::write(&file, "{\"format\": \"studio-project\", \"version\": 1, \"proj").unwrap();
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.press("cmd-o", cx));
    visual.run_until_parked();
    visual.simulate_path_prompt_response(|_| Some(vec![file.clone()]));
    visual.run_until_parked();
    render(visual);
    visual.update(|window, cx| assert!(window.has_active_dialog(cx), "an error dialog explains what went wrong"));
    // The dialog has an OK button to dismiss it.
    click_label(visual, "OK");
    render(visual);
    visual.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    visual.update(|_, cx| assert!(workspace.read(cx).state.file().is_none(), "still the same untitled project"));
    std::fs::remove_dir_all(&folder).unwrap();
}

#[gpui_kit::test]
fn closing_the_window_with_unsaved_changes_asks_first(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    make_an_edit(&workspace, visual);
    assert!(!visual.simulate_close(), "the window stays open while asking");
    visual.run_until_parked();
    assert!(visual.has_pending_prompt());
}

#[gpui_kit::test]
fn quitting_with_unsaved_changes_asks_first(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    make_an_edit(&workspace, visual);
    visual.update(|window, cx| window.press("cmd-q", cx));
    visual.run_until_parked();
    assert!(visual.has_pending_prompt(), "the window's own Quit handler runs, not the app-wide one");
    visual.simulate_prompt_answer("Cancel");
    visual.run_until_parked();
    assert!(has_unsaved_changes(&workspace, visual), "cancelling keeps working");
}

/// Waits (up to 60 s) for the background export to finish, letting its events through.
fn wait_for_export(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    let started = std::time::Instant::now();
    while visual.update(|_, cx| workspace.read(cx).ui.export_job.is_some()) {
        assert!(started.elapsed() < std::time::Duration::from_secs(60), "the export never finished");
        // Real time for the export thread, then the editor's next check on it.
        std::thread::sleep(std::time::Duration::from_millis(20));
        visual.executor().advance_clock(std::time::Duration::from_millis(100));
        visual.run_until_parked();
    }
}

/// Opens the Export dialog with a short work area (frames 0–11) to keep the test quick.
fn open_export_dialog(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.set_work_area(studio_core::time::FrameRange::new(0.0, 12.0)); cx.notify(); }));
    render(visual);
    visual.update(|window, cx| window.press("cmd-m", cx));
    render(visual);
}

#[gpui_kit::test]
fn exporting_writes_the_chosen_format_in_the_background(cx: &mut TestAppContext) {
    let folder = scratch_folder("export");
    let (workspace, visual) = open(cx);
    open_export_dialog(&workspace, visual);
    visual.update(|window, cx| window.click("export-format", cx));
    render(visual);
    click_label(visual, "ProRes 422 HQ (MOV)");
    render(visual);
    // ProRes has no quality setting: that menu goes away.
    assert!(visual.update(|window, _| window.try_find("export-quality")).is_none());
    visual.update(|window, cx| window.click("export-size", cx));
    render(visual);
    click_label(visual, "Half (960 × 540)");
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.export_choice.scale, 0.5));
    click_label(visual, "Export…");
    visual.run_until_parked();
    assert!(visual.did_prompt_for_new_path());
    // A name without the right extension gets it.
    let chosen = folder.join("Main Comp");
    visual.simulate_new_path_selection(|_| Some(chosen.clone()));
    visual.run_until_parked();
    render(visual);
    visual.update(|window, _| assert!(window.try_find_by_label("Cancel").is_some(), "the toolbar shows the export with a Cancel button"));
    wait_for_export(&workspace, visual);

    let movie = folder.join("Main Comp.mov");
    let info = studio_media::probe(&movie).unwrap();
    assert_eq!((info.codec.as_str(), info.width, info.height, info.frame_count), ("prores", 960, 540, 12), "half size, as chosen");
    std::fs::remove_dir_all(&folder).unwrap();
}

#[gpui_kit::test]
fn cancelling_an_export_from_the_toolbar_leaves_no_file(cx: &mut TestAppContext) {
    let folder = scratch_folder("export-cancel");
    let (workspace, visual) = open(cx);
    // The whole 240-frame composition, so there is time to cancel.
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.ui.export_choice.work_area_only = false));
    visual.update(|window, cx| window.press("cmd-m", cx));
    render(visual);
    click_label(visual, "Export…");
    visual.run_until_parked();
    let chosen = folder.join("Cancelled.mp4");
    visual.simulate_new_path_selection(|_| Some(chosen.clone()));
    visual.run_until_parked();
    render(visual);
    click_label(visual, "Cancel");
    wait_for_export(&workspace, visual);
    assert!(std::fs::read_dir(&folder).unwrap().next().is_none(), "neither the movie nor its partial file remains");
    std::fs::remove_dir_all(&folder).unwrap();
}

#[gpui_kit::test]
fn quitting_during_an_export_asks_first(cx: &mut TestAppContext) {
    let folder = scratch_folder("export-quit");
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.ui.export_choice.work_area_only = false));
    visual.update(|window, cx| window.press("cmd-m", cx));
    render(visual);
    click_label(visual, "Export…");
    visual.run_until_parked();
    let chosen = folder.join("Busy.mp4");
    visual.simulate_new_path_selection(|_| Some(chosen.clone()));
    visual.run_until_parked();
    render(visual);
    visual.update(|window, cx| window.press("cmd-q", cx));
    visual.run_until_parked();
    assert!(visual.has_pending_prompt(), "asks before stopping the export");
    visual.simulate_prompt_answer("Stop Export and Quit");
    wait_for_export(&workspace, visual);
    assert!(std::fs::read_dir(&folder).unwrap().next().is_none(), "the partial file was removed before quitting");
    std::fs::remove_dir_all(&folder).unwrap();
}

#[gpui_kit::test]
fn shortcuts_still_work_after_a_dialog_closes_with_ok(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.press("cmd-k", cx));
    render(visual);
    click_label(visual, "OK");
    render(visual);
    visual.update(|window, cx| window.press("space", cx));
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).state.is_playing(), "Space reaches the editor without clicking first"));
}

fn layer_timing(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, layer: Id) -> (i64, i64, i64) {
    visual.update(|_, cx| {
        let layer = workspace.read(cx).state.composition().layer(layer).unwrap();
        (layer.start_frame, layer.in_frame, layer.out_frame)
    })
}

#[gpui_kit::test]
fn alt_brackets_trim_the_selected_layer_to_the_playhead(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        ws.state.set_current_frame(50);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.press("alt-[", cx));
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.set_current_frame(80); cx.notify(); }));
    render(visual);
    visual.update(|window, cx| window.press("alt-]", cx));
    render(visual);
    let (_, in_frame, out_frame) = layer_timing(&workspace, visual, TITLE);
    assert_eq!((in_frame, out_frame), (50, 81));
}

#[gpui_kit::test]
fn alt_dragging_a_clip_slips_its_source(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    // The colors clip (48 frames) on top, trimmed to frames 12–36 so there is source to slip.
    let clip = visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/studio-media/tests/fixtures/colors.mp4");
        ws.state.import_files(vec![path]);
        let item = ws.state.selected_item.unwrap();
        ws.state.add_layer_from_item(item, 0, 0);
        let clip = ws.state.composition().layers[0].id;
        let comp = ws.state.active_comp;
        studio_core::trim::trim_in(&mut ws.state.project, comp, clip, 12);
        studio_core::trim::trim_out(&mut ws.state.project, comp, clip, 36);
        ws.state.select_layer(clip, false);
        cx.notify();
        clip
    }));
    render(visual);
    let bar = center_of(visual, ("clip", clip.0 as usize));
    visual.update(|window, cx| {
        window.change_modifiers(gpui_kit::Modifiers { alt: true, ..Default::default() }, cx);
        window.drag(bar, point(bar.x + px(40.0), bar.y), cx);
        window.change_modifiers(gpui_kit::Modifiers::default(), cx);
    });
    render(visual);
    let (start, in_frame, out_frame) = layer_timing(&workspace, visual, clip);
    assert_eq!((in_frame, out_frame), (12, 36), "the clip stays where it is");
    assert!(start > 0 && start <= 12, "the source moved later under it, to {start}");
}

fn layer_ids(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> Vec<Id> {
    visual.update(|_, cx| workspace.read(cx).state.composition().layers.iter().map(|layer| layer.id).collect())
}

#[gpui_kit::test]
fn cmd_shift_d_splits_the_selected_layer_at_the_playhead(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let before = layer_ids(&workspace, visual);
    let (_, title_in, title_out) = layer_timing(&workspace, visual, TITLE);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        ws.state.set_current_frame(100);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.press("cmd-shift-d", cx));
    render(visual);

    let after = layer_ids(&workspace, visual);
    assert_eq!(after.len(), before.len() + 1);
    let second = after[0];
    assert_eq!(after[1], TITLE, "the second half sits above the first");
    assert_eq!(layer_timing(&workspace, visual, TITLE), (title_in, title_in, 100));
    assert_eq!(layer_timing(&workspace, visual, second).1..layer_timing(&workspace, visual, second).2, 100..title_out);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.selected_layers, vec![second], "the new half is selected"));

    visual.update(|window, cx| window.press("cmd-z", cx));
    render(visual);
    assert_eq!(layer_ids(&workspace, visual), before, "one undo puts it back together");
}

#[gpui_kit::test]
fn shift_delete_ripple_deletes_and_closes_the_gap(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let (_, title_in, title_out) = layer_timing(&workspace, visual, TITLE);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        ws.state.set_current_frame(100);
        cx.notify();
    }));
    visual.update(|window, cx| window.press("cmd-shift-d", cx));
    let second = layer_ids(&workspace, visual)[0];
    // Select the first half and ripple delete it: the second half moves back to where the title began.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.select_layer(TITLE, false); cx.notify(); }));
    render(visual);
    visual.update(|window, cx| window.press("shift-backspace", cx));
    render(visual);
    let ids = layer_ids(&workspace, visual);
    assert!(!ids.contains(&TITLE));
    let (_, in_frame, out_frame) = layer_timing(&workspace, visual, second);
    assert_eq!((in_frame, out_frame), (title_in, title_in + (title_out - 100)));
}

/// Screen pixels per timeline frame at the current zoom.
fn pixels_per_frame(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> f32 {
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        ws.ui.timeline.graph.get().width / ws.state.view_range.len() as f32
    })
}

/// Drags the title's bar `frames` to the right, holding it there; returns the release point.
fn hold_title_drag(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, frames: f32) -> Point<Pixels> {
    let bar = center_of(visual, ("clip", TITLE.0 as usize));
    let to = point(bar.x + px(frames * pixels_per_frame(workspace, visual)), bar.y);
    visual.update(|window, cx| {
        // Hover first, as a real pointer would, then press and move in steps without releasing.
        window.pointer_move(bar, None, cx);
        window.pointer_down(bar, MouseButton::Left, cx);
        for step in 1..=8 {
            let x = bar.x + (to.x - bar.x) * (step as f32 / 8.0);
            window.pointer_move(point(x, bar.y), Some(MouseButton::Left), cx);
        }
    });
    to
}

#[gpui_kit::test]
fn moving_a_layer_snaps_to_the_playhead_and_shows_where(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let (_, title_in, _) = layer_timing(&workspace, visual, TITLE);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.set_current_frame(title_in + 20); cx.notify(); }));
    render(visual);

    // Two frames short of the playhead: it snaps the rest of the way.
    let released = hold_title_drag(&workspace, visual, 18.0);
    assert_eq!(layer_timing(&workspace, visual, TITLE).1, title_in + 20);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.snap_line(), Some(title_in + 20), "a guide marks the snap"));
    visual.update(|window, cx| window.pointer_up(released, MouseButton::Left, cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.snap_line(), None, "the guide goes away on release"));
}

#[gpui_kit::test]
fn s_turns_snapping_off_and_on(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let (_, title_in, _) = layer_timing(&workspace, visual, TITLE);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.set_current_frame(title_in + 20); cx.notify(); }));
    render(visual);
    visual.update(|window, cx| window.press("s", cx));
    render(visual);
    visual.update(|_, cx| assert!(!workspace.read(cx).state.snapping));
    let released = hold_title_drag(&workspace, visual, 18.0);
    let moved = layer_timing(&workspace, visual, TITLE).1;
    visual.update(|window, cx| window.pointer_up(released, MouseButton::Left, cx));
    assert!((moved - (title_in + 18)).abs() <= 1, "without snapping it stays where dragged ({moved})");
    visual.update(|window, cx| window.press("s", cx));
    visual.update(|_, cx| assert!(workspace.read(cx).state.snapping));
}

#[gpui_kit::test]
fn ok_on_unchanged_composition_settings_keeps_the_exact_length(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    // 123 frames at 120 fps is 1.025 s, shown as "1.02" (or "1.03"): not a whole number of frames.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let settings = studio_core::edit::CompositionSettings {
            frame_rate: studio_core::time::FrameRate::new(120, 1),
            duration_frames: 123,
            ..ws.state.composition_settings()
        };
        ws.state.update_composition(&settings).unwrap();
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.press("cmd-k", cx));
    render(visual);
    click_label(visual, "OK");
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.composition().duration_frames, 123));
}

// Tracks ---------------------------------------------------------------------------------------

fn tall_window(visual: &mut VisualTestContext) {
    // A tall window, so every track of the sample project is on screen without scrolling.
    visual.simulate_resize(size(px(1600.0), px(1500.0)));
    render(visual);
}

fn track_of(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, layer: Id) -> i32 {
    visual.update(|_, cx| workspace.read(cx).state.composition().layer(layer).unwrap().track)
}

/// The background solid, alone on the main track in the sample project.
fn main_clip(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> Id {
    visual.update(|_, cx| workspace.read(cx).state.composition().layers.iter().find(|l| l.track == studio_core::tracks::MAIN_TRACK).unwrap().id)
}

#[gpui_kit::test]
fn dragging_a_clip_onto_the_main_track_puts_two_clips_on_one_line(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let solid = main_clip(&workspace, visual);
    // Make room on the main track: the solid ends at frame 24, so the title (24–168) fits after it.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.magnetic_main_track = false;
        ws.state.composition_mut_for_test().layer_mut(solid).unwrap().out_frame = 24;
        cx.notify();
    }));
    tall_window(visual);
    let title = center_of(visual, ("clip", TITLE.0 as usize));
    let main = center_of(visual, ("clip", solid.0 as usize));
    // Straight down onto the main track's row, keeping the title's time.
    visual.update(|window, cx| window.drag(title, point(title.x, main.y), cx));
    render(visual);
    assert_eq!(track_of(&workspace, visual, TITLE), studio_core::tracks::MAIN_TRACK);
    assert_eq!(track_of(&workspace, visual, solid), studio_core::tracks::MAIN_TRACK, "both on one line");
    visual.update(|window, cx| window.press("cmd-z", cx));
    render(visual);
    assert_ne!(track_of(&workspace, visual, TITLE), studio_core::tracks::MAIN_TRACK, "one undo puts it back");
}

#[gpui_kit::test]
fn project_items_drop_onto_the_track_under_the_pointer(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let solid = main_clip(&workspace, visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.magnetic_main_track = false;
        ws.state.composition_mut_for_test().layer_mut(solid).unwrap().out_frame = 24;
        cx.notify();
    }));
    tall_window(visual);
    // Drop the gray solid item onto the main track, past the existing clip.
    let item = center_of(visual, ("project-item", studio_core::sample::ids::GRAY_SOLID.0 as usize));
    let main = center_of(visual, ("clip", solid.0 as usize));
    let target = point(main.x + px(200.0), main.y);
    visual.update(|window, cx| window.drag(item, target, cx));
    render(visual);
    let added = visual.update(|_, cx| workspace.read(cx).state.selected_layers[0]);
    assert_ne!(added, solid);
    assert_eq!(track_of(&workspace, visual, added), studio_core::tracks::MAIN_TRACK, "it landed on the main track");
}

#[gpui_kit::test]
fn clip_grips_trim_in_the_tracks_view(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    let (_, _, before) = layer_timing(&workspace, visual, TITLE);
    let grip = center_of(visual, ("clip-trim-out", TITLE.0 as usize));
    visual.update(|window, cx| window.drag(grip, point(grip.x - px(40.0), grip.y), cx));
    render(visual);
    let (_, _, after) = layer_timing(&workspace, visual, TITLE);
    assert!(after < before, "the out point moved earlier: {before} → {after}");
}

#[gpui_kit::test]
fn clip_header_toggles_and_the_magnet_work(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|window, cx| window.click("magnetic", cx));
    render(visual);
    visual.update(|_, cx| assert!(!workspace.read(cx).state.magnetic_main_track));
    let switches = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().switches);
    visual.update(|window, cx| window.click(("clip-show", TITLE.0 as usize), cx));
    render(visual);
    assert!(!switches(visual).visible, "the eye hides the clip");
    visual.update(|window, cx| window.click(("clip-lock", TITLE.0 as usize), cx));
    render(visual);
    assert!(switches(visual).locked, "the padlock locks it");
}

#[gpui_kit::test]
fn expanding_a_clip_shows_its_keyframes_which_drag_in_time(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|window, cx| window.click(("clip-expand", TITLE.0 as usize), cx));
    render(visual);
    // The title's opacity has keyframes at 24, 36, 156 and 168.
    let key = ElementId::Name(format!("clip-kf:{}:{OPACITY:?}:36", TITLE.0).into());
    let at = center_of(visual, key);
    let frames = 12.0 * pixels_per_frame(&workspace, visual);
    visual.update(|window, cx| window.drag(at, point(at.x + px(frames), at.y), cx));
    render(visual);
    let keys = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().property(OPACITY, 0).unwrap().keyframes);
    assert!(!keys.contains(&36) && keys.iter().any(|&k| (46..=50).contains(&k)), "the keyframe moved about 12 frames later: {keys:?}");
}

#[gpui_kit::test]
fn the_export_button_opens_the_export_dialog(cx: &mut TestAppContext) {
    let (_, visual) = open(cx);
    visual.update(|window, cx| window.click("rail-export", cx));
    render(visual);
    visual.update(|window, cx| assert!(window.has_active_dialog(cx)));
}

#[gpui_kit::test]
fn the_tracks_toolbar_plays_and_zooms(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|window, cx| window.click("tracks-play", cx));
    visual.update(|_, cx| assert!(workspace.read(cx).state.is_playing()));
    visual.update(|window, cx| window.click("tracks-play", cx));
    visual.update(|window, cx| window.click("tracks-zoom-in", cx));
    let zoomed = visual.update(|_, cx| workspace.read(cx).state.view_range.len());
    visual.update(|window, cx| window.click("tracks-zoom-fit", cx));
    let fit = visual.update(|_, cx| workspace.read(cx).state.view_range.len());
    assert!(zoomed < fit && fit == 240.0, "zoomed {zoomed}, fit {fit}");
}

// Files dragged in from the Finder ---------------------------------------------------------------

/// Drags `paths` from outside the window and drops them at `at`, as the Finder does.
fn drop_files(visual: &mut VisualTestContext, paths: Vec<std::path::PathBuf>, at: Point<Pixels>) {
    use gpui_kit::{ExternalPaths, FileDropEvent, PlatformInput};
    visual.update(|window, cx| {
        window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Entered { position: at, paths: ExternalPaths(paths.into()) }), cx);
        window.render_frame(cx);
        window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Pending { position: point(at.x + px(1.0), at.y) }), cx);
        window.render_frame(cx);
        window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Submit { position: point(at.x + px(1.0), at.y) }), cx);
    });
    // Dropped files are read in the background.
    visual.run_until_parked();
    render(visual);
}

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/studio-media/tests/fixtures").join(name)
}

#[gpui_kit::test]
fn files_dropped_on_the_project_panel_are_imported(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let before = visual.update(|_, cx| workspace.read(cx).state.project.items.len());
    let at = center_of(visual, "project-items");
    drop_files(visual, vec![fixture("tone.wav"), fixture("colors.mp4")], at);
    let items = visual.update(|_, cx| workspace.read(cx).state.project.items.iter().map(|item| item.name.clone()).collect::<Vec<_>>());
    assert_eq!(items.len(), before + 2, "both files were imported: {items:?}");
    assert!(items.iter().any(|name| name == "colors.mp4") && items.iter().any(|name| name == "tone.wav"));
}

#[gpui_kit::test]
fn files_dropped_on_a_track_become_clips_there_one_after_another(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    // Drop two copies' worth (the same clip twice) onto the title's track, past its end.
    let title_end = center_of(visual, ("clip-trim-out", TITLE.0 as usize));
    let at = point(title_end.x + px(20.0), title_end.y);
    drop_files(visual, vec![fixture("colors.mp4"), fixture("colors.mp4")], at);
    let clips: Vec<(i32, i64, i64)> = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        state.composition().layers.iter().filter(|layer| layer.name == "colors.mp4").map(|layer| (layer.track, layer.in_frame, layer.out_frame)).collect()
    });
    assert_eq!(clips.len(), 2, "two clips: {clips:?}");
    let (first, second) = if clips[0].1 < clips[1].1 { (clips[0], clips[1]) } else { (clips[1], clips[0]) };
    assert_eq!(second.1, first.2, "the second starts where the first ends");
    assert_eq!(first.0, second.0, "on the same track");
}

#[gpui_kit::test]
fn files_dropped_on_the_viewer_become_layers_at_the_playhead(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.set_current_frame(12); cx.notify(); }));
    render(visual);
    // A sound file: video under the playhead would start a decoder thread, which the headless
    // test scheduler refuses.
    let at = screen_point(&workspace, visual, Vec2::new(960.0, 540.0));
    drop_files(visual, vec![fixture("tone.wav")], at);
    let added = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        state.composition().layers.iter().filter(|layer| layer.name == "tone.wav").map(|layer| layer.in_frame).collect::<Vec<_>>()
    });
    assert_eq!(added, vec![12], "imported and added at the playhead");
}

#[gpui_kit::test]
fn project_items_dropped_on_the_viewer_land_where_they_are_let_go(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let item = center_of(visual, ("project-item", studio_core::sample::ids::GRAY_SOLID.0 as usize));
    let to = screen_point(&workspace, visual, Vec2::new(500.0, 400.0));
    visual.update(|window, cx| window.drag(item, to, cx));
    render(visual);
    let content = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        state.selected_layers.first().and_then(|id| state.composition().layer(*id)).map(|layer| layer.content.clone())
    });
    assert!(matches!(content, Some(studio_core::model::LayerContent::Solid { .. })), "the solid was added and selected: {content:?}");
}

#[gpui_kit::test]
fn dropping_a_folder_imports_the_media_inside_it(cx: &mut TestAppContext) {
    let folder = scratch_folder("drop-folder");
    std::fs::copy(fixture("tone.wav"), folder.join("a.wav")).unwrap();
    std::fs::copy(fixture("colors.mp4"), folder.join("b.mp4")).unwrap();
    std::fs::write(folder.join(".hidden"), "not media").unwrap();
    let (workspace, visual) = open(cx);
    let before = visual.update(|_, cx| workspace.read(cx).state.project.items.len());
    let at = center_of(visual, "project-items");
    drop_files(visual, vec![folder.clone()], at);
    let after = visual.update(|_, cx| workspace.read(cx).state.project.items.len());
    assert_eq!(after, before + 2);
    std::fs::remove_dir_all(&folder).unwrap();
}

// Remembering projects and sessions --------------------------------------------------------------

/// Saves the open project as `name` in `folder` through the Save dialog.
fn save_as(visual: &mut VisualTestContext, folder: &std::path::Path, name: &str) -> std::path::PathBuf {
    visual.update(|window, cx| window.press("cmd-shift-s", cx));
    visual.run_until_parked();
    let path = folder.join(name);
    visual.simulate_new_path_selection(|_| Some(path.clone()));
    visual.run_until_parked();
    render(visual);
    path
}

#[gpui_kit::test]
fn a_saved_project_reopens_with_the_editor_as_it_was(cx: &mut TestAppContext) {
    let folder = scratch_folder("remember");
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.ui.timeline.expanded_clips.insert(TITLE);
        ws.state.snapping = false;
        ws.state.set_view_range(studio_core::time::FrameRange::new(24.0, 120.0));
        cx.notify();
    }));
    let path = save_as(visual, &folder, "Remembered.studio");
    // Change everything, then open the file again.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.ui.timeline.expanded_clips.clear();
        ws.state.snapping = true;
        ws.state.set_view_range(studio_core::time::FrameRange::new(0.0, 240.0));
        cx.notify();
    }));
    visual.update(|window, cx| workspace.update(cx, |ws, cx| ws.open_file(path.clone(), window, cx)));
    render(visual);
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        assert!(ws.ui.timeline.expanded_clips.contains(&TITLE));
        assert!(!ws.state.snapping);
        assert_eq!(ws.state.view_range, studio_core::time::FrameRange::new(24.0, 120.0));
    });
    std::fs::remove_dir_all(&folder).unwrap();
}

#[gpui_kit::test]
fn launching_reopens_the_last_project(cx: &mut TestAppContext) {
    let folder = scratch_folder("last-project");
    let (workspace, visual) = open(cx);
    make_an_edit(&workspace, visual);
    let path = save_as(visual, &folder, "Last.studio");
    // A fresh start: the sample project, then the session is restored.
    visual.update(|window, cx| workspace.update(cx, |ws, cx| {
        ws.state = crate::state::EditorState::new();
        ws.restore_session(window, cx);
    }));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.file(), Some(path.as_path())));
    assert_eq!(title_value(&workspace, visual, ROTATION, 0), PropertyValue::Scalar(33.0), "the saved edit is back");
    std::fs::remove_dir_all(&folder).unwrap();
}

#[gpui_kit::test]
fn unsaved_work_is_recovered_after_studio_closes_without_saving(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    make_an_edit(&workspace, visual);
    visual.update(|_, cx| workspace.read(cx).keep_recovery_copy().unwrap());
    assert!(crate::app_settings::recovery_path().exists(), "unsaved work was copied");
    // As if Studio closed and started again.
    visual.update(|window, cx| workspace.update(cx, |ws, cx| {
        ws.state = crate::state::EditorState::new();
        ws.restore_session(window, cx);
    }));
    visual.run_until_parked();
    assert!(visual.has_pending_prompt(), "it offers the work back");
    visual.simulate_prompt_answer("Restore");
    visual.run_until_parked();
    render(visual);
    assert_eq!(title_value(&workspace, visual, ROTATION, 0), PropertyValue::Scalar(33.0));
    assert!(has_unsaved_changes(&workspace, visual), "restored work is still unsaved");
    visual.update(|_, cx| assert!(workspace.read(cx).state.file().is_none(), "it was untitled, and still is"));
}

#[gpui_kit::test]
fn undoing_back_to_the_saved_project_removes_the_recovery_copy(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    make_an_edit(&workspace, visual);
    visual.update(|_, cx| workspace.read(cx).keep_recovery_copy().unwrap());
    assert!(crate::app_settings::recovery_path().exists());
    visual.update(|_, cx| workspace.update(cx, |ws, _| assert!(ws.state.undo())));
    assert!(!has_unsaved_changes(&workspace, visual));
    visual.update(|_, cx| workspace.read(cx).keep_recovery_copy().unwrap());
    assert!(!crate::app_settings::recovery_path().exists(), "nothing to offer back on the next launch");
}

#[gpui_kit::test]
fn a_recovery_copy_that_cant_be_written_is_reported_once(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    make_an_edit(&workspace, visual);
    // A folder where the recovery file goes, so writing it fails.
    let blocker = crate::app_settings::recovery_path();
    let _ = std::fs::remove_file(&blocker);
    std::fs::create_dir_all(&blocker).unwrap();
    visual.update(|window, cx| workspace.update(cx, |ws, cx| ws.keep_work_safe(window, cx)));
    assert!(visual.update(|_, cx| workspace.read(cx).ui.recovery_failing), "the failure is noticed");
    let count = |visual: &mut VisualTestContext| visual.update(|window, cx| window.notifications(cx).len());
    let shown = count(visual);
    assert!(shown >= 1, "and the user is told");
    visual.update(|window, cx| workspace.update(cx, |ws, cx| ws.keep_work_safe(window, cx)));
    assert_eq!(count(visual), shown, "but only once");
    std::fs::remove_dir_all(&blocker).unwrap();
    visual.update(|window, cx| workspace.update(cx, |ws, cx| ws.keep_work_safe(window, cx)));
    assert!(!visual.update(|_, cx| workspace.read(cx).ui.recovery_failing), "and it recovers once writing works");
}

#[gpui_kit::test]
fn discarding_recovered_work_removes_the_copy(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    make_an_edit(&workspace, visual);
    visual.update(|_, cx| workspace.read(cx).keep_recovery_copy().unwrap());
    visual.update(|window, cx| workspace.update(cx, |ws, cx| ws.restore_session(window, cx)));
    visual.run_until_parked();
    visual.simulate_prompt_answer("Discard");
    visual.run_until_parked();
    assert!(!crate::app_settings::recovery_path().exists());
}

// Keyframes in the Tracks view ------------------------------------------------------------------

fn bounds_of(visual: &mut VisualTestContext, id: impl Into<ElementId>) -> gpui_kit::Bounds<Pixels> {
    let id = id.into();
    visual.update(|window, _| window.find(id).bounds())
}

/// The Tracks view with the title clip opened to its keyframe rows.
fn open_title_keyframes(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|window, cx| window.click(("clip-expand", TITLE.0 as usize), cx));
    render(visual);
    (workspace, visual)
}

#[gpui_kit::test]
fn keyframe_labels_never_sit_under_a_keyframe(cx: &mut TestAppContext) {
    let (_, visual) = open_title_keyframes(cx);
    // The title's first opacity keyframe is at its in point, right where the label starts.
    let label = bounds_of(visual, ElementId::Name(format!("clip-prop-name:{}:{OPACITY:?}", TITLE.0).into()));
    let key = bounds_of(visual, ElementId::Name(format!("clip-kf:{}:{OPACITY:?}:24", TITLE.0).into()));
    assert!(label.bottom() <= key.top() + px(2.0), "label {label:?} and keyframe {key:?} overlap");
}

#[gpui_kit::test]
fn pressing_a_property_row_selects_all_its_keyframes(cx: &mut TestAppContext) {
    let (workspace, visual) = open_title_keyframes(cx);
    let row = ElementId::Name(format!("clip-prop:{}:{OPACITY:?}", TITLE.0).into());
    // Press near the right end of the row, away from any keyframe.
    let bounds = bounds_of(visual, row.clone());
    visual.update(|window, cx| window.click_with_options(row, gpui_kit::test::ClickOptions::new().with_offset(point(bounds.size.width - px(30.0), px(6.0))), cx));
    render(visual);
    let selected: Vec<i64> = visual.update(|_, cx| workspace.read(cx).state.selected_keyframes.iter().filter(|key| key.path == OPACITY).map(|key| key.frame).collect());
    assert_eq!(selected.len(), 4, "every opacity keyframe: {selected:?}");
}

#[gpui_kit::test]
fn clicking_the_line_between_keyframes_opens_the_graph_editor_for_that_segment(cx: &mut TestAppContext) {
    let (workspace, visual) = open_title_keyframes(cx);
    // The opacity segment from frame 36 to 156.
    visual.update(|window, cx| window.click(ElementId::Name(format!("clip-seg:{}:{OPACITY:?}:36", TITLE.0).into()), cx));
    render(visual);
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        let mut frames: Vec<i64> = ws.state.selected_keyframes.iter().map(|key| key.frame).collect();
        frames.sort();
        assert_eq!(frames, vec![36, 156], "the segment's two keyframes");
        assert!(ws.ui.graph_editor.is_some(), "the graph editor opened");
    });
    // An easing chosen there applies to exactly that segment.
    visual.update(|window, cx| window.click(ElementId::Name("easing:Expo:Both".into()), cx));
    render(visual);
    let easings = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().keyframe_easings(OPACITY));
    assert!(matches!(easings[1].1, Easing::Curve(_)), "36 → 156 is eased: {easings:?}");
    assert_eq!(easings[0].1, Easing::Linear, "24 → 36 is untouched");
}

#[gpui_kit::test]
fn double_clicking_a_keyframe_opens_the_graph_editor(cx: &mut TestAppContext) {
    let (workspace, visual) = open_title_keyframes(cx);
    let key = ElementId::Name(format!("clip-kf:{}:{OPACITY:?}:36", TITLE.0).into());
    visual.update(|window, cx| window.click_with_options(key, gpui_kit::test::ClickOptions::new().with_count(2), cx));
    render(visual);
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        assert!(ws.ui.graph_editor.is_some());
        assert!(ws.state.selected_keyframes.iter().any(|key| key.frame == 36));
    });
}

// Effects panel ---------------------------------------------------------------------------------

#[gpui_kit::test]
fn dragging_an_effect_onto_a_clip_adds_it(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|window, cx| window.click("rail-effects", cx));
    render(visual);
    let from = center_of(visual, "effect:Gaussian Blur");
    let to = center_of(visual, ("clip", TITLE.0 as usize));
    visual.update(|window, cx| window.drag(from, to, cx));
    render(visual);
    let effects = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().effects.len());
    assert_eq!(effects, 1, "the blur was added to the title");
}

#[gpui_kit::test]
fn the_plus_button_adds_an_effect_to_the_selected_clip(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.click("rail-effects", cx));
    render(visual);
    // With nothing selected there is nothing to apply to, so there is no + button.
    assert!(visual.update(|window, _| window.try_find("effect-apply:Sharpen")).is_none());
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.select_layer(TITLE, false); cx.notify(); }));
    // Transitions are listed first; fold them away so Sharpen is in view.
    render(visual);
    visual.update(|window, cx| window.click("transitions-category", cx));
    render(visual);
    visual.update(|window, cx| window.click("effect-apply:Sharpen", cx));
    render(visual);
    let names = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().effects.iter().map(|e| e.name.clone()).collect::<Vec<_>>());
    assert_eq!(names, vec!["Sharpen"]);
}

#[gpui_kit::test]
fn only_effects_that_suit_the_selected_clip_are_listed(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.click("rail-effects", cx));
    let listed = |visual: &mut VisualTestContext, name: &str| visual.update(|window, _| window.try_find(ElementId::Name(format!("effect:{name}").into())).is_some());
    render(visual);
    assert!(listed(visual, "Exposure") && listed(visual, "Gaussian Blur"), "everything shows with nothing selected");
    assert!(listed(visual, "Stroke") && listed(visual, "Linear Color Key"), "effects the renderer now draws are listed");
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.select_layer(TITLE, false); cx.notify(); }));
    render(visual);
    assert!(!listed(visual, "Exposure"), "no colour correction for text");
    assert!(listed(visual, "Gaussian Blur"));
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.add_layer_from_item(studio_core::sample::ids::MUSIC, 0, 0);
        cx.notify();
    }));
    render(visual);
    assert!(!listed(visual, "Gaussian Blur"), "nothing for audio");
}

#[gpui_kit::test]
fn the_rail_switches_the_side_panel(cx: &mut TestAppContext) {
    let (_workspace, visual) = open(cx);
    visual.update(|window, cx| window.click("rail-effects", cx));
    render(visual);
    assert!(visual.update(|window, _| window.try_find("effect:Gaussian Blur")).is_some());
    visual.update(|window, cx| window.click("rail-media", cx));
    render(visual);
    assert!(visual.update(|window, _| window.try_find("effect:Gaussian Blur")).is_none(), "the Media panel replaced it");
}

// Shapes, Cut and the clipboard -----------------------------------------------------------------

#[gpui_kit::test]
fn holding_the_shape_button_opens_the_shape_menu(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let button = center_of(visual, "tool-shape");
    visual.simulate_mouse_down(button, gpui_kit::MouseButton::Left, gpui_kit::Modifiers::default());
    visual.executor().advance_clock(crate::panels::toolbar::HOLD_FOR_MENU * 2);
    visual.run_until_parked();
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).ui.shape_menu, "held long enough, the menu opens"));
    visual.simulate_mouse_up(button, gpui_kit::MouseButton::Left, gpui_kit::Modifiers::default());
    render(visual);
    visual.update(|window, cx| window.click(("shape-kind", 4usize), cx));
    render(visual);
    visual.update(|_, cx| {
        let ui = &workspace.read(cx).ui;
        assert_eq!((ui.tool, ui.shape_kind, ui.shape_menu), (crate::workspace::Tool::Shape, studio_core::model::ShapeKind::Star, false));
    });
}

#[gpui_kit::test]
fn a_quick_click_on_the_shape_button_only_picks_the_tool(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.click("tool-shape", cx));
    visual.executor().advance_clock(crate::panels::toolbar::HOLD_FOR_MENU * 2);
    visual.run_until_parked();
    render(visual);
    visual.update(|_, cx| {
        let ui = &workspace.read(cx).ui;
        assert_eq!(ui.tool, crate::workspace::Tool::Shape);
        assert!(!ui.shape_menu, "released before the hold time");
    });
}

#[gpui_kit::test]
fn dragging_in_the_viewer_with_the_shape_tool_draws_a_shape(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.press("q", cx));
    let from = screen_point(&workspace, visual, Vec2::new(400.0, 300.0));
    let to = screen_point(&workspace, visual, Vec2::new(800.0, 500.0));
    visual.update(|window, cx| window.drag(from, to, cx));
    render(visual);
    let (size, content) = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        let layer = state.composition().layer(state.selected_layers[0]).unwrap();
        (layer.size, layer.content.clone())
    });
    assert!(matches!(content, LayerContent::Shape { .. }));
    assert!((size.x - 400.0).abs() < 4.0 && (size.y - 200.0).abs() < 4.0, "drawn 400 × 200: {size:?}");
    // Its settings show in Properties: a rectangle has corners to round.
    assert!(visual.update(|window, _| window.try_find("shape-kind")).is_some());
}

#[gpui_kit::test]
fn the_cut_tool_splits_the_clicked_clip(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.snapping = false));
    visual.update(|window, cx| window.click("tool-group-edit", cx));
    render(visual);
    let count = layer_ids(&workspace, visual).len();
    visual.update(|window, cx| window.click(("clip", TITLE.0 as usize), cx));
    render(visual);
    assert_eq!(layer_ids(&workspace, visual).len(), count + 1, "one clip became two");
    let (_, _, out_frame) = layer_timing(&workspace, visual, TITLE);
    assert!(out_frame < 168, "the title now ends where it was clicked: {out_frame}");
}

#[gpui_kit::test]
fn cmd_c_cmd_v_and_cmd_d_copy_paste_and_duplicate(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        cx.notify();
    }));
    render(visual);
    let count = layer_ids(&workspace, visual).len();
    visual.update(|window, cx| window.press("cmd-d", cx));
    render(visual);
    assert_eq!(layer_ids(&workspace, visual).len(), count + 1, "duplicated");
    visual.update(|window, cx| window.press("cmd-c", cx));
    visual.update(|window, cx| window.press("cmd-v", cx));
    render(visual);
    assert_eq!(layer_ids(&workspace, visual).len(), count + 2, "pasted");
    visual.update(|window, cx| window.press("cmd-z", cx));
    render(visual);
    assert_eq!(layer_ids(&workspace, visual).len(), count + 1, "the paste undoes on its own");
}

#[gpui_kit::test]
fn an_expanded_folder_shows_as_open(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    // Folders start expanded; collapsing one keeps its row and hides its contents.
    let folder = studio_core::sample::ids::FOOTAGE_FOLDER;
    assert!(visual.update(|window, _| window.try_find(("project-item", studio_core::sample::ids::A_CAM.0 as usize))).is_some());
    visual.update(|window, cx| window.double_click(("project-item", folder.0 as usize), cx));
    render(visual);
    visual.update(|_, cx| assert!(!workspace.read(cx).ui.folder_open(folder)));
    assert!(visual.update(|window, _| window.try_find(("project-item", studio_core::sample::ids::A_CAM.0 as usize))).is_none());
}

#[gpui_kit::test]
fn the_viewer_bar_lines_up_with_the_project_footer(cx: &mut TestAppContext) {
    let (_workspace, visual) = open(cx);
    let bounds = |visual: &mut VisualTestContext, id: &'static str| visual.update(|window, _| window.find(id).bounds());
    let (folder, play) = (bounds(visual, "new-folder"), bounds(visual, "play"));
    assert!((folder.center().y - play.center().y).abs() < px(1.0), "same row: {folder:?} vs {play:?}");
    // The timeline has no separate title row: the name shares the row with its transport.
    let (name, start) = (bounds(visual, "timeline-comp-name"), bounds(visual, "tracks-start"));
    assert!((name.center().y - start.center().y).abs() < px(1.0), "name beside the controls: {name:?} vs {start:?}");
}

// Shape styling and the color wheel ------------------------------------------------------------

/// Adds a 300 px shape of `kind` and selects it, so Properties shows its settings (in a tall
/// window, so all of them are on screen).
fn add_shape(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, kind: studio_core::model::ShapeKind) -> Id {
    tall_window(visual);
    let layer = visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let layer = ws.state.begin_shape_draw(kind, Vec2::new(960.0, 540.0)).unwrap();
        ws.state.end_viewer_drag();
        cx.notify();
        layer
    }));
    render(visual);
    layer
}

fn shape_style(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, layer: Id) -> studio_core::model::ShapeStyle {
    visual.update(|_, cx| *workspace.read(cx).state.shape_of(layer).unwrap())
}

/// Clicks `id` at a point `fraction` of the way across and down it.
fn click_at_fraction(visual: &mut VisualTestContext, id: &'static str, fraction: (f32, f32)) {
    let bounds = visual.update(|window, _| window.find(id).bounds());
    let offset = point(bounds.size.width * fraction.0, bounds.size.height * fraction.1);
    visual.update(|window, cx| window.click_with_options(id, gpui_kit::test::ClickOptions::new().with_offset(offset), cx));
    render(visual);
}

#[gpui_kit::test]
fn the_color_picker_sets_hue_saturation_brightness_and_opacity(cx: &mut TestAppContext) {
    use studio_core::model::{Paint, ShapeKind};
    let (workspace, visual) = open(cx);
    let layer = add_shape(&workspace, visual, ShapeKind::Rectangle);
    let fill = |visual: &mut VisualTestContext| match shape_style(&workspace, visual, layer).fill {
        Some(Paint::Solid(color)) => color,
        other => panic!("expected a solid fill, got {other:?}"),
    };
    visual.update(|window, cx| window.click("shape-fill-color:0", cx));
    render(visual);
    // Hue all the way left is red; the square's top right is that hue at full strength.
    click_at_fraction(visual, "color-hue", (0.0, 0.5));
    click_at_fraction(visual, "color-field", (0.999, 0.001));
    let red = fill(visual);
    assert!(red.r > 0.95 && red.g < 0.05 && red.b < 0.05, "full red: {red:?}");
    // The square's bottom is black, whatever the hue.
    click_at_fraction(visual, "color-field", (0.5, 0.999));
    assert!(fill(visual).r < 0.02, "black at the bottom");
    // The opacity slider sets alpha.
    click_at_fraction(visual, "color-alpha", (0.5, 0.5));
    assert!((fill(visual).a - 0.5).abs() < 0.03, "half opacity: {}", fill(visual).a);
    // A color in the project's row applies as it is.
    let first = visual.update(|_, cx| workspace.read(cx).state.project_colors()[0]);
    visual.update(|window, cx| window.click(("color-swatch", 0usize), cx));
    render(visual);
    assert!(fill(visual).same_look(first));
}

#[gpui_kit::test]
fn dragging_in_the_color_picker_is_one_undo_step(cx: &mut TestAppContext) {
    use studio_core::model::ShapeKind;
    let (workspace, visual) = open(cx);
    let layer = add_shape(&workspace, visual, ShapeKind::Ellipse);
    let before = shape_style(&workspace, visual, layer).fill;
    visual.update(|window, cx| window.click("shape-fill-color:0", cx));
    render(visual);
    let field = visual.update(|window, _| window.find("color-field").bounds());
    visual.update(|window, cx| window.drag(field.center(), point(field.center().x + px(60.0), field.center().y + px(30.0)), cx));
    render(visual);
    assert_ne!(shape_style(&workspace, visual, layer).fill, before);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.undo();
        cx.notify();
    }));
    render(visual);
    assert_eq!(shape_style(&workspace, visual, layer).fill, before, "one undo puts the color back");
}

#[gpui_kit::test]
fn fills_become_gradients_and_strokes_turn_on(cx: &mut TestAppContext) {
    use studio_core::model::{Paint, ShapeKind};
    let (workspace, visual) = open(cx);
    tall_window(visual);
    let layer = add_shape(&workspace, visual, ShapeKind::Star);
    visual.update(|window, cx| window.click("shape-fill-type", cx));
    render(visual);
    click_label(visual, "Linear Gradient");
    render(visual);
    assert!(matches!(shape_style(&workspace, visual, layer).fill, Some(Paint::Linear { .. })));
    // A gradient shows a stop marker for each of its two colors.
    assert!(visual.update(|window, _| window.try_find("gradient-stop:fill:1")).is_some());
    visual.update(|window, cx| window.click("shape-fill-type", cx));
    render(visual);
    click_label(visual, "Radial Gradient");
    render(visual);
    assert!(matches!(shape_style(&workspace, visual, layer).fill, Some(Paint::Radial { .. })));

    visual.update(|window, cx| window.click("shape-stroke-on", cx));
    render(visual);
    let stroke = shape_style(&workspace, visual, layer).stroke.expect("the stroke is on");
    assert_eq!(stroke.width, 4.0);
    visual.update(|window, cx| window.click("shape-fill-on", cx));
    render(visual);
    assert_eq!(shape_style(&workspace, visual, layer).fill, None, "an outline only");
}

#[gpui_kit::test]
fn gradient_stops_are_added_moved_and_deleted_and_its_ends_drag_in_the_viewer(cx: &mut TestAppContext) {
    use crate::state::PaintSlot;
    use studio_core::model::ShapeKind;
    let (workspace, visual) = open(cx);
    tall_window(visual);
    let layer = add_shape(&workspace, visual, ShapeKind::Rectangle);
    visual.update(|window, cx| window.click("shape-fill-type", cx));
    render(visual);
    click_label(visual, "Linear Gradient");
    render(visual);
    let gradient = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.shape_gradient(layer, PaintSlot::Fill).unwrap());
    // A click in the middle of the bar adds a stop there.
    let bar = visual.update(|window, _| window.find("gradient-bar:fill").bounds());
    visual.update(|window, cx| window.click_at("gradient-bar:fill", point(bar.size.width / 2.0, px(6.0)), cx));
    render(visual);
    let stops = gradient(visual).stops().to_vec();
    assert_eq!(stops.len(), 3, "{stops:?}");
    assert!((stops[1].at - 0.5).abs() < 0.03, "added half way: {stops:?}");
    // Dragging its marker a quarter of the bar to the right moves it.
    let marker = center_of(visual, "gradient-stop:fill:1");
    visual.update(|window, cx| window.drag(marker, point(marker.x + bar.size.width / 4.0, marker.y), cx));
    render(visual);
    let moved = gradient(visual).stops()[1].at;
    assert!((moved - 0.75).abs() < 0.04, "moved to about 75%: {moved}");
    // Delete removes the picked stop.
    visual.update(|window, cx| window.click("gradient-delete:fill", cx));
    render(visual);
    assert_eq!(gradient(visual).stops().len(), 2);
    // The gradient's end handle drags in the viewer.
    let before = gradient(visual).to;
    let handle = center_of(visual, ElementId::Name(format!("gradient-to:{}", layer.0).into()));
    visual.update(|window, cx| window.drag(handle, point(handle.x - px(30.0), handle.y + px(20.0)), cx));
    render(visual);
    let after = gradient(visual).to;
    assert!(after.x < before.x && after.y > before.y, "the end moved left and down: {before:?} → {after:?}");
}

#[gpui_kit::test]
fn shapes_size_by_width_and_height_or_by_radius(cx: &mut TestAppContext) {
    use crate::state::ShapeSetting;
    use studio_core::model::ShapeKind;
    let (workspace, visual) = open(cx);
    let rectangle = add_shape(&workspace, visual, ShapeKind::Rectangle);
    // Rectangles show W and H; polygons and stars show radii instead.
    let has = |visual: &mut VisualTestContext, setting: ShapeSetting, layer: Id| {
        visual.update(|window, _| window.try_find(field_id(FieldTarget::Shape { layer, setting }))).is_some()
    };
    assert!(has(visual, ShapeSetting::Width, rectangle) && !has(visual, ShapeSetting::OuterRadius, rectangle));
    visual.update(|window, cx| window.click("shape-size-link", cx));
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.set_shape_setting(rectangle, ShapeSetting::Width, 600.0);
        cx.notify();
    }));
    render(visual);
    let size = visual.update(|_, cx| workspace.read(cx).state.composition().layer(rectangle).unwrap().size);
    assert_eq!(size, Vec2::new(600.0, 600.0), "linked, the height followed");

    let star = add_shape(&workspace, visual, ShapeKind::Star);
    assert!(has(visual, ShapeSetting::OuterRadius, star) && has(visual, ShapeSetting::InnerRadius, star) && !has(visual, ShapeSetting::Width, star));
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.set_shape_setting(star, ShapeSetting::OuterRadius, 200.0);
        ws.state.set_shape_setting(star, ShapeSetting::InnerRadius, 50.0);
        cx.notify();
    }));
    render(visual);
    let (size, inner) = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        (state.composition().layer(star).unwrap().size, state.shape_of(star).unwrap().inner_radius)
    });
    assert_eq!(size, Vec2::new(400.0, 400.0));
    assert_eq!(inner, 0.25, "an inner radius of 50 px on a 200 px star");
}

#[gpui_kit::test]
fn the_text_fill_chip_opens_the_color_picker(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click("fill-color", cx));
    render(visual);
    click_at_fraction(visual, "color-field", (0.0, 0.999));
    let fill = visual.update(|_, cx| workspace.read(cx).state.text_of(TITLE).unwrap().1.fill).unwrap();
    assert!(fill.r < 0.02 && fill.g < 0.02 && fill.b < 0.02, "black: {fill:?}");
}

// Libraries ------------------------------------------------------------------------------------

/// Opens the Libraries panel and makes a brand named `name` from its New Brand button.
fn make_brand(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, name: &str) -> studio_core::model::LinkId {
    visual.update(|window, cx| window.click("rail-libraries", cx));
    render(visual);
    visual.update(|window, cx| window.click("library-new-brand", cx));
    render(visual);
    // The new brand's name is ready to type over.
    visual.update(|window, cx| window.press("cmd-a", cx));
    visual.update(|window, cx| window.input(name, cx));
    visual.update(|window, cx| window.press("enter", cx));
    render(visual);
    visual.update(|_, cx| {
        let brands = &workspace.read(cx).state.libraries.brands;
        let brand = brands.last().expect("a brand was made");
        assert_eq!(brand.name, name);
        brand.id
    })
}

#[gpui_kit::test]
fn a_library_color_links_and_changing_it_updates_the_shape(cx: &mut TestAppContext) {
    use studio_core::model::{Paint, Rgba, ShapeKind};
    let (workspace, visual) = open(cx);
    let brand = make_brand(&workspace, visual, "Brand A");
    let layer = add_shape(&workspace, visual, ShapeKind::Ellipse);
    // + adds the selected shape's fill to the brand, and opens the picker on the new color.
    visual.update(|window, cx| window.click(ElementId::Name(format!("library-add-color:{}", brand.0).into()), cx));
    render(visual);
    let color = visual.update(|_, cx| workspace.read(cx).state.libraries.brand(brand).unwrap().colors[0].id);
    click_at_fraction(visual, "color-field", (0.999, 0.001));
    visual.update(|window, cx| window.click("color-picker-close", cx));
    render(visual);
    // Clicking the library color uses it on the shape, linked.
    visual.update(|window, cx| window.click(ElementId::Name(format!("library-color:{}", color.0).into()), cx));
    render(visual);
    let Some(Paint::Solid(fill)) = shape_style(&workspace, visual, layer).fill else { panic!() };
    assert_eq!(fill.link, Some(color));
    // Changing the library color changes the shape.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.set_library_color(color, Rgba::hex(0x00ff00));
        cx.notify();
    }));
    render(visual);
    let Some(Paint::Solid(fill)) = shape_style(&workspace, visual, layer).fill else { panic!() };
    assert!(fill.same_look(Rgba::hex(0x00ff00)) && fill.link == Some(color));
}

#[gpui_kit::test]
fn the_pickers_libraries_tab_links_a_color(cx: &mut TestAppContext) {
    use studio_core::model::{Paint, Rgba, ShapeKind};
    let (workspace, visual) = open(cx);
    tall_window(visual);
    let brand = make_brand(&workspace, visual, "Brand A");
    let primary = visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.add_library_color(brand, Rgba::hex(0x2d8ceb)).unwrap()));
    let layer = add_shape(&workspace, visual, ShapeKind::Star);
    visual.update(|window, cx| window.click("rail-media", cx));
    render(visual);
    visual.update(|window, cx| window.click("shape-fill-color:0", cx));
    render(visual);
    visual.update(|window, cx| window.click(("picker-tab", 1usize), cx));
    render(visual);
    visual.update(|window, cx| window.click(("library-swatch", 0usize), cx));
    render(visual);
    let Some(Paint::Solid(fill)) = shape_style(&workspace, visual, layer).fill else { panic!() };
    assert_eq!(fill.link, Some(primary));
    // The picker says where it comes from, and Detach stops following the library.
    visual.update(|window, cx| window.click(("picker-tab", 0usize), cx));
    render(visual);
    // Pressing Enter in the hex and opacity fields without changing them keeps the link.
    for field in [0, 1] {
        visual.update(|window, cx| {
            let picker = workspace.read(cx).ui.color_picker.as_ref().unwrap();
            let input = if field == 0 { picker.hex.clone() } else { picker.opacity.clone() };
            input.update(cx, |state, cx| state.focus(window, cx));
        });
        visual.update(|window, cx| window.press("enter", cx));
        render(visual);
        let Some(Paint::Solid(fill)) = shape_style(&workspace, visual, layer).fill else { panic!() };
        assert_eq!(fill.link, Some(primary), "field {field} left unchanged");
    }
    visual.update(|window, cx| window.click("color-unlink", cx));
    render(visual);
    let Some(Paint::Solid(fill)) = shape_style(&workspace, visual, layer).fill else { panic!() };
    assert_eq!(fill.link, None);
}

#[gpui_kit::test]
fn text_styles_save_from_a_layer_and_apply_to_another(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let brand = make_brand(&workspace, visual, "Brand A");
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        ws.state.set_text_setting(TITLE, crate::state::TextSetting::FontSize, 48.0);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click(ElementId::Name(format!("library-add-text:{}", brand.0).into()), cx));
    render(visual);
    let style = visual.update(|_, cx| workspace.read(cx).state.libraries.brand(brand).unwrap().text_styles[0].id);
    // A second text layer takes the style.
    let other = visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let other = ws.state.add_text_layer("Body", Vec2::new(200.0, 200.0)).unwrap();
        ws.state.select_layer(other, false);
        cx.notify();
        other
    }));
    render(visual);
    visual.update(|window, cx| window.click(ElementId::Name(format!("library-text:{}", style.0).into()), cx));
    render(visual);
    let applied = visual.update(|_, cx| workspace.read(cx).state.text_of(other).unwrap().1.clone());
    assert_eq!((applied.font_size, applied.template), (48.0, Some(style)));
}

#[gpui_kit::test]
fn library_items_and_whole_brands_delete(cx: &mut TestAppContext) {
    use studio_core::model::Rgba;
    let (workspace, visual) = open(cx);
    let brand = make_brand(&workspace, visual, "Brand A");
    let (one, two) = visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let one = ws.state.add_library_color(brand, Rgba::hex(0x111111)).unwrap();
        let two = ws.state.add_library_color(brand, Rgba::hex(0x222222)).unwrap();
        cx.notify();
        (one, two)
    }));
    render(visual);
    visual.update(|window, cx| window.click(ElementId::Name(format!("library-delete:{}", one.0).into()), cx));
    render(visual);
    let left: Vec<_> = visual.update(|_, cx| workspace.read(cx).state.libraries.brand(brand).unwrap().colors.iter().map(|c| c.id).collect());
    assert_eq!(left, vec![two], "just that color went");
    // Deleting the brand asks first, then takes everything in it.
    visual.update(|window, cx| window.click(ElementId::Name(format!("library-delete-brand:{}", brand.0).into()), cx));
    render(visual);
    click_label(visual, "Delete");
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).state.libraries.brands.is_empty()));
}

// Anchor points, effect visibility and the timeline toolbar -------------------------------------

fn anchor_of(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, layer: Id) -> Vec2 {
    visual.update(|_, cx| workspace.read(cx).state.composition().layer(layer).unwrap().transform.as_ref().unwrap().anchor_point.value_at(0.0))
}

#[gpui_kit::test]
fn the_anchor_drags_in_the_viewer_and_snaps_to_corners_unless_turned_off(cx: &mut TestAppContext) {
    use studio_core::model::ShapeKind;
    let (workspace, visual) = open(cx);
    let layer = add_shape(&workspace, visual, ShapeKind::Rectangle);
    let corners = visual.update(|_, cx| studio_core::transform::layer_corners(workspace.read(cx).state.composition(), layer, 0.0).unwrap());
    let handle = ElementId::Name(format!("anchor-handle:{}", layer.0).into());
    // Drop it a few pixels from the top left corner: it snaps onto the corner.
    let from = center_of(visual, handle.clone());
    let near_corner = screen_point(&workspace, visual, corners[0] + Vec2::new(6.0, 5.0));
    visual.update(|window, cx| window.drag(from, near_corner, cx));
    render(visual);
    assert_eq!(anchor_of(&workspace, visual, layer), Vec2::new(0.0, 0.0));
    let after = visual.update(|_, cx| studio_core::transform::layer_corners(workspace.read(cx).state.composition(), layer, 0.0).unwrap());
    for (a, b) in corners.iter().zip(after) {
        assert!((a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01, "the shape stays where it was: {corners:?} vs {after:?}");
    }
    // With snapping off, it lands where it is dropped.
    visual.update(|window, cx| window.click("viewer-snapping", cx));
    render(visual);
    let from = center_of(visual, handle);
    let inside = screen_point(&workspace, visual, corners[0] + Vec2::new(40.0, 30.0));
    visual.update(|window, cx| window.drag(from, inside, cx));
    render(visual);
    let anchor = anchor_of(&workspace, visual, layer);
    assert!((anchor.x - 40.0).abs() < 3.0 && (anchor.y - 30.0).abs() < 3.0, "not snapped: {anchor:?}");
}

#[gpui_kit::test]
fn the_anchor_grid_moves_the_anchor_to_a_corner(cx: &mut TestAppContext) {
    use studio_core::model::ShapeKind;
    let (workspace, visual) = open(cx);
    tall_window(visual);
    let layer = add_shape(&workspace, visual, ShapeKind::Ellipse);
    visual.update(|window, cx| window.click(("anchor-grid", 8usize), cx));
    render(visual);
    assert_eq!(anchor_of(&workspace, visual, layer), Vec2::new(300.0, 300.0), "the bottom right corner");
}

#[gpui_kit::test]
fn the_eye_turns_an_effect_off_and_on(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let effect = visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        ws.state.add_effect(TITLE, "Gaussian Blur");
        ws.ui.tabs.insert(crate::workspace::PanelId::Properties, 1);
        cx.notify();
        ws.state.composition().layer(TITLE).unwrap().effects[0].id
    }));
    render(visual);
    let enabled = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().effects[0].enabled);
    visual.update(|window, cx| window.click(ElementId::Name(format!("fx-on:{}", effect.0).into()), cx));
    render(visual);
    assert!(!enabled(visual));
    visual.update(|window, cx| window.click(ElementId::Name(format!("fx-on:{}", effect.0).into()), cx));
    render(visual);
    assert!(enabled(visual));
}

#[gpui_kit::test]
fn a_long_composition_name_does_not_move_the_timeline_transport(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let play = center_of(visual, "tracks-play");
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let settings = studio_core::edit::CompositionSettings { name: "A Much Longer Composition Name For The Client Review".into(), ..ws.state.composition_settings() };
        ws.state.update_composition(&settings).unwrap();
        cx.notify();
    }));
    render(visual);
    assert_eq!(center_of(visual, "tracks-play"), play);
}

// Anchor grid layout, text editing in the viewer, effect drops, panels and settings -------------

#[gpui_kit::test]
fn the_anchor_grid_is_three_by_three(cx: &mut TestAppContext) {
    use studio_core::model::ShapeKind;
    let (workspace, visual) = open(cx);
    add_shape(&workspace, visual, ShapeKind::Rectangle);
    let at = |visual: &mut VisualTestContext, index: usize| center_of(visual, ("anchor-grid", index));
    let (first, third, fourth) = (at(visual, 0), at(visual, 2), at(visual, 3));
    assert_eq!(first.y, third.y, "a row runs across");
    assert!(third.x > first.x);
    assert_eq!(first.x, fourth.x, "the next row starts under the first");
    assert!(fourth.y > first.y);
}

#[gpui_kit::test]
fn double_clicking_text_in_the_viewer_edits_it_there(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    // The title starts at frame 24.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.set_current_frame(40); cx.notify(); }));
    render(visual);
    // The title is centered in the frame.
    let at = screen_point(&workspace, visual, Vec2::new(960.0, 540.0));
    visual.update(|window, cx| window.click_with_options("viewer", gpui_kit::test::ClickOptions::new().with_count(2).with_offset(point(at.x - window.find("viewer").bounds().origin.x, at.y - window.find("viewer").bounds().origin.y)), cx));
    visual.run_until_parked();
    render(visual);
    let editing = visual.update(|_, cx| workspace.read(cx).ui.editing.as_ref().map(|editing| (editing.target, editing.in_viewer)));
    assert_eq!(editing, Some((crate::workspace::EditTarget::SourceText(TITLE), true)));
    visual.update(|window, cx| window.press("cmd-a", cx));
    visual.update(|window, cx| window.input("SALE", cx));
    visual.update(|window, cx| window.press("enter", cx));
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.text_of(TITLE).unwrap().0.to_owned()), "SALE");
}

#[gpui_kit::test]
fn an_effect_dropped_on_the_viewer_goes_on_the_layer_under_it(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    // The title starts at frame 24.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.set_current_frame(40); cx.notify(); }));
    render(visual);
    visual.update(|window, cx| window.click("rail-effects", cx));
    render(visual);
    let from = center_of(visual, "effect:Gaussian Blur");
    let to = screen_point(&workspace, visual, Vec2::new(960.0, 540.0));
    visual.update(|window, cx| window.drag(from, to, cx));
    render(visual);
    let names = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().effects.iter().map(|e| e.name.clone()).collect::<Vec<_>>());
    assert_eq!(names, vec!["Gaussian Blur"], "the title, on top in the middle, took it");
}

#[gpui_kit::test]
fn the_viewer_zooms_with_the_wheel_and_cmd_plus_minus(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let zoom = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).ui.viewer_zoom);
    assert_eq!(zoom(visual), None, "starts at Fit");
    // Scrolling up over the picture zooms in.
    visual.update(|window, cx| window.scroll("viewer", gpui_kit::ScrollDelta::Pixels(point(px(0.0), px(120.0))), cx));
    render(visual);
    let wheel = zoom(visual).expect("a set zoom after scrolling");
    visual.update(|window, cx| window.press("cmd-=", cx));
    render(visual);
    let stepped = zoom(visual).unwrap();
    assert!(stepped > wheel, "Cmd+= zooms in: {wheel} → {stepped}");
    visual.update(|window, cx| window.press("cmd--", cx));
    render(visual);
    assert!(zoom(visual).unwrap() < stepped, "Cmd+- zooms out");
}

#[gpui_kit::test]
fn cmd_brackets_step_one_frame_and_five_with_shift(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let frame = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.current_frame);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.set_current_frame(20); cx.notify(); }));
    render(visual);
    visual.update(|window, cx| window.press("cmd-]", cx));
    assert_eq!(frame(visual), 21);
    visual.update(|window, cx| window.press("cmd-[", cx));
    assert_eq!(frame(visual), 20);
    visual.update(|window, cx| window.press("cmd-shift-]", cx));
    assert_eq!(frame(visual), 25);
    visual.update(|window, cx| window.press("cmd-shift-[", cx));
    assert_eq!(frame(visual), 20);
}

#[gpui_kit::test]
fn right_column_panels_collapse_to_their_headers(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let align_top = center_of(visual, "collapse:Align").y;
    visual.update(|window, cx| window.click("collapse:Properties", cx));
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).ui.collapsed_panels.contains(&crate::workspace::PanelId::Properties)));
    assert!(center_of(visual, "collapse:Align").y < align_top, "Align moved up into the space");
    visual.update(|window, cx| window.click("collapse:Properties", cx));
    render(visual);
    assert_eq!(center_of(visual, "collapse:Align").y, align_top, "and back");
}

#[gpui_kit::test]
fn settings_change_the_theme_and_accent_and_clear_the_cache(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.press("cmd-,", cx));
    render(visual);
    // Appearance is the second page.
    visual.update(|window, cx| window.click(("settings-section", 1_usize), cx));
    render(visual);
    visual.update(|window, cx| window.click(("theme", crate::theme::ThemeName::Light as usize), cx));
    visual.update(|window, cx| window.click(("accent", crate::theme::Accent::Green as usize), cx));
    render(visual);
    assert_eq!(crate::theme::themed(crate::theme::color::ACCENT), crate::theme::Accent::Green.value());
    assert_ne!(crate::theme::themed(crate::theme::color::PANEL), crate::theme::color::PANEL, "light panels");
    let saved = crate::app_settings::load();
    assert_eq!((saved.theme, saved.accent), (crate::theme::ThemeName::Light, crate::theme::Accent::Green));
    // Clearing the (test) proxy folder removes the proxies in it, and nothing else.
    let folder = visual.update(|_, cx| workspace.read(cx).render.proxies.cache_folder().to_path_buf());
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("0123456789abcdef.mov"), vec![0u8; 1000]).unwrap();
    std::fs::write(folder.join("notes.txt"), "mine").unwrap();
    // The cache has its own section in the menu on the left.
    visual.update(|window, cx| window.click(("settings-section", 6_usize), cx));
    render(visual);
    visual.update(|window, cx| window.click("cache-clear", cx));
    render(visual);
    assert!(!folder.join("0123456789abcdef.mov").exists());
    assert!(folder.join("notes.txt").exists(), "a file that isn't a proxy stays");
    std::fs::remove_file(folder.join("notes.txt")).unwrap();
    visual.update(|_, cx| crate::theme::apply(crate::theme::ThemeName::Dark, crate::theme::Accent::Violet, cx));
}

// Captions -------------------------------------------------------------------------------------

/// Adds a caption layer for the title's time with three words, selected.
fn add_test_captions(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> Id {
    let words = ["make", "it", "pop."].iter().enumerate().map(|(i, text)| crate::state::HeardWord { text: (*text).into(), start: i as f64 * 0.5, end: i as f64 * 0.5 + 0.5 }).collect::<Vec<_>>();
    let layer = visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let layer = ws.state.add_captions(TITLE, &words).unwrap();
        cx.notify();
        layer
    }));
    tall_window(visual);
    layer
}

#[gpui_kit::test]
fn generating_captions_asks_before_downloading_the_model(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let clip = visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        // Sound only, so no picture is decoded in the background.
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/studio-media/tests/fixtures/tone.m4a");
        ws.state.import_files(vec![path]);
        let item = ws.state.selected_item.unwrap();
        ws.state.add_layer_from_item(item, 0, 0);
        let clip = ws.state.selected_layers[0];
        cx.notify();
        clip
    }));
    tall_window(visual);
    // Tests keep their settings (and so the model) in a folder of their own, where it is missing.
    assert!(!studio_captions::model_present(&crate::panels::captions::model_folder()));
    let before = layer_ids(&workspace, visual).len();
    visual.update(|window, cx| window.click("generate-captions", cx));
    render(visual);
    click_label(visual, "Cancel");
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).ui.caption_job.is_none(), "nothing started without saying yes"));
    assert_eq!(layer_ids(&workspace, visual).len(), before);
    let _ = clip;
}

#[gpui_kit::test]
fn caption_style_and_animation_apply_to_the_whole_layer(cx: &mut TestAppContext) {
    use studio_core::captions::CaptionAnimation;
    let (workspace, visual) = open(cx);
    let layer = add_test_captions(&workspace, visual);
    visual.update(|window, cx| window.click("captions-animation", cx));
    render(visual);
    click_label(visual, "Pop");
    render(visual);
    visual.update(|window, cx| window.click("captions-words", cx));
    render(visual);
    click_label(visual, "1 word");
    render(visual);
    visual.update(|window, cx| window.click("captions-uppercase", cx));
    render(visual);
    let style = visual.update(|_, cx| workspace.read(cx).state.caption_style(layer).unwrap().clone());
    assert_eq!((style.animation, style.words_per_phrase, style.uppercase), (CaptionAnimation::Pop, 1, false));
    // The font size field works on captions too.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.set_text_setting(layer, crate::state::TextSetting::FontSize, 96.0);
        cx.notify();
    }));
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.caption_style(layer).unwrap().text.font_size), 96.0);
}

#[gpui_kit::test]
fn a_phrase_in_the_transcript_can_be_corrected(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let layer = add_test_captions(&workspace, visual);
    visual.update(|window, cx| window.click_with_options(("caption-phrase", 0usize), gpui_kit::test::ClickOptions::new().with_count(2), cx));
    render(visual);
    visual.update(|window, cx| window.press("cmd-a", cx));
    visual.update(|window, cx| window.input("make it shine.", cx));
    visual.update(|window, cx| window.press("enter", cx));
    render(visual);
    let words: Vec<String> = visual.update(|_, cx| workspace.read(cx).state.caption_words(layer).unwrap().iter().map(|w| w.text.clone()).collect());
    assert_eq!(words, ["make", "it", "shine."]);
}

// Nested compositions ---------------------------------------------------------------------------

#[gpui_kit::test]
fn precomposing_then_double_clicking_the_clip_opens_the_new_composition(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.press("cmd-shift-c", cx));
    render(visual);
    // The Pre-compose dialog opens; OK makes the composition.
    visual.update(|window, cx| window.click("precompose-ok", cx));
    render(visual);
    let (main, layer) = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        (state.active_comp, state.selected_layers[0])
    });
    let inner = visual.update(|_, cx| workspace.read(cx).state.nested_composition(layer)).expect("the title is now inside a composition");
    visual.update(|window, cx| window.click_with_options(("clip", layer.0 as usize), gpui_kit::test::ClickOptions::new().with_count(2), cx));
    render(visual);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        assert_eq!(state.active_comp, inner, "the nested composition opened");
        assert_ne!(inner, main);
        assert!(state.composition().layer(TITLE).is_some(), "with the title in it");
    });
}

// Audio ----------------------------------------------------------------------------------------

/// Imports the colors clip (with sound) and places it at frame 120, away from the playhead
/// at 0 (so the test decodes no picture), selected.
fn add_video_with_sound(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> Id {
    let clip = visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/studio-media/tests/fixtures/colors.mp4");
        ws.state.import_files(vec![path]);
        let item = ws.state.selected_item.unwrap();
        ws.state.add_layer_from_item(item, 0, 120);
        let clip = ws.state.selected_layers[0];
        cx.notify();
        clip
    }));
    tall_window(visual);
    clip
}

#[gpui_kit::test]
fn separate_audio_moves_the_sound_to_its_own_clip(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let video = add_video_with_sound(&workspace, visual);
    // Taller still, so the Audio card's buttons are in view without scrolling the panel.
    visual.simulate_resize(size(px(1600.0), px(2000.0)));
    render(visual);
    visual.update(|window, cx| window.click("separate-audio", cx));
    render(visual);
    let (audio, video_plays) = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        (state.selected_layers[0], state.composition().layer(video).unwrap().switches.audio)
    });
    assert_ne!(audio, video);
    assert!(!video_plays, "the video went quiet");
    let content = visual.update(|_, cx| workspace.read(cx).state.composition().layer(audio).unwrap().content.clone());
    assert!(matches!(content, LayerContent::Audio { .. }));
}

#[gpui_kit::test]
fn video_and_sound_switch_separately_and_volume_and_fades_set(cx: &mut TestAppContext) {
    use crate::state::AudioSetting;
    let (workspace, visual) = open(cx);
    let video = add_video_with_sound(&workspace, visual);
    let switches = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(video).unwrap().switches);
    visual.update(|window, cx| window.click("layer-play-audio", cx));
    render(visual);
    assert!(switches(visual).visible && !switches(visual).audio, "picture on, sound off");
    visual.update(|window, cx| window.click("layer-play-audio", cx));
    visual.update(|window, cx| window.click("layer-show-video", cx));
    render(visual);
    assert!(!switches(visual).visible && switches(visual).audio, "picture off, sound on");
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.set_audio_setting(video, AudioSetting::Volume, -6.0);
        ws.state.set_audio_setting(video, AudioSetting::FadeIn, 0.5);
        ws.state.set_audio_setting(video, AudioSetting::FadeOut, 99.0);
        cx.notify();
    }));
    render(visual);
    let audio = visual.update(|_, cx| workspace.read(cx).state.composition().layer(video).unwrap().audio.clone());
    assert_eq!((audio.volume_db, audio.fade_in), (-6.0, 0.5));
    assert_eq!(audio.fade_out, 2.0, "no longer than the two-second clip");
    assert!((audio.gain() - 0.501).abs() < 0.01, "-6 dB is about half");
}

/// Clicks once at a screen point (a press and release, no movement).
fn click_at(visual: &mut VisualTestContext, at: Point<Pixels>) {
    visual.update(|window, cx| {
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
    });
    render(visual);
}

#[gpui_kit::test]
fn the_pen_tool_draws_a_closed_shape_whose_points_can_be_dragged(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.press("g", cx));
    render(visual);
    for corner in [(400.0, 300.0), (800.0, 300.0), (600.0, 700.0)] {
        let at = screen_point(&workspace, visual, Vec2::new(corner.0, corner.1));
        click_at(visual, at);
    }
    assert!(visual.update(|_, cx| workspace.read(cx).state.pen.as_ref().map(|pen| pen.points.len())) == Some(3), "three points so far");
    // Clicking the first point again closes the path into a shape layer.
    let first = screen_point(&workspace, visual, Vec2::new(401.0, 301.0));
    click_at(visual, first);
    let layer = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        assert!(state.pen.is_none());
        state.selected_layers[0]
    });
    let size = visual.update(|_, cx| workspace.read(cx).state.composition().layer(layer).unwrap().size);
    assert!((size.x - 400.0).abs() < 2.0 && (size.y - 400.0).abs() < 2.0, "boxed around the drawn points: {size:?}");
    // Properties offer opening and closing it, and the viewer shows its points.
    assert!(visual.update(|window, _| window.try_find("path-closed")).is_some());
    let point_id = ElementId::Name(format!("path-point:{}:2", layer.0).into());
    let from = center_of(visual, point_id);
    let to = screen_point(&workspace, visual, Vec2::new(600.0, 900.0));
    visual.update(|window, cx| window.drag(from, to, cx));
    render(visual);
    let size = visual.update(|_, cx| workspace.read(cx).state.composition().layer(layer).unwrap().size);
    assert!((size.y - 600.0).abs() < 4.0, "the bottom point moved down, stretching the box: {size:?}");
}

#[gpui_kit::test]
fn enter_ends_a_pen_path_as_a_line(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.click("tool-pen", cx));
    render(visual);
    for point in [(300.0, 500.0), (900.0, 500.0)] {
        let at = screen_point(&workspace, visual, Vec2::new(point.0, point.1));
        click_at(visual, at);
    }
    visual.update(|window, cx| window.press("enter", cx));
    render(visual);
    let content = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        state.composition().layer(state.selected_layers[0]).unwrap().content.clone()
    });
    let LayerContent::Shape { style, path: Some(path) } = content else { panic!("a path layer") };
    assert!(!path.closed && style.stroke.is_some() && style.fill.is_none(), "an open, stroked line");
}

#[gpui_kit::test]
fn a_masked_layer_shows_its_mask_and_how_it_masks(cx: &mut TestAppContext) {
    use studio_core::model::{LayerMask, MaskMode};
    let (workspace, visual) = open(cx);
    let mask = visual.update(|_, cx| workspace.read(cx).state.composition().layers[2].id);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        cx.notify();
    }));
    render(visual);
    assert!(visual.update(|window, _| window.try_find("layer-mask")).is_some(), "every picture layer can be masked");
    assert!(visual.update(|window, _| window.try_find("layer-mask-mode")).is_none(), "no mode until there is a mask");
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.set_layer_mask(TITLE, Some(LayerMask { layer: mask, mode: MaskMode::Hide }));
        cx.notify();
    }));
    render(visual);
    assert!(visual.update(|window, _| window.try_find("layer-mask-mode")).is_some());
}

#[gpui_kit::test]
fn picking_a_subject_places_clicks_on_the_picture_and_asks_before_downloading(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let layer = visual.update(|_, cx| {
        workspace.update(cx, |ws, cx| {
            let item = studio_core::edit::import_image(&mut ws.state.project, "/media/portrait.png".into(), Vec2::new(400.0, 200.0));
            let comp = ws.state.active_comp;
            let layer = studio_core::edit::add_layer_from_item(&mut ws.state.project, comp, item, 0, 0).unwrap();
            ws.state.select_layer(layer, false);
            cx.notify();
            layer
        })
    });
    tall_window(visual);
    visual.update(|window, cx| window.click("cutout-add", cx));
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).ui.cutout_pick), Some(layer));
    // A click on the subject (the image's top-left quarter). The model is not on this (test)
    // computer, so Studio asks before downloading it, and nothing runs until that is agreed.
    let keep = screen_point(&workspace, visual, Vec2::new(860.0, 490.0));
    click_at(visual, keep);
    assert!(visual.update(|window, _| window.try_find_by_label("Download and Continue")).is_some());
    assert!(visual.update(|_, cx| workspace.read(cx).ui.cutout_job.is_none()));
    click_label(visual, "Cancel");
    render(visual);
    // Then an Option-click on something to leave out.
    let leave_out = screen_point(&workspace, visual, Vec2::new(1060.0, 590.0));
    visual.update(|window, cx| {
        window.change_modifiers(gpui_kit::Modifiers::alt(), cx);
        window.pointer_down(leave_out, MouseButton::Left, cx);
        window.pointer_up(leave_out, MouseButton::Left, cx);
        window.change_modifiers(gpui_kit::Modifiers::default(), cx);
    });
    render(visual);
    let clicks = visual.update(|_, cx| workspace.read(cx).state.composition().layer(layer).unwrap().cutout.clone().unwrap().clicks);
    assert_eq!(clicks.iter().map(|click| click.keep).collect::<Vec<_>>(), vec![true, false]);
    assert!((clicks[0].at.x - 0.25).abs() < 0.01 && (clicks[0].at.y - 0.25).abs() < 0.01, "{clicks:?}");
}

#[gpui_kit::test]
fn a_tool_group_menu_picks_a_tool_that_its_button_then_shows(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let button = center_of(visual, "tool-group-edit");
    visual.update(|window, cx| window.pointer_down(button, MouseButton::Right, cx));
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).ui.tool_menu), Some(1), "right-click opens the group's menu");
    visual.update(|window, cx| window.click("tool-ripple", cx));
    render(visual);
    let tool = visual.update(|_, cx| workspace.read(cx).ui.tool);
    assert_eq!(tool, crate::workspace::Tool::Ripple);
    // The button now shows Ripple; picking another tool and clicking it again goes back to Ripple.
    visual.update(|window, cx| window.press("v", cx));
    visual.update(|window, cx| window.click("tool-group-edit", cx));
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).ui.tool), crate::workspace::Tool::Ripple);
}

#[gpui_kit::test]
fn the_zoom_tool_zooms_where_clicked_and_the_hand_pans(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.press("z", cx));
    render(visual);
    let target = Vec2::new(1400.0, 300.0);
    let at = screen_point(&workspace, visual, target);
    click_at(visual, at);
    let zoom = visual.update(|_, cx| workspace.read(cx).ui.viewer_zoom);
    assert!(zoom.is_some(), "zoomed in from Fit");
    // The clicked point stays under the pointer.
    let after = screen_point(&workspace, visual, target);
    assert!((f32::from(after.x) - f32::from(at.x)).abs() < 2.0 && (f32::from(after.y) - f32::from(at.y)).abs() < 2.0, "{at:?} → {after:?}");
    // The Hand drags the picture along.
    visual.update(|window, cx| window.press("h", cx));
    let to = point(at.x + px(40.0), at.y + px(10.0));
    visual.update(|window, cx| window.drag(at, to, cx));
    render(visual);
    let moved = screen_point(&workspace, visual, target);
    assert!((f32::from(moved.x) - f32::from(to.x)).abs() < 2.0, "{moved:?} follows the pointer to {to:?}");
}

#[gpui_kit::test]
fn the_eyedropper_picks_a_color_from_the_viewer(cx: &mut TestAppContext) {
    use studio_core::model::Rgba;
    let (workspace, visual) = open(cx);
    // A shape layer to recolor; open its fill's picker and turn on the eyedropper.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let layer = ws.state.begin_shape_draw(studio_core::model::ShapeKind::Rectangle, Vec2::new(300.0, 300.0)).unwrap();
        ws.state.end_viewer_drag();
        ws.state.select_layer(layer, false);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click("shape-fill-color:0", cx));
    render(visual);
    visual.update(|window, cx| window.click("picker-eyedropper", cx));
    render(visual);
    // The sample project's background solid is dark gray at the bottom right.
    let at = screen_point(&workspace, visual, Vec2::new(1800.0, 1000.0));
    click_at(visual, at);
    let (layer, sampling) = visual.update(|_, cx| {
        let ws = workspace.read(cx);
        (ws.state.selected_layers[0], ws.ui.color_picker.as_ref().map(|picker| picker.sampling))
    });
    assert_eq!(sampling, Some(false), "the eyedropper turns off after a pick, leaving the picker open");
    let fill = visual.update(|_, cx| workspace.read(cx).state.shape_paint(layer, crate::state::PaintSlot::Fill));
    assert_ne!(fill, Some(studio_core::model::Paint::Solid(Rgba::hex(0x8b5cf6))), "the fill took the sampled color: {fill:?}");
}

#[gpui_kit::test]
fn a_folders_arrow_closes_and_opens_it_with_one_click(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let folder = visual.update(|_, cx| workspace.read(cx).state.project.items.iter().find(|item| item.kind == studio_core::model::ProjectItemKind::Folder).unwrap().id);
    let shown = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).ui.folder_open(folder));
    assert!(shown(visual));
    visual.update(|window, cx| window.click(("folder-twirl", folder.0 as usize), cx));
    render(visual);
    assert!(!shown(visual), "one click closes it");
    visual.update(|window, cx| window.click(("folder-twirl", folder.0 as usize), cx));
    render(visual);
    assert!(shown(visual), "and opens it again");
}

#[gpui_kit::test]
fn settings_choose_the_project_panels_columns(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.press("cmd-,", cx));
    render(visual);
    visual.update(|window, cx| window.click(("settings-section", 4_usize), cx));
    render(visual);
    visual.update(|window, cx| window.click("column-type", cx));
    visual.update(|window, cx| window.click("column-duration", cx));
    render(visual);
    let columns = visual.update(|_, cx| workspace.read(cx).ui.project_columns);
    assert!(!columns.kind && columns.duration, "{columns:?}");
    assert_eq!(crate::app_settings::load().project_columns, columns, "remembered");
}

#[gpui_kit::test]
fn a_new_project_starts_with_the_default_folders_and_imports_are_sorted_into_them(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.new_project(&["Footage".to_owned(), "Music".to_owned()], crate::app_settings::NewComposition::default());
        ws.state.import_folders = crate::app_settings::ImportFolders { video: Some("Footage".into()), image: Some("Images".into()), audio: Some("Music".into()) };
        let tone = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/studio-media/tests/fixtures/tone.m4a");
        ws.state.import_files(vec![tone]);
        cx.notify();
    }));
    render(visual);
    visual.update(|_, cx| {
        let project = &workspace.read(cx).state.project;
        let music = studio_core::folders::top_folder(project, "Music").expect("a default folder");
        assert!(studio_core::folders::top_folder(project, "Footage").is_some());
        let tone = project.items.iter().find(|item| item.kind == studio_core::model::ProjectItemKind::Audio).unwrap();
        assert_eq!(tone.parent, Some(music), "sound goes into Music");
        assert_eq!(project.compositions.len(), 1);
    });
}

#[gpui_kit::test]
fn the_preview_plays_and_pauses_sound(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let tone = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/studio-media/tests/fixtures/tone.m4a");
        ws.state.import_files(vec![tone]);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click("project-preview", cx));
    render(visual);
    assert!(visual.update(|_, cx| workspace.read(cx).ui.preview.playing.is_some()), "clicking the preview plays it");
    visual.update(|window, cx| window.click("project-preview", cx));
    render(visual);
    assert!(visual.update(|_, cx| workspace.read(cx).ui.preview.playing.is_none()), "and again pauses it");
}

#[gpui_kit::test]
fn the_plus_on_a_device_picks_a_finish_then_adds_it_to_the_open_composition(cx: &mut TestAppContext) {
    use studio_core::mockups::Device;
    use studio_core::model::LayerContent;
    let (workspace, visual) = open(cx);
    tall_window(visual);
    // A picture in the project, selected. (A still, so no decoder thread runs in the test.)
    let clip = visual.update(|_, cx| workspace.update(cx, |ws, _| {
        let (items, _) = ws.state.import_paths(vec![fixture("still.png")]);
        items[0]
    }));
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.selected_item = Some(clip)));
    visual.update(|window, cx| window.click("rail-mockups", cx));
    render(visual);
    for index in 0..Device::ALL.len() {
        assert!(visual.update(|window, _| window.try_find(("mockup", index)).is_some()), "device {index} has a card");
    }
    let (comp, layers_before) = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        (state.active_comp, state.composition().layers.len())
    });
    // Clicking a card to look at it adds nothing.
    visual.update(|window, cx| window.click(("mockup", 0_usize), cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.composition().layers.len(), layers_before));
    // The iPhone's + asks for a finish first: nothing is added until one is picked.
    visual.update(|window, cx| window.click(("mockup-add", 0_usize), cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.composition().layers.len(), layers_before));
    click_label(visual, "Silver");
    render(visual);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        assert_eq!(state.active_comp, comp, "it stays in the open composition");
        let main = state.composition();
        assert_eq!(main.layers.len(), layers_before + 1, "as one layer");
        let added = main.layer(state.selected_layers[0]).unwrap();
        let LayerContent::Composition { composition: mockup } = added.content else { panic!("the mockup is a composition layer") };
        let position = added.transform.as_ref().unwrap().position.value;
        assert_eq!(position, Vec2::new(main.width as f32 * 0.5, main.height as f32 * 0.5), "centered");
        let mockup = state.project.composition(mockup).unwrap();
        assert!(mockup.name.starts_with("iPhone Mockup"), "added upright");
        let silver = studio_core::mockups::Finish::presets()[1].1.body;
        let body = mockup.layers.iter().find(|layer| layer.name == "Body").unwrap();
        assert!(matches!(&body.content, LayerContent::Shape { style, .. } if style.fill == Some(studio_core::model::Paint::Solid(silver))), "in the finish picked");
        let screen = mockup.layers.iter().find_map(|layer| match layer.content {
            LayerContent::Composition { composition } => Some(composition),
            _ => None,
        });
        let screen = state.project.composition(screen.unwrap()).unwrap();
        assert_eq!((screen.width, screen.height), (1320, 2868), "an upright iPhone screen");
        assert!(screen.layers.iter().any(|layer| layer.content == LayerContent::Image { item: clip }), "the selected picture is on the screen");
    });
}

#[gpui_kit::test]
fn view_all_shows_every_template_and_adds_the_one_clicked(cx: &mut TestAppContext) {
    use studio_core::templates::TEMPLATES;
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.click("rail-templates", cx));
    render(visual);
    visual.update(|window, cx| window.click("templates-view-all", cx));
    render(visual);
    assert!(visual.update(|window, _| window.try_find("template-gallery")).is_some(), "the gallery opens");
    // Every template has a card, the newest included.
    let last = TEMPLATES.len() - 1;
    assert!(visual.update(|window, _| window.try_find(("gallery-template", 0_usize)).is_some() && window.try_find(("gallery-template", last)).is_some()));
    // Its shape chips change the shape, as the panel's do.
    visual.update(|window, cx| window.click(("gallery-format", 0_usize), cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.template_format, studio_core::templates::Format::Wide));
    // Escape closes it; View All opens it again.
    visual.update(|window, cx| window.press("escape", cx));
    render(visual);
    assert!(visual.update(|window, _| window.try_find("template-gallery")).is_none(), "Escape closes it");
    visual.update(|window, cx| window.click("templates-view-all", cx));
    render(visual);
    // A category narrows it down; clicking a card adds that template and closes the gallery.
    let intros = studio_core::templates::Category::ALL.iter().position(|&category| category == studio_core::templates::Category::Intros).unwrap();
    visual.update(|window, cx| window.click(("gallery-category", intros), cx));
    render(visual);
    let first_intro = TEMPLATES.iter().position(|template| template.category == studio_core::templates::Category::Intros).unwrap();
    assert!(visual.update(|window, _| window.try_find(("gallery-template", 0_usize)).is_none()), "other categories are hidden");
    visual.update(|window, cx| window.click(("gallery-template", first_intro), cx));
    render(visual);
    assert!(visual.update(|window, _| window.try_find("template-gallery")).is_none(), "adding one closes it");
    visual.update(|_, cx| {
        let composition = workspace.read(cx).state.composition();
        assert!(composition.name.starts_with(TEMPLATES[first_intro].name));
        assert_eq!((composition.width, composition.height), (1920, 1080));
    });
}

#[gpui_kit::test]
fn a_template_click_adds_and_opens_it_in_the_chosen_shape(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.click("rail-templates", cx));
    render(visual);
    // 4:5, then the first template.
    visual.update(|window, cx| window.click(("template-format", 1_usize), cx));
    render(visual);
    visual.update(|window, cx| window.click(("template", 0_usize), cx));
    render(visual);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        let composition = state.composition();
        assert_eq!((composition.width, composition.height), (1080, 1350));
        assert!(composition.name.starts_with(studio_core::templates::TEMPLATES[0].name));
        assert_eq!(state.selected_item, Some(composition.id), "it is opened and selected");
    });
    // Its picture was made for the card.
    assert!(visual.update(|_, cx| workspace.read(cx).ui.template_pictures.contains_key(&(0, studio_core::templates::Format::Portrait))));
}

#[gpui_kit::test]
fn the_captions_panel_lists_clips_with_sound_and_restyles_captions(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let captions = add_test_captions(&workspace, visual);
    visual.update(|window, cx| window.click("rail-captions", cx));
    render(visual);
    // The sample project's score is a clip with sound.
    let has_clip = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        state.composition().layers.iter().find(|layer| state.caption_source(layer.id).is_some()).map(|layer| layer.id)
    });
    if let Some(clip) = has_clip {
        assert!(visual.update(|window, _| window.try_find(("caption-clip", clip.0 as usize))).is_some());
    }
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(captions, false);
        cx.notify();
    }));
    render(visual);
    // "One Word" is the third look.
    visual.update(|window, cx| window.click(("caption-look", 2_usize), cx));
    render(visual);
    let per_phrase = visual.update(|_, cx| workspace.read(cx).state.caption_style(captions).unwrap().words_per_phrase);
    assert_eq!(per_phrase, 1);
}

/// A clip of `tone.wav` (a second of silence, then a second of tone) at frame 0, selected.
fn add_tone_clip(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> Id {
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/studio-media/tests/fixtures/tone.wav");
        ws.state.import_files(vec![path]);
        let item = ws.state.selected_item.unwrap();
        ws.state.add_layer_from_item(item, 0, 0);
        let clip = ws.state.selected_layers[0];
        cx.notify();
        clip
    }))
}

/// Lets a background analysis finish: its thread works in real time, and the editor checks on
/// it with a timer.
fn wait_for_analysis(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    for _ in 0..200 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        visual.executor().advance_clock(std::time::Duration::from_millis(200));
        visual.run_until_parked();
        if visual.update(|_, cx| workspace.read(cx).ui.analysis_job.is_none()) {
            return;
        }
    }
    panic!("the analysis did not finish");
}

#[gpui_kit::test]
fn remove_silences_cuts_the_quiet_start_of_a_clip(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    add_tone_clip(&workspace, visual);
    tall_window(visual);
    visual.update(|window, cx| window.click("remove-silences", cx));
    wait_for_analysis(&workspace, visual);
    // The silent first second is gone: what is left starts on the tone, at the clip's old start.
    let (in_frame, source_seconds) = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        let composition = state.composition();
        // The clip's sound is the only audio layer from that file.
        let piece = composition.layers.iter().filter(|layer| matches!(layer.content, LayerContent::Audio { .. })).min_by_key(|layer| layer.in_frame).expect("what is left of the clip");
        (piece.in_frame, (piece.in_frame - piece.start_frame) as f64 / composition.frame_rate.fps())
    });
    assert_eq!(in_frame, 0);
    assert!((0.8..1.0).contains(&source_seconds), "starts just before the tone: {source_seconds} s into the file");
}

#[gpui_kit::test]
fn speed_and_reverse_change_a_footage_clip(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let clip = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        state.composition().layers.iter().find(|layer| matches!(layer.content, LayerContent::Footage { .. })).unwrap().id
    });
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(clip, false);
        cx.notify();
    }));
    tall_window(visual);
    let length = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(clip).map(|layer| layer.out_frame - layer.in_frame).unwrap());
    let before = length(visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let target = crate::panels::fields::FieldTarget::Speed { layer: clip };
        target.write(&mut ws.state, 200.0);
        cx.notify();
    }));
    render(visual);
    assert_eq!(length(visual), (before + 1) / 2, "twice as fast, half as long");
    visual.update(|window, cx| window.click("speed-reverse", cx));
    render(visual);
    assert!(visual.update(|_, cx| workspace.read(cx).state.composition().layer(clip).unwrap().time.reverse));
    // Track Motion is offered for graphics, not for the footage itself.
    assert!(visual.update(|window, _| window.try_find("track-motion-add")).is_none());
}

#[gpui_kit::test]
fn a_speed_ramp_comes_off_with_one_click(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let clip = first_footage(&workspace, visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(clip, false);
        ws.state.set_speed(clip, 50.0);
        cx.notify();
    }));
    tall_window(visual);
    let before = visual.update(|_, cx| workspace.read(cx).state.composition().layer(clip).unwrap().clone());
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.apply_speed_ramp(clip, studio_core::speed::Ramp::Punch);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click("speed-unramp", cx));
    render(visual);
    let after = visual.update(|_, cx| workspace.read(cx).state.composition().layer(clip).unwrap().clone());
    assert_eq!(after, before, "back to a steady 50%, with the same length");
    // The Ramp menu's No Ramp does the same.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.apply_speed_ramp(clip, studio_core::speed::Ramp::SlowMiddle);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click("speed-ramp", cx));
    render(visual);
    click_label(visual, "No Ramp");
    render(visual);
    let after = visual.update(|_, cx| workspace.read(cx).state.composition().layer(clip).unwrap().clone());
    assert_eq!(after, before);
}

#[gpui_kit::test]
fn section_headings_carry_info_and_reset_without_folding(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let clip = first_footage(&workspace, visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(clip, false);
        ws.state.set_speed(clip, 50.0);
        ws.state.set_reverse(clip, true);
        cx.notify();
    }));
    tall_window(visual);
    // The explainer is an info button now, not a paragraph.
    assert!(visual.update(|window, _| window.try_find("section-speed-info")).is_some());
    let open = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).ui.section_open("speed"));
    let was_open = open(visual);
    visual.update(|window, cx| window.click("reset-speed", cx));
    render(visual);
    let time = visual.update(|_, cx| workspace.read(cx).state.composition().layer(clip).unwrap().time.clone());
    assert!(time.is_normal(), "a steady 100%, forwards: {time:?}");
    assert_eq!(open(visual), was_open, "the heading's button doesn't fold the section");
}

#[gpui_kit::test]
fn animate_and_track_motion_headings_add_and_reset(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        cx.notify();
    }));
    tall_window(visual);
    let animation = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().animation);
    // + opens In, Out and Loop submenus.
    visual.update(|window, cx| window.click("animate-add", cx));
    render(visual);
    click_label(visual, "Loop");
    render(visual);
    click_label(visual, "Wiggle");
    render(visual);
    assert_eq!(animation(visual).looping, Some(studio_core::presets::Loop::Wiggle));
    visual.update(|window, cx| window.click("animate-reset", cx));
    render(visual);
    let cleared = animation(visual);
    assert!(cleared.enter.is_none() && cleared.exit.is_none() && cleared.looping.is_none(), "every animation is off: {cleared:?}");
    // Track Motion's + starts picking the point to follow.
    visual.update(|window, cx| window.click("track-motion-add", cx));
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).ui.track_pick), Some(TITLE));
}

#[gpui_kit::test]
fn a_clips_right_click_menu_flips_resets_and_duplicates(cx: &mut TestAppContext) {
    use studio_core::property::{PropertyPath, PropertyValue, TransformProperty};
    let (workspace, visual) = open(cx);
    tall_window(visual);
    let scale = |visual: &mut VisualTestContext| visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        state.composition().layer(TITLE).unwrap().property(PropertyPath::Transform(TransformProperty::Scale), state.current_frame).unwrap().value
    });
    let PropertyValue::Pair(before) = scale(visual) else { panic!() };
    visual.update(|window, cx| window.right_click(("clip-body", TITLE.0 as usize), cx));
    render(visual);
    click_label(visual, "Flip Horizontal");
    render(visual);
    assert_eq!(scale(visual), PropertyValue::Pair(studio_core::animation::Vec2::new(-before.x, before.y)), "mirrored left to right");
    visual.update(|window, cx| window.right_click(("clip-body", TITLE.0 as usize), cx));
    render(visual);
    click_label(visual, "Reset Transform");
    render(visual);
    assert_eq!(scale(visual), PropertyValue::Pair(studio_core::animation::Vec2::new(100.0, 100.0)), "back to 100%");
    let count = visual.update(|_, cx| workspace.read(cx).state.composition().layers.len());
    visual.update(|window, cx| window.right_click(("clip-body", TITLE.0 as usize), cx));
    render(visual);
    click_label(visual, "Duplicate");
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.composition().layers.len()), count + 1);
}

#[gpui_kit::test]
fn the_viewers_right_click_menu_acts_on_the_layer_under_the_pointer(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    // The title is on top in the middle at frame 40.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.set_current_frame(40); ws.state.clear_selection(); cx.notify(); }));
    render(visual);
    let at = screen_point(&workspace, visual, studio_core::animation::Vec2::new(960.0, 540.0));
    let viewer = visual.update(|window, _| window.find("viewer").bounds());
    visual.update(|window, cx| window.click_with_options("viewer", gpui_kit::test::ClickOptions::new().with_offset(at - viewer.origin).with_button(MouseButton::Right), cx));
    render(visual);
    click_label(visual, "Pre-compose…");
    render(visual);
    visual.update(|window, cx| window.click("precompose-ok", cx));
    render(visual);
    let layer = visual.update(|_, cx| workspace.read(cx).state.selected_layers[0]);
    let inner = visual.update(|_, cx| workspace.read(cx).state.nested_composition(layer)).expect("the title went into a new composition");
    // And Ungroup brings it back, keeping the composition in the project.
    visual.update(|window, cx| window.click_with_options("viewer", gpui_kit::test::ClickOptions::new().with_offset(at - viewer.origin).with_button(MouseButton::Right), cx));
    render(visual);
    click_label(visual, "Ungroup Composition");
    render(visual);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        assert!(state.composition().layers.iter().any(|layer| layer.name == "Title"), "the title is back: {:?}", state.composition().layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>());
        assert!(state.project.item(inner).is_some(), "the composition stays in the Project panel");
    });
}

#[gpui_kit::test]
fn holding_the_middle_button_pans_the_viewer(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let middle = center_of(visual, "viewer");
    visual.update(|window, cx| {
        window.pointer_move(middle, None, cx);
        window.pointer_down(middle, MouseButton::Middle, cx);
        window.pointer_move(point(middle.x + px(40.0), middle.y + px(25.0)), Some(MouseButton::Middle), cx);
        window.pointer_up(point(middle.x + px(40.0), middle.y + px(25.0)), MouseButton::Middle, cx);
    });
    render(visual);
    let pan = visual.update(|_, cx| workspace.read(cx).ui.viewer_pan);
    assert_eq!(pan, (40.0, 25.0), "the picture moved with the pointer");
    assert!(visual.update(|_, cx| workspace.read(cx).ui.middle_pan.is_none()), "letting go ends the pan");
}

#[gpui_kit::test]
fn track_motion_waits_for_a_click_on_a_video(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        cx.notify();
    }));
    tall_window(visual);
    visual.update(|window, cx| window.click("track-motion-add", cx));
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).ui.track_pick), Some(TITLE));
    visual.update(|window, cx| window.click("track-motion-add", cx));
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).ui.track_pick), None, "and can be called off");
}

#[gpui_kit::test]
fn settings_switches_change_what_the_panels_show(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.press("cmd-,", cx));
    render(visual);
    // Timeline & Viewer is the fourth page.
    visual.update(|window, cx| window.click(("settings-section", 3_usize), cx));
    render(visual);
    visual.update(|window, cx| window.click("timeline-render-time", cx));
    render(visual);
    assert!(!visual.update(|_, cx| workspace.read(cx).ui.settings.show_render_time));
    assert!(!crate::app_settings::load().show_render_time, "remembered");
    // General: new compositions take the chosen frame rate.
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.ui.settings.new_composition.fps = 60));
    let made = crate::state::new_composition_settings(visual.update(|_, cx| workspace.read(cx).ui.settings.new_composition));
    assert_eq!(made.frame_rate, studio_core::time::FrameRate::new(60, 1));
}

#[gpui_kit::test]
fn the_animate_section_shows_for_a_layer_and_presets_land_on_it(cx: &mut TestAppContext) {
    use studio_core::presets::{Loop, Motion, Phase};
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        cx.notify();
    }));
    tall_window(visual);
    assert!(visual.update(|window, _| window.try_find("animate-in")).is_some(), "In, Out and Loop menus are offered");
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.set_layer_motion(TITLE, Phase::In, Some(Motion::Pop), 0.5);
        ws.state.set_layer_loop(TITLE, Some(Loop::Float));
        cx.notify();
    }));
    render(visual);
    let animation = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().animation);
    assert_eq!(animation.enter.map(|(motion, _)| motion), Some(Motion::Pop));
    assert_eq!(animation.looping, Some(Loop::Float));
    // The length menu appears once an In motion is chosen.
    assert!(visual.update(|window, _| window.try_find("animate-in-length")).is_some());
}

#[gpui_kit::test]
fn a_transitions_plus_puts_it_on_the_selected_clip(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        cx.notify();
    }));
    visual.update(|window, cx| window.click("rail-effects", cx));
    render(visual);
    // Cross Dissolve is the first; the title has nothing touching it, so it fades in.
    visual.update(|window, cx| window.click(("transition-apply", 0_usize), cx));
    render(visual);
    let enter = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().animation.enter);
    assert_eq!(enter.map(|(motion, _)| motion), Some(studio_core::presets::Motion::Fade));
}

#[gpui_kit::test]
fn an_element_click_adds_it_selected_in_the_middle(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let before = visual.update(|_, cx| workspace.read(cx).state.composition().layers.len());
    visual.update(|window, cx| window.click("rail-elements", cx));
    render(visual);
    visual.update(|window, cx| window.click(("element", 0_usize), cx));
    render(visual);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        let composition = state.composition();
        assert!(composition.layers.len() > before);
        let added = state.selected_layers[0];
        let bounds = studio_core::align::layer_bounds(composition, added, state.current_frame).unwrap();
        let middle = (bounds.min + bounds.max) * 0.5;
        assert!((middle.x - composition.width as f32 * 0.5).abs() < 2.0 && (middle.y - composition.height as f32 * 0.5).abs() < 2.0, "{bounds:?}");
    });
}

#[gpui_kit::test]
fn magic_resize_makes_the_ticked_sizes_and_opens_the_last(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let (source, count) = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        (state.composition().id, state.project.compositions.len())
    });
    visual.update(|window, cx| window.dispatch_action(Box::new(crate::workspace::MagicResize), cx));
    render(visual);
    // The three social shapes start ticked; untick 4:5.
    visual.update(|window, cx| window.click(("resize-size", 1_usize), cx));
    render(visual);
    visual.update(|window, cx| window.click("resize-ok", cx));
    render(visual);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        assert_eq!(state.project.compositions.len(), count + 2);
        let open = state.composition();
        assert_ne!(open.id, source);
        assert_eq!((open.width, open.height), (1080, 1080));
        assert!(open.name.ends_with("1:1"), "{}", open.name);
        assert!(state.project.compositions.iter().any(|comp| (comp.width, comp.height) == (1080, 1920)));
    });
    assert_eq!(crate::panels::resize::short_label("Vertical 9:16 · Reels, TikTok, Shorts"), "9:16");
}

#[gpui_kit::test]
fn the_quality_menu_beside_fit_sets_playback_quality(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.click("viewer-quality", cx));
    render(visual);
    click_label(visual, "Quarter");
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).ui.settings.playback_quality), crate::app_settings::PlaybackQuality::Quarter);
    assert_eq!(crate::app_settings::load().playback_quality, crate::app_settings::PlaybackQuality::Quarter, "remembered");
}

#[gpui_kit::test]
fn properties_sections_and_project_folders_start_closed(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.ui.sections_start_open = false;
        ws.state.select_layer(TITLE, false);
        cx.notify();
    }));
    render(visual);
    // Folded: the headings show, their controls don't.
    assert!(visual.update(|window, _| window.try_find("section-transform")).is_some());
    assert!(visual.update(|window, _| window.try_find("scale-link")).is_none());
    visual.update(|window, cx| window.click("section-transform", cx));
    render(visual);
    assert!(visual.update(|window, _| window.try_find("scale-link")).is_some(), "a click opens it");
    // Folders start closed too, so the sample's footage is hidden in its folder.
    let footage = studio_core::sample::ids::A_CAM;
    assert!(visual.update(|window, _| window.try_find(("project-item", footage.0 as usize))).is_none());
    assert!(!visual.update(|_, cx| workspace.read(cx).ui.folder_open(studio_core::sample::ids::FOOTAGE_FOLDER)));
}

#[gpui_kit::test]
fn new_compositions_are_black_and_projects_have_an_exports_folder(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.new_project(&["Footage".to_owned()], crate::app_settings::NewComposition::default())));
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        assert_eq!(state.composition().background, studio_core::model::Rgba::hex(0x000000));
        assert!(studio_core::folders::top_folder(&state.project, studio_core::folders::EXPORTS_FOLDER).is_some());
    });
}

#[gpui_kit::test]
fn double_clicking_the_shape_tool_adds_a_full_frame_background(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.double_click("tool-shape", cx));
    render(visual);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        let composition = state.composition();
        let background = state.selected_layers[0];
        assert_eq!(composition.layer(background).unwrap().name, "Background");
        let bounds = studio_core::align::layer_bounds(composition, background, 0).unwrap();
        assert_eq!((bounds.max.x - bounds.min.x, bounds.max.y - bounds.min.y), (composition.width as f32, composition.height as f32));
        let bottom = composition.layers.iter().rev().find(|layer| layer.content.has_picture()).unwrap();
        assert_eq!(bottom.id, background, "behind everything");
        assert!(!workspace.read(cx).ui.shape_menu, "the shape menu stays shut");
    });
}

#[gpui_kit::test]
fn renaming_a_clip_from_its_right_click_menu(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.right_click(("clip-body", TITLE.0 as usize), cx));
    render(visual);
    click_label(visual, "Rename…");
    render(visual);
    visual.update(|window, cx| {
        window.input("Headline", cx);
        window.press("enter", cx);
    });
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().name.clone()), "Headline");
}

#[gpui_kit::test]
fn project_items_rename_and_take_labels_from_their_right_click_menu(cx: &mut TestAppContext) {
    use studio_core::model::LabelColor;
    use studio_core::sample::ids;
    let (workspace, visual) = open(cx);
    let item = |visual: &mut VisualTestContext, id| visual.update(|_, cx| workspace.read(cx).state.project.item(id).cloned().unwrap());
    // Rename the main composition: its composition is renamed too.
    visual.update(|window, cx| window.right_click(("project-item", ids::MAIN_COMP.0 as usize), cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.selected_item, Some(ids::MAIN_COMP), "right-clicking selects the item"));
    click_label(visual, "Rename…");
    render(visual);
    visual.update(|window, cx| {
        window.input("Intro", cx);
        window.press("enter", cx);
    });
    render(visual);
    assert_eq!(item(visual, ids::MAIN_COMP).name, "Intro");
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.project.composition(ids::MAIN_COMP).unwrap().name, "Intro"));
    // Give the gray solid a blue label.
    visual.update(|window, cx| window.right_click(("project-item", ids::GRAY_SOLID.0 as usize), cx));
    render(visual);
    click_label(visual, "Label");
    render(visual);
    click_label(visual, "Blue");
    render(visual);
    assert_eq!(item(visual, ids::GRAY_SOLID).label, LabelColor::Blue);
    // Delete from the menu asks first, as the Delete key does.
    visual.update(|window, cx| window.right_click(("project-item", ids::GRAY_SOLID.0 as usize), cx));
    render(visual);
    click_label(visual, "Delete");
    render(visual);
    visual.update(|window, cx| assert!(window.has_active_dialog(cx), "a confirmation opened"));
}

#[gpui_kit::test]
fn double_clicking_a_name_renames_it(cx: &mut TestAppContext) {
    use studio_core::sample::ids;
    let (workspace, visual) = open(cx);
    tall_window(visual);
    // A Project item's name.
    visual.update(|window, cx| window.click_with_options(("project-item-name", ids::LOGO.0 as usize), gpui_kit::test::ClickOptions::new().with_count(2), cx));
    render(visual);
    visual.update(|window, cx| {
        window.input("brand", cx);
        window.press("enter", cx);
    });
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.project.item(ids::LOGO).unwrap().name.clone()), "brand");
    // A clip's name in the timeline.
    visual.update(|window, cx| window.click_with_options(("clip-name", TITLE.0 as usize), gpui_kit::test::ClickOptions::new().with_count(2), cx));
    render(visual);
    visual.update(|window, cx| {
        window.input("Headline", cx);
        window.press("enter", cx);
    });
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().name.clone()), "Headline");
}

#[gpui_kit::test]
fn cmd_d_in_the_project_panel_duplicates_the_selected_item(cx: &mut TestAppContext) {
    use studio_core::sample::ids;
    let (workspace, visual) = open(cx);
    let count = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.project.items.len());
    let layers = visual.update(|_, cx| workspace.read(cx).state.composition().layers.len());
    let before = count(visual);
    visual.update(|window, cx| window.click(("project-item", ids::MAIN_COMP.0 as usize), cx));
    render(visual);
    visual.update(|window, cx| window.press("cmd-d", cx));
    render(visual);
    assert_eq!(count(visual), before + 1, "the composition was duplicated");
    let selected = visual.update(|_, cx| workspace.read(cx).state.selected_item).unwrap();
    assert_ne!(selected, ids::MAIN_COMP, "the copy is selected");
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.composition().layers.len()), layers, "no layers were duplicated");
}

#[gpui_kit::test]
fn videos_take_a_size_above_scale_that_reset_transform_restores(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    let clip = first_footage(&workspace, visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(clip, false);
        if !ws.ui.section_open("transform") {
            ws.ui.toggle_section("transform");
        }
        cx.notify();
    }));
    render(visual);
    let width = FieldTarget::MediaSize { layer: clip, side: 0 };
    assert!(visual.update(|window, _| window.try_find(field_id(width))).is_some(), "Size shows for video");
    let native = visual.update(|_, cx| workspace.read(cx).state.composition().layer(clip).unwrap().size);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        width.write(&mut ws.state, native.x / 2.0);
        cx.notify();
    }));
    render(visual);
    let size = visual.update(|_, cx| workspace.read(cx).state.composition().layer(clip).unwrap().size);
    assert_eq!(size, native * 0.5, "linked, so the height follows");
    visual.update(|window, cx| window.click("reset-transform", cx));
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.composition().layer(clip).unwrap().size), native, "Reset Transform puts the size back");
}

#[gpui_kit::test]
fn export_and_replace_puts_a_transparent_render_in_the_layers_place(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    // A short title keeps the render quick.
    let (track, in_frame) = visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let title = ws.state.composition_mut_for_test().layer_mut(TITLE).unwrap();
        title.out_frame = title.in_frame + 6;
        cx.notify();
        (title.track, title.in_frame)
    }));
    render(visual);
    visual.update(|window, cx| window.right_click(("clip-body", TITLE.0 as usize), cx));
    render(visual);
    click_label(visual, "Export and Replace Layer");
    visual.run_until_parked();
    wait_for_export(&workspace, visual);
    render(visual);
    let (clip, path) = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        let composition = state.composition();
        assert!(composition.layer(TITLE).is_none(), "the title was replaced");
        let clip = composition.layers.iter().find(|layer| layer.name == "Title").expect("by a clip with its name");
        assert_eq!((clip.track, clip.in_frame, clip.out_frame), (track, in_frame, in_frame + 6));
        let LayerContent::Footage { item } = clip.content else { panic!("a movie clip") };
        let item = state.project.item(item).unwrap();
        let exports = studio_core::folders::top_folder(&state.project, studio_core::folders::EXPORTS_FOLDER);
        assert_eq!(item.parent, exports, "filed in Exports");
        (clip.id, item.path.clone().unwrap())
    });
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.selected_layers.clone()), vec![clip]);
    let mut decoder = studio_media::VideoDecoder::open(&path, 640, false).unwrap();
    assert_eq!(decoder.frame_count(), 6);
    let frame = decoder.frame(2).unwrap();
    let lowest = frame.rgba.chunks(4).map(|pixel| pixel[3]).min();
    assert!(frame.has_alpha && lowest == Some(0), "around the text is see-through (alpha in the file: {}, lowest {lowest:?})", frame.has_alpha);
    std::fs::remove_file(&path).ok();
}

#[gpui_kit::test]
fn compact_tracks_are_shorter_rows(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let height = |visual: &mut VisualTestContext| visual.update(|window, _| window.find(("clip", TITLE.0 as usize)).bounds().size.height);
    let tall = height(visual);
    visual.update(|window, cx| window.click("tracks-compact", cx));
    render(visual);
    assert!(height(visual) < tall * 0.6, "{:?} vs {tall:?}", height(visual));
    assert!(visual.update(|_, cx| workspace.read(cx).ui.settings.compact_timeline));
}

/// Lets the Enhance Speech job finish (up to 60 s): its thread works in real time, and the
/// editor checks on it with a timer.
fn wait_for_enhance(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    let started = std::time::Instant::now();
    render(visual);
    while visual.update(|_, cx| workspace.read(cx).ui.enhance_job.is_some()) {
        assert!(started.elapsed() < std::time::Duration::from_secs(60), "the cleanup never finished");
        std::thread::sleep(std::time::Duration::from_millis(20));
        visual.executor().advance_clock(std::time::Duration::from_millis(150));
        visual.run_until_parked();
    }
    render(visual);
}

#[gpui_kit::test]
fn enhance_speech_makes_a_cleaned_copy_that_plays_instead(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let clip = add_tone_clip(&workspace, visual);
    tall_window(visual);
    visual.update(|window, cx| window.click("enhance-add", cx));
    wait_for_enhance(&workspace, visual);
    let heard = |visual: &mut VisualTestContext| visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        studio_core::audio::composition_audio(&state.project, state.composition()).into_iter().find(|sound| sound.layer == clip).unwrap().path
    });
    let cleaned = heard(visual);
    assert!(cleaned.to_string_lossy().contains("Enhanced"), "the cleaned copy plays: {cleaned:?}");
    let (channels, rate) = studio_enhance::read_float_wav(&cleaned).unwrap();
    assert_eq!((channels.len(), rate), (2, 48_000));
    assert!((channels[0].len() as f32 / 48_000.0 - 2.0).abs() < 0.05, "as long as the original");
    assert!(visual.update(|_, cx| workspace.read(cx).state.enhance_of(clip).is_some_and(|enhance| enhance.file.is_some())));

    // Off plays the original again; on brings the copy straight back, without cleaning again.
    visual.update(|window, cx| window.click("enhance-on", cx));
    render(visual);
    assert!(heard(visual).ends_with("tone.wav"));
    visual.update(|window, cx| window.click("enhance-on", cx));
    render(visual);
    assert_eq!(heard(visual), cleaned);
    assert!(visual.update(|_, cx| workspace.read(cx).ui.enhance_job.is_none()));

    // Removing it is one undoable step, and undo finds the copy again.
    visual.update(|window, cx| window.click("enhance-remove", cx));
    render(visual);
    assert!(heard(visual).ends_with("tone.wav"));
    visual.update(|window, cx| window.press("cmd-z", cx));
    render(visual);
    assert_eq!(heard(visual), cleaned);

    // A copy cleared from the cache is made again, rather than leaving the clip silent.
    std::fs::remove_file(&cleaned).unwrap();
    render(visual);
    assert!(heard(visual).ends_with("tone.wav"), "the original plays meanwhile");
    wait_for_enhance(&workspace, visual);
    assert_eq!(heard(visual), cleaned);
    assert!(cleaned.is_file());
}

#[gpui_kit::test]
fn project_items_drag_into_out_of_and_between_folders(cx: &mut TestAppContext) {
    use studio_core::sample::ids;
    let (workspace, visual) = open(cx);
    tall_window(visual);
    let parent = |visual: &mut VisualTestContext, item: Id| visual.update(|_, cx| workspace.read(cx).state.project.item(item).unwrap().parent);
    let solids = visual.update(|_, cx| studio_core::folders::top_folder(&workspace.read(cx).state.project, "Solids").unwrap());
    // A footage file onto the middle of the Solids folder goes in.
    let from = center_of(visual, ("project-item", ids::A_CAM.0 as usize));
    let to = center_of(visual, ("project-item", solids.0 as usize));
    visual.update(|window, cx| window.drag(from, to, cx));
    render(visual);
    assert_eq!(parent(visual, ids::A_CAM), Some(solids));
    // The Footage folder into Solids: folders go in folders.
    let from = center_of(visual, ("project-item", ids::FOOTAGE_FOLDER.0 as usize));
    let to = center_of(visual, ("project-item", solids.0 as usize));
    visual.update(|window, cx| window.drag(from, to, cx));
    render(visual);
    assert_eq!(parent(visual, ids::FOOTAGE_FOLDER), Some(solids));
    // And back out, onto the empty space below the rows: the top level.
    let list = visual.update(|window, _| window.find("project-items").bounds());
    let from = center_of(visual, ("project-item", ids::FOOTAGE_FOLDER.0 as usize));
    let below = point(list.center().x, list.bottom() - px(4.0));
    visual.update(|window, cx| window.drag(from, below, cx));
    render(visual);
    assert_eq!(parent(visual, ids::FOOTAGE_FOLDER), None);
    // A folder dropped on another's top edge goes just before it.
    let top_level_folders = |visual: &mut VisualTestContext| visual.update(|_, cx| {
        workspace.read(cx).state.project.items.iter().filter(|item| item.kind == studio_core::model::ProjectItemKind::Folder && item.parent.is_none()).map(|item| item.id).collect::<Vec<_>>()
    });
    let first = top_level_folders(visual)[0];
    let solids_row = visual.update(|window, _| window.find(("project-item", solids.0 as usize)).bounds());
    let first_row = visual.update(|window, _| window.find(("project-item", first.0 as usize)).bounds());
    visual.update(|window, cx| window.drag(solids_row.center(), point(first_row.center().x, first_row.top() + px(2.0)), cx));
    render(visual);
    assert_eq!(top_level_folders(visual)[0], solids, "Solids is first now");
    assert_eq!(parent(visual, solids), None);
}

#[gpui_kit::test]
fn ai_features_can_be_hidden_and_models_offered_for_download(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let clip = add_tone_clip(&workspace, visual);
    tall_window(visual);
    assert!(visual.update(|window, _| window.try_find("rail-captions")).is_some());
    assert!(visual.update(|window, _| window.try_find("section-enhance")).is_some());
    visual.update(|window, cx| window.press("cmd-,", cx));
    render(visual);
    // AI Models is the eighth page.
    visual.update(|window, cx| window.click(("settings-section", 7_usize), cx));
    render(visual);
    // The test model folders are empty, so both models offer a download.
    assert!(visual.update(|window, _| window.try_find("model-whisper-download")).is_some());
    assert!(visual.update(|window, _| window.try_find("model-sam-download")).is_some());
    visual.update(|window, cx| window.click("ai-all", cx));
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).ui.settings.ai), crate::app_settings::AiFeatures::all(false));
    assert_eq!(crate::app_settings::load().ai, crate::app_settings::AiFeatures::all(false), "remembered");
    visual.update(|window, cx| window.press("escape", cx));
    render(visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.select_layer(clip, false); cx.notify(); }));
    render(visual);
    for hidden in ["rail-captions", "section-enhance", "generate-captions"] {
        assert!(visual.update(|window, _| window.try_find(hidden)).is_none(), "{hidden} is hidden");
    }
}

#[gpui_kit::test]
fn importing_shows_a_popup_until_the_files_are_in(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let items = visual.update(|_, cx| workspace.read(cx).state.project.items.len());
    let files = vec![fixture("colors.mp4"), fixture("still.png"), fixture("tone.wav")];
    visual.update(|window, cx| workspace.update(cx, |ws, cx| ws.import_paths(files, None, window, cx)));
    render(visual);
    assert!(visual.update(|window, _| window.try_find("import-popup")).is_some(), "the popup shows while files are read");
    visual.run_until_parked();
    render(visual);
    assert!(visual.update(|window, _| window.try_find("import-popup")).is_none(), "and goes once they are in");
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.project.items.len()), items + 3);
}

#[gpui_kit::test]
fn a_cancelled_import_adds_nothing(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let items = visual.update(|_, cx| workspace.read(cx).state.project.items.len());
    let files = vec![fixture("colors.mp4"), fixture("still.png")];
    visual.update(|window, cx| workspace.update(cx, |ws, cx| ws.import_paths(files, None, window, cx)));
    render(visual);
    visual.update(|window, cx| window.click("import-cancel", cx));
    visual.run_until_parked();
    render(visual);
    assert!(visual.update(|window, _| window.try_find("import-popup")).is_none());
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.project.items.len()), items, "nothing was added");
}

#[gpui_kit::test]
fn importing_a_lottie_file_opens_its_animation(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let folder = scratch_folder("lottie");
    let file = folder.join("Bounce.json");
    std::fs::write(&file, r#"{"v":"5.7.0","fr":30,"ip":0,"op":45,"w":640,"h":360,"nm":"Bounce","assets":[],
        "layers":[{"ty":4,"nm":"Ball","ind":1,"ip":0,"op":45,"st":0,
            "ks":{"p":{"a":1,"k":[{"t":0,"s":[320,60],"o":{"x":[0.5],"y":[0]},"i":{"x":[0.5],"y":[1]}},{"t":30,"s":[320,300]}]}},
            "shapes":[{"ty":"el","s":{"a":0,"k":[80,80]},"p":{"a":0,"k":[0,0]}},{"ty":"fl","c":{"a":0,"k":[1,0.5,0,1]},"o":{"a":0,"k":100}}]}]}"#).unwrap();
    visual.update(|window, cx| workspace.update(cx, |ws, cx| ws.import_paths(vec![file.clone()], None, window, cx)));
    visual.run_until_parked();
    render(visual);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        let comp = state.composition();
        assert_eq!((comp.name.as_str(), comp.width, comp.height, comp.duration_frames), ("Bounce", 640, 360, 45), "the animation opened");
        let ball = comp.layers.iter().find(|layer| layer.name == "Ball").unwrap();
        assert_eq!(ball.transform.as_ref().unwrap().position.keyframe_frames(), vec![0, 30]);
        assert!(studio_core::folders::top_folder(&state.project, "Bounce").is_some(), "in a folder named after the file");
    });
    // Undo takes the whole import back in one step.
    let compositions = visual.update(|_, cx| workspace.read(cx).state.project.compositions.len());
    visual.update(|window, cx| window.press("cmd-z", cx));
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.project.compositions.len()), compositions - 1);
    std::fs::remove_dir_all(&folder).ok();
}

/// The sample project's first video.
const A_CAM: Id = Id(104);

#[gpui_kit::test]
fn the_eyedropper_tool_keeps_a_current_color_to_apply_and_save(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.press("i", cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.tool, crate::workspace::Tool::Eyedropper));
    // The sample project's background solid is dark gray at the bottom right.
    let at = screen_point(&workspace, visual, Vec2::new(1800.0, 1000.0));
    click_at(visual, at);
    let picked = visual.update(|_, cx| workspace.read(cx).state.colors.clone());
    assert_eq!(picked.recent.len(), 1, "the pick is remembered");
    assert_eq!(picked.recent[0], picked.current);
    assert_ne!(picked.current, studio_core::model::Rgba::hex(0xffffff), "the picture's color, not the starting white");

    // The color menu puts it on the selected text and saves it as a swatch.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.right_click("tool-eyedropper", cx));
    render(visual);
    visual.update(|window, cx| window.click("well-apply", cx));
    render(visual);
    let fill = visual.update(|_, cx| workspace.read(cx).state.text_style_of(TITLE).unwrap().fill);
    assert_eq!(fill, Some(picked.current));
    visual.update(|window, cx| window.click("well-save-swatch", cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.colors.swatches, vec![picked.current]));
    // A right-click on a swatch removes it.
    visual.update(|window, cx| window.right_click(("well-swatch", 0_usize), cx));
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).state.colors.swatches.is_empty()));
}

#[gpui_kit::test]
fn the_color_tab_grades_the_whole_video_on_its_adjustment_layer(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|window, cx| window.click("tab:Properties:2", cx));
    render(visual);
    visual.update(|window, cx| window.click("grade-target-video", cx));
    render(visual);
    // The panel starts empty; Add Correction adds a section.
    visual.update(|window, _| assert!(window.try_find("grade-basic-heading").is_none()));
    visual.update(|window, cx| window.click("grade-add:grade-basic", cx));
    render(visual);
    let (grade, basic) = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        let grade = state.grade_layer(crate::state::GradeTarget::Video).unwrap();
        (grade, state.effect_named(grade, "Basic Correction").unwrap())
    });
    // Its controls are the effect's parameters: typing a temperature sets it.
    let temperature = FieldTarget::Property { layer: grade, path: PropertyPath::Effect { effect: basic, parameter: 0 }, component: 0 };
    visual.update(|window, cx| window.click(field_id(temperature), cx));
    render(visual);
    visual.update(|window, cx| {
        window.input("40", cx);
        window.press("enter", cx);
    });
    render(visual);
    let value = |visual: &mut VisualTestContext, parameter| visual.update(|_, cx| workspace.read(cx).state.composition().layer(grade).unwrap().property(PropertyPath::Effect { effect: basic, parameter }, 0).unwrap().value.component(0));
    assert_eq!(value(visual, 0), 40.0);
    visual.update(|window, cx| window.click("grade-basic-reset", cx));
    render(visual);
    assert_eq!(value(visual, 0), 0.0, "Reset");
    visual.update(|window, cx| window.click("grade-basic-on", cx));
    render(visual);
    let enabled = visual.update(|_, cx| workspace.read(cx).state.composition().layer(grade).unwrap().effects.iter().find(|e| e.id == basic).unwrap().enabled);
    assert!(!enabled, "the eye turns it off");

    // The white balance picker waits for a click in the viewer, then lets go.
    visual.update(|window, cx| window.click(ElementId::Name(format!("white-balance-pick:{}", basic.0).into()), cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.white_balance_pick, Some((grade, basic))));
    let gray = screen_point(&workspace, visual, Vec2::new(1800.0, 1000.0));
    click_at(visual, gray);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.white_balance_pick, None));

    visual.update(|window, cx| window.click("grade-basic-remove", cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.effect_named(grade, "Basic Correction"), None));
}

#[gpui_kit::test]
fn the_color_tabs_lut_browser_shows_each_look_on_the_frame_and_picks_one(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(A_CAM, false);
        ws.ui.tabs.insert(crate::workspace::PanelId::Properties, 2);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click("grade-add:grade-lut", cx));
    render(visual);
    let effect = visual.update(|_, cx| workspace.read(cx).state.effect_named(A_CAM, "Apply LUT").unwrap());
    // A new LUT section opens its browser.
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.lut_browser, Some(effect)));
    // The previews are made a few per redraw, until every look has one.
    for _ in 0..20 {
        render(visual);
    }
    let looks: Vec<studio_core::model::LutSource> = studio_core::grade::looks().map(|look| studio_core::model::LutSource::Look(look.to_owned())).collect();
    visual.update(|_, cx| {
        let previews = &workspace.read(cx).ui.lut_previews;
        assert!(looks.iter().all(|look| previews.picture(look).is_some()), "every look has a preview");
    });
    // Clicking a tile puts that look on the clip, and the browser stays open to compare.
    visual.update(|window, cx| window.click(("lut-tile", 1_usize), cx));
    render(visual);
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        assert_eq!(ws.state.effect_lut(A_CAM, effect), Some(&looks[1]));
        assert_eq!(ws.ui.lut_browser, Some(effect));
    });
    // Done closes it; the chosen look stays.
    visual.update(|window, cx| window.click(ElementId::Name(format!("lut-browse:{}", effect.0).into()), cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.lut_browser, None));
    // With a correction added, the tiles give way to the Add Correction menu, which offers
    // only what isn't there yet.
    visual.update(|window, _| {
        assert!(window.try_find("grade-add:grade-basic").is_none());
        assert!(window.try_find("grade-add").is_some());
    });
    visual.update(|window, cx| window.click("grade-add", cx));
    render(visual);
    click_label(visual, "Levels");
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).state.effect_named(A_CAM, "Levels").is_some()));
}

#[gpui_kit::test]
fn color_sliders_set_reset_and_undo_in_one_step(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(A_CAM, false);
        ws.ui.tabs.insert(crate::workspace::PanelId::Properties, 2);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click("grade-add:grade-basic", cx));
    render(visual);
    let basic = visual.update(|_, cx| workspace.read(cx).state.effect_named(A_CAM, "Basic Correction").unwrap());
    let temperature = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(A_CAM).unwrap().property(PropertyPath::Effect { effect: basic, parameter: 0 }, 0).unwrap().value.component(0));
    let track = ElementId::Name(format!("grade-slider:{}:0", basic.0).into());
    let bounds = visual.update(|window, _| window.find(track.clone()).bounds());
    // Drag from the middle (0) to three quarters along: −100 to 100, so 50.
    let middle = bounds.center();
    let three_quarters = point(bounds.origin.x + bounds.size.width * 0.75, middle.y);
    visual.update(|window, cx| window.drag(middle, three_quarters, cx));
    render(visual);
    assert!((temperature(visual) - 50.0).abs() <= 1.0, "dragged to {}", temperature(visual));
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.undo();
        cx.notify();
    }));
    render(visual);
    assert_eq!(temperature(visual), 0.0, "one undo takes the whole drag back");
    // A click sets it straight away; a double-click on the name puts it back.
    let quarter = point(bounds.origin.x + bounds.size.width * 0.25, middle.y);
    click_at(visual, quarter);
    assert!((temperature(visual) + 50.0).abs() <= 1.0, "clicked to {}", temperature(visual));
    visual.update(|window, cx| window.double_click(ElementId::Name(format!("grade-label:{}:0", basic.0).into()), cx));
    render(visual);
    assert_eq!(temperature(visual), 0.0);
}

#[gpui_kit::test]
fn dragging_a_color_wheel_tints_and_undoes_in_one_step(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(A_CAM, false);
        ws.ui.tabs.insert(crate::workspace::PanelId::Properties, 2);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click("grade-add:grade-wheels", cx));
    render(visual);
    let wheels = visual.update(|_, cx| workspace.read(cx).state.effect_named(A_CAM, "Color Wheels").unwrap());
    let amount = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(A_CAM).unwrap().property(PropertyPath::Effect { effect: wheels, parameter: 7 }, 0).unwrap().value.component(0));
    let hue = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(A_CAM).unwrap().property(PropertyPath::Effect { effect: wheels, parameter: 6 }, 0).unwrap().value.component(0));
    // Drag the Highlights wheel from its middle straight up: toward yellow-green (90°).
    let circle = visual.update(|window, _| window.find(("grade-wheel", 2_usize)).bounds());
    let center = circle.center();
    visual.update(|window, cx| window.drag(center, point(center.x, center.y - circle.size.height * 0.4), cx));
    render(visual);
    assert!(amount(visual) > 50.0, "pushed most of the way out: {}", amount(visual));
    assert!((hue(visual) - 90.0).abs() < 3.0, "straight up is 90°: {}", hue(visual));
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.undo();
        cx.notify();
    }));
    render(visual);
    assert_eq!(amount(visual), 0.0, "one undo takes the whole drag back");
}

#[gpui_kit::test]
fn hovering_a_lut_previews_it_in_the_viewer_without_changing_the_project(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|window, cx| window.click("rail-effects", cx));
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(A_CAM, false);
        // Fold the other categories so the LUTs are in view.
        ws.ui.closed_effect_categories.extend(studio_core::effects::CATEGORIES.iter().map(|category| category.name));
        ws.ui.closed_effect_categories.insert("Transitions");
        cx.notify();
    }));
    render(visual);
    let before = visual.update(|_, cx| workspace.read(cx).state.project.clone());
    let row = visual.update(|window, _| window.find(("lut", 0_usize)).bounds().center());
    visual.update(|window, cx| window.pointer_move(row, None, cx));
    render(visual);
    let first = studio_core::model::LutSource::Look(studio_core::grade::looks().next().unwrap().to_owned());
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        assert_eq!(ws.ui.lut_hover.as_ref(), Some(&first), "the viewer previews the LUT under the pointer");
        assert_eq!(ws.state.project, before, "previewing changes nothing");
        // What the viewer draws: the selected clip with the LUT on it.
        let shown = crate::panels::viewer::lut_preview(ws).unwrap();
        let clip = shown.composition(ws.state.active_comp).unwrap().layer(A_CAM).unwrap();
        assert!(clip.effects.iter().any(|effect| effect.lut.as_ref() == Some(&first)));
    });
    // With nothing selected, it goes over the whole picture.
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.selected_layers.clear()));
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        let shown = crate::panels::viewer::lut_preview(ws).unwrap();
        let top = &shown.composition(ws.state.active_comp).unwrap().layers[0];
        assert!(matches!(top.content, LayerContent::Adjustment) && top.effects.iter().any(|effect| effect.lut.as_ref() == Some(&first)));
    });
    // Leaving the row ends the preview, and it left no undo step behind.
    visual.update(|window, cx| window.pointer_move(point(px(800.0), px(10.0)), None, cx));
    render(visual);
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        assert_eq!(ws.ui.lut_hover, None);
        assert_eq!(ws.state.project, before);
    });
}

#[gpui_kit::test]
fn luts_go_on_clips_from_the_effects_panel_and_change_in_effect_controls(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|window, cx| window.click("rail-effects", cx));
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(A_CAM, false);
        // Fold the other categories so the LUTs are in view.
        ws.ui.closed_effect_categories.extend(studio_core::effects::CATEGORIES.iter().map(|category| category.name));
        ws.ui.closed_effect_categories.insert("Transitions");
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click(("lut-apply", 0_usize), cx));
    render(visual);
    let effect = visual.update(|_, cx| workspace.read(cx).state.effect_named(A_CAM, "Apply LUT").unwrap());
    let lut = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.effect_lut(A_CAM, effect).cloned());
    assert_eq!(lut(visual), Some(studio_core::model::LutSource::Look(studio_core::grade::looks().next().unwrap().to_owned())));

    // Effect Controls offers the LUT menu.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.ui.tabs.insert(crate::workspace::PanelId::Properties, 1);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click(ElementId::Name(format!("lut-menu:{}", effect.0).into()), cx));
    render(visual);
    click_label(visual, "Moody Green");
    render(visual);
    assert_eq!(lut(visual), Some(studio_core::model::LutSource::Look("Moody Green".into())));

    // New Adjustment Layer adds one over everything and selects it.
    visual.update(|window, cx| window.click("new-adjustment-layer", cx));
    render(visual);
    let selected = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        state.selected_layers.first().and_then(|id| state.composition().layer(*id)).map(|layer| layer.content.clone())
    });
    assert!(matches!(selected, Some(LayerContent::Adjustment)));
}

#[gpui_kit::test]
fn the_viewer_draws_a_frame_only_when_it_changes(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    for _ in 0..3 {
        draw_frame(visual);
    }
    let drawn = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).render.frames_drawn);
    let start = drawn(visual);
    // Edits elsewhere redraw the viewer panel, but the same picture isn't drawn again.
    for _ in 0..5 {
        visual.update(|_, cx| workspace.update(cx, |_, cx| cx.notify()));
        draw_frame(visual);
    }
    assert_eq!(drawn(visual), start, "nothing it shows changed");
    // Moving the playhead draws the new frame.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.current_frame += 10;
        cx.notify();
    }));
    draw_frame(visual);
    assert_eq!(drawn(visual), start + 1);
    // Dragging the divider above the timeline makes the viewer taller: the picture on screen
    // is stretched instead of drawn at every size, then drawn once at the new size.
    let viewer = visual.update(|window, _| window.find("viewer").bounds());
    // Below the viewer's footer, in the gap between it and the timeline.
    let grip = point(viewer.center().x, viewer.origin.y + viewer.size.height + px(crate::theme::size::PANEL_FOOTER_HEIGHT + crate::theme::size::GUTTER * 0.5));
    visual.update(|window, cx| window.pointer_down(grip, MouseButton::Left, cx));
    draw_frame(visual);
    for step in 1..=20 {
        visual.update(|window, cx| window.pointer_move(point(grip.x, grip.y + px(step as f32 * 4.0)), Some(MouseButton::Left), cx));
        draw_frame(visual);
    }
    let grown = visual.update(|window, _| window.find("viewer").bounds()).size.height - viewer.size.height;
    assert!(grown > px(40.0), "the viewer grew: {grown:?}");
    assert_eq!(drawn(visual), start + 1, "no frames drawn mid-drag");
    visual.update(|window, cx| window.pointer_up(point(grip.x, grip.y + px(80.0)), MouseButton::Left, cx));
    draw_frame(visual);
    draw_frame(visual);
    assert_eq!(drawn(visual), start + 2, "one frame at the new size once the drag ends");
}

/// The first footage clip in the open composition.
fn first_footage(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> Id {
    visual.update(|_, cx| workspace.read(cx).state.composition().layers.iter().find(|layer| matches!(layer.content, LayerContent::Footage { .. })).unwrap().id)
}

#[gpui_kit::test]
fn the_effects_tab_adds_effects_and_slides_their_settings(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    let clip = first_footage(&workspace, visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(clip, false);
        ws.ui.tabs.insert(crate::workspace::PanelId::Properties, 1);
        cx.notify();
    }));
    render(visual);
    // The Add Effect menu lists what suits the clip; picking one adds it as a card.
    visual.update(|window, cx| window.click("fx-add", cx));
    render(visual);
    // Crop is near the top of the list, so it is in view without scrolling the menu.
    click_label(visual, "Crop");
    render(visual);
    let crop = visual.update(|_, cx| workspace.read(cx).state.effect_named(clip, "Crop")).expect("Crop was added");
    assert!(visual.update(|window, _| window.try_find(ElementId::Name(format!("fx-twirl:{}", crop.0).into())).is_some()), "it shows as a card");
    // Its Left (0–100%) has a slider: a quarter of the way along is 25%.
    let amount = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(clip).unwrap().property(PropertyPath::Effect { effect: crop, parameter: 0 }, 0).unwrap().value.component(0));
    let bounds = visual.update(|window, _| window.find(ElementId::Name(format!("grade-slider:{}:0", crop.0).into())).bounds());
    let middle = bounds.center();
    visual.update(|window, cx| window.drag(middle, point(bounds.origin.x + bounds.size.width * 0.25, middle.y), cx));
    render(visual);
    assert!((amount(visual) - 25.0).abs() <= 1.0, "dragged to {}", amount(visual));
    // The card's buttons turn it off and remove it.
    visual.update(|window, cx| window.click(ElementId::Name(format!("fx-on:{}", crop.0).into()), cx));
    render(visual);
    let enabled = visual.update(|_, cx| workspace.read(cx).state.composition().layer(clip).unwrap().effects.iter().find(|e| e.id == crop).unwrap().enabled);
    assert!(!enabled, "the eye turns it off");
    visual.update(|window, cx| window.click(ElementId::Name(format!("fx-remove:{}", crop.0).into()), cx));
    render(visual);
    assert!(visual.update(|_, cx| workspace.read(cx).state.effect_named(clip, "Crop")).is_none(), "removed");
}

#[gpui_kit::test]
fn new_layers_markers_and_guides_come_from_the_menus(cx: &mut TestAppContext) {
    use crate::workspace::{NewAdjustmentLayer, NewNull, NewSolid, ToggleGuides, ToggleMarker};
    let (workspace, visual) = open(cx);
    let top_name = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layers[0].name.clone());
    visual.update(|window, cx| window.dispatch_action(Box::new(NewSolid), cx));
    render(visual);
    assert_eq!(top_name(visual), "Solid 1");
    visual.update(|window, cx| window.dispatch_action(Box::new(NewAdjustmentLayer), cx));
    render(visual);
    assert!(top_name(visual).starts_with("Adjustment Layer"));
    // The top bar's New Layer button offers the same.
    visual.update(|window, cx| window.click("tool-new-layer", cx));
    render(visual);
    click_label(visual, "Null Object (⇧⌥⌘Y)");
    render(visual);
    assert_eq!(top_name(visual), "Null 1");
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        assert_eq!(state.selected_layers, vec![state.composition().layers[0].id], "the new layer is selected");
    });
    visual.update(|window, cx| window.dispatch_action(Box::new(NewNull), cx));
    render(visual);
    assert_eq!(top_name(visual), "Null 2");

    // M marks the playhead, and again takes the mark away.
    let markers_at = |visual: &mut VisualTestContext| visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        state.composition().markers.iter().filter(|marker| marker.frame == state.current_frame).count()
    });
    visual.update(|window, cx| window.dispatch_action(Box::new(ToggleMarker), cx));
    render(visual);
    assert_eq!(markers_at(visual), 1);
    visual.update(|window, cx| window.dispatch_action(Box::new(ToggleMarker), cx));
    render(visual);
    assert_eq!(markers_at(visual), 0);

    // The guides show over the picture, from the menu or the viewer's button.
    let guides = |visual: &mut VisualTestContext| visual.update(|window, _| window.try_find("viewer-guides-overlay").is_some());
    assert!(!guides(visual));
    visual.update(|window, cx| window.dispatch_action(Box::new(ToggleGuides), cx));
    render(visual);
    assert!(guides(visual));
    visual.update(|window, cx| window.click("viewer-guides", cx));
    render(visual);
    assert!(!guides(visual));
}

#[gpui_kit::test]
fn stagger_layers_spaces_the_selection_out(cx: &mut TestAppContext) {
    use crate::workspace::{NewSolid, STAGGER_FRAMES, StaggerLayers};
    let (workspace, visual) = open(cx);
    let mut solids = Vec::new();
    for _ in 0..3 {
        visual.update(|window, cx| window.dispatch_action(Box::new(NewSolid), cx));
        render(visual);
        solids.push(visual.update(|_, cx| workspace.read(cx).state.selected_layers[0]));
    }
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.clear_selection();
        for &solid in &solids {
            ws.state.select_layer(solid, true);
        }
        cx.notify();
    }));
    visual.update(|window, cx| window.dispatch_action(Box::new(StaggerLayers), cx));
    render(visual);
    let ins: Vec<i64> = solids.iter().map(|&solid| layer_timing(&workspace, visual, solid).1).collect();
    assert_eq!(ins, vec![ins[0], ins[0] + STAGGER_FRAMES, ins[0] + 2 * STAGGER_FRAMES]);
}

#[gpui_kit::test]
fn the_rate_stretch_tool_changes_a_clips_speed(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    let clip = first_footage(&workspace, visual);
    visual.update(|window, cx| window.press("r", cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.tool, crate::workspace::Tool::RateStretch));
    let (_, in_before, out_before) = layer_timing(&workspace, visual, clip);
    let grip = center_of(visual, ("clip-trim-out", clip.0 as usize));
    visual.update(|window, cx| window.drag(grip, point(grip.x + px(60.0), grip.y), cx));
    render(visual);
    let (_, in_after, out_after) = layer_timing(&workspace, visual, clip);
    let speed = visual.update(|_, cx| workspace.read(cx).state.composition().layer(clip).unwrap().time.speed.value);
    assert_eq!(in_after, in_before, "the start stays put");
    assert!(out_after > out_before, "the clip got longer: {out_before} → {out_after}");
    let expected = 100.0 * (out_before - in_before) as f32 / (out_after - in_after) as f32;
    assert!(speed < 100.0 && (speed - expected).abs() < 2.0, "slowed to fill it: {speed}% (about {expected}%)");
}
