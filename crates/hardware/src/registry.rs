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
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      DeviceRegistry                             │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐            │
//! │  │ Device<Ell14>│  │ Device<1830C>│  │ Device<ESP300>│  ...    │
//! │  └─────────────┘  └─────────────┘  └─────────────┘            │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                    Capability Traits                            │
//! │  Movable | Readable | Triggerable | FrameProducer | ...        │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                    Hardware Drivers                             │
//! │  Ell14Driver | Newport1830CDriver | MaiTaiDriver | Esp300Driver │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Known Instruments (from docs/HARDWARE_INVENTORY.md)
//!
//! | Device | Driver | Port | Capabilities |
//! |--------|--------|------|--------------|
//! | Newport 1830-C Power Meter | `Newport1830CDriver` | `/dev/ttyS0` | Readable |
//! | Spectra-Physics MaiTai Laser | `MaiTaiDriver` | `/dev/ttyUSB5` | Readable |
//! | Thorlabs ELL14 Rotation Mount (3x) | `Ell14Driver` | `/dev/ttyUSB0` @ 2,3,8 | Movable |
//! | Newport ESP300 Motion Controller | `Esp300Driver` | `/dev/ttyUSB1` | Movable |
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
//!     // Register devices using factory-based driver types
//!     registry.register_from_toml(
//!         "power_meter",
//!         "Newport 1830-C",
//!         "newport_1830c",
//!         toml::toml! { port = "/dev/ttyS0" }.into(),
//!     ).await?;
//!
//!     registry.register_from_toml(
//!         "rotator_2",
//!         "ELL14 Address 2",
//!         "ell14",
//!         toml::toml! { port = "/dev/ttyUSB0"; address = "2" }.into(),
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

use anyhow::{anyhow, Result};
use common::capabilities::{
    Commandable, EmissionControl, ExposureControl, FrameProducer, Movable, Parameterized, Readable,
    Settable, ShutterControl, Stageable, Triggerable, WavelengthTunable,
};
use common::data::Frame;
use common::driver::{Capability, DeviceComponents, DeviceLifecycle, DriverFactory};
use common::error::DaqError;
use common::observable::ParameterMetadata;
use common::pipeline::MeasurementSource;

#[cfg(feature = "serial")]
use crate::plugin::driver::GenericDriver;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
#[cfg(feature = "serial")]
use tokio::sync::RwLock;

// Configuration validation is now handled by individual DriverFactory::validate() implementations.

// =============================================================================
// Device Identification
// =============================================================================

/// Unique identifier for a registered device
///
/// Format: lowercase alphanumeric with underscores (e.g., "power_meter", "rotator_2")
pub type DeviceId = String;

// =============================================================================
// Device Configuration
// =============================================================================

/// Configuration for registering a device.
///
/// The `driver` field is a table containing a `type` key (matched to a registered
/// `DriverFactory`) and driver-specific config fields passed to the factory's
/// `validate()` / `build()` methods.
///
/// # TOML Format
///
/// ```toml
/// [[devices]]
/// id = "rotator_2"
/// name = "ELL14 Rotator (Address 2)"
/// [devices.driver]
/// type = "ell14"
/// port = "/dev/serial/by-id/..."
/// address = "2"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    /// Unique identifier (e.g., "power_meter", "rotator_2")
    pub id: DeviceId,
    /// Human-readable name (e.g., "Newport 1830-C Power Meter")
    pub name: String,
    /// Driver configuration containing `type` and driver-specific fields
    pub driver: DriverConfig,
    /// Whether this device is enabled (default: true)
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// Driver configuration extracted from TOML.
///
/// Deserializes a table like `{ type = "ell14", port = "...", address = "2" }` into
/// a `driver_type` string and a `config` value containing the remaining fields.
#[derive(Debug, Clone)]
pub struct DriverConfig {
    /// Driver type string (matches `DriverFactory::driver_type()`)
    pub driver_type: String,
    /// Driver-specific config (the table minus the `type` key)
    pub config: toml::Value,
}

impl serde::Serialize for DriverConfig {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        // Reconstruct the table with `type` merged in
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("type", &self.driver_type)?;
        match &self.config {
            toml::Value::Table(t) => {
                for (k, v) in t {
                    map.serialize_entry(k, v)?;
                }
            }
            other => {
                tracing::warn!(
                    driver_type = %self.driver_type,
                    value_type = other.type_str(),
                    "DriverConfig.config is not a Table, driver-specific fields will be lost",
                );
            }
        }
        map.end()
    }
}

impl<'de> serde::Deserialize<'de> for DriverConfig {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut table: toml::map::Map<String, toml::Value> =
            toml::map::Map::deserialize(deserializer)?;
        let driver_type = table
            .remove("type")
            .and_then(|v| v.as_str().map(String::from))
            .ok_or_else(|| serde::de::Error::missing_field("type"))?;
        Ok(DriverConfig {
            driver_type,
            config: toml::Value::Table(table),
        })
    }
}

impl DriverConfig {
    /// Create a new DriverConfig from type string and config value.
    pub fn new(driver_type: impl Into<String>, config: toml::Value) -> Self {
        Self {
            driver_type: driver_type.into(),
            config,
        }
    }
}

// =============================================================================
// Device Info (for introspection)
// =============================================================================

/// Information about a registered device (returned by list operations)
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// Unique identifier
    pub id: DeviceId,
    /// Human-readable name
    pub name: String,
    /// Driver type name (e.g., "ell14", "newport_1830c")
    pub driver_type: String,
    /// Capabilities this device supports
    pub capabilities: Vec<Capability>,
    /// Capability-specific metadata
    pub metadata: DeviceMetadata,
}

/// Capability-specific metadata for a device
#[derive(Debug, Clone, Default)]
pub struct DeviceMetadata {
    /// Device category for UI grouping (bd-le6k: moved from gRPC inference layer)
    ///
    /// Drivers should set this explicitly. The gRPC layer will fall back to
    /// string-based driver name inference only if this is None.
    pub category: Option<common::capabilities::DeviceCategory>,
    /// For Movable devices: position units (e.g., "mm", "degrees")
    pub position_units: Option<String>,
    /// For Movable devices: min position
    pub min_position: Option<f64>,
    /// For Movable devices: max position
    pub max_position: Option<f64>,
    /// For Readable devices: measurement units (e.g., "W", "V")
    pub measurement_units: Option<String>,
    /// For FrameProducer devices: frame width in pixels
    pub frame_width: Option<u32>,
    /// For FrameProducer devices: frame height in pixels
    pub frame_height: Option<u32>,
    /// For FrameProducer devices: bits per pixel (e.g., 8, 12, 16)
    pub bits_per_pixel: Option<u32>,
    /// For ExposureControl devices: minimum exposure in milliseconds
    pub min_exposure_ms: Option<f64>,
    /// For ExposureControl devices: maximum exposure in milliseconds
    pub max_exposure_ms: Option<f64>,
    /// For WavelengthTunable devices: minimum wavelength in nm (bd-pwjo)
    pub min_wavelength_nm: Option<f64>,
    /// For WavelengthTunable devices: maximum wavelength in nm (bd-pwjo)
    pub max_wavelength_nm: Option<f64>,
}

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
    /// Optional lifecycle hooks for registration/shutdown
    lifecycle: Option<Arc<dyn DeviceLifecycle>>,
    /// Device metadata (units, ranges, etc.)
    metadata: DeviceMetadata,
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
                metadata.insert(name.to_string(), ParameterMetadata::from(&param.metadata()));
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
        if self.shutter_control.is_some() {
            caps.push(Capability::ShutterControl);
        }
        if self.emission_control.is_some() {
            caps.push(Capability::EmissionControl);
        }
        if self.wavelength_tunable.is_some() {
            caps.push(Capability::WavelengthTunable);
        }

        caps
    }
}

// =============================================================================
// Device Registry
// =============================================================================

/// Central registry for hardware device management
///
/// The DeviceRegistry is the primary interface for:
/// - Registering devices from configuration
/// - Discovering connected devices
/// - Accessing devices by capability
/// - Querying device information
///
/// # Thread Safety
///
/// DeviceRegistry is internally thread-safe using DashMap for the devices collection.
/// This eliminates the need for external RwLock wrapping and allows concurrent access
/// to different devices without global lock contention. Individual device lookups
/// only lock the specific entry being accessed.
///
/// Usage:
/// - Pass as `Arc<DeviceRegistry>`
/// - Call methods directly (no `.read().await` needed)
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
pub struct DeviceRegistry {
    /// Registered devices by ID (thread-safe via DashMap)
    devices: DashMap<DeviceId, RegisteredDevice>,

    /// Registered driver factories by driver_type (thread-safe via DashMap)
    ///
    /// All device registration goes through factories. The factory matching
    /// the driver_type is used to validate config and build the device.
    factories: DashMap<String, Box<dyn DriverFactory>>,

    /// Plugin factory for loading YAML-defined drivers (serial feature only)
    #[cfg(feature = "serial")]
    plugin_factory: Arc<RwLock<crate::plugin::registry::PluginFactory>>,

    /// Registration failures for debugging (device_id, driver_type, error_message)
    registration_failures: DashMap<DeviceId, RegistrationFailure>,

    /// Per-device health tracking for supervisor (bd-qa36.4.2)
    device_health: DashMap<DeviceId, common::health::DeviceHealthState>,

    /// Consecutive failures before transitioning to Faulted (default: 3).
    /// Configurable via [`set_fault_threshold`](DeviceRegistry::set_fault_threshold).
    fault_threshold: AtomicU32,
}

/// Information about a failed device registration
#[derive(Debug, Clone)]
pub struct RegistrationFailure {
    /// Device ID that failed to register
    pub device_id: String,
    /// Device name from config
    pub device_name: String,
    /// Driver type that failed
    pub driver_type: String,
    /// Error message describing the failure
    pub error: String,
}

/// Information about a registered driver factory
#[derive(Debug, Clone)]
pub struct FactoryInfo {
    /// The driver_type string this factory handles
    pub driver_type: String,
    /// Human-readable factory name
    pub name: String,
    /// List of capabilities this driver provides
    pub capabilities: Vec<String>,
}

impl DeviceRegistry {
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
                    format!(
                        "Lifecycle on_register failed for device '{}': {}",
                        device_id, e
                    ),
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
                    format!(
                        "Lifecycle on_unregister failed for device '{}': {}",
                        device_id, e
                    ),
                ))
            })?;
        }
        Ok(())
    }
    /// Create a new empty device registry
    pub fn new() -> Self {
        Self {
            devices: DashMap::new(),
            factories: DashMap::new(),
            #[cfg(feature = "serial")]
            plugin_factory: Arc::new(RwLock::new(crate::plugin::registry::PluginFactory::new())),
            registration_failures: DashMap::new(),
            device_health: DashMap::new(),
            fault_threshold: AtomicU32::new(3),
        }
    }

    /// Create a new device registry with a pre-configured PluginFactory
    #[cfg(feature = "serial")]
    pub fn with_plugin_factory(
        plugin_factory: Arc<RwLock<crate::plugin::registry::PluginFactory>>,
    ) -> Self {
        Self {
            devices: DashMap::new(),
            factories: DashMap::new(),
            plugin_factory,
            registration_failures: DashMap::new(),
            device_health: DashMap::new(),
            fault_threshold: AtomicU32::new(3),
        }
    }

    /// Get a reference to the plugin factory
    #[cfg(feature = "serial")]
    pub fn plugin_factory(&self) -> Arc<RwLock<crate::plugin::registry::PluginFactory>> {
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
        tracing::info!(
            driver_type = %driver_type,
            name = %factory.name(),
            capabilities = ?factory.capabilities(),
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
                    .map(|c| format!("{:?}", c))
                    .collect(),
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
                "Device '{}' is already registered",
                device_id
            )));
        }

        // Look up factory
        let factory = self.factories.get(driver_type).ok_or_else(|| {
            DaqError::Configuration(format!(
                "No factory registered for driver_type '{}'. \
                 Available factories: {:?}",
                driver_type,
                self.list_factories()
            ))
        })?;

        // Validate configuration
        factory.validate(&config).map_err(|e| {
            DaqError::Driver(common::error::DriverError::new(
                driver_type,
                common::error::DriverErrorKind::Configuration,
                format!(
                    "Configuration validation failed for device '{}' ({}): {}",
                    device_id, driver_type, e
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

        let components = factory.build(config).await.map_err(|e| {
            DaqError::Driver(common::error::DriverError::new(
                driver_type,
                common::error::DriverErrorKind::Initialization,
                format!(
                    "Factory build failed for device '{}' ({}): {}",
                    device_id, driver_type, e
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
        );

        self.devices.insert(device_id.to_string(), registered);
        self.device_health.insert(
            device_id.to_string(),
            common::health::DeviceHealthState::new(),
        );
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
    ) -> RegisteredDevice {
        let config = DeviceConfig {
            id: device_id,
            name: device_name,
            driver: DriverConfig::new(driver_type.clone(), toml::Value::Table(Default::default())),
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
            lifecycle: components.lifecycle,
            metadata,
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
            .insert(device_id, common::health::DeviceHealthState::new());
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
            let driver_type = device.driver_type.clone();
            self.run_on_unregister(&device.config.id, &driver_type, device.lifecycle.as_ref())
                .await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Shutdown all registered devices, collecting any lifecycle errors.
    pub async fn shutdown_all(&self) -> Result<(), DaqError> {
        let device_ids: Vec<String> = self
            .devices
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        let mut errors = Vec::new();

        for device_id in device_ids {
            if let Err(err) = self.unregister(&device_id).await {
                errors.push(err);
            }
        }

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
            .insert(failure.device_id.clone(), failure);
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
    pub fn report_device_failure(&self, device_id: &str, error: impl Into<String>) {
        let threshold = self.fault_threshold.load(Ordering::Relaxed);
        if let Some(mut state) = self.device_health.get_mut(device_id) {
            state.record_failure(error, threshold);
            tracing::warn!(
                device_id = %device_id,
                health = %state.health,
                consecutive_failures = state.consecutive_failures,
                "Device failure recorded"
            );
        }
    }

    /// Report a successful device operation, resetting failure counters.
    pub fn report_device_success(&self, device_id: &str) {
        if let Some(mut state) = self.device_health.get_mut(device_id) {
            state.record_success();
        }
    }

    /// Get the health state for a specific device.
    pub fn get_device_health(&self, device_id: &str) -> Option<common::health::DeviceHealthState> {
        self.device_health.get(device_id).map(|s| s.clone())
    }

    /// Get health states for all devices.
    pub fn list_device_health(&self) -> Vec<(DeviceId, common::health::DeviceHealthState)> {
        self.device_health
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
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
            .map(|s| s.health == common::health::DeviceHealth::Faulted)
            .unwrap_or(false);

        if !is_faulted {
            return Ok(false);
        }

        // Mark as recovering (increments restart_attempts in the snapshot)
        if let Some(mut state) = self.device_health.get_mut(device_id) {
            state.mark_recovering();
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
                if let Some(prev) = &preserved_health {
                    if let Some(mut new_state) = self.device_health.get_mut(device_id) {
                        new_state.restart_attempts = prev.restart_attempts;
                    }
                }
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
                self.device_health.insert(device_id.to_string(), state);

                // Re-insert a stub device entry so the supervisor can retry.
                // Without this, the config is lost and the device becomes
                // permanently unreachable (devices map empty, health shows Faulted).
                self.devices.insert(
                    device_id.to_string(),
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
                        lifecycle: None,
                        metadata: old_metadata,
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

    /// Get a device as Settable (if it supports this capability)
    pub fn get_settable(&self, id: &str) -> Option<Arc<dyn Settable>> {
        self.devices.get(id).and_then(|d| d.settable.clone())
    }

    /// Get a device as Commandable (if it supports this capability)
    pub fn get_commandable(&self, id: &str) -> Option<Arc<dyn Commandable>> {
        self.devices.get(id).and_then(|d| d.commandable.clone())
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
    ) -> Result<RegisteredDevice> {
        if config.driver.driver_type != "plugin" {
            return Err(anyhow!(
                "Invalid driver type for create_registered_plugin: expected 'plugin', got '{}'",
                config.driver.driver_type
            ));
        }
        let plugin_id = config
            .driver
            .config
            .get("plugin_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'plugin_id' in plugin driver config"))?
            .to_string();
        let driver_type_name = config.driver.driver_type.clone();

        // Introspect capabilities from the plugin configuration
        let factory = self.plugin_factory.read().await;
        let plugin_config = factory
            .get_config(&plugin_id)
            .ok_or_else(|| anyhow!("Plugin '{}' not found in factory", plugin_id))?;

        let mut metadata = DeviceMetadata::default();

        // Check for movable capability
        let movable: Option<Arc<dyn Movable>> = if plugin_config.capabilities.movable.is_some() {
            // Extract metadata from first axis
            if let Some(movable_cap) = &plugin_config.capabilities.movable {
                if let Some(first_axis) = movable_cap.axes.first() {
                    metadata.position_units.clone_from(&first_axis.unit);
                    metadata.min_position = first_axis.min;
                    metadata.max_position = first_axis.max;
                }
            }

            // Create axis handle for the first axis (convention)
            let axis_name = plugin_config
                .capabilities
                .movable
                .as_ref()
                .and_then(|m| m.axes.first())
                .map(|a| a.name.as_str())
                .unwrap_or("axis");

            Some(Arc::new(crate::plugin::handles::PluginAxisHandle::new(
                driver.clone(),
                axis_name.to_string(),
                false, // not mocking
            )))
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

            Some(Arc::new(crate::plugin::handles::PluginSensorHandle::new(
                driver.clone(),
                readable_name.to_string(),
                false, // not mocking
            )))
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
            lifecycle: None,
            metadata,
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
                    snapshot.insert(device_id.clone(), device_params);
                }
            }
        }

        snapshot
    }
}

impl Default for DeviceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Hardware Configuration File Support
// =============================================================================

/// Hardware configuration loaded from a TOML file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareConfig {
    /// Plugin search paths (in priority order, first = highest priority)
    /// Convention: user paths before system paths
    #[serde(default)]
    pub plugin_paths: Vec<std::path::PathBuf>,

    /// List of devices to register
    pub devices: Vec<DeviceConfig>,
}

impl HardwareConfig {
    /// Load hardware configuration from a TOML file
    pub fn from_file(path: &std::path::Path) -> Result<Self, DaqError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            DaqError::Configuration(format!("Failed to read hardware config file: {}", e))
        })?;
        toml::from_str(&content).map_err(|e| {
            DaqError::Configuration(format!("Failed to parse hardware config file: {}", e))
        })
    }
}

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
pub async fn create_registry_from_config(
    config: &HardwareConfig,
    config_dir: Option<&std::path::Path>,
) -> Result<DeviceRegistry, DaqError> {
    let registry = DeviceRegistry::new();

    // Register all available driver factories BEFORE loading devices
    register_all_factories(&registry, config_dir).await?;

    // Load plugins from configured search paths
    #[cfg(feature = "serial")]
    {
        let mut factory = registry.plugin_factory.write().await;
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
                device_id: device_config.id.clone(),
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

    Ok(registry)
}

/// Load hardware configuration from a file and create a DeviceRegistry
pub async fn create_registry_from_file(path: &std::path::Path) -> Result<DeviceRegistry, DaqError> {
    let config = HardwareConfig::from_file(path)?;
    // Default factory directory is alongside the hardware config (config/devices).
    let config_dir = path
        .parent()
        .map(|p| p.join("devices"))
        .filter(|p| p.exists());
    create_registry_from_config(&config, config_dir.as_deref()).await
}

// =============================================================================
// Convenience Functions for Lab Configuration
// =============================================================================

/// Create a DeviceRegistry with mock devices for testing
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
pub fn register_mock_factories(registry: &DeviceRegistry) {
    use driver_mock::{MockCameraFactory, MockPowerMeterFactory, MockStageFactory};

    registry.register_factory(Box::new(MockStageFactory));
    registry.register_factory(Box::new(MockCameraFactory));
    registry.register_factory(Box::new(MockPowerMeterFactory));
}

/// Register all available hardware driver factories.
///
/// This registers factories for all enabled hardware drivers:
/// - Mock drivers (always available)
/// - Thorlabs ELL14 (when `thorlabs` feature enabled)
/// - Newport ESP300 and 1830-C (when `newport` feature enabled)
/// - Spectra-Physics MaiTai (when `spectra_physics` feature enabled)
/// - Config-driven devices from TOML files
///
/// # Example
///
/// ```rust,ignore
/// use daq_hardware::registry::{DeviceRegistry, register_all_factories};
/// use std::path::Path;
///
/// let registry = DeviceRegistry::new();
/// register_all_factories(&registry, Some(Path::new("config/devices"))).await?;
///
/// // Now use register_from_toml() for any supported driver type
/// ```
pub async fn register_all_factories(
    registry: &DeviceRegistry,
    config_dir: Option<&std::path::Path>,
) -> Result<(), DaqError> {
    // Register mock factories (always available)
    register_mock_factories(registry);

    // Register Thorlabs factories
    #[cfg(feature = "thorlabs")]
    {
        use driver_thorlabs::Ell14Factory;
        registry.register_factory(Box::new(Ell14Factory));
    }

    // Register Newport factories
    #[cfg(feature = "newport")]
    {
        use driver_newport::{Esp300Factory, Newport1830CFactory};
        registry.register_factory(Box::new(Esp300Factory));
        registry.register_factory(Box::new(Newport1830CFactory));
    }

    // Register Spectra-Physics factories
    #[cfg(feature = "spectra_physics")]
    {
        use driver_spectra_physics::MaiTaiFactory;
        registry.register_factory(Box::new(MaiTaiFactory));
    }

    // Register Red Pitaya factories
    #[cfg(feature = "red_pitaya")]
    {
        use driver_red_pitaya::RedPitayaPidFactory;
        registry.register_factory(Box::new(RedPitayaPidFactory));
    }

    // Register Andor SDK3 factories (iStar camera + Shamrock spectrograph)
    #[cfg(feature = "andor")]
    {
        use driver_andor_sdk3::{AndorCameraFactory, AndorSpectrographFactory};
        registry.register_factory(Box::new(AndorCameraFactory));
        registry.register_factory(Box::new(AndorSpectrographFactory));
    }

    // Register PVCAM factory
    #[cfg(feature = "pvcam")]
    {
        use driver_pvcam::PvcamFactory;
        registry.register_factory(Box::new(PvcamFactory));
    }

    // Register Comedi factories (NI DAQ, etc.)
    #[cfg(feature = "comedi")]
    {
        use driver_comedi::{ComediAnalogInputFactory, ComediAnalogOutputFactory};
        registry.register_factory(Box::new(ComediAnalogInputFactory));
        registry.register_factory(Box::new(ComediAnalogOutputFactory));
    }

    // Load and register config-driven factories from TOML files (schema_version=3)
    if let Some(dir) = config_dir {
        if dir.exists() {
            match driver_universal::factory::load_all_factories(dir) {
                Ok(factories) => {
                    for factory in factories {
                        let driver_type = factory.driver_type().to_string();
                        registry.register_factory(Box::new(factory));
                        tracing::debug!(driver_type = %driver_type, "Registered universal config factory");
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to load config factories from {}: {}",
                        dir.display(),
                        e
                    );
                }
            }
        }
    }

    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn test_register_mock_devices() {
        let registry = create_mock_registry().await.unwrap();

        assert_eq!(registry.len(), 3);
        assert!(registry.contains("mock_stage"));
        assert!(registry.contains("mock_power_meter"));
        assert!(registry.contains("mock_camera"));
    }

    #[tokio::test]
    async fn test_list_devices() {
        let registry = create_mock_registry().await.unwrap();
        let devices = registry.list_devices();

        assert_eq!(devices.len(), 3);

        let stage = devices.iter().find(|d| d.id == "mock_stage").unwrap();
        assert_eq!(stage.driver_type, "mock_stage");
        assert!(stage.capabilities.contains(&Capability::Movable));

        let meter = devices.iter().find(|d| d.id == "mock_power_meter").unwrap();
        assert_eq!(meter.driver_type, "mock_power_meter");
        assert!(meter.capabilities.contains(&Capability::Readable));

        let camera = devices.iter().find(|d| d.id == "mock_camera").unwrap();
        assert_eq!(camera.driver_type, "mock_camera");
        assert!(camera.capabilities.contains(&Capability::FrameProducer));
        assert!(camera.capabilities.contains(&Capability::Triggerable));
        assert!(camera.capabilities.contains(&Capability::ExposureControl));
    }

    #[tokio::test]
    async fn test_legacy_toml_config_registers_mock_devices() {
        let toml_str = r#"
[[devices]]
id = "legacy_stage"
name = "Legacy Stage"
[devices.driver]
type = "mock_stage"
initial_position = 1.23

[[devices]]
id = "legacy_camera"
name = "Legacy Camera"
[devices.driver]
type = "mock_camera"
width = 320
height = 240
"#;

        let config: HardwareConfig = toml::from_str(toml_str).unwrap();
        let registry = create_registry_from_config(&config, None).await.unwrap();

        let devices = registry.list_devices();
        assert_eq!(devices.len(), 2);

        let stage = devices.iter().find(|d| d.id == "legacy_stage").unwrap();
        assert_eq!(stage.driver_type, "mock_stage");
        assert!(stage.capabilities.contains(&Capability::Movable));

        let camera = devices.iter().find(|d| d.id == "legacy_camera").unwrap();
        assert_eq!(camera.driver_type, "mock_camera");
        assert!(camera.capabilities.contains(&Capability::FrameProducer));
        assert!(camera.capabilities.contains(&Capability::Triggerable));
        assert!(camera.capabilities.contains(&Capability::ExposureControl));

        assert!(registry.get_movable("legacy_stage").is_some());
        assert!(registry.get_frame_producer("legacy_camera").is_some());
    }

    #[tokio::test]
    async fn test_factory_only_path() {
        // All devices must go through factory registration
        let toml_str = r#"
[[devices]]
id = "test_device"
name = "Test Device With Factory"

[devices.driver]
type = "mock_stage"
initial_position = 0.0
"#;

        let config: HardwareConfig = toml::from_str(toml_str).unwrap();

        // MockStageFactory is registered by register_mock_factories(),
        // so this should succeed
        let result = create_registry_from_config(&config, None).await;
        assert!(result.is_ok(), "Should succeed when factory exists");

        let registry = result.unwrap();
        let devices = registry.list_devices();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "test_device");
    }

    #[tokio::test]
    async fn test_factory_path_logging() {
        // Test that logging distinguishes between factory and legacy paths
        // This is a smoke test - actual verification would require log capture
        let toml_str = r#"
[[devices]]
id = "factory_device"
name = "Device Using Factory"

[devices.driver]
type = "mock_stage"
initial_position = 0.0
"#;

        let config: HardwareConfig = toml::from_str(toml_str).unwrap();
        let registry = create_registry_from_config(&config, None).await.unwrap();

        let devices = registry.list_devices();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "factory_device");
    }

    #[tokio::test]
    async fn test_get_movable() {
        let registry = create_mock_registry().await.unwrap();

        let movable = registry.get_movable("mock_stage");
        assert!(movable.is_some());

        let not_movable = registry.get_movable("mock_power_meter");
        assert!(not_movable.is_none());
    }

    #[tokio::test]
    async fn test_get_readable() {
        let registry = create_mock_registry().await.unwrap();

        let readable = registry.get_readable("mock_power_meter");
        assert!(readable.is_some());

        let not_readable = registry.get_readable("mock_stage");
        assert!(not_readable.is_none());
    }

    #[tokio::test]
    async fn test_devices_with_capability() {
        let registry = create_mock_registry().await.unwrap();

        let movables = registry.devices_with_capability(Capability::Movable);
        assert_eq!(movables.len(), 1);
        assert!(movables.contains(&"mock_stage".to_string()));

        let readables = registry.devices_with_capability(Capability::Readable);
        assert_eq!(readables.len(), 1);
        assert!(readables.contains(&"mock_power_meter".to_string()));
    }

    #[tokio::test]
    async fn test_duplicate_registration_fails() {
        let registry = DeviceRegistry::new();
        register_mock_factories(&registry);

        registry
            .register_from_toml(
                "test",
                "Test Device",
                "mock_stage",
                toml::Value::Table({
                    let mut m = toml::map::Map::new();
                    m.insert("initial_position".into(), toml::Value::Float(0.0));
                    m
                }),
            )
            .await
            .unwrap();

        let result = registry
            .register_from_toml(
                "test",
                "Duplicate",
                "mock_stage",
                toml::Value::Table({
                    let mut m = toml::map::Map::new();
                    m.insert("initial_position".into(), toml::Value::Float(0.0));
                    m
                }),
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_unregister() {
        let registry = create_mock_registry().await.unwrap();

        assert!(registry.contains("mock_stage"));
        assert!(registry.unregister("mock_stage").await.unwrap());
        assert!(!registry.contains("mock_stage"));
        assert!(!registry.unregister("mock_stage").await.unwrap()); // Already removed
    }

    struct TestLifecycle {
        registered: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        unregistered: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl common::driver::DeviceLifecycle for TestLifecycle {
        fn on_register(&self) -> futures::future::BoxFuture<'static, Result<()>> {
            let counter = self.registered.clone();
            Box::pin(async move {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
        }

        fn on_unregister(&self) -> futures::future::BoxFuture<'static, Result<()>> {
            let counter = self.unregistered.clone();
            Box::pin(async move {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
        }
    }

    struct TestFactory {
        lifecycle: std::sync::Arc<dyn common::driver::DeviceLifecycle>,
    }

    impl common::driver::DriverFactory for TestFactory {
        fn driver_type(&self) -> &'static str {
            "test_factory"
        }

        fn name(&self) -> &'static str {
            "Test Factory"
        }

        fn validate(&self, _config: &toml::Value) -> Result<()> {
            Ok(())
        }

        fn build(
            &self,
            _config: toml::Value,
        ) -> futures::future::BoxFuture<'static, Result<DeviceComponents>> {
            let lifecycle = self.lifecycle.clone();
            Box::pin(async move {
                let driver = std::sync::Arc::new(crate::drivers::mock::MockStage::new());
                Ok(DeviceComponents::new()
                    .with_movable(driver.clone())
                    .with_parameterized(driver)
                    .with_lifecycle(lifecycle))
            })
        }
    }

    struct LifecycleFactory {
        driver_type: &'static str,
        lifecycle: std::sync::Arc<dyn common::driver::DeviceLifecycle>,
    }

    impl common::driver::DriverFactory for LifecycleFactory {
        fn driver_type(&self) -> &'static str {
            self.driver_type
        }

        fn name(&self) -> &'static str {
            "Lifecycle Factory"
        }

        fn validate(&self, _config: &toml::Value) -> Result<()> {
            Ok(())
        }

        fn build(
            &self,
            _config: toml::Value,
        ) -> futures::future::BoxFuture<'static, Result<DeviceComponents>> {
            let lifecycle = self.lifecycle.clone();
            Box::pin(async move {
                let driver = std::sync::Arc::new(crate::drivers::mock::MockStage::new());
                Ok(DeviceComponents::new()
                    .with_movable(driver.clone())
                    .with_parameterized(driver)
                    .with_lifecycle(lifecycle))
            })
        }
    }

    #[tokio::test]
    async fn test_lifecycle_hooks_on_register_unregister() {
        let registered = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let unregistered = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let lifecycle = std::sync::Arc::new(TestLifecycle {
            registered: registered.clone(),
            unregistered: unregistered.clone(),
        });

        let registry = DeviceRegistry::new();
        registry.register_factory(Box::new(TestFactory {
            lifecycle: lifecycle.clone(),
        }));

        registry
            .register_from_toml(
                "test-device",
                "Test Device",
                "test_factory",
                toml::Value::Table(toml::map::Map::new()),
            )
            .await
            .unwrap();

        assert_eq!(registered.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(registry.unregister("test-device").await.unwrap());
        assert_eq!(unregistered.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    struct FailingLifecycle {
        unregistered: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl common::driver::DeviceLifecycle for FailingLifecycle {
        fn on_register(&self) -> futures::future::BoxFuture<'static, Result<()>> {
            Box::pin(async { Err(anyhow!("boom")) })
        }

        fn on_unregister(&self) -> futures::future::BoxFuture<'static, Result<()>> {
            let counter = self.unregistered.clone();
            Box::pin(async move {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn test_failed_lifecycle_register_cleans_up() {
        let unregistered = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let lifecycle = std::sync::Arc::new(FailingLifecycle {
            unregistered: unregistered.clone(),
        });

        let registry = DeviceRegistry::new();
        registry.register_factory(Box::new(TestFactory { lifecycle }));

        let result = registry
            .register_from_toml(
                "test-device",
                "Test Device",
                "test_factory",
                toml::Value::Table(toml::map::Map::new()),
            )
            .await;

        assert!(matches!(result, Err(DaqError::Driver(_))));
        assert!(!registry.contains("test-device"));
        assert_eq!(unregistered.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    struct CountingLifecycle {
        unregistered: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        fail_on_unregister: bool,
    }

    impl common::driver::DeviceLifecycle for CountingLifecycle {
        fn on_unregister(&self) -> futures::future::BoxFuture<'static, Result<()>> {
            let counter = self.unregistered.clone();
            let fail = self.fail_on_unregister;
            Box::pin(async move {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if fail {
                    Err(anyhow!("boom"))
                } else {
                    Ok(())
                }
            })
        }
    }

    #[tokio::test]
    async fn test_shutdown_all_attempts_all_unregister_hooks() {
        let ok_unregistered = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let fail_unregistered = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let ok_lifecycle = std::sync::Arc::new(CountingLifecycle {
            unregistered: ok_unregistered.clone(),
            fail_on_unregister: false,
        });
        let fail_lifecycle = std::sync::Arc::new(CountingLifecycle {
            unregistered: fail_unregistered.clone(),
            fail_on_unregister: true,
        });

        let registry = DeviceRegistry::new();
        registry.register_factory(Box::new(LifecycleFactory {
            driver_type: "test_factory_ok",
            lifecycle: ok_lifecycle,
        }));
        registry.register_factory(Box::new(LifecycleFactory {
            driver_type: "test_factory_fail",
            lifecycle: fail_lifecycle,
        }));

        registry
            .register_from_toml(
                "test-device-ok",
                "Test Device Ok",
                "test_factory_ok",
                toml::Value::Table(toml::map::Map::new()),
            )
            .await
            .unwrap();
        registry
            .register_from_toml(
                "test-device-fail",
                "Test Device Fail",
                "test_factory_fail",
                toml::Value::Table(toml::map::Map::new()),
            )
            .await
            .unwrap();

        let result = registry.shutdown_all().await;
        let Err(DaqError::ShutdownFailed(errors)) = result else {
            panic!("Expected ShutdownFailed error");
        };

        assert_eq!(errors.len(), 1);
        assert!(!registry.contains("test-device-ok"));
        assert!(!registry.contains("test-device-fail"));
        assert_eq!(ok_unregistered.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            fail_unregistered.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn test_capability_access() {
        let registry = create_mock_registry().await.unwrap();

        // Test that we can use the movable interface
        let movable = registry.get_movable("mock_stage").unwrap();
        movable.move_abs(10.0).await.unwrap();
        let pos = movable.position().await.unwrap();
        assert!((pos - 10.0).abs() < 0.001);

        // Test that we can use the readable interface
        // MockPowerMeter noise model: shot_noise = 0.01 * sqrt(power) = 0.01 * sqrt(1e-6) = 1e-5
        // Use fixed tolerance of 1.5e-5 (1.5x max shot noise) to account for thermal floor
        let readable = registry.get_readable("mock_power_meter").unwrap();
        let reading = readable.read().await.unwrap();
        assert!(
            (reading - 1e-6).abs() < 1.5e-5,
            "Reading {} deviates more than 1.5e-5 from base 1e-6",
            reading
        );
    }

    #[tokio::test]
    async fn test_snapshot_all_parameters() {
        let registry = create_mock_registry().await.unwrap();

        // Snapshot all parameters
        let snapshot = registry.snapshot_all_parameters();

        // Should have parameters from both mock devices
        assert!(!snapshot.is_empty(), "Snapshot should not be empty");

        // Mock devices implement Parameterized, so they should have parameters
        assert!(
            snapshot.contains_key("mock_stage") || snapshot.contains_key("mock_power_meter"),
            "Snapshot should contain at least one device"
        );

        // If a device is present, its parameters should be serializable JSON values
        for (device_id, params) in &snapshot {
            assert!(
                !params.is_empty(),
                "Device {} should have parameters",
                device_id
            );
            for (param_name, value) in params {
                assert!(
                    value.is_number()
                        || value.is_string()
                        || value.is_boolean()
                        || value.is_object(),
                    "Parameter {}.{} should be a valid JSON value",
                    device_id,
                    param_name
                );
            }
        }
    }

    #[cfg(feature = "serial")]
    #[tokio::test]
    async fn test_plugin_device_registration() {
        use std::sync::Arc;
        use tokio::sync::RwLock;

        // Create a plugin factory and registry
        let factory = Arc::new(RwLock::new(crate::plugin::registry::PluginFactory::new()));
        let registry = DeviceRegistry::with_plugin_factory(factory.clone());

        // Note: This test verifies that the plugin infrastructure is wired up correctly.
        // Actual plugin loading requires YAML files, which would be in integration tests.

        // Verify that we can access the plugin factory
        let factory_ref = registry.plugin_factory();
        assert!(Arc::ptr_eq(&factory, &factory_ref));

        // Verify that the registry starts empty
        assert_eq!(registry.len(), 0);
    }

    #[tokio::test]
    async fn test_register_fails_on_unknown_driver_type() {
        let registry = DeviceRegistry::new();
        register_mock_factories(&registry);

        let result = registry
            .register_from_toml(
                "invalid_device",
                "Invalid Device",
                "nonexistent_driver_type",
                toml::Value::Table(Default::default()),
            )
            .await;

        assert!(result.is_err());

        // Registry should remain empty
        assert_eq!(registry.len(), 0);
    }

    #[tokio::test]
    async fn test_mock_camera_in_registry() {
        let registry = create_mock_registry().await.unwrap();

        // Verify mock_camera is registered
        assert!(registry.contains("mock_camera"));

        // Verify it has the expected capabilities through capability getters
        let frame_producer = registry.get_frame_producer("mock_camera");
        assert!(
            frame_producer.is_some(),
            "MockCamera should be retrievable as FrameProducer"
        );

        let triggerable = registry.get_triggerable("mock_camera");
        assert!(
            triggerable.is_some(),
            "MockCamera should be retrievable as Triggerable"
        );

        let exposure_control = registry.get_exposure_control("mock_camera");
        assert!(
            exposure_control.is_some(),
            "MockCamera should be retrievable as ExposureControl"
        );

        // Verify device info includes all capabilities
        let device_info = registry.get_device_info("mock_camera").unwrap();
        assert!(device_info
            .capabilities
            .contains(&Capability::FrameProducer));
        assert!(device_info.capabilities.contains(&Capability::Triggerable));
        assert!(device_info
            .capabilities
            .contains(&Capability::ExposureControl));
        assert_eq!(device_info.driver_type, "mock_camera");

        // Test that we can get parameters (bd-pf31: use get_parameterized)
        let parameterized = registry.get_parameterized("mock_camera").unwrap();
        let params = parameterized.parameters();
        assert!(params.get("exposure_s").is_some());
        assert!(params.get("armed").is_some());
        assert!(params.get("streaming").is_some());
        assert!(params.get("staged").is_some());
    }

    #[tokio::test]
    async fn test_get_device_info_nonexistent() {
        let registry = create_mock_registry().await.unwrap();
        let info = registry.get_device_info("nonexistent");
        assert!(info.is_none());
    }

    #[tokio::test]
    async fn test_registry_len() {
        let registry = DeviceRegistry::new();
        assert_eq!(registry.len(), 0);

        register_mock_factories(&registry);
        registry
            .register_from_toml(
                "test1",
                "Test 1",
                "mock_stage",
                toml::Value::Table(Default::default()),
            )
            .await
            .unwrap();
        assert_eq!(registry.len(), 1);

        registry
            .register_from_toml(
                "test2",
                "Test 2",
                "mock_stage",
                toml::Value::Table(Default::default()),
            )
            .await
            .unwrap();
        assert_eq!(registry.len(), 2);

        registry.unregister("test1").await.unwrap();
        assert_eq!(registry.len(), 1);
    }

    #[tokio::test]
    async fn test_capability_getters_return_none_for_wrong_type() {
        let registry = create_mock_registry().await.unwrap();

        // mock_stage is not readable
        assert!(registry.get_readable("mock_stage").is_none());

        // mock_power_meter is not movable
        assert!(registry.get_movable("mock_power_meter").is_none());

        // mock_stage is not a frame producer
        assert!(registry.get_frame_producer("mock_stage").is_none());
    }

    #[tokio::test]
    async fn test_devices_with_capability_empty_result() {
        let registry = create_mock_registry().await.unwrap();

        // No devices with WavelengthTunable capability in mock registry
        let tunable = registry.devices_with_capability(Capability::WavelengthTunable);
        assert_eq!(tunable.len(), 0);
    }

    #[tokio::test]
    async fn test_shutdown_all_empty_registry() {
        let registry = DeviceRegistry::new();
        let result = registry.shutdown_all().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_multiple_capabilities_on_single_device() {
        let registry = create_mock_registry().await.unwrap();

        // mock_camera has multiple capabilities
        let frame_producer = registry.get_frame_producer("mock_camera");
        let triggerable = registry.get_triggerable("mock_camera");
        let exposure_control = registry.get_exposure_control("mock_camera");
        let parameterized = registry.get_parameterized("mock_camera");

        assert!(frame_producer.is_some());
        assert!(triggerable.is_some());
        assert!(exposure_control.is_some());
        assert!(parameterized.is_some());
    }

    #[tokio::test]
    async fn test_factory_validation_failure() {
        let registry = DeviceRegistry::new();
        register_mock_factories(&registry);

        // Try to register with invalid config (driver expects valid types)
        let invalid_config = toml::Value::try_from(toml::toml! {
            invalid_field = "this_should_not_exist"
        })
        .unwrap();

        // The validation should happen before registration
        let result = registry
            .register_from_toml("invalid", "Invalid Device", "mock_stage", invalid_config)
            .await;

        // Result depends on whether the factory validates strictly
        // If validation passes, device should be registered
        if result.is_ok() {
            assert!(registry.contains("invalid"));
        } else {
            assert!(!registry.contains("invalid"));
        }
    }

    #[tokio::test]
    async fn test_get_parameterized_for_all_devices() {
        let registry = create_mock_registry().await.unwrap();

        // All mock devices should implement Parameterized
        let devices = registry.list_devices();
        for device in devices {
            let parameterized = registry.get_parameterized(&device.id);
            assert!(
                parameterized.is_some(),
                "Device {} should be parameterized",
                device.id
            );
        }
    }

    #[tokio::test]
    async fn test_list_devices_empty_registry() {
        let registry = DeviceRegistry::new();
        let devices = registry.list_devices();
        assert_eq!(devices.len(), 0);
    }

    #[tokio::test]
    async fn test_unregister_nonexistent_device() {
        let registry = DeviceRegistry::new();
        let result = registry.unregister("nonexistent").await.unwrap();
        assert!(!result, "Should return false for nonexistent device");
    }

    #[tokio::test]
    async fn test_device_metadata_preserved() {
        let registry = create_mock_registry().await.unwrap();
        let device_info = registry.get_device_info("mock_stage").unwrap();

        assert_eq!(device_info.id, "mock_stage");
        assert_eq!(device_info.name, "Mock Stage");
        assert_eq!(device_info.driver_type, "mock_stage");
    }
}
