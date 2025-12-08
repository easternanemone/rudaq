//! ScriptEngine Trait - Universal Scripting Interface
//!
//! This module defines the `ScriptEngine` trait, which provides a unified interface
//! for executing scripts in rust-daq regardless of the underlying scripting backend
//! (Rhai, Python via PyO3, Lua, etc.).
//!
//! # Architecture
//!
//! The ScriptEngine trait is async-first and designed to work with the V5 headless-first
//! architecture. It supports:
//!
//! - **Async execution**: All script operations are async to integrate with tokio runtime
//! - **Multiple backends**: Rhai (embedded), PyO3 (Python), future backends (Lua, JavaScript)
//! - **Type-safe values**: `ScriptValue` wraps `Box<dyn Any>` for Rust↔script data exchange
//! - **Error handling**: Rich `ScriptError` enum with backtraces and backend-specific info
//! - **Function registration**: Register Rust functions callable from scripts
//! - **Global variables**: Set/get variables in script global scope
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use rust_daq::scripting::{ScriptEngine, RhaiEngine, ScriptValue};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut engine = RhaiEngine::new()?;
//!
//!     // Set global variables
//!     engine.set_global("wavelength", ScriptValue::new(800))?;
//!
//!     // Execute script
//!     let script = r#"
//!         print("Setting wavelength to " + wavelength + " nm");
//!         result = wavelength * 2;
//!     "#;
//!
//!     engine.execute_script(script).await?;
//!     let result = engine.get_global("result")?;
//!     println!("Result: {:?}", result.downcast_ref::<i64>());
//!
//!     Ok(())
//! }
//! ```
//!
//! # Backend Implementations
//!
//! - `RhaiEngine` - Rhai embedded scripting (default, no external dependencies)
//! - `PyO3Engine` - Python via PyO3 (requires `scripting_python` feature)
//!
//! # Design Principles
//!
//! 1. **Backend agnostic**: CLI tools and libraries use the trait, not concrete types
//! 2. **Async-first**: All execution is async to avoid blocking tokio runtime
//! 3. **Error transparency**: ScriptError preserves backend-specific error details
//! 4. **Zero-copy where possible**: ScriptValue uses `Box<dyn Any>` to avoid unnecessary cloning

use async_trait::async_trait;
use std::any::Any;
use std::fmt;

// =============================================================================
// ScriptValue - Type-Erased Value Container
// =============================================================================

/// Type-erased container for values passed between Rust and scripts
///
/// This wraps `Box<dyn Any + Send + Sync>` to allow passing arbitrary Rust types
/// to/from scripts. Backend implementations are responsible for converting between
/// their native types (Rhai Dynamic, Python PyObject) and Rust types.
///
/// # Example
/// ```rust,ignore
/// let value = ScriptValue::new(42_i64);
/// let number: i64 = value.downcast().unwrap();
/// ```
pub struct ScriptValue {
    inner: Box<dyn Any + Send + Sync>,
}

impl ScriptValue {
    /// Create a new ScriptValue from any Send + Sync type
    pub fn new<T: Any + Send + Sync>(value: T) -> Self {
        Self {
            inner: Box::new(value),
        }
    }

    /// Try to downcast to a concrete type by value
    ///
    /// Returns the value if the type matches, otherwise returns an error.
    pub fn downcast<T: Any>(self) -> Result<T, Self> {
        match self.inner.downcast::<T>() {
            Ok(boxed) => Ok(*boxed),
            Err(inner) => Err(Self { inner }),
        }
    }

    /// Try to downcast to a concrete type by reference
    ///
    /// Returns a reference to the value if the type matches.
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.inner.downcast_ref::<T>()
    }

    /// Try to downcast to a concrete type by mutable reference
    pub fn downcast_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.inner.downcast_mut::<T>()
    }

    /// Check if the value is of a specific type
    pub fn is<T: Any>(&self) -> bool {
        self.inner.is::<T>()
    }
}

impl fmt::Debug for ScriptValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Try to format common types for debugging
        if let Some(s) = self.downcast_ref::<String>() {
            write!(f, "ScriptValue(String: {:?})", s)
        } else if let Some(i) = self.downcast_ref::<i64>() {
            write!(f, "ScriptValue(i64: {})", i)
        } else if let Some(fl) = self.downcast_ref::<f64>() {
            write!(f, "ScriptValue(f64: {})", fl)
        } else if let Some(b) = self.downcast_ref::<bool>() {
            write!(f, "ScriptValue(bool: {})", b)
        } else if self.downcast_ref::<()>().is_some() {
            write!(f, "ScriptValue(())")
        } else {
            write!(f, "ScriptValue(<unknown type>)")
        }
    }
}

// =============================================================================
// ScriptError - Unified Error Type
// =============================================================================

/// Error type for script execution failures
///
/// Provides detailed error information including backend name, error messages,
/// and optional backtraces. All backend implementations should map their native
/// errors to this type.
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    /// Script compilation/syntax error
    ///
    /// Indicates the script has invalid syntax and cannot be compiled/parsed.
    /// Check the message for details about what syntax is incorrect.
    #[error("Syntax error: {message}")]
    SyntaxError {
        /// Human-readable description of the syntax error
        message: String,
    },

    /// Runtime error during script execution
    ///
    /// Indicates an error occurred while executing a valid script, such as
    /// division by zero, type mismatch, or hardware operation failure.
    #[error("Runtime error: {message}{}", .backtrace.as_ref().map(|b| format!("\n{}", b)).unwrap_or_default())]
    RuntimeError {
        /// Human-readable description of the runtime error
        message: String,
        /// Optional stack trace showing where the error occurred in the script
        backtrace: Option<String>,
    },

    /// Variable not found in global scope
    ///
    /// Indicates an attempt to access a variable that doesn't exist in the
    /// script's global namespace.
    #[error("Variable not found: {name}")]
    VariableNotFound {
        /// Name of the missing variable
        name: String,
    },

    /// Type conversion error between Rust and script types
    ///
    /// Indicates a value couldn't be converted between Rust and the script
    /// backend's type system (e.g., trying to extract a String from an i64).
    #[error("Type conversion error: expected {expected}, found {found}")]
    TypeConversionError {
        /// Expected type name
        expected: String,
        /// Actual type found
        found: String,
    },

    /// Backend-specific error (e.g., PyO3 initialization failure)
    ///
    /// Indicates an error specific to the scripting backend implementation,
    /// such as Python interpreter initialization failure or Rhai engine setup error.
    #[error("Backend error ({backend}): {message}")]
    BackendError {
        /// Name of the backend (e.g., "Rhai", "PyO3")
        backend: String,
        /// Backend-specific error message
        message: String,
    },

    /// Async task join error
    ///
    /// Indicates a tokio task failed to complete. This typically means the
    /// task panicked or was cancelled unexpectedly.
    #[error("Async error: {message}")]
    AsyncError {
        /// Description of the async error
        message: String,
    },

    /// Function registration error
    ///
    /// Indicates a function could not be registered with the script engine,
    /// usually due to type incompatibility or backend limitations.
    #[error("Function registration error: {message}")]
    FunctionRegistrationError {
        /// Description of why registration failed
        message: String,
    },
}

// =============================================================================
// ScriptEngine Trait
// =============================================================================

/// Universal scripting interface for rust-daq
///
/// This trait defines the contract that all scripting backends must implement.
/// It provides async execution, global variable management, function registration,
/// and validation capabilities.
///
/// # Thread Safety
///
/// All trait methods take `&mut self` to ensure exclusive access during script
/// operations. Implementations should use internal synchronization (Arc<Mutex<>>)
/// if they need to be cloned across threads.
///
/// # Async Design
///
/// All execution methods are async to integrate with tokio runtime. Backends that
/// are synchronous (like Rhai) should use `tokio::task::spawn_blocking` internally.
#[async_trait]
pub trait ScriptEngine: Send + Sync {
    /// Execute a script and return the result
    ///
    /// The script has access to all registered functions and global variables.
    /// The return value is backend-specific:
    /// - Rhai: Returns the last expression value
    /// - Python: Returns the value of a variable named `result` if it exists
    ///
    /// # Arguments
    /// * `script` - Script source code
    ///
    /// # Returns
    /// The script's return value wrapped in `ScriptValue`
    ///
    /// # Errors
    /// Returns `ScriptError` if execution fails (syntax error, runtime error, etc.)
    async fn execute_script(&mut self, script: &str) -> Result<ScriptValue, ScriptError>;

    /// Validate script syntax without executing it
    ///
    /// This checks if the script would compile/parse successfully without
    /// actually running it. Useful for validating user input before execution.
    ///
    /// # Arguments
    /// * `script` - Script source code
    ///
    /// # Errors
    /// Returns `ScriptError::SyntaxError` if script has syntax errors
    async fn validate_script(&self, script: &str) -> Result<(), ScriptError>;

    /// Register a Rust function that can be called from scripts
    ///
    /// The function is wrapped in `Box<dyn Any>` to allow backend-specific types.
    /// Implementations should downcast to their native function type:
    /// - Rhai: Should be a closure compatible with rhai::Engine::register_fn
    /// - PyO3: Should be a Py<PyAny> created with wrap_pyfunction!
    ///
    /// # Arguments
    /// * `name` - Function name as it appears in scripts
    /// * `function` - Backend-specific function wrapper
    ///
    /// # Errors
    /// Returns error if function type is incompatible with backend
    fn register_function(
        &mut self,
        name: &str,
        function: Box<dyn Any + Send + Sync>,
    ) -> Result<(), ScriptError>;

    /// Set a global variable accessible to all subsequent script executions
    ///
    /// # Arguments
    /// * `name` - Variable name
    /// * `value` - Variable value
    ///
    /// # Errors
    /// Returns error if type conversion fails
    fn set_global(&mut self, name: &str, value: ScriptValue) -> Result<(), ScriptError>;

    /// Get a global variable from the script environment
    ///
    /// # Arguments
    /// * `name` - Variable name
    ///
    /// # Returns
    /// The variable's value wrapped in `ScriptValue`
    ///
    /// # Errors
    /// Returns `ScriptError::VariableNotFound` if variable doesn't exist
    fn get_global(&self, name: &str) -> Result<ScriptValue, ScriptError>;

    /// Clear all global variables and registered functions
    ///
    /// Resets the script environment to a clean state.
    fn clear_globals(&mut self);

    /// Get the backend name for debugging/logging
    ///
    /// # Returns
    /// Human-readable backend name (e.g., "Rhai", "PyO3 (Python)")
    fn backend_name(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_script_value_creation() {
        let value = ScriptValue::new(42_i64);
        assert!(value.is::<i64>());
        assert!(!value.is::<String>());
    }

    #[test]
    fn test_script_value_downcast() {
        let value = ScriptValue::new(42_i64);
        let number: i64 = value.downcast().unwrap();
        assert_eq!(number, 42);
    }

    #[test]
    fn test_script_value_downcast_ref() {
        let value = ScriptValue::new("hello".to_string());
        let text = value.downcast_ref::<String>().unwrap();
        assert_eq!(text, "hello");
    }

    #[test]
    fn test_script_value_debug() {
        let value = ScriptValue::new(42_i64);
        let debug_str = format!("{:?}", value);
        assert!(debug_str.contains("i64"));
        assert!(debug_str.contains("42"));
    }

    #[test]
    fn test_script_error_display() {
        let err = ScriptError::SyntaxError {
            message: "unexpected token".to_string(),
        };
        assert_eq!(err.to_string(), "Syntax error: unexpected token");

        let err = ScriptError::VariableNotFound {
            name: "foo".to_string(),
        };
        assert_eq!(err.to_string(), "Variable not found: foo");
    }
}
