//! Edit commands for undo/redo functionality in the graph editor.

use egui_snarl::{InPinId, NodeId, OutPinId, Snarl};
use undo::{Edit, Merged};

use super::ExperimentNode;

/// The target type for all graph edits.
pub type GraphTarget = Snarl<ExperimentNode>;

/// Add a node to the graph.
pub struct AddNode {
    /// The node to add.
    pub node: ExperimentNode,
    /// Position for the new node.
    pub position: egui::Pos2,
    /// Set after edit() to allow undo.
    pub node_id: Option<NodeId>,
}

impl Edit for AddNode {
    type Target = GraphTarget;
    type Output = ();

    fn edit(&mut self, target: &mut Self::Target) -> Self::Output {
        // Insert node and store ID for undo
        let id = target.insert_node(self.position, self.node.clone());
        self.node_id = Some(id);
    }

    fn undo(&mut self, target: &mut Self::Target) -> Self::Output {
        if let Some(id) = self.node_id {
            target.remove_node(id);
        }
    }
}

/// Remove a node from the graph.
pub struct RemoveNode {
    /// ID of the node to remove.
    pub node_id: NodeId,
    /// Stored for undo - the node data.
    pub node: Option<ExperimentNode>,
    /// Stored for undo - the node position.
    pub position: Option<egui::Pos2>,
}

impl Edit for RemoveNode {
    type Target = GraphTarget;
    type Output = ();

    fn edit(&mut self, target: &mut Self::Target) -> Self::Output {
        // Store node data and position for undo
        if let Some(node_info) = target.get_node_info(self.node_id) {
            self.node = Some(node_info.value.clone());
            self.position = Some(node_info.pos);
        }
        target.remove_node(self.node_id);
    }

    fn undo(&mut self, target: &mut Self::Target) -> Self::Output {
        if let (Some(node), Some(pos)) = (self.node.take(), self.position) {
            // Re-insert the node at its original position
            // Note: node ID may change after re-insert
            target.insert_node(pos, node);
        }
    }
}

/// Modify a node's properties.
pub struct ModifyNode {
    /// ID of the node to modify.
    pub node_id: NodeId,
    /// The old node data (before modification).
    pub old_data: ExperimentNode,
    /// The new node data (after modification).
    pub new_data: ExperimentNode,
}

impl Edit for ModifyNode {
    type Target = GraphTarget;
    type Output = ();

    fn edit(&mut self, target: &mut Self::Target) -> Self::Output {
        if let Some(node) = target.get_node_mut(self.node_id) {
            *node = self.new_data.clone();
        }
    }

    fn undo(&mut self, target: &mut Self::Target) -> Self::Output {
        if let Some(node) = target.get_node_mut(self.node_id) {
            *node = self.old_data.clone();
        }
    }

    fn merge(&mut self, other: Self) -> Merged<Self>
    where
        Self: Sized,
    {
        // Merge consecutive modifications to the same node
        if self.node_id == other.node_id {
            self.new_data = other.new_data;
            Merged::Yes
        } else {
            Merged::No(other)
        }
    }
}

/// Connect two nodes via their pins.
pub struct ConnectNodes {
    /// Output pin to connect from.
    pub from: OutPinId,
    /// Input pin to connect to.
    pub to: InPinId,
}

impl Edit for ConnectNodes {
    type Target = GraphTarget;
    type Output = ();

    fn edit(&mut self, target: &mut Self::Target) -> Self::Output {
        target.connect(self.from, self.to);
    }

    fn undo(&mut self, target: &mut Self::Target) -> Self::Output {
        target.disconnect(self.from, self.to);
    }
}

/// Disconnect two nodes.
pub struct DisconnectNodes {
    /// Output pin to disconnect from.
    pub from: OutPinId,
    /// Input pin to disconnect to.
    pub to: InPinId,
}

impl Edit for DisconnectNodes {
    type Target = GraphTarget;
    type Output = ();

    fn edit(&mut self, target: &mut Self::Target) -> Self::Output {
        target.disconnect(self.from, self.to);
    }

    fn undo(&mut self, target: &mut Self::Target) -> Self::Output {
        target.connect(self.from, self.to);
    }
}
