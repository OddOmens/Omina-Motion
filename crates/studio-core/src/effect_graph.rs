//! Node-based effects, like Fusion's node editor. A layer's effects (`Layer::effects`) are the
//! effect nodes; this module adds the connections between them, Merge nodes that combine two
//! pictures, an Object Matte node, and where each node sits on the canvas.
//!
//! Until someone rewires a layer, its graph is "unwired": the effects simply run as a chain in
//! list order (Source → first effect → … → Output), exactly as they always have. The first
//! structural edit (connecting, disconnecting, adding a Merge) writes that chain down as real
//! links and from then on the links decide what runs.
//!
//! `plan` turns a graph into a list of steps in running order, with numbered image slots
//! ("registers") reused once nothing needs them, which is what the renderer executes.

use std::collections::{HashMap, HashSet};

use crate::animation::Vec2;
use crate::model::{BlendMode, Id, Layer};

/// A node in a layer's graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum NodeId {
    /// The layer's own picture (after its object matte, if the graph doesn't use the matte itself).
    Source,
    /// What the layer finally shows.
    Output,
    /// The layer's object matte (see `cutout`) as a picture: white where the object is.
    Matte,
    /// One of the layer's effects, by its id.
    Effect(Id),
    /// A Merge node, by its id.
    Merge(Id),
}

/// A named input on a node. An output has no name: each node has at most one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum InputPort {
    /// The picture an effect works on, or the picture the Output shows.
    Main,
    /// Limits where a node's result shows: by the alpha of the picture connected here.
    Mask,
    /// A Merge's lower picture.
    Background,
    /// A Merge's upper picture, laid over the background.
    Foreground,
}

impl InputPort {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Main => "Input",
            Self::Mask => "Mask",
            Self::Background => "Background",
            Self::Foreground => "Foreground",
        }
    }
}

/// A connection from one node's output into another node's input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Link {
    pub from: NodeId,
    pub to: NodeId,
    pub port: InputPort,
}

/// Combines a background and a foreground picture with a blend mode, like a layer laid over
/// the layers below it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MergeNode {
    pub id: Id,
    pub blend: BlendMode,
    /// How much of the foreground shows, 0–1.
    pub opacity: f32,
    pub enabled: bool,
}

/// A layer's node graph. See the module comment for what "wired" means.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EffectGraph {
    /// Whether `links` decide what runs. When false, the effects run as a chain in list order.
    #[serde(default)]
    pub wired: bool,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default)]
    pub merges: Vec<MergeNode>,
    /// Whether the Object Matte node is on the canvas.
    #[serde(default)]
    pub matte_node: bool,
    /// Where nodes were placed on the canvas, in canvas units. Nodes not listed are laid out
    /// automatically (see `layout`).
    #[serde(default)]
    pub positions: Vec<(NodeId, Vec2)>,
}

impl EffectGraph {
    /// Whether this is the graph every layer starts with, so files leave it out.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Why a connection was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectError {
    /// One of the two nodes isn't in this layer's graph.
    MissingNode,
    /// The node has no such input, or the source has no output.
    NoSuchPort,
    /// The connection would feed a node's result back into itself.
    Loop,
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MissingNode => "that node is no longer there",
            Self::NoSuchPort => "those two can't be connected that way",
            Self::Loop => "that would feed a node's result back into itself",
        })
    }
}

/// The inputs a node has, top to bottom as drawn.
pub fn inputs(node: NodeId) -> &'static [InputPort] {
    match node {
        NodeId::Source | NodeId::Matte => &[],
        NodeId::Output => &[InputPort::Main],
        NodeId::Effect(_) => &[InputPort::Main, InputPort::Mask],
        NodeId::Merge(_) => &[InputPort::Background, InputPort::Foreground, InputPort::Mask],
    }
}

/// Whether a node has an output that can feed other nodes.
pub fn has_output(node: NodeId) -> bool {
    node != NodeId::Output
}

/// Every node in the layer's graph: Source, the effects in list order, the Merges, the Object
/// Matte if it is on the canvas, then Output.
pub fn nodes(layer: &Layer) -> Vec<NodeId> {
    let graph = &layer.effect_graph;
    let mut nodes = vec![NodeId::Source];
    nodes.extend(picture_effects(layer).map(|effect| NodeId::Effect(effect.id)));
    nodes.extend(graph.merges.iter().map(|merge| NodeId::Merge(merge.id)));
    if graph.matte_node {
        nodes.push(NodeId::Matte);
    }
    nodes.push(NodeId::Output);
    nodes
}

/// Whether `node` is in the layer's graph.
pub fn contains(layer: &Layer, node: NodeId) -> bool {
    match node {
        NodeId::Source | NodeId::Output => true,
        NodeId::Matte => layer.effect_graph.matte_node,
        NodeId::Effect(id) => picture_effects(layer).any(|effect| effect.id == id),
        NodeId::Merge(id) => layer.effect_graph.merges.iter().any(|merge| merge.id == id),
    }
}

/// The layer's effects on its picture, in list order. Effects on its sound run in the mixer,
/// not in the graph.
pub fn picture_effects(layer: &Layer) -> impl Iterator<Item = &crate::model::Effect> {
    layer.effects.iter().filter(|effect| !crate::effects::is_audio(&effect.name))
}

/// The links of the chain an unwired layer runs: Source through every effect in list order to
/// Output. (Turned-off effects stay in the chain; they let their input through.)
fn chain_links(layer: &Layer) -> Vec<Link> {
    let mut links = Vec::with_capacity(layer.effects.len() + 1);
    let mut previous = NodeId::Source;
    for effect in picture_effects(layer) {
        let node = NodeId::Effect(effect.id);
        links.push(Link { from: previous, to: node, port: InputPort::Main });
        previous = node;
    }
    links.push(Link { from: previous, to: NodeId::Output, port: InputPort::Main });
    links
}

/// The links that decide what the layer shows: its own once wired, else the plain chain.
pub fn links(layer: &Layer) -> Vec<Link> {
    if layer.effect_graph.wired { layer.effect_graph.links.clone() } else { chain_links(layer) }
}

/// What feeds `port` on `node`, if anything.
pub fn source_of(layer: &Layer, node: NodeId, port: InputPort) -> Option<NodeId> {
    links(layer).iter().find(|link| link.to == node && link.port == port).map(|link| link.from)
}

/// Writes the plain chain down as real links, so the graph can be edited freely. Every node
/// is pinned where it is drawn now, so rewiring never moves nodes about on the canvas.
fn ensure_wired(layer: &mut Layer) {
    if !layer.effect_graph.wired {
        layer.effect_graph.positions = layout(layer);
        layer.effect_graph.links = chain_links(layer);
        layer.effect_graph.wired = true;
    }
}

/// Whether `target`'s result already feeds, directly or through other nodes, into `start`.
/// Connecting `start → target` would then make a loop.
fn feeds(links: &[Link], target: NodeId, start: NodeId) -> bool {
    let mut stack = vec![start];
    let mut seen = HashSet::new();
    while let Some(node) = stack.pop() {
        if node == target {
            return true;
        }
        if seen.insert(node) {
            stack.extend(links.iter().filter(|link| link.to == node).map(|link| link.from));
        }
    }
    false
}

/// Connects `from`'s output into `port` on `to`, replacing whatever fed that input before.
/// One output can feed any number of inputs.
pub fn connect(layer: &mut Layer, from: NodeId, to: NodeId, port: InputPort) -> Result<(), ConnectError> {
    if !contains(layer, from) || !contains(layer, to) {
        return Err(ConnectError::MissingNode);
    }
    if !has_output(from) || !inputs(to).contains(&port) {
        return Err(ConnectError::NoSuchPort);
    }
    // `to` must not already feed `from`, or the picture would go round in a circle.
    if from == to || feeds(&links(layer), to, from) {
        return Err(ConnectError::Loop);
    }
    ensure_wired(layer);
    let graph = &mut layer.effect_graph;
    graph.links.retain(|link| !(link.to == to && link.port == port));
    graph.links.push(Link { from, to, port });
    Ok(())
}

/// Takes the connection off `port` on `to`. Returns whether there was one.
pub fn disconnect(layer: &mut Layer, to: NodeId, port: InputPort) -> bool {
    if source_of(layer, to, port).is_none() {
        return false;
    }
    ensure_wired(layer);
    layer.effect_graph.links.retain(|link| !(link.to == to && link.port == port));
    true
}

/// Adds a Merge node with id `id` (unconnected). Returns its node.
pub fn add_merge(layer: &mut Layer, id: Id) -> NodeId {
    ensure_wired(layer);
    layer.effect_graph.merges.push(MergeNode { id, blend: BlendMode::Normal, opacity: 1.0, enabled: true });
    NodeId::Merge(id)
}

/// Puts the Object Matte node on the canvas (unconnected). Returns whether it was added.
pub fn add_matte_node(layer: &mut Layer) -> bool {
    if layer.effect_graph.matte_node {
        return false;
    }
    ensure_wired(layer);
    layer.effect_graph.matte_node = true;
    true
}

/// The Merge node with id `id`, to change its settings.
pub fn merge_mut(layer: &mut Layer, id: Id) -> Option<&mut MergeNode> {
    layer.effect_graph.merges.iter_mut().find(|merge| merge.id == id)
}

/// The input a node passes along when it is removed or turned off: an effect's picture, or a
/// Merge's background.
fn pass_through_port(node: NodeId) -> Option<InputPort> {
    match node {
        NodeId::Effect(_) => Some(InputPort::Main),
        NodeId::Merge(_) => Some(InputPort::Background),
        _ => None,
    }
}

/// Unhooks `node` from a wired graph before it goes away. Whatever fed its main input is
/// connected straight to everything it fed, so removing a node from a chain keeps the chain.
/// The effect itself is removed by the caller; Merge and Matte nodes are removed here.
pub fn remove_node(layer: &mut Layer, node: NodeId) {
    let graph = &mut layer.effect_graph;
    match node {
        NodeId::Merge(id) => graph.merges.retain(|merge| merge.id != id),
        NodeId::Matte => graph.matte_node = false,
        NodeId::Source | NodeId::Output => return,
        NodeId::Effect(_) => {}
    }
    graph.positions.retain(|(placed, _)| *placed != node);
    if !graph.wired {
        return;
    }
    let upstream = pass_through_port(node).and_then(|port| graph.links.iter().find(|link| link.to == node && link.port == port).map(|link| link.from));
    let fed: Vec<Link> = graph.links.iter().filter(|link| link.from == node).copied().collect();
    graph.links.retain(|link| link.from != node && link.to != node);
    if let Some(upstream) = upstream {
        graph.links.extend(fed.into_iter().map(|link| Link { from: upstream, ..link }));
    }
}

/// Hooks a newly added effect into a wired graph: it goes in just before the Output, so adding
/// an effect from a menu or the Effects panel puts it last, as in the list view.
pub fn effect_added(layer: &mut Layer, effect: Id) {
    let is_sound = layer.effects.iter().any(|candidate| candidate.id == effect && crate::effects::is_audio(&candidate.name));
    let graph = &mut layer.effect_graph;
    if !graph.wired || is_sound {
        return;
    }
    let node = NodeId::Effect(effect);
    let into_output = graph.links.iter().position(|link| link.to == NodeId::Output && link.port == InputPort::Main);
    let upstream = match into_output {
        Some(index) => graph.links.remove(index).from,
        None => NodeId::Source,
    };
    graph.links.push(Link { from: upstream, to: node, port: InputPort::Main });
    graph.links.push(Link { from: node, to: NodeId::Output, port: InputPort::Main });
}

/// Gives a copied layer's effects and Merge nodes fresh ids from `allocate`, and points its
/// graph at them, so the copy's graph is wired the same way as the original's.
pub fn reassign_ids(layer: &mut Layer, mut allocate: impl FnMut() -> Id) {
    let mut fresh: HashMap<Id, Id> = HashMap::new();
    for effect in &mut layer.effects {
        let id = allocate();
        fresh.insert(effect.id, id);
        effect.id = id;
    }
    for merge in &mut layer.effect_graph.merges {
        let id = allocate();
        fresh.insert(merge.id, id);
        merge.id = id;
    }
    let renamed = |node: NodeId| match node {
        NodeId::Effect(id) => NodeId::Effect(fresh.get(&id).copied().unwrap_or(id)),
        NodeId::Merge(id) => NodeId::Merge(fresh.get(&id).copied().unwrap_or(id)),
        other => other,
    };
    let graph = &mut layer.effect_graph;
    for link in &mut graph.links {
        link.from = renamed(link.from);
        link.to = renamed(link.to);
    }
    for (node, _) in &mut graph.positions {
        *node = renamed(*node);
    }
}

/// Drops links and positions that point at nodes or inputs that don't exist, as a file edited
/// by hand (or by an older version) might have. Returns whether anything was dropped.
pub fn sanitize(layer: &mut Layer) -> bool {
    let valid: HashSet<NodeId> = nodes(layer).into_iter().collect();
    let graph = &mut layer.effect_graph;
    let before = (graph.links.len(), graph.positions.len());
    graph.links.retain(|link| valid.contains(&link.from) && valid.contains(&link.to) && has_output(link.from) && inputs(link.to).contains(&link.port));
    // Only one link may feed each input; the last one wins, as `connect` would leave it.
    let mut seen = HashSet::new();
    let mut kept: Vec<Link> = graph.links.iter().rev().filter(|link| seen.insert((link.to, link.port))).copied().collect();
    kept.reverse();
    graph.links = kept;
    graph.positions.retain(|(node, _)| valid.contains(node));
    before != (graph.links.len(), graph.positions.len())
}

/// Whether the graph uses the layer's object matte itself (through the Object Matte node), in
/// which case the matte is not also cut out of the Source picture.
pub fn uses_matte(layer: &Layer) -> bool {
    layer.effect_graph.wired && layer.effect_graph.links.iter().any(|link| link.from == NodeId::Matte)
}

/// Where a node sits on the canvas: where it was put, or an automatic spot.
pub fn position(layer: &Layer, node: NodeId) -> Vec2 {
    layout(layer).into_iter().find(|(placed, _)| *placed == node).map_or(Vec2::new(0.0, 0.0), |(_, at)| at)
}

/// Moves a node on the canvas.
pub fn set_position(layer: &mut Layer, node: NodeId, at: Vec2) {
    let positions = &mut layer.effect_graph.positions;
    match positions.iter_mut().find(|(placed, _)| *placed == node) {
        Some((_, spot)) => *spot = at,
        None => positions.push((node, at)),
    }
}

/// Horizontal and vertical spacing of automatically placed nodes, in canvas units.
pub const LAYOUT_COLUMN: f32 = 200.0;
pub const LAYOUT_ROW: f32 = 90.0;

/// Every node's spot on the canvas. Nodes the user placed keep their spot; the rest flow left
/// to right in columns by how many steps they are from the Source, stacked within a column.
pub fn layout(layer: &Layer) -> Vec<(NodeId, Vec2)> {
    let links = links(layer);
    let all = nodes(layer);
    // Depth: the longest path from a node with no inputs (so a node sits right of all it uses).
    let mut depth: HashMap<NodeId, usize> = HashMap::new();
    fn depth_of(node: NodeId, links: &[Link], depth: &mut HashMap<NodeId, usize>, visiting: &mut HashSet<NodeId>) -> usize {
        if let Some(&known) = depth.get(&node) {
            return known;
        }
        if !visiting.insert(node) {
            return 0; // A loop in a damaged file; give up on this branch.
        }
        let upstream: Vec<NodeId> = links.iter().filter(|link| link.to == node).map(|link| link.from).collect();
        let value = upstream.into_iter().map(|from| depth_of(from, links, depth, visiting) + 1).max().unwrap_or(0);
        visiting.remove(&node);
        depth.insert(node, value);
        value
    }
    let mut visiting = HashSet::new();
    for &node in &all {
        depth_of(node, &links, &mut depth, &mut visiting);
    }
    // Output always sits right of everything, even when nothing feeds it yet.
    let deepest = all.iter().filter(|node| **node != NodeId::Output).map(|node| depth[node]).max().unwrap_or(0);
    depth.insert(NodeId::Output, depth[&NodeId::Output].max(deepest + 1));
    let mut rows: HashMap<usize, usize> = HashMap::new();
    all.into_iter()
        .map(|node| {
            let placed = layer.effect_graph.positions.iter().find(|(placed, _)| *placed == node).map(|(_, at)| *at);
            let at = placed.unwrap_or_else(|| {
                let column = depth[&node];
                let row = rows.entry(column).or_insert(0);
                let spot = Vec2::new(column as f32 * LAYOUT_COLUMN, *row as f32 * LAYOUT_ROW);
                *row += 1;
                spot
            });
            (node, at)
        })
        .collect()
}

/// A numbered image slot the renderer keeps a picture in while running a plan. Register 0
/// always holds the Source picture and is never written.
pub type Register = usize;

/// One thing the renderer does while running a plan.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Fill `out` with transparent black (the picture for an input nothing feeds).
    Clear { out: Register },
    /// Draw the layer's object matte into `out` as white where the object is.
    Matte { out: Register },
    /// Run `layer.effects[effect]` on `input` into `out`. With a mask, the effect's result is
    /// written to `scratch` first, then mixed with the input by the mask's alpha into `out`.
    Effect { effect: usize, input: Register, mask: Option<Register>, scratch: Option<Register>, out: Register },
    /// Lay `foreground` over `background` with `blend` and `opacity`, limited by `mask`'s alpha.
    Merge { background: Register, foreground: Register, mask: Option<Register>, blend: BlendMode, opacity: f32, out: Register },
}

/// What the renderer runs for one layer's wired graph.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub steps: Vec<Step>,
    /// The register holding what the layer shows, or `None` when nothing feeds the Output
    /// (the layer then shows nothing).
    pub output: Option<Register>,
    /// How many registers the plan uses, counting register 0.
    pub registers: usize,
}

/// Hands out registers, reusing ones whose picture is no longer needed.
struct Registers {
    free: Vec<Register>,
    count: usize,
}

impl Registers {
    fn take(&mut self) -> Register {
        self.free.pop().unwrap_or_else(|| {
            self.count += 1;
            self.count - 1
        })
    }

    fn give_back(&mut self, register: Register) {
        if register != 0 && !self.free.contains(&register) {
            self.free.push(register);
        }
    }
}

/// Builds what the renderer runs for a wired layer. Only nodes that lead to the Output run.
/// Turned-off effects and Merges pass their main picture through, as do links to nodes that
/// are gone. Returns `None` for an unwired layer (the renderer runs the plain chain) or a
/// graph with a loop in it.
pub fn plan(layer: &Layer) -> Option<Plan> {
    if !layer.effect_graph.wired {
        return None;
    }
    let links = &layer.effect_graph.links;
    let input_of = |node: NodeId, port: InputPort| links.iter().find(|link| link.to == node && link.port == port).map(|link| link.from);

    // Nodes in running order: every node after the nodes it reads from.
    let mut order = Vec::new();
    let mut state: HashMap<NodeId, bool> = HashMap::new(); // false: in progress, true: done
    fn visit(node: NodeId, links: &[Link], order: &mut Vec<NodeId>, state: &mut HashMap<NodeId, bool>) -> bool {
        match state.get(&node) {
            Some(true) => return true,
            Some(false) => return false, // A loop.
            None => {}
        }
        state.insert(node, false);
        for link in links.iter().filter(|link| link.to == node) {
            if !visit(link.from, links, order, state) {
                return false;
            }
        }
        state.insert(node, true);
        order.push(node);
        true
    }
    if !visit(NodeId::Output, links, &mut order, &mut state) {
        return None;
    }

    // How many times each node's picture is still to be read, to free its register after.
    let mut readers: HashMap<NodeId, usize> = HashMap::new();
    for link in links.iter().filter(|link| state.get(&link.to) == Some(&true)) {
        *readers.entry(link.from).or_default() += 1;
    }

    let mut registers = Registers { free: Vec::new(), count: 1 };
    let mut value: HashMap<NodeId, Register> = HashMap::new();
    let mut steps = Vec::new();
    let mut output = None;

    for node in order {
        // The register holding what feeds `port`, if anything does.
        let read = |port: InputPort, value: &HashMap<NodeId, Register>| input_of(node, port).and_then(|from| value.get(&from).copied());
        // An input nothing feeds reads as a transparent picture.
        let read_or_clear = |port: InputPort, value: &HashMap<NodeId, Register>, steps: &mut Vec<Step>, registers: &mut Registers| {
            read(port, value).unwrap_or_else(|| {
                let out = registers.take();
                steps.push(Step::Clear { out });
                out
            })
        };
        let mut own_clears = Vec::new();
        let result = match node {
            NodeId::Source => Some(0),
            NodeId::Output => {
                output = read(InputPort::Main, &value);
                None
            }
            NodeId::Matte => {
                let out = registers.take();
                steps.push(Step::Matte { out });
                Some(out)
            }
            NodeId::Effect(id) => {
                // A link to an effect that is gone (a preview copy without it) passes its input on.
                let Some(index) = layer.effects.iter().position(|effect| effect.id == id) else {
                    value.extend(read(InputPort::Main, &value).map(|register| (node, register)));
                    continue;
                };
                let input_fed = read(InputPort::Main, &value);
                let input = read_or_clear(InputPort::Main, &value, &mut steps, &mut registers);
                if input_fed.is_none() {
                    own_clears.push(input);
                }
                if layer.effects[index].enabled {
                    let mask = read(InputPort::Mask, &value);
                    let scratch = mask.map(|_| registers.take());
                    let out = registers.take();
                    steps.push(Step::Effect { effect: index, input, mask, scratch, out });
                    if let Some(scratch) = scratch {
                        registers.give_back(scratch);
                    }
                    Some(out)
                } else {
                    own_clears.retain(|clear| *clear != input);
                    Some(input)
                }
            }
            NodeId::Merge(id) => {
                let Some(merge) = layer.effect_graph.merges.iter().find(|merge| merge.id == id) else {
                    value.extend(read(InputPort::Background, &value).map(|register| (node, register)));
                    continue;
                };
                let background_fed = read(InputPort::Background, &value);
                let background = read_or_clear(InputPort::Background, &value, &mut steps, &mut registers);
                if background_fed.is_none() {
                    own_clears.push(background);
                }
                match read(InputPort::Foreground, &value).filter(|_| merge.enabled) {
                    Some(foreground) => {
                        let out = registers.take();
                        let mask = read(InputPort::Mask, &value);
                        steps.push(Step::Merge { background, foreground, mask, blend: merge.blend, opacity: merge.opacity, out });
                        Some(out)
                    }
                    None => {
                        own_clears.retain(|clear| *clear != background);
                        Some(background)
                    }
                }
            }
        };
        // Pictures this node was the last to read are free now.
        for link in links.iter().filter(|link| link.to == node) {
            if let Some(count) = readers.get_mut(&link.from) {
                *count -= 1;
                if *count == 0
                    && value.get(&link.from) != result.as_ref()
                    && let Some(&register) = value.get(&link.from)
                {
                    registers.give_back(register);
                }
            }
        }
        for clear in own_clears {
            if Some(clear) != result {
                registers.give_back(clear);
            }
        }
        if let Some(result) = result {
            value.insert(node, result);
        }
    }
    Some(Plan { steps, output, registers: registers.count })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Effect;
    use crate::sample;

    /// A layer from the sample project with effects named `names`, ids 1000 and up.
    fn layer_with(names: &[&str]) -> Layer {
        let mut layer = sample::sample_project().compositions[0].layers[0].clone();
        layer.effects = names
            .iter()
            .enumerate()
            .map(|(index, name)| crate::effects::instantiate(crate::effects::find(name).unwrap(), Id(1000 + index as u64)))
            .collect::<Vec<Effect>>();
        layer.effect_graph = EffectGraph::default();
        layer
    }

    fn effect(index: u64) -> NodeId {
        NodeId::Effect(Id(1000 + index))
    }

    #[test]
    fn an_unwired_layer_runs_its_effects_as_a_chain() {
        let layer = layer_with(&["Gaussian Blur", "Glow"]);
        assert_eq!(plan(&layer), None, "the renderer runs the plain chain");
        assert_eq!(source_of(&layer, effect(1), InputPort::Main), Some(effect(0)));
        assert_eq!(source_of(&layer, NodeId::Output, InputPort::Main), Some(effect(1)));
    }

    #[test]
    fn rewiring_writes_the_chain_down_then_follows_the_links() {
        let mut layer = layer_with(&["Gaussian Blur", "Glow"]);
        // Skip the blur: the Source feeds the glow directly.
        connect(&mut layer, NodeId::Source, effect(1), InputPort::Main).unwrap();
        assert!(layer.effect_graph.wired);
        let plan = plan(&layer).unwrap();
        assert_eq!(plan.steps, vec![Step::Effect { effect: 1, input: 0, mask: None, scratch: None, out: 1 }], "the unused blur doesn't run");
        assert_eq!(plan.output, Some(1));
    }

    #[test]
    fn connections_that_would_loop_or_use_missing_ports_are_refused() {
        let mut layer = layer_with(&["Gaussian Blur", "Glow"]);
        assert_eq!(connect(&mut layer, effect(1), effect(0), InputPort::Main), Err(ConnectError::Loop));
        assert_eq!(connect(&mut layer, effect(0), effect(0), InputPort::Mask), Err(ConnectError::Loop));
        assert_eq!(connect(&mut layer, NodeId::Output, effect(0), InputPort::Main), Err(ConnectError::NoSuchPort));
        assert_eq!(connect(&mut layer, NodeId::Source, effect(0), InputPort::Foreground), Err(ConnectError::NoSuchPort));
        assert_eq!(connect(&mut layer, NodeId::Source, NodeId::Merge(Id(5)), InputPort::Background), Err(ConnectError::MissingNode));
        assert!(!layer.effect_graph.wired, "refused edits leave the layer unwired");
    }

    #[test]
    fn a_merge_lays_one_branch_over_another() {
        let mut layer = layer_with(&["Gaussian Blur", "Glow"]);
        let merge = add_merge(&mut layer, Id(7));
        // Source → Blur → Merge background; Source → Glow → Merge foreground; Merge → Output.
        connect(&mut layer, NodeId::Source, effect(1), InputPort::Main).unwrap();
        connect(&mut layer, effect(0), merge, InputPort::Background).unwrap();
        connect(&mut layer, effect(1), merge, InputPort::Foreground).unwrap();
        connect(&mut layer, merge, NodeId::Output, InputPort::Main).unwrap();
        merge_mut(&mut layer, Id(7)).unwrap().blend = BlendMode::Screen;
        let plan = plan(&layer).unwrap();
        assert_eq!(plan.steps.len(), 3);
        let Step::Merge { background, foreground, blend, out, .. } = plan.steps[2].clone() else { panic!("{:?}", plan.steps) };
        assert_eq!(blend, BlendMode::Screen);
        assert_ne!(background, foreground);
        assert_eq!(plan.output, Some(out));
        assert!(plan.registers <= 4, "registers are reused: {}", plan.registers);
    }

    #[test]
    fn a_mask_input_gets_a_scratch_register() {
        let mut layer = layer_with(&["Gaussian Blur"]);
        add_matte_node(&mut layer);
        connect(&mut layer, NodeId::Matte, effect(0), InputPort::Mask).unwrap();
        assert!(uses_matte(&layer));
        let plan = plan(&layer).unwrap();
        let [Step::Matte { out: matte }, Step::Effect { mask, scratch, out, .. }] = plan.steps.as_slice() else { panic!("{:?}", plan.steps) };
        assert_eq!(*mask, Some(*matte));
        assert!(scratch.is_some_and(|scratch| scratch != *out && scratch != *matte));
    }

    #[test]
    fn an_output_with_nothing_connected_shows_nothing_and_off_nodes_pass_through() {
        let mut layer = layer_with(&["Gaussian Blur"]);
        assert!(disconnect(&mut layer, NodeId::Output, InputPort::Main));
        assert_eq!(plan(&layer).unwrap().output, None);
        connect(&mut layer, effect(0), NodeId::Output, InputPort::Main).unwrap();
        layer.effects[0].enabled = false;
        let plan = plan(&layer).unwrap();
        assert!(plan.steps.is_empty());
        assert_eq!(plan.output, Some(0), "a turned-off effect shows its input");
    }

    #[test]
    fn an_effect_with_no_input_works_on_a_transparent_picture() {
        let mut layer = layer_with(&["Gaussian Blur"]);
        disconnect(&mut layer, effect(0), InputPort::Main);
        let plan = plan(&layer).unwrap();
        let [Step::Clear { out: clear }, Step::Effect { input, .. }] = plan.steps.as_slice() else { panic!("{:?}", plan.steps) };
        assert_eq!(clear, input);
    }

    #[test]
    fn removing_a_node_reconnects_around_it_and_new_effects_go_last() {
        let mut layer = layer_with(&["Gaussian Blur", "Glow", "Invert"]);
        connect(&mut layer, NodeId::Source, effect(0), InputPort::Main).unwrap(); // wires it
        remove_node(&mut layer, effect(1));
        layer.effects.remove(1);
        assert_eq!(source_of(&layer, effect(2), InputPort::Main), Some(effect(0)));
        layer.effects.push(crate::effects::instantiate(crate::effects::find("Glow").unwrap(), Id(2000)));
        effect_added(&mut layer, Id(2000));
        assert_eq!(source_of(&layer, NodeId::Effect(Id(2000)), InputPort::Main), Some(effect(2)));
        assert_eq!(source_of(&layer, NodeId::Output, InputPort::Main), Some(NodeId::Effect(Id(2000))));
    }

    #[test]
    fn copies_get_fresh_ids_with_the_same_wiring() {
        let mut layer = layer_with(&["Gaussian Blur", "Glow"]);
        let merge = add_merge(&mut layer, Id(7));
        connect(&mut layer, effect(1), merge, InputPort::Foreground).unwrap();
        set_position(&mut layer, merge, Vec2::new(5.0, 6.0));
        let mut next = 500;
        reassign_ids(&mut layer, || {
            next += 1;
            Id(next)
        });
        let ids: Vec<u64> = layer.effects.iter().map(|effect| effect.id.0).collect();
        assert_eq!(ids, vec![501, 502]);
        assert_eq!(source_of(&layer, NodeId::Merge(Id(503)), InputPort::Foreground), Some(NodeId::Effect(Id(502))));
        assert_eq!(position(&layer, NodeId::Merge(Id(503))), Vec2::new(5.0, 6.0));
    }

    #[test]
    fn damaged_links_are_dropped() {
        let mut layer = layer_with(&["Gaussian Blur"]);
        connect(&mut layer, NodeId::Source, effect(0), InputPort::Main).unwrap();
        layer.effect_graph.links.push(Link { from: NodeId::Effect(Id(99)), to: NodeId::Output, port: InputPort::Main });
        layer.effect_graph.links.push(Link { from: NodeId::Source, to: effect(0), port: InputPort::Background });
        assert!(sanitize(&mut layer));
        assert_eq!(source_of(&layer, NodeId::Output, InputPort::Main), Some(effect(0)));
        assert!(!sanitize(&mut layer));
    }

    #[test]
    fn automatic_layout_flows_left_to_right() {
        let layer = layer_with(&["Gaussian Blur", "Glow"]);
        let x = |node| position(&layer, node).x;
        assert!(x(NodeId::Source) < x(effect(0)) && x(effect(0)) < x(effect(1)) && x(effect(1)) < x(NodeId::Output));
    }
}
