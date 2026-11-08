//! Node graph edits from the Nodes panel: connecting and disconnecting inputs, adding Merge
//! and Object Matte nodes, moving and removing nodes. See `studio_core::effect_graph`.

use studio_core::animation::Vec2;
use studio_core::effect_graph::{self, ConnectError, InputPort, MergeNode, NodeId};
use studio_core::model::{Id, Layer};

use super::EditorState;

impl EditorState {
    /// The layer `layer` in the open composition, to edit, unless it is locked.
    fn unlocked_layer_mut(&mut self, layer: Id) -> Option<&mut Layer> {
        self.composition_mut().layer_mut(layer).filter(|layer| !layer.switches.locked)
    }

    /// Connects `from`'s output into `port` on `to` in `layer`'s graph.
    pub fn connect_nodes(&mut self, layer: Id, from: NodeId, to: NodeId, port: InputPort) -> Result<(), ConnectError> {
        let layer = self.unlocked_layer_mut(layer).ok_or(ConnectError::MissingNode)?;
        effect_graph::connect(layer, from, to, port)
    }

    /// Takes the connection off `port` on `to`. Returns whether there was one.
    pub fn disconnect_node(&mut self, layer: Id, to: NodeId, port: InputPort) -> bool {
        self.unlocked_layer_mut(layer).is_some_and(|layer| effect_graph::disconnect(layer, to, port))
    }

    /// Adds an unconnected Merge node to `layer`'s graph at `at` on the canvas.
    pub fn add_merge_node(&mut self, layer: Id, at: Vec2) -> Option<NodeId> {
        let id = self.project.allocate_id();
        let layer = self.unlocked_layer_mut(layer)?;
        let node = effect_graph::add_merge(layer, id);
        effect_graph::set_position(layer, node, at);
        Some(node)
    }

    /// Puts the Object Matte node on `layer`'s canvas at `at`, for layers that can have an
    /// object matte (footage and images). Returns whether it was added.
    pub fn add_matte_node(&mut self, layer: Id, at: Vec2) -> bool {
        let Some(layer) = self.unlocked_layer_mut(layer) else { return false };
        if !studio_core::cutout::can_cut_out(layer) || !effect_graph::add_matte_node(layer) {
            return false;
        }
        effect_graph::set_position(layer, NodeId::Matte, at);
        true
    }

    /// Adds the catalog effect `name` to `layer` (it joins the graph just before the Output)
    /// and places its node at `at`. Returns its node.
    pub fn add_effect_node(&mut self, layer: Id, name: &str, at: Vec2) -> Option<NodeId> {
        let before: Vec<Id> = self.composition().layer(layer)?.effects.iter().map(|effect| effect.id).collect();
        self.add_effect(layer, name);
        let layer = self.composition_mut().layer_mut(layer)?;
        let added = layer.effects.iter().map(|effect| effect.id).find(|id| !before.contains(id))?;
        effect_graph::set_position(layer, NodeId::Effect(added), at);
        Some(NodeId::Effect(added))
    }

    /// Removes a node from `layer`'s graph: an effect is removed from the layer; a Merge or the
    /// Object Matte node just goes. Source and Output always stay.
    pub fn remove_graph_node(&mut self, layer: Id, node: NodeId) {
        match node {
            NodeId::Effect(effect) => self.remove_effect(layer, effect),
            NodeId::Source | NodeId::Output => {}
            NodeId::Merge(_) | NodeId::Matte => {
                if let Some(layer) = self.unlocked_layer_mut(layer) {
                    effect_graph::remove_node(layer, node);
                }
            }
        }
    }

    /// Turns an effect or Merge node off (it then passes its main picture through) or back on.
    pub fn toggle_graph_node(&mut self, layer: Id, node: NodeId) {
        match node {
            NodeId::Effect(effect) => self.toggle_effect(layer, effect),
            NodeId::Merge(id) => {
                if let Some(merge) = self.unlocked_layer_mut(layer).and_then(|layer| effect_graph::merge_mut(layer, id)) {
                    merge.enabled = !merge.enabled;
                }
            }
            _ => {}
        }
    }

    /// Moves a node on the canvas. (Placement isn't refused on locked layers: it changes
    /// nothing in the picture.)
    pub fn place_node(&mut self, layer: Id, node: NodeId, at: Vec2) {
        if let Some(layer) = self.composition_mut().layer_mut(layer) {
            effect_graph::set_position(layer, node, at);
        }
    }

    /// Changes a Merge node's settings with `change`.
    pub fn set_merge(&mut self, layer: Id, merge: Id, change: impl FnOnce(&mut MergeNode)) {
        if let Some(merge) = self.unlocked_layer_mut(layer).and_then(|layer| effect_graph::merge_mut(layer, merge)) {
            change(merge);
            merge.opacity = merge.opacity.clamp(0.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_edits_change_the_layers_graph_and_respect_locks() {
        let mut state = EditorState::new();
        let a_cam = Id(104);
        let blur = NodeId::Effect(Id(202));
        assert!(state.disconnect_node(a_cam, NodeId::Output, InputPort::Main));
        let merge = state.add_merge_node(a_cam, Vec2::new(10.0, 20.0)).unwrap();
        state.connect_nodes(a_cam, blur, merge, InputPort::Foreground).unwrap();
        state.connect_nodes(a_cam, merge, NodeId::Output, InputPort::Main).unwrap();
        let layer = state.composition().layer(a_cam).unwrap();
        assert_eq!(effect_graph::source_of(layer, NodeId::Output, InputPort::Main), Some(merge));
        assert_eq!(effect_graph::position(layer, merge), Vec2::new(10.0, 20.0));
        assert!(state.add_matte_node(a_cam, Vec2::new(0.0, 0.0)), "footage can have an object matte");

        state.remove_graph_node(a_cam, blur);
        let layer = state.composition().layer(a_cam).unwrap();
        assert!(layer.effects.is_empty());
        assert_eq!(effect_graph::source_of(layer, merge, InputPort::Foreground), Some(NodeId::Source), "what fed the blur now feeds what it fed");

        state.composition_mut_for_test().layer_mut(a_cam).unwrap().switches.locked = true;
        assert_eq!(state.connect_nodes(a_cam, NodeId::Source, NodeId::Output, InputPort::Main), Err(ConnectError::MissingNode));
        assert!(state.add_merge_node(a_cam, Vec2::new(0.0, 0.0)).is_none());
    }
}
