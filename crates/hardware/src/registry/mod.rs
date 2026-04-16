//! Device Registry for Runtime Hardware Management
//!
//! This module provides a central registry for discovering, registering, and managing
//! hardware devices at runtime. It follows patterns from PyMoDAQ and DynExp frameworks:
//!
//! - **Device Trait**: Wraps hardware drivers with metadata and capability introspection
//! - **DeviceRegistry**: Central hub for device lifecycle management
//! - **Capability Introspection**: Runtime discovery of device capabilities
//!
//! # Architecture (DynExp-style three-tier)
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────┐
//! │                       DeviceRegistry                             │
//! │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐          │
//! │  │ UniversalDev  │  │ PvcamCamera  │  │ MockDevice   │  ...    │
//! │  └──────────────┘  └──────────────┘  └──────────────┘          │
//! ├──────────────────────────────────────────────────────────────────┤
//! │                     Capability Traits                            │
//! │  Movable | Readable | Triggerable | FrameProducer | ...         │
//! ├──────────────────────────────────────────────────────────────────┤
//! │                     Hardware Drivers                             │
//! │  driver-universal (TOML) | driver-pvcam | driver-andor-sdk3     │
//! │  driver-comedi | driver-mock                                     │
//! └──────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Instruments
//!
//! Serial/TCP/SCPI devices are defined as TOML manifests in `config/devices/`
//! and loaded by `driver-universal`. Native SDK drivers (PVCAM, Andor, Comedi)
//! use dedicated crates with FFI bindings.
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use hardware::registry::{DeviceRegistry, DeviceConfig, DriverConfig};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut registry = DeviceRegistry::new();
//!
//!     // Register devices from TOML manifests via driver-universal
//!     // Serial/TCP/SCPI devices are defined in config/devices/*.toml
//!     registry.register_from_toml(
//!         "pid_controller",
//!         "Red Pitaya PID",
//!         "universal_red_pitaya_pid",
//!         toml::toml! { host = "192.168.1.100"; port = 5000 }.into(),
//!     ).await?;
//!
//!     // List all devices
//!     for info in registry.list_devices() {
//!         println!("{}: {} ({:?})", info.id, info.name, info.capabilities);
//!     }
//!
//!     // Get device by capability
//!     if let Some(device) = registry.get_movable("rotator_2") {
//!         device.move_abs(45.0).await?;
//!     }
//!
//!     Ok(())
//! }
//! ```

use common::capabilities::{
    CapabilityProvider, Commandable, CounterConfigurable, DeviceIntrospection, EmissionControl,
    ExposureControl, FrameProducer, GatedCamera, Movable, Parameterized, RangeIntrospectable,
    Readable, ReadableWithMetadata, Reconfigurable, Settable, ShutterControl, SpectrometerControl,
    SpectrumReadable, Stageable, StateRefreshable, Triggerable, WavelengthTunable,
};
use common::data::Frame;
use common::driver::{Capability, DeviceComponents, DeviceLifecycle, DriverFactory};
use common::error::DaqError;
use common::observable::ParameterMetadata;
use common::pipeline::MeasurementSource;

#[cfg(feature = "serial")]
use crate::manifest_driver::driver::GenericDriver;
use common::health::{DeviceHealth, DeviceHealthState};
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
#[cfg(feature = "serial")]
use tokio::sync::RwLock;
use tokio::sync::broadcast;

// =============================================================================
// Device Identification
// =============================================================================

/// Re-export the canonical `DeviceId` from `common-traits`.
///
/// `DeviceId` is an `Arc<str>`-backed newtype with cheap cloning (ref-count
/// bump, not heap copy). Implements `Deref<Target = str>` so `&DeviceId`
/// auto-coerces to `&str` — all existing APIs accepting `&str` work
/// transparently with `&DeviceId`.
pub use common::device_id::DeviceId;

mod types;
pub use types::*;

// =============================================================================
// Registered Device (Internal)
// =============================================================================

/// A registered device with its driver instance and metadata
struct RegisteredDevice {
    /// Device configuration
    config: DeviceConfig,
    /// Actual driver type string (preserves factory-registered type)
    driver_type: String,
    /// Movable implementation (if supported)
    movable: Option<Arc<dyn Movable>>,
    /// Readable implementation (if supported)
    readable: Option<Arc<dyn Readable>>,
    /// Triggerable implementation (if supported)
    triggerable: Option<Arc<dyn Triggerable>>,
    /// FrameProducer implementation (if supported)
    frame_producer: Option<Arc<dyn FrameProducer>>,
    /// MeasurementSource implementation (if supported)
    source_frame: Option<Arc<dyn MeasurementSource<Output = Arc<Frame>, Error = anyhow::Error>>>,
    /// ExposureControl implementation (if supported)
    exposure_control: Option<Arc<dyn ExposureControl>>,
    /// Settable implementation (if supported) - observable parameters
    settable: Option<Arc<dyn Settable>>,
    /// Stageable implementation (if supported) - Bluesky-style lifecycle (bd-7aq6)
    stageable: Option<Arc<dyn Stageable>>,
    /// Commandable implementation (if supported) - structured device commands
    commandable: Option<Arc<dyn Commandable>>,
    /// Parameterized implementation (if supported) - parameter registry access
    ///
    /// Enables generic code to enumerate and subscribe to device parameters.
    /// Populated during device registration if driver implements Parameterized trait.
    parameterized: Option<Arc<dyn Parameterized>>,
    /// Cached parameter metadata for fast validation (bd-izdj.3)
    parameter_metadata: HashMap<String, ParameterMetadata>,
    /// ShutterControl implementation (if supported) - laser shutter
    shutter_control: Option<Arc<dyn ShutterControl>>,
    /// EmissionControl implementation (if supported) - laser on/off
    emission_control: Option<Arc<dyn EmissionControl>>,
    /// WavelengthTunable implementation (if supported) - tunable laser wavelength (bd-pwjo)
    wavelength_tunable: Option<Arc<dyn WavelengthTunable>>,
    /// GatedCamera implementation (if supported) - ICCD/DDG control.
    gated_camera: Option<Arc<dyn GatedCamera>>,
    /// SpectrometerControl implementation (if supported) - grating/wavelength control.
    spectrometer_control: Option<Arc<dyn SpectrometerControl>>,
    /// Reconfigurable implementation (if supported) - runtime config changes
    reconfigurable: Option<Arc<dyn Reconfigurable>>,
    /// StateRefreshable implementation (if supported) - post-reconnection refresh (bd-47p2)
    state_refreshable: Option<Arc<dyn StateRefreshable>>,
    /// CounterConfigurable implementation (if supported) - DAQ counter/timer config (bd-f3pq)
    counter_configurable: Option<Arc<dyn CounterConfigurable>>,
    /// RangeIntrospectable implementation (if supported) - analog range queries (bd-3bjp)
    range_introspectable: Option<Arc<dyn RangeIntrospectable>>,
    /// DeviceIntrospection implementation (if supported) - board/subdevice metadata (bd-sa9p)
    device_introspection: Option<Arc<dyn DeviceIntrospection>>,
    /// ReadableWithMetadata implementation (if supported) - structured analog reads (bd-09ls)
    readable_with_metadata: Option<Arc<dyn ReadableWithMetadata>>,
    /// SpectrumReadable implementation (if supported) - 1D detector/spectrum data (bd-lncj.1.2)
    spectrum_readable: Option<Arc<dyn SpectrumReadable>>,
    /// Optional lifecycle hooks for registration/shutdown
    lifecycle: Option<Arc<dyn DeviceLifecycle>>,
    /// Device metadata (units, ranges, etc.)
    metadata: DeviceMetadata,
    /// Hash of the config used to create this device (for change detection)
    config_hash: u64,
}

impl RegisteredDevice {
    fn build_parameter_metadata(
        parameterized: Option<&Arc<dyn Parameterized>>,
    ) -> HashMap<String, ParameterMetadata> {
        let Some(parameterized) = parameterized else {
            return HashMap::new();
        };

        let mut metadata = HashMap::new();
        for name in parameterized.parameters().names() {
            if let Some(param) = parameterized.parameters().get(name) {
                metadata.insert(name.to_string(), param.metadata());
            }
        }
        metadata
    }

    /// Compute capabilities from the actual registered trait objects.
    ///
    /// This introspects which trait implementations are present rather than
    /// relying on static metadata.
    fn capabilities(&self) -> Vec<Capability> {
        let mut caps = Vec::new();

        if self.movable.is_some() {
            caps.push(Capability::Movable);
        }
        if self.readable.is_some() {
            caps.push(Capability::Readable);
        }
        if self.triggerable.is_some() {
            caps.push(Capability::Triggerable);
        }
        if self.frame_producer.is_some() {
            caps.push(Capability::FrameProducer);
        }
        if self.exposure_control.is_some() {
            caps.push(Capability::ExposureControl);
        }
        if self.settable.is_some() {
            caps.push(Capability::Settable);
        }
        if self.parameterized.is_some() {
            caps.push(Capability::Parameterized);
        }
        if self.commandable.is_some() {
            caps.push(Capability::Commandable);
        }
        if self.shutter_control.is_some() {
            caps.push(Capability::ShutterControl);
        }
        if self.emission_control.is_some() {
            caps.push(Capability::EmissionControl);
        }
        if self.wavelength_tunable.is_some() {
            caps.push(Capability::WavelengthTunable);
        }
        if self.gated_camera.is_some() {
            caps.push(Capability::GatedCamera);
        }
        if self.spectrometer_control.is_some() {
            caps.push(Capability::SpectrometerControl);
        }
        if self.reconfigurable.is_some() {
            caps.push(Capability::Reconfigurable);
        }
        if self.state_refreshable.is_some() {
            caps.push(Capability::StateRefreshable);
        }
        if self.counter_configurable.is_some() {
            caps.push(Capability::CounterConfigurable);
        }
        if self.range_introspectable.is_some() {
            caps.push(Capability::RangeIntrospectable);
        }
        if self.device_introspection.is_some() {
            caps.push(Capability::DeviceIntrospection);
        }
        if self.readable_with_metadata.is_some() {
            caps.push(Capability::ReadableWithMetadata);
        }
        if self.spectrum_readable.is_some() {
            caps.push(Capability::SpectrumReadable);
        }

        caps
    }
}

// =============================================================================
// Device Registry
// =============================================================================

/// Central registry for hardware device management.
///
/// The `DeviceRegistry` is the primary interface for:
/// - Registering devices from configuration
/// - Discovering connected devices
/// - Accessing devices by capability
/// - Querying device information
///
/// # Shared-Ownership Clone (M-SERVICES-CLONE)
///
/// `DeviceRegistry` wraps its internal state in an `Arc`, so cloning is cheap
/// (just an `Arc::clone`) and all clones share the same underlying data.
/// This means callers can pass `DeviceRegistry` by value instead of
/// `Arc<DeviceRegistry>`.
///
/// # Thread Safety
///
/// Internally thread-safe using `DashMap` for the devices collection.
/// This eliminates the need for external `RwLock` wrapping and allows concurrent
/// access to different devices without global lock contention.
///
/// # Plugin Architecture (DriverFactory)
///
/// The registry supports dynamic driver registration via the [`DriverFactory`] trait.
/// Driver crates can register their factories at startup, which are then used to
/// instantiate devices based on TOML configuration.
///
/// ```rust,ignore
/// // In main.rs or setup code:
/// registry.register_factory(Box::new(MyDriverFactory));
///
/// // Later, devices with driver_type matching the factory are auto-instantiated:
/// registry.register_from_config(config).await?;
/// ```
#[derive(Clone)]
pub struct DeviceRegistry(Arc<DeviceRegistryInner>);

impl DeviceRegistry {
    /// Create a new empty device registry.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(DeviceRegistryInner::new()))
    }

    /// Create a new device registry with a pre-configured `PluginFactory`.
    #[cfg(feature = "serial")]
    #[must_use]
    pub fn with_plugin_factory(
        plugin_factory: Arc<RwLock<crate::manifest_driver::registry::PluginFactory>>,
    ) -> Self {
        Self(Arc::new(DeviceRegistryInner::with_plugin_factory(
            plugin_factory,
        )))
    }
}

impl Default for DeviceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::ops::Deref for DeviceRegistry {
    type Target = DeviceRegistryInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl CapabilityProvider for DeviceRegistry {
    fn get_movable(&self, id: &str) -> Option<Arc<dyn Movable>> {
        self.0.get_movable(id)
    }

    fn get_readable(&self, id: &str) -> Option<Arc<dyn Readable>> {
        self.0.get_readable(id)
    }

    fn get_triggerable(&self, id: &str) -> Option<Arc<dyn Triggerable>> {
        self.0.get_triggerable(id)
    }

    fn get_frame_producer(&self, id: &str) -> Option<Arc<dyn FrameProducer>> {
        self.0.get_frame_producer(id)
    }

    fn get_exposure_control(&self, id: &str) -> Option<Arc<dyn ExposureControl>> {
        self.0.get_exposure_control(id)
    }

    fn get_shutter_control(&self, id: &str) -> Option<Arc<dyn ShutterControl>> {
        self.0.get_shutter_control(id)
    }

    fn get_wavelength_tunable(&self, id: &str) -> Option<Arc<dyn WavelengthTunable>> {
        self.0.get_wavelength_tunable(id)
    }

    fn get_emission_control(&self, id: &str) -> Option<Arc<dyn EmissionControl>> {
        self.0.get_emission_control(id)
    }

    fn get_settable(&self, id: &str) -> Option<Arc<dyn Settable>> {
        self.0.get_settable(id)
    }

    fn get_counter_configurable(&self, id: &str) -> Option<Arc<dyn CounterConfigurable>> {
        self.0.get_counter_configurable(id)
    }

    fn get_range_introspectable(&self, id: &str) -> Option<Arc<dyn RangeIntrospectable>> {
        self.0.get_range_introspectable(id)
    }

    fn get_device_introspection(&self, id: &str) -> Option<Arc<dyn DeviceIntrospection>> {
        self.0.get_device_introspection(id)
    }

    fn get_readable_with_metadata(&self, id: &str) -> Option<Arc<dyn ReadableWithMetadata>> {
        self.0.get_readable_with_metadata(id)
    }

    fn get_spectrum_readable(&self, id: &str) -> Option<Arc<dyn SpectrumReadable>> {
        self.0.get_spectrum_readable(id)
    }
}

impl std::fmt::Debug for DeviceRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceRegistry")
            .field("device_count", &self.0.len())
            .field("factory_count", &self.0.list_factories().len())
            .finish()
    }
}

/// Inner storage for the device registry.
///
/// This type is not re-exported from the crate root. Callers interact with
/// [`DeviceRegistry`], which wraps this in an `Arc` for cheap, shared-ownership
/// `Clone`. Methods are accessible via `Deref`.
pub struct DeviceRegistryInner {
    /// Registered devices by ID (thread-safe via DashMap)
    devices: DashMap<DeviceId, RegisteredDevice>,

    /// Registered driver factories by driver_type (thread-safe via DashMap)
    ///
    /// All device registration goes through factories. The factory matching
    /// the driver_type is used to validate config and build the device.
    factories: DashMap<String, Box<dyn DriverFactory>>,

    /// Plugin factory for loading YAML-defined drivers (serial feature only)
    #[cfg(feature = "serial")]
    plugin_factory: Arc<RwLock<crate::manifest_driver::registry::PluginFactory>>,

    /// Registration failures for debugging (device_id, driver_type, error_message)
    registration_failures: DashMap<DeviceId, RegistrationFailure>,

    /// Per-device health tracking for supervisor (bd-qa36.4.2)
    device_health: DashMap<DeviceId, DeviceHealthState>,

    /// Broadcast channel for health state change notifications (bd-vgrj).
    health_broadcast: broadcast::Sender<DeviceHealthEvent>,

    /// Consecutive failures before transitioning to Faulted (default: 3).
    /// Configurable via [`set_fault_threshold`](DeviceRegistry::set_fault_threshold).
    fault_threshold: AtomicU32,

    /// Per-device measurement lock state for safe reconfiguration.
    ///
    /// Drivers call `set_measurement_lock` when starting/stopping measurements.
    /// The reconciler checks this before calling `reconfigure()` to avoid
    /// changing hardware config mid-measurement (safety-critical for lasers).
    measurement_locks: DashMap<DeviceId, common::capabilities::MeasurementLock>,
}

impl DeviceRegistryInner {
    async fn run_on_register(
        &self,
        device_id: &str,
        driver_type: &str,
        lifecycle: Option<&Arc<dyn DeviceLifecycle>>,
    ) -> Result<(), DaqError> {
        if let Some(hook) = lifecycle {
            hook.on_register().await.map_err(|e| {
                DaqError::Driver(common::error::DriverError::new(
                    driver_type,
                    common::error::DriverErrorKind::Initialization,
                    format!("Lifecycle on_register failed for device '{device_id}': {e}"),
                ))
            })?;
        }
        Ok(())
    }

    async fn run_on_unregister(
        &self,
        device_id: &str,
        driver_type: &str,
        lifecycle: Option<&Arc<dyn DeviceLifecycle>>,
    ) -> Result<(), DaqError> {
        if let Some(hook) = lifecycle {
            hook.on_unregister().await.map_err(|e| {
                DaqError::Driver(common::error::DriverError::new(
                    driver_type,
                    common::error::DriverErrorKind::Shutdown,
                    format!("Lifecycle on_unregister failed for device '{device_id}': {e}"),
                ))
            })?;
        }
        Ok(())
    }
    /// Create a new empty device registry inner.
    fn new() -> Self {
        let (health_tx, _) = broadcast::channel(64);
        Self {
            devices: DashMap::new(),
            factories: DashMap::new(),
            #[cfg(feature = "serial")]
            plugin_factory: Arc::new(RwLock::new(
                crate::manifest_driver::registry::PluginFactory::new(),
            )),
            registration_failures: DashMap::new(),
            device_health: DashMap::new(),
            health_broadcast: health_tx,
            fault_threshold: AtomicU32::new(3),
            measurement_locks: DashMap::new(),
        }
    }

    /// Create a new device registry with a pre-configured PluginFactory.
    #[cfg(feature = "serial")]
    fn with_plugin_factory(
        plugin_factory: Arc<RwLock<crate::manifest_driver::registry::PluginFactory>>,
    ) -> Self {
        let (health_tx, _) = broadcast::channel(64);
        Self {
            devices: DashMap::new(),
            factories: DashMap::new(),
            plugin_factory,
            registration_failures: DashMap::new(),
            device_health: DashMap::new(),
            health_broadcast: health_tx,
            fault_threshold: AtomicU32::new(3),
            measurement_locks: DashMap::new(),
        }
    }

    /// Get a reference to the plugin factory
    #[cfg(feature = "serial")]
    pub fn plugin_factory(&self) -> Arc<RwLock<crate::manifest_driver::registry::PluginFactory>> {
        self.plugin_factory.clone()
    }

    /// Load plugins from a directory
    ///
    /// Scans the directory for YAML plugin files and loads them into the factory.
    ///
    /// # Arguments
    /// * `path` - Path to directory containing .yaml/.yml plugin files
    ///
    /// # Errors
    /// Returns error if path is not a directory or if any plugin fails to load
    #[cfg(feature = "serial")]
    pub async fn load_plugins(&self, path: &std::path::Path) -> Result<(), DaqError> {
        let mut factory = self.plugin_factory.write().await;
        factory
            .load_plugins(path)
            .await
            .map_err(|e| DaqError::Configuration(e.to_string()))
    }

    // =========================================================================
    // Driver Factory Management
    // =========================================================================

    /// Register a driver factory for a specific driver type.
    ///
    /// When a device with matching driver_type is registered, this factory's
    /// `build()` method will be called to construct the device.
    ///
    /// # Arguments
    /// * `factory` - The factory to register
    ///
    /// # Returns
    /// The previous factory for this driver_type, if any was registered.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use common::driver::DriverFactory;
    ///
    /// struct MyMotorFactory;
    /// impl DriverFactory for MyMotorFactory {
    ///     fn driver_type(&self) -> &'static str { "my_motor" }
    ///     // ... other methods
    /// }
    ///
    /// registry.register_factory(Box::new(MyMotorFactory));
    /// ```
    ///
    /// # Thread Safety
    /// This method is thread-safe and can be called concurrently.
    pub fn register_factory(
        &self,
        factory: Box<dyn DriverFactory>,
    ) -> Option<Box<dyn DriverFactory>> {
        let driver_type = factory.driver_type().to_string();
        let api_version = factory.api_version();
        let current = common::driver::DRIVER_FACTORY_API_VERSION;

        if api_version != current {
            tracing::warn!(
                driver_type = %driver_type,
                factory_api_version = api_version,
                current_api_version = current,
                "Driver factory built against different API version"
            );
        }

        tracing::info!(
            driver_type = %driver_type,
            name = %factory.name(),
            capabilities = ?factory.capabilities(),
            api_version,
            "Registering driver factory"
        );
        self.factories.insert(driver_type, factory)
    }

    /// Unregister a driver factory by driver type.
    ///
    /// # Arguments
    /// * `driver_type` - The driver type string to unregister
    ///
    /// # Returns
    /// The removed factory, if one was registered with this driver_type.
    pub fn unregister_factory(&self, driver_type: &str) -> Option<Box<dyn DriverFactory>> {
        self.factories
            .remove(driver_type)
            .map(|(_, factory)| factory)
    }

    /// Check if a factory is registered for a driver type.
    pub fn has_factory(&self, driver_type: &str) -> bool {
        self.factories.contains_key(driver_type)
    }

    /// List all registered factory driver types.
    pub fn list_factories(&self) -> Vec<String> {
        self.factories
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Get factory information for debugging/introspection.
    pub fn factory_info(&self, driver_type: &str) -> Option<FactoryInfo> {
        self.factories.get(driver_type).map(|entry| {
            let factory = entry.value();
            FactoryInfo {
                driver_type: factory.driver_type().to_string(),
                name: factory.name().to_string(),
                capabilities: factory
                    .capabilities()
                    .iter()
                    .map(|c| c.as_str().to_string())
                    .collect(),
                available_commands: factory.available_commands(),
            }
        })
    }

    /// Register a device from TOML configuration using registered factories.
    ///
    /// This method is the preferred way to register devices when using the
    /// DriverFactory plugin architecture. It:
    /// 1. Looks up a factory matching the driver_type in the TOML config
    /// 2. Validates the config using the factory's validate() method
    /// 3. Builds the device using the factory's build() method
    /// 4. Registers the resulting DeviceComponents
    ///
    /// # Arguments
    /// * `device_id` - Unique identifier for the device
    /// * `device_name` - Human-readable name for the device
    /// * `driver_type` - The driver type string (must match a registered factory)
    /// * `config` - TOML configuration value for the driver
    ///
    /// # Errors
    /// Returns error if:
    /// - Device ID is already registered
    /// - No factory is registered for the driver_type
    /// - Configuration validation fails
    /// - Driver build fails
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use toml::Value;
    ///
    /// let config: Value = toml::from_str(r#"
    ///     port = "/dev/ttyUSB0"
    ///     address = "2"
    /// "#)?;
    ///
    /// registry.register_from_toml(
    ///     "rotator_2",
    ///     "ELL14 Rotator #2",
    ///     "ell14",
    ///     config,
    /// ).await?;
    /// ```
    pub async fn register_from_toml(
        &self,
        device_id: &str,
        device_name: &str,
        driver_type: &str,
        config: toml::Value,
    ) -> Result<(), DaqError> {
        if self.devices.contains_key(device_id) {
            return Err(DaqError::Configuration(format!(
                "Device '{device_id}' is already registered"
            )));
        }

        // Look up factory — first try an exact match on driver_type. If the user
        // wrote `type = "universal"` with a `manifest` field, resolve the derived
        // factory name (`universal_{device_name}`) from the manifest file.
        let resolved_type: Option<String>;
        let factory = match self.factories.get(driver_type) {
            Some(f) => {
                resolved_type = None;
                f
            }
            None if driver_type == "universal" => {
                let derived = resolve_universal_factory_name(&config)?;
                let f = self.factories.get(&derived).ok_or_else(|| {
                    DaqError::Configuration(format!(
                        "No factory registered for derived driver_type '{}' \
                         (from manifest). Available factories: {:?}",
                        derived,
                        self.list_factories()
                    ))
                })?;
                resolved_type = Some(derived);
                f
            }
            None => {
                return Err(DaqError::Configuration(format!(
                    "No factory registered for driver_type '{}'. \
                     Available factories: {:?}",
                    driver_type,
                    self.list_factories()
                )));
            }
        };
        let driver_type = resolved_type.as_deref().unwrap_or(driver_type);

        // Validate configuration
        factory.validate(&config).map_err(|e| {
            DaqError::Driver(common::error::DriverError::new(
                driver_type,
                common::error::DriverErrorKind::Configuration,
                format!(
                    "Configuration validation failed for device '{device_id}' ({driver_type}): {e}"
                ),
            ))
        })?;

        // Build device components
        tracing::info!(
            device_id = %device_id,
            device_name = %device_name,
            driver_type = %driver_type,
            "Building device from factory"
        );

        // Spawn the build on a separate task to isolate potentially blocking
        // hardware initialization (serial port open, USB enumeration, etc.)
        // from the caller's task.  If the factory's build future contains
        // synchronous I/O, this prevents it from stalling the reconciler.
        let raw_config = config.clone();
        let build_future = factory.build(config);
        drop(factory); // Release DashMap ref before spawning.
        let components = tokio::task::spawn(build_future)
            .await
            .map_err(|join_err| {
                DaqError::Driver(common::error::DriverError::new(
                    driver_type,
                    common::error::DriverErrorKind::Initialization,
                    format!(
                        "Factory build task panicked for device '{device_id}' ({driver_type}): {join_err}"
                    ),
                ))
            })?
            .map_err(|e| {
                DaqError::Driver(common::error::DriverError::new(
                    driver_type,
                    common::error::DriverErrorKind::Initialization,
                    format!(
                        "Factory build failed for device '{device_id}' ({driver_type}): {e}"
                    ),
                ))
            })?;

        if let Err(err) = self
            .run_on_register(device_id, driver_type, components.lifecycle.as_ref())
            .await
        {
            let _ = self
                .run_on_unregister(device_id, driver_type, components.lifecycle.as_ref())
                .await;
            return Err(err);
        }

        // Convert to RegisteredDevice
        let registered = self.components_to_registered(
            device_id.to_string(),
            device_name.to_string(),
            driver_type.to_string(),
            components,
            raw_config,
        );

        self.devices.insert(DeviceId::from(device_id), registered);
        self.device_health
            .insert(DeviceId::from(device_id), DeviceHealthState::new());
        tracing::info!(device_id = %device_id, "Device registered successfully");
        Ok(())
    }

    /// Convert DeviceComponents from a factory into a RegisteredDevice.
    ///
    /// This bridges the new DriverFactory pattern with the legacy RegisteredDevice
    /// structure used internally by the registry.
    fn components_to_registered(
        &self,
        device_id: String,
        device_name: String,
        driver_type: String,
        components: DeviceComponents,
        raw_config: toml::Value,
    ) -> RegisteredDevice {
        let config = DeviceConfig {
            id: DeviceId::from(device_id.as_str()),
            name: device_name,
            driver: DriverConfig::new(driver_type.clone(), raw_config),
            enabled: true,
        };

        // Convert common::driver::DeviceMetadata to local DeviceMetadata
        let metadata = DeviceMetadata {
            category: components.metadata.category,
            position_units: components.metadata.position_units.clone(),
            min_position: components.metadata.min_position,
            max_position: components.metadata.max_position,
            measurement_units: components.metadata.measurement_units.clone(),
            frame_width: components.metadata.frame_width,
            frame_height: components.metadata.frame_height,
            bits_per_pixel: components.metadata.bits_per_pixel,
            min_exposure_ms: components.metadata.min_exposure_ms,
            max_exposure_ms: components.metadata.max_exposure_ms,
            min_wavelength_nm: components.metadata.min_wavelength_nm,
            max_wavelength_nm: components.metadata.max_wavelength_nm,
            available_commands: components.metadata.available_commands.clone(),
            ui_schema_json: components.metadata.ui_schema_json.clone(),
            panel_kind: components.metadata.panel_kind.clone(),
            config_source: None, // Caller sets after registration
            manifest_features: components.metadata.manifest_features.clone(),
        };

        let parameter_metadata =
            RegisteredDevice::build_parameter_metadata(components.parameterized.as_ref());

        // Log the actual driver_type for debugging (not the synthetic one)
        tracing::debug!(
            driver_type = %driver_type,
            capabilities = ?components.capabilities(),
            "Converting DeviceComponents to RegisteredDevice"
        );

        RegisteredDevice {
            config,
            driver_type,
            movable: components.movable,
            readable: components.readable,
            triggerable: components.triggerable,
            frame_producer: components.frame_producer,
            source_frame: components.source_frame,
            exposure_control: components.exposure_control,
            settable: components.settable,
            stageable: components.stageable,
            commandable: components.commandable,
            parameterized: components.parameterized,
            parameter_metadata,
            shutter_control: components.shutter_control,
            emission_control: components.emission_control,
            wavelength_tunable: components.wavelength_tunable,
            gated_camera: components.gated_camera,
            spectrometer_control: components.spectrometer_control,
            reconfigurable: components.reconfigurable,
            state_refreshable: components.state_refreshable,
            counter_configurable: components.counter_configurable,
            range_introspectable: components.range_introspectable,
            device_introspection: components.device_introspection,
            readable_with_metadata: components.readable_with_metadata,
            spectrum_readable: components.spectrum_readable,
            lifecycle: components.lifecycle,
            metadata,
            config_hash: 0, // Default — set by reconciler when registering from DB
        }
    }

    /// Register a device from a `DeviceConfig`.
    ///
    /// Delegates to `register_from_toml()` using the factory system.
    ///
    /// # Thread Safety (bd-pf31)
    /// This method is thread-safe and can be called concurrently.
    pub async fn register(&self, config: DeviceConfig) -> Result<(), DaqError> {
        self.register_from_toml(
            &config.id,
            &config.name,
            &config.driver.driver_type,
            config.driver.config,
        )
        .await
    }

    /// Register a pre-spawned plugin instance
    ///
    /// This is used by the PluginService to register drivers that it manages.
    /// It bypasses the normal driver instantiation process.
    ///
    /// # Arguments
    /// * `config` - Device configuration (driver_type must be "plugin")
    /// * `driver` - The pre-spawned GenericDriver instance
    ///
    /// # Errors
    /// Returns error if the device ID is already registered
    ///
    /// # Thread Safety (bd-pf31)
    /// This method is thread-safe and can be called concurrently.
    #[cfg(feature = "serial")]
    pub async fn register_plugin_instance(
        &self,
        config: DeviceConfig,
        driver: Arc<GenericDriver>,
    ) -> Result<(), DaqError> {
        if self.devices.contains_key(&config.id) {
            return Err(DaqError::Configuration(format!(
                "Device '{}' is already registered",
                config.id
            )));
        }

        let registered = self
            .create_registered_plugin(config, driver)
            .await
            .map_err(|e| {
                DaqError::Driver(common::error::DriverError::new(
                    "plugin",
                    common::error::DriverErrorKind::Initialization,
                    e.to_string(),
                ))
            })?;
        let driver_type = registered.driver_type.clone();
        if let Err(err) = self
            .run_on_register(
                &registered.config.id,
                &driver_type,
                registered.lifecycle.as_ref(),
            )
            .await
        {
            let _ = self
                .run_on_unregister(
                    &registered.config.id,
                    &driver_type,
                    registered.lifecycle.as_ref(),
                )
                .await;
            return Err(err);
        }
        let device_id = registered.config.id.clone();
        self.devices.insert(device_id.clone(), registered);
        self.device_health
            .insert(device_id, DeviceHealthState::new());
        Ok(())
    }

    /// Unregister a device
    ///
    /// # Arguments
    /// * `id` - Device ID to remove
    ///
    /// # Returns
    /// true if device was found and removed, false if not found
    ///
    /// # Thread Safety (bd-pf31)
    /// This method is thread-safe and can be called concurrently.
    pub async fn unregister(&self, id: &str) -> Result<bool, DaqError> {
        if let Some((_, device)) = self.devices.remove(id) {
            self.device_health.remove(id);
            self.measurement_locks.remove(id);
            let driver_type = device.driver_type.clone();
            self.run_on_unregister(&device.config.id, &driver_type, device.lifecycle.as_ref())
                .await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Shutdown all registered devices with bounded concurrency, collecting any errors.
    ///
    /// Uses `buffer_unordered(4)` rather than unbounded `join_all` because some drivers
    /// share underlying transports (e.g., multiple devices on one serial port) and fully
    /// parallel teardown could cause resource contention.
    pub async fn shutdown_all(&self) -> Result<(), DaqError> {
        use futures::stream::{self, StreamExt};

        let device_ids: Vec<DeviceId> = self
            .devices
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        let errors: Vec<DaqError> = stream::iter(device_ids)
            .map(|id| async move { self.unregister(&id).await })
            .buffer_unordered(4)
            .filter_map(|r| async { r.err() })
            .collect()
            .await;

        if errors.is_empty() {
            Ok(())
        } else {
            Err(DaqError::ShutdownFailed(errors))
        }
    }

    /// List all registered devices
    ///
    /// # Thread Safety (bd-pf31)
    /// This method iterates over all devices with fine-grained locking per entry.
    pub fn list_devices(&self) -> Vec<DeviceInfo> {
        self.devices
            .iter()
            .map(|entry| {
                let d = entry.value();
                DeviceInfo {
                    id: d.config.id.clone(),
                    name: d.config.name.clone(),
                    driver_type: d.driver_type.clone(),
                    capabilities: d.capabilities(),
                    metadata: d.metadata.clone(),
                }
            })
            .collect()
    }

    /// Record a registration failure for debugging
    ///
    /// Called when a device fails to register, allowing the failure to be
    /// queried later (e.g., shown in the GUI).
    pub fn record_registration_failure(&self, failure: RegistrationFailure) {
        tracing::error!(
            device_id = %failure.device_id,
            device_name = %failure.device_name,
            driver_type = %failure.driver_type,
            error = %failure.error,
            "Device registration failed"
        );
        self.registration_failures
            .insert(DeviceId::from(failure.device_id.as_str()), failure);
    }

    /// List all registration failures
    ///
    /// Returns devices that failed to register during initialization.
    /// Useful for GUI display and debugging.
    pub fn list_registration_failures(&self) -> Vec<RegistrationFailure> {
        self.registration_failures
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Check if there are any registration failures
    pub fn has_registration_failures(&self) -> bool {
        !self.registration_failures.is_empty()
    }

    /// Get the number of registration failures
    pub fn registration_failure_count(&self) -> usize {
        self.registration_failures.len()
    }

    /// Clear all registration failures (e.g., after user acknowledges)
    pub fn clear_registration_failures(&self) {
        self.registration_failures.clear();
    }

    // =========================================================================
    // Device Health (bd-qa36.4.2)
    // =========================================================================

    /// Set the consecutive-failure threshold before a device transitions to Faulted.
    ///
    /// Default is 3. Set to 1 for fail-fast behavior.
    pub fn set_fault_threshold(&self, threshold: u32) {
        self.fault_threshold.store(threshold, Ordering::Relaxed);
    }

    /// Get the current fault threshold.
    pub fn fault_threshold(&self) -> u32 {
        self.fault_threshold.load(Ordering::Relaxed)
    }

    /// Report a device failure to the health tracker.
    ///
    /// Increments consecutive failures and transitions to Degraded/Faulted
    /// based on the configured fault threshold (default: 3).
    /// Emits a [`DeviceHealthEvent`] on the broadcast channel if the health state changed.
    pub fn report_device_failure(&self, device_id: &str, error: impl Into<String>) {
        let threshold = self.fault_threshold.load(Ordering::Relaxed);
        let error_str = error.into();
        if let Some(mut state) = self.device_health.get_mut(device_id) {
            let old_health = state.health;
            state.record_failure(&error_str, threshold);
            tracing::warn!(
                device_id = %device_id,
                health = %state.health,
                consecutive_failures = state.consecutive_failures,
                error = %error_str,
                "Device failure recorded"
            );
            if state.health != old_health {
                let _ = self.health_broadcast.send(DeviceHealthEvent {
                    device_id: device_id.to_string(),
                    old_state: old_health,
                    new_state: state.health,
                    reason: error_str,
                    timestamp: std::time::Instant::now(),
                    consecutive_failures: state.consecutive_failures,
                    restart_attempts: state.restart_attempts,
                });
            }
        }
    }

    /// Report a successful device operation, resetting failure counters.
    ///
    /// Emits a [`DeviceHealthEvent`] on the broadcast channel if the health state changed.
    pub fn report_device_success(&self, device_id: &str) {
        if let Some(mut state) = self.device_health.get_mut(device_id) {
            let old_health = state.health;
            state.record_success();
            if state.health != old_health {
                let _ = self.health_broadcast.send(DeviceHealthEvent {
                    device_id: device_id.to_string(),
                    old_state: old_health,
                    new_state: state.health,
                    reason: "operation succeeded".to_string(),
                    timestamp: std::time::Instant::now(),
                    consecutive_failures: state.consecutive_failures,
                    restart_attempts: state.restart_attempts,
                });
            }
        }
    }

    /// Get the health state for a specific device.
    pub fn get_device_health(&self, device_id: &str) -> Option<DeviceHealthState> {
        self.device_health.get(device_id).map(|s| s.clone())
    }

    /// Get health states for all devices.
    pub fn list_device_health(&self) -> Vec<(DeviceId, DeviceHealthState)> {
        self.device_health
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Subscribe to device health state changes.
    ///
    /// Returns a broadcast receiver that yields [`DeviceHealthEvent`] whenever
    /// a device transitions between health states (e.g., Healthy -> Degraded).
    pub fn subscribe_health_changes(&self) -> broadcast::Receiver<DeviceHealthEvent> {
        self.health_broadcast.subscribe()
    }

    /// Get device IDs that are in Faulted state.
    pub fn faulted_devices(&self) -> Vec<DeviceId> {
        self.device_health
            .iter()
            .filter(|entry| entry.value().health == common::health::DeviceHealth::Faulted)
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Attempt to restart a faulted device by unregistering and re-registering via factory.
    ///
    /// Transitions the device to Recovering, then rebuilds it from the original
    /// config using the registered factory. On success, transitions to Healthy.
    /// On failure, transitions back to Faulted with updated error.
    ///
    /// Returns Ok(true) if restart succeeded, Ok(false) if device not found or not faulted,
    /// Err if the restart itself encountered an error.
    pub async fn restart_device(&self, device_id: &str) -> Result<bool, DaqError> {
        // Get the device config before removing it
        let device_config = match self.devices.get(device_id) {
            Some(d) => d.config.clone(),
            None => return Ok(false),
        };

        // Snapshot the current health state so we can preserve restart_attempts
        // across the unregister/re-register cycle (review fix: counter was lost).
        let prev_health = self.device_health.get(device_id).map(|s| s.clone());

        let is_faulted = prev_health
            .as_ref()
            .map(|s| s.health == DeviceHealth::Faulted)
            .unwrap_or(false);

        if !is_faulted {
            return Ok(false);
        }

        // Mark as recovering (increments restart_attempts in the snapshot)
        if let Some(mut state) = self.device_health.get_mut(device_id) {
            state.mark_recovering();
            let _ = self.health_broadcast.send(DeviceHealthEvent {
                device_id: device_id.to_string(),
                old_state: DeviceHealth::Faulted,
                new_state: DeviceHealth::Recovering,
                reason: "restart initiated".to_string(),
                timestamp: std::time::Instant::now(),
                consecutive_failures: state.consecutive_failures,
                restart_attempts: state.restart_attempts,
            });
        }

        // Re-snapshot after mark_recovering so we have the updated restart_attempts
        let preserved_health = self.device_health.get(device_id).map(|s| s.clone());

        tracing::info!(
            device_id = %device_id,
            driver_type = %device_config.driver.driver_type,
            restart_attempt = preserved_health.as_ref().map_or(0, |s| s.restart_attempts),
            "Attempting device restart"
        );

        // Quiesce: Remove the device from the registry and run on_unregister
        // lifecycle to give it a chance to clean up (close shutters, stop motors)
        // before rebuilding. This is best-effort — we continue even if it fails.
        // NOTE: We intentionally keep the device_health entry (Recovering status)
        // so the device remains visible in health queries during the restart window.
        // Capture metadata before removal so we can reconstruct on failure.
        let old_driver_type = self
            .devices
            .get(device_id)
            .map(|d| d.driver_type.clone())
            .unwrap_or_else(|| device_config.driver.driver_type.clone());
        let old_metadata = self
            .devices
            .get(device_id)
            .map(|d| d.metadata.clone())
            .unwrap_or_default();

        if let Some((_, old_device)) = self.devices.remove(device_id) {
            let driver_type = old_device.driver_type.clone();
            if let Err(e) = self
                .run_on_unregister(
                    &old_device.config.id,
                    &driver_type,
                    old_device.lifecycle.as_ref(),
                )
                .await
            {
                tracing::warn!(
                    device_id = %device_id,
                    error = %e,
                    "Quiesce (on_unregister) failed during restart — continuing"
                );
            }
        }

        // Re-register via factory
        match self.register(device_config.clone()).await {
            Ok(()) => {
                // Success: restore restart_attempts on the fresh health state
                // so the supervisor can still track the cumulative count.
                if let Some(prev) = &preserved_health
                    && let Some(mut new_state) = self.device_health.get_mut(device_id)
                {
                    new_state.restart_attempts = prev.restart_attempts;
                }
                // Post-reconnection state refresh (bd-47p2): query all
                // readable parameters from hardware to re-sync cached state.
                if let Some(refreshable) = self
                    .devices
                    .get(device_id)
                    .and_then(|d| d.state_refreshable.clone())
                {
                    match refreshable.refresh_state().await {
                        Ok(state) => {
                            tracing::info!(
                                device_id = %device_id,
                                refreshed_params = state.len(),
                                "Post-reconnection state refresh completed"
                            );
                            for (key, value) in &state {
                                tracing::debug!(
                                    device_id = %device_id,
                                    param = %key,
                                    value = %value,
                                    "Refreshed parameter"
                                );
                            }
                        }
                        Err(e) => {
                            // State refresh failure is non-fatal: the device
                            // reconnected successfully but cached state may
                            // be stale.  Log a warning so operators are aware.
                            tracing::warn!(
                                device_id = %device_id,
                                error = %e,
                                "Post-reconnection state refresh failed — cached state may be stale"
                            );
                        }
                    }
                }

                // Emit Recovering -> Healthy transition event
                let _ = self.health_broadcast.send(DeviceHealthEvent {
                    device_id: device_id.to_string(),
                    old_state: DeviceHealth::Recovering,
                    new_state: DeviceHealth::Healthy,
                    reason: "restart succeeded".to_string(),
                    timestamp: std::time::Instant::now(),
                    consecutive_failures: 0,
                    restart_attempts: preserved_health.as_ref().map_or(0, |s| s.restart_attempts),
                });
                tracing::info!(device_id = %device_id, "Device restart successful");
                Ok(true)
            }
            Err(e) => {
                tracing::error!(
                    device_id = %device_id,
                    error = %e,
                    "Device restart failed"
                );
                // Restore the preserved health state with updated failure info.
                // This keeps restart_attempts, consecutive_failures, etc. intact.
                let threshold = self.fault_threshold.load(Ordering::Relaxed);
                let mut state = preserved_health.unwrap_or_default();
                state.record_failure(e.to_string(), threshold);
                // Emit Recovering -> Faulted transition event
                let _ = self.health_broadcast.send(DeviceHealthEvent {
                    device_id: device_id.to_string(),
                    old_state: DeviceHealth::Recovering,
                    new_state: state.health,
                    reason: e.to_string(),
                    timestamp: std::time::Instant::now(),
                    consecutive_failures: state.consecutive_failures,
                    restart_attempts: state.restart_attempts,
                });
                self.device_health.insert(DeviceId::from(device_id), state);

                // Re-insert a stub device entry so the supervisor can retry.
                // Without this, the config is lost and the device becomes
                // permanently unreachable (devices map empty, health shows Faulted).
                self.devices.insert(
                    DeviceId::from(device_id),
                    RegisteredDevice {
                        config: device_config,
                        driver_type: old_driver_type,
                        movable: None,
                        readable: None,
                        triggerable: None,
                        frame_producer: None,
                        source_frame: None,
                        exposure_control: None,
                        settable: None,
                        stageable: None,
                        commandable: None,
                        parameterized: None,
                        parameter_metadata: HashMap::new(),
                        shutter_control: None,
                        emission_control: None,
                        wavelength_tunable: None,
                        gated_camera: None,
                        spectrometer_control: None,
                        reconfigurable: None,
                        state_refreshable: None,
                        counter_configurable: None,
                        range_introspectable: None,
                        device_introspection: None,
                        readable_with_metadata: None,
                        spectrum_readable: None,
                        lifecycle: None,
                        metadata: old_metadata,
                        config_hash: 0,
                    },
                );

                Err(e)
            }
        }
    }

    /// Get device info by ID
    pub fn get_device_info(&self, id: &str) -> Option<DeviceInfo> {
        self.devices.get(id).map(|d| DeviceInfo {
            id: d.config.id.clone(),
            name: d.config.name.clone(),
            driver_type: d.driver_type.clone(),
            capabilities: d.capabilities(),
            metadata: d.metadata.clone(),
        })
    }

    /// Check if a device is registered
    pub fn contains(&self, id: &str) -> bool {
        self.devices.contains_key(id)
    }

    /// Get count of registered devices
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    /// Check if registry is empty
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    // =========================================================================
    // Capability Access
    // =========================================================================

    /// Get a device as Movable (if it supports this capability)
    pub fn get_movable(&self, id: &str) -> Option<Arc<dyn Movable>> {
        self.devices.get(id).and_then(|d| d.movable.clone())
    }

    /// Get a device as Readable (if it supports this capability)
    pub fn get_readable(&self, id: &str) -> Option<Arc<dyn Readable>> {
        self.devices.get(id).and_then(|d| d.readable.clone())
    }

    /// Get a device as Triggerable (if it supports this capability)
    pub fn get_triggerable(&self, id: &str) -> Option<Arc<dyn Triggerable>> {
        self.devices.get(id).and_then(|d| d.triggerable.clone())
    }

    /// Get a device as FrameProducer (if it supports this capability)
    pub fn get_frame_producer(&self, id: &str) -> Option<Arc<dyn FrameProducer>> {
        self.devices.get(id).and_then(|d| d.frame_producer.clone())
    }

    /// Get MeasurementSource (frames) capability for a device (if supported)
    pub fn get_measurement_source_frame(
        &self,
        id: &str,
    ) -> Option<Arc<dyn MeasurementSource<Output = Arc<Frame>, Error = anyhow::Error>>> {
        self.devices.get(id).and_then(|d| d.source_frame.clone())
    }

    /// Get a string-valued driver config entry for a registered device.
    pub fn get_driver_config_string(&self, id: &str, key: &str) -> Option<String> {
        self.devices.get(id).and_then(|device| {
            let toml::Value::Table(config) = &device.config.driver.config else {
                return None;
            };
            config.get(key)?.as_str().map(ToOwned::to_owned)
        })
    }

    /// Get a device as ExposureControl (if it supports this capability)
    pub fn get_exposure_control(&self, id: &str) -> Option<Arc<dyn ExposureControl>> {
        self.devices
            .get(id)
            .and_then(|d| d.exposure_control.clone())
    }

    /// Get Stageable capability for a device
    pub fn get_stageable(&self, device_id: &str) -> Option<Arc<dyn Stageable>> {
        self.devices
            .get(device_id)
            .and_then(|d| d.stageable.clone())
    }

    /// Get parameterized trait for a device (bd-9clg)
    ///
    /// Enables generic code (gRPC, presets, HDF5 writers) to enumerate and subscribe
    /// to device parameters. Returns None if device doesn't implement Parameterized.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// if let Some(parameterized) = registry.get_parameterized("mock_camera") {
    ///     let params = parameterized.parameters();
    ///     for name in params.names() {
    ///         println!("Parameter: {}", name);
    ///     }
    /// }
    /// ```
    ///
    /// # Thread Safety (bd-pf31)
    /// Returns an Arc that can be used outside the registry lock scope.
    pub fn get_parameterized(&self, device_id: &str) -> Option<Arc<dyn Parameterized>> {
        self.devices
            .get(device_id)
            .and_then(|d| d.parameterized.clone())
    }

    /// Get cached parameter metadata for a specific parameter (bd-izdj.3).
    pub fn get_parameter_metadata(&self, device_id: &str, name: &str) -> Option<ParameterMetadata> {
        self.devices
            .get(device_id)
            .and_then(|d| d.parameter_metadata.get(name).cloned())
    }

    /// Get a device as ShutterControl (if it supports this capability)
    pub fn get_shutter_control(&self, id: &str) -> Option<Arc<dyn ShutterControl>> {
        self.devices.get(id).and_then(|d| d.shutter_control.clone())
    }

    /// Get a device as EmissionControl (if it supports this capability)
    pub fn get_emission_control(&self, id: &str) -> Option<Arc<dyn EmissionControl>> {
        self.devices
            .get(id)
            .and_then(|d| d.emission_control.clone())
    }

    /// Get a device as WavelengthTunable (if it supports this capability) - bd-pwjo
    pub fn get_wavelength_tunable(&self, id: &str) -> Option<Arc<dyn WavelengthTunable>> {
        self.devices
            .get(id)
            .and_then(|d| d.wavelength_tunable.clone())
    }

    /// Get a device as GatedCamera (if it supports this capability).
    pub fn get_gated_camera(&self, id: &str) -> Option<Arc<dyn GatedCamera>> {
        self.devices.get(id).and_then(|d| d.gated_camera.clone())
    }

    /// Get a device as SpectrometerControl (if it supports this capability).
    pub fn get_spectrometer_control(&self, id: &str) -> Option<Arc<dyn SpectrometerControl>> {
        self.devices
            .get(id)
            .and_then(|d| d.spectrometer_control.clone())
    }

    /// Get a device as Settable (if it supports this capability)
    pub fn get_settable(&self, id: &str) -> Option<Arc<dyn Settable>> {
        self.devices.get(id).and_then(|d| d.settable.clone())
    }

    /// Get a device as Commandable (if it supports this capability)
    pub fn get_commandable(&self, id: &str) -> Option<Arc<dyn Commandable>> {
        self.devices.get(id).and_then(|d| d.commandable.clone())
    }

    /// Get a device as Reconfigurable (if it supports runtime config changes)
    pub fn get_reconfigurable(&self, id: &str) -> Option<Arc<dyn Reconfigurable>> {
        self.devices.get(id).and_then(|d| d.reconfigurable.clone())
    }

    /// Get a device as StateRefreshable (if it supports post-reconnection refresh, bd-47p2)
    pub fn get_state_refreshable(&self, id: &str) -> Option<Arc<dyn StateRefreshable>> {
        self.devices
            .get(id)
            .and_then(|d| d.state_refreshable.clone())
    }

    /// Get a device as CounterConfigurable (if it supports counter/timer config, bd-f3pq)
    pub fn get_counter_configurable(&self, id: &str) -> Option<Arc<dyn CounterConfigurable>> {
        self.devices
            .get(id)
            .and_then(|d| d.counter_configurable.clone())
    }

    /// Get a device as RangeIntrospectable (if it supports analog range queries, bd-3bjp)
    pub fn get_range_introspectable(&self, id: &str) -> Option<Arc<dyn RangeIntrospectable>> {
        self.devices
            .get(id)
            .and_then(|d| d.range_introspectable.clone())
    }

    /// Get a device as DeviceIntrospection (if it supports board/subdevice metadata, bd-sa9p)
    pub fn get_device_introspection(&self, id: &str) -> Option<Arc<dyn DeviceIntrospection>> {
        self.devices
            .get(id)
            .and_then(|d| d.device_introspection.clone())
    }

    /// Get a device as ReadableWithMetadata (if it supports structured analog reads, bd-09ls)
    pub fn get_readable_with_metadata(&self, id: &str) -> Option<Arc<dyn ReadableWithMetadata>> {
        self.devices
            .get(id)
            .and_then(|d| d.readable_with_metadata.clone())
    }

    /// Get a device as SpectrumReadable (if it supports 1D detector/spectrum data, bd-lncj.1.2)
    pub fn get_spectrum_readable(&self, id: &str) -> Option<Arc<dyn SpectrumReadable>> {
        self.devices
            .get(id)
            .and_then(|d| d.spectrum_readable.clone())
    }

    /// Get the config hash for a registered device (for change detection).
    pub fn config_hash(&self, id: &str) -> Option<u64> {
        self.devices.get(id).map(|d| d.config_hash)
    }

    /// Update the config hash after a successful reconfiguration.
    pub fn set_config_hash(&self, id: &str, hash: u64) {
        if let Some(mut entry) = self.devices.get_mut(id) {
            entry.config_hash = hash;
        }
    }

    /// Get a string value from a device's raw driver config.
    ///
    /// Useful for retrieving driver-specific fields (e.g., `"device"` for Comedi
    /// drivers) that aren't exposed through `DeviceMetadata`.
    pub fn get_driver_config_str(&self, id: &str, key: &str) -> Option<String> {
        self.devices.get(id).and_then(|d| {
            d.config
                .driver
                .config
                .get(key)
                .and_then(|v| v.as_str())
                .map(String::from)
        })
    }

    /// Set the config source for a device (e.g., "toml", "db").
    pub fn set_config_source(&self, id: &str, source: &str) {
        if let Some(mut entry) = self.devices.get_mut(id) {
            entry.metadata.config_source = Some(source.to_string());
        }
    }

    /// Set the measurement lock state for a device.
    ///
    /// Drivers should call this with `Measuring` when starting a measurement
    /// and `Idle` when done. The reconciler checks this before calling
    /// `reconfigure()` to avoid changing hardware config mid-measurement.
    pub fn set_measurement_lock(&self, id: &str, lock: common::capabilities::MeasurementLock) {
        self.measurement_locks.insert(DeviceId::from(id), lock);
    }

    /// Check whether a device is idle (safe to reconfigure).
    ///
    /// Returns `true` if the device has no lock or the lock is `Idle`.
    /// Returns `false` if the device is actively measuring.
    pub fn is_device_idle(&self, id: &str) -> bool {
        self.measurement_locks
            .get(id)
            .is_none_or(|lock| lock.is_idle())
    }

    /// Get all devices that support a specific capability
    ///
    /// # Thread Safety (bd-pf31)
    /// This method iterates over all devices with fine-grained locking per entry.
    pub fn devices_with_capability(&self, capability: Capability) -> Vec<DeviceId> {
        self.devices
            .iter()
            .filter(|entry| entry.value().capabilities().contains(&capability))
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Creates a RegisteredDevice from a pre-spawned plugin driver
    ///
    /// This is the shared implementation used by both `instantiate_plugin_device`
    /// (for config-based registration) and `register_plugin_instance` (for
    /// PluginService-managed registration).
    #[cfg(feature = "serial")]
    async fn create_registered_plugin(
        &self,
        config: DeviceConfig,
        driver: Arc<GenericDriver>,
    ) -> Result<RegisteredDevice, DaqError> {
        if config.driver.driver_type != "plugin" {
            return Err(DaqError::Configuration(format!(
                "Invalid driver type for create_registered_plugin: expected 'plugin', got '{}'",
                config.driver.driver_type
            )));
        }
        let plugin_id = config
            .driver
            .config
            .get("plugin_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                DaqError::Configuration("Missing 'plugin_id' in plugin driver config".into())
            })?
            .to_string();
        let driver_type_name = config.driver.driver_type.clone();

        // Introspect capabilities from the plugin configuration
        let factory = self.plugin_factory.read().await;
        let plugin_config = factory.get_config(&plugin_id).ok_or_else(|| {
            DaqError::Configuration(format!("Plugin '{plugin_id}' not found in factory"))
        })?;

        let mut metadata = DeviceMetadata::default();

        // Check for movable capability
        let movable: Option<Arc<dyn Movable>> = if plugin_config.capabilities.movable.is_some() {
            // Extract metadata from first axis
            if let Some(movable_cap) = &plugin_config.capabilities.movable
                && let Some(first_axis) = movable_cap.axes.first()
            {
                metadata.position_units.clone_from(&first_axis.unit);
                metadata.min_position = first_axis.min;
                metadata.max_position = first_axis.max;
            }

            // Create axis handle for the first axis (convention)
            let axis_name = plugin_config
                .capabilities
                .movable
                .as_ref()
                .and_then(|m| m.axes.first())
                .map(|a| a.name.as_str())
                .unwrap_or("axis");

            Some(Arc::new(
                crate::manifest_driver::handles::PluginAxisHandle::new(
                    driver.clone(),
                    axis_name.to_string(),
                    false, // not mocking
                ),
            ))
        } else {
            None
        };

        // Check for readable capability
        let readable: Option<Arc<dyn Readable>> = if !plugin_config.capabilities.readable.is_empty()
        {
            // Extract metadata from first readable
            if let Some(first_readable) = plugin_config.capabilities.readable.first() {
                metadata.measurement_units.clone_from(&first_readable.unit);
            }

            // Create readable handle for the first readable capability (convention)
            let readable_name = plugin_config
                .capabilities
                .readable
                .first()
                .map(|r| r.name.as_str())
                .unwrap_or("reading");

            Some(Arc::new(
                crate::manifest_driver::handles::PluginSensorHandle::new(
                    driver.clone(),
                    readable_name.to_string(),
                    false, // not mocking
                ),
            ))
        } else {
            None
        };

        // Note: FrameProducer, Triggerable, and ExposureControl are not yet
        // supported by the plugin system, so we leave them as None
        let parameterized: Option<Arc<dyn Parameterized>> = Some(driver.clone());
        let parameter_metadata = RegisteredDevice::build_parameter_metadata(parameterized.as_ref());

        Ok(RegisteredDevice {
            config,
            driver_type: driver_type_name.clone(),
            movable,
            readable,
            triggerable: None,
            frame_producer: None,
            source_frame: None,
            exposure_control: None,
            settable: None,
            stageable: None,
            commandable: None,
            parameterized, // bd-plb6: Wire Parameterized for plugin devices
            parameter_metadata,
            shutter_control: None,
            emission_control: None,
            wavelength_tunable: None,
            gated_camera: None,
            spectrometer_control: None,
            reconfigurable: None,
            state_refreshable: None,
            counter_configurable: None,
            range_introspectable: None,
            device_introspection: None,
            readable_with_metadata: None,
            spectrum_readable: None,
            lifecycle: None,
            metadata,
            config_hash: 0,
        })
    }

    /// Snapshot all parameters from all devices with Parameterized trait (bd-ej44)
    ///
    /// Returns a nested map: device_id -> parameter_name -> JSON value
    /// This is used for experiment manifests to capture complete hardware state.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let snapshot = registry.snapshot_all_parameters();
    /// // Returns:
    /// // {
    /// //   "mock_camera": {
    /// //     "exposure_ms": 100.0,
    /// //     "gain": 1.5
    /// //   },
    /// //   "mock_stage": {
    /// //     "position": 0.0
    /// //   }
    /// // }
    /// ```
    ///
    /// # Thread Safety (bd-pf31)
    /// This method iterates over all devices with fine-grained locking per entry.
    pub fn snapshot_all_parameters(&self) -> HashMap<String, HashMap<String, serde_json::Value>> {
        let mut snapshot = HashMap::new();

        for entry in &self.devices {
            let device_id = entry.key();
            let device = entry.value();
            if let Some(parameterized) = &device.parameterized {
                let params = parameterized.parameters();
                let mut device_params = HashMap::new();

                for (name, param) in params.iter() {
                    // Get JSON value for each parameter
                    if let Ok(value) = param.get_json() {
                        device_params.insert(name.to_string(), value);
                    } else {
                        // If serialization fails, store error marker
                        device_params.insert(
                            name.to_string(),
                            serde_json::json!({"error": "serialization_failed"}),
                        );
                    }
                }

                if !device_params.is_empty() {
                    snapshot.insert(device_id.to_string(), device_params);
                }
            }
        }

        snapshot
    }
}

// =============================================================================
// CapabilityProvider Implementation (bd-bog5)
// =============================================================================

impl CapabilityProvider for DeviceRegistryInner {
    fn get_movable(&self, id: &str) -> Option<Arc<dyn Movable>> {
        self.get_movable(id)
    }

    fn get_readable(&self, id: &str) -> Option<Arc<dyn Readable>> {
        self.get_readable(id)
    }

    fn get_triggerable(&self, id: &str) -> Option<Arc<dyn Triggerable>> {
        self.get_triggerable(id)
    }

    fn get_frame_producer(&self, id: &str) -> Option<Arc<dyn FrameProducer>> {
        self.get_frame_producer(id)
    }

    fn get_exposure_control(&self, id: &str) -> Option<Arc<dyn ExposureControl>> {
        self.get_exposure_control(id)
    }

    fn get_shutter_control(&self, id: &str) -> Option<Arc<dyn ShutterControl>> {
        self.get_shutter_control(id)
    }

    fn get_wavelength_tunable(&self, id: &str) -> Option<Arc<dyn WavelengthTunable>> {
        self.get_wavelength_tunable(id)
    }

    fn get_emission_control(&self, id: &str) -> Option<Arc<dyn EmissionControl>> {
        self.get_emission_control(id)
    }

    fn get_settable(&self, id: &str) -> Option<Arc<dyn Settable>> {
        self.get_settable(id)
    }

    fn get_counter_configurable(&self, id: &str) -> Option<Arc<dyn CounterConfigurable>> {
        self.get_counter_configurable(id)
    }

    fn get_range_introspectable(&self, id: &str) -> Option<Arc<dyn RangeIntrospectable>> {
        self.get_range_introspectable(id)
    }

    fn get_device_introspection(&self, id: &str) -> Option<Arc<dyn DeviceIntrospection>> {
        self.get_device_introspection(id)
    }

    fn get_readable_with_metadata(&self, id: &str) -> Option<Arc<dyn ReadableWithMetadata>> {
        self.get_readable_with_metadata(id)
    }

    fn get_spectrum_readable(&self, id: &str) -> Option<Arc<dyn SpectrumReadable>> {
        self.get_spectrum_readable(id)
    }
}

impl Default for DeviceRegistryInner {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Hardware Configuration File Support
// =============================================================================

/// Create a DeviceRegistry from a hardware configuration file
///
/// # Example TOML format:
/// ```toml
/// # Optional: Plugin search paths (first = highest priority)
/// plugin_paths = [
///     "~/.config/rust-daq/plugins",
///     "/usr/share/rust-daq/plugins"
/// ]
///
/// [[devices]]
/// id = "rotator_2"
/// name = "ELL14 Rotation Mount (Addr 2)"
/// [devices.driver]
/// type = "ell14"
/// port = "/dev/ttyUSB0"
/// address = "2"
///
/// [[devices]]
/// id = "my_sensor"
/// name = "Custom Sensor (Plugin-Based)"
/// [devices.driver]
/// type = "plugin"
/// plugin_id = "my-sensor-v1"
/// address = "/dev/ttyUSB2"
/// ```
/// Populate a registry with plugins and devices from a hardware config.
///
/// This loads manifest-driver plugins from configured search paths and
/// registers all configured devices. **Factories must be registered first**
/// (via `register_mock_factories` or `driver_registry::register_all_factories`).
///
/// This is the abstract half of registry creation — it does not reference
/// any concrete driver crates, only the factory system already registered
/// on the `DeviceRegistry`.
pub async fn populate_registry_from_config(
    registry: &DeviceRegistry,
    config: &HardwareConfig,
) -> Result<(), DaqError> {
    // Load plugins from configured search paths
    #[cfg(feature = "serial")]
    {
        let plugin_factory = registry.plugin_factory();
        let mut factory = plugin_factory.write().await;
        for path in &config.plugin_paths {
            // Expand ~ to home directory
            let expanded = if path.starts_with("~") {
                if let Some(home) = dirs::home_dir() {
                    home.join(path.strip_prefix("~").unwrap_or(path))
                } else {
                    path.clone()
                }
            } else {
                path.clone()
            };
            factory.add_search_path(expanded);
        }

        // Scan all paths and report errors
        let errors = factory.scan().await;
        for err in &errors {
            tracing::warn!("Plugin load warning: {}", err);
        }

        // Log loaded plugins
        let plugins = factory.available_plugins();
        if !plugins.is_empty() {
            tracing::info!("Loaded {} plugin(s): {:?}", plugins.len(), plugins);
        }
    }

    // Register all configured devices via factory system
    let mut success_count = 0;
    let mut failure_count = 0;

    for device_config in &config.devices {
        if !device_config.enabled {
            tracing::info!(
                device_id = %device_config.id,
                "Skipping disabled device"
            );
            continue;
        }

        let driver_type = &device_config.driver.driver_type;

        tracing::info!(
            device_id = %device_config.id,
            device_name = %device_config.name,
            driver_type = %driver_type,
            "Registering device"
        );

        let result = registry
            .register_from_toml(
                &device_config.id,
                &device_config.name,
                driver_type,
                device_config.driver.config.clone(),
            )
            .await;

        if let Err(e) = result {
            failure_count += 1;
            registry.record_registration_failure(RegistrationFailure {
                device_id: device_config.id.to_string(),
                device_name: device_config.name.clone(),
                driver_type: driver_type.clone(),
                error: e.to_string(),
            });
        } else {
            success_count += 1;
            tracing::info!(
                device_id = %device_config.id,
                "Device registered successfully"
            );
        }
    }

    // Summary logging
    if failure_count > 0 {
        tracing::warn!(
            success_count,
            failure_count,
            "Device registration completed with failures"
        );
    } else {
        tracing::info!(success_count, "All devices registered successfully");
    }

    Ok(())
}

// =============================================================================
// Convenience Functions for Lab Configuration
// =============================================================================

/// Create a DeviceRegistry with mock devices for testing.
///
/// **Deprecated**: Prefer `driver_registry::create_canonical_mock_registry()` which
/// uses universal-driver emulators for universal-eligible instruments. This function
/// uses handwritten `driver-mock` implementations that don't exercise the manifest
/// emulator code path.
///
/// Requires the `test-util` feature (M-TEST-UTIL, bd-qd4zn).
#[cfg(any(test, feature = "test-util"))]
#[deprecated(
    note = "use driver_registry::create_canonical_mock_registry() for universal mock parity"
)]
pub async fn create_mock_registry() -> Result<DeviceRegistry, DaqError> {
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);

    registry
        .register_from_toml(
            "mock_stage",
            "Mock Stage",
            "mock_stage",
            toml::toml! { initial_position = 0.0 }.into(),
        )
        .await?;

    registry
        .register_from_toml(
            "mock_power_meter",
            "Mock Power Meter",
            "mock_power_meter",
            toml::toml! { base_power = 1e-6 }.into(),
        )
        .await?;

    registry
        .register_from_toml(
            "mock_camera",
            "Mock Camera",
            "mock_camera",
            toml::toml! {
                width = 640
                height = 480
            }
            .into(),
        )
        .await?;

    Ok(registry)
}

/// Register all mock driver factories with a registry.
///
/// This enables using `register_from_toml()` for mock devices:
///
/// ```rust,ignore
/// use daq_hardware::registry::{DeviceRegistry, register_mock_factories};
///
/// let registry = DeviceRegistry::new();
/// register_mock_factories(&registry);
///
/// // Now register mock devices via TOML config
/// registry.register_from_toml(
///     "my_stage",
///     "My Test Stage",
///     "mock_stage",
///     toml::Value::Table(Default::default()),
/// ).await?;
/// ```
///
/// Requires the `test-util` feature (M-TEST-UTIL, bd-qd4zn).
#[cfg(any(test, feature = "test-util"))]
pub fn register_mock_factories(registry: &DeviceRegistry) {
    use driver_mock::{MockCameraFactory, MockPowerMeterFactory, MockStageFactory};

    registry.register_factory(Box::new(MockStageFactory));
    registry.register_factory(Box::new(MockCameraFactory));
    registry.register_factory(Box::new(MockPowerMeterFactory));
}

// NOTE: `register_all_factories` has been moved to the `driver-registry` crate.
// Use `driver_registry::register_all_factories()` for concrete driver registration.
// `register_mock_factories` remains here since it only needs driver-mock (always compiled).

// =============================================================================
// Universal manifest resolution
// =============================================================================

/// Derive the factory name for a `type = "universal"` device by reading its
/// manifest file.
///
/// The manifest TOML must contain `[device] name = "..."`. The factory name
/// is `universal_{name_lowercased_underscored}`, matching the convention in
/// `UniversalDriverFactory::new`.
fn resolve_universal_factory_name(config: &toml::Value) -> Result<String, DaqError> {
    let manifest_path = config
        .get("manifest")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| {
            DaqError::Configuration(
                "driver type 'universal' requires a 'manifest' field pointing \
                 to the device TOML manifest (e.g., manifest = \"config/devices/my_device.toml\")"
                    .to_string(),
            )
        })?;

    let content = std::fs::read_to_string(manifest_path).map_err(|e| {
        DaqError::Configuration(format!(
            "Failed to read universal manifest '{manifest_path}': {e}"
        ))
    })?;

    let table: toml::Value = toml::from_str(&content).map_err(|e| {
        DaqError::Configuration(format!(
            "Failed to parse universal manifest '{manifest_path}': {e}"
        ))
    })?;

    let device_name = table
        .get("device")
        .and_then(|d| d.get("name"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| {
            DaqError::Configuration(format!(
                "Universal manifest '{manifest_path}' is missing [device].name"
            ))
        })?;

    let derived = format!("universal_{}", device_name.to_lowercase().replace(' ', "_"));

    tracing::info!(
        manifest = %manifest_path,
        derived_factory = %derived,
        "Resolved type='universal' to derived factory name"
    );

    Ok(derived)
}

#[cfg(test)]
mod tests;
