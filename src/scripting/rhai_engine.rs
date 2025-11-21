//! Rhai Implementation of ScriptEngine Trait
//!
//! This module provides a `RhaiEngine` implementation of the `ScriptEngine` trait,
//! using the Rhai scripting language. Rhai is a fast, embedded scripting language
//! with Rust-like syntax and excellent Rust integration.
//!
//! # Features
//!
//! - **Fast Execution** - Compiled to bytecode for efficient execution
//! - **Type Safety** - Strong typing with runtime type checking
//! - **Rust Integration** - Natural FFI with Rust functions and types
//! - **Safety Limits** - Built-in protection against infinite loops
//!
//! # Example
//!
//! ```rust,ignore
//! use rust_daq::scripting::{ScriptEngine, RhaiEngine, ScriptValue};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut engine = RhaiEngine::new();
//!
//!     // Register a function
//!     engine.register_rust_fn("greet", |name: String| {
//!         format!("Hello, {}!", name)
//!     })?;
//!
//!     // Execute script
//!     let result = engine.execute_script(r#"
//!         let msg = greet("World");
//!         msg
//!     "#).await?;
//!
//!     let msg: String = result.downcast()?;
//!     println!("{}", msg);
//!     Ok(())
//! }
//! ```

use super::script_engine::{ScriptEngine, ScriptError, ScriptValue};
use async_trait::async_trait;
use rhai::{Dynamic, Engine, EvalAltResult, Scope};
use std::any::Any;
use std::sync::{Arc, Mutex};

// =============================================================================
// RhaiEngine Implementation
// =============================================================================

/// Rhai-based implementation of ScriptEngine
///
/// This struct wraps a Rhai `Engine` and provides thread-safe access through
/// interior mutability. The Rhai engine is configured with safety limits to
/// prevent infinite loops and excessive operations.
///
/// # Thread Safety
///
/// The engine is wrapped in `Arc<Mutex<>>` to allow safe concurrent access.
/// While Rhai engines themselves are not `Sync`, the mutex ensures thread safety.
///
/// # Safety Limits
///
/// - Maximum 10,000 operations per script execution
/// - Progress callback to detect infinite loops
pub struct RhaiEngine {
    engine: Arc<Mutex<Engine>>,
    scope: Arc<Mutex<Scope<'static>>>,
}

impl RhaiEngine {
    /// Create a new RhaiEngine with default safety settings
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let engine = RhaiEngine::new();
    /// ```
    pub fn new() -> Self {
        let mut engine = Engine::new();

        // Configure safety limits to prevent infinite loops
        engine.on_progress(|count| {
            if count > 10000 {
                Some("Safety limit exceeded: maximum 10000 operations".into())
            } else {
                None
            }
        });

        Self {
            engine: Arc::new(Mutex::new(engine)),
            scope: Arc::new(Mutex::new(Scope::new())),
        }
    }

    /// Create a new RhaiEngine with custom operation limit
    ///
    /// # Arguments
    ///
    /// * `max_operations` - Maximum number of operations allowed per execution
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let engine = RhaiEngine::with_limit(100_000);
    /// ```
    pub fn with_limit(max_operations: u64) -> Self {
        let mut engine = Engine::new();

        engine.on_progress(move |count| {
            if count > max_operations {
                Some(
                    format!(
                        "Safety limit exceeded: maximum {} operations",
                        max_operations
                    )
                    .into(),
                )
            } else {
                None
            }
        });

        Self {
            engine: Arc::new(Mutex::new(engine)),
            scope: Arc::new(Mutex::new(Scope::new())),
        }
    }

    /// Register a Rust function with proper type handling
    ///
    /// This is a type-safe wrapper around `register_function` that works
    /// with concrete Rust function types. Due to Rhai's type system, this
    /// method accepts any function that Rhai can register.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// engine.register_rust_fn("add", |a: i64, b: i64| a + b)?;
    /// ```
    pub fn register_rust_fn<F>(&mut self, name: &str, func: F) -> Result<(), ScriptError>
    where
        F: rhai::plugin::RhaiNativeFunc + 'static,
    {
        let mut engine = self.engine.lock().unwrap();
        engine.register_fn(name, func);
        Ok(())
    }

    /// Get mutable access to the underlying Rhai engine
    ///
    /// Useful for advanced configuration not exposed through the trait.
    ///
    /// # Safety
    ///
    /// This locks the mutex, so don't hold the returned guard across await points.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// {
    ///     let mut guard = engine.engine_mut();
    ///     guard.register_type::<MyCustomType>();
    /// } // Lock released here
    /// ```
    pub fn engine_mut(&mut self) -> std::sync::MutexGuard<'_, Engine> {
        self.engine.lock().unwrap()
    }
}

impl Default for RhaiEngine {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// ScriptEngine Trait Implementation
// =============================================================================

#[async_trait]
impl ScriptEngine for RhaiEngine {
    async fn execute_script(&mut self, script: &str) -> Result<ScriptValue, ScriptError> {
        let engine = self.engine.clone();
        let scope = self.scope.clone();
        let script = script.to_string();

        // Execute in a blocking task to avoid blocking the async runtime
        tokio::task::spawn_blocking(move || {
            let engine = engine.lock().unwrap();
            let mut scope = scope.lock().unwrap();

            let result: Dynamic = engine
                .eval_with_scope(&mut scope, &script)
                .map_err(|e| convert_rhai_error(e))?;

            Ok(ScriptValue::new(result))
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
            let engine = engine.lock().unwrap();
            engine
                .compile(&script)
                .map_err(|e| convert_rhai_error(e))?;
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
        function: Box<dyn Any + Send + Sync>,
    ) -> Result<(), ScriptError> {
        // For Rhai, we need to handle this differently since Rhai's registration
        // requires concrete types at compile time. This method is primarily for
        // type-erased registration, which is tricky with Rhai.
        //
        // Users should prefer `register_rust_fn` for type-safe registration.
        //
        // This implementation serves as a placeholder that documents the limitation.
        Err(ScriptError::BackendError {
            backend: "Rhai".to_string(),
            message: format!(
                "Type-erased function registration not supported for '{}'. \
                 Use RhaiEngine::register_rust_fn() instead for type-safe registration.",
                name
            ),
        })
    }

    fn set_global(&mut self, name: &str, value: ScriptValue) -> Result<(), ScriptError> {
        // Extract the Dynamic value from ScriptValue
        let dynamic = value
            .downcast::<Dynamic>()
            .map_err(|_| ScriptError::TypeConversionError {
                expected: "rhai::Dynamic".to_string(),
                found: "unknown".to_string(),
            })?;

        let mut scope = self.scope.lock().unwrap();
        scope.push(name, dynamic);
        Ok(())
    }

    fn get_global(&self, name: &str) -> Result<ScriptValue, ScriptError> {
        let scope = self.scope.lock().unwrap();

        scope
            .get_value::<Dynamic>(name)
            .ok_or_else(|| ScriptError::VariableNotFound {
                name: name.to_string(),
            })
            .map(ScriptValue::new)
    }

    fn clear_globals(&mut self) {
        let mut scope = self.scope.lock().unwrap();
        scope.clear();
    }

    fn backend_name(&self) -> &str {
        "Rhai"
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Convert Rhai's EvalAltResult to our ScriptError type
fn convert_rhai_error(error: Box<EvalAltResult>) -> ScriptError {
    match *error {
        EvalAltResult::ErrorParsing(parse_error, pos) => ScriptError::CompilationError {
            message: format!("{}", parse_error),
            line: Some(pos.line().unwrap_or(0)),
            column: Some(pos.position().unwrap_or(0)),
        },
        EvalAltResult::ErrorRuntime(message, _) => ScriptError::RuntimeError {
            message: message.to_string(),
            backtrace: None,
        },
        EvalAltResult::ErrorMismatchDataType(expected, actual, _) => {
            ScriptError::TypeConversionError {
                expected,
                found: actual,
            }
        }
        other => ScriptError::RuntimeError {
            message: format!("{}", other),
            backtrace: None,
        },
    }
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rhai_engine_creation() {
        let engine = RhaiEngine::new();
        assert_eq!(engine.backend_name(), "Rhai");
    }

    #[tokio::test]
    async fn test_execute_simple_script() {
        let mut engine = RhaiEngine::new();
        let result = engine.execute_script("1 + 2 + 3").await.unwrap();
        let value: Dynamic = result.downcast().unwrap();
        assert_eq!(value.as_int().unwrap(), 6);
    }

    #[tokio::test]
    async fn test_execute_string_script() {
        let mut engine = RhaiEngine::new();
        let result = engine
            .execute_script(r#""Hello, " + "World!""#)
            .await
            .unwrap();
        let value: Dynamic = result.downcast().unwrap();
        assert_eq!(value.cast::<String>(), "Hello, World!");
    }

    #[tokio::test]
    async fn test_validate_valid_script() {
        let engine = RhaiEngine::new();
        let result = engine.validate_script("let x = 10; x * 2").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_validate_invalid_script() {
        let engine = RhaiEngine::new();
        let result = engine.validate_script("let x = ; invalid").await;
        assert!(result.is_err());

        if let Err(ScriptError::CompilationError { message, .. }) = result {
            assert!(!message.is_empty());
        } else {
            panic!("Expected CompilationError");
        }
    }

    #[tokio::test]
    async fn test_set_and_get_global() {
        let mut engine = RhaiEngine::new();

        // Set a global variable
        engine
            .set_global("test_var", ScriptValue::new(Dynamic::from(42_i64)))
            .unwrap();

        // Use it in a script
        let result = engine.execute_script("test_var * 2").await.unwrap();
        let value: Dynamic = result.downcast().unwrap();
        assert_eq!(value.as_int().unwrap(), 84);

        // Get it back
        let retrieved = engine.get_global("test_var").unwrap();
        let value: Dynamic = retrieved.downcast().unwrap();
        assert_eq!(value.as_int().unwrap(), 42);
    }

    #[tokio::test]
    async fn test_get_nonexistent_global() {
        let engine = RhaiEngine::new();
        let result = engine.get_global("nonexistent");
        assert!(matches!(result, Err(ScriptError::VariableNotFound { .. })));
    }

    #[tokio::test]
    async fn test_clear_globals() {
        let mut engine = RhaiEngine::new();

        engine
            .set_global("var1", ScriptValue::new(Dynamic::from(10_i64)))
            .unwrap();
        engine
            .set_global("var2", ScriptValue::new(Dynamic::from(20_i64)))
            .unwrap();

        engine.clear_globals();

        assert!(engine.get_global("var1").is_err());
        assert!(engine.get_global("var2").is_err());
    }

    #[tokio::test]
    async fn test_register_rust_function() {
        let mut engine = RhaiEngine::new();

        // Register a simple addition function
        engine
            .register_rust_fn("add", |a: i64, b: i64| a + b)
            .unwrap();

        let result = engine.execute_script("add(10, 20)").await.unwrap();
        let value: Dynamic = result.downcast().unwrap();
        assert_eq!(value.as_int().unwrap(), 30);
    }

    #[tokio::test]
    async fn test_register_rust_function_string() {
        let mut engine = RhaiEngine::new();

        engine
            .register_rust_fn("greet", |name: String| format!("Hello, {}!", name))
            .unwrap();

        let result = engine
            .execute_script(r#"greet("Rust")"#)
            .await
            .unwrap();
        let value: Dynamic = result.downcast().unwrap();
        assert_eq!(value.cast::<String>(), "Hello, Rust!");
    }

    #[tokio::test]
    async fn test_safety_limit() {
        let mut engine = RhaiEngine::with_limit(100);

        // This should exceed the limit
        let result = engine
            .execute_script(
                r#"
            let x = 0;
            loop {
                x += 1;
                if x > 1000 { break; }
            }
            x
        "#,
            )
            .await;

        assert!(result.is_err());
        if let Err(ScriptError::RuntimeError { message, .. }) = result {
            assert!(message.contains("Safety limit"));
        } else {
            panic!("Expected RuntimeError with safety limit message");
        }
    }

    #[tokio::test]
    async fn test_runtime_error_handling() {
        let mut engine = RhaiEngine::new();

        // Division by zero should produce a runtime error
        let result = engine.execute_script("1 / 0").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_multiple_executions() {
        let mut engine = RhaiEngine::new();

        // First execution
        let result1 = engine.execute_script("5 + 5").await.unwrap();
        let value1: Dynamic = result1.downcast().unwrap();
        assert_eq!(value1.as_int().unwrap(), 10);

        // Second execution should work independently
        let result2 = engine.execute_script("10 * 2").await.unwrap();
        let value2: Dynamic = result2.downcast().unwrap();
        assert_eq!(value2.as_int().unwrap(), 20);
    }

    #[tokio::test]
    async fn test_persistent_scope() {
        let mut engine = RhaiEngine::new();

        // Set a variable in first script
        engine.execute_script("let counter = 0;").await.unwrap();

        // Use it in second script
        engine.execute_script("counter += 10;").await.unwrap();

        // Verify persistence
        let result = engine.execute_script("counter").await.unwrap();
        let value: Dynamic = result.downcast().unwrap();
        assert_eq!(value.as_int().unwrap(), 10);
    }
}
