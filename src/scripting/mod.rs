//! Scripting engine for experimental control and automation.
//!
//! This module provides a unified scripting interface for controlling hardware and
//! automating experiments in rust-daq. It supports multiple scripting backends
//! (Rhai, Python) through the [`ScriptEngine`] trait.
//!
//! # Architecture
//!
//! ```text
//! ScriptEngine trait
//!     ├── RhaiEngine (embedded, zero-dep)
//!     └── PyO3Engine (Python via PyO3)
//!
//! Hardware Bindings
//!     ├── V5 bindings (StageHandle, CameraHandle)
//!     └── V3 bindings (V3StageHandle, V3CameraHandle, etc.)
//!
//! Plan Bindings (bd-73yh.4)
//!     └── RunEngineHandle, PlanHandle for experiment orchestration
//! ```
//!
//! # Choosing a Backend
//!
//! - **RhaiEngine**: Embedded scripting, zero external dependencies, fast startup
//! - **PyO3Engine**: Python backend, requires Python installation, runtime function registration
//!
//! # Example: Basic Rhai Scripting
//!
//! ```rust,ignore
//! use rust_daq::scripting::{ScriptEngine, RhaiEngine, ScriptValue};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut engine = RhaiEngine::new()?;
//!     
//!     engine.set_global("wavelength", ScriptValue::new(800_i64))?;
//!     
//!     let script = r#"
//!         print(`Wavelength: ${wavelength} nm`);
//!         wavelength * 2
//!     "#;
//!     
//!     let result = engine.execute_script(script).await?;
//!     println!("Result: {:?}", result);
//!     Ok(())
//! }
//! ```
//!
//! # Example: Hardware Control
//!
//! ```rust,ignore
//! use rust_daq::scripting::{RhaiEngine, ScriptEngine, ScriptValue, StageHandle};
//! use rust_daq::hardware::mock::MockStage;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut engine = RhaiEngine::with_hardware()?;
//!     
//!     engine.set_global("stage", ScriptValue::new(StageHandle {
//!         driver: Arc::new(MockStage::new()),
//!         data_tx: None,
//!     }))?;
//!     
//!     let script = r#"
//!         stage.move_abs(10.0);
//!         stage.wait_settled();
//!         let pos = stage.position();
//!         print(`Position: ${pos}mm`);
//!     "#;
//!     
//!     engine.execute_script(script).await?;
//!     Ok(())
//! }
//! ```
//!
//! # V3 vs V5 Bindings
//!
//! - **V5 bindings** (`bindings.rs`): Use capability traits (`Movable`, `Camera`)
//! - **V3 bindings** (`bindings_v3.rs`): Use V3 instrument traits (legacy, being phased out)
//!
//! Use V5 bindings for new code. V3 bindings exist for backward compatibility.

// V5 ScriptEngine trait and implementations
pub mod pyo3_engine;
pub mod rhai_engine;
pub mod script_engine;

// Legacy Rhai-specific (V4 compatibility)
pub mod bindings;
pub mod bindings_v3;
pub mod engine;

// Plan bindings for experiment orchestration (bd-73yh.4)
pub mod plan_bindings;

// Re-export V5 ScriptEngine types
#[cfg(feature = "scripting_python")]
pub use pyo3_engine::PyO3Engine;
pub use rhai_engine::RhaiEngine;
pub use script_engine::{ScriptEngine, ScriptError, ScriptValue};

// Re-export legacy types (V4)
pub use bindings::{register_hardware, CameraHandle, StageHandle};
pub use bindings_v3::{
    register_v3_hardware, V3CameraHandle, V3LaserHandle, V3PowerMeterHandle, V3StageHandle,
};
pub use engine::ScriptHost;

// Re-export plan bindings (bd-73yh.4)
pub use plan_bindings::{register_plans, PlanHandle, RunEngineHandle};
