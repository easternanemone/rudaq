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
//! use rust_daq::hardware::registry::{DeviceRegistry, DeviceConfig, DriverType};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut registry = DeviceRegistry::new();
//!
//!     // Register devices from configuration
//!     registry.register(DeviceConfig {
//!         id: "power_meter".into(),
//!         name: "Newport 1830-C".into(),
//!         driver: DriverType::Newport1830C { port: "/dev/ttyS0".into() },
//!     }).await?;
//!
//!     registry.register(DeviceConfig {
//!         id: "rotator_2".into(),
//!         name: "ELL14 Address 2".into(),
//!         driver: DriverType::Ell14 {
//!             port: "/dev/ttyUSB0".into(),
//!             address: "2".into(),
//!         },
//!     }).await?;
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

#[cfg(feature = "instrument_spectra_physics")]
use crate::hardware::capabilities::{EmissionControl, ShutterControl, WavelengthTunable};
use crate::hardware::capabilities::{
    ExposureControl, FrameProducer, Movable, Parameterized, Readable, Settable, Stageable,
    Triggerable,
};
#[cfg(feature = "tokio_serial")]
use crate::hardware::plugin::driver::GenericDriver;
use crate::observable::ParameterSet; // NEW: For parameter registry
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "tokio_serial")]
use tokio::sync::RwLock;

// =============================================================================
// Configuration Validation
// =============================================================================

/// Validate a driver configuration before attempting to instantiate
///
/// This checks for common configuration errors that would cause driver spawn to fail:
/// - Serial ports that don't exist
/// - Invalid baud rates (if applicable)
/// - Invalid device addresses
/// - Missing required fields
///
/// Returns Ok(()) if configuration is valid, or an error with helpful diagnostics.
pub fn validate_driver_config(driver: &DriverType) -> Result<()> {
    match driver {
        #[cfg(feature = "instrument_serial")]
        DriverType::Newport1830C { port } => {
            validate_serial_port(port, "Newport 1830-C")?;
        }

        #[cfg(feature = "instrument_serial")]
        DriverType::MaiTai { port } => {
            validate_serial_port(port, "MaiTai Laser")?;
        }

        #[cfg(feature = "instrument_serial")]
        DriverType::Ell14 { port, address } => {
            validate_serial_port(port, "ELL14 Rotation Mount")?;
            validate_ell14_address(address)?;
        }

        #[cfg(feature = "instrument_serial")]
        DriverType::Esp300 { port, axis } => {
            validate_serial_port(port, "ESP300 Motion Controller")?;
            if *axis < 1 || *axis > 3 {
                anyhow::bail!("Invalid ESP300 axis: {}. Must be 1-3", axis);
            }
        }

        DriverType::Pvcam { camera_name } => {
            if camera_name.is_empty() {
                anyhow::bail!("PVCAM camera name cannot be empty");
            }
        }

        #[cfg(feature = "tokio_serial")]
        DriverType::Plugin { plugin_id, address } => {
            if plugin_id.is_empty() {
                anyhow::bail!("Plugin ID cannot be empty");
            }
            if address.is_empty() {
                anyhow::bail!("Plugin address cannot be empty");
            }
            // Address can be serial port or network address
            // Don't validate serial port here as it might be network
        }

        // Mock devices don't need validation
        DriverType::MockStage { .. }
        | DriverType::MockPowerMeter { .. }
        | DriverType::MockCamera => {}
    }

    Ok(())
}

/// Validate that a serial port exists and is accessible
///
/// Provides helpful error messages listing available ports if the requested port is not found.
#[cfg(feature = "instrument_serial")]
fn validate_serial_port(port: &str, device_name: &str) -> Result<()> {
    // Check if port path exists (basic check)
    let port_path = std::path::Path::new(port);

    if !port_path.exists() {
        // Port doesn't exist - provide helpful diagnostics
        let available = match serialport::available_ports() {
            Ok(ports) => {
                if ports.is_empty() {
                    "No serial ports detected on this system".to_string()
                } else {
                    let port_list: Vec<String> = ports
                        .iter()
                        .map(|p| format!("  - {}", p.port_name))
                        .collect();
                    format!("Available serial ports:\n{}", port_list.join("\n"))
                }
            }
            Err(e) => {
                format!("Could not enumerate serial ports: {}", e)
            }
        };

        anyhow::bail!(
            "Serial port '{}' does not exist for device '{}'.\n\n{}\n\n\
             Troubleshooting:\n\
             - Verify device is connected and powered on\n\
             - Check USB cable connection\n\
             - On Linux, ensure you have permissions (add user to 'dialout' group)\n\
             - On macOS, check /dev/tty.* and /dev/cu.* devices\n\
             - Run 'ls /dev/tty*' or 'ls /dev/cu*' to list available ports",
            port,
            device_name,
            available
        );
    }

    Ok(())
}

/// Stub validator when serial support is disabled.
#[cfg(not(feature = "instrument_serial"))]
fn validate_serial_port(_port: &str, _device_name: &str) -> Result<()> {
    // Serial devices are not available without the instrument_serial feature enabled.
    // Validation is a no-op so builds without serialport dependency succeed.
    Ok(())
}

/// Validate ELL14 device address
///
/// ELL14 addresses must be hex digits 0-F
fn validate_ell14_address(address: &str) -> Result<()> {
    if address.len() != 1 {
        anyhow::bail!(
            "Invalid ELL14 address '{}': must be a single hex digit (0-F)",
            address
        );
    }

    let addr_char = address.chars().next().unwrap();
    if !addr_char.is_ascii_hexdigit() {
        anyhow::bail!(
            "Invalid ELL14 address '{}': must be a hex digit (0-9, A-F)",
            address
        );
    }

    Ok(())
}

// =============================================================================
// Device Identification
// =============================================================================

/// Unique identifier for a registered device
///
/// Format: lowercase alphanumeric with underscores (e.g., "power_meter", "rotator_2")
pub type DeviceId = String;

/// Capabilities a device can have (for introspection)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// Can move to positions (stages, rotation mounts)
    Movable,
    /// Can read scalar values (power meters, temperature sensors)
    Readable,
    /// Can be armed and triggered (cameras, pulse generators)
    Triggerable,
    /// Produces image frames (cameras)
    FrameProducer,
    /// Has exposure/integration time control
    ExposureControl,
    /// Has settable parameters (QCodes/ScopeFoundry pattern)
    Settable,
    /// Has shutter control (lasers) - bd-pwjo
    ShutterControl,
    /// Has wavelength tuning (tunable lasers) - bd-pwjo
    WavelengthTunable,
    /// Has emission on/off control (lasers) - bd-pwjo
    EmissionControl,
    /// Can be staged/unstaged for acquisition sequences (Bluesky pattern) - bd-7aq6
    Stageable,
}

// =============================================================================
// Driver Types (Configuration)
// =============================================================================

/// Driver configuration for instantiating hardware
///
/// Each variant corresponds to a hardware driver with its required configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DriverType {
    /// Newport 1830-C Optical Power Meter
    #[cfg(feature = "instrument_serial")]
    Newport1830C {
        /// Serial port path (e.g., "/dev/ttyS0")
        port: String,
    },

    /// Spectra-Physics MaiTai Ti:Sapphire Laser
    #[cfg(feature = "instrument_serial")]
    MaiTai {
        /// Serial port path (e.g., "/dev/ttyUSB5")
        port: String,
    },

    /// Thorlabs Elliptec ELL14 Rotation Mount
    #[cfg(feature = "instrument_serial")]
    Ell14 {
        /// Serial port path (e.g., "/dev/ttyUSB0")
        port: String,
        /// Device address on multidrop bus ("0"-"F", typically "2", "3", or "8")
        address: String,
    },

    /// Newport ESP300 Multi-Axis Motion Controller
    #[cfg(feature = "instrument_serial")]
    Esp300 {
        /// Serial port path (e.g., "/dev/ttyUSB1")
        port: String,
        /// Axis number (1-3)
        axis: u8,
    },

    /// Mock stage for testing (always available)
    MockStage {
        /// Initial position
        initial_position: f64,
    },

    /// Mock power meter for testing (always available)
    MockPowerMeter {
        /// Fixed reading value
        reading: f64,
    },

    /// Mock camera for testing (FrameProducer + Triggerable + ExposureControl)
    MockCamera,

    /// Photometrics PVCAM camera driver (software or hardware backed)
    Pvcam {
        /// Camera name (e.g., "PrimeBSI", "PMCam")
        camera_name: String,
    },

    /// Plugin-based device loaded from YAML configuration
    #[cfg(feature = "tokio_serial")]
    Plugin {
        /// Plugin ID from YAML metadata.id (e.g., "my-sensor-v1")
        plugin_id: String,
        /// Connection address (serial port path or TCP "host:port")
        address: String,
    },
}

impl DriverType {
    /// Get the capabilities this driver type provides
    pub fn capabilities(&self) -> Vec<Capability> {
        match self {
            #[cfg(feature = "instrument_serial")]
            DriverType::Newport1830C { .. } => vec![Capability::Readable],
            #[cfg(feature = "instrument_serial")]
            DriverType::MaiTai { .. } => vec![Capability::Readable],
            #[cfg(feature = "instrument_serial")]
            DriverType::Ell14 { .. } => vec![Capability::Movable],
            #[cfg(feature = "instrument_serial")]
            DriverType::Esp300 { .. } => vec![Capability::Movable],
            DriverType::MockStage { .. } => vec![Capability::Movable],
            DriverType::MockPowerMeter { .. } => vec![Capability::Readable],
            DriverType::MockCamera => vec![
                Capability::FrameProducer,
                Capability::Triggerable,
                Capability::ExposureControl,
            ],
            DriverType::Pvcam { .. } => vec![
                Capability::FrameProducer,
                Capability::Triggerable,
                Capability::ExposureControl,
            ],
            #[cfg(feature = "tokio_serial")]
            DriverType::Plugin { .. } => {
                // Note: Plugin capabilities are determined at runtime from YAML
                // This returns an empty vec, but actual capabilities are introspected
                // during registration via PluginFactory
                vec![]
            }
        }
    }

    /// Get human-readable driver type name
    pub fn driver_name(&self) -> &'static str {
        match self {
            #[cfg(feature = "instrument_serial")]
            DriverType::Newport1830C { .. } => "newport_1830c",
            #[cfg(feature = "instrument_serial")]
            DriverType::MaiTai { .. } => "maitai",
            #[cfg(feature = "instrument_serial")]
            DriverType::Ell14 { .. } => "ell14",
            #[cfg(feature = "instrument_serial")]
            DriverType::Esp300 { .. } => "esp300",
            DriverType::MockStage { .. } => "mock_stage",
            DriverType::MockPowerMeter { .. } => "mock_power_meter",
            DriverType::MockCamera => "mock_camera",
            DriverType::Pvcam { .. } => "pvcam",
            #[cfg(feature = "tokio_serial")]
            DriverType::Plugin { plugin_id, .. } => {
                // Note: This is a generic name; actual plugin name is stored in plugin_id
                "plugin"
            }
        }
    }
}

// =============================================================================
// Device Configuration
// =============================================================================

/// Configuration for registering a device
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    /// Unique identifier (e.g., "power_meter", "rotator_2")
    pub id: DeviceId,
    /// Human-readable name (e.g., "Newport 1830-C Power Meter")
    pub name: String,
    /// Driver type and configuration
    pub driver: DriverType,
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
    /// Movable implementation (if supported)
    movable: Option<Arc<dyn Movable>>,
    /// Readable implementation (if supported)
    readable: Option<Arc<dyn Readable>>,
    /// Triggerable implementation (if supported)
    triggerable: Option<Arc<dyn Triggerable>>,
    /// FrameProducer implementation (if supported)
    frame_producer: Option<Arc<dyn FrameProducer>>,
    /// ExposureControl implementation (if supported)
    exposure_control: Option<Arc<dyn ExposureControl>>,
    /// Settable implementation (if supported) - observable parameters
    settable: Option<Arc<dyn Settable>>,
    /// Stageable implementation (if supported) - Bluesky-style lifecycle (bd-7aq6)
    stageable: Option<Arc<dyn Stageable>>,
    /// Parameterized implementation (if supported) - parameter registry access
    ///
    /// Enables generic code to enumerate and subscribe to device parameters.
    /// Populated during device registration if driver implements Parameterized trait.
    parameterized: Option<Arc<dyn Parameterized>>,
    /// ShutterControl implementation (if supported) - laser shutter
    #[cfg(feature = "instrument_spectra_physics")]
    shutter_control: Option<Arc<dyn ShutterControl>>,
    /// EmissionControl implementation (if supported) - laser on/off
    #[cfg(feature = "instrument_spectra_physics")]
    emission_control: Option<Arc<dyn EmissionControl>>,
    /// WavelengthTunable implementation (if supported) - tunable laser wavelength (bd-pwjo)
    #[cfg(feature = "instrument_spectra_physics")]
    wavelength_tunable: Option<Arc<dyn WavelengthTunable>>,
    /// Device metadata (units, ranges, etc.)
    metadata: DeviceMetadata,
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
pub struct DeviceRegistry {
    /// Registered devices by ID
    devices: HashMap<DeviceId, RegisteredDevice>,

    /// Shared serial ports for ELL14 multidrop bus (interior mutability for async access)
    /// Key: port path (e.g., "/dev/ttyUSB0"), Value: shared Arc<Mutex<SerialStream>>
    #[cfg(feature = "instrument_thorlabs")]
    ell14_shared_ports: RwLock<HashMap<String, crate::hardware::ell14::SharedPort>>,

    /// Plugin factory for loading YAML-defined drivers (tokio_serial feature only)
    #[cfg(feature = "tokio_serial")]
    plugin_factory: Arc<RwLock<crate::hardware::plugin::registry::PluginFactory>>,
}

impl DeviceRegistry {
    /// Create a new empty device registry
    pub fn new() -> Self {
        Self {
            devices: HashMap::new(),
            #[cfg(feature = "instrument_thorlabs")]
            ell14_shared_ports: RwLock::new(HashMap::new()),
            #[cfg(feature = "tokio_serial")]
            plugin_factory: Arc::new(RwLock::new(
                crate::hardware::plugin::registry::PluginFactory::new(),
            )),
        }
    }

    /// Create a new device registry with a pre-configured PluginFactory
    #[cfg(feature = "tokio_serial")]
    pub fn with_plugin_factory(
        plugin_factory: Arc<RwLock<crate::hardware::plugin::registry::PluginFactory>>,
    ) -> Self {
        Self {
            devices: HashMap::new(),
            #[cfg(feature = "instrument_thorlabs")]
            ell14_shared_ports: RwLock::new(HashMap::new()),
            plugin_factory,
        }
    }

    /// Get a reference to the plugin factory
    #[cfg(feature = "tokio_serial")]
    pub fn plugin_factory(&self) -> Arc<RwLock<crate::hardware::plugin::registry::PluginFactory>> {
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
    #[cfg(feature = "tokio_serial")]
    pub async fn load_plugins(&self, path: &std::path::Path) -> Result<()> {
        let mut factory = self.plugin_factory.write().await;
        factory.load_plugins(path).await
    }

    /// Register a device from configuration
    ///
    /// This instantiates the hardware driver and registers it in the registry.
    ///
    /// # Arguments
    /// * `config` - Device configuration including driver type
    ///
    /// # Errors
    /// Returns error if:
    /// - Device ID is already registered
    /// - Configuration validation fails (missing ports, invalid parameters)
    /// - Hardware driver fails to initialize
    pub async fn register(&mut self, config: DeviceConfig) -> Result<()> {
        if self.devices.contains_key(&config.id) {
            return Err(anyhow!("Device '{}' is already registered", config.id));
        }

        // Validate configuration before attempting to instantiate
        validate_driver_config(&config.driver).map_err(|e| {
            anyhow!(
                "Configuration validation failed for device '{}' ({}): {}",
                config.id,
                config.driver.driver_name(),
                e
            )
        })?;

        let registered = self.instantiate_device(config).await?;
        self.devices
            .insert(registered.config.id.clone(), registered);
        Ok(())
    }

    /// Register a pre-spawned plugin instance
    ///
    /// This is used by the PluginService to register drivers that it manages.
    /// It bypasses the normal driver instantiation process.
    ///
    /// # Arguments
    /// * `config` - Device configuration (must be DriverType::Plugin)
    /// * `driver` - The pre-spawned GenericDriver instance
    ///
    /// # Errors
    /// Returns error if the device ID is already registered
    #[cfg(feature = "tokio_serial")]
    pub async fn register_plugin_instance(
        &mut self,
        config: DeviceConfig,
        driver: Arc<GenericDriver>,
    ) -> Result<()> {
        if self.devices.contains_key(&config.id) {
            return Err(anyhow!("Device '{}' is already registered", config.id));
        }

        let registered = self.create_registered_plugin(config, driver).await?;
        self.devices
            .insert(registered.config.id.clone(), registered);
        Ok(())
    }

    /// Unregister a device
    ///
    /// # Arguments
    /// * `id` - Device ID to remove
    ///
    /// # Returns
    /// true if device was found and removed, false if not found
    pub fn unregister(&mut self, id: &str) -> bool {
        self.devices.remove(id).is_some()
    }

    /// List all registered devices
    pub fn list_devices(&self) -> Vec<DeviceInfo> {
        self.devices
            .values()
            .map(|d| DeviceInfo {
                id: d.config.id.clone(),
                name: d.config.name.clone(),
                driver_type: d.config.driver.driver_name().to_string(),
                capabilities: d.config.driver.capabilities(),
                metadata: d.metadata.clone(),
            })
            .collect()
    }

    /// Get device info by ID
    pub fn get_device_info(&self, id: &str) -> Option<DeviceInfo> {
        self.devices.get(id).map(|d| DeviceInfo {
            id: d.config.id.clone(),
            name: d.config.name.clone(),
            driver_type: d.config.driver.driver_name().to_string(),
            capabilities: d.config.driver.capabilities(),
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

    /// Get parameter registry for a device (bd-9clg)
    ///
    /// Enables generic code (gRPC, presets, HDF5 writers) to enumerate and subscribe
    /// to device parameters. Returns None if device doesn't implement Parameterized.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// if let Some(params) = registry.get_parameters("mock_camera") {
    ///     for name in params.names() {
    ///         println!("Parameter: {}", name);
    ///     }
    /// }
    /// ```
    pub fn get_parameters(&self, device_id: &str) -> Option<&ParameterSet> {
        let device = self.devices.get(device_id)?;
        device.parameterized.as_ref().map(|p| p.parameters())
    }

    #[cfg(feature = "instrument_spectra_physics")]
    /// Get a device as ShutterControl (if it supports this capability)
    pub fn get_shutter_control(&self, id: &str) -> Option<Arc<dyn ShutterControl>> {
        self.devices.get(id).and_then(|d| d.shutter_control.clone())
    }

    #[cfg(feature = "instrument_spectra_physics")]
    /// Get a device as EmissionControl (if it supports this capability)
    pub fn get_emission_control(&self, id: &str) -> Option<Arc<dyn EmissionControl>> {
        self.devices
            .get(id)
            .and_then(|d| d.emission_control.clone())
    }

    #[cfg(feature = "instrument_spectra_physics")]
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

    /// Get all devices that support a specific capability
    pub fn devices_with_capability(&self, capability: Capability) -> Vec<DeviceId> {
        self.devices
            .iter()
            .filter(|(_, d)| d.config.driver.capabilities().contains(&capability))
            .map(|(id, _)| id.clone())
            .collect()
    }

    // =========================================================================
    // Device Instantiation (Private)
    // =========================================================================

    /// Instantiate a device from configuration
    async fn instantiate_device(&self, config: DeviceConfig) -> Result<RegisteredDevice> {
        // Clone driver before matching to avoid borrow issues
        let driver = config.driver.clone();

        match driver {
            DriverType::MockStage { initial_position } => {
                let driver = Arc::new(crate::hardware::mock::MockStage::with_position(
                    initial_position,
                ));
                Ok(RegisteredDevice {
                    config,
                    movable: Some(driver.clone()),
                    readable: None,
                    triggerable: None,
                    frame_producer: None,
                    exposure_control: None,
                    settable: None,
                    stageable: None,
                    parameterized: Some(driver.clone()),
                    #[cfg(feature = "instrument_spectra_physics")]
                    shutter_control: None,
                    #[cfg(feature = "instrument_spectra_physics")]
                    emission_control: None,
                    #[cfg(feature = "instrument_spectra_physics")]
                    wavelength_tunable: None,
                    metadata: DeviceMetadata {
                        position_units: Some("mm".to_string()),
                        min_position: Some(-100.0),
                        max_position: Some(100.0),
                        ..Default::default()
                    },
                })
            }

            DriverType::MockPowerMeter { reading } => {
                let driver = Arc::new(crate::hardware::mock::MockPowerMeter::new(reading));
                Ok(RegisteredDevice {
                    config,
                    movable: None,
                    readable: Some(driver.clone()),
                    triggerable: None,
                    frame_producer: None,
                    exposure_control: None,
                    settable: None,
                    stageable: None,
                    parameterized: Some(driver.clone()),
                    #[cfg(feature = "instrument_spectra_physics")]
                    shutter_control: None,
                    #[cfg(feature = "instrument_spectra_physics")]
                    emission_control: None,
                    #[cfg(feature = "instrument_spectra_physics")]
                    wavelength_tunable: None,
                    metadata: DeviceMetadata {
                        measurement_units: Some("W".to_string()),
                        ..Default::default()
                    },
                })
            }

            DriverType::MockCamera => {
                let driver = Arc::new(crate::hardware::mock::MockCamera::default());
                let (width, height) = driver.resolution();
                Ok(RegisteredDevice {
                    config,
                    movable: None,
                    readable: None,
                    triggerable: Some(driver.clone()),
                    frame_producer: Some(driver.clone()),
                    exposure_control: Some(driver.clone()),
                    settable: None,
                    stageable: Some(driver.clone()),
                    parameterized: Some(driver.clone()),
                    #[cfg(feature = "instrument_spectra_physics")]
                    shutter_control: None,
                    #[cfg(feature = "instrument_spectra_physics")]
                    emission_control: None,
                    #[cfg(feature = "instrument_spectra_physics")]
                    wavelength_tunable: None,
                    metadata: DeviceMetadata {
                        frame_width: Some(width),
                        frame_height: Some(height),
                        ..Default::default()
                    },
                })
            }

            #[cfg(feature = "tokio_serial")]
            DriverType::Plugin { plugin_id, address } => {
                self.instantiate_plugin_device(config, &plugin_id, &address)
                    .await
            }

            #[cfg(feature = "instrument_photometrics")]
            DriverType::Pvcam { camera_name } => {
                let driver = Arc::new(
                    crate::hardware::pvcam::PvcamDriver::new_async(camera_name.clone()).await?,
                );
                let (width, height) = driver.resolution();
                Ok(RegisteredDevice {
                    config,
                    movable: None,
                    readable: None,
                    triggerable: Some(driver.clone()),
                    frame_producer: Some(driver.clone()),
                    exposure_control: Some(driver.clone()),
                    settable: None,
                    stageable: None,
                    parameterized: Some(driver.clone()),
                    #[cfg(feature = "instrument_spectra_physics")]
                    shutter_control: None,
                    #[cfg(feature = "instrument_spectra_physics")]
                    emission_control: None,
                    #[cfg(feature = "instrument_spectra_physics")]
                    wavelength_tunable: None,
                    metadata: DeviceMetadata {
                        frame_width: Some(width),
                        frame_height: Some(height),
                        bits_per_pixel: Some(16),
                        ..Default::default()
                    },
                })
            }

            #[cfg(not(feature = "instrument_photometrics"))]
            DriverType::Pvcam { .. } => Err(anyhow!(
                "PVCAM driver requires 'instrument_photometrics' feature"
            )),

            #[cfg(feature = "instrument_thorlabs")]
            DriverType::Ell14 { port, address } => {
                // Use shared port for multidrop bus - multiple ELL14 devices share one serial connection
                let shared_port = {
                    // Check if port already exists
                    let read_guard = self.ell14_shared_ports.read().await;
                    if let Some(existing) = read_guard.get(&port) {
                        existing.clone()
                    } else {
                        drop(read_guard); // Release read lock before acquiring write lock
                        let new_port =
                            crate::hardware::ell14::Ell14Driver::open_shared_port(&port)?;
                        let mut write_guard = self.ell14_shared_ports.write().await;
                        // Double-check in case another task created it
                        if let Some(existing) = write_guard.get(&port) {
                            existing.clone()
                        } else {
                            write_guard.insert(port.clone(), new_port.clone());
                            new_port
                        }
                    }
                };

                let driver = Arc::new(crate::hardware::ell14::Ell14Driver::with_shared_port(
                    shared_port,
                    &address,
                ));
                Ok(RegisteredDevice {
                    config,
                    movable: Some(driver.clone()),
                    readable: None,
                    triggerable: None,
                    frame_producer: None,
                    exposure_control: None,
                    settable: None,
                    stageable: None,
                    parameterized: Some(driver.clone()),
                    #[cfg(feature = "instrument_spectra_physics")]
                    shutter_control: None,
                    #[cfg(feature = "instrument_spectra_physics")]
                    emission_control: None,
                    #[cfg(feature = "instrument_spectra_physics")]
                    wavelength_tunable: None,
                    metadata: DeviceMetadata {
                        position_units: Some("degrees".to_string()),
                        min_position: Some(0.0),
                        max_position: Some(360.0),
                        ..Default::default()
                    },
                })
            }

            #[cfg(feature = "instrument_newport_power_meter")]
            DriverType::Newport1830C { port } => {
                let driver = Arc::new(crate::hardware::newport_1830c::Newport1830CDriver::new(
                    &port,
                )?);
                Ok(RegisteredDevice {
                    config,
                    movable: None,
                    readable: Some(driver.clone()),
                    triggerable: None,
                    frame_producer: None,
                    exposure_control: None,
                    settable: None,
                    stageable: None,
                    parameterized: Some(driver.clone()),
                    #[cfg(feature = "instrument_spectra_physics")]
                    shutter_control: None,
                    #[cfg(feature = "instrument_spectra_physics")]
                    emission_control: None,
                    #[cfg(feature = "instrument_spectra_physics")]
                    wavelength_tunable: None,
                    metadata: DeviceMetadata {
                        measurement_units: Some("W".to_string()),
                        ..Default::default()
                    },
                })
            }

            #[cfg(feature = "instrument_spectra_physics")]
            DriverType::MaiTai { port } => {
                let driver = Arc::new(crate::hardware::maitai::MaiTaiDriver::new(&port)?);
                Ok(RegisteredDevice {
                    config,
                    movable: None,
                    readable: Some(driver.clone()),
                    triggerable: None,
                    frame_producer: None,
                    exposure_control: None,
                    settable: None,
                    stageable: None,
                    parameterized: Some(driver.clone()),
                    #[cfg(feature = "instrument_spectra_physics")]
                    shutter_control: Some(driver.clone()),
                    #[cfg(feature = "instrument_spectra_physics")]
                    emission_control: Some(driver.clone()),
                    #[cfg(feature = "instrument_spectra_physics")]
                    wavelength_tunable: Some(driver),
                    metadata: DeviceMetadata {
                        measurement_units: Some("W".to_string()),
                        ..Default::default()
                    },
                })
            }

            #[cfg(feature = "instrument_newport")]
            DriverType::Esp300 { port, axis } => {
                let driver = Arc::new(crate::hardware::esp300::Esp300Driver::new(&port, axis)?);
                Ok(RegisteredDevice {
                    config,
                    movable: Some(driver.clone()),
                    readable: None,
                    triggerable: None,
                    frame_producer: None,
                    exposure_control: None,
                    settable: None,
                    stageable: None,
                    parameterized: Some(driver),
                    #[cfg(feature = "instrument_spectra_physics")]
                    shutter_control: None,
                    #[cfg(feature = "instrument_spectra_physics")]
                    emission_control: None,
                    #[cfg(feature = "instrument_spectra_physics")]
                    wavelength_tunable: None,
                    metadata: DeviceMetadata {
                        position_units: Some("mm".to_string()),
                        min_position: Some(-25.0), // Typical ESP300 stage range
                        max_position: Some(25.0),
                        ..Default::default()
                    },
                })
            }

            // Handle disabled features
            #[cfg(all(not(feature = "instrument_thorlabs"), feature = "instrument_serial"))]
            DriverType::Ell14 { .. } => Err(anyhow!(
                "ELL14 driver requires 'instrument_thorlabs' feature"
            )),

            #[cfg(all(
                not(feature = "instrument_newport_power_meter"),
                feature = "instrument_serial"
            ))]
            DriverType::Newport1830C { .. } => Err(anyhow!(
                "Newport 1830-C driver requires 'instrument_newport_power_meter' feature"
            )),

            #[cfg(all(not(feature = "instrument_spectra_physics"), feature = "instrument_serial"))]
            DriverType::MaiTai { .. } => Err(anyhow!(
                "MaiTai driver requires 'instrument_spectra_physics' feature"
            )),

            #[cfg(all(not(feature = "instrument_newport"), feature = "instrument_serial"))]
            DriverType::Esp300 { .. } => Err(anyhow!(
                "ESP300 driver requires 'instrument_newport' feature"
            )),
        }
    }

    /// Instantiate a plugin-based device
    #[cfg(feature = "tokio_serial")]
    async fn instantiate_plugin_device(
        &self,
        config: DeviceConfig,
        plugin_id: &str,
        address: &str,
    ) -> Result<RegisteredDevice> {
        // Spawn the driver from the plugin factory
        let factory = self.plugin_factory.read().await;
        let driver = Arc::new(factory.spawn(plugin_id, address).await?);
        drop(factory); // Release lock before calling helper

        // Create the registered device using the common helper
        self.create_registered_plugin(config, driver).await
    }

    /// Creates a RegisteredDevice from a pre-spawned plugin driver
    ///
    /// This is the shared implementation used by both `instantiate_plugin_device`
    /// (for config-based registration) and `register_plugin_instance` (for
    /// PluginService-managed registration).
    #[cfg(feature = "tokio_serial")]
    async fn create_registered_plugin(
        &self,
        config: DeviceConfig,
        driver: Arc<GenericDriver>,
    ) -> Result<RegisteredDevice> {
        let plugin_id = match &config.driver {
            DriverType::Plugin { plugin_id, .. } => plugin_id,
            _ => {
                return Err(anyhow!(
                    "Invalid driver type for create_registered_plugin: expected Plugin"
                ))
            }
        };

        // Introspect capabilities from the plugin configuration
        let factory = self.plugin_factory.read().await;
        let plugin_config = factory
            .get_config(plugin_id)
            .ok_or_else(|| anyhow!("Plugin '{}' not found in factory", plugin_id))?;

        let mut metadata = DeviceMetadata::default();

        // Check for movable capability
        let movable: Option<Arc<dyn Movable>> = if plugin_config.capabilities.movable.is_some() {
            // Extract metadata from first axis
            if let Some(movable_cap) = &plugin_config.capabilities.movable {
                if let Some(first_axis) = movable_cap.axes.first() {
                    metadata.position_units = first_axis.unit.clone();
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

            Some(Arc::new(
                crate::hardware::plugin::handles::PluginAxisHandle::new(
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
                metadata.measurement_units = first_readable.unit.clone();
            }

            // Create readable handle for the first readable capability (convention)
            let readable_name = plugin_config
                .capabilities
                .readable
                .first()
                .map(|r| r.name.as_str())
                .unwrap_or("reading");

            Some(Arc::new(
                crate::hardware::plugin::handles::PluginSensorHandle::new(
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

        Ok(RegisteredDevice {
            config,
            movable,
            readable,
            triggerable: None,
            frame_producer: None,
            exposure_control: None,
            settable: None,
            stageable: None,
            parameterized: Some(driver.clone()), // bd-plb6: Wire Parameterized for plugin devices
            #[cfg(feature = "instrument_spectra_physics")]
            shutter_control: None,
            #[cfg(feature = "instrument_spectra_physics")]
            emission_control: None,
            #[cfg(feature = "instrument_spectra_physics")]
            wavelength_tunable: None,
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
    pub fn snapshot_all_parameters(&self) -> HashMap<String, HashMap<String, serde_json::Value>> {
        let mut snapshot = HashMap::new();

        for (device_id, device) in &self.devices {
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
    pub fn from_file(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow!("Failed to read hardware config file: {}", e))?;
        toml::from_str(&content).map_err(|e| anyhow!("Failed to parse hardware config file: {}", e))
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
pub async fn create_registry_from_config(config: &HardwareConfig) -> Result<DeviceRegistry> {
    let mut registry = DeviceRegistry::new();

    // Validate all device configurations first (fail fast)
    let mut validation_errors = Vec::new();
    for device_config in &config.devices {
        if let Err(e) = validate_driver_config(&device_config.driver) {
            validation_errors.push(format!(
                "Device '{}' ({}): {}",
                device_config.id,
                device_config.driver.driver_name(),
                e
            ));
        }
    }

    if !validation_errors.is_empty() {
        anyhow::bail!(
            "Hardware configuration validation failed:\n  - {}",
            validation_errors.join("\n  - ")
        );
    }

    // Load plugins from configured search paths
    #[cfg(feature = "tokio_serial")]
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

    // Register all configured devices
    for device_config in &config.devices {
        if let Err(e) = registry.register(device_config.clone()).await {
            tracing::warn!(
                "Failed to register device '{}': {} (continuing with other devices)",
                device_config.id,
                e
            );
        }
    }

    Ok(registry)
}

/// Load hardware configuration from a file and create a DeviceRegistry
pub async fn create_registry_from_file(path: &std::path::Path) -> Result<DeviceRegistry> {
    let config = HardwareConfig::from_file(path)?;
    create_registry_from_config(&config).await
}

// =============================================================================
// Convenience Functions for Lab Configuration
// =============================================================================

/// Create a DeviceRegistry pre-configured for the maitai@100.117.5.12 lab system
///
/// This registers all known instruments from docs/HARDWARE_INVENTORY.md:
/// - Newport 1830-C Power Meter on /dev/ttyS0
/// - MaiTai Laser on /dev/ttyUSB5
/// - ELL14 Rotators on /dev/ttyUSB0 (addresses 2, 3, 8)
/// - ESP300 on /dev/ttyUSB1 (if available)
#[cfg(feature = "instrument_serial")]
pub async fn create_lab_registry() -> Result<DeviceRegistry> {
    let mut registry = DeviceRegistry::new();

    // Newport 1830-C Power Meter
    if let Err(e) = registry
        .register(DeviceConfig {
            id: "power_meter".into(),
            name: "Newport 1830-C Power Meter".into(),
            driver: DriverType::Newport1830C {
                port: "/dev/ttyS0".into(),
            },
        })
        .await
    {
        tracing::warn!("Newport 1830-C registration failed: {}", e);
    }

    // MaiTai Laser
    if let Err(e) = registry
        .register(DeviceConfig {
            id: "maitai".into(),
            name: "Spectra-Physics MaiTai Ti:Sapphire Laser".into(),
            driver: DriverType::MaiTai {
                port: "/dev/ttyUSB5".into(),
            },
        })
        .await
    {
        tracing::warn!("MaiTai registration failed: {}", e);
    }

    // ELL14 Rotators (3 units on multidrop bus)
    for (addr, serial) in [("2", "005172023"), ("3", "002842021"), ("8", "006092023")] {
        if let Err(e) = registry
            .register(DeviceConfig {
                id: format!("rotator_{}", addr),
                name: format!(
                    "Thorlabs ELL14 Rotation Mount (Addr {}, SN {})",
                    addr, serial
                ),
                driver: DriverType::Ell14 {
                    port: "/dev/ttyUSB0".into(),
                    address: addr.into(),
                },
            })
            .await
        {
            tracing::warn!("ELL14 (addr {}) registration failed: {}", addr, e);
        }
    }

    // ESP300 Motion Controller (may not be powered on)
    // Note: This often fails if ESP300 is not powered
    if let Err(e) = registry
        .register(DeviceConfig {
            id: "stage_1".into(),
            name: "Newport ESP300 Axis 1".into(),
            driver: DriverType::Esp300 {
                port: "/dev/ttyUSB1".into(),
                axis: 1,
            },
        })
        .await
    {
        tracing::warn!("ESP300 registration failed (likely powered off): {}", e);
    }

    Ok(registry)
}

#[cfg(not(feature = "instrument_serial"))]
pub async fn create_lab_registry() -> Result<DeviceRegistry> {
    Ok(DeviceRegistry::new())
}

/// Create a DeviceRegistry with mock devices for testing
pub async fn create_mock_registry() -> Result<DeviceRegistry> {
    let mut registry = DeviceRegistry::new();

    registry
        .register(DeviceConfig {
            id: "mock_stage".into(),
            name: "Mock Stage".into(),
            driver: DriverType::MockStage {
                initial_position: 0.0,
            },
        })
        .await?;

    registry
        .register(DeviceConfig {
            id: "mock_power_meter".into(),
            name: "Mock Power Meter".into(),
            driver: DriverType::MockPowerMeter { reading: 1e-6 },
        })
        .await?;

    Ok(registry)
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

        assert_eq!(registry.len(), 2);
        assert!(registry.contains("mock_stage"));
        assert!(registry.contains("mock_power_meter"));
    }

    #[tokio::test]
    async fn test_list_devices() {
        let registry = create_mock_registry().await.unwrap();
        let devices = registry.list_devices();

        assert_eq!(devices.len(), 2);

        let stage = devices.iter().find(|d| d.id == "mock_stage").unwrap();
        assert_eq!(stage.driver_type, "mock_stage");
        assert!(stage.capabilities.contains(&Capability::Movable));

        let meter = devices.iter().find(|d| d.id == "mock_power_meter").unwrap();
        assert_eq!(meter.driver_type, "mock_power_meter");
        assert!(meter.capabilities.contains(&Capability::Readable));
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
        let mut registry = DeviceRegistry::new();

        registry
            .register(DeviceConfig {
                id: "test".into(),
                name: "Test Device".into(),
                driver: DriverType::MockStage {
                    initial_position: 0.0,
                },
            })
            .await
            .unwrap();

        let result = registry
            .register(DeviceConfig {
                id: "test".into(),
                name: "Duplicate".into(),
                driver: DriverType::MockStage {
                    initial_position: 0.0,
                },
            })
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_unregister() {
        let mut registry = create_mock_registry().await.unwrap();

        assert!(registry.contains("mock_stage"));
        assert!(registry.unregister("mock_stage"));
        assert!(!registry.contains("mock_stage"));
        assert!(!registry.unregister("mock_stage")); // Already removed
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
        // MockPowerMeter adds ~1% noise, so allow 2% tolerance
        let readable = registry.get_readable("mock_power_meter").unwrap();
        let reading = readable.read().await.unwrap();
        assert!(
            (reading - 1e-6).abs() < 1e-7,
            "Reading {} not close to 1e-6",
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

    #[cfg(feature = "tokio_serial")]
    #[tokio::test]
    async fn test_plugin_device_registration() {
        use std::sync::Arc;
        use tokio::sync::RwLock;

        // Create a plugin factory and registry
        let factory = Arc::new(RwLock::new(
            crate::hardware::plugin::registry::PluginFactory::new(),
        ));
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
    async fn test_validate_driver_config_missing_serial_port() {
        let driver = DriverType::Newport1830C {
            port: "/dev/nonexistent_serial_port_xyz123".to_string(),
        };

        let result = validate_driver_config(&driver);
        assert!(result.is_err());

        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(err_msg.contains("does not exist"));
        assert!(err_msg.contains("Newport 1830-C"));
    }

    #[tokio::test]
    async fn test_validate_driver_config_invalid_ell14_address() {
        // Create a temporary file to act as a valid serial port for this test
        let temp_port = std::env::temp_dir().join("test_serial_port");
        std::fs::write(&temp_port, "").unwrap();

        let driver = DriverType::Ell14 {
            port: temp_port.to_string_lossy().to_string(),
            address: "XYZ".to_string(), // Invalid address
        };

        let result = validate_driver_config(&driver);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err.to_string().contains("single hex digit"));

        // Clean up
        std::fs::remove_file(temp_port).ok();
    }

    #[tokio::test]
    async fn test_validate_driver_config_invalid_esp300_axis() {
        let temp_port = std::env::temp_dir().join("test_serial_port_esp");
        std::fs::write(&temp_port, "").unwrap();

        let driver = DriverType::Esp300 {
            port: temp_port.to_string_lossy().to_string(),
            axis: 5, // Invalid axis (must be 1-3)
        };

        let result = validate_driver_config(&driver);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err.to_string().contains("Must be 1-3"));

        std::fs::remove_file(temp_port).ok();
    }

    #[tokio::test]
    async fn test_validate_driver_config_empty_pvcam_name() {
        let driver = DriverType::Pvcam {
            camera_name: "".to_string(),
        };

        let result = validate_driver_config(&driver);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[tokio::test]
    async fn test_validate_driver_config_mock_devices_always_valid() {
        // Mock devices should always pass validation
        assert!(validate_driver_config(&DriverType::MockStage {
            initial_position: 0.0
        })
        .is_ok());

        assert!(validate_driver_config(&DriverType::MockPowerMeter { reading: 1e-6 }).is_ok());

        assert!(validate_driver_config(&DriverType::MockCamera).is_ok());
    }

    #[tokio::test]
    async fn test_register_fails_on_invalid_config() {
        let mut registry = DeviceRegistry::new();

        let result = registry
            .register(DeviceConfig {
                id: "invalid_device".into(),
                name: "Invalid Device".into(),
                driver: DriverType::Newport1830C {
                    port: "/dev/definitely_does_not_exist_xyz".into(),
                },
            })
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Configuration validation failed"));

        // Registry should remain empty
        assert_eq!(registry.len(), 0);
    }
}
