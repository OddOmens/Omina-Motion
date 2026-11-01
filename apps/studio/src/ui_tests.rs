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
use crate::timeline_rows::layer_key;
use crate::workspace::Workspace;

const TITLE: Id = Id(100);
const ROTATION: PropertyPath = PropertyPath::Transform(TransformProperty::Rotation);
const POSITION: PropertyPath = PropertyPath::Transform(TransformProperty::Position);
const OPACITY: PropertyPath = PropertyPath::Transform(TransformProperty::Opacity);

/// Opens the editor in a 1600 × 960 offscreen window and renders the first frames.
fn open(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply(cx);
        crate::workspace::bind_keys(cx);
    });
    let mut workspace = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    visual.simulate_resize(size(px(1600.0), px(960.0)));
    // Most tests drive the Layers view; Tracks view tests switch with `show_tracks`.
    let view = workspace.clone().expect("workspace was built");
    visual.update(|_, cx| view.update(cx, |ws, _| ws.ui.timeline.view = crate::panels::timeline::TimelineView::Layers));
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
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.state.expanded_rows.insert(layer_key(TITLE));
            ws.state.expanded_rows.insert(format!("{}/transform", layer_key(TITLE)));
        })
    });
    render(visual);

    // Click the opacity keyframe at frame 36 in the timeline, then press Delete.
    visual.update(|window, cx| window.click(ElementId::Name("kf:layer:100:transform:Opacity:36".into()), cx));
    render(visual);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        assert_eq!(state.selected_keyframes, vec![KeyframeRef { layer: TITLE, path: OPACITY, frame: 36 }]);
    });
    visual.update(|window, cx| window.press("backspace", cx));
    render(visual);
    let keys = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().property(OPACITY, 0).unwrap().keyframes);
    assert_eq!(keys, vec![24, 156, 168]);

    visual.update(|window, cx| window.press("backspace", cx));
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).state.composition().layer(TITLE).is_none(), "the layer is deleted next"));
}

#[gpui_kit::test]
fn dragging_a_keyframe_moves_it_in_time(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.state.expanded_rows.insert(layer_key(TITLE));
            ws.state.expanded_rows.insert(format!("{}/transform", layer_key(TITLE)));
        })
    });
    render(visual);
    let from = center_of(visual, ElementId::Name("kf:layer:100:transform:Opacity:156".into()));
    visual.update(|window, cx| window.drag(from, point(from.x - px(40.0), from.y), cx));
    render(visual);
    let keys = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().property(OPACITY, 0).unwrap().keyframes);
    assert!(keys.len() == 4 && !keys.contains(&156) && keys[2] < 156, "the keyframe moved earlier: {keys:?}");
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
    click_label(visual, "Vertical 9:16");
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
fn expand_title(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    visual.update(|_, cx| {
        workspace.update(cx, |ws, _| {
            ws.state.expanded_rows.insert(layer_key(TITLE));
            ws.state.expanded_rows.insert(format!("{}/transform", layer_key(TITLE)));
        })
    });
    render(visual);
}

#[gpui_kit::test]
fn dragging_a_box_selects_the_keyframes_inside_it(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    expand_title(&workspace, visual);
    // From just left of the Position keyframe at 24 down past the Opacity keyframe at 36.
    let position_24 = center_of(visual, ElementId::Name("kf:layer:100:transform:Position:24".into()));
    let opacity_36 = center_of(visual, ElementId::Name("kf:layer:100:transform:Opacity:36".into()));
    let from = point(position_24.x - px(8.0), position_24.y - px(6.0));
    let to = point(opacity_36.x + px(8.0), opacity_36.y + px(6.0));
    visual.update(|window, cx| window.drag(from, to, cx));
    render(visual);
    visual.update(|_, cx| {
        let mut picked: Vec<(String, i64)> =
            workspace.read(cx).state.selected_keyframes.iter().map(|key| (format!("{:?}", key.path), key.frame)).collect();
        picked.sort();
        assert_eq!(
            picked,
            vec![
                ("Transform(Opacity)".into(), 24),
                ("Transform(Opacity)".into(), 36),
                ("Transform(Position)".into(), 24),
            ],
            "the box covers Position 24 and Opacity 24–36, but not Position 48"
        );
    });
}

#[gpui_kit::test]
fn clicking_a_property_name_selects_its_keyframes_and_f9_eases_them(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    expand_title(&workspace, visual);
    visual.update(|window, cx| window.click(ElementId::Name("prop-name:layer:100:transform:Opacity".into()), cx));
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
    click_label(visual, "Square 1:1");
    render(visual);
    click_label(visual, "OK");
    render(visual);
    visual.update(|_, cx| {
        let comp = workspace.read(cx).state.composition();
        assert_eq!((comp.width, comp.height), (1080, 1080));
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
                ws.state.expanded_rows.insert(layer_key(TITLE));
                ws.state.expanded_rows.insert(format!("{}/transform", layer_key(TITLE)));
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
fn the_mode_column_menu_sets_the_blend_mode(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.click(("mode", TITLE.0 as usize), cx));
    render(visual);
    click_label(visual, "Multiply");
    render(visual);
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        assert_eq!(ws.state.composition().layer(TITLE).unwrap().blend_mode, studio_core::model::BlendMode::Multiply);
    });
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
    for kind in [PanelKind::Toolbar, PanelKind::Project, PanelKind::Effects, PanelKind::Align] {
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
    click_label(visual, "ProRes 422 HQ (MOV)");
    render(visual);
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
    assert_eq!((info.codec.as_str(), info.width, info.height, info.frame_count), ("prores", 1920, 1080, 12));
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
fn dragging_a_bar_end_trims_and_undoes_in_one_step(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let before = layer_timing(&workspace, visual, TITLE);
    let grip = center_of(visual, ElementId::Name(format!("trim-out:{}", TITLE.0).into()));
    visual.update(|window, cx| window.drag(grip, point(grip.x - px(60.0), grip.y), cx));
    render(visual);
    let trimmed = layer_timing(&workspace, visual, TITLE);
    assert!(trimmed.2 < before.2, "the out point moved earlier: {before:?} → {trimmed:?}");
    assert_eq!((trimmed.0, trimmed.1), (before.0, before.1), "only the out point changes");
    // Keyframes past the new out point are kept, as in After Effects, so extending it back restores them.
    visual.update(|window, cx| window.press("cmd-z", cx));
    render(visual);
    assert_eq!(layer_timing(&workspace, visual, TITLE), before);
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
    let bar = center_of(visual, ElementId::Name(format!("bar:{}", clip.0).into()));
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
    let bar = center_of(visual, ElementId::Name(format!("bar:{}", TITLE.0).into()));
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

#[gpui_kit::test]
fn the_parent_column_is_visible_at_the_default_layout(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let outline = visual.update(|_, cx| workspace.read(cx).ui.timeline.outline_width);
    assert!(outline >= 762.0, "every outline column fits, Parent & Link included ({outline} px)");
}

// Tracks view ---------------------------------------------------------------------------------

fn show_tracks(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    // A tall window, so every track of the sample project is on screen without scrolling.
    visual.simulate_resize(size(px(1600.0), px(1500.0)));
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.ui.timeline.view = crate::panels::timeline::TimelineView::Tracks;
        cx.notify();
    }));
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
fn the_header_switches_between_layers_and_tracks(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.click("timeline-view-tracks", cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.timeline.view, crate::panels::timeline::TimelineView::Tracks));
    // The title is drawn as a clip (finding it fails the test if it isn't there).
    center_of(visual, ("clip", TITLE.0 as usize));
    visual.update(|window, cx| window.click("timeline-view-layers", cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.timeline.view, crate::panels::timeline::TimelineView::Layers));
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
    show_tracks(&workspace, visual);
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
    show_tracks(&workspace, visual);
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
    show_tracks(&workspace, visual);
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
    show_tracks(&workspace, visual);
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
    show_tracks(&workspace, visual);
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
    visual.update(|window, cx| window.click("toolbar-export", cx));
    render(visual);
    visual.update(|window, cx| assert!(window.has_active_dialog(cx)));
}

#[gpui_kit::test]
fn the_tracks_toolbar_plays_and_zooms(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    show_tracks(&workspace, visual);
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
    show_tracks(&workspace, visual);
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
fn files_dropped_on_the_layers_timeline_become_layers(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let before = visual.update(|_, cx| workspace.read(cx).state.composition().layers.len());
    let bar = center_of(visual, ElementId::Name(format!("bar:{}", TITLE.0).into()));
    drop_files(visual, vec![fixture("colors.mp4")], bar);
    let after = visual.update(|_, cx| workspace.read(cx).state.composition().layers.len());
    assert_eq!(after, before + 1);
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
    show_tracks(&workspace, visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.ui.timeline.expanded_clips.insert(TITLE);
        ws.state.snapping = false;
        ws.state.set_view_range(studio_core::time::FrameRange::new(24.0, 120.0));
        cx.notify();
    }));
    let path = save_as(visual, &folder, "Remembered.studio");
    // Change everything, then open the file again.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.ui.timeline.view = crate::panels::timeline::TimelineView::Layers;
        ws.ui.timeline.expanded_clips.clear();
        ws.state.snapping = true;
        ws.state.set_view_range(studio_core::time::FrameRange::new(0.0, 240.0));
        cx.notify();
    }));
    visual.update(|window, cx| workspace.update(cx, |ws, cx| ws.open_file(path.clone(), window, cx)));
    render(visual);
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        assert_eq!(ws.ui.timeline.view, crate::panels::timeline::TimelineView::Tracks);
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
    visual.update(|_, cx| workspace.read(cx).keep_recovery_copy());
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
fn discarding_recovered_work_removes_the_copy(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    make_an_edit(&workspace, visual);
    visual.update(|_, cx| workspace.read(cx).keep_recovery_copy());
    visual.update(|window, cx| workspace.update(cx, |ws, cx| ws.restore_session(window, cx)));
    visual.run_until_parked();
    visual.simulate_prompt_answer("Discard");
    visual.run_until_parked();
    assert!(!crate::app_settings::recovery_path().exists());
}
