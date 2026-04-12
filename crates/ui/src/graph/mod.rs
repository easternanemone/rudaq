//! Node graph editor module for experiment design.
//!
//! Core logic (nodes, commands, codegen, validation, serialization) lives in
//! the `ui-graph` crate. This module re-exports everything and adds the
//! rendering layer (`viewer`) which depends on egui context.

// Re-export all public items from ui-graph
pub use ui_graph::adaptive;
#[cfg(not(target_arch = "wasm32"))]
//
pub use ui_graph::commands;
pub use ui_graph::execution_state;
pub use ui_graph::nodes;
pub use ui_graph::serialization;
#[cfg(not(target_arch = "wasm32"))]
pub use ui_graph::translation;
pub use ui_graph::validation;

// Viewer stays local — it depends on egui rendering context
pub mod viewer;

#[cfg(not(target_arch = "wasm32"))]
pub use ui_graph::graph_to_rhai_script;
#[cfg(target_arch = "wasm32")]
pub use ui_graph::graph_to_rhai_script;

#[cfg(not(target_arch = "wasm32"))]
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

#[allow(unused_imports)]
pub use adaptive::{DetectedPeak, TriggerResult, detect_peaks, evaluate_triggers};

// Re-export Snarl for convenience
#[allow(unused_imports)]
pub use egui_snarl::Snarl;
