//! Edit commands for undo/redo functionality in the graph editor.

use egui_snarl::{InPinId, NodeId, OutPinId, Snarl};
use undo::{Edit, Merged};

use super::ExperimentNode;

/// The target type for all graph edits.
pub type GraphTarget = Snarl<ExperimentNode>;

/// Unified edit command for all graph operations.
///
/// This enum wraps all edit command types so they can be stored
/// in a single `undo::Record<GraphEdit>`.
#[derive(Clone)]
#[allow(dead_code)]
pub enum GraphEdit {
    AddNode(AddNodeData),
    RemoveNode(RemoveNodeData),
    ModifyNode(ModifyNodeData),
    ConnectNodes(ConnectNodesData),
    DisconnectNodes(DisconnectNodesData),
}

impl Edit for GraphEdit {
    type Target = GraphTarget;
    type Output = ();

    fn edit(&mut self, target: &mut Self::Target) -> Self::Output {
        match self {
            GraphEdit::AddNode(data) => {
                let id = target.insert_node(data.position, data.node.clone());
                data.node_id = Some(id);
            }
            GraphEdit::RemoveNode(data) => {
                if let Some(node_info) = target.get_node_info(data.node_id) {
                    data.node = Some(node_info.value.clone());
                    data.position = Some(node_info.pos);
                }
                target.remove_node(data.node_id);
            }
            GraphEdit::ModifyNode(data) => {
                if let Some(node) = target.get_node_mut(data.node_id) {
                    *node = data.new_data.clone();
                }
            }
            GraphEdit::ConnectNodes(data) => {
                target.connect(data.from, data.to);
            }
            GraphEdit::DisconnectNodes(data) => {
                target.disconnect(data.from, data.to);
            }
        }
    }

    fn undo(&mut self, target: &mut Self::Target) -> Self::Output {
        match self {
            GraphEdit::AddNode(data) => {
                if let Some(id) = data.node_id {
                    target.remove_node(id);
                }
            }
            GraphEdit::RemoveNode(data) => {
                if let (Some(node), Some(pos)) = (data.node.take(), data.position) {
                    target.insert_node(pos, node);
                }
            }
            GraphEdit::ModifyNode(data) => {
                if let Some(node) = target.get_node_mut(data.node_id) {
                    *node = data.old_data.clone();
                }
            }
            GraphEdit::ConnectNodes(data) => {
                target.disconnect(data.from, data.to);
            }
            GraphEdit::DisconnectNodes(data) => {
                target.connect(data.from, data.to);
            }
        }
    }

    fn merge(&mut self, other: Self) -> Merged<Self>
    where
        Self: Sized,
    {
        // Only merge consecutive modifications to the same node
        match (self, &other) {
            (GraphEdit::ModifyNode(this), GraphEdit::ModifyNode(other_data)) => {
                if this.node_id == other_data.node_id {
                    this.new_data = other_data.new_data.clone();
                    Merged::Yes
                } else {
                    Merged::No(other)
                }
            }
            _ => Merged::No(other),
        }
    }
}

/// Data for AddNode command.
#[derive(Clone)]
pub struct AddNodeData {
    /// The node to add.
    pub node: ExperimentNode,
    /// Position for the new node.
    pub position: egui::Pos2,
    /// Set after edit() to allow undo.
    pub node_id: Option<NodeId>,
}

/// Data for RemoveNode command.
#[derive(Clone)]
pub struct RemoveNodeData {
    /// ID of the node to remove.
    pub node_id: NodeId,
    /// Stored for undo - the node data.
    pub node: Option<ExperimentNode>,
    /// Stored for undo - the node position.
    pub position: Option<egui::Pos2>,
}

/// Data for ModifyNode command.
#[derive(Clone)]
pub struct ModifyNodeData {
    /// ID of the node to modify.
    pub node_id: NodeId,
    /// The old node data (before modification).
    pub old_data: ExperimentNode,
    /// The new node data (after modification).
    pub new_data: ExperimentNode,
}

/// Data for ConnectNodes command.
#[derive(Clone, Copy)]
pub struct ConnectNodesData {
    /// Output pin to connect from.
    pub from: OutPinId,
    /// Input pin to connect to.
    pub to: InPinId,
}

/// Data for DisconnectNodes command.
#[derive(Clone, Copy)]
pub struct DisconnectNodesData {
    /// Output pin to disconnect from.
    pub from: OutPinId,
    /// Input pin to disconnect to.
    pub to: InPinId,
}
