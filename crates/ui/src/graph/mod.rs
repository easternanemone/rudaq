//! Node graph editor module for experiment design.

#[allow(dead_code)]
pub mod adaptive;
// codegen + translation depend on the experiment crate (native-only)
#[cfg(not(target_arch = "wasm32"))]
pub mod codegen;
pub mod commands;
pub mod execution_state;
pub mod nodes;
pub mod serialization;
#[cfg(not(target_arch = "wasm32"))]
pub mod translation;
pub mod validation;
pub mod viewer;

#[cfg(not(target_arch = "wasm32"))]
pub use codegen::graph_to_rhai_script;
/// Placeholder on WASM — code generation requires the desktop application.
#[cfg(target_arch = "wasm32")]
pub fn graph_to_rhai_script(
    _snarl: &egui_snarl::Snarl<nodes::ExperimentNode>,
    _filename: Option<&str>,
) -> String {
    "// Code generation requires the desktop application\n".to_string()
}
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
    GRAPH_FILE_EXTENSION, GRAPH_FILE_FILTER, GraphFile, GraphMetadata, load_graph, save_graph,
};
#[cfg(not(target_arch = "wasm32"))]
#[allow(unused_imports)]
pub use translation::{
    GraphPlan, TranslationError, build_adjacency, detect_cycles, topological_sort,
};
#[allow(unused_imports)]
pub use validation::{
    PinType, input_pin_type, output_pin_type, validate_adaptive_scan, validate_connection,
    validate_graph_structure,
};
pub use viewer::ExperimentViewer;

// Adaptive scan trigger evaluation
#[allow(unused_imports)]
pub use adaptive::{DetectedPeak, TriggerResult, detect_peaks, evaluate_triggers};

// Re-export Snarl for convenience
#[allow(unused_imports)]
pub use egui_snarl::Snarl;
