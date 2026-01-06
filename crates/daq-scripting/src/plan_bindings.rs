//! Plan Bindings for Rhai Scripts (bd-73yh.4)
//!
//! Provides high-level experiment plan functions for Rhai scripting.
//! These bindings allow scripts to create and run declarative scans.
//!
//! # Available Functions
//!
//! - `line_scan(motor, start, end, points, detector)` - 1D linear scan
//! - `grid_scan(x_motor, x_start, x_end, x_points, y_motor, y_start, y_end, y_points, detector)` - 2D grid
//! - `count(num_points, detector, dwell)` - Repeated measurements
//!
//! # Example Usage
//!
//! ```rhai
//! // Simple line scan
//! let scan = line_scan("stage_x", 0.0, 10.0, 11, "power_meter");
//! run_engine.queue(scan);
//! run_engine.start();
//!
//! // Subscribe to documents
//! for doc in run_engine.documents() {
//!     if doc.type == "event" {
//!         print(`Point ${doc.seq_num}: power = ${doc.data.power_meter}`);
//!     }
//! }
//! ```

use crate::rhai::{Engine, EvalAltResult, Position};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::task::block_in_place;

use daq_experiment::plans::{Count, GridScan, LineScan, Plan};
use daq_experiment::run_engine::RunEngine;
use daq_hardware::registry::DeviceRegistry;

// =============================================================================
// RunEngine Handle - Rhai-Compatible Wrapper
// =============================================================================

/// Handle to RunEngine for use in Rhai scripts
///
/// Provides methods to queue plans, start/pause/resume execution,
/// and subscribe to document streams.
#[derive(Clone)]
pub struct RunEngineHandle {
    /// The underlying RunEngine
    pub engine: Arc<RunEngine>,
}

impl RunEngineHandle {
    /// Create a new RunEngineHandle wrapping a DeviceRegistry
    pub fn new(registry: Arc<DeviceRegistry>) -> Self {
        Self {
            engine: Arc::new(RunEngine::new(registry)),
        }
    }

    /// Create from an existing RunEngine
    pub fn from_engine(engine: Arc<RunEngine>) -> Self {
        Self { engine }
    }
}

// =============================================================================
// Plan Handle - Boxed Plan for Rhai
// =============================================================================

/// Handle to a Plan for use in Rhai scripts
///
/// Wraps a boxed Plan trait object so it can be passed around in scripts.
#[derive(Clone)]
pub struct PlanHandle {
    /// The wrapped plan (Arc for Clone)
    pub plan: Arc<std::sync::Mutex<Option<Box<dyn Plan>>>>,
}

impl PlanHandle {
    /// Create a new PlanHandle from a Plan
    pub fn new<P: Plan + 'static>(plan: P) -> Self {
        Self {
            plan: Arc::new(std::sync::Mutex::new(Some(Box::new(plan)))),
        }
    }

    /// Take the plan out (can only be done once)
    pub fn take(&self) -> Option<Box<dyn Plan>> {
        self.plan.lock().ok()?.take()
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

fn validate_points(points: i64) -> Result<usize, Box<EvalAltResult>> {
    const MAX_SCAN_POINTS: i64 = 1_000_000;
    if points <= 0 {
        return Err(Box::new(EvalAltResult::ErrorRuntime(
            format!("points must be positive, got {}", points).into(),
            Position::NONE,
        )));
    }
    if points > MAX_SCAN_POINTS {
        return Err(Box::new(EvalAltResult::ErrorRuntime(
            format!(
                "points exceeds maximum ({}), got {}",
                MAX_SCAN_POINTS, points
            )
            .into(),
            Position::NONE,
        )));
    }
    Ok(points as usize)
}

// =============================================================================
// Plan Registration
// =============================================================================

/// Register all plan bindings with the Rhai engine
///
/// This function registers:
/// - Plan creation functions: `line_scan`, `grid_scan`, `count`
/// - RunEngine methods: `queue`, `start`, `pause`, `resume`, `abort`, `state`
/// - Document types for pattern matching
pub fn register_plans(engine: &mut Engine) {
    // Register custom types
    engine.register_type_with_name::<RunEngineHandle>("RunEngine");
    engine.register_type_with_name::<PlanHandle>("Plan");

    // =========================================================================
    // Plan Creation Functions
    // =========================================================================

    // line_scan(motor, start, end, points, detector)
    engine.register_fn(
        "line_scan",
        |motor: &str,
         start: f64,
         end: f64,
         points: i64,
         detector: &str|
         -> Result<PlanHandle, Box<EvalAltResult>> {
            let valid_points = validate_points(points)?;
            let plan = LineScan::new(motor, start, end, valid_points).with_detector(detector);
            Ok(PlanHandle::new(plan))
        },
    );

    // line_scan with settle time
    engine.register_fn(
        "line_scan_with_settle",
        |motor: &str,
         start: f64,
         end: f64,
         points: i64,
         detector: &str,
         settle: f64|
         -> Result<PlanHandle, Box<EvalAltResult>> {
            let valid_points = validate_points(points)?;
            let plan = LineScan::new(motor, start, end, valid_points)
                .with_detector(detector)
                .with_settle_time(settle);
            Ok(PlanHandle::new(plan))
        },
    );

    // grid_scan(x_motor, x_start, x_end, x_points, y_motor, y_start, y_end, y_points, detector)
    engine.register_fn(
        "grid_scan",
        |x_motor: &str,
         x_start: f64,
         x_end: f64,
         x_points: i64,
         y_motor: &str,
         y_start: f64,
         y_end: f64,
         y_points: i64,
         detector: &str|
         -> Result<PlanHandle, Box<EvalAltResult>> {
            let valid_x_points = validate_points(x_points)?;
            let valid_y_points = validate_points(y_points)?;

            // Note: GridScan takes (outer_axis, ..., inner_axis, ...)
            // x is typically the inner (fast) axis, y is the outer (slow) axis
            let plan = GridScan::new(
                y_motor,
                y_start,
                y_end,
                valid_y_points,
                x_motor,
                x_start,
                x_end,
                valid_x_points,
            )
            .with_detector(detector);
            Ok(PlanHandle::new(plan))
        },
    );

    // count(num_points, detector, delay_seconds)
    engine.register_fn(
        "count",
        |num_points: i64, detector: &str, delay: f64| -> Result<PlanHandle, Box<EvalAltResult>> {
            let valid_points = validate_points(num_points)?;
            let plan = Count::new(valid_points)
                .with_detector(detector)
                .with_delay(delay);
            Ok(PlanHandle::new(plan))
        },
    );

    // count simple (no delay)
    engine.register_fn(
        "count_simple",
        |num_points: i64| -> Result<PlanHandle, Box<EvalAltResult>> {
            let valid_points = validate_points(num_points)?;
            let plan = Count::new(valid_points);
            Ok(PlanHandle::new(plan))
        },
    );

    // =========================================================================
    // Plan Properties
    // =========================================================================

    engine.register_fn("plan_type", |plan: &mut PlanHandle| -> String {
        plan.plan
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|p| p.plan_type().to_string()))
            .unwrap_or_else(|| "unknown".to_string())
    });

    engine.register_fn("plan_name", |plan: &mut PlanHandle| -> String {
        plan.plan
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|p| p.plan_name().to_string()))
            .unwrap_or_else(|| "unknown".to_string())
    });

    engine.register_fn("num_points", |plan: &mut PlanHandle| -> i64 {
        plan.plan
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|p| p.num_points() as i64))
            .unwrap_or(0)
    });

    // =========================================================================
    // RunEngine Methods
    // =========================================================================

    // run_engine.queue(plan)
    engine.register_fn(
        "queue",
        |re: &mut RunEngineHandle, plan: PlanHandle| -> Result<String, Box<EvalAltResult>> {
            let boxed_plan = plan.take().ok_or_else(|| {
                Box::new(EvalAltResult::ErrorRuntime(
                    "Plan already consumed".into(),
                    Position::NONE,
                ))
            })?;

            let run_uid =
                block_in_place(|| Handle::current().block_on(re.engine.queue(boxed_plan)));
            Ok(run_uid)
        },
    );

    // run_engine.queue_with_metadata(plan, metadata_map)
    engine.register_fn(
        "queue_with_metadata",
        |re: &mut RunEngineHandle,
         plan: PlanHandle,
         metadata: crate::rhai::Map|
         -> Result<String, Box<EvalAltResult>> {
            let boxed_plan = plan.take().ok_or_else(|| {
                Box::new(EvalAltResult::ErrorRuntime(
                    "Plan already consumed".into(),
                    Position::NONE,
                ))
            })?;

            // Convert Rhai Map to HashMap<String, String>
            let mut meta: HashMap<String, String> = HashMap::new();
            for (k, v) in metadata.iter() {
                meta.insert(k.to_string(), v.to_string());
            }

            let run_uid = block_in_place(|| {
                Handle::current().block_on(re.engine.queue_with_metadata(boxed_plan, meta))
            });
            Ok(run_uid)
        },
    );

    // run_engine.start()
    engine.register_fn(
        "start",
        |re: &mut RunEngineHandle| -> Result<(), Box<EvalAltResult>> {
            block_in_place(|| Handle::current().block_on(re.engine.start())).map_err(|e| {
                Box::new(EvalAltResult::ErrorRuntime(
                    format!("RunEngine start failed: {}", e).into(),
                    Position::NONE,
                ))
            })
        },
    );

    // run_engine.pause()
    engine.register_fn(
        "pause",
        |re: &mut RunEngineHandle| -> Result<(), Box<EvalAltResult>> {
            block_in_place(|| Handle::current().block_on(re.engine.pause())).map_err(|e| {
                Box::new(EvalAltResult::ErrorRuntime(
                    format!("RunEngine pause failed: {}", e).into(),
                    Position::NONE,
                ))
            })
        },
    );

    // run_engine.resume()
    engine.register_fn(
        "resume",
        |re: &mut RunEngineHandle| -> Result<(), Box<EvalAltResult>> {
            block_in_place(|| Handle::current().block_on(re.engine.resume())).map_err(|e| {
                Box::new(EvalAltResult::ErrorRuntime(
                    format!("RunEngine resume failed: {}", e).into(),
                    Position::NONE,
                ))
            })
        },
    );

    // run_engine.abort(reason)
    engine.register_fn(
        "abort",
        |re: &mut RunEngineHandle, reason: &str| -> Result<(), Box<EvalAltResult>> {
            block_in_place(|| Handle::current().block_on(re.engine.abort(reason))).map_err(|e| {
                Box::new(EvalAltResult::ErrorRuntime(
                    format!("RunEngine abort failed: {}", e).into(),
                    Position::NONE,
                ))
            })
        },
    );

    // run_engine.halt()
    engine.register_fn(
        "halt",
        |re: &mut RunEngineHandle| -> Result<(), Box<EvalAltResult>> {
            block_in_place(|| Handle::current().block_on(re.engine.halt())).map_err(|e| {
                Box::new(EvalAltResult::ErrorRuntime(
                    format!("RunEngine halt failed: {}", e).into(),
                    Position::NONE,
                ))
            })
        },
    );

    // run_engine.state() -> string
    engine.register_fn("state", |re: &mut RunEngineHandle| -> String {
        let state = block_in_place(|| Handle::current().block_on(re.engine.state()));
        state.to_string()
    });

    // run_engine.queue_len() -> int
    engine.register_fn("queue_len", |re: &mut RunEngineHandle| -> i64 {
        block_in_place(|| Handle::current().block_on(re.engine.queue_len())) as i64
    });

    // run_engine.current_run_uid() -> string or ""
    engine.register_fn("current_run_uid", |re: &mut RunEngineHandle| -> String {
        block_in_place(|| Handle::current().block_on(re.engine.current_run_uid()))
            .unwrap_or_default()
    });

    // run_engine.current_progress() -> int
    engine.register_fn("current_progress", |re: &mut RunEngineHandle| -> i64 {
        block_in_place(|| Handle::current().block_on(re.engine.current_progress()))
            .map(|p| p as i64)
            .unwrap_or(-1)
    });
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rhai_engine::RhaiEngine;
    use crate::traits::ScriptEngine;
    use daq_hardware::registry::DeviceRegistry;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_plan_bindings_registered() {
        // Verify that plan creation functions are available after with_hardware()
        let mut engine = RhaiEngine::with_hardware().unwrap();

        // This should not panic - plan functions should be registered
        let script = r#"
            let plan = line_scan("stage_x", 0.0, 10.0, 11, "detector");
        "#;

        let result = engine.execute_script(script).await;
        assert!(result.is_ok(), "Plan creation should work: {:?}", result);
    }

    #[tokio::test]
    async fn test_run_engine_injection() {
        // Verify that run_engine can be injected and used
        let mut engine = RhaiEngine::with_hardware().unwrap();

        // Create and inject run_engine
        let registry = Arc::new(DeviceRegistry::new());
        let run_engine = RunEngineHandle::new(registry);
        engine
            .set_run_engine(run_engine)
            .expect("Failed to set run_engine");

        // This should not panic - run_engine should be accessible
        let script = r#"
            let plan = count_simple(5);
            run_engine.queue(plan);
        "#;

        let result = engine.execute_script(script).await;
        assert!(
            result.is_ok(),
            "RunEngine operations should work: {:?}",
            result
        );
    }

    #[test]
    fn test_line_scan_creation() {
        let mut engine = Engine::new();
        register_plans(&mut engine);

        let result: PlanHandle = engine
            .eval(r#"line_scan("stage_x", 0.0, 10.0, 11, "power_meter")"#)
            .expect("Failed to create line scan");

        let guard = result.plan.lock().unwrap();
        let plan = guard.as_ref().unwrap();
        assert_eq!(plan.plan_type(), "line_scan");
        assert_eq!(plan.num_points(), 11);
    }

    #[test]
    fn test_grid_scan_creation() {
        let mut engine = Engine::new();
        register_plans(&mut engine);

        let result: PlanHandle = engine
            .eval(r#"grid_scan("x", 0.0, 10.0, 5, "y", 0.0, 5.0, 3, "det")"#)
            .expect("Failed to create grid scan");

        let guard = result.plan.lock().unwrap();
        let plan = guard.as_ref().unwrap();
        assert_eq!(plan.plan_type(), "grid_scan");
        assert_eq!(plan.num_points(), 15); // 5 * 3
    }

    #[test]
    fn test_count_creation() {
        let mut engine = Engine::new();
        register_plans(&mut engine);

        let result: PlanHandle = engine
            .eval(r#"count(10, "detector", 0.5)"#)
            .expect("Failed to create count");

        let guard = result.plan.lock().unwrap();
        let plan = guard.as_ref().unwrap();
        assert_eq!(plan.plan_type(), "count");
        assert_eq!(plan.num_points(), 10);
    }

    #[test]
    fn test_plan_properties() {
        let mut engine = Engine::new();
        register_plans(&mut engine);

        let num_points: i64 = engine
            .eval(
                r#"
                let scan = line_scan("motor", 0.0, 10.0, 21, "det");
                num_points(scan)
            "#,
            )
            .expect("Failed to get num_points");

        assert_eq!(num_points, 21);
    }

    #[test]
    fn test_validate_points_negative() {
        let mut engine = Engine::new();
        register_plans(&mut engine);

        let result: Result<PlanHandle, _> = engine.eval(r#"line_scan("x", 0.0, 10.0, -5, "det")"#);
        if let Err(e) = result {
            assert!(e.to_string().contains("points must be positive"));
        } else {
            panic!("Expected error for negative points, got Ok");
        }
    }

    #[test]
    fn test_validate_points_zero() {
        let mut engine = Engine::new();
        register_plans(&mut engine);

        let result: Result<PlanHandle, _> = engine.eval(r#"line_scan("x", 0.0, 10.0, 0, "det")"#);
        if let Err(e) = result {
            assert!(e.to_string().contains("points must be positive"));
        } else {
            panic!("Expected error for zero points, got Ok");
        }
    }

    #[test]
    fn test_validate_points_too_large() {
        let mut engine = Engine::new();
        register_plans(&mut engine);

        let result: Result<PlanHandle, _> =
            engine.eval(r#"line_scan("x", 0.0, 10.0, 1000001, "det")"#);
        if let Err(e) = result {
            assert!(e.to_string().contains("points exceeds maximum"));
        } else {
            panic!("Expected error for too many points, got Ok");
        }
    }
}
