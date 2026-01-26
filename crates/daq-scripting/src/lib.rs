// TODO: Fix doc comment generic types to use backticks
#![allow(rustdoc::invalid_html_tags)]
#![allow(rustdoc::broken_intra_doc_links)]

pub mod bindings;
pub mod comedi_bindings;
pub mod engine;
pub mod plan_bindings;
pub mod rhai_engine;
pub mod script_runner;
pub mod shutter_safety;
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
pub use shutter_safety::{HeartbeatShutterGuard, ShutterRegistry, DEFAULT_HEARTBEAT_TIMEOUT};
pub use traits::{ScriptEngine, ScriptError, ScriptValue};
pub use yield_handle::{YieldChannelBuilder, YieldHandle, YieldResult, YieldedValue};

#[cfg(feature = "python")]
pub use pyo3_engine::PyO3Engine;

pub use rhai;

// =============================================================================
// Rhai Error Helpers - bd-q2kl.5
// =============================================================================

use rhai::{EvalAltResult, Position};
use std::future::Future;
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio::task::block_in_place;

/// Create a Rhai runtime error with a formatted message
///
/// This helper eliminates the repetitive pattern of:
/// ```ignore
/// Box::new(EvalAltResult::ErrorRuntime(
///     format!("...: {}", e).into(),
///     Position::NONE,
/// ))
/// ```
///
/// # Example
/// ```ignore
/// some_operation().map_err(|e| rhai_error("Operation failed", e))
/// ```
pub fn rhai_error(label: &str, error: impl std::fmt::Display) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorRuntime(
        format!("{}: {}", label, error).into(),
        Position::NONE,
    ))
}

/// Execute an async future in a blocking context for Rhai bindings
///
/// This helper safely bridges async Rust hardware traits to synchronous Rhai scripts.
/// It validates the Tokio runtime flavor to prevent deadlocks.
///
/// # Errors
/// - Returns error if no Tokio runtime is available
/// - Returns error if running on current-thread runtime (would deadlock)
/// - Propagates any error from the future
///
/// # Example
/// ```ignore
/// run_blocking("move_abs", driver.move_abs(position))
/// ```
pub fn run_blocking<Fut, T, E>(label: &str, fut: Fut) -> Result<T, Box<EvalAltResult>>
where
    Fut: Future<Output = Result<T, E>> + Send,
    T: Send,
    E: std::fmt::Display,
{
    let handle = Handle::try_current()
        .map_err(|e| rhai_error(&format!("{}: missing Tokio runtime", label), e))?;

    if handle.runtime_flavor() == RuntimeFlavor::CurrentThread {
        return Err(Box::new(EvalAltResult::ErrorRuntime(
            format!(
                "{}: Tokio current-thread runtime cannot run blocking hardware calls. \
                 Use the multi-thread runtime (#[tokio::main(flavor = \"multi_thread\")]).",
                label
            )
            .into(),
            Position::NONE,
        )));
    }

    block_in_place(|| handle.block_on(fut)).map_err(|e| rhai_error(label, e))
}
