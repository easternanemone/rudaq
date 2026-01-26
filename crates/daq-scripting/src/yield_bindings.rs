//! Yield Bindings - Rhai registration for yield-based plan scripting (bd-94zq.4)
//!
//! This module registers the `yield_plan` function and helper functions with
//! the Rhai engine, enabling scripts to yield plans and receive results.
//!
//! # Script Syntax
//!
//! ```rhai
//! // Yield a plan and get the result
//! let result = yield_plan(line_scan("x", 0, 10, 11, "det"));
//!
//! // Access result data
//! print(`Run: ${result.run_uid}`);
//! print(`Status: ${result.exit_status}`);
//! print(`Events: ${result.num_events}`);
//!
//! // Use data in conditionals
//! if result.data["det"] > threshold {
//!     yield_plan(high_res_scan(...));
//! }
//! ```
//!
//! # Yield Helpers
//!
//! For common operations, helper functions are provided:
//!
//! ```rhai
//! // Move a device (yields ImperativePlan internally)
//! yield_move("stage_x", 10.0);
//!
//! // Set a parameter
//! yield_set("laser", "wavelength", 800.0);
//!
//! // Wait for a duration
//! yield_wait(0.5);
//! ```

use std::sync::Arc;

use rhai::{Dynamic, Engine, EvalAltResult, Map};

use daq_experiment::plans_imperative::ImperativePlan;

use crate::plan_bindings::PlanHandle;
use crate::rhai_error;
use crate::yield_handle::{YieldHandle, YieldResult}; // bd-q2kl.5

/// Register yield-related functions with the Rhai engine
///
/// This must be called when creating an engine with yield support.
/// The engine must also have a YieldHandle set as a global variable.
pub fn register_yield_bindings(engine: &mut Engine) {
    // Register YieldResult type and accessors
    register_yield_result_type(engine);

    // Register the main yield_plan function
    // Note: This expects __yield_handle to be set as a global
    engine.register_fn("yield_plan", yield_plan_impl);

    // Register yield helpers for common operations
    engine.register_fn("yield_move", yield_move_impl);
    engine.register_fn("yield_set", yield_set_impl);
    engine.register_fn("yield_set_f64", yield_set_f64_impl);
    engine.register_fn("yield_wait", yield_wait_impl);
    engine.register_fn("yield_trigger", yield_trigger_impl);
    engine.register_fn("yield_read", yield_read_impl);
}

/// Register the YieldResult type and its accessor methods
fn register_yield_result_type(engine: &mut Engine) {
    // Register the type
    engine.register_type_with_name::<YieldResult>("YieldResult");

    // Register property getters
    engine.register_get("run_uid", |r: &mut YieldResult| r.run_uid.clone());
    engine.register_get("exit_status", |r: &mut YieldResult| r.exit_status.clone());
    engine.register_get("num_events", |r: &mut YieldResult| r.num_events as i64);
    engine.register_get("error", |r: &mut YieldResult| {
        r.error.clone().unwrap_or_default()
    });

    // Register data accessor - returns a Rhai Map
    engine.register_get("data", |r: &mut YieldResult| {
        let mut map = Map::new();
        for (k, v) in &r.data {
            map.insert(k.clone().into(), Dynamic::from(*v));
        }
        map
    });

    // Register positions accessor - returns a Rhai Map
    engine.register_get("positions", |r: &mut YieldResult| {
        let mut map = Map::new();
        for (k, v) in &r.positions {
            map.insert(k.clone().into(), Dynamic::from(*v));
        }
        map
    });

    // Register helper methods
    engine.register_fn("is_success", |r: &mut YieldResult| r.is_success());
    engine.register_fn("is_fail", |r: &mut YieldResult| r.is_fail());
    engine.register_fn("is_abort", |r: &mut YieldResult| r.is_abort());

    // Register indexer for data access: result["key"]
    engine.register_indexer_get(|r: &mut YieldResult, key: &str| -> Dynamic {
        if let Some(&value) = r.data.get(key) {
            Dynamic::from(value)
        } else if let Some(&value) = r.positions.get(key) {
            Dynamic::from(value)
        } else {
            Dynamic::UNIT
        }
    });
}

/// Implementation of yield_plan function
///
/// Called from Rhai scripts as: `let result = yield_plan(plan);`
fn yield_plan_impl(
    handle: Arc<YieldHandle>,
    plan: PlanHandle,
) -> Result<YieldResult, Box<EvalAltResult>> {
    // Take the plan from the handle
    let plan = plan
        .take()
        .ok_or_else(|| rhai_error("yield_plan", "Plan already consumed"))?;

    // Yield the plan and wait for result
    handle
        .yield_plan(plan)
        .map_err(|e| rhai_error("yield_plan", e))
}

/// Implementation of yield_move helper
///
/// Called from Rhai scripts as: `yield_move("stage_x", 10.0);`
fn yield_move_impl(
    handle: Arc<YieldHandle>,
    device_id: &str,
    position: f64,
) -> Result<YieldResult, Box<EvalAltResult>> {
    tracing::debug!(
        target: "daq_scripting::yield",
        device = %device_id,
        position = %position,
        "yield_move"
    );

    let plan = Box::new(ImperativePlan::move_to(device_id, position));

    handle
        .yield_plan(plan)
        .map_err(|e| rhai_error("yield_move", e))
}

/// Implementation of yield_set helper (string value)
///
/// Called from Rhai scripts as: `yield_set("laser", "wavelength", "800.0");`
fn yield_set_impl(
    handle: Arc<YieldHandle>,
    device_id: &str,
    parameter: &str,
    value: &str,
) -> Result<YieldResult, Box<EvalAltResult>> {
    tracing::debug!(
        target: "daq_scripting::yield",
        device = %device_id,
        parameter = %parameter,
        value = %value,
        "yield_set"
    );

    let plan = Box::new(ImperativePlan::set_parameter(device_id, parameter, value));

    handle
        .yield_plan(plan)
        .map_err(|e| rhai_error("yield_set", e))
}

/// Implementation of yield_set helper (f64 value)
///
/// Called from Rhai scripts as: `yield_set_f64("laser", "wavelength", 800.0);`
fn yield_set_f64_impl(
    handle: Arc<YieldHandle>,
    device_id: &str,
    parameter: &str,
    value: f64,
) -> Result<YieldResult, Box<EvalAltResult>> {
    yield_set_impl(handle, device_id, parameter, &value.to_string())
}

/// Implementation of yield_wait helper
///
/// Called from Rhai scripts as: `yield_wait(0.5);`
fn yield_wait_impl(
    handle: Arc<YieldHandle>,
    seconds: f64,
) -> Result<YieldResult, Box<EvalAltResult>> {
    tracing::debug!(
        target: "daq_scripting::yield",
        seconds = %seconds,
        "yield_wait"
    );

    let plan = Box::new(ImperativePlan::wait(seconds));

    handle
        .yield_plan(plan)
        .map_err(|e| rhai_error("yield_wait", e))
}

/// Implementation of yield_trigger helper
///
/// Called from Rhai scripts as: `yield_trigger("camera");`
fn yield_trigger_impl(
    handle: Arc<YieldHandle>,
    device_id: &str,
) -> Result<YieldResult, Box<EvalAltResult>> {
    tracing::debug!(
        target: "daq_scripting::yield",
        device = %device_id,
        "yield_trigger"
    );

    let plan = Box::new(ImperativePlan::trigger(device_id));

    handle
        .yield_plan(plan)
        .map_err(|e| rhai_error("yield_trigger", e))
}

/// Implementation of yield_read helper
///
/// Called from Rhai scripts as: `let result = yield_read("power_meter");`
fn yield_read_impl(
    handle: Arc<YieldHandle>,
    device_id: &str,
) -> Result<YieldResult, Box<EvalAltResult>> {
    tracing::debug!(
        target: "daq_scripting::yield",
        device = %device_id,
        "yield_read"
    );

    let plan = Box::new(ImperativePlan::read(device_id));

    handle
        .yield_plan(plan)
        .map_err(|e| rhai_error("yield_read", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yield_result_accessors() {
        use std::collections::HashMap;

        let mut data = HashMap::new();
        data.insert("power".to_string(), 42.0);

        let mut positions = HashMap::new();
        positions.insert("stage_x".to_string(), 10.0);

        let result = YieldResult {
            run_uid: "test_run".to_string(),
            exit_status: "success".to_string(),
            data,
            positions,
            num_events: 5,
            error: None,
        };

        assert!(result.is_success());
        assert!(!result.is_fail());
        assert_eq!(result.num_events, 5);
    }

    #[test]
    fn test_register_yield_bindings_compiles() {
        let mut engine = Engine::new();
        register_yield_bindings(&mut engine);

        // Just verify it compiles and doesn't panic
        // Full integration testing requires RunEngine setup
    }
}
