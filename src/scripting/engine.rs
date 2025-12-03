//! Legacy Rhai script host for V4 compatibility.
//!
//! Wraps a Rhai engine with a tokio runtime handle to enable async
//! script execution. This implementation predates the V5 `ScriptEngine`
//! trait and is maintained for backward compatibility.
//!
//! # Migration Path
//!
//! Replace with [`crate::scripting::RhaiEngine`] for new code:
//!
//! ```rust,ignore
//! // Old V4 code:
//! let host = ScriptHost::new(Handle::current());
//! host.run_script("...")?;
//!
//! // New V5 code:
//! let mut engine = RhaiEngine::new()?;
//! engine.execute_script("...").await?;
//! ```
//!
//! # Safety Limits
//!
//! This host enforces a 10,000 operation limit to prevent infinite loops.
//!
//! # Example
//!
//! ```rust,ignore
//! use rust_daq::scripting::ScriptHost;
//! use tokio::runtime::Handle;
//!
//! let host = ScriptHost::new(Handle::current());
//! let result = host.run_script("let x = 10; x * 2")?;
//! ```

use rhai::{Dynamic, Engine, EvalAltResult, Scope};
use tokio::runtime::Handle;

use crate::scripting::bindings;

/// Legacy Rhai script host for V4 compatibility.
///
/// Wraps a Rhai engine with a tokio runtime handle to enable async
/// script execution. This implementation predates the V5 `ScriptEngine`
/// trait and is maintained for backward compatibility.
///
/// # Migration Path
///
/// Replace with [`crate::scripting::RhaiEngine`] for new code:
///
/// ```rust,ignore
/// // Old V4 code:
/// let host = ScriptHost::new(Handle::current());
/// host.run_script("...")?;
///
/// // New V5 code:
/// let mut engine = RhaiEngine::new()?;
/// engine.execute_script("...").await?;
/// ```
///
/// # Safety Limits
///
/// This host enforces a 10,000 operation limit to prevent infinite loops.
pub struct ScriptHost {
    /// Rhai engine instance
    engine: Engine,
    #[expect(
        dead_code,
        reason = "Runtime handle kept alive to ensure tokio context"
    )]
    /// Tokio runtime handle (kept alive for async context)
    runtime: Handle,
}

impl ScriptHost {
    /// Create a new ScriptHost with the default safety limit (10,000 operations).
    ///
    /// # Arguments
    ///
    /// * `runtime` - Tokio runtime handle for async operations
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use rust_daq::scripting::ScriptHost;
    /// use tokio::runtime::Handle;
    ///
    /// let host = ScriptHost::new(Handle::current());
    /// ```
    pub fn new(runtime: Handle) -> Self {
        let mut engine = Engine::new();

        // Safety: Limit operations to prevent infinite loops
        engine.on_progress(|count| {
            if count > 10000 {
                Some("Safety limit exceeded: maximum 10000 operations".into())
            } else {
                None
            }
        });

        Self { engine, runtime }
    }

    /// Create ScriptHost with hardware bindings registered.
    ///
    /// This enables scripts to control hardware devices through
    /// StageHandle and CameraHandle types.
    ///
    /// # Arguments
    ///
    /// * `runtime` - Tokio runtime handle for async operations
    ///
    /// # Hardware Methods Available
    ///
    /// **Stage:**
    /// - `stage.move_abs(pos)` - Move to absolute position
    /// - `stage.move_rel(dist)` - Move relative distance
    /// - `stage.position()` - Get current position
    /// - `stage.wait_settled()` - Wait for motion to complete
    ///
    /// **Camera:**
    /// - `camera.arm()` - Prepare for trigger
    /// - `camera.trigger()` - Capture frame
    /// - `camera.resolution()` - Get [width, height]
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use rust_daq::scripting::ScriptHost;
    /// use tokio::runtime::Handle;
    ///
    /// let host = ScriptHost::with_hardware(Handle::current());
    /// // Scripts can now use stage.move_abs(), camera.trigger(), etc.
    /// ```
    pub fn with_hardware(runtime: Handle) -> Self {
        let mut engine = Engine::new();

        // Safety limit
        engine.on_progress(|count| {
            if count > 10000 {
                Some("Safety limit exceeded: maximum 10000 operations".into())
            } else {
                None
            }
        });

        // Register hardware bindings
        bindings::register_hardware(&mut engine);

        Self { engine, runtime }
    }

    /// Execute a Rhai script and return the result.
    ///
    /// Evaluates the script with a fresh scope. Variables do not persist
    /// between calls. For persistent state, use the V5 `RhaiEngine` instead.
    ///
    /// # Arguments
    ///
    /// * `script` - Rhai script source code
    ///
    /// # Returns
    ///
    /// The last expression value from the script as a Rhai `Dynamic`.
    ///
    /// # Errors
    ///
    /// Returns `Box<EvalAltResult>` if:
    /// - Script has syntax errors
    /// - Runtime error occurs during execution
    /// - Safety limit (10,000 operations) is exceeded
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let result = host.run_script("let x = 10; x * 2")?;
    /// assert_eq!(result.cast::<i64>(), 20);
    /// ```
    pub fn run_script(&self, script: &str) -> Result<Dynamic, Box<EvalAltResult>> {
        let mut scope = Scope::new();
        self.engine.eval_with_scope(&mut scope, script)
    }

    /// Validate script syntax without executing it.
    ///
    /// This checks if the script would compile successfully without
    /// actually running it. Useful for validating user input before execution.
    ///
    /// # Arguments
    ///
    /// * `script` - Rhai script source code to validate
    ///
    /// # Errors
    ///
    /// Returns `Box<EvalAltResult>` if the script has syntax errors.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Valid script
    /// assert!(host.validate_script("let x = 1 + 2;").is_ok());
    ///
    /// // Invalid script
    /// assert!(host.validate_script("let x = 1 +").is_err());
    /// ```
    pub fn validate_script(&self, script: &str) -> Result<(), Box<EvalAltResult>> {
        self.engine.compile(script)?;
        Ok(())
    }

    /// Get mutable access to the underlying Rhai engine.
    ///
    /// This allows registering custom functions, types, or modifying
    /// engine settings directly. Use with caution as it bypasses the
    /// ScriptHost abstractions.
    ///
    /// # Returns
    ///
    /// Mutable reference to the Rhai engine.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut host = ScriptHost::new(Handle::current());
    /// host.engine_mut().register_fn("custom_fn", |x: i64| x * 3);
    /// ```
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }
}
