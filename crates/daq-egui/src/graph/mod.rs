//! Node graph editor module for experiment design.

pub mod commands;
pub mod nodes;
pub mod viewer;

pub use commands::{AddNode, ConnectNodes, DisconnectNodes, GraphTarget, ModifyNode, RemoveNode};
pub use nodes::ExperimentNode;
pub use viewer::ExperimentViewer;

// Re-export Snarl for convenience
pub use egui_snarl::Snarl;
