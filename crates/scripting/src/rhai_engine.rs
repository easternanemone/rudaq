//! Rhai ScriptEngine Implementation
//!
//! This module provides the Rhai scripting backend, implementing the `ScriptEngine`
//! trait. Rhai is an embedded scripting language written in pure Rust with zero
//! external dependencies.
//!
//! # Features
//!
//! - Pure Rust embedded scripting (no external interpreter required)
//! - Fast compilation and execution
//! - Type-safe value passing between Rust and Rhai
//! - Async-compatible execution model
//! - Safety limits to prevent infinite loops
//! - Hardware bindings for controlling stages and cameras
//!
//! # Advantages of Rhai
//!
//! - **Zero dependencies**: No external Python/Lua installation required
//! - **Fast startup**: No interpreter initialization overhead
//! - **Type safety**: Strong integration with Rust type system
//! - **Sandboxed**: Cannot access filesystem or network by default
//! - **Small footprint**: Entire engine is ~200KB compiled
//!
//! # Example: Basic Scripting
//!
//! ```rust,ignore
//! use daq_scripting::{ScriptEngine, RhaiEngine, ScriptValue};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut engine = RhaiEngine::new()?;
//!
//!     // Set global variables
//!     engine.set_global("wavelength", ScriptValue::new(800_i64))?;
//!
//!     // Execute Rhai script
//!     let script = r#"
//!         print(`Setting wavelength to ${wavelength} nm`);
//!         let result = wavelength * 2;
//!         result  // Return value
//!     "#;
//!
//!     let result = engine.execute_script(script).await?;
//!     let value: i64 = result.downcast().unwrap();
//!     println!("Result: {}", value);
//!
//!     Ok(())
//! }
//! ```
//!
//! # Example: Hardware Control
//!
//! ```rust,ignore
//! use daq_scripting::{ScriptEngine, RhaiEngine, ScriptValue, StageHandle};
//! use driver_mock::mock_stage::MockStage;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Use with_hardware() to enable stage/camera control
//!     let mut engine = RhaiEngine::with_hardware()?;
//!
//!     // Create hardware and register as global
//!     engine.set_global("stage", ScriptValue::new(StageHandle {
//!         driver: Arc::new(MockStage::new()),
//!     }))?;
//!
//!     // Execute script that controls hardware
//!     let script = r#"
//!         print("Moving to 10mm...");
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
//! # Function Registration Limitation
//!
//! Rhai requires compile-time type information for function registration via
//! `Engine::register_fn()`. The generic `ScriptEngine::register_function()`
//! interface cannot support this.
//!
//! **Solutions:**
//! 1. Use `RhaiEngine::with_hardware()` for stage/camera bindings
//! 2. Create custom constructors that register functions before Arc::new()
//! 3. Use PyO3Engine for runtime function registration
//!
//! See [`RhaiEngine::register_function`] documentation for details.

use async_trait::async_trait;
use rhai::{Dynamic, Engine, EvalAltResult, Scope};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::traits::{ScriptEngine, ScriptError, ScriptValue};

/// Rhai-based scripting engine
///
/// This engine wraps the Rhai interpreter and provides the `ScriptEngine`
/// interface for executing Rhai scripts. It maintains a global scope for
/// variables that persists across script executions.
///
/// # Thread Safety
///
/// The engine uses Arc<Mutex<Scope>> internally to allow cloning and sharing
/// across threads while maintaining exclusive access during operations.
///
/// # Safety Limits
///
/// The engine enforces a maximum of 10,000 operations per script execution
/// to prevent infinite loops from hanging the application.
/// RAII guard that resets the deadline atomic to 0 on drop, ensuring
/// the deadline is cleared even if `execute_script` panics.
struct DeadlineGuard(Arc<AtomicU64>);

impl Drop for DeadlineGuard {
    fn drop(&mut self) {
        self.0.store(0, Ordering::Relaxed);
    }
}

pub struct RhaiEngine {
    /// Rhai script engine
    engine: Arc<Engine>,
    /// Global scope for variables (shared across executions)
    scope: Arc<Mutex<Scope<'static>>>,
    /// Monotonic baseline for deadline computation (millis offset from this instant)
    baseline: Arc<Instant>,
    /// Execution deadline (millis elapsed from baseline), 0 = no deadline
    deadline_offset_ms: Arc<AtomicU64>,
}

impl RhaiEngine {
    /// Create a new RhaiEngine instance with safety limits
    ///
    /// This initializes the Rhai engine with a progress callback that limits
    /// script execution to 10,000 operations.
    ///
    /// # Errors
    ///
    /// Always succeeds for Rhai (returns Ok). Signature matches trait requirements.
    pub fn new() -> Result<Self, ScriptError> {
        Self::with_limit(10_000)
    }

    /// Create a new RhaiEngine with a specific operations limit.
    pub fn with_limit(max_operations: u64) -> Result<Self, ScriptError> {
        let mut engine = Engine::new();
        let baseline = Arc::new(Instant::now());
        let deadline_offset_ms = Arc::new(AtomicU64::new(0));
        let deadline_clone = deadline_offset_ms.clone();
        let baseline_clone = baseline.clone();

        // Safety: Limit operations to prevent infinite loops
        engine.on_progress(move |count| {
            if count > max_operations {
                return Some(
                    format!(
                        "Safety limit exceeded: maximum {} operations",
                        max_operations
                    )
                    .into(),
                );
            }

            // Check timeout every 1024 ops to avoid syscall overhead
            if count.trailing_zeros() >= 10 {
                let dl = deadline_clone.load(Ordering::Relaxed);
                if dl > 0 {
                    #[allow(clippy::cast_possible_truncation)]
                    // SAFETY: elapsed millis for script timeout will not exceed u64 range
                    let now = baseline_clone.elapsed().as_millis() as u64;
                    if now > dl {
                        return Some("Script execution timed out".into());
                    }
                }
            }

            None
        });

        Ok(Self {
            engine: Arc::new(engine),
            scope: Arc::new(Mutex::new(Scope::new())),
            baseline,
            deadline_offset_ms,
        })
    }

    /// Create a new RhaiEngine with yield-based plan scripting support (bd-94zq.4)
    ///
    /// This constructor registers:
    /// - Hardware bindings (stage, camera)
    /// - Plan bindings (line_scan, grid_scan, etc.)
    /// - Yield bindings (yield_plan, yield_move, etc.)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut engine = RhaiEngine::with_yield_support()?;
    ///
    /// // Set up yield channels
    /// let (handle, rx, tx) = YieldChannelBuilder::new().build();
    /// engine.set_yield_handle(handle)?;
    ///
    /// // Scripts can use yield-based syntax:
    /// let script = r#"
    ///     let result = yield_plan(line_scan("x", 0, 10, 11, "det"));
    ///     if result.data["det"] > threshold {
    ///         yield_plan(high_res_scan(...));
    ///     }
    /// "#;
    /// ```
    pub fn with_yield_support() -> Result<Self, ScriptError> {
        let mut engine = Engine::new();
        let baseline = Arc::new(Instant::now());
        let deadline_offset_ms = Arc::new(AtomicU64::new(0));
        let deadline_clone = deadline_offset_ms.clone();
        let baseline_clone = baseline.clone();

        // Safety: Limit operations to prevent infinite loops
        engine.on_progress(move |count| {
            if count > 100_000 {
                // Higher limit for yield scripts
                return Some("Safety limit exceeded: maximum 100,000 operations".into());
            }

            // Check timeout every 1024 ops to avoid syscall overhead
            if count.trailing_zeros() >= 10 {
                let dl = deadline_clone.load(Ordering::Relaxed);
                if dl > 0 {
                    #[allow(clippy::cast_possible_truncation)]
                    // SAFETY: elapsed millis for script timeout will not exceed u64 range
                    let now = baseline_clone.elapsed().as_millis() as u64;
                    if now > dl {
                        return Some("Script execution timed out".into());
                    }
                }
            }

            None
        });

        // Register hardware bindings
        crate::bindings::register_hardware(&mut engine);

        // Register LIBS hardware bindings (gated camera, spectrograph, TOP stage)
        crate::libs_bindings::register_libs_hardware(&mut engine);

        // Register plan bindings
        crate::plan_bindings::register_plans(&mut engine);

        // Register yield bindings (bd-94zq.4)
        crate::yield_bindings::register_yield_bindings(&mut engine);

        // Register coordinator bindings (bd-u32r)
        crate::coordinator_bindings::register_coordinator(&mut engine);

        Ok(Self {
            engine: Arc::new(engine),
            scope: Arc::new(Mutex::new(Scope::new())),
            baseline,
            deadline_offset_ms,
        })
    }

    /// Create a new RhaiEngine with hardware bindings registered
    ///
    /// This constructor calls [`crate::scripting::bindings::register_hardware`]
    /// to enable script access to Stage and Camera handles. Use this when
    /// executing scripts that need to control hardware devices.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use daq_scripting::{RhaiEngine, ScriptEngine, ScriptValue, StageHandle};
    /// use driver_mock::mock_stage::MockStage;
    /// use std::sync::Arc;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let mut engine = RhaiEngine::with_hardware()?;
    ///
    ///     // Set hardware handle as global variable
    ///     engine.set_global("stage", ScriptValue::new(StageHandle {
    ///         driver: Arc::new(MockStage::new()),
    ///     }))?;
    ///
    ///     // Execute script that controls hardware
    ///     engine.execute_script("stage.move_abs(10.0);").await?;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Hardware Methods Available in Scripts
    ///
    /// **Stage methods:**
    /// - `stage.move_abs(pos)` - Move to absolute position
    /// - `stage.move_rel(dist)` - Move relative distance
    /// - `stage.position()` - Get current position
    /// - `stage.wait_settled()` - Wait for motion to complete
    ///
    /// **Camera methods:**
    /// - `camera.arm()` - Prepare camera for trigger
    /// - `camera.trigger()` - Capture frame
    /// - `camera.resolution()` - Get [width, height] array
    ///
    /// **Utility functions:**
    /// - `sleep(seconds)` - Sleep for specified seconds
    ///
    /// # Errors
    ///
    /// Always succeeds for Rhai (returns Ok). Signature matches trait requirements.
    pub fn with_hardware() -> Result<Self, ScriptError> {
        Self::with_hardware_and_limit(10_000)
    }

    /// Create a RhaiEngine with hardware bindings and a custom operations limit.
    ///
    /// Use this for long-running scripts that exceed the default 10,000 operation limit.
    /// For example, the polarization characterization experiment needs ~1,000,000 operations.
    ///
    /// # Arguments
    ///
    /// * `max_operations` - Maximum number of operations before script termination
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // For long experiments, use a higher limit
    /// let mut engine = RhaiEngine::with_hardware_and_limit(1_000_000)?;
    /// ```
    pub fn with_hardware_and_limit(max_operations: u64) -> Result<Self, ScriptError> {
        let mut engine = Engine::new();
        let baseline = Arc::new(Instant::now());
        let deadline_offset_ms = Arc::new(AtomicU64::new(0));
        let deadline_clone = deadline_offset_ms.clone();
        let baseline_clone = baseline.clone();

        // Safety: Limit operations to prevent infinite loops
        engine.on_progress(move |count| {
            if count > max_operations {
                return Some(
                    format!(
                        "Safety limit exceeded: maximum {} operations",
                        max_operations
                    )
                    .into(),
                );
            }

            // Check timeout every 1024 ops to avoid syscall overhead
            if count.trailing_zeros() >= 10 {
                let dl = deadline_clone.load(Ordering::Relaxed);
                if dl > 0 {
                    #[allow(clippy::cast_possible_truncation)]
                    // SAFETY: elapsed millis for script timeout will not exceed u64 range
                    let now = baseline_clone.elapsed().as_millis() as u64;
                    if now > dl {
                        return Some("Script execution timed out".into());
                    }
                }
            }

            None
        });

        // Register hardware bindings
        crate::bindings::register_hardware(&mut engine);

        // Register LIBS hardware bindings (gated camera, spectrograph, TOP stage)
        crate::libs_bindings::register_libs_hardware(&mut engine);

        // Register plan bindings (bd-w14j.1)
        crate::plan_bindings::register_plans(&mut engine);

        // Register coordinator bindings (bd-u32r)
        crate::coordinator_bindings::register_coordinator(&mut engine);

        Ok(Self {
            engine: Arc::new(engine),
            scope: Arc::new(Mutex::new(Scope::new())),
            baseline,
            deadline_offset_ms,
        })
    }

    /// Inject a RunEngine handle into the script scope as a global variable.
    ///
    /// This enables scripts to queue and execute plans using the RunEngine.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut engine = RhaiEngine::with_hardware()?;
    /// let registry = Arc::new(RwLock::new(DeviceRegistry::new()));
    /// let run_engine = RunEngineHandle::new(registry);
    /// engine.set_run_engine(run_engine);
    ///
    /// // Now scripts can use: run_engine.queue(plan); run_engine.start();
    /// ```
    pub fn set_run_engine(
        &mut self,
        handle: crate::plan_bindings::RunEngineHandle,
    ) -> Result<(), ScriptError> {
        self.set_global("run_engine", ScriptValue::new(handle))
    }

    /// Set a YieldHandle for yield-based plan scripting (bd-94zq.4)
    ///
    /// This enables scripts to use `yield_plan()` and related functions.
    /// The handle is set as `__yield_handle` in the global scope.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let (handle, rx, tx) = YieldChannelBuilder::new().build();
    /// engine.set_yield_handle(handle)?;
    ///
    /// // Scripts can now use:
    /// // let result = yield_plan(line_scan(...));
    /// // yield_move("stage", 10.0);
    /// ```
    pub fn set_yield_handle(
        &mut self,
        handle: Arc<crate::yield_handle::YieldHandle>,
    ) -> Result<(), ScriptError> {
        // Store in scope as Dynamic
        let mut scope = self.scope.lock().unwrap_or_else(|p| p.into_inner());
        scope.push("__yield_handle", handle);
        Ok(())
    }

    /// Execute a script with a timeout (wall clock time).
    ///
    /// Uses a monotonic `Instant` baseline to avoid NTP clock jumps.
    /// A `DeadlineGuard` ensures the deadline is reset even if `execute_script` panics.
    /// The `on_progress` callback checks the deadline every 1024 operations.
    pub async fn execute_script_with_timeout(
        &mut self,
        script: &str,
        timeout: Duration,
    ) -> Result<ScriptValue, ScriptError> {
        #[allow(clippy::cast_possible_truncation)]
        // SAFETY: timeout deadline in millis will not exceed u64 range
        let deadline_ms = (self.baseline.elapsed() + timeout).as_millis() as u64;

        self.deadline_offset_ms
            .store(deadline_ms, Ordering::Relaxed);
        let _guard = DeadlineGuard(self.deadline_offset_ms.clone());

        self.execute_script(script).await
    }

    /// Synchronous evaluation (for use in spawn_blocking)
    ///
    /// This is used by ScriptPlanRunner to execute scripts in a blocking context.
    pub fn eval<T>(&mut self, script: &str) -> Result<T, ScriptError>
    where
        T: Clone + Send + Sync + 'static,
    {
        let mut scope_guard = self.scope.lock().unwrap_or_else(|p| p.into_inner());
        self.engine
            .eval_with_scope::<T>(&mut scope_guard, script)
            .map_err(Self::convert_rhai_error)
    }

    /// Convert a Rhai Dynamic to ScriptValue
    ///
    /// Primitives (i64, f64, bool, String, unit) are extracted directly for
    /// zero-overhead hot-path conversion. All other types (arrays, maps, nested
    /// structures) are round-tripped through `rhai::serde` into `serde_json::Value`
    /// so that callers get a well-known, inspectable representation instead of an
    /// opaque `Dynamic` wrapper.
    fn dynamic_to_script_value(value: Dynamic) -> ScriptValue {
        // Fast path: extract common primitives directly
        if value.is::<i64>() {
            ScriptValue::new(value.cast::<i64>())
        } else if value.is::<f64>() {
            ScriptValue::new(value.cast::<f64>())
        } else if value.is::<bool>() {
            ScriptValue::new(value.cast::<bool>())
        } else if value.is::<String>() {
            ScriptValue::new(value.cast::<String>())
        } else if value.is::<()>() {
            ScriptValue::new(())
        } else {
            // Fallback: use rhai::serde to convert maps, arrays, and other
            // composite types into serde_json::Value for a portable representation
            match rhai::serde::from_dynamic::<serde_json::Value>(&value) {
                Ok(json_val) => ScriptValue::new(json_val),
                Err(_) => {
                    // Last resort: wrap the raw Dynamic (e.g. opaque custom types
                    // returned by Rhai that are not serde-compatible)
                    ScriptValue::new(value)
                }
            }
        }
    }

    /// Convert a ScriptValue to Rhai Dynamic
    ///
    /// Primitives are converted directly (hot path). Opaque hardware handles
    /// (containing `Arc<dyn Trait>`) are handled explicitly because they cannot
    /// be serialized. All other serde-compatible types (e.g. `HashMap`, `Vec`,
    /// `serde_json::Value`) go through `rhai::serde::to_dynamic` as an
    /// extensible fallback.
    fn script_value_to_dynamic(value: ScriptValue) -> Result<Dynamic, ScriptError> {
        use crate::bindings::{CameraHandle, StageHandle};
        use crate::plan_bindings::RunEngineHandle;

        // ── Fast path: primitives ──────────────────────────────────────────
        if let Some(i) = value.downcast_ref::<i64>() {
            return Ok(Dynamic::from(*i));
        }
        if let Some(f) = value.downcast_ref::<f64>() {
            return Ok(Dynamic::from(*f));
        }
        if let Some(b) = value.downcast_ref::<bool>() {
            return Ok(Dynamic::from(*b));
        }
        if let Some(s) = value.downcast_ref::<String>() {
            return Ok(Dynamic::from(s.clone()));
        }
        if let Some(s) = value.downcast_ref::<&str>() {
            return Ok(Dynamic::from(*s));
        }
        if value.downcast_ref::<()>().is_some() {
            return Ok(Dynamic::UNIT);
        }

        // ── Opaque hardware handles (not serde-compatible) ─────────────────
        if let Some(stage) = value.downcast_ref::<StageHandle>() {
            return Ok(Dynamic::from(stage.clone()));
        }
        if let Some(camera) = value.downcast_ref::<CameraHandle>() {
            return Ok(Dynamic::from(camera.clone()));
        }
        if let Some(run_engine) = value.downcast_ref::<RunEngineHandle>() {
            return Ok(Dynamic::from(run_engine.clone()));
        }

        // ── Serde fallback: serde_json::Value → Dynamic ────────────────────
        // Handles values stored as serde_json::Value by dynamic_to_script_value(),
        // as well as any other serde-compatible types wrapped in ScriptValue.
        if let Some(json_val) = value.downcast_ref::<serde_json::Value>() {
            return rhai::serde::to_dynamic(json_val).map_err(|e| {
                ScriptError::TypeConversionError {
                    expected: "serde-compatible type".to_string(),
                    found: format!("serde_json::Value conversion failed: {e}"),
                }
            });
        }

        // ── Already a Dynamic (round-trip from dynamic_to_script_value) ────
        // downcast() consumes ownership; on failure Err returns the ScriptValue
        match value.downcast::<Dynamic>() {
            Ok(dyn_val) => Ok(dyn_val),
            Err(_) => Err(ScriptError::TypeConversionError {
                expected: "i64, f64, bool, String, hardware handle, serde_json::Value, or Dynamic"
                    .to_string(),
                found: "unknown type".to_string(),
            }),
        }
    }

    /// Convert Rhai error to ScriptError
    #[allow(clippy::boxed_local)] // Box is required as this is how Rhai returns errors
    fn convert_rhai_error(err: Box<EvalAltResult>) -> ScriptError {
        match *err {
            EvalAltResult::ErrorParsing(parse_error, pos) => ScriptError::SyntaxError {
                message: format!("{} at position {}", parse_error, pos),
            },
            EvalAltResult::ErrorRuntime(msg, pos) => ScriptError::RuntimeError {
                message: format!("{} at position {}", msg, pos),
                backtrace: None,
            },
            EvalAltResult::ErrorVariableNotFound(name, pos) => ScriptError::VariableNotFound {
                name: format!("{} at position {}", name, pos),
            },
            other => ScriptError::RuntimeError {
                message: other.to_string(),
                backtrace: None,
            },
        }
    }
}

#[async_trait]
impl ScriptEngine for RhaiEngine {
    async fn execute_script(&mut self, script: &str) -> Result<ScriptValue, ScriptError> {
        let engine = self.engine.clone();
        let scope = self.scope.clone();
        let script = script.to_string();

        // Execute in a blocking task to avoid blocking the async runtime
        tokio::task::spawn_blocking(move || {
            let mut scope_guard = scope.lock().unwrap_or_else(|p| p.into_inner());
            let result = engine
                .eval_with_scope::<Dynamic>(&mut scope_guard, &script)
                .map_err(Self::convert_rhai_error)?;

            Ok(Self::dynamic_to_script_value(result))
        })
        .await
        .map_err(|e| ScriptError::AsyncError {
            message: format!("Task join error: {}", e),
        })?
    }

    async fn validate_script(&self, script: &str) -> Result<(), ScriptError> {
        let engine = self.engine.clone();
        let script = script.to_string();

        tokio::task::spawn_blocking(move || {
            engine
                .compile(&script)
                .map_err(|parse_error| ScriptError::SyntaxError {
                    message: format!("Parse error: {}", parse_error),
                })?;
            Ok(())
        })
        .await
        .map_err(|e| ScriptError::AsyncError {
            message: format!("Task join error: {}", e),
        })?
    }

    fn register_function(
        &mut self,
        name: &str,
        _function: Box<dyn std::any::Any + Send + Sync>,
    ) -> Result<(), ScriptError> {
        // For Rhai, function registration is more complex because Rhai functions
        // need to be registered with the engine's type system at compile time
        // using generics. This would require generic closures or macros like
        // Engine::register_fn!.
        //
        // The generic register_function() interface cannot support Rhai's
        // compile-time type-safe registration without macros.
        //
        // WORKAROUND: Use RhaiEngine::with_hardware() constructor which
        // pre-registers all hardware control functions (move_abs, trigger, etc.).
        //
        // For custom functions, you must create a custom RhaiEngine constructor
        // that calls Engine::register_fn() before wrapping in Arc.
        Err(ScriptError::FunctionRegistrationError {
            message: format!(
                "Cannot register Rhai function '{}' via generic interface. \n\
                 \n\
                 Rhai requires compile-time type information for function registration.\n\
                 \n\
                 Solutions:\n\
                 1. Use RhaiEngine::with_hardware() for hardware bindings (stage, camera)\n\
                 2. Create a custom constructor that calls Engine::register_fn() before Arc::new()\n\
                 3. For Python or other backends, use PyO3Engine which supports runtime registration\n\
                 \n\
                 Example custom constructor:\n\
                 ```rust\n\
                 pub fn with_custom_functions() -> Result<Self, ScriptError> {{\n\
                     let mut engine = Engine::new();\n\
                     engine.register_fn(\"my_function\", |x: i64| x * 2);\n\
                     Ok(Self {{ engine: Arc::new(engine), scope: Arc::new(Mutex::new(Scope::new())) }})\n\
                 }}\n\
                 ```",
                name
            ),
        })
    }

    fn set_global(&mut self, name: &str, value: ScriptValue) -> Result<(), ScriptError> {
        let dynamic = Self::script_value_to_dynamic(value)?;
        let mut scope = self.scope.lock().unwrap_or_else(|p| p.into_inner());

        // Check if variable exists, if so update it, otherwise push new
        if scope.get_value::<Dynamic>(name).is_some() {
            // Variable exists, remove and re-add
            let _ = scope.remove::<Dynamic>(name);
        }

        scope.push(name, dynamic);
        Ok(())
    }

    fn get_global(&self, name: &str) -> Result<ScriptValue, ScriptError> {
        let scope = self.scope.lock().unwrap_or_else(|p| p.into_inner());

        scope
            .get_value::<Dynamic>(name)
            .map(Self::dynamic_to_script_value)
            .ok_or_else(|| ScriptError::VariableNotFound {
                name: name.to_string(),
            })
    }

    fn clear_globals(&mut self) {
        let mut scope = self.scope.lock().unwrap_or_else(|p| p.into_inner());
        *scope = Scope::new();
    }

    fn backend_name(&self) -> &str {
        "Rhai"
    }
}

// Implement Default for convenience
impl Default for RhaiEngine {
    fn default() -> Self {
        Self::new().expect("RhaiEngine::new with default config should not fail")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rhai_engine_creation() {
        let engine = RhaiEngine::new();
        assert!(engine.is_ok());
    }

    #[tokio::test]
    async fn test_rhai_execute_simple_script() {
        let mut engine = RhaiEngine::new().unwrap();

        let script = r"
            let x = 10;
            let y = 20;
            x + y
        ";

        let result = engine.execute_script(script).await.unwrap();
        let value: i64 = result.downcast().unwrap();
        assert_eq!(value, 30);
    }

    #[tokio::test]
    async fn test_rhai_global_variables() {
        let mut engine = RhaiEngine::new().unwrap();

        engine
            .set_global("test_var", ScriptValue::new(42_i64))
            .unwrap();

        let script = "test_var * 2";
        let result = engine.execute_script(script).await.unwrap();
        let value: i64 = result.downcast().unwrap();
        assert_eq!(value, 84);
    }

    #[tokio::test]
    async fn test_rhai_get_global() {
        let mut engine = RhaiEngine::new().unwrap();

        engine.set_global("foo", ScriptValue::new(123_i64)).unwrap();

        let value = engine.get_global("foo").unwrap();
        let number: i64 = value.downcast().unwrap();
        assert_eq!(number, 123);
    }

    #[tokio::test]
    async fn test_rhai_backend_name() {
        let engine = RhaiEngine::new().unwrap();
        assert_eq!(engine.backend_name(), "Rhai");
    }

    #[tokio::test]
    async fn test_rhai_clear_globals() {
        let mut engine = RhaiEngine::new().unwrap();

        engine
            .set_global("test", ScriptValue::new(123_i64))
            .unwrap();
        assert!(engine.get_global("test").is_ok());

        engine.clear_globals();
        assert!(engine.get_global("test").is_err());
    }

    #[tokio::test]
    async fn test_rhai_validate_script() {
        let engine = RhaiEngine::new().unwrap();

        // Valid script
        assert!(engine.validate_script("let x = 1 + 2;").await.is_ok());

        // Invalid script
        assert!(engine.validate_script("let x = 1 +").await.is_err());
    }

    #[tokio::test]
    async fn test_rhai_safety_limit() {
        let mut engine = RhaiEngine::new().unwrap();

        // Script with many operations should be stopped by safety limit (10,000 operations)
        let script = r"
            let x = 0;
            for i in 0..20000 {
                x = x + 1;
            }
            x
        ";

        let result = engine.execute_script(script).await;
        assert!(
            result.is_err(),
            "Expected safety limit error, got: {:?}",
            result
        );

        if let Err(e) = result {
            let err_msg = e.to_string();
            assert!(
                err_msg.contains("Safety limit")
                    || err_msg.contains("10000")
                    || err_msg.contains("10,000")
                    || err_msg.contains("progress")
                    || err_msg.contains("terminated"), // Rhai's safety callback message
                "Error message should mention safety limit, progress, or terminated, got: {}",
                err_msg
            );
        }
    }

    #[tokio::test]
    async fn test_rhai_timeout() {
        // Infinite loop — only the timeout can stop it.
        let script = r"
            let x = 0;
            loop { x += 1; }
            x
        ";

        // Large op limit so timeout fires first, not the safety limit.
        let mut engine = RhaiEngine::with_limit(u64::MAX).unwrap();

        let result = engine
            .execute_script_with_timeout(script, Duration::from_millis(100))
            .await;

        assert!(result.is_err(), "Expected timeout error");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("timed out") || err_msg.contains("terminated"),
            "Error should mention timeout: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_rhai_variable_persistence() {
        let mut engine = RhaiEngine::new().unwrap();

        // First script sets a variable
        engine.execute_script("let counter = 10;").await.unwrap();

        // Second script should see the variable
        let result = engine.execute_script("counter + 5").await.unwrap();
        let value: i64 = result.downcast().unwrap();
        assert_eq!(value, 15);
    }

    #[test]
    fn test_register_function_error_is_informative() {
        use crate::traits::ScriptEngine;

        let mut engine = RhaiEngine::new().unwrap();
        let dummy_fn = Box::new(|| 42);

        let result = engine.register_function("my_function", dummy_fn);
        assert!(result.is_err());

        let err = result.unwrap_err();
        let err_msg = err.to_string();

        // Verify error message is helpful
        assert!(
            err_msg.contains("with_hardware"),
            "Error should mention with_hardware() constructor"
        );
        assert!(
            err_msg.contains("register_fn"),
            "Error should mention Engine::register_fn()"
        );
        assert!(
            err_msg.contains("compile-time"),
            "Error should explain compile-time limitation"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_with_hardware_constructor() {
        use crate::bindings::{SoftLimits, StageHandle};
        use crate::traits::{ScriptEngine, ScriptValue};
        use driver_mock::MockStage;

        let mut engine = RhaiEngine::with_hardware().unwrap();

        // Register hardware
        engine
            .set_global(
                "stage",
                ScriptValue::new(StageHandle {
                    driver: Arc::new(MockStage::new()),
                    data_tx: None,
                    soft_limits: SoftLimits::unlimited(),
                }),
            )
            .unwrap();

        // Test that hardware methods are available
        let script = r"
            stage.move_abs(5.0);
            let pos = stage.position();
            pos
        ";

        let result = engine.execute_script(script).await.unwrap();
        let value: f64 = result.downcast().unwrap();
        assert_eq!(value, 5.0);
    }
}
