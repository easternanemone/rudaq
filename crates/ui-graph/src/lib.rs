//! Node graph editor for experiment design.
//!
//! Extracted from the `ui` crate per Microsoft Rust Guidelines M-SMALLER-CRATES
//! to improve compile times and enforce module boundaries.
//!
//! Contains the data model, validation, serialization, code generation, and
//! undo/redo infrastructure for the visual experiment programming editor.
//! The rendering layer (`viewer`) remains in the `ui` crate since it depends
//! on egui rendering context.

pub mod adaptive;
#[cfg(not(target_arch = "wasm32"))]
pub mod codegen;
pub mod commands;
pub mod execution_state;
pub mod nodes;
pub mod serialization;
#[cfg(not(target_arch = "wasm32"))]
pub mod translation;
pub mod validation;

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
pub use commands::{
    AddNodeData, ConnectNodesData, DisconnectNodesData, GraphEdit, GraphTarget, ModifyNodeData,
    RemoveNodeData,
};
pub use nodes::ExperimentNode;

/// Cross-platform time types.
///
/// On native: re-exports from `std::time`.
/// On WASM: uses `web_time::Instant` which delegates to `performance.now()`.
pub mod time {
    pub use std::time::Duration;

    #[cfg(not(target_arch = "wasm32"))]
    pub use std::time::Instant;

    #[cfg(target_arch = "wasm32")]
    pub use web_time::Instant;
}
