//! Node graph editor module for experiment design.

pub mod adaptive;
pub mod codegen;
pub mod commands;
pub mod execution_state;
pub mod nodes;
pub mod serialization;
pub mod translation;
pub mod validation;
pub mod viewer;

pub use codegen::graph_to_rhai_script;
#[allow(unused_imports)]
pub use commands::{
    AddNodeData, ConnectNodesData, DisconnectNodesData, GraphEdit, GraphTarget, ModifyNodeData,
    RemoveNodeData,
};
#[allow(unused_imports)]
pub use execution_state::{EngineStateLocal, ExecutionState, NodeExecutionState};
pub use nodes::ExperimentNode;
#[allow(unused_imports)]
pub use serialization::{
    load_graph, save_graph, GraphFile, GraphMetadata, GRAPH_FILE_EXTENSION, GRAPH_FILE_FILTER,
};
#[allow(unused_imports)]
pub use translation::{
    build_adjacency, detect_cycles, topological_sort, GraphPlan, TranslationError,
};
#[allow(unused_imports)]
pub use validation::{
    input_pin_type, output_pin_type, validate_adaptive_scan, validate_connection,
    validate_graph_structure, PinType,
};
pub use viewer::ExperimentViewer;

// Adaptive scan trigger evaluation
#[allow(unused_imports)]
pub use adaptive::{detect_peaks, evaluate_triggers, DetectedPeak, TriggerResult};

// Re-export Snarl for convenience
#[allow(unused_imports)]
pub use egui_snarl::Snarl;
