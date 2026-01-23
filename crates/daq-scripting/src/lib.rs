// TODO: Fix doc comment generic types to use backticks
#![allow(rustdoc::invalid_html_tags)]
#![allow(rustdoc::broken_intra_doc_links)]

pub mod bindings;
pub mod comedi_bindings;
pub mod engine;
pub mod plan_bindings;
pub mod rhai_engine;
pub mod script_runner;
pub mod traits;
pub mod yield_bindings;
pub mod yield_handle;

#[cfg(feature = "python")]
pub mod pyo3_engine;

pub use bindings::{CameraHandle, ReadableHandle, ShutterHandle, SoftLimits, StageHandle};

#[cfg(feature = "scripting_full")]
pub use bindings::Ell14Handle;

#[cfg(feature = "hdf5_scripting")]
pub use bindings::Hdf5Handle;
pub use comedi_bindings::{
    register_comedi_hardware, AnalogInput, AnalogInputHandle, AnalogOutput, AnalogOutputHandle,
    Counter, CounterHandle, DigitalIO, DigitalIOHandle,
};
pub use rhai_engine::RhaiEngine;
pub use script_runner::{ScriptPlanRunner, ScriptRunConfig, ScriptRunReport};
pub use traits::{ScriptEngine, ScriptError, ScriptValue};
pub use yield_handle::{YieldChannelBuilder, YieldHandle, YieldResult, YieldedValue};

#[cfg(feature = "python")]
pub use pyo3_engine::PyO3Engine;

pub use rhai;
