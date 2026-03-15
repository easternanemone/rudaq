//! Rhai bindings for `AcquisitionCoordinator` (bd-u32r)
//!
//! Exposes the `AcquisitionCoordinator` struct to Rhai scripts so users can
//! construct and run move-trigger-acquire workflows from scripts.
//!
//! # Available Functions
//!
//! - `acquisition_coordinator(stage, trigger, frame_producer)` - Constructor
//! - `coord.stage_id()` - Get stage device ID
//! - `coord.trigger_id()` - Get trigger device ID
//! - `coord.frame_producer_id()` - Get frame producer device ID
//! - `coord.validate(registry)` - Validate all devices are present
//! - `coord.acquire_at(registry, position)` - Acquire at a single position
//! - `coord.acquire_scan(registry, positions)` - Acquire at multiple positions
//!
//! # Example
//!
//! ```rhai
//! let coord = acquisition_coordinator("stage_x", "camera", "camera");
//! print(`Stage: ${coord.stage_id}`);
//!
//! // Validate devices before scanning
//! coord.validate(registry);
//!
//! // Single acquisition
//! coord.acquire_at(registry, 10.5);
//!
//! // Scan across positions
//! coord.acquire_scan(registry, [0.0, 5.0, 10.0, 15.0, 20.0]);
//! ```

use rhai::{Array, Engine, EvalAltResult};
use std::sync::Arc;

use crate::{rhai_error, run_blocking};
use experiment::AcquisitionCoordinator;
use hardware::registry::DeviceRegistry;

/// Rhai-compatible handle wrapping an `AcquisitionCoordinator`.
///
/// The coordinator is behind an `Arc` so Rhai's `Clone` requirement is cheap.
#[derive(Clone)]
pub struct CoordinatorHandle {
    pub inner: Arc<AcquisitionCoordinator>,
}

/// Register all `AcquisitionCoordinator` bindings with the Rhai engine.
pub fn register_coordinator(engine: &mut Engine) {
    // -- Type registration -----------------------------------------------
    engine.register_type_with_name::<CoordinatorHandle>("AcquisitionCoordinator");

    // -- Constructor ------------------------------------------------------
    engine.register_fn(
        "acquisition_coordinator",
        |stage: &str, trigger: &str, frame_producer: &str| -> CoordinatorHandle {
            CoordinatorHandle {
                inner: Arc::new(AcquisitionCoordinator::new(stage, trigger, frame_producer)),
            }
        },
    );

    // -- Property getters -------------------------------------------------
    engine.register_get("stage_id", |h: &mut CoordinatorHandle| -> String {
        h.inner.stage_id().to_string()
    });

    engine.register_get("trigger_id", |h: &mut CoordinatorHandle| -> String {
        h.inner.trigger_id().to_string()
    });

    engine.register_get("frame_producer_id", |h: &mut CoordinatorHandle| -> String {
        h.inner.frame_producer_id().to_string()
    });

    // -- validate(registry) -----------------------------------------------
    engine.register_fn(
        "validate",
        |h: &mut CoordinatorHandle,
         registry: Arc<DeviceRegistry>|
         -> Result<(), Box<EvalAltResult>> {
            h.inner
                .validate(registry.as_ref())
                .map_err(|e| rhai_error("AcquisitionCoordinator validate", e))
        },
    );

    // -- acquire_at(registry, position) -----------------------------------
    engine.register_fn(
        "acquire_at",
        |h: &mut CoordinatorHandle,
         registry: Arc<DeviceRegistry>,
         position: f64|
         -> Result<(), Box<EvalAltResult>> {
            let coord = h.inner.clone();
            run_blocking("AcquisitionCoordinator acquire_at", async move {
                coord.acquire_at(registry.as_ref(), position).await
            })
        },
    );

    // -- acquire_scan(registry, positions_array) --------------------------
    engine.register_fn(
        "acquire_scan",
        |h: &mut CoordinatorHandle,
         registry: Arc<DeviceRegistry>,
         positions: Array|
         -> Result<(), Box<EvalAltResult>> {
            // Convert Rhai Array (Vec<Dynamic>) to Vec<f64>
            let positions: Vec<f64> = positions
                .into_iter()
                .enumerate()
                .map(|(i, v)| {
                    v.as_float().map_err(|_| {
                        rhai_error(
                            "acquire_scan",
                            format!("positions[{}] is not a float (got {})", i, v.type_name()),
                        )
                    })
                })
                .collect::<Result<Vec<f64>, _>>()?;

            let coord = h.inner.clone();
            run_blocking("AcquisitionCoordinator acquire_scan", async move {
                coord.acquire_scan(registry.as_ref(), &positions).await
            })
        },
    );
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use rhai::Engine;

    #[test]
    fn test_coordinator_bindings_register() {
        let mut engine = Engine::new();
        register_coordinator(&mut engine);
        // Should not panic -- bindings are registered
    }

    #[test]
    fn test_coordinator_construction_from_rhai() {
        let mut engine = Engine::new();
        register_coordinator(&mut engine);

        let handle: CoordinatorHandle = engine
            .eval(r#"acquisition_coordinator("stage_x", "camera", "camera")"#)
            .expect("Failed to construct coordinator from Rhai");

        assert_eq!(handle.inner.stage_id(), "stage_x");
        assert_eq!(handle.inner.trigger_id(), "camera");
        assert_eq!(handle.inner.frame_producer_id(), "camera");
    }

    #[test]
    fn test_coordinator_property_getters() {
        let mut engine = Engine::new();
        register_coordinator(&mut engine);

        let stage_id: String = engine
            .eval(
                r#"
                let coord = acquisition_coordinator("s1", "t1", "fp1");
                coord.stage_id
            "#,
            )
            .expect("Failed to get stage_id");
        assert_eq!(stage_id, "s1");

        let trigger_id: String = engine
            .eval(
                r#"
                let coord = acquisition_coordinator("s1", "t1", "fp1");
                coord.trigger_id
            "#,
            )
            .expect("Failed to get trigger_id");
        assert_eq!(trigger_id, "t1");

        let fp_id: String = engine
            .eval(
                r#"
                let coord = acquisition_coordinator("s1", "t1", "fp1");
                coord.frame_producer_id
            "#,
            )
            .expect("Failed to get frame_producer_id");
        assert_eq!(fp_id, "fp1");
    }

    #[test]
    fn test_coordinator_with_rhai_engine() {
        use crate::rhai_engine::RhaiEngine;
        use crate::traits::ScriptEngine;

        let mut engine = RhaiEngine::with_hardware().expect("Failed to create RhaiEngine");

        // Construct a coordinator from a script and read back properties
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            engine
                .execute_script(
                    r#"
                    let coord = acquisition_coordinator("stage_x", "cam", "cam");
                    coord.stage_id + ":" + coord.trigger_id + ":" + coord.frame_producer_id
                "#,
                )
                .await
        });

        let value: String = result.expect("script failed").downcast().unwrap();
        assert_eq!(value, "stage_x:cam:cam");
    }
}
