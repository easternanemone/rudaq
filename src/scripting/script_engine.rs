//! Generic ScriptEngine Trait for Hot-Swappable Scripting Backends
//!
//! This module defines the `ScriptEngine` trait that provides a unified interface
//! for multiple scripting backends (Rhai, Python/PyO3, Lua/mlua, etc.). The design
//! allows for hot-swapping different scripting engines at runtime while maintaining
//! a consistent API.
//!
//! # Architecture
//!
//! The trait is designed around three core concepts:
//! 1. **Script Execution** - Running scripts with various input/output types
//! 2. **Function Registration** - Exposing Rust functions to scripts
//! 3. **Global State Management** - Setting and getting global variables
//!
//! # Async Support
//!
//! All execution methods return `BoxFuture` to support both synchronous and
//! asynchronous backends. Synchronous engines (like Rhai) can wrap their
//! operations in `Box::pin(async move { ... })`.
//!
//! # Error Handling
//!
//! The trait uses a custom `ScriptError` type that encapsulates errors from
//! different backends, providing consistent error reporting across engines.
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use rust_daq::scripting::{ScriptEngine, RhaiEngine};
//!
//! async fn run_experiment() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut engine = RhaiEngine::new();
//!
//!     // Register a Rust function
//!     engine.register_function("log", |msg: String| {
//!         println!("Script: {}", msg);
//!     })?;
//!
//!     // Set global variables
//!     engine.set_global("experiment_name", "Demo")?;
//!     engine.set_global("num_samples", 100)?;
//!
//!     // Execute script
//!     let script = r#"
//!         log("Starting " + experiment_name);
//!         for i in 0..num_samples {
//!             // Acquisition logic
//!         }
//!     "#;
//!
//!     engine.execute_script(script).await?;
//!     Ok(())
//! }
//! ```
//!
//! # Implementation Guide
//!
//! To implement `ScriptEngine` for a new backend:
//!
//! 1. Create a struct to hold the backend state
//! 2. Implement `execute_script` to run code
//! 3. Implement `register_function` to expose Rust functions
//! 4. Implement `set_global` and `get_global` for state management
//! 5. Handle type conversions between Rust and the script language
//!
//! See `RhaiEngine` for a reference implementation.

use async_trait::async_trait;
use std::any::Any;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

// Type alias for boxed async futures
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// =============================================================================
// Error Types
// =============================================================================

/// Errors that can occur during script execution
///
/// This enum provides a unified error type across different scripting backends,
/// making it easier to handle errors consistently regardless of which engine
/// is being used.
#[derive(Debug, Clone)]
pub enum ScriptError {
    /// Compilation or parsing error
    CompilationError {
        message: String,
        line: Option<usize>,
        column: Option<usize>,
    },

    /// Runtime execution error
    RuntimeError {
        message: String,
        backtrace: Option<String>,
    },

    /// Type conversion error between Rust and script types
    TypeConversionError {
        expected: String,
        found: String,
    },

    /// Function not found during registration
    FunctionNotFound {
        name: String,
    },

    /// Variable not found in global scope
    VariableNotFound {
        name: String,
    },

    /// Backend-specific error (for errors unique to a particular engine)
    BackendError {
        backend: String,
        message: String,
    },

    /// Error during async operation
    AsyncError {
        message: String,
    },
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScriptError::CompilationError { message, line, column } => {
                write!(f, "Compilation error: {}", message)?;
                if let Some(line) = line {
                    write!(f, " at line {}", line)?;
                }
                if let Some(col) = column {
                    write!(f, ", column {}", col)?;
                }
                Ok(())
            }
            ScriptError::RuntimeError { message, backtrace } => {
                write!(f, "Runtime error: {}", message)?;
                if let Some(bt) = backtrace {
                    write!(f, "\n{}", bt)?;
                }
                Ok(())
            }
            ScriptError::TypeConversionError { expected, found } => {
                write!(f, "Type conversion error: expected {}, found {}", expected, found)
            }
            ScriptError::FunctionNotFound { name } => {
                write!(f, "Function not found: {}", name)
            }
            ScriptError::VariableNotFound { name } => {
                write!(f, "Variable not found: {}", name)
            }
            ScriptError::BackendError { backend, message } => {
                write!(f, "{} backend error: {}", backend, message)
            }
            ScriptError::AsyncError { message } => {
                write!(f, "Async error: {}", message)
            }
        }
    }
}

impl std::error::Error for ScriptError {}

// =============================================================================
// ScriptValue - Type-Erased Value Container
// =============================================================================

/// A type-erased container for values that can be passed between Rust and scripts
///
/// This type wraps `Box<dyn Any>` to allow different backends to return their
/// native types while maintaining type safety through downcasting.
///
/// # Example
///
/// ```rust,ignore
/// let value = ScriptValue::from(42_i64);
/// let num: i64 = value.downcast()?;
/// assert_eq!(num, 42);
/// ```
#[derive(Debug)]
pub struct ScriptValue {
    inner: Box<dyn Any + Send + Sync>,
}

impl ScriptValue {
    /// Create a new ScriptValue from any type that is Send + Sync + 'static
    pub fn new<T: Any + Send + Sync>(value: T) -> Self {
        Self {
            inner: Box::new(value),
        }
    }

    /// Attempt to downcast to a concrete type
    pub fn downcast<T: Any>(self) -> Result<T, ScriptError> {
        self.inner
            .downcast::<T>()
            .map(|boxed| *boxed)
            .map_err(|_| ScriptError::TypeConversionError {
                expected: std::any::type_name::<T>().to_string(),
                found: "unknown".to_string(),
            })
    }

    /// Attempt to get a reference to the inner value
    pub fn downcast_ref<T: Any>(&self) -> Result<&T, ScriptError> {
        self.inner
            .downcast_ref::<T>()
            .ok_or_else(|| ScriptError::TypeConversionError {
                expected: std::any::type_name::<T>().to_string(),
                found: "unknown".to_string(),
            })
    }
}

// Note: Cannot implement From<T> for ScriptValue due to blanket implementation in core.
// Users should call ScriptValue::new(value) directly or use specific From implementations.

// =============================================================================
// ScriptEngine Trait
// =============================================================================

/// Generic interface for scripting backends
///
/// This trait defines the core operations that any scripting engine must support
/// to be used within the rust-daq system. Implementations exist for:
/// - Rhai (embedded, fast, Rust-like syntax)
/// - Python (via PyO3, for complex analysis)
/// - Lua (via mlua, lightweight)
///
/// # Design Principles
///
/// 1. **Backend Agnostic** - Code using this trait should work with any engine
/// 2. **Async First** - All operations are async to support both sync and async backends
/// 3. **Type Safe** - Strong typing with runtime type checking via `ScriptValue`
/// 4. **Error Recovery** - Detailed error information for debugging
///
/// # Thread Safety
///
/// Implementations must be Send + Sync to allow use across async tasks.
/// Most scripting engines achieve this through internal synchronization or
/// by using thread-local storage.
#[async_trait]
pub trait ScriptEngine: Send + Sync {
    /// Execute a script and return the result
    ///
    /// This is the primary method for running scripts. The script is compiled
    /// (if necessary) and executed, returning the final expression value or
    /// an explicit return value.
    ///
    /// # Arguments
    ///
    /// * `script` - The script source code as a string
    ///
    /// # Returns
    ///
    /// * `Ok(ScriptValue)` - The result of script execution
    /// * `Err(ScriptError)` - Compilation or runtime error
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let result = engine.execute_script("1 + 2 * 3").await?;
    /// let num: i64 = result.downcast()?;
    /// assert_eq!(num, 7);
    /// ```
    async fn execute_script(&mut self, script: &str) -> Result<ScriptValue, ScriptError>;

    /// Validate script syntax without executing it
    ///
    /// Useful for providing early feedback in script editors or before
    /// saving scripts to disk.
    ///
    /// # Arguments
    ///
    /// * `script` - The script source code to validate
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Script is syntactically valid
    /// * `Err(ScriptError::CompilationError)` - Syntax error with location
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// if let Err(e) = engine.validate_script(user_script).await {
    ///     println!("Syntax error: {}", e);
    /// }
    /// ```
    async fn validate_script(&self, script: &str) -> Result<(), ScriptError>;

    /// Register a Rust function to be callable from scripts
    ///
    /// This method allows exposing Rust functionality to scripts. The function
    /// signature and name are determined by the implementation. Different backends
    /// may have different constraints on function signatures.
    ///
    /// # Arguments
    ///
    /// * `name` - The name the function will have in the script
    /// * `function` - A boxed function that can be called from scripts
    ///
    /// # Type Constraints
    ///
    /// The function must be Send + Sync + 'static to allow safe concurrent access.
    /// The exact signature varies by backend but typically looks like:
    /// - Rhai: `Fn(T1, T2, ...) -> R`
    /// - Python: `Fn(Python, &PyTuple, Option<&PyDict>) -> PyResult<T>`
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// engine.register_function("add", Box::new(|a: i64, b: i64| a + b))?;
    /// let result = engine.execute_script("add(10, 20)").await?;
    /// ```
    fn register_function(
        &mut self,
        name: &str,
        function: Box<dyn Any + Send + Sync>,
    ) -> Result<(), ScriptError>;

    /// Set a global variable in the script environment
    ///
    /// Global variables persist across script executions and can be used to
    /// pass state from Rust to scripts or between script invocations.
    ///
    /// # Arguments
    ///
    /// * `name` - The variable name in the script environment
    /// * `value` - The value to set (wrapped in ScriptValue)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// engine.set_global("max_samples", ScriptValue::from(1000_i64))?;
    /// engine.execute_script("print(max_samples)").await?;
    /// ```
    fn set_global(&mut self, name: &str, value: ScriptValue) -> Result<(), ScriptError>;

    /// Get a global variable from the script environment
    ///
    /// Retrieves a value that was either set via `set_global` or created
    /// by a script.
    ///
    /// # Arguments
    ///
    /// * `name` - The variable name to retrieve
    ///
    /// # Returns
    ///
    /// * `Ok(ScriptValue)` - The variable's value
    /// * `Err(ScriptError::VariableNotFound)` - Variable doesn't exist
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// engine.execute_script("let result = 42").await?;
    /// let value = engine.get_global("result")?;
    /// let num: i64 = value.downcast()?;
    /// ```
    fn get_global(&self, name: &str) -> Result<ScriptValue, ScriptError>;

    /// Clear all global variables
    ///
    /// Resets the script environment to a clean state, removing all global
    /// variables and registered functions. Useful for isolating script
    /// executions or cleaning up between tests.
    fn clear_globals(&mut self);

    /// Get the name of the scripting backend
    ///
    /// Returns a human-readable name for the engine implementation, useful
    /// for logging and debugging.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// println!("Using {} scripting engine", engine.backend_name());
    /// ```
    fn backend_name(&self) -> &str;
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_script_value_creation() {
        let value = ScriptValue::from(42_i64);
        let num: i64 = value.downcast().unwrap();
        assert_eq!(num, 42);
    }

    #[test]
    fn test_script_value_string() {
        let value = ScriptValue::from("hello".to_string());
        let s: String = value.downcast().unwrap();
        assert_eq!(s, "hello");
    }

    #[test]
    fn test_script_value_downcast_error() {
        let value = ScriptValue::from(42_i64);
        let result: Result<String, _> = value.downcast();
        assert!(result.is_err());
    }

    #[test]
    fn test_script_value_downcast_ref() {
        let value = ScriptValue::from(42_i64);
        let num_ref: &i64 = value.downcast_ref().unwrap();
        assert_eq!(*num_ref, 42);
    }

    #[test]
    fn test_script_error_display() {
        let error = ScriptError::CompilationError {
            message: "unexpected token".to_string(),
            line: Some(10),
            column: Some(5),
        };
        let display = format!("{}", error);
        assert!(display.contains("line 10"));
        assert!(display.contains("column 5"));
    }

    #[test]
    fn test_script_error_runtime() {
        let error = ScriptError::RuntimeError {
            message: "division by zero".to_string(),
            backtrace: Some("at function foo()".to_string()),
        };
        let display = format!("{}", error);
        assert!(display.contains("division by zero"));
        assert!(display.contains("at function foo()"));
    }
}
