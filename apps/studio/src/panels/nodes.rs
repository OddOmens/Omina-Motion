//! The Nodes panel: the selected layer's effects as a node graph you wire freely, like
//! Fusion's node editor. Pictures flow left to right along wires from a node's output (the dot
//! on its right edge) into another node's input (the dots on its left edge).
//!
//! - Drag from an output to an input (or the other way) to connect them. An input takes one
//!   wire; connecting a new one replaces the old. One output can feed many inputs.
//! - Drag a wire off an input to move it to another input, or drop it on empty canvas to
//!   disconnect it.
//! - Drag a node's title to move it. Drag the empty canvas (or scroll) to look around;
//!   ⌘-scroll zooms. Right-click the canvas to add a node, or a node to turn it off or delete it.
//! - Clicking a node picks it; its settings show in the Effects tab (the Object Matte's in
//!   the Properties tab).

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, WindowExt as _};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, DragMoveEvent, ElementId, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    PathBuilder, Pixels, Point, ScrollEvent, StatefulInteractiveElement, Styled, TestSupportExt as _, canvas, div, point,
    prelude::FluentBuilder as _, px,
};
use studio_core::animation::Vec2;
use studio_core::effect_graph::{self, InputPort, NodeId};
use studio_core::model::{Id, Layer};

use super::chrome::{EmptyDragView, MeasuredBounds, hint, panel};
use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, Workspace};

/// The Nodes panel's own state: whether it is open, where the canvas is looking, what is
/// picked, and the drag under way.
#[derive(Clone)]
pub struct NodesUi {
    pub open: bool,
    /// The canvas point (in canvas units) at the panel's top-left corner.
    pub pan: Vec2,
    /// Screen pixels per canvas unit.
    pub zoom: f32,
    /// The node picked last, whose settings the Effects tab shows.
    pub selected: Option<NodeId>,
    /// Where on its title a dragged node was grabbed, in screen pixels from its corner.
    grab: Vec2,
    /// The pan and pointer when a drag of the empty canvas began.
    pan_start: Option<(Vec2, Point<Pixels>)>,
    /// The wire being dragged and where the pointer is, for drawing its loose end.
    wire: Option<(WireDrag, Point<Pixels>)>,
    /// Where the canvas was right-clicked, in canvas units: new nodes from its menu go there.
    menu_at: Vec2,
    /// Where the canvas is on screen.
    bounds: MeasuredBounds,
    /// Where each Merge's opacity slider is on screen (in the Effects tab), by Merge id.
    opacity_sliders: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<Id, MeasuredBounds>>>,
}

impl Default for NodesUi {
    fn default() -> Self {
        Self {
            open: false,
            pan: Vec2::new(-40.0, -60.0),
            zoom: 1.0,
            selected: None,
            grab: Vec2::new(0.0, 0.0),
            pan_start: None,
            wire: None,
            menu_at: Vec2::new(0.0, 0.0),
            bounds: MeasuredBounds::default(),
            opacity_sliders: Default::default(),
        }
    }
}

/// Node sizes in canvas units (multiplied by the zoom on screen).
const NODE_WIDTH: f32 = 150.0;
const TITLE_HEIGHT: f32 = 24.0;
const PORT_ROW: f32 = 20.0;
const PORT_DOT: f32 = 10.0;
/// The hit area around a port's dot, so it is easy to grab, in screen pixels.
const PORT_HIT: f32 = 18.0;
const ZOOM_RANGE: (f32, f32) = (0.35, 2.0);

/// A node being moved by its title.
#[derive(Clone)]
struct NodeMove {
    layer: Id,
    node: NodeId,
}

/// A wire being dragged: from an output looking for an input, or from an input looking for an
/// output. `lifted` is the input a wire was pulled off, which loses it unless dropped back.
#[derive(Clone)]
struct WireDrag {
    layer: Id,
    output: Option<NodeId>,
    input: Option<(NodeId, InputPort)>,
    lifted: Option<(NodeId, InputPort)>,
}

/// The empty canvas being dragged to look around.
#[derive(Clone)]
struct CanvasPan;

/// How tall a node is on the canvas, in canvas units.
fn node_height(node: NodeId) -> f32 {
    TITLE_HEIGHT + PORT_ROW * effect_graph::inputs(node).len().max(1) as f32 + 6.0
}

impl NodesUi {
    /// A canvas point's place in the canvas element, in screen pixels from its corner.
    fn to_local(&self, at: Vec2) -> Vec2 {
        Vec2::new((at.x - self.pan.x) * self.zoom, (at.y - self.pan.y) * self.zoom)
    }

    /// The canvas point under a window position.
    fn to_canvas(&self, window_point: Point<Pixels>) -> Vec2 {
        let bounds = self.bounds.get();
        Vec2::new((f32::from(window_point.x) - bounds.left) / self.zoom + self.pan.x, (f32::from(window_point.y) - bounds.top) / self.zoom + self.pan.y)
    }

    /// Zooms by `factor`, keeping the canvas point under `window_point` where it is.
    fn zoom_about(&mut self, factor: f32, window_point: Point<Pixels>) {
        let before = self.to_canvas(window_point);
        self.zoom = (self.zoom * factor).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        let after = self.to_canvas(window_point);
        self.pan = Vec2::new(self.pan.x + before.x - after.x, self.pan.y + before.y - after.y);
    }

    /// Frames every node of `layer` in the canvas, at most at full size.
    fn fit(&mut self, layer: &Layer) {
        let spots = effect_graph::layout(layer);
        let Some(first) = spots.first() else { return };
        let (mut low, mut high) = (first.1, first.1);
        for (node, at) in &spots {
            low = Vec2::new(low.x.min(at.x), low.y.min(at.y));
            high = Vec2::new(high.x.max(at.x + NODE_WIDTH), high.y.max(at.y + node_height(*node)));
        }
        let bounds = self.bounds.get();
        let margin = 40.0;
        let (width, height) = ((high.x - low.x).max(1.0), (high.y - low.y).max(1.0));
        if bounds.width > margin * 2.0 && bounds.height > margin * 2.0 {
            self.zoom = ((bounds.width - margin * 2.0) / width).min((bounds.height - margin * 2.0) / height).clamp(ZOOM_RANGE.0, 1.0);
        }
        let center = Vec2::new((low.x + high.x) / 2.0, (low.y + high.y) / 2.0);
        self.pan = Vec2::new(center.x - bounds.width / 2.0 / self.zoom, center.y - bounds.height / 2.0 / self.zoom);
    }
}

/// The node's title, icon and whether it is turned off.
fn node_face(layer: &Layer, node: NodeId) -> (String, IconName, bool) {
    match node {
        NodeId::Source => (layer.name.clone(), IconName::Film, false),
        NodeId::Output => ("Output".to_owned(), IconName::MonitorPlay, false),
        NodeId::Matte => ("Object Matte".to_owned(), IconName::Scan, false),
        NodeId::Effect(id) => layer
            .effects
            .iter()
            .find(|effect| effect.id == id)
            .map_or(("Effect".to_owned(), IconName::Sparkles, false), |effect| (effect.name.clone(), super::effect_controls::category_icon(&effect.name), !effect.enabled)),
        NodeId::Merge(id) => {
            let merge = layer.effect_graph.merges.iter().find(|merge| merge.id == id);
            let title = merge.map_or_else(|| "Merge".to_owned(), |merge| format!("Merge · {}", merge.blend.label()));
            (title, IconName::Blend, merge.is_some_and(|merge| !merge.enabled))
        }
    }
}

/// A stable text form of a node for element ids.
fn node_key(node: NodeId) -> String {
    match node {
        NodeId::Source => "source".to_owned(),
        NodeId::Output => "output".to_owned(),
        NodeId::Matte => "matte".to_owned(),
        NodeId::Effect(id) => format!("effect-{}", id.0),
        NodeId::Merge(id) => format!("merge-{}", id.0),
    }
}

fn port_key(port: InputPort) -> &'static str {
    match port {
        InputPort::Main => "main",
        InputPort::Mask => "mask",
        InputPort::Background => "background",
        InputPort::Foreground => "foreground",
    }
}

/// The element id of a node, of one of its inputs, and of its output (tests use these too).
pub fn node_id(node: NodeId) -> ElementId {
    ElementId::Name(format!("node:{}", node_key(node)).into())
}
pub fn input_id(node: NodeId, port: InputPort) -> ElementId {
    ElementId::Name(format!("node-in:{}:{}", node_key(node), port_key(port)).into())
}
pub fn output_id(node: NodeId) -> ElementId {
    ElementId::Name(format!("node-out:{}", node_key(node)).into())
}

/// Where an input's dot is in the canvas element, in screen pixels.
fn input_spot(ui: &NodesUi, at: Vec2, node: NodeId, port: InputPort) -> Vec2 {
    let row = effect_graph::inputs(node).iter().position(|p| *p == port).unwrap_or(0) as f32;
    ui.to_local(Vec2::new(at.x, at.y + TITLE_HEIGHT + PORT_ROW * (row + 0.5)))
}

/// Where a node's output dot is in the canvas element, in screen pixels.
fn output_spot(ui: &NodesUi, at: Vec2) -> Vec2 {
    ui.to_local(Vec2::new(at.x + NODE_WIDTH, at.y + TITLE_HEIGHT + PORT_ROW * 0.5))
}

/// The selected layer the panel shows, when exactly one layer is selected.
fn shown_layer(workspace: &Workspace) -> Option<&Layer> {
    let composition = workspace.state.composition();
    match workspace.state.selected_layers.as_slice() {
        [id] => composition.layer(*id),
        _ => None,
    }
}

/// Picks `node`: its settings show in the Effects tab (the Object Matte's in Properties).
fn pick(workspace: &mut Workspace, node: NodeId) {
    workspace.ui.nodes.selected = Some(node);
    // A correction the Color tab edits opens there; it is only a line in the Effects tab.
    let graded = shown_layer(workspace).map(|layer| super::color_grade::graded_effects(workspace, layer.id)).unwrap_or_default();
    match node {
        NodeId::Effect(effect) if graded.contains(&effect) => {
            workspace.ui.tabs.insert(PanelId::Properties, 2);
        }
        NodeId::Effect(_) | NodeId::Merge(_) => {
            workspace.ui.tabs.insert(PanelId::Properties, 1);
        }
        NodeId::Matte => {
            workspace.ui.tabs.insert(PanelId::Properties, 0);
        }
        NodeId::Source | NodeId::Output => {}
    }
}

/// Deletes the picked node (Delete while the Nodes panel is active). Returns whether there was
/// one to delete.
pub fn delete_selected(workspace: &mut Workspace) -> bool {
    let Some(layer) = shown_layer(workspace).map(|layer| layer.id) else { return false };
    let Some(node) = workspace.ui.nodes.selected.filter(|node| !matches!(node, NodeId::Source | NodeId::Output)) else { return false };
    workspace.state.remove_graph_node(layer, node);
    workspace.ui.nodes.selected = None;
    true
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let body = match shown_layer(workspace) {
        Some(layer) => graph_canvas(layer, workspace, cx),
        None => hint("Select one layer to edit its effects as nodes.").into_any_element(),
    };
    let footer = shown_layer(workspace).map(|layer| footer(layer, workspace, cx).into_any_element());
    panel(PanelId::Nodes, &["Nodes"], footer, body, workspace, cx)
}

/// The strip under the canvas: add a node, fit everything in view, the zoom, and close.
fn footer(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let layer_id = layer.id;
    let weak = cx.weak_entity();
    // Object Matte runs an AI model, so it is left out while it is turned off in Settings.
    let matte = workspace.ui.settings.ai.remove_background.then(|| studio_core::cutout::can_cut_out(layer) && !layer.effect_graph.matte_node);
    let names = suitable_effects(layer);
    let add = Button::new("nodes-add").ghost().small().icon(Icon::new(IconName::Plus)).tooltip("Add a node").dropdown_menu(move |menu, _, _| {
        // From the footer, new nodes go in the middle of the view.
        let weak = weak.clone();
        add_node_items(menu, weak, layer_id, None, matte, &names)
    });
    div()
        .flex()
        .items_center()
        .gap_1()
        .px_1()
        .h_full()
        .text_size(px(size::SMALL_TEXT))
        .text_color(hsla(color::MUTED))
        .child(add)
        .child(
            Button::new("nodes-fit").ghost().small().icon(Icon::new(IconName::Scan)).tooltip("Fit all nodes in view").on_click(cx.listener(move |this, _, _, cx| {
                if let Some(layer) = this.state.composition().layer(layer_id).cloned() {
                    this.ui.nodes.fit(&layer);
                }
                cx.notify();
            })),
        )
        .child(
            Button::new("nodes-zoom-out").ghost().small().icon(Icon::new(IconName::ZoomOut)).tooltip("Zoom out").on_click(cx.listener(|this, _, _, cx| {
                let center = canvas_center(&this.ui.nodes);
                this.ui.nodes.zoom_about(1.0 / 1.25, center);
                cx.notify();
            })),
        )
        .child(div().w(px(36.0)).text_center().child(format!("{:.0}%", workspace.ui.nodes.zoom * 100.0)))
        .child(
            Button::new("nodes-zoom-in").ghost().small().icon(Icon::new(IconName::ZoomIn)).tooltip("Zoom in").on_click(cx.listener(|this, _, _, cx| {
                let center = canvas_center(&this.ui.nodes);
                this.ui.nodes.zoom_about(1.25, center);
                cx.notify();
            })),
        )
        .child(div().flex_1().min_w_0().truncate().child(if layer.effect_graph.wired { "Wired by hand" } else { "Effects run in list order until you rewire them" }))
        .child(
            Button::new("nodes-close").ghost().small().icon(Icon::new(IconName::X)).tooltip("Close the Nodes panel").on_click(cx.listener(|this, _, _, cx| {
                this.ui.nodes.open = false;
                cx.notify();
            })),
        )
}

/// The middle of the canvas on screen.
fn canvas_center(ui: &NodesUi) -> Point<Pixels> {
    let bounds = ui.bounds.get();
    point(px(bounds.left + bounds.width / 2.0), px(bounds.top + bounds.height / 2.0))
}

/// Every effect the renderer draws that suits the layer, grouped by category.
fn suitable_effects(layer: &Layer) -> Vec<(&'static str, Vec<&'static str>)> {
    studio_core::effects::CATEGORIES
        .iter()
        .filter(|category| studio_core::effects::suits(category, &layer.content))
        .map(|category| (category.name, category.effects.iter().map(|effect| effect.name).filter(|name| studio_render::effects::is_rendered(name)).collect::<Vec<_>>()))
        .filter(|(_, effects)| !effects.is_empty())
        .collect()
}

/// The items of an Add Node menu: Merge, Object Matte, then every suitable effect. New nodes go
/// at `at` (canvas units), or in the middle of the view when `at` is `None`. `matte` is whether
/// an Object Matte can be added, or `None` to leave it out of the menu.
fn add_node_items(
    mut menu: PopupMenu,
    weak: gpui_kit::WeakEntity<Workspace>,
    layer: Id,
    at: Option<Vec2>,
    matte: Option<bool>,
    effects: &[(&'static str, Vec<&'static str>)],
) -> PopupMenu {
    // Where a new node goes: the right-clicked spot from the context menu, else the middle,
    // moved down clear of the nodes already there.
    let spot = move |workspace: &Workspace| {
        let ui = &workspace.ui.nodes;
        let wanted = at.unwrap_or_else(|| ui.to_canvas(canvas_center(ui)));
        workspace.state.composition().layer(layer).map_or(wanted, |layer| free_spot(layer, wanted))
    };
    let added = move |workspace: &mut Workspace, node: Option<NodeId>| {
        if let Some(node) = node {
            pick(workspace, node);
        }
    };
    menu = menu.label("Combine").item(PopupMenuItem::new("Merge").on_click({
        let weak = weak.clone();
        move |_, _, cx| {
            weak.update(cx, |this, cx| {
                let at = spot(this);
                let node = this.state.add_merge_node(layer, at);
                added(this, node);
                cx.notify();
            })
            .ok();
        }
    }));
    if let Some(can_matte) = matte {
        menu = menu.item(PopupMenuItem::new("Object Matte").disabled(!can_matte).on_click({
            let weak = weak.clone();
            move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    let at = spot(this);
                    if this.state.add_matte_node(layer, at) {
                        added(this, Some(NodeId::Matte));
                    }
                    cx.notify();
                })
                .ok();
            }
        }));
    }
    for (category, names) in effects {
        menu = menu.separator().label(*category);
        for &name in names {
            let weak = weak.clone();
            menu = menu.item(PopupMenuItem::new(name).on_click(move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    let at = spot(this);
                    let node = this.state.add_effect_node(layer, name, at);
                    added(this, node);
                    cx.notify();
                })
                .ok();
            }));
        }
    }
    menu
}

/// The first spot at or below `wanted` where a new node covers no other node.
fn free_spot(layer: &Layer, wanted: Vec2) -> Vec2 {
    let spots = effect_graph::layout(layer);
    let overlaps = |at: Vec2| {
        spots.iter().any(|(node, other)| {
            at.x < other.x + NODE_WIDTH + 10.0 && other.x < at.x + NODE_WIDTH + 10.0 && at.y < other.y + node_height(*node) + 10.0 && other.y < at.y + node_height(NodeId::Merge(Id(0))) + 10.0
        })
    };
    let mut at = wanted;
    // A bounded search: a crowded canvas still gets its node, just below the crowd.
    for _ in 0..100 {
        if !overlaps(at) {
            break;
        }
        at.y += effect_graph::LAYOUT_ROW / 2.0;
    }
    at
}

/// The canvas: wires drawn underneath, nodes on top, and the drag handling for both.
fn graph_canvas(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = &workspace.ui.nodes;
    let layer_id = layer.id;
    let spots = effect_graph::layout(layer);
    let spot_of = |node: NodeId| spots.iter().find(|(placed, _)| *placed == node).map_or(Vec2::new(0.0, 0.0), |(_, at)| *at);

    // Wires as pairs of local points (output dot → input dot). A wire being pulled off an
    // input is drawn to the pointer instead.
    // The wire under way (only while a drag is: one dropped outside the panel leaves no trace).
    let wire = ui.wire.clone().filter(|_| cx.has_active_drag());
    let lifted = wire.as_ref().and_then(|(drag, _)| drag.lifted);
    let mut wires: Vec<(Vec2, Vec2, bool)> = effect_graph::links(layer)
        .iter()
        .filter(|link| Some((link.to, link.port)) != lifted)
        .map(|link| (output_spot(ui, spot_of(link.from)), input_spot(ui, spot_of(link.to), link.to, link.port), link.port == InputPort::Mask))
        .collect();
    if let Some((drag, end)) = wire {
        let bounds = ui.bounds.get();
        let pointer = Vec2::new(f32::from(end.x) - bounds.left, f32::from(end.y) - bounds.top);
        match (drag.output, drag.input) {
            (Some(output), _) => wires.push((output_spot(ui, spot_of(output)), pointer, false)),
            (None, Some((node, port))) => wires.push((pointer, input_spot(ui, spot_of(node), node, port), false)),
            (None, None) => {}
        }
    }
    let zoom = ui.zoom;
    let wire_layer = canvas(|_, _, _| (), move |bounds, _, window, _| {
        for (from, to, mask) in &wires {
            let start = point(bounds.origin.x + px(from.x), bounds.origin.y + px(from.y));
            let end = point(bounds.origin.x + px(to.x), bounds.origin.y + px(to.y));
            // Wires leave outputs to the right and enter inputs from the left.
            let reach = px(((to.x - from.x).abs() * 0.5).max(40.0 * zoom));
            let mut path = PathBuilder::stroke(px((2.0 * zoom).max(1.0)));
            path.move_to(start);
            path.cubic_bezier_to(end, point(start.x + reach, start.y), point(end.x - reach, end.y));
            if let Ok(path) = path.build() {
                window.paint_path(path, hsla(if *mask { color::VALUE } else { color::MUTED }));
            }
        }
    })
    .absolute()
    .size_full();

    let nodes: Vec<AnyElement> = spots.iter().map(|(node, at)| graph_node(layer, *node, *at, workspace, cx)).collect();

    // Object Matte runs an AI model, so it is left out while it is turned off in Settings.
    let matte = workspace.ui.settings.ai.remove_background.then(|| studio_core::cutout::can_cut_out(layer) && !layer.effect_graph.matte_node);
    let names = suitable_effects(layer);
    let weak = cx.weak_entity();
    div()
        .id("nodes-canvas")
        .relative()
        .size_full()
        .overflow_hidden()
        .bg(hsla(color::INSET))
        .child(ui.bounds.probe())
        .child(wire_layer)
        .children(nodes)
        // Pressing the empty canvas starts looking around (and lets go of the picked node).
        .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| {
            this.ui.nodes.pan_start = Some((this.ui.nodes.pan, event.position));
            this.ui.nodes.selected = None;
            cx.notify();
        }))
        .on_mouse_down(MouseButton::Right, cx.listener(|this, event: &MouseDownEvent, _, _| {
            this.ui.nodes.menu_at = this.ui.nodes.to_canvas(event.position);
        }))
        .on_drag(CanvasPan, |_, _, _, cx| cx.new(|_| EmptyDragView))
        .on_drag_move(cx.listener(|this, event: &DragMoveEvent<CanvasPan>, _, cx| {
            if let Some((pan, start)) = this.ui.nodes.pan_start {
                let zoom = this.ui.nodes.zoom;
                let moved = event.event.position - start;
                this.ui.nodes.pan = Vec2::new(pan.x - f32::from(moved.x) / zoom, pan.y - f32::from(moved.y) / zoom);
                cx.notify();
            }
        }))
        .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<NodeMove>, _, cx| {
            let drag = event.drag(cx).clone();
            let ui = &this.ui.nodes;
            let corner = point(event.event.position.x - px(ui.grab.x), event.event.position.y - px(ui.grab.y));
            let at = ui.to_canvas(corner);
            this.state.place_node(drag.layer, drag.node, at);
            cx.notify();
        }))
        .on_drag_move(cx.listener(|this, event: &DragMoveEvent<WireDrag>, _, cx| {
            this.ui.nodes.wire = Some((event.drag(cx).clone(), event.event.position));
            cx.notify();
        }))
        // A wire dropped on empty canvas: one pulled off an input is disconnected.
        .on_drop(cx.listener(|this, drag: &WireDrag, _, cx| {
            this.ui.nodes.wire = None;
            if let Some((node, port)) = drag.lifted {
                this.state.disconnect_node(drag.layer, node, port);
            }
            cx.notify();
        }))
        .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
            let delta = event.delta.pixel_delta(px(16.0));
            let ui = &mut this.ui.nodes;
            if event.modifiers.platform || event.modifiers.control {
                ui.zoom_about((f32::from(delta.y) / 200.0).exp(), event.position);
            } else {
                ui.pan = Vec2::new(ui.pan.x - f32::from(delta.x) / ui.zoom, ui.pan.y - f32::from(delta.y) / ui.zoom);
            }
            cx.notify();
        }))
        .test_support()
        .context_menu(move |menu, _, cx| {
            // New nodes go where the canvas was right-clicked.
            let at = weak.upgrade().map(|workspace| workspace.read(cx).ui.nodes.menu_at);
            add_node_items(menu, weak.clone(), layer_id, at, matte, &names)
        })
        .into_any_element()
}

/// One node: a title bar to grab and pick it by, input dots down its left edge with their
/// names, and an output dot on the right (except the Output node).
fn graph_node(layer: &Layer, node: NodeId, at: Vec2, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let ui = &workspace.ui.nodes;
    let layer_id = layer.id;
    let zoom = ui.zoom;
    let local = ui.to_local(at);
    let (title, icon, off) = node_face(layer, node);
    let picked = ui.selected == Some(node);
    let fill = match node {
        NodeId::Source | NodeId::Output => color::PANEL,
        NodeId::Merge(_) => color::WORK_AREA,
        _ => color::RAISED,
    };
    let text = px(size::SMALL_TEXT * zoom.clamp(0.6, 1.4));

    let title_bar = div()
        .id(ElementId::Name(format!("node-title:{}", node_key(node)).into()))
        .h(px(TITLE_HEIGHT * zoom))
        .flex()
        .items_center()
        .gap(px(5.0 * zoom))
        .px(px(6.0 * zoom))
        .border_b_1()
        .border_color(hsla(color::HAIRLINE))
        .cursor_grab()
        .child(Icon::new(icon).size(px(12.0 * zoom.clamp(0.6, 1.4))).text_color(hsla(color::MUTED)))
        .child(div().flex_1().min_w_0().truncate().text_color(hsla(color::TEXT)).child(title.clone()))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            let ui = &mut this.ui.nodes;
            let corner = ui.to_local(at);
            let bounds = ui.bounds.get();
            ui.grab = Vec2::new(f32::from(event.position.x) - bounds.left - corner.x, f32::from(event.position.y) - bounds.top - corner.y);
            pick(this, node);
            cx.notify();
        }))
        .on_drag(NodeMove { layer: layer_id, node }, |_, _, _, cx| cx.new(|_| EmptyDragView))
        .test_support();

    let inputs = effect_graph::inputs(node);
    let rows = div().flex().flex_col().pt(px(0.0)).children(inputs.iter().map(|&port| {
        div()
            .h(px(PORT_ROW * zoom))
            .flex()
            .items_center()
            .pl(px(10.0 * zoom))
            .text_color(hsla(color::MUTED))
            .child(port.label())
    }));
    let rows = if inputs.is_empty() { rows.child(div().h(px(PORT_ROW * zoom))) } else { rows };

    let mut card = div()
        .id(node_id(node))
        .absolute()
        .left(px(local.x))
        .top(px(local.y))
        .w(px(NODE_WIDTH * zoom))
        .h(px(node_height(node) * zoom))
        .rounded(px(6.0 * zoom))
        .border_1()
        .border_color(hsla(if picked { color::ACCENT } else { color::HAIRLINE }))
        .bg(hsla(fill))
        .text_size(text)
        .shadow_md()
        .when(off, |card| card.opacity(0.5))
        .child(title_bar)
        .child(rows)
        // Clicks on a node never reach the canvas below (which would let go of it).
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .test_support();

    for (row, &port) in inputs.iter().enumerate() {
        card = card.child(input_port(layer, node, port, row, zoom, cx));
    }
    if effect_graph::has_output(node) {
        card = card.child(output_port(layer_id, node, zoom, cx));
    }
    let editable = matches!(node, NodeId::Effect(_) | NodeId::Merge(_) | NodeId::Matte);
    if !editable {
        return card.into_any_element();
    }
    let weak = cx.weak_entity();
    card.context_menu(move |menu, _, _| node_menu(menu, weak.clone(), layer_id, node, off)).into_any_element()
}

/// A node's right-click menu: turn it off or on, disconnect its inputs, delete it.
fn node_menu(menu: PopupMenu, weak: gpui_kit::WeakEntity<Workspace>, layer: Id, node: NodeId, off: bool) -> PopupMenu {
    let workspace = move |cx: &mut App, change: &dyn Fn(&mut Workspace)| {
        weak.update(cx, |this, cx| {
            change(this);
            cx.notify();
        })
        .ok();
    };
    let mut menu = menu;
    if matches!(node, NodeId::Effect(_) | NodeId::Merge(_)) {
        let workspace = workspace.clone();
        menu = menu.item(PopupMenuItem::new(if off { "Turn On" } else { "Turn Off" }).on_click(move |_, _, cx| workspace(cx, &|this| this.state.toggle_graph_node(layer, node))));
    }
    let disconnect = workspace.clone();
    menu.item(PopupMenuItem::new("Disconnect Inputs").on_click(move |_, _, cx| {
        let workspace = &disconnect;
        workspace(cx, &|this| {
            for &port in effect_graph::inputs(node) {
                this.state.disconnect_node(layer, node, port);
            }
        })
    }))
    .separator()
    .item(PopupMenuItem::new("Delete").on_click(move |_, _, cx| {
        workspace(cx, &|this| {
            this.state.remove_graph_node(layer, node);
            if this.ui.nodes.selected == Some(node) {
                this.ui.nodes.selected = None;
            }
        })
    }))
}

/// The hit area for a port dot at `center` (local to the node), with the dot drawn inside.
fn port_dot(id: ElementId, center: Vec2, filled: bool, zoom: f32) -> gpui_kit::Stateful<gpui_kit::Div> {
    let dot = PORT_DOT * zoom.clamp(0.7, 1.4);
    div()
        .id(id)
        .absolute()
        .left(px(center.x - PORT_HIT / 2.0))
        .top(px(center.y - PORT_HIT / 2.0))
        .size(px(PORT_HIT))
        .flex()
        .items_center()
        .justify_center()
        .cursor_crosshair()
        .child(
            div()
                .size(px(dot))
                .rounded_full()
                .border_1()
                .border_color(hsla(color::TEXT))
                .bg(hsla(if filled { color::ACCENT } else { color::INSET })),
        )
}

/// An input dot. Dragging from it pulls its wire off (or starts a new one back to an output);
/// dropping a wire from an output on it connects them.
fn input_port(layer: &Layer, node: NodeId, port: InputPort, row: usize, zoom: f32, cx: &mut Context<Workspace>) -> AnyElement {
    let layer_id = layer.id;
    let source = effect_graph::source_of(layer, node, port);
    let center = Vec2::new(0.0, (TITLE_HEIGHT + PORT_ROW * (row as f32 + 0.5)) * zoom);
    let drag = match source {
        Some(from) => WireDrag { layer: layer_id, output: Some(from), input: None, lifted: Some((node, port)) },
        None => WireDrag { layer: layer_id, output: None, input: Some((node, port)), lifted: None },
    };
    port_dot(input_id(node, port), center, source.is_some(), zoom)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_drag(drag, |_, _, _, cx| cx.new(|_| EmptyDragView))
        .drag_over::<WireDrag>(|style, _, _, _| style.bg(hsla(color::ROW_SELECTED)).rounded_full())
        .on_drop(cx.listener(move |this, drag: &WireDrag, window, cx| {
            cx.stop_propagation();
            this.ui.nodes.wire = None;
            if let Some(output) = drag.output.filter(|_| drag.layer == layer_id) {
                let result = this.state.connect_nodes(layer_id, output, node, port);
                match result {
                    // A wire moved from another input leaves that one.
                    Ok(()) => {
                        if let Some(lifted) = drag.lifted.filter(|lifted| *lifted != (node, port)) {
                            this.state.disconnect_node(layer_id, lifted.0, lifted.1);
                        }
                    }
                    Err(problem) => window.push_notification(format!("Not connected: {problem}.").as_str(), cx),
                }
            }
            cx.notify();
        }))
        .test_support()
        .into_any_element()
}

/// An output dot. Dragging from it starts a new wire; dropping a wire from an empty input on
/// it connects them.
fn output_port(layer: Id, node: NodeId, zoom: f32, cx: &mut Context<Workspace>) -> AnyElement {
    let center = Vec2::new(NODE_WIDTH * zoom, (TITLE_HEIGHT + PORT_ROW * 0.5) * zoom);
    port_dot(output_id(node), center, true, zoom)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_drag(WireDrag { layer, output: Some(node), input: None, lifted: None }, |_, _, _, cx| cx.new(|_| EmptyDragView))
        .drag_over::<WireDrag>(|style, _, _, _| style.bg(hsla(color::ROW_SELECTED)).rounded_full())
        .on_drop(cx.listener(move |this, drag: &WireDrag, window, cx| {
            cx.stop_propagation();
            this.ui.nodes.wire = None;
            if let (Some((to, port)), true) = (drag.input, drag.layer == layer)
                && let Err(problem) = this.state.connect_nodes(layer, node, to, port)
            {
                window.push_notification(format!("Not connected: {problem}.").as_str(), cx);
            }
            cx.notify();
        }))
        .test_support()
        .into_any_element()
}

/// A Merge's opacity slider being dragged.
#[derive(Clone, Copy)]
struct OpacityDrag {
    layer: Id,
    merge: Id,
}

/// Sets a Merge's opacity from where `position` falls along its slider.
fn drag_opacity(workspace: &mut Workspace, drag: OpacityDrag, position: Point<Pixels>) {
    let track = workspace.ui.nodes.opacity_sliders.borrow().get(&drag.merge).map(MeasuredBounds::get);
    let Some(track) = track.filter(|track| track.width > 0.0) else { return };
    let fraction = ((f32::from(position.x) - track.left) / track.width).clamp(0.0, 1.0);
    workspace.state.set_merge(drag.layer, drag.merge, |merge| merge.opacity = (fraction * 100.0).round() / 100.0);
}

/// A Merge node's card in the Effects tab: on/off and remove in its heading, then its blend
/// mode and opacity. `picked` marks it as the node picked in the Nodes panel.
pub fn merge_card(layer: Id, merge: &studio_core::effect_graph::MergeNode, picked: bool, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    use super::chrome::{CardHeading, card};
    let merge_id = merge.id;
    let node = NodeId::Merge(merge_id);
    let on = super::chrome::heading_button(ElementId::Name(format!("merge-on:{}", merge_id.0).into()), if merge.enabled { IconName::Eye } else { IconName::EyeOff }, if merge.enabled { "Turn Off" } else { "Turn On" }, false, cx.listener(move |this, _, _, cx| {
        this.state.toggle_graph_node(layer, node);
        cx.notify();
    }));
    let remove = super::chrome::heading_button(ElementId::Name(format!("merge-remove:{}", merge_id.0).into()), IconName::Trash, "Remove", false, cx.listener(move |this, _, _, cx| {
        this.state.remove_graph_node(layer, node);
        cx.notify();
    }));
    let heading = CardHeading {
        id: ElementId::Name(format!("merge-card:{}", merge_id.0).into()),
        icon: Some(IconName::Blend),
        title: "Merge".into(),
        info: Some("Lays the Foreground input over the Background input, like a layer over the layers below. A Mask input limits where the foreground shows.".into()),
        open: true,
        muted: !merge.enabled,
        trailing: vec![Some(on.into_any_element()), Some(remove.into_any_element())],
    };

    let blend = Button::new(ElementId::Name(format!("merge-blend:{}", merge_id.0).into())).outline().small().label(merge.blend.label()).dropdown_menu({
        let weak = cx.weak_entity();
        move |mut menu, _, _| {
            for mode in studio_core::model::BlendMode::ALL {
                let weak = weak.clone();
                menu = menu.item(PopupMenuItem::new(mode.label()).on_click(move |_, _, cx| {
                    weak.update(cx, |this, cx| {
                        this.state.set_merge(layer, merge_id, |merge| merge.blend = mode);
                        cx.notify();
                    })
                    .ok();
                }));
            }
            menu
        }
    });
    let bounds = workspace.ui.nodes.opacity_sliders.borrow_mut().entry(merge_id).or_default().clone();
    let drag = OpacityDrag { layer, merge: merge_id };
    let slider = div()
        .id(ElementId::Name(format!("merge-opacity:{}", merge_id.0).into()))
        .relative()
        .flex_1()
        .h(px(16.0))
        .cursor_pointer()
        .child(bounds.probe())
        .child(div().absolute().left_0().right_0().top(px(6.0)).h(px(4.0)).rounded_full().bg(hsla(color::HAIRLINE)))
        .child(div().absolute().left_0().top(px(6.0)).h(px(4.0)).rounded_full().w(gpui_kit::relative(merge.opacity)).bg(hsla(color::ACCENT)))
        .child(div().absolute().top(px(2.0)).left(gpui_kit::relative(merge.opacity)).ml(px(-6.0)).size(px(12.0)).rounded_full().bg(gpui_kit::rgb(0xffffff)).border_1().border_color(gpui_kit::rgb(0x000000)))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            drag_opacity(this, drag, event.position);
            cx.notify();
        }))
        .on_drag(drag, |_, _, _, cx| cx.new(|_| EmptyDragView))
        .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<OpacityDrag>, _, cx| {
            let moved = *event.drag(cx);
            if moved.merge == merge_id {
                drag_opacity(this, moved, event.event.position);
                cx.notify();
            }
        }))
        .test_support();
    let row = |label: &'static str| div().flex().items_center().gap_2().h(px(26.0)).child(div().w(px(64.0)).text_size(px(size::SMALL_TEXT)).text_color(hsla(color::TEXT)).child(label));
    let body = div()
        .flex()
        .flex_col()
        .child(row("Blend").child(blend))
        .child(row("Opacity").child(slider).child(div().w(px(36.0)).text_right().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::VALUE)).child(format!("{:.0}%", merge.opacity * 100.0))));
    let card = card(heading, |_, _, _| {}, Some(body.into_any_element()));
    div().when(picked, |wrap| wrap.border_l_2().border_color(hsla(color::ACCENT))).child(card).into_any_element()
}
