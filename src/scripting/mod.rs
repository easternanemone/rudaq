// V5 ScriptEngine trait and implementations
pub mod script_engine;
pub mod rhai_engine;
pub mod pyo3_engine;

// Legacy Rhai-specific (V4 compatibility)
pub mod bindings;
pub mod bindings_v3;
pub mod engine;
pub mod rhai_engine;
pub mod script_engine;

// Re-export V5 ScriptEngine types
pub use script_engine::{ScriptEngine, ScriptError, ScriptValue};
pub use rhai_engine::RhaiEngine;
#[cfg(feature = "scripting_python")]
pub use pyo3_engine::PyO3Engine;

// Re-export legacy types (V4)
pub use bindings::{register_hardware, CameraHandle, StageHandle};
pub use bindings_v3::{
    register_v3_hardware, V3CameraHandle, V3LaserHandle, V3PowerMeterHandle, V3StageHandle,
};
pub use engine::ScriptHost;
pub use rhai_engine::RhaiEngine;
pub use script_engine::{ScriptEngine, ScriptError, ScriptValue};
