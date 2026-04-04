// Suppressed: generic types in doc comments parse as HTML tags
#![allow(rustdoc::invalid_html_tags)]
#![allow(rustdoc::broken_intra_doc_links)]
// Allow Rust 1.93+ clippy lints that require significant refactoring
#![allow(
    clippy::unnecessary_literal_bound,         // lifetime-related str returns
    clippy::assigning_clones,                  // clone vs clone_from
    clippy::needless_continue,                 // redundant continue
    clippy::iter_over_hash_type,               // iter over HashMap
    clippy::missing_const_for_thread_local,    // thread_local const init (renamed from thread_local_initializer_can_be_made_const)
    clippy::manual_let_else,                   // if-let to let-else
    clippy::match_single_binding,              // matching over ()
    clippy::manual_string_new,                 // String::new() vs "".to_string()
    clippy::explicit_iter_loop,                // for x in iter.iter() vs for x in iter
    clippy::semicolon_if_nothing_returned,     // missing semicolons
    clippy::ignored_unit_patterns              // _ vs () in patterns
)]

pub mod bindings;
pub mod comedi_bindings;
pub mod coordinator_bindings;
pub mod libs_bindings;
pub mod path_security;
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
pub use coordinator_bindings::CoordinatorHandle;

#[cfg(feature = "hdf5_scripting")]
pub use bindings::Hdf5Handle;
pub use comedi_bindings::{
    AnalogInput, AnalogInputHandle, AnalogOutput, AnalogOutputHandle, Counter, CounterHandle,
    DigitalIO, DigitalIOHandle, register_comedi_hardware,
};
pub use libs_bindings::{
    CalibratorHandle, DoverAxisHandle, GatedCameraHandle, ScanController, SpectrographHandle,
    register_libs_hardware,
};
pub use rhai_engine::RhaiEngine;
pub use script_runner::{ScriptPlanRunner, ScriptRunConfig, ScriptRunReport};
pub use shutter_safety::{DEFAULT_HEARTBEAT_TIMEOUT, HeartbeatShutterGuard, ShutterRegistry};
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
use std::collections::HashMap;
use std::future::Future;
use std::sync::{LazyLock, RwLock};
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio::task::block_in_place;

thread_local! {
    static SCRIPT_RUNTIME_HANDLE: RefCell<Option<Handle>> = const { RefCell::new(None) };
    static SCRIPT_CALIBRATION_CONTEXT: RefCell<Option<String>> = const { RefCell::new(None) };
}

static ACTIVE_SCRIPT_CALIBRATIONS: LazyLock<
    RwLock<HashMap<String, HashMap<String, experiment::CalibrationFreshness>>>,
> = LazyLock::new(|| RwLock::new(HashMap::new()));

/// Set the Tokio runtime handle for the current script thread.
/// This allows scripts running in dedicated threads (detached from Tokio)
/// to execute async hardware calls via `run_blocking`.
pub fn set_script_runtime_handle(handle: Handle) {
    SCRIPT_RUNTIME_HANDLE.with(|h| *h.borrow_mut() = Some(handle));
}

/// Bind subsequent calibration updates on this thread to a script execution context.
pub fn set_script_calibration_context(context_id: String) {
    SCRIPT_CALIBRATION_CONTEXT.with(|ctx| *ctx.borrow_mut() = Some(context_id));
}

/// Clear the current script calibration context binding.
pub fn clear_script_calibration_context() {
    SCRIPT_CALIBRATION_CONTEXT.with(|ctx| *ctx.borrow_mut() = None);
}

/// Update the active calibration snapshot for the current script execution context.
pub fn update_current_script_calibration(snapshot: experiment::CalibrationFreshness) {
    let context_id = SCRIPT_CALIBRATION_CONTEXT.with(|ctx| ctx.borrow().clone());
    let Some(context_id) = context_id else {
        return;
    };
    let mut snapshot = snapshot;
    snapshot.device_type = snapshot.device_type.to_ascii_lowercase();

    let mut all_contexts = ACTIVE_SCRIPT_CALIBRATIONS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    all_contexts
        .entry(context_id)
        .or_default()
        .insert(snapshot.device_type.clone(), snapshot);
}

/// Read the latest calibration snapshot for a script execution context and device type.
pub fn current_script_calibration(
    context_id: &str,
    device_type: &str,
) -> Option<experiment::CalibrationFreshness> {
    ACTIVE_SCRIPT_CALIBRATIONS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(context_id)
        .and_then(|by_type| by_type.get(&device_type.to_ascii_lowercase()))
        .cloned()
}

/// Drop all calibration snapshots associated with a script execution context.
pub fn clear_script_calibrations(context_id: &str) {
    ACTIVE_SCRIPT_CALIBRATIONS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(context_id);
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
        format!("{label}: {error}").into(),
        Position::NONE,
    ))
}

/// Create a Rhai runtime error from a plain string message (no secondary error value).
///
/// Use this variant when the error is self-describing (e.g., validation failures).
///
/// # Example
/// ```ignore
/// u64::try_from(val).map_err(|_| rhai_error_str("value must be non-negative"))?;
/// ```
pub fn rhai_error_str(message: &str) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorRuntime(message.into(), Position::NONE))
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
                &format!("{label}: missing Tokio runtime"),
                "No runtime available",
            )
        })?;

    // If we are in a Tokio runtime thread (TLS is set), we might need block_in_place.
    // If we are in a detached thread (TLS not set), block_on is safe.
    if Handle::try_current().is_ok() {
        if handle.runtime_flavor() == RuntimeFlavor::CurrentThread {
            return Err(Box::new(EvalAltResult::ErrorRuntime(
                format!("{label}: Cannot block current-thread runtime from within runtime context")
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
