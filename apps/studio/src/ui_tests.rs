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
    visual.update(|window, cx| window.click("tracks-start", cx));
    assert_eq!(frame(visual), 0);
    visual.update(|window, cx| window.click("next-frame", cx));
    assert_eq!(frame(visual), 1);
    visual.update(|window, cx| window.click("previous-frame", cx));
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
fn the_easings_button_shows_the_graph_editor_in_the_motion_tab(cx: &mut TestAppContext) {
    use crate::workspace::PanelId;
    let (workspace, visual) = open(cx);
    // Even with the panel folded away and its Keyframe Easing section closed.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        ws.ui.collapsed_panels.insert(PanelId::Properties);
        if ws.ui.section_open("easing") {
            ws.ui.toggle_section("easing");
        }
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click("easings", cx));
    render(visual);
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        assert!(crate::panels::graph_editor::showing(ws));
        assert_eq!(ws.ui.tab(PanelId::Properties), crate::panels::motion::TAB);
        assert!(ws.ui.section_open("easing"));
    });
    assert!(visual.update(|window, _| window.try_find("motion-body")).is_some(), "the Motion tab is drawn");
}

#[gpui_kit::test]
fn an_in_animations_easing_is_picked_or_dragged_in_the_motion_tab(cx: &mut TestAppContext) {
    use studio_core::presets::{Motion, Phase, default_easing, motion_easing};
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        ws.ui.tabs.insert(crate::workspace::PanelId::Properties, crate::panels::motion::TAB);
        ws.ui.open_section("animate");
        ws.state.set_layer_motion(TITLE, Phase::In, Some(Motion::Rise), 0.5);
        cx.notify();
    }));
    render(visual);
    let animation = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().animation);
    assert_eq!(motion_easing(&animation(visual), Phase::In), Some(default_easing(Motion::Rise, Phase::In)));
    // Spring: the rise now springs into place, on its keyframes too.
    visual.update(|window, cx| window.click(ElementId::Name("motion-in-ease-Spring".into()), cx));
    render(visual);
    let spring = Easing::Elastic(studio_core::animation::EaseMode::Out);
    assert_eq!(animation(visual).enter_easing, Some(spring));
    let first = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().transform.as_ref().unwrap().position.keyframes()[0].easing);
    assert_eq!(first, spring);
    // Snappy is a curve, whose handle drags into a custom one.
    visual.update(|window, cx| window.click(ElementId::Name("motion-in-ease-Snappy".into()), cx));
    render(visual);
    let handle = center_of(visual, ElementId::Name("motion-in-handle-1".into()));
    visual.update(|window, cx| window.drag(handle, point(handle.x - px(60.0), handle.y), cx));
    render(visual);
    let Some(Easing::Curve(curve)) = animation(visual).enter_easing else { panic!("a custom curve") };
    assert!(curve.x2 < 0.2, "the second handle moved left: {curve:?}");
    // Default goes back to the motion's own easing.
    visual.update(|window, cx| window.click(ElementId::Name("motion-in-ease-Default".into()), cx));
    render(visual);
    assert_eq!(animation(visual).enter_easing, None);
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
fn the_nodes_panel_connects_disconnects_merges_and_deletes(cx: &mut TestAppContext) {
    use crate::panels::nodes::{input_id, node_id, output_id};
    use studio_core::effect_graph::{self, InputPort, NodeId};
    let (workspace, visual) = open(cx);
    let a_cam = Id(104);
    let blur = NodeId::Effect(Id(202));
    let layer = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(a_cam).unwrap().clone());
    let fed = |visual: &mut VisualTestContext, node: NodeId, port: InputPort| effect_graph::source_of(&layer(visual), node, port);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(a_cam, false);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click(ElementId::Name("tab:Properties:1".into()), cx));
    render(visual);
    visual.update(|window, cx| window.click("fx-open-nodes", cx));
    render(visual);
    visual.update(|window, cx| window.click("nodes-fit", cx));
    render(visual);
    visual.update(|window, _| {
        window.find(node_id(NodeId::Source));
        window.find(node_id(blur));
        window.find(node_id(NodeId::Output));
    });
    assert!(!layer(visual).effect_graph.wired, "nothing is rewired just by looking");

    // Source's output dragged onto the Output's input bypasses the blur.
    visual.update(|window, cx| window.drag_to(output_id(NodeId::Source), input_id(NodeId::Output, InputPort::Main), cx));
    render(visual);
    assert_eq!(fed(visual, NodeId::Output, InputPort::Main), Some(NodeId::Source));
    assert!(layer(visual).effect_graph.wired);

    // Pulling that wire off the input and dropping it on empty canvas disconnects it.
    let canvas = visual.update(|window, _| window.find("nodes-canvas").bounds());
    let empty = point(canvas.origin.x + px(12.0), canvas.origin.y + canvas.size.height - px(12.0));
    let input = center_of(visual, input_id(NodeId::Output, InputPort::Main));
    visual.update(|window, cx| window.drag(input, empty, cx));
    render(visual);
    assert_eq!(fed(visual, NodeId::Output, InputPort::Main), None);

    // Wires can be drawn from an empty input back to an output, too.
    visual.update(|window, cx| window.drag_to(input_id(NodeId::Output, InputPort::Main), output_id(blur), cx));
    render(visual);
    assert_eq!(fed(visual, NodeId::Output, InputPort::Main), Some(blur));

    // A Merge from the footer's Add menu, wired in: Source under the blurred picture.
    visual.update(|window, cx| window.click("nodes-add", cx));
    render(visual);
    click_label(visual, "Merge");
    render(visual);
    let merge = layer(visual).effect_graph.merges.first().map(|merge| NodeId::Merge(merge.id)).expect("a Merge was added");
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.nodes.selected, Some(merge), "the new node is picked"));
    let NodeId::Merge(merge_id) = merge else { unreachable!() };
    visual.update(|window, cx| window.click("nodes-fit", cx));
    render(visual);
    visual.update(|window, cx| window.drag_to(output_id(NodeId::Source), input_id(merge, InputPort::Background), cx));
    render(visual);
    visual.update(|window, cx| window.drag_to(output_id(blur), input_id(merge, InputPort::Foreground), cx));
    render(visual);
    visual.update(|window, cx| window.drag_to(output_id(merge), input_id(NodeId::Output, InputPort::Main), cx));
    render(visual);
    assert_eq!(fed(visual, merge, InputPort::Background), Some(NodeId::Source));
    assert_eq!(fed(visual, merge, InputPort::Foreground), Some(blur));
    assert_eq!(fed(visual, NodeId::Output, InputPort::Main), Some(merge));
    let plan = effect_graph::plan(&layer(visual)).unwrap();
    assert!(matches!(plan.steps.last(), Some(effect_graph::Step::Merge { .. })), "{:?}", plan.steps);

    // A connection that would loop is refused and changes nothing.
    visual.update(|window, cx| window.drag_to(output_id(merge), input_id(blur, InputPort::Main), cx));
    render(visual);
    assert_eq!(fed(visual, blur, InputPort::Main), Some(NodeId::Source));

    // The Merge's settings are in the Effects tab: its blend mode and opacity.
    visual.update(|window, cx| window.click(ElementId::Name(format!("merge-blend:{}", merge_id.0).into()), cx));
    render(visual);
    click_label(visual, "Screen");
    render(visual);
    let slider = visual.update(|window, _| window.find(ElementId::Name(format!("merge-opacity:{}", merge_id.0).into())).bounds());
    visual.update(|window, cx| window.click_at(ElementId::Name(format!("merge-opacity:{}", merge_id.0).into()), point(slider.size.width * 0.25, px(8.0)), cx));
    render(visual);
    let settings = layer(visual).effect_graph.merges[0].clone();
    assert_eq!(settings.blend, studio_core::model::BlendMode::Screen);
    assert!((settings.opacity - 0.25).abs() < 0.03, "{}", settings.opacity);

    // Dragging a node's title moves it.
    let before = effect_graph::position(&layer(visual), blur);
    let title = center_of(visual, ElementId::Name("node-title:effect-202".into()));
    visual.update(|window, cx| window.drag(title, point(title.x + px(60.0), title.y + px(30.0)), cx));
    render(visual);
    let after = effect_graph::position(&layer(visual), blur);
    assert!(after.x > before.x + 20.0 && after.y > before.y + 10.0, "{before:?} → {after:?}");

    // Picking the Merge and pressing Delete removes it; undo brings it back.
    visual.update(|window, cx| window.click(ElementId::Name(format!("node-title:merge-{}", merge_id.0).into()), cx));
    render(visual);
    visual.update(|window, cx| window.press("delete", cx));
    render(visual);
    assert!(layer(visual).effect_graph.merges.is_empty());
    visual.update(|window, cx| window.press("cmd-z", cx));
    render(visual);
    assert_eq!(layer(visual).effect_graph.merges.len(), 1);
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
        window.press("cmd-enter", cx);
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
        window.press("cmd-enter", cx);
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

/// Opens the open menu's submenu `submenu` (Time, Transform, Export…), then clicks `label` in it.
fn click_in_submenu(visual: &mut VisualTestContext, submenu: &str, label: &str) {
    click_label(visual, submenu);
    render(visual);
    click_label(visual, label);
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
    visual.update(|window, cx| window.click("rail-new-comp", cx));
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
fn the_export_preview_shows_the_frame_picked_on_its_scrubber(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    // The work area is frames 0–11 and the playhead is at 0.
    open_export_dialog(&workspace, visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.export_preview.drawn_frame(), Some(0), "the playhead's frame is drawn"));
    click_at_fraction(visual, "export-scrub", (0.999, 0.5));
    render(visual);
    visual.update(|_, cx| {
        let preview = &workspace.read(cx).ui.export_preview;
        assert_eq!((preview.frame, preview.drawn_frame()), (Some(11), Some(11)), "the end of the scrubber is the last exported frame");
    });
    visual.update(|window, cx| window.click("export-preview-back", cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.export_preview.drawn_frame(), Some(10), "the arrow steps back one frame"));
    // Exporting the whole composition widens what the scrubber covers.
    visual.update(|window, cx| window.click("export-whole", cx));
    render(visual);
    click_at_fraction(visual, "export-scrub", (0.999, 0.5));
    render(visual);
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        assert_eq!(ws.ui.export_preview.frame, Some(ws.state.composition().duration_frames - 1));
    });
}

#[gpui_kit::test]
fn a_gif_export_runs_at_the_chosen_frame_rate_without_sound(cx: &mut TestAppContext) {
    let folder = scratch_folder("export-gif");
    let (workspace, visual) = open(cx);
    open_export_dialog(&workspace, visual);
    visual.update(|window, cx| window.click("export-format", cx));
    render(visual);
    click_label(visual, "Animated GIF");
    render(visual);
    // GIF has fixed quality and no sound.
    assert!(visual.update(|window, _| window.try_find("export-quality")).is_none());
    visual.update(|window, cx| window.click("export-frame-rate", cx));
    render(visual);
    click_label(visual, "15 fps");
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.export_choice.frame_rate, Some(studio_core::time::FrameRate::new(15, 1))));
    visual.update(|window, cx| window.click("export-size", cx));
    render(visual);
    click_label(visual, "Quarter (480 × 270)");
    render(visual);
    click_label(visual, "Export…");
    visual.run_until_parked();
    visual.simulate_new_path_selection(|_| Some(folder.join("Loop")));
    visual.run_until_parked();
    render(visual);
    wait_for_export(&workspace, visual);

    let gif = std::fs::read(folder.join("Loop.gif")).expect("saved with the .gif extension");
    assert_eq!(&gif[..6], b"GIF89a");
    assert_eq!((u16::from_le_bytes([gif[6], gif[7]]), u16::from_le_bytes([gif[8], gif[9]])), (480, 270), "quarter size");
    std::fs::remove_dir_all(&folder).unwrap();
}

#[gpui_kit::test]
fn a_lottie_export_lists_what_it_left_out(cx: &mut TestAppContext) {
    let folder = scratch_folder("export-lottie");
    let (workspace, visual) = open(cx);
    open_export_dialog(&workspace, visual);
    visual.update(|window, cx| window.click("export-format", cx));
    render(visual);
    click_label(visual, "Lottie (.json)");
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.export_choice.format, studio_export::ExportFormat::Lottie));
    // Vector files have no size to choose.
    assert!(visual.update(|window, _| window.try_find("export-size")).is_none());
    click_label(visual, "Export…");
    visual.run_until_parked();
    visual.simulate_new_path_selection(|_| Some(folder.join("Main Comp")));
    visual.run_until_parked();
    render(visual);
    wait_for_export(&workspace, visual);
    render(visual);

    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(folder.join("Main Comp.json")).expect("saved with the .json extension")).unwrap();
    assert_eq!((json["w"].as_i64(), json["op"].as_f64()), (Some(1920), Some(12.0)), "the work area's twelve frames");
    // The sample has footage, which Lottie can't hold: the notice says so.
    visual.update(|window, cx| assert!(window.has_active_dialog(cx), "a notice lists what was left out"));
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

#[gpui_kit::test]
fn the_work_area_handles_set_what_plays(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    // At first the work area is the whole 240-frame comp, shown whole, so the handles sit at its ends.
    let start = center_of(visual, "work-start");
    let end = center_of(visual, "work-end");
    let quarter = (end.x - start.x) / 4.0;
    visual.update(|window, cx| window.drag(start, point(start.x + quarter, start.y), cx));
    render(visual);
    let work = visual.update(|_, cx| workspace.read(cx).state.work_area);
    assert!((work.start - 60.0).abs() <= 3.0 && work.end == 240.0, "{work:?}");
    let end = center_of(visual, "work-end");
    visual.update(|window, cx| window.drag(end, point(end.x - quarter, end.y), cx));
    render(visual);
    let work = visual.update(|_, cx| workspace.read(cx).state.work_area);
    assert!((work.end - 180.0).abs() <= 3.0, "{work:?}");
    // Playback starts inside it.
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.current_frame = 0));
    visual.update(|window, cx| window.click("tracks-play", cx));
    visual.update(|_, cx| assert!(workspace.read(cx).state.current_frame >= work.start as i64));
    visual.update(|window, cx| window.click("tracks-play", cx));
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
fn a_picture_dropped_on_a_placeholder_fills_it_and_another_swaps_it(cx: &mut TestAppContext) {
    use studio_core::model::{LayerContent, ShapeKind, ShapeStyle};
    let (workspace, visual) = open(cx);
    // A rounded "Drop your photo here" placeholder on top of the open composition.
    let placeholder = visual.update(|_, cx| {
        workspace.update(cx, |ws, cx| {
            let comp = ws.state.active_comp;
            let style = ShapeStyle { corner_radius: 24.0, ..ShapeStyle::new(ShapeKind::Rectangle) };
            let id = studio_core::edit::add_shape_layer(&mut ws.state.project, comp, style, Vec2::new(400.0, 300.0), Vec2::new(500.0, 400.0), 0).unwrap();
            ws.state.project.composition_mut(comp).unwrap().layer_mut(id).unwrap().name = studio_core::slots::placeholder_name("photo");
            cx.notify();
            id
        })
    });
    render(visual);
    let on_it = screen_point(&workspace, visual, Vec2::new(500.0, 400.0));
    // Held over the placeholder, a file outlines it; dragged back out of the window, the outline goes.
    visual.update(|window, cx| {
        use gpui_kit::{ExternalPaths, FileDropEvent, PlatformInput};
        window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Entered { position: on_it, paths: ExternalPaths(vec![fixture("still.png")].into()) }), cx);
        window.render_frame(cx);
        window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Pending { position: point(on_it.x + px(1.0), on_it.y) }), cx);
        window.render_frame(cx);
    });
    let outline = visual.update(|_, cx| workspace.read(cx).ui.slot_drop).expect("the placeholder is outlined");
    assert!((outline[0].x - 300.0).abs() < 0.5 && (outline[0].y - 250.0).abs() < 0.5, "outline starts at its top-left: {outline:?}");
    visual.update(|window, cx| {
        window.dispatch_event(gpui_kit::PlatformInput::FileDrop(gpui_kit::FileDropEvent::Exited), cx);
        window.render_frame(cx);
    });
    assert!(visual.update(|_, cx| workspace.read(cx).ui.slot_drop).is_none(), "nothing outlined off it");
    drop_files(visual, vec![fixture("still.png")], on_it);
    let inner = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        let layer = state.composition().layer(placeholder).expect("the layer stays");
        assert_eq!(state.selected_layers, vec![placeholder], "the filled layer is selected");
        match layer.content {
            LayerContent::Composition { composition } => composition,
            ref other => panic!("not filled: {other:?}"),
        }
    });
    let shows = |cx: &mut gpui_kit::VisualTestContext| {
        cx.update(|_, cx| {
            let project = &workspace.read(cx).state.project;
            let composition = project.composition(inner).unwrap();
            composition.layers.iter().filter(|layer| layer.switches.visible).filter_map(|layer| match layer.content {
                LayerContent::Image { item } => project.item(item).map(|item| item.name.clone()),
                _ => None,
            }).collect::<Vec<_>>()
        })
    };
    assert_eq!(shows(visual), vec!["still.png".to_owned()]);
    // A second picture, dragged from the Project panel onto the same spot, takes its place.
    let project_panel = center_of(visual, "project-items");
    drop_files(visual, vec![fixture("still.jpg")], project_panel);
    let jpg = visual.update(|_, cx| workspace.read(cx).state.project.items.iter().find(|item| item.name == "still.jpg").map(|item| item.id).unwrap());
    let item = center_of(visual, ("project-item", jpg.0 as usize));
    visual.update(|window, cx| window.drag(item, on_it, cx));
    render(visual);
    assert_eq!(shows(visual), vec!["still.jpg".to_owned()]);
    let layers = visual.update(|_, cx| workspace.read(cx).state.composition().layers.len());
    let before = studio_core::sample::sample_project().composition(studio_core::sample::ids::MAIN_COMP).unwrap().layers.len() + 1;
    assert_eq!(layers, before, "nothing else was added to the composition");
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
fn launching_shows_the_start_screen_or_reopens_the_last_project(cx: &mut TestAppContext) {
    use crate::app_settings::StartWith;
    let folder = scratch_folder("last-project");
    let (workspace, visual) = open(cx);
    make_an_edit(&workspace, visual);
    let path = save_as(visual, &folder, "Last.studio");
    // A fresh start: the sample project, then the session is restored.
    let relaunch = |visual: &mut VisualTestContext| {
        visual.update(|window, cx| workspace.update(cx, |ws, cx| {
            ws.state = crate::state::EditorState::new();
            ws.ui.start_screen = None;
            ws.restore_session(window, cx);
        }));
        render(visual);
    };
    // By default, the start screen, with the saved project first on its list.
    relaunch(visual);
    assert!(start_screen_showing(&workspace, visual));
    assert_eq!(crate::app_settings::load().recent_projects[0].path, path);
    visual.update(|_, cx| assert!(workspace.read(cx).state.file().is_none(), "nothing opened by itself"));

    // Set to reopen the last project, it opens straight away.
    crate::app_settings::update(|settings| settings.start_with = StartWith::LastProject);
    relaunch(visual);
    assert!(!start_screen_showing(&workspace, visual));
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.file(), Some(path.as_path())));
    assert_eq!(title_value(&workspace, visual, ROTATION, 0), PropertyValue::Scalar(33.0), "the saved edit is back");

    // If it has gone, the start screen shows instead.
    std::fs::remove_dir_all(&folder).unwrap();
    relaunch(visual);
    assert!(start_screen_showing(&workspace, visual));
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
        assert!(crate::panels::graph_editor::showing(ws), "the graph editor opened");
    });
    // An easing chosen there applies to exactly that segment.
    visual.update(|window, cx| window.click(ElementId::Name("easing:Expo:Both".into()), cx));
    render(visual);
    let easings = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().keyframe_easings(OPACITY));
    assert!(matches!(easings[1].1, Easing::Curve(_)), "36 → 156 is eased: {easings:?}");
    assert_eq!(easings[0].1, Easing::Linear, "24 → 36 is untouched");
}

#[gpui_kit::test]
fn a_fade_in_and_out_are_eased_one_segment_at_a_time(cx: &mut TestAppContext) {
    use studio_core::animation::EASING_FAMILIES;
    let (workspace, visual) = open_title_keyframes(cx);
    let expo = EASING_FAMILIES.iter().find(|(name, _)| *name == "Expo").unwrap().1;
    let quad = EASING_FAMILIES.iter().find(|(name, _)| *name == "Quad").unwrap().1;
    let easings = |visual: &mut VisualTestContext| -> Vec<Easing> {
        visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().keyframe_easings(OPACITY).into_iter().map(|(_, easing)| easing).collect())
    };
    let before = easings(visual);
    // Every opacity keyframe: a fade in (24 → 36), a hold at full (36 → 156) and a fade out.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_property_keyframes(TITLE, OPACITY, false);
        crate::panels::graph_editor::show(ws);
        cx.notify();
    }));
    render(visual);
    assert!(visual.update(|window, _| window.try_find(("easing-segment", 2_usize)).is_some()), "each segment is offered");
    // To start, only the fade in is eased.
    visual.update(|window, cx| window.click(ElementId::Name("easing:Expo:Out".into()), cx));
    render(visual);
    let after = easings(visual);
    assert_eq!(after[0], Easing::Curve(expo[1]));
    assert_eq!(&after[1..3], &before[1..3], "the hold and the fade out are untouched");
    // Picking the fade out eases just it.
    visual.update(|window, cx| window.click(("easing-segment", 2_usize), cx));
    render(visual);
    visual.update(|window, cx| window.click(ElementId::Name("easing:Expo:In".into()), cx));
    render(visual);
    let after = easings(visual);
    assert_eq!((after[0], after[1], after[2]), (Easing::Curve(expo[1]), before[1], Easing::Curve(expo[0])));
    // All eases every segment at once.
    visual.update(|window, cx| window.click("easing-segment-all", cx));
    render(visual);
    visual.update(|window, cx| window.click(ElementId::Name("easing:Quad:Both".into()), cx));
    render(visual);
    assert!(easings(visual)[..3].iter().all(|easing| *easing == Easing::Curve(quad[2])), "{:?}", easings(visual));
}

#[gpui_kit::test]
fn double_clicking_a_keyframe_opens_the_graph_editor(cx: &mut TestAppContext) {
    let (workspace, visual) = open_title_keyframes(cx);
    let key = ElementId::Name(format!("clip-kf:{}:{OPACITY:?}:36", TITLE.0).into());
    visual.update(|window, cx| window.click_with_options(key, gpui_kit::test::ClickOptions::new().with_count(2), cx));
    render(visual);
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        assert!(crate::panels::graph_editor::showing(ws));
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
    open_effect_category(visual, "Gaussian Blur");
    let from = center_of(visual, "effect:Gaussian Blur");
    let to = center_of(visual, ("clip", TITLE.0 as usize));
    visual.update(|window, cx| window.drag(from, to, cx));
    render(visual);
    let effects = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().effects.len());
    assert_eq!(effects, 1, "the blur was added to the title");
}

/// Unfolds the Effects panel category that lists `effect` (categories start folded).
fn open_effect_category(visual: &mut VisualTestContext, effect: &str) {
    let index = studio_core::effects::CATEGORIES
        .iter()
        .position(|category| category.effects.iter().any(|definition| definition.name == effect))
        .expect("the effect is in a category");
    visual.update(|window, cx| window.click(("effect-category", index), cx));
    render(visual);
}

#[gpui_kit::test]
fn the_plus_button_adds_an_effect_to_the_selected_clip(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.click("rail-effects", cx));
    render(visual);
    // With nothing selected there is nothing to apply to, so there is no + button.
    assert!(visual.update(|window, _| window.try_find("effect-apply:Sharpen")).is_none());
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.select_layer(TITLE, false); cx.notify(); }));
    // Categories start folded: unfold the one Sharpen is in.
    render(visual);
    open_effect_category(visual, "Sharpen");
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
    // Unfold every category, so whatever is offered is listed.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.ui.open_effect_categories.extend(studio_core::effects::CATEGORIES.iter().map(|category| category.name));
        cx.notify();
    }));
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
    assert!(visual.update(|window, _| window.try_find("effect:Gaussian Blur")).is_none(), "categories start folded");
    open_effect_category(visual, "Gaussian Blur");
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
    let (folder, snapping) = (bounds(visual, "new-folder"), bounds(visual, "viewer-snapping"));
    assert!((folder.center().y - snapping.center().y).abs() < px(1.0), "same row: {folder:?} vs {snapping:?}");
    // Playback has one set of controls, in the timeline.
    visual.update(|window, _| assert!(window.try_find("play").is_none() && window.try_find("tracks-play").is_some()));
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
fn the_type_tool_types_a_new_text_layer_where_clicked(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    // As in the app, Properties sections start folded, so typing can't rely on its Text card.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.ui.sections_start_open = false;
        ws.ui.tool = crate::workspace::Tool::Type;
        cx.notify();
    }));
    render(visual);
    let at = screen_point(&workspace, visual, Vec2::new(400.0, 300.0));
    click_at(visual, at);
    visual.run_until_parked();
    render(visual);
    // The field opens in the viewer, over where the text goes.
    let editing = visual.update(|_, cx| workspace.read(cx).ui.editing.as_ref().map(|editing| editing.in_viewer));
    assert_eq!(editing, Some(true));
    let layer = visual.update(|_, cx| workspace.read(cx).state.selected_layers[0]);
    let width = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(layer).unwrap().size.x);
    let empty_width = width(visual);
    visual.update(|window, cx| window.input("Hello", cx));
    render(visual);
    // Before Enter, the viewer already draws the typed text in the layer's own style (its box
    // grows to what was drawn), while the project's text is unchanged until it is committed.
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.text_of(layer).unwrap().0, ""));
    assert!(width(visual) > empty_width + 20.0, "drawn live: {} then {}", empty_width, width(visual));
    // Return starts a new line, as in After Effects, rather than finishing.
    let height = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(layer).unwrap().size.y);
    let one_line = height(visual);
    visual.update(|window, cx| window.press("enter", cx));
    visual.update(|window, cx| window.input("World", cx));
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).ui.editing.is_some(), "still typing"));
    assert!(height(visual) > one_line * 1.5, "the text stacks: {one_line} then {}", height(visual));
    // Escape finishes, keeping what was typed.
    visual.update(|window, cx| window.press("escape", cx));
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).ui.editing.is_none(), "Escape finished typing"));
    let typed = visual.update(|_, cx| workspace.read(cx).state.text_of(layer).map(|(text, _)| text.to_owned()));
    assert_eq!(typed.as_deref(), Some("Hello\nWorld"));
}

#[gpui_kit::test]
fn the_type_tool_edits_a_text_layer_it_clicks_with_the_caret_there(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    // The title starts at frame 24 and is centered in the frame.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.set_current_frame(40);
        ws.ui.tool = crate::workspace::Tool::Type;
        cx.notify();
    }));
    render(visual);
    let (before, layers) = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        (state.text_of(TITLE).unwrap().0.to_owned(), state.composition().layers.len())
    });
    let at = screen_point(&workspace, visual, Vec2::new(960.0, 540.0));
    click_at(visual, at);
    visual.run_until_parked();
    render(visual);
    let (editing, caret, layers_now) = visual.update(|_, cx| {
        let ws = workspace.read(cx);
        let editing = ws.ui.editing.as_ref().map(|editing| (editing.target, editing.in_viewer));
        let caret = ws.ui.editing.as_ref().map(|editing| editing.input.selected_range(cx));
        (editing, caret, ws.state.composition().layers.len())
    });
    assert_eq!(editing, Some((crate::workspace::EditTarget::SourceText(TITLE), true)), "it edits the title");
    assert_eq!(layers_now, layers, "rather than making a new text layer");
    let caret = caret.unwrap();
    assert!(caret.is_empty() && caret.start > 0 && caret.start < before.len(), "a caret inside the text, where clicked: {caret:?} in {before:?}");
    visual.update(|window, cx| window.input("X", cx));
    visual.update(|window, cx| window.press("cmd-enter", cx));
    render(visual);
    let after = visual.update(|_, cx| workspace.read(cx).state.text_of(TITLE).unwrap().0.to_owned());
    assert_eq!(after, format!("{}X{}", &before[..caret.start], &before[caret.start..]), "typed at the caret, keeping the rest");
}

#[gpui_kit::test]
fn text_being_typed_selects_by_double_click_and_drag_and_a_click_off_it_finishes(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.set_current_frame(40);
        ws.ui.tool = crate::workspace::Tool::Type;
        cx.notify();
    }));
    render(visual);
    let at = screen_point(&workspace, visual, Vec2::new(960.0, 540.0));
    click_at(visual, at);
    visual.run_until_parked();
    render(visual);
    let text = visual.update(|_, cx| workspace.read(cx).state.text_of(TITLE).unwrap().0.to_owned());
    let selected = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).ui.editing.as_ref().map(|editing| editing.input.selected_range(cx)));
    // A double-click on the text selects the word under it, and typing goes on.
    let offset = point(at.x - visual.update(|window, _| window.find("viewer").bounds().origin.x), at.y - visual.update(|window, _| window.find("viewer").bounds().origin.y));
    visual.update(|window, cx| window.click_with_options("viewer", gpui_kit::test::ClickOptions::new().with_count(2).with_offset(offset), cx));
    render(visual);
    assert_eq!(selected(visual), Some(0..text.len()), "the title is one word: {text:?}");
    // Dragging from the text's left edge to the middle selects that much.
    let (left, middle) = visual.update(|_, cx| {
        let comp = workspace.read(cx).state.composition();
        let corners = studio_core::transform::layer_corners(comp, TITLE, 40.0).unwrap();
        (corners[0] + (corners[3] - corners[0]) * 0.5, (corners[0] + corners[2]) * 0.5)
    });
    let (from, to) = (screen_point(&workspace, visual, left), screen_point(&workspace, visual, middle));
    visual.update(|window, cx| window.drag(from, to, cx));
    render(visual);
    let range = selected(visual).unwrap();
    assert!(range.start == 0 && range.end > 0 && range.end < text.len(), "part of the title: {range:?}");
    visual.update(|_, cx| assert!(workspace.read(cx).ui.editing.is_some(), "still typing"));
    // A click off the text (near the frame's corner) finishes the title; with the Type tool, as
    // in After Effects, it also starts a new text layer there.
    let corner = screen_point(&workspace, visual, Vec2::new(40.0, 40.0));
    click_at(visual, corner);
    visual.run_until_parked();
    render(visual);
    let target = visual.update(|_, cx| workspace.read(cx).ui.editing.as_ref().map(|editing| editing.target));
    assert!(target.is_some_and(|target| target != crate::workspace::EditTarget::SourceText(TITLE)), "the title is finished: {target:?}");
}

#[gpui_kit::test]
fn text_animation_comes_in_by_the_chosen_piece_and_can_be_tuned(cx: &mut TestAppContext) {
    use studio_core::presets::Phase;
    use studio_core::text_animation::{TextMotion, TextUnit};
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        ws.ui.tabs.insert(crate::workspace::PanelId::Properties, crate::panels::motion::TAB);
        cx.notify();
    }));
    render(visual);
    let animator = |visual: &mut VisualTestContext, phase| visual.update(|_, cx| workspace.read(cx).state.text_animator(TITLE, phase));
    // The heading's + adds an In animation: rising by character to start.
    visual.update(|window, cx| window.click("text-animation-add", cx));
    render(visual);
    click_label(visual, "Animate In");
    render(visual);
    assert_eq!(animator(visual, Phase::In).map(|a| (a.motion, a.unit)), Some((TextMotion::Rise, TextUnit::Character)));
    // By word, popping in.
    visual.update(|window, cx| window.click(ElementId::Name("text-animation-in-unit-Word".into()), cx));
    render(visual);
    visual.update(|window, cx| window.click(ElementId::Name("text-animation-in-motion".into()), cx));
    render(visual);
    click_label(visual, "Pop");
    render(visual);
    assert_eq!(animator(visual, Phase::In).map(|a| (a.motion, a.unit)), Some((TextMotion::Pop, TextUnit::Word)));
    // Its duration is typed like any value.
    visual.update(|window, cx| window.click(field_id(FieldTarget::TextAnimationSeconds { layer: TITLE, phase: Phase::In }), cx));
    render(visual);
    visual.update(|window, cx| {
        window.input("2.5", cx);
        window.press("enter", cx);
    });
    render(visual);
    assert_eq!(animator(visual, Phase::In).map(|a| a.seconds), Some(2.5));
    // The Out animation is separate; Remove takes the In one off.
    visual.update(|window, cx| window.click("text-animation-add", cx));
    render(visual);
    click_label(visual, "Animate Out");
    render(visual);
    assert!(animator(visual, Phase::Out).is_some());
    visual.update(|window, cx| window.click(ElementId::Name("text-animation-in-remove".into()), cx));
    render(visual);
    assert_eq!(animator(visual, Phase::In), None);
    assert!(animator(visual, Phase::Out).is_some(), "the Out one stays");
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
    visual.update(|window, cx| window.press("cmd-enter", cx));
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
    open_effect_category(visual, "Gaussian Blur");
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
    visual.update(|window, cx| window.click(("settings-section", 7_usize), cx));
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

#[gpui_kit::test]
fn caption_timing_can_be_synced_and_each_phrase_retimed(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let layer = add_test_captions(&workspace, visual);
    let type_into = |visual: &mut VisualTestContext, target: FieldTarget, value: &str| {
        visual.update(|window, cx| window.click(field_id(target), cx));
        render(visual);
        visual.update(|window, cx| {
            window.press("cmd-a", cx);
            window.input(value, cx);
            window.press("enter", cx);
        });
        render(visual);
    };
    let phrase = |visual: &mut VisualTestContext, start| visual.update(|_, cx| workspace.read(cx).state.caption_phrase_frame(layer, 0..3, start).unwrap());
    let first_shown = phrase(visual, true);
    // Sync: every caption shows 12 frames later; the words keep their own times.
    type_into(visual, FieldTarget::CaptionSync { layer }, "12");
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.caption_style(layer).unwrap().offset, 12));
    assert_eq!(phrase(visual, true), first_shown + 12, "the phrase now shows later");
    // A new look keeps the sync.
    visual.update(|_, cx| workspace.update(cx, |ws, _| {
        let look = studio_core::captions::presets()[0].1.clone();
        ws.state.edit_captions(layer, |current| *current = studio_core::captions::CaptionStyle { offset: current.offset, ..look });
    }));
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.caption_style(layer).unwrap().offset, 12));
    // The phrase's end is typed in seconds of the composition; its words squeeze to fit.
    let fps = visual.update(|_, cx| workspace.read(cx).state.composition().frame_rate.fps());
    let end_frame = phrase(visual, true) + 20;
    type_into(visual, FieldTarget::CaptionPhrase { layer, first: 0, end: 3, start: false }, &format!("{:.4}", end_frame as f64 / fps));
    assert_eq!(phrase(visual, false), end_frame);
    let words = visual.update(|_, cx| workspace.read(cx).state.caption_words(layer).unwrap().to_vec());
    assert!(words.windows(2).all(|pair| pair[0].end <= pair[1].start) && words.iter().all(|word| word.end > word.start), "{words:?}");
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
fn a_nested_composition_with_sound_can_be_muted_like_a_clip(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    add_video_with_sound(&workspace, visual);
    let nested = visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let layer = ws.state.precompose_selection("With Sound", false).unwrap();
        cx.notify();
        layer
    }));
    render(visual);
    let heard = |visual: &mut VisualTestContext| visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        studio_core::audio::composition_audio(&state.project, state.composition()).len()
    });
    assert_eq!(heard(visual), 1, "the video's sound plays from inside the nested composition");
    // Its clip has a mute button, and its Audio card the switch, volume and fades, but not the
    // tools that work on one file's sound.
    assert!(visual.update(|window, _| window.try_find(("clip-mute", nested.0 as usize))).is_some());
    assert!(visual.update(|window, _| window.try_find("remove-silences")).is_none());
    visual.update(|window, cx| window.click("layer-play-audio", cx));
    render(visual);
    assert_eq!(heard(visual), 0, "muted");
    visual.update(|window, cx| window.click(("clip-mute", nested.0 as usize), cx));
    render(visual);
    assert_eq!(heard(visual), 1, "and back on from its clip");
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
fn painting_an_object_matte_lays_strokes_on_the_picture_and_asks_before_downloading(cx: &mut TestAppContext) {
    use crate::panels::fields::{FieldTarget, MatteEdgeSetting};
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
    let strokes = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(layer).unwrap().cutout.clone().unwrap().strokes);
    // A stroke dragged across the object (the image's top-left quarter). The model is not on this
    // (test) computer, so Studio asks before downloading it, and nothing runs until agreed.
    let (from, to) = (screen_point(&workspace, visual, Vec2::new(860.0, 490.0)), screen_point(&workspace, visual, Vec2::new(920.0, 520.0)));
    visual.update(|window, cx| window.drag(from, to, cx));
    render(visual);
    assert!(visual.update(|window, _| window.try_find_by_label("Download and Continue")).is_some());
    assert!(visual.update(|_, cx| workspace.read(cx).ui.cutout_job.is_none()));
    click_label(visual, "Cancel");
    render(visual);
    let painted = strokes(visual);
    assert_eq!(painted.len(), 1);
    assert!(painted[0].add && painted[0].points.len() >= 2, "{painted:?}");
    assert!((painted[0].points[0].x - 0.25).abs() < 0.01 && (painted[0].points[0].y - 0.25).abs() < 0.01, "{painted:?}");

    // The brush can be made bigger; the next stroke is wider.
    visual.update(|window, cx| window.click("viewer-brush-bigger", cx));
    render(visual);
    // Then an Option-press (a dot) on something that isn't part of the object.
    let leave_out = screen_point(&workspace, visual, Vec2::new(1060.0, 590.0));
    visual.update(|window, cx| {
        window.change_modifiers(gpui_kit::Modifiers::alt(), cx);
        window.pointer_down(leave_out, MouseButton::Left, cx);
        window.pointer_up(leave_out, MouseButton::Left, cx);
        window.change_modifiers(gpui_kit::Modifiers::default(), cx);
    });
    render(visual);
    if visual.update(|window, _| window.try_find_by_label("Cancel")).is_some() {
        click_label(visual, "Cancel");
        render(visual);
    }
    let painted = strokes(visual);
    assert_eq!(painted.iter().map(|stroke| stroke.add).collect::<Vec<_>>(), vec![true, false]);
    assert!(painted[1].radius > painted[0].radius, "the bigger brush: {} vs {}", painted[1].radius, painted[0].radius);

    // Refine Edge settings take typed values.
    let feather = FieldTarget::MatteEdge { layer, setting: MatteEdgeSetting::Feather };
    visual.update(|window, cx| window.click(field_id(feather), cx));
    render(visual);
    visual.update(|window, cx| {
        window.input("12", cx);
        window.press("enter", cx);
    });
    render(visual);
    let edges = visual.update(|_, cx| workspace.read(cx).state.composition().layer(layer).unwrap().cutout.clone().unwrap().edges);
    assert_eq!(edges.feather, 12.0);

    // Clear Frame takes this frame's strokes away again.
    visual.update(|window, cx| window.click("cutout-clear-frame", cx));
    render(visual);
    assert!(strokes(visual).is_empty());
}

#[gpui_kit::test]
fn a_keyed_layer_can_be_viewed_as_its_selection_or_its_matte(cx: &mut TestAppContext) {
    use studio_core::key_view::KeyView;
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
    let find = |visual: &mut gpui_kit::VisualTestContext, id: &'static str| visual.update(|window, _| window.try_find(id)).is_some();
    let view = |visual: &mut gpui_kit::VisualTestContext| visual.update(|_, cx| workspace.read(cx).ui.key_view);
    assert!(!find(visual, "viewer-key-selection"), "nothing is keyed yet, so there is nothing to view");
    // Picking starts in the Selection view, with a hint on how to pick.
    visual.update(|window, cx| window.click("cutout-add", cx));
    render(visual);
    assert_eq!(view(visual), Some(KeyView::Selection));
    assert!(find(visual, "viewer-picking-hint"));
    assert!(!find(visual, "viewer-key-legend"), "no strokes yet, so nothing is selected to show");
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.add_matte_stroke(layer, &[Vec2::new(860.0, 490.0)], 10.0, true);
        cx.notify();
    }));
    render(visual);
    assert!(find(visual, "viewer-key-legend") && find(visual, "viewer-key-matte"));
    // Done stops picking; the view stays, as the layer is still selected.
    visual.update(|window, cx| window.click("viewer-picking-done", cx));
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).ui.cutout_pick), None);
    assert!(!find(visual, "viewer-picking-hint") && find(visual, "viewer-key-legend"));
    visual.update(|window, cx| window.click("viewer-key-matte", cx));
    render(visual);
    assert_eq!(view(visual), Some(KeyView::Matte));
    visual.update(|window, cx| window.click("viewer-key-result", cx));
    render(visual);
    assert_eq!(view(visual), None);
    assert!(!find(visual, "viewer-key-legend"), "the plain result needs no legend");
    // A green screen key counts too.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.clear_cutout(layer);
        let comp = ws.state.active_comp;
        studio_core::edit::add_effect(&mut ws.state.project, comp, layer, "Keyer").unwrap();
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click("viewer-key-selection", cx));
    render(visual);
    assert_eq!(view(visual), Some(KeyView::Selection));
    assert!(find(visual, "viewer-key-legend"));
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
    visual.update(|window, cx| window.click(("settings-section", 5_usize), cx));
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
    visual.update(|window, cx| window.click(("gallery-template-add", first_intro), cx));
    render(visual);
    assert!(visual.update(|window, _| window.try_find("template-gallery")).is_none(), "adding one closes it");
    visual.update(|_, cx| {
        let composition = workspace.read(cx).state.composition();
        assert!(composition.name.starts_with(TEMPLATES[first_intro].name));
        assert_eq!((composition.width, composition.height), (1920, 1080));
    });
}

#[gpui_kit::test]
fn the_colors_menu_makes_a_template_in_another_palette(cx: &mut TestAppContext) {
    use crate::state::TemplateLook;
    use studio_core::model::{LayerContent, Paint};
    use studio_core::templates::LOOKS;
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.click("rail-templates", cx));
    render(visual);
    visual.update(|window, cx| window.click("template-brand", cx));
    render(visual);
    click_label(visual, "Cobalt");
    render(visual);
    let cobalt = LOOKS.iter().position(|(name, _)| *name == "Cobalt").unwrap();
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.template_look, TemplateLook::Palette(cobalt)));
    // The template is made in Cobalt's colors: its background starts from Cobalt's.
    visual.update(|window, cx| window.click(("template-add", 0_usize), cx));
    render(visual);
    visual.update(|_, cx| {
        let composition = workspace.read(cx).state.composition();
        let background = composition.layers.iter().find(|layer| layer.name == "Background").expect("a background");
        let LayerContent::Shape { style, .. } = &background.content else { panic!("a shape") };
        let Some(Paint::Linear(gradient)) = &style.fill else { panic!("a gradient: {:?}", style.fill) };
        assert_eq!(gradient.stops()[0].color, studio_core::model::Rgba::hex(LOOKS[cobalt].1.background));
    });
}

#[gpui_kit::test]
fn hovering_a_template_plays_it_and_only_its_plus_adds_it(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|window, cx| window.click("rail-templates", cx));
    render(visual);
    let compositions = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.project.compositions.len());
    let before = compositions(visual);
    // Clicking the card itself adds nothing.
    visual.update(|window, cx| window.click(("template", 0_usize), cx));
    render(visual);
    assert_eq!(compositions(visual), before, "only the + adds a template");
    // Hovering starts it playing on its card; moving away stops it.
    visual.update(|window, cx| window.hover(("template", 0_usize), cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.template_preview.as_ref().map(|preview| preview.index), Some(0)));
    visual.update(|window, cx| window.hover("template-brand", cx));
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).ui.template_preview.is_none()));
    // The + adds it and says so.
    visual.update(|window, cx| window.click(("template-add", 0_usize), cx));
    render(visual);
    assert_eq!(compositions(visual), before + 1);
    visual.update(|_, cx| assert!(workspace.read(cx).state.composition().name.starts_with(studio_core::templates::TEMPLATES[0].name)));
}

#[gpui_kit::test]
fn a_template_click_adds_and_opens_it_in_the_chosen_shape(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.click("rail-templates", cx));
    render(visual);
    // 4:5, then the first template.
    visual.update(|window, cx| window.click(("template-format", 1_usize), cx));
    render(visual);
    visual.update(|window, cx| window.click(("template-add", 0_usize), cx));
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
        let target = crate::panels::fields::FieldTarget::Property { layer: clip, path: studio_core::property::PropertyPath::Speed, component: 0 };
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
        ws.ui.tabs.insert(crate::workspace::PanelId::Properties, crate::panels::motion::TAB);
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
    // Track Motion's + (on the Properties tab) starts picking the point to follow.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.ui.tabs.insert(crate::workspace::PanelId::Properties, 0);
        cx.notify();
    }));
    render(visual);
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
    click_in_submenu(visual, "Transform", "Flip Horizontal");
    render(visual);
    assert_eq!(scale(visual), PropertyValue::Pair(studio_core::animation::Vec2::new(-before.x, before.y)), "mirrored left to right");
    visual.update(|window, cx| window.right_click(("clip-body", TITLE.0 as usize), cx));
    render(visual);
    click_in_submenu(visual, "Transform", "Reset Transform");
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
    // Off by default; the switch turns it on, and that is remembered.
    assert!(visual.update(|_, cx| workspace.read(cx).ui.settings.show_render_time));
    assert!(crate::app_settings::load().show_render_time, "remembered");
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
        ws.ui.tabs.insert(crate::workspace::PanelId::Properties, crate::panels::motion::TAB);
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
    visual.update(|window, cx| window.click("transitions-category", cx));
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
fn duplicate_and_resize_copies_a_composition_to_another_shape_beside_it(cx: &mut TestAppContext) {
    use studio_core::sample::ids;
    let (workspace, visual) = open(cx);
    // A 16:9 composition named for its shape, kept in a folder.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.rename_item(ids::MAIN_COMP, "Promo 16:9");
        ws.state.move_item(ids::MAIN_COMP, Some(ids::SOLIDS_FOLDER));
        if !ws.ui.folder_open(ids::SOLIDS_FOLDER) {
            ws.ui.toggle_folder(ids::SOLIDS_FOLDER);
        }
        cx.notify();
    }));
    render(visual);
    let source = visual.update(|_, cx| {
        let comp = workspace.read(cx).state.project.composition(ids::MAIN_COMP).unwrap();
        (comp.width, comp.height, comp.layers.len())
    });
    assert_eq!((source.0, source.1), (1920, 1080), "the sample is 16:9");
    visual.update(|window, cx| window.right_click(("project-item", ids::MAIN_COMP.0 as usize), cx));
    render(visual);
    click_label(visual, "Duplicate and Resize");
    render(visual);
    click_label(visual, "Vertical 9:16 · Reels, TikTok, Shorts  1080×1920");
    render(visual);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        let copy = state.composition();
        assert_ne!(copy.id, ids::MAIN_COMP, "the copy is open");
        assert_eq!((copy.width, copy.height), (1080, 1920));
        assert_eq!(copy.name, "Promo 9:16", "named for its new shape, not Promo 16:9 9:16");
        assert_eq!(copy.layers.len(), source.2, "every layer came along");
        assert_eq!(state.project.item(copy.id).unwrap().parent, Some(ids::SOLIDS_FOLDER), "beside the original");
        assert_eq!(state.project.composition(ids::MAIN_COMP).unwrap().width, 1920, "the original is unchanged");
    });
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
    click_in_submenu(visual, "Export", "Export and Replace Layer");
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
    // AI Models is the ninth page.
    visual.update(|window, cx| window.click(("settings-section", 8_usize), cx));
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
fn the_color_tab_grades_an_adjustment_layer_over_the_whole_video(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|window, cx| window.click("tab:Properties:2", cx));
    render(visual);
    // With nothing selected there is nothing to grade, so no corrections are offered.
    visual.update(|window, _| assert!(window.try_find("grade-add:grade-basic").is_none()));
    // The whole video is graded by grading an adjustment layer over it.
    let grade = visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let grade = ws.state.add_adjustment_layer().unwrap();
        cx.notify();
        grade
    }));
    render(visual);
    // Every correction is a card; one the layer doesn't have yet has a + to add it.
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.effect_named(grade, "Basic Correction"), None));
    visual.update(|window, cx| window.click("grade-add:grade-basic", cx));
    render(visual);
    visual.update(|window, _| assert!(window.try_find("grade-add:grade-basic").is_none(), "added, so its + gives way to its own buttons"));
    let basic = visual.update(|_, cx| workspace.read(cx).state.effect_named(grade, "Basic Correction").unwrap());
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
    // The LUT card has its own buttons now; the corrections not added yet still offer a +.
    visual.update(|window, _| {
        assert!(window.try_find("grade-add:grade-lut").is_none());
        assert!(window.try_find("grade-add:grade-basic").is_some());
    });
    // A correction not added yet starts folded, and unfolds like any card (to a note on how to add it).
    let unfolded = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).ui.toggled_sections.contains("grade-levels"));
    assert!(!unfolded(visual));
    visual.update(|window, cx| window.click("grade-levels-heading", cx));
    render(visual);
    assert!(unfolded(visual));
    visual.update(|window, cx| window.click("grade-add:grade-levels", cx));
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
        // Categories start folded; unfold the LUTs.
        ws.ui.open_effect_categories.insert("LUTs");
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
        // Categories start folded; unfold the LUTs.
        ws.ui.open_effect_categories.insert("LUTs");
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
    click_label(visual, "Null Object (⌥⇧⌘Y)");
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
    // The viewer's button opens the guides menu, which can hide them again.
    visual.update(|window, cx| window.click("viewer-guides", cx));
    render(visual);
    click_label(visual, "Show Guides (')");
    render(visual);
    assert!(!guides(visual));
}

#[gpui_kit::test]
fn the_guides_menu_offers_title_safe_on_wide_frames_and_social_zones_on_tall_ones(cx: &mut TestAppContext) {
    use crate::panels::guides::SocialApp;
    let (workspace, visual) = open(cx);
    let has = |visual: &mut VisualTestContext, id: &'static str| visual.update(|window, _| window.try_find(id).is_some());
    let has_label = |visual: &mut VisualTestContext, label: &str| visual.update(|window, _| window.try_find_by_label(label).is_some());
    let open_menu = |visual: &mut VisualTestContext| {
        visual.update(|window, cx| window.click("viewer-guides", cx));
        render(visual);
    };
    let close_menu = |visual: &mut VisualTestContext| {
        visual.update(|window, cx| window.press("escape", cx));
        render(visual);
    };
    let set_size = |visual: &mut VisualTestContext, width: u32, height: u32| {
        visual.update(|_, cx| workspace.update(cx, |ws, _| {
            let composition = ws.state.composition_mut_for_test();
            (composition.width, composition.height) = (width, height);
        }));
        render(visual);
    };

    // 16:9: title safe is offered (and on by default); the social zones are not.
    set_size(visual, 1920, 1080);
    open_menu(visual);
    assert!(has_label(visual, "Title and Action Safe"));
    assert!(!has_label(visual, "TikTok"));
    // While hidden, picking a guide shows the guides with it on rather than turning it off.
    click_label(visual, "Title and Action Safe");
    render(visual);
    assert!(has(visual, "viewer-guides-overlay"), "picking a guide shows the guides");
    assert!(has(visual, "guide-title-safe") && has(visual, "guide-action-safe"));
    // Once shown, picking it again turns it off.
    open_menu(visual);
    click_label(visual, "Title and Action Safe");
    render(visual);
    assert!(!has(visual, "guide-title-safe"));
    open_menu(visual);
    click_label(visual, "Title and Action Safe");
    render(visual);
    assert!(has(visual, "guide-title-safe"));

    // 9:16: the social zones are offered instead, and title safe is neither offered nor drawn.
    set_size(visual, 1080, 1920);
    assert!(!has(visual, "guide-title-safe"));
    open_menu(visual);
    assert!(!has_label(visual, "Title and Action Safe"));
    assert!(has_label(visual, "Social Safe Zones"));
    click_label(visual, "TikTok");
    render(visual);
    assert!(has(visual, "guide-zone-tiktok"));
    open_menu(visual);
    click_label(visual, "Instagram Reels");
    render(visual);
    assert!(has(visual, "guide-zone-tiktok") && has(visual, "guide-zone-instagram-reels"));
    assert!(!has(visual, "guide-zone-youtube-shorts"));
    // The Reels box sits 14% in from the top, 35% from the bottom and 6% from each side.
    let (frame, zone) = visual.update(|window, _| (window.find("viewer-guides-overlay").bounds(), window.find("guide-zone-instagram-reels").bounds()));
    let near = |actual: gpui_kit::Pixels, expected: gpui_kit::Pixels| (actual - expected).abs() <= px(1.0);
    assert!(near(zone.top() - frame.top(), frame.size.height * 0.14), "{zone:?} in {frame:?}");
    assert!(near(frame.bottom() - zone.bottom(), frame.size.height * 0.35), "{zone:?} in {frame:?}");
    assert!(near(zone.left() - frame.left(), frame.size.width * 0.06), "{zone:?} in {frame:?}");
    assert!(near(frame.right() - zone.right(), frame.size.width * 0.06), "{zone:?} in {frame:?}");
    assert_eq!(visual.update(|_, cx| workspace.read(cx).ui.guide_choice.social.clone()), [SocialApp::TikTok, SocialApp::InstagramReels]);

    // Square frames only get the grid; the social zones stay chosen for the next tall comp.
    set_size(visual, 1080, 1080);
    assert!(!has(visual, "guide-zone-tiktok"));
    open_menu(visual);
    assert!(!has_label(visual, "TikTok") && !has_label(visual, "Title and Action Safe"));
    close_menu(visual);
    set_size(visual, 1080, 1920);
    assert!(has(visual, "guide-zone-tiktok"));
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

// Start screen, recent projects and the standard Mac menu commands ------------------------------

fn start_screen_showing(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> bool {
    let showing = visual.update(|window, _| window.try_find("start-screen").is_some());
    assert_eq!(showing, visual.update(|_, cx| workspace.read(cx).ui.start_screen.is_some()));
    showing
}

#[gpui_kit::test]
fn the_start_screen_opens_recent_projects_and_starts_new_ones(cx: &mut TestAppContext) {
    use crate::workspace::ShowStartScreen;
    let folder = scratch_folder("start-screen");
    let (workspace, visual) = open(cx);
    // A saved project, as if opened before: it is listed, newest first.
    make_an_edit(&workspace, visual);
    let file = folder.join("Launch Promo.studio");
    let text = visual.update(|_, cx| workspace.read(cx).state.project_text().unwrap());
    std::fs::write(&file, text).unwrap();
    crate::app_settings::note_project_used(&file);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.mark_saved(std::path::PathBuf::from("/elsewhere/Current.studio"), false);
        cx.notify();
    }));

    visual.update(|window, cx| window.dispatch_action(Box::new(ShowStartScreen), cx));
    render(visual);
    assert!(start_screen_showing(&workspace, visual));
    assert!(visual.update(|window, _| window.try_find(("start-recent-row", 0usize)).is_some()), "the recent project is listed");
    assert!(visual.update(|window, _| window.try_find(("start-recent-row", 1usize)).is_none()), "and only it");
    // Escape goes to the editor, and File ▸ Start Screen comes back.
    visual.update(|window, cx| window.press("escape", cx));
    render(visual);
    assert!(!start_screen_showing(&workspace, visual));
    visual.update(|window, cx| window.dispatch_action(Box::new(ShowStartScreen), cx));
    render(visual);

    // Clicking it opens it and goes to the editor.
    visual.update(|window, cx| window.click(("start-recent-row", 0usize), cx));
    visual.run_until_parked();
    render(visual);
    assert!(!start_screen_showing(&workspace, visual));
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.file(), Some(file.as_path())));
    assert_eq!(title_value(&workspace, visual, ROTATION, 0), PropertyValue::Scalar(33.0));

    // New Project starts an empty one.
    visual.update(|window, cx| window.dispatch_action(Box::new(ShowStartScreen), cx));
    render(visual);
    visual.update(|window, cx| window.click("start-new", cx));
    visual.run_until_parked();
    render(visual);
    assert!(!start_screen_showing(&workspace, visual));
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        assert!(state.file().is_none());
        assert_eq!(state.project.compositions.len(), 1);
        assert!(state.composition().layers.is_empty(), "a new, empty project");
    });

    // A project that has gone missing is taken off the list when clicked.
    std::fs::remove_file(&file).unwrap();
    visual.update(|window, cx| window.dispatch_action(Box::new(ShowStartScreen), cx));
    render(visual);
    assert!(visual.update(|window, _| window.try_find(("start-recent-row", 0usize)).is_none()), "missing files aren't listed");
    std::fs::remove_dir_all(&folder).unwrap();
}

#[gpui_kit::test]
fn shortcuts_leave_the_project_alone_while_the_start_screen_covers_it(cx: &mut TestAppContext) {
    use crate::workspace::ShowStartScreen;
    let (workspace, visual) = open(cx);
    let layers = visual.update(|_, cx| workspace.update(cx, |ws, _| {
        let first = ws.state.composition().layers[0].id;
        ws.state.selected_layers = vec![first];
        ws.state.composition().layers.len()
    }));
    visual.update(|window, cx| window.dispatch_action(Box::new(ShowStartScreen), cx));
    render(visual);
    assert!(start_screen_showing(&workspace, visual));
    // Delete, Space and Duplicate, by key and as menu commands, reach nothing behind it.
    for key in ["backspace", "space", "cmd-d"] {
        visual.update(|window, cx| window.press(key, cx));
        render(visual);
    }
    visual.update(|window, cx| window.dispatch_action(Box::new(crate::workspace::Delete), cx));
    render(visual);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        assert_eq!(state.composition().layers.len(), layers, "no layer deleted or duplicated");
        assert!(!state.is_playing(), "playback didn't start");
        assert!(!state.has_unsaved_changes());
    });
    // Back in the editor, Delete works again.
    visual.update(|window, cx| window.press("escape", cx));
    render(visual);
    visual.update(|window, cx| window.press("backspace", cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.composition().layers.len(), layers - 1));
}

#[gpui_kit::test]
fn notifications_show_over_the_start_screen_and_can_be_clicked(cx: &mut TestAppContext) {
    use crate::workspace::ShowStartScreen;
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    use gpui_kit::component::notification::Notification;
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.dispatch_action(Box::new(ShowStartScreen), cx));
    render(visual);
    assert!(start_screen_showing(&workspace, visual));
    // A message with a button, as a finished package or export posts one.
    let clicked = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = clicked.clone();
    visual.update(|window, cx| {
        let note = Notification::success("Done").autohide(false).action(move |_, _, _| {
            let seen = seen.clone();
            Button::new("note-button").ghost().label("Show").on_click(move |_, _, _| seen.set(true))
        });
        window.push_notification(note, cx);
    });
    // Let it finish sliding in: the slide (0.4 s) runs on the real clock, not the test's.
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        render(visual);
    }
    visual.update(|window, cx| window.click("note-button", cx));
    render(visual);
    assert!(clicked.get(), "the button above the start screen took the click");
}

#[gpui_kit::test]
fn a_template_on_the_start_screen_becomes_a_new_project(cx: &mut TestAppContext) {
    use crate::workspace::ShowStartScreen;
    use studio_core::templates::TEMPLATES;
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.dispatch_action(Box::new(ShowStartScreen), cx));
    render(visual);
    visual.update(|window, cx| window.click("start-templates", cx));
    render(visual);
    assert!(visual.update(|window, _| window.try_find(("start-template", 0usize)).is_some()), "every template is offered");
    // The sample project has unsaved changes: Don't Save goes on.
    make_an_edit(&workspace, visual);
    render(visual);
    visual.update(|window, cx| window.click(("start-template-add", 3usize), cx));
    visual.run_until_parked();
    visual.simulate_prompt_answer("Don’t Save");
    visual.run_until_parked();
    render(visual);
    assert!(!start_screen_showing(&workspace, visual));
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        assert_eq!(state.project.compositions.len(), 1, "the template's composition in place of an empty one");
        assert!(state.composition().name.contains(TEMPLATES[3].name), "{}", state.composition().name);
        assert!(!state.composition().layers.is_empty());
        assert!(!state.can_undo(), "the template is where the project starts");
    });
}

#[gpui_kit::test]
fn clicking_studio_offers_to_save_then_shows_the_start_screen(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    render(visual);
    // Nothing unsaved: straight to the start screen, and Esc goes back to the same project.
    visual.update(|window, cx| window.click("studio-home", cx));
    visual.run_until_parked();
    render(visual);
    assert!(!visual.has_pending_prompt());
    assert!(start_screen_showing(&workspace, visual));
    visual.update(|window, cx| window.press("escape", cx));
    render(visual);
    assert!(!start_screen_showing(&workspace, visual));

    // Unsaved changes: Cancel stays in the editor with them.
    make_an_edit(&workspace, visual);
    render(visual);
    visual.update(|window, cx| window.click("studio-home", cx));
    visual.run_until_parked();
    assert!(visual.has_pending_prompt(), "asks to save first");
    visual.simulate_prompt_answer("Cancel");
    visual.run_until_parked();
    render(visual);
    assert!(!start_screen_showing(&workspace, visual));
    assert!(has_unsaved_changes(&workspace, visual));

    // Don't Save shows the start screen; Esc goes back to the work, still unsaved.
    let dont_save_to_start_screen = |visual: &mut VisualTestContext| {
        visual.update(|window, cx| window.click("studio-home", cx));
        visual.run_until_parked();
        assert!(visual.has_pending_prompt(), "asks every time it goes there");
        visual.simulate_prompt_answer("Don’t Save");
        visual.run_until_parked();
        render(visual);
    };
    dont_save_to_start_screen(visual);
    assert!(start_screen_showing(&workspace, visual));
    visual.update(|window, cx| window.press("escape", cx));
    render(visual);
    assert!(!start_screen_showing(&workspace, visual));
    assert!(has_unsaved_changes(&workspace, visual));

    // Starting a project from the start screen doesn't ask a second time.
    dont_save_to_start_screen(visual);
    visual.update(|window, cx| window.click("start-new", cx));
    visual.run_until_parked();
    render(visual);
    assert!(!visual.has_pending_prompt(), "it was already asked");
    assert!(!start_screen_showing(&workspace, visual));
    visual.update(|_, cx| assert!(workspace.read(cx).state.composition().layers.is_empty(), "a new, empty project"));
    assert!(!has_unsaved_changes(&workspace, visual));
}

#[gpui_kit::test]
fn clicking_a_project_items_label_swatch_picks_its_color(cx: &mut TestAppContext) {
    use studio_core::model::LabelColor;
    use studio_core::sample::ids::LOGO;
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.ui.project_columns.label = true;
        cx.notify();
    }));
    render(visual);
    let label = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.project.item(LOGO).unwrap().label);
    let other = LabelColor::ALL.into_iter().find(|color| *color != label(visual)).unwrap();
    let selected_before = visual.update(|_, cx| workspace.read(cx).state.selected_item);
    visual.update(|window, cx| window.click(("project-label", LOGO.0 as usize), cx));
    render(visual);
    click_label(visual, other.name());
    render(visual);
    assert_eq!(label(visual), other);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.selected_item, selected_before, "the press didn't select the row"));
}

#[gpui_kit::test]
fn close_project_goes_back_to_the_start_screen(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.press("cmd-w", cx));
    visual.run_until_parked();
    render(visual);
    assert!(start_screen_showing(&workspace, visual));
    visual.update(|_, cx| assert!(workspace.read(cx).state.composition().layers.is_empty(), "the project was closed"));
}

#[gpui_kit::test]
fn open_recent_lists_projects_and_opens_them(cx: &mut TestAppContext) {
    use crate::workspace::{ClearRecentProjects, OpenRecent};
    let folder = scratch_folder("open-recent");
    let (workspace, visual) = open(cx);
    make_an_edit(&workspace, visual);
    let file = folder.join("Teaser.studio");
    let text = visual.update(|_, cx| workspace.read(cx).state.project_text().unwrap());
    std::fs::write(&file, text).unwrap();
    crate::app_settings::note_project_used(&file);
    let open_recent_names = || {
        let menus = crate::workspace::menus();
        let file_menu = menus.into_iter().find(|menu| menu.name == "File").unwrap();
        file_menu
            .items
            .into_iter()
            .find_map(|item| match item {
                gpui_kit::MenuItem::Submenu(menu) if menu.name == "Open Recent" => Some(menu),
                _ => None,
            })
            .unwrap()
            .items
            .into_iter()
            .filter_map(|item| match item {
                gpui_kit::MenuItem::Action { name, .. } => Some(name.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(open_recent_names(), ["Teaser", "Clear Menu"]);

    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.mark_saved(folder.join("Other.studio"), false); cx.notify(); }));
    visual.update(|window, cx| window.dispatch_action(Box::new(OpenRecent { path: file.clone() }), cx));
    visual.run_until_parked();
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.file(), Some(file.as_path())));

    visual.update(|window, cx| window.dispatch_action(Box::new(ClearRecentProjects), cx));
    render(visual);
    assert_eq!(open_recent_names(), ["Clear Menu"]);
    std::fs::remove_dir_all(&folder).unwrap();
}

#[gpui_kit::test]
fn select_all_and_cut_work_on_layers(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let layer_count = visual.update(|_, cx| workspace.read(cx).state.composition().layers.len());
    assert!(layer_count > 1);
    visual.update(|window, cx| window.press("cmd-a", cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.selected_layers.len(), layer_count, "every layer is selected"));
    visual.update(|window, cx| window.press("cmd-x", cx));
    render(visual);
    visual.update(|_, cx| assert!(workspace.read(cx).state.composition().layers.is_empty(), "cut removes them"));
    visual.update(|window, cx| window.press("cmd-v", cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.composition().layers.len(), layer_count, "and paste brings them back"));
}
// Packaging ------------------------------------------------------------------------------------

/// Waits (up to 60 s) for the background package to finish, letting its events through.
fn wait_for_package(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    let started = std::time::Instant::now();
    while visual.update(|_, cx| workspace.read(cx).ui.package_job.is_some()) {
        assert!(started.elapsed() < std::time::Duration::from_secs(60), "the package was never finished");
        // Real time for the packaging thread, then the editor's next check on it.
        std::thread::sleep(std::time::Duration::from_millis(20));
        visual.executor().advance_clock(std::time::Duration::from_millis(100));
        visual.run_until_parked();
    }
}

#[gpui_kit::test]
fn packaging_zips_the_project_with_its_media_and_the_copy_reopens(cx: &mut TestAppContext) {
    use studio_core::model::LutSource;
    use studio_core::sample::ids;
    let folder = scratch_folder("package");
    let (media, out, unzipped) = (folder.join("media"), folder.join("out"), folder.join("unzipped"));
    for made in [&media, &out, &unzipped] {
        std::fs::create_dir_all(made).unwrap();
    }
    let (workspace, visual) = open(cx);
    // Media linked to files: two clips named alike (from two cards) in Footage, one in a
    // "B-Roll" folder inside it, a logo at the top level, music that is missing, and a LUT file
    // on a color grade layer. They are left off the timeline, so nothing tries to decode them.
    std::fs::create_dir_all(media.join("card2")).unwrap();
    for name in ["clip.mov", "card2/clip.mov", "wide.mov", "logo.png", "Film.cube"] {
        std::fs::write(media.join(name), format!("contents of {name}")).unwrap();
    }
    let linked = visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        use studio_core::{edit, folders};
        let project = &mut ws.state.project;
        let info = edit::FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: 2.0, frame_rate: studio_core::time::FrameRate::new(24, 1), has_audio: false };
        let b_roll = folders::add_folder(project, "B-Roll", Some(ids::FOOTAGE_FOLDER));
        let clips = [media.join("clip.mov"), media.join("card2/clip.mov"), media.join("wide.mov")].map(|path| edit::import_footage(project, path, &info));
        folders::move_to(project, clips[0], Some(ids::FOOTAGE_FOLDER));
        folders::move_to(project, clips[1], Some(ids::FOOTAGE_FOLDER));
        folders::move_to(project, clips[2], Some(b_roll));
        let logo = edit::import_image(project, media.join("logo.png"), Vec2::new(64.0, 36.0));
        let music = edit::import_audio(project, media.join("missing/score.wav"), 3.0);
        let grade = edit::ensure_color_grade_layer(project, ids::MAIN_COMP).unwrap();
        edit::add_lut(project, ids::MAIN_COMP, grade, LutSource::File(media.join("Film.cube"))).unwrap();
        cx.notify();
        (clips, logo, music)
    }));
    let (clips, logo, music) = linked;
    render(visual);

    // File ▸ Package… opens the dialog; Choose… picks the folder to save in.
    visual.update(|window, cx| window.dispatch_action(Box::new(crate::workspace::Package), cx));
    render(visual);
    visual.update(|window, _| assert!(window.try_find("package-ok").is_some(), "the Package dialog is open"));
    visual.update(|window, cx| window.click("package-choose", cx));
    visual.run_until_parked();
    assert!(visual.did_prompt_for_paths());
    visual.simulate_path_prompt_response(|options| {
        assert!(options.directories && !options.files, "asks for a folder");
        Some(vec![out.clone()])
    });
    visual.run_until_parked();
    render(visual);
    visual.update(|window, cx| window.click("package-ok", cx));
    render(visual);
    // While it runs, the top bar shows it and File ▸ Package… is greyed out.
    visual.update(|window, cx| {
        assert!(window.try_find("package-cancel").is_some(), "the top bar shows the package with a Cancel button");
        assert!(!window.is_action_available(&crate::workspace::Package, cx), "Package… is unavailable while a package is made");
    });
    wait_for_package(&workspace, visual);
    render(visual);
    visual.update(|window, cx| assert!(window.is_action_available(&crate::workspace::Package, cx), "and available again once it is done"));

    // The zip holds a folder named after the project, mirroring the Project panel's folders.
    let zip = out.join("Untitled.zip");
    assert!(zip.exists(), "named after the untitled project");
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 1, "no partial file is left beside it");
    zip::ZipArchive::new(std::fs::File::open(&zip).unwrap()).unwrap().extract(&unzipped).unwrap();
    let package = unzipped.join("Untitled");
    let copies = [("Footage/clip.mov", "clip.mov"), ("Footage/clip 2.mov", "card2/clip.mov"), ("Footage/B-Roll/wide.mov", "wide.mov"), ("Media/logo.png", "logo.png"), ("LUTs/Film.cube", "Film.cube")];
    for (copy, original) in copies {
        assert_eq!(std::fs::read_to_string(package.join(copy)).unwrap(), format!("contents of {original}"), "{copy}");
    }
    assert!(std::fs::read_to_string(package.join("Missing files.txt")).unwrap().contains("score.wav"));

    // The packaged project opens from where it was unzipped, with its media found there.
    let file = package.join("Untitled.studio");
    let missing = visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.open_project(&std::fs::read_to_string(&file).unwrap(), file.clone()).unwrap().0));
    assert_eq!(missing, [media.join("missing/score.wav")], "only the missing music is reported, at its old path");
    visual.update(|_, cx| {
        let project = &workspace.read(cx).state.project;
        let path = |id| project.item(id).unwrap().path.clone().unwrap();
        assert_eq!(path(clips[0]), package.join("Footage/clip.mov"));
        assert_eq!(path(clips[1]), package.join("Footage/clip 2.mov"));
        assert_eq!(path(clips[2]), package.join("Footage/B-Roll/wide.mov"));
        assert_eq!(path(logo), package.join("Media/logo.png"));
        assert_eq!(path(music), media.join("missing/score.wav"));
        let luts: Vec<_> = project.compositions.iter().flat_map(|comp| &comp.layers).flat_map(|layer| &layer.effects).filter_map(|effect| effect.lut.clone()).collect();
        assert_eq!(luts, [LutSource::File(package.join("LUTs/Film.cube"))]);
    });
    std::fs::remove_dir_all(&folder).unwrap();
}

// Morphing --------------------------------------------------------------------------------------

#[gpui_kit::test]
fn morph_to_turns_one_shape_into_another_from_the_menus(cx: &mut TestAppContext) {
    use crate::workspace::MorphTo;
    use studio_core::model::ShapeKind;
    let (workspace, visual) = open(cx);
    let square = add_shape(&workspace, visual, ShapeKind::Rectangle);
    let star = add_shape(&workspace, visual, ShapeKind::Star);
    let select_both = |visual: &mut VisualTestContext| {
        visual.update(|_, cx| workspace.update(cx, |ws, cx| {
            ws.state.select_layer(square, false);
            ws.state.select_layer(star, true);
            cx.notify();
        }));
        render(visual);
    };
    let morphing = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(square).unwrap().is_morphing());
    select_both(visual);

    // The square's right-click menu: Morph To… opens the dialog, and Morph makes the keyframes.
    visual.update(|window, cx| window.right_click(("clip-body", square.0 as usize), cx));
    render(visual);
    click_label(visual, "Morph To…");
    render(visual);
    visual.update(|window, cx| window.click("morph-ok", cx));
    render(visual);
    assert!(morphing(visual), "the square morphs");
    visual.update(|_, cx| {
        let workspace = workspace.read(cx);
        let state = &workspace.state;
        let layer = state.composition().layer(square).unwrap();
        let fps = state.composition().frame_rate.fps();
        let frames = layer.morph.as_ref().unwrap().path.keyframe_frames();
        assert_eq!(frames, vec![state.current_frame, state.current_frame + fps.round() as i64], "one second from the playhead");
        assert!(!state.composition().layer(star).unwrap().switches.visible, "the star is hidden");
        assert_eq!(state.selected_layers, vec![square]);
        assert_eq!(workspace.ui.tab(crate::workspace::PanelId::Properties), crate::panels::motion::TAB, "the Motion tab shows the morph");
    });

    // The Motion tab's Morph card keyframes the outline at the playhead.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.set_current_frame(ws.state.current_frame + 5);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click("morph-path-keyframe", cx));
    render(visual);
    let keyframes = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(square).unwrap().morph.as_ref().map_or(0, |morph| morph.path.keyframe_frames().len()));
    assert_eq!(keyframes(visual), 3);

    // Undone, then made again from Layer ▸ Morph To… with a length typed in and no easing.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        // Back past the added keyframe and the morph, but no further (the shapes stay).
        while ws.state.composition().layer(square).is_some_and(|layer| layer.morph.is_some()) && ws.state.undo() {}
        cx.notify();
    }));
    render(visual);
    assert!(!morphing(visual), "undo takes the morph off");
    select_both(visual);
    visual.update(|window, cx| window.dispatch_action(Box::new(MorphTo), cx));
    render(visual);
    visual.update(|window, cx| window.click("morph-length", cx));
    visual.update(|window, cx| {
        window.press("cmd-a", cx);
        for key in ["0", ".", "5"] {
            window.press(key, cx);
        }
    });
    render(visual);
    visual.update(|window, cx| window.click("morph-ease", cx));
    visual.update(|window, cx| window.click("morph-ok", cx));
    render(visual);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        let layer = state.composition().layer(square).unwrap();
        let path = &layer.morph.as_ref().unwrap().path;
        let fps = state.composition().frame_rate.fps();
        assert_eq!(path.keyframe_frames(), vec![state.current_frame, state.current_frame + (fps * 0.5).round() as i64], "half a second");
        assert_eq!(path.easing_at(state.current_frame), Some(studio_core::animation::Easing::Linear), "not eased");
    });

    // With one layer selected, Morph To says what it needs instead of opening.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(square, false);
        cx.notify();
    }));
    visual.update(|window, cx| window.dispatch_action(Box::new(MorphTo), cx));
    render(visual);
    assert!(visual.update(|window, _| window.try_find("morph-ok").is_none()), "no dialog for one layer");
}

#[gpui_kit::test]
fn magic_move_joins_two_compositions_from_the_composition_menu(cx: &mut TestAppContext) {
    use crate::workspace::MagicMove;
    use studio_core::model::ShapeKind;
    let (workspace, visual) = open(cx);
    // A second composition like the open one, each with a shape named "Card".
    let (first, second) = visual.update(|_, cx| workspace.update(cx, |ws, _| {
        let open = ws.state.composition().clone();
        let settings = studio_core::edit::CompositionSettings { name: "Scene 2".into(), width: open.width, height: open.height, frame_rate: open.frame_rate, duration_frames: 90, background: open.background };
        (open.id, studio_core::edit::new_composition(&mut ws.state.project, &settings).unwrap())
    }));
    for (comp, kind) in [(first, ShapeKind::Rectangle), (second, ShapeKind::Star)] {
        visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.open_composition(comp)));
        let card = add_shape(&workspace, visual, kind);
        visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.composition_mut_for_test().layer_mut(card).unwrap().name = "Card".into()));
    }
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.open_composition(first);
        cx.notify();
    }));
    render(visual);

    visual.update(|window, cx| window.dispatch_action(Box::new(MagicMove), cx));
    render(visual);
    // Scene 2 is the only composition that fits, so the dialog offers it first.
    visual.update(|window, _| assert!(window.find_by_label("Scene 2").bounds().size.width > gpui_kit::px(0.0)));
    visual.update(|window, cx| window.click("magic-move-ok", cx));
    render(visual);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        let joined = state.composition();
        let (first, second) = (state.project.composition(first).unwrap(), state.project.composition(second).unwrap());
        assert_eq!(joined.name, format!("{} → Scene 2", first.name), "the joined composition opens");
        let transition = first.frame_rate.fps().round() as i64;
        assert_eq!(joined.duration_frames, first.duration_frames + transition + second.duration_frames, "one second between them");
        let middle = first.duration_frames + transition / 2;
        let cards: Vec<_> = joined.layers.iter().filter(|layer| layer.name == "Card" && layer.is_active_at(middle)).collect();
        assert!(matches!(cards[..], [card] if card.is_morphing()), "the card morphs from one into the other");
    });
}

#[gpui_kit::test]
fn a_shortcut_changes_by_pressing_the_new_key(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.press("cmd-,", cx));
    render(visual);
    // Keyboard Shortcuts is the tenth page.
    visual.update(|window, cx| window.click(("settings-section", 9_usize), cx));
    render(visual);
    visual.update(|window, cx| window.click("shortcut-change:toggle-playback", cx));
    render(visual);
    // K is free in Studio's set; pressing it doesn't run anything else, it becomes Play's key.
    visual.update(|window, cx| window.press("k", cx));
    render(visual);
    assert_eq!(crate::app_settings::load().shortcuts.custom.get("toggle-playback"), Some(&vec!["k".to_owned()]), "remembered");
    assert!(!visual.update(|_, cx| workspace.read(cx).state.is_playing()), "pressing it only set it");
    visual.update(|window, cx| window.press("escape", cx));
    render(visual);
    visual.update(|window, cx| window.press("space", cx));
    render(visual);
    assert!(!visual.update(|_, cx| workspace.read(cx).state.is_playing()), "Space no longer plays");
    visual.update(|window, cx| window.press("k", cx));
    render(visual);
    assert!(visual.update(|_, cx| workspace.read(cx).state.is_playing()), "K does");
    visual.update(|window, cx| window.press("k", cx));
    render(visual);

    // Premiere Pro's set: the right arrow steps a frame.
    let mut shortcuts = crate::keymap::Shortcuts { preset: crate::keymap::KeyPreset::Premiere, ..Default::default() };
    shortcuts.custom.clear();
    visual.update(|_, cx| crate::keymap::apply(&crate::app_settings::AppSettings { shortcuts, ..Default::default() }, cx));
    let before = visual.update(|_, cx| workspace.read(cx).state.current_frame);
    visual.update(|window, cx| window.press("right", cx));
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.current_frame), before + 1);
}

#[gpui_kit::test]
fn empty_panels_say_what_to_do(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.new_project(&[], crate::app_settings::NewComposition::default());
        cx.notify();
    }));
    render(visual);
    assert!(visual.update(|window, _| window.try_find("project-empty-hint")).is_some(), "the Project panel says how to import");
    assert!(visual.update(|window, _| window.try_find("timeline-empty-hint")).is_some(), "the empty timeline says what to add");
    // A layer and some media: both hints go.
    let clip = add_tone_clip(&workspace, visual);
    render(visual);
    assert!(visual.update(|_, cx| workspace.read(cx).state.composition().layer(clip).is_some()));
    assert!(visual.update(|window, _| window.try_find("project-empty-hint")).is_none());
    assert!(visual.update(|window, _| window.try_find("timeline-empty-hint")).is_none());
}

#[gpui_kit::test]
fn a_start_screen_preset_starts_a_project_at_its_size(cx: &mut TestAppContext) {
    use crate::workspace::ShowStartScreen;
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| window.dispatch_action(Box::new(ShowStartScreen), cx));
    render(visual);
    // Vertical: 1080 × 1920 at 30 fps.
    visual.update(|window, cx| window.click(("start-preset", 2usize), cx));
    visual.run_until_parked();
    render(visual);
    assert!(!start_screen_showing(&workspace, visual));
    visual.update(|_, cx| {
        let comp = workspace.read(cx).state.composition();
        assert_eq!((comp.width, comp.height, comp.frame_rate), (1080, 1920, studio_core::time::FrameRate::new(30, 1)));
    });
}

#[gpui_kit::test]
fn earlier_versions_are_kept_and_restored(cx: &mut TestAppContext) {
    use crate::workspace::BrowseVersions;
    let folder = scratch_folder("versions");
    let (workspace, visual) = open(cx);
    make_an_edit(&workspace, visual);
    visual.update(|window, cx| window.press("cmd-s", cx));
    visual.run_until_parked();
    let saved = folder.join("Versions Demo");
    visual.simulate_new_path_selection(|_| Some(saved.clone()));
    visual.run_until_parked();
    render(visual);
    let file = folder.join("Versions Demo.studio");
    assert_eq!(crate::versions::list(Some(&file)).len(), 1, "saving keeps a version");

    // Working on: a version is kept while it changes, once the time comes.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.set_property_component(TITLE, ROTATION, 0, 75.0); cx.notify(); }));
    visual.update(|_, cx| workspace.update(cx, |ws, _| { ws.ui.last_version_at = None; ws.keep_timed_version(); }));
    assert_eq!(crate::versions::list(Some(&file)).len(), 2);

    // Restore the saved one (the older): the rotation goes back, as unsaved changes.
    visual.update(|window, cx| window.dispatch_action(Box::new(BrowseVersions), cx));
    render(visual);
    visual.update(|window, cx| window.click(("version-restore", 1usize), cx));
    visual.run_until_parked();
    render(visual);
    assert_eq!(title_value(&workspace, visual, ROTATION, 0), PropertyValue::Scalar(33.0));
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.file(), Some(file.as_path()), "still the same project"));
    assert!(has_unsaved_changes(&workspace, visual));
    // The work at 75° wasn't lost: it is the newest version.
    let newest = crate::versions::list(Some(&file)).remove(0);
    assert!(std::fs::read_to_string(&newest.path).unwrap().contains("75.0"));
    std::fs::remove_dir_all(crate::versions::folder_for(Some(&file))).ok();
    std::fs::remove_dir_all(&folder).ok();
}

#[gpui_kit::test]
fn frames_rendered_ahead_play_without_rendering(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.ui.settings.render_ahead_mb = 256;
        ws.state.set_work_area(studio_core::time::FrameRange::new(0.0, 12.0));
        cx.notify();
    }));
    render(visual);
    // The background renderer fills in the work area from the playhead.
    let started = std::time::Instant::now();
    while visual.update(|_, cx| workspace.read(cx).render.preview.ranges()) != vec![(0, 12)] {
        assert!(started.elapsed().as_secs() < 30, "the frames ahead never came");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    render(visual);
    // Playing shows them instead of rendering.
    let drawn = visual.update(|_, cx| workspace.read(cx).render.frames_drawn);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| ws.toggle_playback(cx)));
    render(visual);
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(40));
        render(visual);
    }
    assert!(visual.update(|_, cx| workspace.read(cx).state.current_frame) > 0, "it played");
    assert_eq!(visual.update(|_, cx| workspace.read(cx).render.frames_drawn), drawn, "no frame was rendered while playing");
    visual.update(|_, cx| workspace.update(cx, |ws, cx| ws.toggle_playback(cx)));
    // An edit makes them out of date: frames made from the project before it are let go.
    let (before, comp) = visual.update(|_, cx| (workspace.read(cx).state.project.clone(), workspace.read(cx).state.active_comp));
    assert!(visual.update(|_, cx| workspace.read(cx).render.preview.frame_for(&before, comp, 3).is_some()));
    make_an_edit(&workspace, visual);
    render(visual);
    assert!(visual.update(|_, cx| workspace.read(cx).render.preview.frame_for(&before, comp, 3).is_none()), "nothing from before the edit is shown");
}

/// A big project: 500 layers (text, shapes and solids, each animated) across a minute, to find
/// what slows down as projects grow. Prints where the time goes. Run with
/// `cargo test -p studio --release stress_500_layers -- --ignored --nocapture`.
#[gpui_kit::test]
#[ignore = "benchmark; run on demand"]
fn stress_500_layers(cx: &mut TestAppContext) {
    use studio_core::animation::{Animated, Easing, Keyframe};
    use studio_core::model::{ShapeKind, ShapeStyle};
    let (workspace, visual) = open(cx);
    let time = |label: &str, runs: u32, mut work: Box<dyn FnMut(&mut VisualTestContext) + '_>, visual: &mut VisualTestContext| {
        let started = std::time::Instant::now();
        for _ in 0..runs {
            work(visual);
        }
        let each = started.elapsed().as_secs_f64() * 1000.0 / f64::from(runs);
        println!("{label:<42} {each:>8.2} ms");
        each
    };
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.new_project(&[], crate::app_settings::NewComposition { width: 1920, height: 1080, fps: 30, seconds: 60 });
        let comp = ws.state.active_comp;
        let project = &mut ws.state.project;
        for index in 0..500u32 {
            let at = Vec2::new(100.0 + (index % 20) as f32 * 85.0, 80.0 + (index / 20) as f32 * 38.0);
            let start = i64::from(index % 60) * 30;
            let made = match index % 5 {
                0 | 1 => studio_core::edit::add_text_layer(project, comp, &format!("Line {index}"), at, start),
                2 | 3 => studio_core::edit::add_shape_layer(project, comp, ShapeStyle::new(if index % 2 == 0 { ShapeKind::Rectangle } else { ShapeKind::Ellipse }), Vec2::new(60.0, 30.0), at, start),
                _ => studio_core::edit::add_background(project, comp, studio_core::model::Rgba::hex(0x223344)),
            }
            .unwrap();
            let layer = project.composition_mut(comp).unwrap().layer_mut(made).unwrap();
            layer.out_frame = (start + 600).min(1800);
            if let Some(transform) = layer.transform.as_mut() {
                transform.position = Animated::keyframed(vec![Keyframe { frame: start, value: at, easing: Easing::Linear }, Keyframe { frame: start + 60, value: at + Vec2::new(40.0, 0.0), easing: Easing::Linear }]);
                transform.opacity = Animated::keyframed(vec![Keyframe::linear(start, 0.0), Keyframe::linear(start + 15, 100.0)]);
            }
        }
        ws.state.current_frame = 900;
        cx.notify();
    }));
    render(visual);
    println!("layers: {}", visual.update(|_, cx| workspace.read(cx).state.composition().layers.len()));

    time("whole window after an edit", 20, Box::new(|visual| {
        visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.current_frame = 900; cx.notify(); }));
        draw_frame(visual);
    }), visual);
    time("moving the playhead (scrub)", 30, Box::new(|visual| {
        visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.set_current_frame(ws.state.current_frame + 1); cx.notify(); }));
        draw_frame(visual);
    }), visual);
    time("selecting a layer", 20, Box::new(|visual| {
        visual.update(|_, cx| workspace.update(cx, |ws, cx| {
            let layer = ws.state.composition().layers[250].id;
            ws.state.select_layer(layer, false);
            cx.notify();
        }));
        draw_frame(visual);
    }), visual);
    // Each panel's share of a whole-window redraw.
    let before = visual.update(|_, cx| workspace.read(cx).panel_render_times(cx));
    for _ in 0..10 {
        visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.current_frame = 900; cx.notify(); }));
        draw_frame(visual);
    }
    let after = visual.update(|_, cx| workspace.read(cx).panel_render_times(cx));
    let names = ["toolbar", "project", "viewer", "timeline", "properties", "effects", "libraries", "align", "templates", "captions", "elements", "mockups"];
    for (index, name) in names.iter().enumerate() {
        let each = (after[index] - before[index]).as_secs_f64() * 100.0;
        if each > 0.05 {
            println!("  panel {name:<38} {each:>8.2} ms");
        }
    }
    time("  changing a property", 20, Box::new(|visual| {
        visual.update(|_, cx| workspace.update(cx, |ws, _| {
            let layer = ws.state.composition().layers[250].id;
            ws.state.set_property_component(layer, studio_core::property::PropertyPath::Transform(studio_core::property::TransformProperty::Rotation), 0, 10.0);
        }));
    }), visual);
    time("  recording it for undo", 20, Box::new(|visual| {
        visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.record_history(false)));
    }), visual);
    time("an edit recorded for undo", 20, Box::new(|visual| {
        visual.update(|_, cx| workspace.update(cx, |ws, cx| {
            let layer = ws.state.composition().layers[250].id;
            ws.state.set_property_component(layer, studio_core::property::PropertyPath::Transform(studio_core::property::TransformProperty::Rotation), 0, 10.0);
            ws.state.record_history(false);
            cx.notify();
        }));
    }), visual);
    time("comparing the project (viewer cache check)", 50, Box::new(|visual| {
        visual.update(|_, cx| {
            let project = &workspace.read(cx).state.project;
            let copy = project.clone();
            assert!(copy == *project);
        });
    }), visual);
    time("saving to text", 5, Box::new(|visual| {
        visual.update(|_, cx| { workspace.read(cx).state.project_text().unwrap(); });
    }), visual);
    let text = visual.update(|_, cx| workspace.read(cx).state.project_text().unwrap());
    println!("file size: {} KB", text.len() / 1024);
    time("opening from text", 5, Box::new(|_| { studio_core::project_file::from_text(&text).unwrap(); }), visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| ws.toggle_playback(cx)));
    render(visual);
    time("a playback frame", 30, Box::new(draw_frame), visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| ws.toggle_playback(cx)));
    let ms = visual.update(|_, cx| workspace.read(cx).render.last_render_ms);
    println!("viewer render alone (last frame): {ms:?} ms");
}

#[gpui_kit::test]
fn a_freeze_frame_goes_in_from_a_clips_right_click_menu(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    let clip = first_footage(&workspace, visual);
    let (_, in_frame, out_frame) = layer_timing(&workspace, visual, clip);
    let middle = (in_frame + out_frame) / 2;
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.current_frame = middle; cx.notify(); }));
    render(visual);
    let count = visual.update(|_, cx| workspace.read(cx).state.composition().layers.len());
    visual.update(|window, cx| window.right_click(("clip-body", clip.0 as usize), cx));
    render(visual);
    click_in_submenu(visual, "Time", "Insert Freeze Frame");
    render(visual);
    // The clip is split and a hold goes between the halves: two more layers, the hold selected.
    let (count_after, hold) = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        (state.composition().layers.len(), state.composition().layer(state.selected_layers[0]).unwrap().clone())
    });
    assert_eq!(count_after, count + 2);
    assert!(hold.time.hold.is_some() && hold.in_frame == middle && hold.out_frame == middle + 48, "{:?}", (hold.in_frame, hold.out_frame));
    assert_eq!(layer_timing(&workspace, visual, clip).2, middle, "the first half ends where the hold starts");
    // One undo takes it all back.
    visual.update(|window, cx| window.press("cmd-z", cx));
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.composition().layers.len()), count);
    assert_eq!(layer_timing(&workspace, visual, clip).2, out_frame);
}

#[gpui_kit::test]
fn paste_attributes_puts_the_copied_clips_effects_and_opacity_on_another(cx: &mut TestAppContext) {
    use studio_core::animation::Animated;
    let (workspace, visual) = open(cx);
    tall_window(visual);
    // The title gets 40% opacity and is copied; the main-track clip is selected.
    let other = main_clip(&workspace, visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let layer = ws.state.composition_mut_for_test().layer_mut(TITLE).unwrap();
        layer.transform.as_mut().unwrap().opacity = Animated::constant(40.0);
        ws.state.add_effect(TITLE, "Gaussian Blur");
        ws.state.selected_layers = vec![TITLE];
        ws.state.copy_selection();
        ws.state.selected_layers = vec![other];
        cx.notify();
    }));
    let effects_before = visual.update(|_, cx| workspace.read(cx).state.composition().layer(other).unwrap().effects.len());
    let title_effects = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().effects.len());
    assert!(title_effects > 0, "the copied title has effects to paste");
    visual.update(|window, cx| window.press("cmd-alt-v", cx));
    render(visual);
    // Leave motion out, then paste.
    visual.update(|window, cx| window.click("paste-attr-motion", cx));
    visual.update(|window, cx| window.click("paste-attributes-ok", cx));
    render(visual);
    let layer = visual.update(|_, cx| workspace.read(cx).state.composition().layer(other).unwrap().clone());
    assert_eq!(layer.transform.as_ref().unwrap().opacity.value, 40.0);
    assert_eq!(layer.effects.len(), effects_before + title_effects);
}

#[gpui_kit::test]
fn close_gaps_packs_a_track_from_the_clips_menu(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    // Move the title later, leaving a gap before it on its track.
    let (track, start) = visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let layer = ws.state.composition_mut_for_test().layer_mut(TITLE).unwrap();
        layer.shift_in_time(50);
        cx.notify();
        (layer.track, layer.in_frame)
    }));
    render(visual);
    let first_on_track = visual.update(|_, cx| {
        let comp = workspace.read(cx).state.composition();
        comp.layers.iter().filter(|layer| layer.track == track).map(|layer| layer.in_frame).min().unwrap()
    });
    assert_eq!(first_on_track, start, "the title is alone on its track");
    visual.update(|window, cx| window.right_click(("clip-body", TITLE.0 as usize), cx));
    render(visual);
    click_in_submenu(visual, "Time", "Close Gaps on Track");
    render(visual);
    assert_eq!(layer_timing(&workspace, visual, TITLE).1, 0, "packed to the start");
}

#[gpui_kit::test]
fn keyframing_speed_remaps_time_and_the_stopwatch_puts_it_back(cx: &mut TestAppContext) {
    use studio_core::property::PropertyPath;
    let (workspace, visual) = open(cx);
    let clip = first_footage(&workspace, visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(clip, false);
        ws.state.set_speed(clip, 50.0);
        cx.notify();
    }));
    tall_window(visual);
    let before = visual.update(|_, cx| workspace.read(cx).state.composition().layer(clip).unwrap().clone());
    let stopwatch = ElementId::Name(format!("stopwatch:{:?}:{}", PropertyPath::Speed, clip.0).into());
    // Stopwatch on at the clip's start, then 200% further in: speed now changes over the clip.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.current_frame = before.in_frame; cx.notify(); }));
    render(visual);
    visual.update(|window, cx| window.click(stopwatch.clone(), cx));
    render(visual);
    let later = before.in_frame + 20;
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.current_frame = later;
        crate::panels::fields::FieldTarget::Property { layer: clip, path: PropertyPath::Speed, component: 0 }.write(&mut ws.state, 200.0);
        cx.notify();
    }));
    render(visual);
    let remapped = visual.update(|_, cx| workspace.read(cx).state.composition().layer(clip).unwrap().clone());
    assert_eq!(remapped.time.speed.keyframe_frames(), vec![before.in_frame, later]);
    assert_eq!(remapped.out_frame, before.out_frame, "remapping keeps the clip's length");
    let rate = studio_core::time::FrameRate::new(24, 1);
    let shown = |layer: &studio_core::model::Layer, frame| layer.source_seconds(frame, rate);
    assert!(shown(&remapped, later + 5) - shown(&remapped, later) > shown(&before, later + 5) - shown(&before, later), "plays faster after the second keyframe");
    // The keyframes show under the clip in the timeline like any other property's.
    let rows = visual.update(|_, cx| crate::panels::timeline::animated_row_names(&workspace.read(cx).state.composition().layer(clip).unwrap().clone()));
    assert!(rows.contains(&"Speed".to_owned()), "{rows:?}");
    // Turning the stopwatch off puts the steady 50% back exactly.
    visual.update(|window, cx| window.click(stopwatch.clone(), cx));
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.composition().layer(clip).unwrap().clone()), before);
}

#[gpui_kit::test]
fn render_and_play_renders_the_work_area_then_plays_it(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.ui.settings.render_ahead_mb = 256;
        ws.state.set_work_area(studio_core::time::FrameRange::new(6.0, 18.0));
        ws.state.current_frame = 40;
        ws.state.loop_playback = false;
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click("tracks-render-play", cx));
    render(visual);
    // It goes to the work area's start and waits, showing its progress, instead of playing.
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        assert!(!ws.state.is_playing() && ws.ui.render_then_play);
        assert_eq!(ws.state.current_frame, 6);
    });
    visual.update(|window, _| assert!(window.try_find("render-play-progress").is_some()));
    // As frames arrive (the editor checks as they do), it starts playing once all twelve are in.
    let started = std::time::Instant::now();
    while !visual.update(|_, cx| workspace.read(cx).state.is_playing()) {
        assert!(started.elapsed().as_secs() < 30, "it never started playing");
        std::thread::sleep(std::time::Duration::from_millis(20));
        visual.update(|_, cx| workspace.update(cx, |ws, cx| ws.check_render_then_play(cx)));
        render(visual);
    }
    let (ready, looping) = visual.update(|_, cx| (workspace.read(cx).render.preview.ranges(), workspace.read(cx).state.loop_playback));
    assert!(ready.iter().any(|&(start, end)| start <= 6 && end >= 18), "{ready:?}");
    assert!(looping, "it plays the work area in a loop");
    visual.update(|_, cx| workspace.update(cx, |ws, cx| ws.toggle_playback(cx)));

    // Cancel stops waiting; with rendering ahead off it just plays.
    visual.update(|window, cx| window.click("tracks-render-play", cx));
    render(visual);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.state.set_work_area(studio_core::time::FrameRange::new(0.0, 200.0)); cx.notify(); }));
    render(visual);
    visual.update(|window, cx| window.click("render-play-cancel", cx));
    render(visual);
    assert!(!visual.update(|_, cx| workspace.read(cx).ui.render_then_play));
    visual.update(|_, cx| workspace.update(cx, |ws, cx| { ws.ui.settings.render_ahead_mb = 0; cx.notify(); }));
    visual.update(|window, cx| window.press("shift-space", cx));
    assert!(visual.update(|_, cx| workspace.read(cx).state.is_playing()));
}

#[gpui_kit::test]
fn the_export_queue_runs_its_exports_one_after_another(cx: &mut TestAppContext) {
    use crate::panels::export_queue::QueueStatus;
    let folder = scratch_folder("export-queue");
    let (workspace, visual) = open(cx);
    // Two short exports go into the queue from the Export dialog's Add to Queue….
    let mut paths = Vec::new();
    for name in ["First.mp4", "Second.mp4"] {
        open_export_dialog(&workspace, visual);
        visual.update(|window, cx| window.click("export-queue-add", cx));
        visual.run_until_parked();
        let chosen = folder.join(name);
        visual.simulate_new_path_selection(|_| Some(chosen.clone()));
        visual.run_until_parked();
        render(visual);
        paths.push(chosen);
    }
    visual.update(|_, cx| {
        let queue = &workspace.read(cx).ui.export_queue;
        assert_eq!(queue.items.len(), 2);
        assert!(queue.items.iter().all(|item| item.status == QueueStatus::Waiting), "nothing runs until started");
    });
    assert!(paths.iter().all(|path| !path.exists()));
    // An edit made now doesn't change what's queued: each export keeps the project as it was.
    make_an_edit(&workspace, visual);
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        assert_ne!(ws.ui.export_queue.items[0].project, ws.state.project, "the queued export kept the project from before the edit");
    });
    // File ▸ Export Queue… lists them; Start Queue runs both.
    visual.update(|window, cx| window.dispatch_action(Box::new(crate::workspace::ExportQueue), cx));
    render(visual);
    visual.update(|window, cx| window.click("export-queue-start", cx));
    render(visual);
    let started = std::time::Instant::now();
    while visual.update(|_, cx| workspace.read(cx).ui.export_queue.running) {
        assert!(started.elapsed().as_secs() < 120, "the queue never finished");
        wait_for_export(&workspace, visual);
        visual.executor().advance_clock(std::time::Duration::from_millis(100));
        visual.run_until_parked();
    }
    visual.update(|_, cx| {
        let queue = &workspace.read(cx).ui.export_queue;
        assert!(queue.items.iter().all(|item| item.status == QueueStatus::Done), "{:?}", queue.items.iter().map(|item| item.status.clone()).collect::<Vec<_>>());
    });
    for path in &paths {
        assert!(std::fs::metadata(path).is_ok_and(|file| file.len() > 0), "{} was written", path.display());
    }
    // Clear Finished empties the list.
    visual.update(|window, cx| window.dispatch_action(Box::new(crate::workspace::ExportQueue), cx));
    render(visual);
    visual.update(|window, cx| window.click("export-queue-clear", cx));
    render(visual);
    assert!(visual.update(|_, cx| workspace.read(cx).ui.export_queue.items.is_empty()));
    std::fs::remove_dir_all(&folder).unwrap();
}

// Keyer and motion blur --------------------------------------------------------------------------

#[gpui_kit::test]
fn the_keyer_is_added_from_effect_controls_and_picks_its_key_color_in_the_viewer(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let a_cam = Id(104);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(a_cam, false);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click(ElementId::Name("tab:Properties:1".into()), cx));
    render(visual);
    // The Add Effect menu lists it under Keying for footage, past the first screenful: scroll
    // the menu with the wheel until it is in view, then pick it.
    visual.update(|window, cx| window.click("fx-add", cx));
    render(visual);
    let first_item = visual.update(|window, _| window.find_by_label("Crop").bounds().center());
    for _ in 0..20 {
        if visual.update(|window, _| window.find_by_label("Keyer").bounds().center().y) < first_item.y {
            break;
        }
        visual.update(|window, cx| {
            use gpui_kit::InputEvent as _;
            let wheel = gpui_kit::ScrollEvent { position: first_item, delta: gpui_kit::ScrollDelta::Pixels(point(px(0.0), px(-60.0))), ..Default::default() };
            window.dispatch_event(wheel.to_platform_input(), cx);
        });
        render(visual);
    }
    click_label(visual, "Keyer");
    render(visual);
    let keyer = visual.update(|_, cx| workspace.read(cx).state.effect_named(a_cam, "Keyer")).expect("the Keyer was added");
    let key = |visual: &mut VisualTestContext| {
        visual.update(|_, cx| {
            let ws = workspace.read(cx);
            (0..3).map(|parameter| ws.state.composition().layer(a_cam).unwrap().property(PropertyPath::Effect { effect: keyer, parameter }, 0).unwrap().value.component(0)).collect::<Vec<_>>()
        })
    };
    assert_eq!(key(visual), [0.0, 255.0, 0.0], "it starts keying pure green");

    // The eyedropper waits for a click in the viewer; meanwhile the viewer shows the picture
    // without this key, so the click reads the screen's own color.
    let pick = ElementId::Name(format!("keyer-pick:{}", keyer.0).into());
    visual.update(|window, cx| window.click(pick.clone(), cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.key_color_pick, Some((a_cam, keyer))));
    // The middle is where the selected layer's anchor point sits; its handles step aside while
    // picking, so the click still reaches the picture.
    let screen = screen_point(&workspace, visual, Vec2::new(960.0, 540.0));
    click_at(visual, screen);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.key_color_pick, None, "one click picks, then the eyedropper lets go"));
    let picked = key(visual);
    assert_ne!(picked, [0.0, 255.0, 0.0], "the key color is now the color clicked");
    assert!(picked.iter().all(|channel| (0.0..=255.0).contains(channel) && channel.fract() == 0.0), "{picked:?}");

    // Pressing the eyedropper again before clicking gives up, as Escape does.
    visual.update(|window, cx| window.click(pick.clone(), cx));
    render(visual);
    visual.update(|window, cx| window.press("escape", cx));
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.key_color_pick, None));

    // Its other settings are ordinary fields: typing a tolerance sets it.
    let tolerance = FieldTarget::Property { layer: a_cam, path: PropertyPath::Effect { effect: keyer, parameter: 3 }, component: 0 };
    visual.update(|window, cx| window.click(field_id(tolerance), cx));
    render(visual);
    visual.update(|window, cx| {
        window.input("42", cx);
        window.press("enter", cx);
    });
    render(visual);
    let typed = visual.update(|_, cx| workspace.read(cx).state.composition().layer(a_cam).unwrap().property(PropertyPath::Effect { effect: keyer, parameter: 3 }, 0).unwrap().value);
    assert_eq!(typed, PropertyValue::Scalar(42.0));
}

#[gpui_kit::test]
fn motion_blur_is_switched_in_properties_and_off_from_the_clip(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let blurred = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().switches.motion_blur);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        cx.notify();
    }));
    render(visual);
    // The clip shows no motion blur button until it is on.
    assert!(visual.update(|window, _| window.try_find(("clip-motion-blur", TITLE.0 as usize))).is_none());
    visual.update(|window, cx| window.click("layer-motion-blur", cx));
    render(visual);
    assert!(blurred(visual), "the Properties checkbox turns it on");
    visual.update(|_, cx| assert!(workspace.read(cx).state.can_undo(), "as an undoable edit"));
    // Now the clip shows it, and clicking there turns it off again.
    visual.update(|window, cx| window.click(("clip-motion-blur", TITLE.0 as usize), cx));
    render(visual);
    assert!(!blurred(visual));
    assert!(visual.update(|window, _| window.try_find(("clip-motion-blur", TITLE.0 as usize))).is_none());

    // The shutter lives in Composition Settings: the dialog shows the composition's own and
    // OK keeps it.
    let shutter = studio_core::motion_blur::MotionBlur { shutter_angle: 270.0, samples: 24 };
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.set_motion_blur(shutter).unwrap();
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.press("cmd-k", cx));
    render(visual);
    click_label(visual, "OK");
    render(visual);
    visual.update(|_, cx| assert_eq!(workspace.read(cx).state.composition().motion_blur, shutter));
}

#[gpui_kit::test]
fn subtitles_import_from_file_import_and_export_from_properties(cx: &mut TestAppContext) {
    let folder = scratch_folder("subtitles");
    // A messy but readable file (byte order mark, Windows line endings, italics, no blank line
    // at the end), and a file that isn't subtitles at all.
    let srt = folder.join("Interview.srt");
    std::fs::write(&srt, "\u{FEFF}1\r\n00:00:01,000 --> 00:00:02,000\r\n<i>Hello</i> there\r\n\r\n2\r\n00:00:02,500 --> 00:00:04,000\r\nWelcome to Studio").unwrap();
    let not_subtitles = folder.join("Notes.vtt");
    std::fs::write(&not_subtitles, "Just some notes.\n").unwrap();
    let (workspace, visual) = open(cx);
    tall_window(visual);
    let layers_before = visual.update(|_, cx| workspace.read(cx).state.composition().layers.len());

    // File ▸ Import takes subtitle files and makes a caption layer from the readable one.
    visual.update(|window, cx| window.dispatch_action(Box::new(crate::workspace::Import), cx));
    visual.run_until_parked();
    assert!(visual.did_prompt_for_paths());
    visual.simulate_path_prompt_response(|_| Some(vec![srt.clone(), not_subtitles.clone()]));
    visual.run_until_parked();
    render(visual);
    let (captions, cues, fps) = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        assert_eq!(state.composition().layers.len(), layers_before + 1, "one caption layer; the notes are refused");
        let captions = state.selected_layers[0];
        assert_eq!(state.composition().layer(captions).unwrap().name, "Interview");
        (captions, state.subtitle_cues(captions), state.composition().frame_rate)
    });
    let second = |seconds: f64| fps.seconds_to_frames(seconds);
    assert_eq!(cues.iter().map(|cue| (cue.text.as_str(), cue.start, cue.end)).collect::<Vec<_>>(), [("Hello there", second(1.0), second(2.0)), ("Welcome to Studio", second(2.5), second(4.0))]);

    // Properties' Export SRT… asks where, then writes the same cues.
    visual.update(|window, cx| window.click("captions-export-srt", cx));
    visual.run_until_parked();
    assert!(visual.did_prompt_for_new_path());
    let exported = folder.join("Exported");
    visual.simulate_new_path_selection(|_| Some(exported.clone()));
    visual.run_until_parked();
    let written = std::fs::read(folder.join("Exported.srt")).expect("the extension is added");
    assert_eq!(studio_core::subtitles::read(&written, fps).unwrap().cues, cues);
    // The WebVTT button writes a WebVTT file.
    visual.update(|window, cx| window.click("captions-export-vtt", cx));
    visual.run_until_parked();
    visual.simulate_new_path_selection(|_| Some(folder.join("Exported.vtt")));
    visual.run_until_parked();
    assert!(std::fs::read_to_string(folder.join("Exported.vtt")).unwrap().starts_with("WEBVTT\n"));
    visual.update(|_, cx| assert!(workspace.read(cx).state.composition().layer(captions).is_some()));
    std::fs::remove_dir_all(&folder).unwrap();
}

/// Writes a short movie of three shots (red, blue, then a checker pattern) with hard cuts at
/// source frames 30 and 60, at 24 frames a second.
fn write_three_shots(path: &std::path::Path) {
    let (width, height) = (96, 54);
    let mut writer = studio_media::MovieWriter::create(path, studio_media::MovieSettings::new(studio_media::MovieFormat::H264, width, height, studio_core::time::FrameRate::new(24, 1), None)).unwrap();
    let flat = |bgra: [u8; 4]| -> Vec<u8> { bgra.iter().copied().cycle().take((width * height * 4) as usize).collect() };
    let checker: Vec<u8> = (0..height).flat_map(|y| (0..width).map(move |x| (x, y))).flat_map(|(x, y)| if (x / 8 + y / 8) % 2 == 0 { [240, 240, 240, 255] } else { [20, 120, 40, 255] }).collect();
    let (red, blue) = (flat([30, 30, 220, 255]), flat([220, 60, 20, 255]));
    for index in 0..96 {
        writer.write_video(if index < 30 { &red } else if index < 60 { &blue } else { &checker }, width, height).unwrap();
    }
    writer.finish().unwrap();
}

#[gpui_kit::test]
fn split_at_scene_cuts_cuts_a_clip_into_its_shots_in_one_undo(cx: &mut TestAppContext) {
    let folder = scratch_folder("scene-cuts");
    let movie = folder.join("shots.mp4");
    write_three_shots(&movie);
    let (workspace, visual) = open(cx);
    let clip = visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.import_files(vec![movie.clone()]);
        let item = ws.state.selected_item.unwrap();
        ws.state.add_layer_from_item(item, 0, 0);
        // The playhead past the clip, so the viewer isn't decoding it while the test waits.
        let last = ws.state.composition().duration_frames - 1;
        ws.state.set_current_frame(last);
        cx.notify();
        ws.state.selected_layers[0]
    }));
    tall_window(visual);
    // Where the cuts fall in the composition: the first frames showing source frames 30 and 60.
    let (expected, whole) = visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        let composition = state.composition();
        let layer = composition.layer(clip).unwrap();
        let rate = studio_core::time::FrameRate::new(24, 1);
        let first_showing = |source: i64| (layer.in_frame..layer.out_frame).find(|&frame| layer.source_frame(frame, composition.frame_rate, rate) >= source).unwrap();
        (vec![first_showing(30), first_showing(60)], (layer.in_frame, layer.out_frame))
    });

    visual.update(|window, cx| window.right_click(("clip-body", clip.0 as usize), cx));
    render(visual);
    click_in_submenu(visual, "Time", "Split at Scene Cuts");
    wait_for_analysis(&workspace, visual);
    render(visual);
    let pieces = |visual: &mut VisualTestContext| visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        let mut spans: Vec<(i64, i64)> = state.composition().layers.iter().filter(|layer| matches!(layer.content, LayerContent::Footage { item } if state.project.item(item).and_then(|item| item.path.as_ref()) == Some(&movie))).map(|layer| (layer.in_frame, layer.out_frame)).collect();
        spans.sort_unstable();
        spans
    });
    assert_eq!(pieces(visual), vec![(whole.0, expected[0]), (expected[0], expected[1]), (expected[1], whole.1)]);

    // One undo puts the clip back whole.
    visual.update(|window, cx| window.press("cmd-z", cx));
    render(visual);
    assert_eq!(pieces(visual), vec![whole]);

    // Mark Scene Cuts drops a marker at each cut instead.
    visual.update(|window, cx| window.right_click(("clip-body", clip.0 as usize), cx));
    render(visual);
    click_in_submenu(visual, "Time", "Mark Scene Cuts");
    wait_for_analysis(&workspace, visual);
    let markers: Vec<i64> = visual.update(|_, cx| workspace.read(cx).state.composition().markers.iter().filter(|marker| marker.label == "Scene").map(|marker| marker.frame).collect());
    assert_eq!(markers, expected);
    assert_eq!(pieces(visual), vec![whole], "marking leaves the clip whole");
    std::fs::remove_dir_all(&folder).ok();
}

/// The sample project's `A-cam_01.mov` layer.
const A_CAM_LAYER: Id = Id(104);
/// Test clips are this size; the A-cam layer shows them at 1920 × 1080, six times larger.
const TEST_CLIP_SIZE: (u32, u32) = (320, 180);

/// A smooth, detailed made-up picture (0–255) defined everywhere, so it can be drawn moved by
/// any amount: a random value on a 6-pixel lattice, blended between lattice points.
fn test_texture(x: f32, y: f32) -> f32 {
    let corner = |cx: i32, cy: i32| (((cx.wrapping_mul(374_761_393) ^ cy.wrapping_mul(668_265_263)).wrapping_mul(1_274_126_177) >> 8) & 0xff) as f32;
    let (gx, gy) = (x / 6.0, y / 6.0);
    let (left, top) = (gx.floor(), gy.floor());
    let (fx, fy) = (gx - left, gy - top);
    let (left, top) = (left as i32, top as i32);
    let upper = corner(left, top) * (1.0 - fx) + corner(left + 1, top) * fx;
    let lower = corner(left, top + 1) * (1.0 - fx) + corner(left + 1, top + 1) * fx;
    upper * (1.0 - fy) + lower * fy
}

/// Writes a 24 fps ProRes clip of `frames` frames, each pixel's gray drawn by `picture(frame,
/// x, y)`, and sets up the A-cam layer to show such a clip full frame from frame 0 for exactly
/// its length. Returns the clip's path, for `with_clip_file`.
fn use_test_clip(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, name: &str, frames: i64, picture: impl Fn(i64, f32, f32) -> f32) -> std::path::PathBuf {
    let path = scratch_folder(name).join("clip.mov");
    let (width, height) = TEST_CLIP_SIZE;
    let rate = studio_core::time::FrameRate::new(24, 1);
    let mut writer = studio_media::MovieWriter::create(&path, studio_media::MovieSettings::new(studio_media::MovieFormat::ProRes422Hq, width, height, rate, None)).unwrap();
    for frame in 0..frames {
        let bgra: Vec<u8> = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .flat_map(|(x, y)| {
                let gray = picture(frame, x as f32, y as f32).clamp(0.0, 255.0) as u8;
                [gray, gray, gray, 255]
            })
            .collect();
        writer.write_video(&bgra, width, height).unwrap();
    }
    writer.finish().unwrap();
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        let item = ws.state.project.items.iter_mut().find(|item| item.id == studio_core::sample::ids::A_CAM).unwrap();
        item.frame_rate = Some(rate);
        item.size = Some(Vec2::new(1920.0, 1080.0));
        item.duration_seconds = Some(frames as f64 / 24.0);
        let layer = ws.state.composition_mut_for_test().layer_mut(A_CAM_LAYER).unwrap();
        layer.size = Vec2::new(1920.0, 1080.0);
        layer.out_frame = frames;
        layer.transform = Some(studio_core::model::Transform::centered(layer.size, Vec2::new(960.0, 540.0)));
        ws.state.record_history(false);
        cx.notify();
    }));
    path
}

/// Runs `act` (which starts a background job) with the A-cam item pointing at `clip`, then
/// takes the file away again before anything is drawn. The jobs read the path when they start;
/// the viewer's decoder, which would otherwise open the file too, wakes the window from its own
/// thread, and the headless test scheduler doesn't allow that.
fn with_clip_file(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, clip: &std::path::Path, act: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App)) {
    let set_path = |visual: &mut VisualTestContext, path: Option<std::path::PathBuf>| {
        visual.update(|_, cx| workspace.update(cx, |ws, _| {
            ws.state.project.items.iter_mut().find(|item| item.id == studio_core::sample::ids::A_CAM).unwrap().path = path;
        }));
    };
    set_path(visual, Some(clip.to_path_buf()));
    visual.update(act);
    set_path(visual, None);
}

#[gpui_kit::test]
fn track_motion_pins_a_layer_to_a_moving_square(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    // A detailed square moving 4 px right and 1 down each frame over a faint background; its
    // center is at (64 + 4f, 84 + f) in the clip's pixels.
    let center = |frame: i64| Vec2::new(64.0 + 4.0 * frame as f32, 84.0 + frame as f32);
    let clip = use_test_clip(&workspace, visual, "track-square", 48, |frame, x, y| {
        let corner = center(frame) - Vec2::new(24.0, 24.0);
        let inside = (corner.x..corner.x + 48.0).contains(&x) && (corner.y..corner.y + 48.0).contains(&y);
        if inside { test_texture(x - corner.x, y - corner.y) } else { 40.0 + test_texture(x + 500.0, y) * 0.1 }
    });
    // The title (on from frame 24) is pinned from frame 30 to the square's middle, as a click
    // there in the viewer does (`track_motion_waits_for_a_click_on_a_video` covers the click).
    with_clip_file(&workspace, visual, &clip, |window, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        ws.state.set_current_frame(30);
        crate::panels::analysis::track_motion(ws, TITLE, A_CAM_LAYER, center(30) * 6.0, window, cx);
    }));
    render(visual);
    visual.update(|window, _| assert!(window.try_find_by_label("Cancel").is_some(), "the top bar shows the job with a Cancel button"));
    wait_for_analysis(&workspace, visual);
    render(visual);
    let position = |visual: &mut VisualTestContext, frame: i64| match title_value(&workspace, visual, POSITION, frame) {
        PropertyValue::Pair(position) => position,
        other => panic!("position is 2D, not {other:?}"),
    };
    let keys = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().property(POSITION, 0).unwrap().keyframes);
    assert_eq!(keys, (24..48).collect::<Vec<_>>(), "a keyframe on every frame the title and the clip share");
    let at_click = position(visual, 30);
    for frame in 24..48 {
        // Six composition pixels to each of the clip's.
        let expected = at_click + (center(frame) - center(30)) * 6.0;
        let found = position(visual, frame);
        assert!((found - expected).length() < 4.0, "frame {frame}: {found:?}, expected {expected:?}");
    }
    // One undo takes the whole track off.
    visual.update(|window, cx| window.press("cmd-z", cx));
    render(visual);
    let keys = visual.update(|_, cx| workspace.read(cx).state.composition().layer(TITLE).unwrap().property(POSITION, 0).unwrap().keyframes);
    assert_eq!(keys, vec![24, 48], "back to the sample's own two keyframes");
}

#[gpui_kit::test]
fn stabilize_steadies_a_shaky_clip_and_comes_off_in_one_step(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    // The whole picture shakes by a few pixels each frame.
    let shake = |frame: i64| Vec2::new(3.0 * (frame as f32 * 1.7).sin(), 2.0 * (frame as f32 * 2.3).cos());
    let clip = use_test_clip(&workspace, visual, "stabilize-shake", 24, |frame, x, y| {
        let moved = shake(frame);
        test_texture(x - moved.x, y - moved.y)
    });
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(A_CAM_LAYER, false);
        ws.state.set_current_frame(0);
        cx.notify();
    }));
    render(visual);
    let layer = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.composition().layer(A_CAM_LAYER).unwrap().clone());
    let before = layer(visual);
    // The heading's + starts it (pressed while the item has its file).
    with_clip_file(&workspace, visual, &clip, |window, cx| window.click("stabilize-add", cx));
    assert!(visual.update(|_, cx| workspace.read(cx).ui.analysis_job.is_some()), "the job started");
    render(visual);
    visual.update(|window, _| assert!(window.try_find_by_label("Cancel").is_some(), "the top bar shows the job with a Cancel button"));
    wait_for_analysis(&workspace, visual);
    render(visual);

    // Where a fixed point of the scene (the clip's middle on a still frame) is on screen.
    let on_screen = |visual: &mut VisualTestContext, frame: i64| visual.update(|_, cx| {
        let composition = workspace.read(cx).state.composition();
        let in_clip = (Vec2::new(160.0, 90.0) + shake(frame)) * 6.0;
        studio_core::transform::world_transform(composition, A_CAM_LAYER, frame as f64).unwrap().apply(in_clip)
    });
    let jitter = |points: &[Vec2]| points.windows(3).map(|three| (three[2] - three[1] - (three[1] - three[0])).length()).sum::<f32>() / (points.len() - 2) as f32;
    let steadied: Vec<Vec2> = (0..24).map(|frame| on_screen(visual, frame)).collect();
    let stabilized = layer(visual);
    let stabilization = stabilized.stabilization.as_ref().expect("stabilized");
    assert!(stabilization.zoom > 1.0, "enlarged to hide the edges: {}", stabilization.zoom);
    let shaky: Vec<Vec2> = (0..24).map(|frame| (Vec2::new(160.0, 90.0) + shake(frame)) * 6.0).collect();
    let (shaky, steadied) = (jitter(&shaky), jitter(&steadied));
    assert!(steadied < shaky * 0.25, "the scene shakes {shaky:.1} px a frame before and {steadied:.1} after");

    // The smoothness menu steadies it again from what was measured.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        if !ws.ui.section_open("stabilize") {
            ws.ui.toggle_section("stabilize");
        }
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click("stabilize-smoothness", cx));
    render(visual);
    click_label(visual, "25%");
    render(visual);
    assert_eq!(layer(visual).stabilization.unwrap().smoothness, 25.0);

    // Undo goes back through each step: the smoothness, then stabilizing itself.
    visual.update(|window, cx| window.press("cmd-z", cx));
    render(visual);
    assert_eq!(layer(visual), stabilized);
    visual.update(|window, cx| window.press("cmd-z", cx));
    render(visual);
    assert_eq!(layer(visual), before, "one undo puts the layer back exactly");
    visual.update(|window, cx| window.press("cmd-shift-z", cx));
    render(visual);
    assert_eq!(layer(visual), stabilized);
    // The heading's trash takes it off, exactly, in one step too.
    visual.update(|window, cx| window.click("stabilize-remove", cx));
    render(visual);
    assert_eq!(layer(visual), before);
    visual.update(|window, cx| window.press("cmd-z", cx));
    render(visual);
    assert_eq!(layer(visual), stabilized);
}

#[gpui_kit::test]
fn stabilize_is_offered_for_video_only_and_explains_a_missing_file(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    tall_window(visual);
    // Text has no Stabilize.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(TITLE, false);
        cx.notify();
    }));
    render(visual);
    assert!(visual.update(|window, _| window.try_find("stabilize-add")).is_none());
    // The sample's video has no file behind it: the job doesn't start.
    visual.update(|_, cx| workspace.update(cx, |ws, cx| {
        ws.state.select_layer(A_CAM_LAYER, false);
        cx.notify();
    }));
    render(visual);
    visual.update(|window, cx| window.click("stabilize-add", cx));
    render(visual);
    assert!(visual.update(|_, cx| workspace.read(cx).ui.analysis_job.is_none()));
    assert!(visual.update(|_, cx| workspace.read(cx).state.composition().layer(A_CAM_LAYER).unwrap().stabilization.is_none()));
    // The right-click menu offers it for the clip too.
    visual.update(|window, cx| window.right_click(("clip-body", A_CAM_LAYER.0 as usize), cx));
    render(visual);
    click_label(visual, "Transform");
    render(visual);
    assert!(visual.update(|window, _| window.try_find_by_label("Stabilize")).is_some());
}

#[gpui_kit::test]
fn a_text_clips_menu_leaves_out_what_cant_apply_to_it(cx: &mut TestAppContext) {
    let (_workspace, visual) = open(cx);
    tall_window(visual);
    visual.update(|window, cx| window.right_click(("clip-body", TITLE.0 as usize), cx));
    render(visual);
    let shown = |visual: &mut VisualTestContext, label: &str| visual.update(|window, _| window.try_find_by_label(label).is_some());
    for missing in ["Ungroup Composition", "Open Composition", "Morph To…", "Convert to Path"] {
        assert!(!shown(visual, missing), "{missing} is left out for text");
    }
    for present in ["Rename…", "Duplicate", "Split at Playhead", "Delete", "Time", "Transform", "Export"] {
        assert!(shown(visual, present), "{present} is in the menu");
    }
    // Video-only timing items stay out of the Time submenu; Close Gaps is there.
    click_label(visual, "Time");
    render(visual);
    assert!(!shown(visual, "Insert Freeze Frame") && !shown(visual, "Split at Scene Cuts"));
    assert!(shown(visual, "Close Gaps on Track"));
}

#[gpui_kit::test]
fn tools_and_features_turned_off_are_hidden_everywhere(cx: &mut TestAppContext) {
    use crate::features::Feature;
    let (workspace, visual) = open(cx);
    tall_window(visual);
    let shown = |visual: &mut VisualTestContext, id: &'static str| visual.update(|window, _| window.try_find(id).is_some());
    for id in ["tool-pen", "tool-group-edit", "rail-export", "rail-effects", "viewer-guides"] {
        assert!(shown(visual, id), "{id} starts out shown");
    }

    // Settings ▸ Tools & Features (the fifth page): the Pen, Export, the Effects panel, Guides,
    // Close Gaps and every clip-editing tool go.
    visual.update(|window, cx| window.press("cmd-,", cx));
    render(visual);
    visual.update(|window, cx| window.click(("settings-section", 4_usize), cx));
    render(visual);
    let over_page = visual.update(|window, _| window.find("feature-track-select").bounds().center());
    for feature in ["cut-tool", "ripple-tool", "roll-tool", "rate-stretch-tool", "slip-tool", "slide-tool", "pen-tool", "effects", "close-gaps", "guides", "export"] {
        click_in_settings(visual, over_page, &format!("feature-{feature}"));
    }
    let settings = visual.update(|_, cx| workspace.read(cx).ui.settings.clone());
    assert!(!settings.feature_on(Feature::PenTool) && !settings.feature_on(Feature::Export) && !settings.feature_on(Feature::Guides));
    assert!(!crate::app_settings::load().feature_on(Feature::PenTool), "remembered");
    visual.update(|window, cx| window.press("escape", cx));
    render(visual);

    // Gone from the tool bar (the whole clip-editing group, as none of it is left), the rail
    // and the viewer.
    for id in ["tool-pen", "tool-group-edit", "rail-export", "rail-effects", "viewer-guides"] {
        assert!(!shown(visual, id), "{id} is hidden");
    }
    for id in ["tool-group-select", "tool-group-view", "rail-new-comp", "rail-import", "rail-settings", "rail-media"] {
        assert!(shown(visual, id), "{id} is always there");
    }
    // Its key does nothing.
    visual.update(|window, cx| window.press("g", cx));
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).ui.tool), crate::workspace::Tool::Selection);
    // Gone from the menu bar, leaving no stray separators.
    let menu_names = |menu: &str| -> Vec<String> {
        crate::workspace::menus()
            .into_iter()
            .find(|candidate| candidate.name == menu)
            .unwrap()
            .items
            .into_iter()
            .map(|item| match item {
                gpui_kit::MenuItem::Action { name, .. } => name.to_string(),
                gpui_kit::MenuItem::Separator => "-".to_owned(),
                gpui_kit::MenuItem::Submenu(menu) => menu.name.to_string(),
                _ => "?".to_owned(),
            })
            .collect()
    };
    let file = menu_names("File");
    assert!(!file.iter().any(|name| name.starts_with("Export")), "{file:?}");
    assert!(file.contains(&"Import…".to_owned()) && file.contains(&"Package…".to_owned()));
    assert!(!menu_names("Edit").contains(&"Close Gaps".to_owned()));
    assert_eq!(menu_names("View"), ["Zoom In", "Zoom Out"], "the separator before Show Guides goes with it");
    // And from a clip's right-click menu: no Close Gaps in Time, and a text layer has nothing
    // else there, so the submenu goes.
    visual.update(|window, cx| window.right_click(("clip-body", TITLE.0 as usize), cx));
    render(visual);
    assert!(visual.update(|window, _| window.try_find_by_label("Time")).is_none());
    assert!(visual.update(|window, _| window.try_find_by_label("Transform")).is_some());
    visual.update(|window, cx| window.press("escape", cx));
    render(visual);

    // Turn All On brings everything back.
    visual.update(|window, cx| window.press("cmd-,", cx));
    render(visual);
    visual.update(|window, cx| window.click(("settings-section", 4_usize), cx));
    render(visual);
    let over_page = visual.update(|window, _| window.find("feature-track-select").bounds().center());
    click_in_settings(visual, over_page, "features-all-on");
    visual.update(|window, cx| window.press("escape", cx));
    render(visual);
    for id in ["tool-pen", "tool-group-edit", "rail-export", "rail-effects", "viewer-guides"] {
        assert!(shown(visual, id), "{id} is back");
    }
    visual.update(|window, cx| window.press("g", cx));
    render(visual);
    assert_eq!(visual.update(|_, cx| workspace.read(cx).ui.tool), crate::workspace::Tool::Pen);
}

/// Clicks `id` on the open Settings page, scrolling the page down first until it is in view.
/// The wheel turns at `over_page`, a point on the page (taken from a control in view when it opened).
fn click_in_settings(visual: &mut VisualTestContext, over_page: gpui_kit::Point<gpui_kit::Pixels>, id: &str) {
    let element = || gpui_kit::ElementId::Name(id.to_owned().into());
    for _ in 0..40 {
        if visual.update(|window, _| window.try_find(element()).is_some_and(|found| found.visible())) {
            break;
        }
        visual.update(|window, cx| {
            use gpui_kit::InputEvent as _;
            let wheel = gpui_kit::ScrollEvent { position: over_page, delta: gpui_kit::ScrollDelta::Pixels(point(px(0.0), px(-80.0))), ..Default::default() };
            window.dispatch_event(wheel.to_platform_input(), cx);
        });
        render(visual);
    }
    visual.update(|window, cx| window.click(element(), cx));
    render(visual);
}

#[gpui_kit::test]
fn the_welcome_setup_runs_on_first_launch_and_applies_each_choice(cx: &mut TestAppContext) {
    use crate::features::Feature;
    use crate::panels::onboarding::Step;
    use crate::theme::{Accent, ThemeName};
    let (workspace, visual) = open(cx);
    tall_window(visual);
    let step = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).ui.onboarding);
    let settings = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).ui.settings.clone());
    let click = |visual: &mut VisualTestContext, id: ElementId| {
        visual.update(|window, cx| window.click(id, cx));
        render(visual);
    };
    // A first launch: the setup shows over the start screen.
    assert!(!crate::app_settings::load().onboarded);
    visual.update(|window, cx| workspace.update(cx, |ws, cx| ws.restore_session(window, cx)));
    render(visual);
    assert_eq!(step(visual), Some(Step::Look));
    assert!(start_screen_showing(&workspace, visual), "the start screen waits underneath");
    // Escape doesn't skip it by accident, and the editor's shortcuts wait too.
    visual.update(|window, cx| window.press("escape", cx));
    render(visual);
    assert_eq!(step(visual), Some(Step::Look));

    // 1. Look: Light, teal. Each applies straight away.
    click(visual, ("theme", ThemeName::Light as usize).into());
    click(visual, ("accent", Accent::Teal as usize).into());
    assert_eq!((settings(visual).theme, settings(visual).accent), (ThemeName::Light, Accent::Teal));
    click(visual, "onboarding-next".into());
    assert_eq!(step(visual), Some(Step::Features));

    // 2. Features: "Just the basics", then the Pen back on by hand.
    click(visual, ("onboarding-focus", 3_usize).into());
    assert!(!settings(visual).feature_on(Feature::PenTool) && !settings(visual).feature_on(Feature::GraphEditor));
    assert!(settings(visual).feature_on(Feature::CutTool));
    click(visual, ElementId::Name("onboarding-feature-pen-tool".into()));
    assert!(settings(visual).feature_on(Feature::PenTool));
    click(visual, "onboarding-next".into());

    // 3. AI: on by default; No AI hides it all, and Use local AI brings back a choice of each.
    assert_eq!(step(visual), Some(Step::Ai));
    click(visual, "onboarding-ai-off".into());
    assert!(!settings(visual).ai.any());
    assert!(visual.update(|window, _| window.try_find(ElementId::Name("onboarding-feature-captions".into()))).is_none(), "no AI choices while it's off");
    click(visual, "onboarding-ai-on".into());
    click(visual, ElementId::Name("onboarding-feature-captions".into()));
    let ai = settings(visual).ai;
    assert!(!ai.captions && ai.remove_background && ai.auto_reframe && ai.enhance_speech);
    click(visual, "onboarding-back".into());
    assert_eq!(step(visual), Some(Step::Features), "Back goes back");
    click(visual, "onboarding-next".into());
    click(visual, "onboarding-ai-off".into());
    click(visual, "onboarding-next".into());

    // 4. Done: a summary, then Start Creating closes it for good.
    assert_eq!(step(visual), Some(Step::Done));
    assert!(visual.update(|window, _| window.try_find("onboarding-summary")).is_some());
    click(visual, "onboarding-next".into());
    assert_eq!(step(visual), None);
    let saved = crate::app_settings::load();
    assert!(saved.onboarded);
    assert_eq!((saved.theme, saved.accent), (ThemeName::Light, Accent::Teal));
    assert!(!saved.feature_on(Feature::GraphEditor) && saved.feature_on(Feature::PenTool) && !saved.ai.any(), "every choice is remembered");
    // The editor behind follows: no Captions on the rail, no graph editor button.
    visual.update(|window, cx| window.press("escape", cx));
    render(visual);
    for hidden in ["rail-captions", "easings"] {
        assert!(visual.update(|window, _| window.try_find(hidden)).is_none(), "{hidden} is hidden");
    }

    // The next launch goes straight to the start screen.
    visual.update(|window, cx| workspace.update(cx, |ws, cx| {
        ws.ui.start_screen = None;
        ws.restore_session(window, cx);
    }));
    render(visual);
    assert_eq!(step(visual), None);

    // Settings ▸ General can show it again; Skip Setup keeps what's there.
    visual.update(|window, cx| window.press("escape", cx));
    render(visual);
    visual.update(|window, cx| window.press("cmd-,", cx));
    render(visual);
    click(visual, "general-welcome".into());
    assert_eq!(step(visual), Some(Step::Look));
    click(visual, "onboarding-skip".into());
    assert_eq!(step(visual), None);
    assert_eq!(settings(visual).theme, ThemeName::Light);
}

#[gpui_kit::test]
fn the_start_screens_home_shows_a_welcome_and_a_strip_of_templates(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|window, cx| workspace.update(cx, |ws, cx| crate::panels::start_screen::show(ws, window, cx)));
    render(visual);
    for id in ["start-hero", "start-template-strip", "start-recent-empty"] {
        assert!(visual.update(|window, _| window.try_find(id)).is_some(), "{id} shows");
    }
    // A template in the strip starts a project from it, as on the Templates page.
    assert!(visual.update(|window, _| window.try_find(("start-template", 0_usize))).is_some());
    // "All templates" goes to the Templates page.
    visual.update(|window, cx| window.click("start-see-templates", cx));
    render(visual);
    let tab = visual.update(|_, cx| workspace.read(cx).ui.start_screen.as_ref().map(|screen| screen.tab));
    assert_eq!(tab, Some(crate::panels::start_screen::StartTab::Templates));
}
