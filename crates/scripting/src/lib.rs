// TODO: Fix doc comment generic types to use backticks
#![allow(rustdoc::invalid_html_tags)]
#![allow(rustdoc::broken_intra_doc_links)]

pub mod bindings;
pub mod comedi_bindings;
pub mod engine;
#[cfg(feature = "generic_driver")]
pub mod generic_driver_bindings;
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
use std::cell::RefCell;
use std::future::Future;
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio::task::block_in_place;

thread_local! {
    static SCRIPT_RUNTIME_HANDLE: RefCell<Option<Handle>> = RefCell::new(None);
}

/// Set the Tokio runtime handle for the current script thread.
/// This allows scripts running in dedicated threads (detached from Tokio)
/// to execute async hardware calls via `run_blocking`.
pub fn set_script_runtime_handle(handle: Handle) {
    SCRIPT_RUNTIME_HANDLE.with(|h| *h.borrow_mut() = Some(handle));
}

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
    // Try to get handle from thread-local storage first, then from Tokio context
    let handle = SCRIPT_RUNTIME_HANDLE
        .with(|h| h.borrow().clone())
        .or_else(|| Handle::try_current().ok())
        .ok_or_else(|| {
            rhai_error(
                &format!("{}: missing Tokio runtime", label),
                "No runtime available",
            )
        })?;

    // If we are in a Tokio runtime thread (TLS is set), we might need block_in_place.
    // If we are in a detached thread (TLS not set), block_on is safe.
    if Handle::try_current().is_ok() {
        if handle.runtime_flavor() == RuntimeFlavor::CurrentThread {
            return Err(Box::new(EvalAltResult::ErrorRuntime(
                format!(
                    "{}: Cannot block current-thread runtime from within runtime context",
                    label
                )
                .into(),
                Position::NONE,
            )));
        }
        block_in_place(|| handle.block_on(fut)).map_err(|e| rhai_error(label, e))
    } else {
        // We are in a detached thread, safe to block
        handle.block_on(fut).map_err(|e| rhai_error(label, e))
    }
}
