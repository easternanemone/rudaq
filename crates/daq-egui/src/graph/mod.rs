//! Node graph editor module for experiment design.

pub mod commands;
pub mod execution_state;
pub mod nodes;
pub mod serialization;
pub mod translation;
pub mod validation;
pub mod viewer;

pub use commands::{
    AddNodeData, ConnectNodesData, DisconnectNodesData, GraphEdit, GraphTarget, ModifyNodeData,
    RemoveNodeData,
};
pub use execution_state::{EngineStateLocal, ExecutionState, NodeExecutionState};
pub use nodes::ExperimentNode;
pub use serialization::{
    load_graph, save_graph, GraphFile, GraphMetadata, GRAPH_FILE_EXTENSION, GRAPH_FILE_FILTER,
};
pub use translation::{detect_cycles, GraphPlan, TranslationError};
pub use validation::{
    input_pin_type, output_pin_type, validate_connection, validate_graph_structure, PinType,
};
pub use viewer::ExperimentViewer;

// Re-export Snarl for convenience
#[allow(unused_imports)]
pub use egui_snarl::Snarl;
