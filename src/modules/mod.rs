//! Module system for experiment logic and workflows.
//!
//! This module defines the `Module` trait, which wraps instruments to implement
//! specific experiment types and workflows. Modules represent high-level experiment
//! logic (e.g., "Laser Scanning Microscopy", "Power Calibration", "Spectrum Analysis")
//! that orchestrate one or more instruments to accomplish a scientific task.
//!
//! # Design Philosophy
//!
//! The module system is inspired by DynExp's module architecture:
//! - **Hardware Abstraction**: Modules interact with instruments through abstract interfaces,
//!   enabling hardware-independent experiment logic
//! - **Runtime Flexibility**: Instruments can be assigned/reassigned to modules at runtime
//! - **Type Safety**: Generic type parameters ensure modules only accept compatible instruments
//! - **Concurrent Execution**: Each module runs in its own Tokio task with isolated state
//! - **Message-Based Control**: Integration with the DaqCommand system for lifecycle management
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────┐
//! │  DaqManagerActor│
//! └────────┬────────┘
//!          │
//!          ├─── spawns ───> ┌──────────────┐
//!          │                 │ InstrumentActor │
//!          │                 └──────────────┘
//!          │                        ↓
//!          │                 broadcast::channel
//!          │                        ↓
//!          └─── spawns ───> ┌──────────────┐
//!                            │ ModuleActor   │ ← subscribes to instrument data
//!                            │  (implements  │ ← sends InstrumentCommand
//!                            │   Module)     │ ← runs experiment logic
//!                            └──────────────┘
//! ```
//!
//! # Type Safety
//!
//! Modules use generic type parameters to enforce instrument compatibility at compile time:
//!
//! ```rust
//! use rust_daq::modules::{Module, ModuleWithInstrument};
//! use rust_daq::measurement::Measure;
//! use rust_daq::core::Instrument;
//! # use std::sync::Arc;
//! # use anyhow::Result;
//!
//! // A camera module that only accepts instruments with camera measurements
//! struct CameraModule<M: Measure> {
//!     camera: Option<Arc<dyn Instrument<Measure = M>>>,
//! }
//!
//! # impl<M: Measure + 'static> Module for CameraModule<M> {
//! #     fn name(&self) -> &str { "camera" }
//! #     fn init(&mut self, _config: rust_daq::modules::ModuleConfig) -> Result<()> { Ok(()) }
//! #     fn status(&self) -> rust_daq::modules::ModuleStatus { rust_daq::modules::ModuleStatus::Idle }
//! # }
//! #
//! impl<M: Measure + 'static> ModuleWithInstrument<M> for CameraModule<M> {
//!     fn assign_instrument(
//!         &mut self,
//!         id: String,
//!         instrument: Arc<dyn Instrument<Measure = M>>
//!     ) -> Result<()> {
//!         self.camera = Some(instrument);
//!         Ok(())
//!     }
//!
//!     fn get_instrument(&self, _id: &str) -> Option<Arc<dyn Instrument<Measure = M>>> {
//!         self.camera.clone()
//!     }
//! }
//! ```
//!
//! # Lifecycle
//!
//! Modules follow a state machine pattern:
//!
//! ```text
//! Idle ──init()──> Initialized ──start()──> Running ──pause()──> Paused
//!                                              │                     │
//!                                              │                     │
//!                                              └──── stop() ─────────┘
//!                                                        │
//!                                                        ↓
//!                                                    Stopped
//! ```
//!
//! # Integration with DaqCommand
//!
//! Modules integrate with the existing message-passing system:
//!
//! ```rust,ignore
//! // Extend DaqCommand enum with module commands
//! pub enum DaqCommand {
//!     // ... existing commands
//!     SpawnModule {
//!         name: String,
//!         config: ModuleConfig,
//!         response: oneshot::Sender<Result<ModuleHandle>>,
//!     },
//!     AssignInstrumentToModule {
//!         module_id: String,
//!         instrument_id: String,
//!         response: oneshot::Sender<Result<()>>,
//!     },
//!     StartModule {
//!         id: String,
//!         response: oneshot::Sender<Result<()>>,
//!     },
//!     StopModule {
//!         id: String,
//!         response: oneshot::Sender<()>,
//!     },
//! }
//! ```
//!
//! # Examples
//!
//! ## Simple Power Meter Module
//!
//! ```rust
//! use rust_daq::modules::{Module, ModuleWithInstrument, ModuleConfig, ModuleStatus};
//! use rust_daq::measurement::Measure;
//! use rust_daq::core::Instrument;
//! use anyhow::Result;
//! use std::sync::Arc;
//!
//! /// A simple module that monitors laser power and triggers alerts
//! struct PowerMonitorModule<M: Measure> {
//!     power_meter: Option<Arc<dyn Instrument<Measure = M>>>,
//!     threshold: f64,
//!     status: ModuleStatus,
//! }
//!
//! impl<M: Measure + 'static> Module for PowerMonitorModule<M> {
//!     fn name(&self) -> &str {
//!         "power_monitor"
//!     }
//!
//!     fn init(&mut self, config: ModuleConfig) -> Result<()> {
//!         self.threshold = config.get("threshold")
//!             .and_then(|v| v.as_f64())
//!             .unwrap_or(100.0);
//!         self.status = ModuleStatus::Initialized;
//!         Ok(())
//!     }
//!
//!     fn status(&self) -> ModuleStatus {
//!         self.status
//!     }
//! }
//!
//! impl<M: Measure + 'static> ModuleWithInstrument<M> for PowerMonitorModule<M> {
//!     fn assign_instrument(
//!         &mut self,
//!         _id: String,
//!         instrument: Arc<dyn Instrument<Measure = M>>
//!     ) -> Result<()> {
//!         self.power_meter = Some(instrument);
//!         Ok(())
//!     }
//!
//!     fn get_instrument(&self, _id: &str) -> Option<Arc<dyn Instrument<Measure = M>>> {
//!         self.power_meter.clone()
//!     }
//! }
//! ```

use crate::core::Instrument;
use crate::measurement::Measure;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Configuration for a module instance.
///
/// Module configuration is typically loaded from TOML and contains
/// module-specific parameters (thresholds, calibration data, etc.).
///
/// # Examples
///
/// ```toml
/// [modules.power_monitor]
/// type = "power_monitor"
/// [modules.power_monitor.config]
/// threshold = 100.0
/// alert_email = "user@example.com"
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModuleConfig {
    /// Module-specific configuration parameters
    #[serde(flatten)]
    pub params: HashMap<String, serde_json::Value>,
}

impl ModuleConfig {
    /// Creates a new empty module configuration
    pub fn new() -> Self {
        Self {
            params: HashMap::new(),
        }
    }

    /// Gets a configuration parameter by key
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.params.get(key)
    }

    /// Sets a configuration parameter
    pub fn set(&mut self, key: String, value: serde_json::Value) {
        self.params.insert(key, value);
    }
}

impl Default for ModuleConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// The current status of a module.
///
/// Modules follow a state machine pattern for lifecycle management.
/// This enum represents the possible states.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModuleStatus {
    /// Module created but not initialized
    Idle,
    /// Module initialized and ready to start
    Initialized,
    /// Module is actively running experiment logic
    Running,
    /// Module execution is paused (can be resumed)
    Paused,
    /// Module has been stopped (requires restart to run again)
    Stopped,
    /// Module encountered an error during execution
    Error,
}

/// Base trait for all experiment modules.
///
/// The `Module` trait defines the core lifecycle and status interface that all
/// modules must implement. Modules represent high-level experiment workflows
/// that orchestrate instruments to accomplish scientific tasks.
///
/// # Lifecycle Methods
///
/// - **init()**: Initialize module state from configuration
/// - **status()**: Query current module state
///
/// For modules that need to control instruments, implement the additional
/// `ModuleWithInstrument` trait which provides type-safe instrument assignment.
///
/// # Thread Safety
///
/// Modules must be `Send + Sync` to enable:
/// - Transfer between async tasks (Send)
/// - Shared access via Arc (Sync)
///
/// # Examples
///
/// See module-level documentation for complete examples.
pub trait Module: Send + Sync {
    /// Returns the unique name/identifier for this module.
    ///
    /// This name is used for module registration and command routing.
    fn name(&self) -> &str;

    /// Initializes the module with the provided configuration.
    ///
    /// This method is called once after module creation and before starting.
    /// Use it to:
    /// - Parse and validate configuration parameters
    /// - Allocate resources
    /// - Set up initial state
    ///
    /// # Arguments
    ///
    /// * `config` - Module-specific configuration loaded from TOML or provided at runtime
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - Configuration is invalid or missing required parameters
    /// - Resource allocation fails
    /// - Module is already initialized
    ///
    /// # State Transition
    ///
    /// On success: Idle → Initialized
    fn init(&mut self, config: ModuleConfig) -> Result<()>;

    /// Returns the current status of the module.
    ///
    /// This method is used by the DAQ system to monitor module health
    /// and enforce state transitions.
    fn status(&self) -> ModuleStatus;
}

/// Extension trait for modules that control instruments.
///
/// `ModuleWithInstrument` provides type-safe instrument assignment and access.
/// The generic parameter `M` ensures that only compatible instruments
/// (those implementing `Instrument<Measure = M>`) can be assigned to the module.
///
/// # Type Safety
///
/// Using generic type parameters enforces compile-time safety:
///
/// ```rust,ignore
/// // This compiles: camera instrument → camera module
/// camera_module.assign_instrument("cam1", camera_instrument)?;
///
/// // This FAILS to compile: motion controller → camera module (type mismatch)
/// camera_module.assign_instrument("stage", motion_controller)?;
/// ```
///
/// # Runtime Assignment
///
/// Instruments can be assigned and reassigned at runtime without recompiling:
///
/// ```rust,ignore
/// // Initial assignment
/// module.assign_instrument("laser1", laser_a)?;
///
/// // Later, switch to a different laser
/// module.assign_instrument("laser1", laser_b)?;
/// ```
///
/// # Multiple Instruments
///
/// Modules that need multiple instruments should track them by ID:
///
/// ```rust,ignore
/// struct ScanningModule {
///     instruments: HashMap<String, Arc<dyn Instrument<Measure = M>>>,
/// }
///
/// impl ModuleWithInstrument<M> for ScanningModule {
///     fn assign_instrument(&mut self, id: String, instrument: Arc<dyn Instrument<Measure = M>>) -> Result<()> {
///         self.instruments.insert(id, instrument);
///         Ok(())
///     }
///
///     fn get_instrument(&self, id: &str) -> Option<Arc<dyn Instrument<Measure = M>>> {
///         self.instruments.get(id).cloned()
///     }
/// }
/// ```
pub trait ModuleWithInstrument<M: Measure + 'static>: Module {
    /// Assigns an instrument to this module.
    ///
    /// The instrument is identified by a unique ID (e.g., "laser", "camera", "stage")
    /// and can be accessed later via `get_instrument()`.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for this instrument within the module context
    /// * `instrument` - Arc-wrapped instrument implementing the required Measure type
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - Module is in Running state (cannot reassign while active)
    /// - Instrument validation fails
    /// - ID conflicts with existing assignment
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // Assign laser to laser control module
    /// module.assign_instrument("main_laser", laser_instrument)?;
    ///
    /// // Assign multiple instruments to scanning module
    /// scan_module.assign_instrument("laser", laser)?;
    /// scan_module.assign_instrument("stage_x", stage_x)?;
    /// scan_module.assign_instrument("stage_y", stage_y)?;
    /// ```
    fn assign_instrument(
        &mut self,
        id: String,
        instrument: Arc<dyn Instrument<Measure = M>>,
    ) -> Result<()>;

    /// Gets an assigned instrument by ID.
    ///
    /// Returns `None` if no instrument with the given ID has been assigned.
    ///
    /// # Arguments
    ///
    /// * `id` - The identifier used when the instrument was assigned
    ///
    /// # Returns
    ///
    /// - `Some(instrument)` if found
    /// - `None` if not assigned or ID is invalid
    fn get_instrument(&self, id: &str) -> Option<Arc<dyn Instrument<Measure = M>>>;

    /// Lists all assigned instrument IDs.
    ///
    /// Default implementation returns empty vector; override if module
    /// tracks multiple instruments.
    fn list_instruments(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Registry for module factory functions.
///
/// Similar to `InstrumentRegistry`, this enables runtime module creation
/// from configuration without hardcoding module types.
///
/// # Examples
///
/// ```rust,ignore
/// let mut registry = ModuleRegistry::new();
///
/// // Register module factories
/// registry.register("power_monitor", |name| {
///     Box::new(PowerMonitorModule::new(name))
/// });
///
/// registry.register("scanning", |name| {
///     Box::new(ScanningModule::new(name))
/// });
///
/// // Create modules from config
/// let module = registry.create("power_monitor", "pm1")?;
/// ```
pub struct ModuleRegistry<M: Measure + 'static> {
    factories: HashMap<String, Box<dyn Fn(String) -> Box<dyn Module> + Send + Sync>>,
    _phantom: std::marker::PhantomData<M>,
}

impl<M: Measure + 'static> ModuleRegistry<M> {
    /// Creates a new empty module registry
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Registers a module factory function.
    ///
    /// # Arguments
    ///
    /// * `module_type` - Type identifier used in configuration (e.g., "power_monitor")
    /// * `factory` - Function that creates a new module instance given a name
    pub fn register<F>(&mut self, module_type: &str, factory: F)
    where
        F: Fn(String) -> Box<dyn Module> + Send + Sync + 'static,
    {
        self.factories
            .insert(module_type.to_string(), Box::new(factory));
    }

    /// Creates a module instance from a registered type.
    ///
    /// # Arguments
    ///
    /// * `module_type` - The type identifier used during registration
    /// * `name` - Unique name for this module instance
    ///
    /// # Errors
    ///
    /// Returns `Err` if the module type is not registered.
    pub fn create(&self, module_type: &str, name: String) -> Result<Box<dyn Module>> {
        let factory = self
            .factories
            .get(module_type)
            .ok_or_else(|| anyhow::anyhow!("Unknown module type: {}", module_type))?;
        Ok(factory(name))
    }

    /// Lists all registered module types
    pub fn list_types(&self) -> Vec<String> {
        self.factories.keys().cloned().collect()
    }
}

impl<M: Measure + 'static> Default for ModuleRegistry<M> {
    fn default() -> Self {
        Self::new()
    }
}

// Concrete module implementations
pub mod power_meter;

#[cfg(test)]
mod tests {
    use super::*;

    // Mock measure type for testing
    #[derive(Clone)]
    struct MockMeasure;

    impl Measure for MockMeasure {
        type Data = f64;
        fn unit() -> &'static str {
            "V"
        }
    }

    // Mock module for testing
    struct TestModule {
        name: String,
        status: ModuleStatus,
    }

    impl Module for TestModule {
        fn name(&self) -> &str {
            &self.name
        }

        fn init(&mut self, _config: ModuleConfig) -> Result<()> {
            self.status = ModuleStatus::Initialized;
            Ok(())
        }

        fn status(&self) -> ModuleStatus {
            self.status
        }
    }

    #[test]
    fn test_module_config() {
        let mut config = ModuleConfig::new();
        config.set("threshold".to_string(), serde_json::json!(42.0));

        assert_eq!(config.get("threshold").unwrap().as_f64(), Some(42.0));
        assert!(config.get("nonexistent").is_none());
    }

    #[test]
    fn test_module_status() {
        let mut module = TestModule {
            name: "test".to_string(),
            status: ModuleStatus::Idle,
        };

        assert_eq!(module.status(), ModuleStatus::Idle);

        module.init(ModuleConfig::new()).unwrap();
        assert_eq!(module.status(), ModuleStatus::Initialized);
    }

    #[test]
    fn test_module_registry() {
        let mut registry: ModuleRegistry<MockMeasure> = ModuleRegistry::new();

        registry.register("test", |name| {
            Box::new(TestModule {
                name,
                status: ModuleStatus::Idle,
            })
        });

        let types = registry.list_types();
        assert!(types.contains(&"test".to_string()));

        let module = registry.create("test", "test_instance".to_string()).unwrap();
        assert_eq!(module.name(), "test_instance");

        // Unknown type should error
        assert!(registry.create("unknown", "foo".to_string()).is_err());
    }
}
