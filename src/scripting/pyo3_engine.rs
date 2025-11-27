//! PyO3 ScriptEngine Implementation
//!
//! This module provides a Python scripting backend using PyO3, implementing
//! the `ScriptEngine` trait. It allows executing Python code within the
//! rust-daq application with full access to registered Rust functions.
//!
//! # Features
//!
//! - Execute Python scripts with access to global variables
//! - Register Rust functions callable from Python
//! - Type-safe value passing between Rust and Python
//! - Async-compatible execution model
//! - Automatic Python interpreter initialization
//!
//! # Example
//!
//! ```rust,ignore
//! use rust_daq::scripting::{ScriptEngine, PyO3Engine};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut engine = PyO3Engine::new()?;
//!     
//!     // Set global variables
//!     engine.set_global("experiment_name", ScriptValue::new("Demo".to_string()))?;
//!     
//!     // Execute Python script
//!     let script = r#"
//! print(f"Running {experiment_name}")
//! result = 42 * 2
//! "#;
//!     
//!     engine.execute_script(script).await?;
//!     let result = engine.get_global("result")?;
//!     println!("Result: {:?}", result);
//!     
//!     Ok(())
//! }
//! ```

#[cfg(feature = "scripting_python")]
use pyo3::prelude::*;
#[cfg(feature = "scripting_python")]
use pyo3::types::PyModule;

/// PyO3-based Python scripting engine
///
/// This engine wraps a Python interpreter and provides the `ScriptEngine`
/// interface for executing Python code. It maintains a global namespace
/// for variables and supports registering Rust functions as Python callables.
///
/// # Thread Safety
///
/// The engine uses Arc<Mutex<>> internally to ensure thread-safe access to
/// the Python interpreter, which is required by the `ScriptEngine` trait.
#[cfg(feature = "scripting_python")]
pub struct PyO3Engine {
    /// Global namespace for Python variables
    globals: Arc<Mutex<HashMap<String, Py<PyAny>>>>,
    /// Registered functions that can be called from Python
    functions: Arc<Mutex<HashMap<String, Py<PyAny>>>>,
}

#[cfg(feature = "scripting_python")]
impl PyO3Engine {
    /// Create a new PyO3Engine instance
    ///
    /// This initializes the Python interpreter if not already initialized
    /// and sets up the global namespace.
    ///
    /// # Errors
    ///
    /// Returns `ScriptError::BackendError` if Python initialization fails.
    pub fn new() -> Result<Self, ScriptError> {
        // PyO3 automatically initializes the Python interpreter with auto-initialize feature
        Ok(Self {
            globals: Arc::new(Mutex::new(HashMap::new())),
            functions: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Convert a PyO3 error to ScriptError
    fn convert_py_error(err: PyErr) -> ScriptError {
        Python::with_gil(|py| {
            let traceback = err
                .traceback_bound(py)
                .and_then(|tb| tb.format().ok())
                .unwrap_or_default();

            ScriptError::RuntimeError {
                message: err.to_string(),
                backtrace: if traceback.is_empty() {
                    None
                } else {
                    Some(traceback)
                },
            }
        })
    }

    /// Convert a ScriptValue to a Python object
    fn script_value_to_py(value: ScriptValue, py: Python) -> PyResult<Py<PyAny>> {
        // Try to downcast to common types
        if let Some(s) = value.downcast_ref::<String>() {
            return Ok(s.to_object(py));
        }
        if let Some(i) = value.downcast_ref::<i64>() {
            return Ok(i.to_object(py));
        }
        if let Some(f) = value.downcast_ref::<f64>() {
            return Ok(f.to_object(py));
        }
        if let Some(b) = value.downcast_ref::<bool>() {
            return Ok(b.to_object(py));
        }

        // If we can't convert, return None
        Ok(py.None())
    }

    /// Convert a Python object to ScriptValue
    fn py_to_script_value(obj: &Bound<'_, PyAny>) -> Result<ScriptValue, ScriptError> {
        // Try to extract common types
        if let Ok(s) = obj.extract::<String>() {
            return Ok(ScriptValue::new(s));
        }
        if let Ok(i) = obj.extract::<i64>() {
            return Ok(ScriptValue::new(i));
        }
        if let Ok(f) = obj.extract::<f64>() {
            return Ok(ScriptValue::new(f));
        }
        if let Ok(b) = obj.extract::<bool>() {
            return Ok(ScriptValue::new(b));
        }

        // If we can't convert, return an error
        Err(ScriptError::TypeConversionError {
            expected: "String, i64, f64, or bool".to_string(),
            found: obj
                .get_type()
                .name()
                .map(|s| s.to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
        })
    }
}

#[cfg(feature = "scripting_python")]
#[async_trait]
impl ScriptEngine for PyO3Engine {
    async fn execute_script(&mut self, script: &str) -> Result<ScriptValue, ScriptError> {
        let globals = self.globals.clone();
        let functions = self.functions.clone();
        let script = script.to_string();

        // Execute in a blocking task to avoid blocking the async runtime
        tokio::task::spawn_blocking(move || {
            Python::with_gil(|py| {
                // Create a new module for this script execution
                let module = PyModule::from_code_bound(py, &script, "script.py", "script")
                    .map_err(Self::convert_py_error)?;

                // Get the module's dict to access variables
                let module_dict = module.dict();

                // Inject global variables
                let globals_lock = globals.lock().unwrap();
                for (name, value) in globals_lock.iter() {
                    module_dict
                        .set_item(name, value.bind(py))
                        .map_err(Self::convert_py_error)?;
                }
                drop(globals_lock);

                // Inject registered functions
                let functions_lock = functions.lock().unwrap();
                for (name, func) in functions_lock.iter() {
                    module_dict
                        .set_item(name, func.bind(py))
                        .map_err(Self::convert_py_error)?;
                }
                drop(functions_lock);

                // Update globals with any new variables created in the script
                let mut globals_lock = globals.lock().unwrap();
                for (key, value) in module_dict.iter() {
                    if let Ok(key_str) = key.extract::<String>() {
                        // Skip built-in variables
                        if !key_str.starts_with("__") {
                            globals_lock.insert(key_str, value.unbind());
                        }
                    }
                }
                drop(globals_lock);

                // Try to get a return value (if the script has one)
                if let Ok(Some(result)) = module_dict.get_item("result") {
                    return Self::py_to_script_value(&result);
                }

                // If no explicit result, return None
                Ok(ScriptValue::new(()))
            })
        })
        .await
        .map_err(|e| ScriptError::AsyncError {
            message: format!("Task join error: {}", e),
        })?
    }

    async fn validate_script(&self, script: &str) -> Result<(), ScriptError> {
        let script = script.to_string();

        tokio::task::spawn_blocking(move || {
            Python::with_gil(|py| {
                // Try to compile the script
                py.run_bound(&script, None, None)
                    .map_err(Self::convert_py_error)?;
                Ok(())
            })
        })
        .await
        .map_err(|e| ScriptError::AsyncError {
            message: format!("Task join error: {}", e),
        })?
    }

    fn register_function(
        &mut self,
        name: &str,
        function: Box<dyn std::any::Any + Send + Sync>,
    ) -> Result<(), ScriptError> {
        // For PyO3, we expect the function to be a Py<PyAny> wrapped in Box<dyn Any>
        // This is a limitation of the generic interface - callers need to provide
        // Python-compatible functions when using PyO3Engine

        Python::with_gil(|py| {
            // Try to downcast to Py<PyAny>
            if let Some(py_func) = function.downcast_ref::<Py<PyAny>>() {
                let mut functions = self.functions.lock().unwrap();
                functions.insert(name.to_string(), py_func.clone_ref(py));
                Ok(())
            } else {
                Err(ScriptError::BackendError {
                    backend: "PyO3".to_string(),
                    message: format!("Function '{}' must be a Py<PyAny> for PyO3 engine", name),
                })
            }
        })
    }

    fn set_global(&mut self, name: &str, value: ScriptValue) -> Result<(), ScriptError> {
        Python::with_gil(|py| {
            let py_value = Self::script_value_to_py(value, py).map_err(Self::convert_py_error)?;

            let mut globals = self.globals.lock().unwrap();
            globals.insert(name.to_string(), py_value);
            Ok(())
        })
    }

    fn get_global(&self, name: &str) -> Result<ScriptValue, ScriptError> {
        Python::with_gil(|_py| {
            let globals = self.globals.lock().unwrap();

            if let Some(py_value) = globals.get(name) {
                Python::with_gil(|py| Self::py_to_script_value(&py_value.bind(py)))
            } else {
                Err(ScriptError::VariableNotFound {
                    name: name.to_string(),
                })
            }
        })
    }

    fn clear_globals(&mut self) {
        let mut globals = self.globals.lock().unwrap();
        globals.clear();

        let mut functions = self.functions.lock().unwrap();
        functions.clear();
    }

    fn backend_name(&self) -> &str {
        "PyO3 (Python)"
    }
}

// Stub implementation when scripting_python feature is disabled
#[cfg(not(feature = "scripting_python"))]
pub struct PyO3Engine;

#[cfg(not(feature = "scripting_python"))]
impl PyO3Engine {
    pub fn new() -> Result<Self, super::script_engine::ScriptError> {
        Err(super::script_engine::ScriptError::BackendError {
            backend: "PyO3".to_string(),
            message: "PyO3 engine requires 'scripting_python' feature to be enabled".to_string(),
        })
    }
}

#[cfg(test)]
#[cfg(feature = "scripting_python")]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pyo3_engine_creation() {
        let engine = PyO3Engine::new();
        assert!(engine.is_ok());
    }

    #[tokio::test]
    async fn test_pyo3_execute_simple_script() {
        let mut engine = PyO3Engine::new().unwrap();

        let script = r#"
x = 10
y = 20
result = x + y
"#;

        let exec_result = engine.execute_script(script).await;
        assert!(exec_result.is_ok());

        let result = engine.get_global("result").unwrap();
        let value: i64 = result.downcast().unwrap();
        assert_eq!(value, 30);
    }

    #[tokio::test]
    async fn test_pyo3_global_variables() {
        let mut engine = PyO3Engine::new().unwrap();

        engine
            .set_global("test_var", ScriptValue::new(42_i64))
            .unwrap();

        let script = "result = test_var * 2";
        engine.execute_script(script).await.unwrap();

        let result = engine.get_global("result").unwrap();
        let value: i64 = result.downcast().unwrap();
        assert_eq!(value, 84);
    }

    #[tokio::test]
    async fn test_pyo3_backend_name() {
        let engine = PyO3Engine::new().unwrap();
        assert_eq!(engine.backend_name(), "PyO3 (Python)");
    }

    #[tokio::test]
    async fn test_pyo3_clear_globals() {
        let mut engine = PyO3Engine::new().unwrap();

        engine
            .set_global("test", ScriptValue::new(123_i64))
            .unwrap();
        assert!(engine.get_global("test").is_ok());

        engine.clear_globals();
        assert!(engine.get_global("test").is_err());
    }

    #[tokio::test]
    async fn test_pyo3_validate_script() {
        let engine = PyO3Engine::new().unwrap();

        // Valid script
        assert!(engine.validate_script("x = 1 + 2").await.is_ok());

        // Invalid script
        assert!(engine.validate_script("x = 1 +").await.is_err());
    }
}
