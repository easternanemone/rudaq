//! Script-based module implementation.
//!
//! Wraps a script file and ScriptEngine to implement the Module trait.
//!
//! # Script Contract
//!
//! Scripts must define the following functions to work as modules:
//!
//! ```rhai
//! // Required: Module metadata (called once at load time)
//! fn module_type_info() {
//!     #{
//!         type_id: "my_script_module",
//!         display_name: "My Script Module",
//!         description: "A script-based module",
//!         version: "1.0.0",
//!         parameters: [],
//!         required_roles: [],
//!         optional_roles: [],
//!         event_types: [],
//!         data_types: []
//!     }
//! }
//!
//! // Optional: Configure with parameters
//! fn configure(params) {
//!     // Store params for later use
//!     []  // Return warnings as array
//! }
//!
//! // Optional: Get current configuration
//! fn get_config() {
//!     #{}  // Return config map
//! }
//!
//! // Optional: Prepare resources before start
//! fn stage(ctx) {
//!     // Allocate buffers, warm up hardware
//! }
//!
//! // Required: Main execution
//! fn start(ctx) {
//!     // Main module logic
//! }
//!
//! // Optional: Pause execution
//! fn pause() {}
//!
//! // Optional: Resume execution
//! fn resume() {}
//!
//! // Optional: Stop execution
//! fn stop() {}
//!
//! // Optional: Release resources after stop
//! fn unstage(ctx) {}
//! ```

use crate::modules::{Module, ModuleContext};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use common::modules::{ModuleParameter, ModuleRole, ModuleState, ModuleTypeInfo};
use daq_scripting::rhai::{Array, Dynamic, Map};
use daq_scripting::{RhaiEngine, ScriptEngine, ScriptError, ScriptValue};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

// =============================================================================
// Helper functions for extracting typed values from Rhai Dynamic
// =============================================================================

/// Get a string value from a Rhai map.
fn get_string(map: &Map, key: &str) -> Option<String> {
    map.get(key).and_then(|v| v.clone().into_string().ok())
}

/// Get a string value from a Rhai map with a default.
fn get_string_or(map: &Map, key: &str, default: &str) -> String {
    get_string(map, key).unwrap_or_else(|| default.to_string())
}

/// Get a bool value from a Rhai map.
fn get_bool(map: &Map, key: &str) -> Option<bool> {
    map.get(key).and_then(|v| v.as_bool().ok())
}

/// Get an array from a Rhai map.
fn get_array(map: &Map, key: &str) -> Option<Array> {
    map.get(key).and_then(|v| v.clone().try_cast::<Array>())
}

/// Convert a Dynamic to a Map if possible.
fn as_map(d: Dynamic) -> Option<Map> {
    d.try_cast::<Map>()
}

/// Convert a Dynamic to a String if possible.
fn as_string(d: Dynamic) -> Option<String> {
    d.into_string().ok()
}

/// Get a string array from a Rhai map.
fn get_string_array(map: &Map, key: &str) -> Vec<String> {
    get_array(map, key)
        .map(|arr| arr.into_iter().filter_map(as_string).collect())
        .unwrap_or_default()
}

// =============================================================================
// ScriptModule
// =============================================================================

/// Script-based module that wraps a ScriptEngine.
///
/// This module loads a script file and executes script functions
/// to implement the Module trait. Scripts can be written in any
/// language supported by a ScriptEngine implementation (Rhai, Python).
///
/// # Implementation Note
///
/// In Rhai, functions defined with `fn` in a script are only available
/// during that script's execution. To call script functions later, we
/// prepend the script source to each function call, effectively
/// re-evaluating the function definitions before each call.
pub struct ScriptModule {
    /// Path to the script file
    script_path: PathBuf,
    /// Script source code (prepended to each function call)
    script_source: String,
    /// Module type info extracted from script
    type_info: ModuleTypeInfo,
    /// Current module state
    state: ModuleState,
    /// Script engine instance
    engine: Arc<Mutex<RhaiEngine>>,
    /// Running flag
    running: Arc<AtomicBool>,
    /// Paused flag
    paused: Arc<AtomicBool>,
    /// Task handle for the running module
    task_handle: Option<tokio::task::JoinHandle<()>>,
    /// Current configuration
    config: HashMap<String, String>,
}

impl std::fmt::Debug for ScriptModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptModule")
            .field("script_path", &self.script_path)
            .field("type_info", &self.type_info.type_id)
            .field("state", &self.state)
            .field("running", &self.running.load(Ordering::Relaxed))
            .field("paused", &self.paused.load(Ordering::Relaxed))
            .finish()
    }
}

impl ScriptModule {
    /// Load a script module from a file path.
    ///
    /// This reads the script, initializes the engine, and extracts
    /// module metadata by calling `module_type_info()` in the script.
    pub async fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let script_source = std::fs::read_to_string(path)
            .map_err(|e| anyhow!("Failed to read script file {:?}: {}", path, e))?;

        Self::from_source(script_source, path.to_path_buf()).await
    }

    /// Create a script module from source code.
    ///
    /// The path is used for error reporting and caching.
    pub async fn from_source(script_source: String, script_path: PathBuf) -> Result<Self> {
        // Create engine with hardware support
        let mut engine = RhaiEngine::with_hardware()
            .map_err(|e| anyhow!("Failed to create script engine: {}", e))?;

        // In Rhai, functions defined with `fn` are only available during
        // the script's execution. So we need to combine the script with
        // the call to module_type_info() in a single execution.
        let combined_script = format!("{}\nmodule_type_info()", script_source);

        // Execute the combined script to get type info
        let result = engine
            .execute_script(&combined_script)
            .await
            .map_err(|e| anyhow!("Script must define module_type_info() function: {}", e))?;

        let type_info = Self::parse_type_info_from_dynamic(result)?;

        Ok(Self {
            script_path,
            script_source,
            type_info,
            state: ModuleState::Created,
            engine: Arc::new(Mutex::new(engine)),
            running: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
            task_handle: None,
            config: HashMap::new(),
        })
    }

    /// Execute a function from the script.
    ///
    /// This combines the script source with the function call, since
    /// Rhai functions are only available during script execution.
    async fn call_script_fn(
        engine: &mut RhaiEngine,
        script_source: &str,
        function_call: &str,
    ) -> Result<ScriptValue, ScriptError> {
        let combined = format!("{}\n{}", script_source, function_call);
        engine.execute_script(&combined).await
    }

    /// Parse ModuleTypeInfo from a script Dynamic value.
    fn parse_type_info_from_dynamic(value: ScriptValue) -> Result<ModuleTypeInfo> {
        // Try to extract as Dynamic
        let dynamic = value
            .downcast::<Dynamic>()
            .map_err(|_| anyhow!("module_type_info() must return a map"))?;

        let map = as_map(dynamic).ok_or_else(|| anyhow!("module_type_info() must return a map"))?;

        // Extract required fields
        let type_id = get_string(&map, "type_id")
            .ok_or_else(|| anyhow!("module_type_info() must have type_id field"))?;

        let display_name = get_string_or(&map, "display_name", &type_id);
        let description = get_string_or(&map, "description", "");
        let version = get_string_or(&map, "version", "1.0.0");

        // Parse parameters
        let parameters = Self::parse_parameters(&map);

        // Parse roles
        let required_roles = Self::parse_roles(&map, "required_roles");
        let optional_roles = Self::parse_roles(&map, "optional_roles");

        // Parse event/data types
        let event_types = get_string_array(&map, "event_types");
        let data_types = get_string_array(&map, "data_types");

        Ok(ModuleTypeInfo {
            type_id,
            display_name,
            description,
            version,
            parameters,
            required_roles,
            optional_roles,
            event_types,
            data_types,
        })
    }

    /// Parse parameters array from script map.
    fn parse_parameters(map: &Map) -> Vec<ModuleParameter> {
        let Some(params_arr) = get_array(map, "parameters") else {
            return vec![];
        };

        params_arr
            .into_iter()
            .filter_map(|p| {
                let param_map = as_map(p)?;

                Some(ModuleParameter {
                    param_id: get_string(&param_map, "param_id")?,
                    display_name: get_string_or(&param_map, "display_name", ""),
                    description: get_string_or(&param_map, "description", ""),
                    param_type: get_string_or(&param_map, "param_type", "string"),
                    default_value: get_string_or(&param_map, "default_value", ""),
                    min_value: get_string(&param_map, "min_value"),
                    max_value: get_string(&param_map, "max_value"),
                    enum_values: get_string_array(&param_map, "enum_values"),
                    units: get_string_or(&param_map, "units", ""),
                    required: get_bool(&param_map, "required").unwrap_or(false),
                })
            })
            .collect()
    }

    /// Parse roles array from script map.
    fn parse_roles(map: &Map, key: &str) -> Vec<ModuleRole> {
        let Some(roles_arr) = get_array(map, key) else {
            return vec![];
        };

        roles_arr
            .into_iter()
            .filter_map(|r| {
                let role_map = as_map(r)?;

                Some(ModuleRole {
                    role_id: get_string(&role_map, "role_id")?,
                    display_name: get_string_or(&role_map, "display_name", ""),
                    description: get_string_or(&role_map, "description", ""),
                    required_capability: get_string_or(&role_map, "required_capability", ""),
                    allows_multiple: get_bool(&role_map, "allows_multiple").unwrap_or(false),
                })
            })
            .collect()
    }

    /// Get the script file path.
    pub fn script_path(&self) -> &Path {
        &self.script_path
    }
}

#[async_trait]
impl Module for ScriptModule {
    fn type_info() -> ModuleTypeInfo
    where
        Self: Sized,
    {
        // This is a placeholder - actual type info comes from the script
        ModuleTypeInfo {
            type_id: "script_module".to_string(),
            display_name: "Script Module".to_string(),
            description: "A module defined by a script file".to_string(),
            version: "1.0.0".to_string(),
            parameters: vec![],
            required_roles: vec![],
            optional_roles: vec![],
            event_types: vec![],
            data_types: vec![],
        }
    }

    fn type_id(&self) -> &str {
        &self.type_info.type_id
    }

    fn configure(&mut self, params: HashMap<String, String>) -> Result<Vec<String>> {
        // Store config
        self.config = params.clone();

        // Convert params to Rhai map format
        let params_str = params
            .iter()
            .map(|(k, v)| format!("\"{}\": \"{}\"", k, v))
            .collect::<Vec<_>>()
            .join(", ");

        let function_call = format!("configure(#{{ {} }})", params_str);

        // Run configure in blocking context
        let engine = self.engine.clone();
        let script_source = self.script_source.clone();
        let warnings = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut engine = engine.lock().await;
                match Self::call_script_fn(&mut engine, &script_source, &function_call).await {
                    Ok(result) => {
                        // Try to extract warnings array
                        if let Ok(dynamic) = result.downcast::<Dynamic>() {
                            if let Some(arr) = dynamic.try_cast::<Array>() {
                                return arr
                                    .into_iter()
                                    .filter_map(as_string)
                                    .collect::<Vec<String>>();
                            }
                        }
                        vec![]
                    }
                    Err(e) => {
                        // configure() might not be defined, which is OK
                        if e.to_string().contains("not found") {
                            vec![]
                        } else {
                            warn!("Script configure() failed: {}", e);
                            vec![format!("Script error: {}", e)]
                        }
                    }
                }
            })
        });

        self.state = ModuleState::Configured;
        Ok(warnings)
    }

    fn get_config(&self) -> HashMap<String, String> {
        self.config.clone()
    }

    async fn stage(&mut self, ctx: &ModuleContext) -> Result<()> {
        let mut engine = self.engine.lock().await;
        let function_call = format!("stage(#{{ module_id: \"{}\" }})", ctx.module_id);

        // Call stage if it exists
        match Self::call_script_fn(&mut engine, &self.script_source, &function_call).await {
            Ok(_) => {
                self.state = ModuleState::Staged;
                Ok(())
            }
            Err(e) if e.to_string().contains("not found") => {
                // stage() is optional
                self.state = ModuleState::Staged;
                Ok(())
            }
            Err(e) => Err(anyhow!("Script stage() failed: {}", e)),
        }
    }

    async fn unstage(&mut self, ctx: &ModuleContext) -> Result<()> {
        let mut engine = self.engine.lock().await;
        let function_call = format!("unstage(#{{ module_id: \"{}\" }})", ctx.module_id);

        // Call unstage if it exists
        match Self::call_script_fn(&mut engine, &self.script_source, &function_call).await {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("not found") => {
                Ok(()) // unstage() is optional
            }
            Err(e) => Err(anyhow!("Script unstage() failed: {}", e)),
        }
    }

    async fn start(&mut self, ctx: ModuleContext) -> Result<()> {
        if self.state == ModuleState::Running {
            return Err(anyhow!("Module is already running"));
        }

        self.running.store(true, Ordering::SeqCst);
        self.paused.store(false, Ordering::SeqCst);
        self.state = ModuleState::Running;

        let engine = Arc::clone(&self.engine);
        let running = Arc::clone(&self.running);
        let script_source = self.script_source.clone();
        let module_id = ctx.module_id.clone();

        // Spawn the script execution task
        let handle = tokio::spawn(async move {
            let mut engine = engine.lock().await;
            let function_call =
                format!("start(#{{ module_id: \"{}\", running: true }})", module_id);

            // Call start function
            match Self::call_script_fn(&mut engine, &script_source, &function_call).await {
                Ok(_) => {
                    info!("Script module {} completed", module_id);
                }
                Err(e) => {
                    warn!("Script start() error: {}", e);
                }
            }

            running.store(false, Ordering::SeqCst);
        });

        self.task_handle = Some(handle);
        info!("ScriptModule started: {}", self.type_info.type_id);
        Ok(())
    }

    async fn pause(&mut self) -> Result<()> {
        if self.state != ModuleState::Running {
            return Err(anyhow!("Module is not running"));
        }

        self.paused.store(true, Ordering::SeqCst);

        // Try to call pause() in script
        let mut engine = self.engine.lock().await;
        let _ = Self::call_script_fn(&mut engine, &self.script_source, "pause()").await;

        self.state = ModuleState::Paused;
        info!("ScriptModule paused");
        Ok(())
    }

    async fn resume(&mut self) -> Result<()> {
        if self.state != ModuleState::Paused {
            return Err(anyhow!("Module is not paused"));
        }

        self.paused.store(false, Ordering::SeqCst);

        // Try to call resume() in script
        let mut engine = self.engine.lock().await;
        let _ = Self::call_script_fn(&mut engine, &self.script_source, "resume()").await;

        self.state = ModuleState::Running;
        info!("ScriptModule resumed");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        if self.state != ModuleState::Running && self.state != ModuleState::Paused {
            return Err(anyhow!("Module is not running"));
        }

        self.running.store(false, Ordering::SeqCst);

        // Try to call stop() in script
        {
            let mut engine = self.engine.lock().await;
            let _ = Self::call_script_fn(&mut engine, &self.script_source, "stop()").await;
        }

        // Wait for task to complete
        if let Some(handle) = self.task_handle.take() {
            tokio::time::timeout(std::time::Duration::from_secs(2), handle)
                .await
                .ok();
        }

        self.state = ModuleState::Stopped;
        info!("ScriptModule stopped");
        Ok(())
    }

    fn state(&self) -> ModuleState {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SCRIPT: &str = r#"
fn module_type_info() {
    #{
        type_id: "test_script_module",
        display_name: "Test Script Module",
        description: "A test module written in Rhai",
        version: "1.0.0",
        parameters: [
            #{
                param_id: "sample_rate",
                display_name: "Sample Rate",
                description: "Sampling rate in Hz",
                param_type: "float",
                default_value: "10.0",
                units: "Hz"
            }
        ],
        required_roles: [],
        optional_roles: [],
        event_types: ["test_event"],
        data_types: ["test_data"]
    }
}

fn configure(params) {
    []
}

fn start(ctx) {
    print("Test module started: " + ctx.module_id);
}
"#;

    #[tokio::test]
    async fn test_script_module_from_source() {
        let module = ScriptModule::from_source(TEST_SCRIPT.to_string(), PathBuf::from("test.rhai"))
            .await
            .unwrap();

        assert_eq!(module.type_id(), "test_script_module");
        assert_eq!(module.state(), ModuleState::Created);
    }

    #[tokio::test]
    async fn test_script_module_type_info_parsing() {
        let module = ScriptModule::from_source(TEST_SCRIPT.to_string(), PathBuf::from("test.rhai"))
            .await
            .unwrap();

        assert_eq!(module.type_info.type_id, "test_script_module");
        assert_eq!(module.type_info.display_name, "Test Script Module");
        assert_eq!(module.type_info.parameters.len(), 1);
        assert_eq!(module.type_info.parameters[0].param_id, "sample_rate");
        assert_eq!(module.type_info.event_types, vec!["test_event"]);
        assert_eq!(module.type_info.data_types, vec!["test_data"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_script_module_configure() {
        let mut module =
            ScriptModule::from_source(TEST_SCRIPT.to_string(), PathBuf::from("test.rhai"))
                .await
                .unwrap();

        let mut params = HashMap::new();
        params.insert("sample_rate".to_string(), "20.0".to_string());

        let warnings = module.configure(params.clone()).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(module.state(), ModuleState::Configured);
        assert_eq!(module.get_config(), params);
    }
}
