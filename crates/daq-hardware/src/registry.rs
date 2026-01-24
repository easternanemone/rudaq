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

use anyhow::{anyhow, Result};
use daq_core::capabilities::{
    Commandable, EmissionControl, ExposureControl, FrameProducer, Movable, Parameterized, Readable,
    Settable, ShutterControl, Stageable, Triggerable, WavelengthTunable,
};
use daq_core::data::Frame;
use daq_core::driver::{DeviceComponents, DeviceLifecycle, DriverFactory};
use daq_core::error::DaqError;
use daq_core::pipeline::MeasurementSource;

#[cfg(feature = "serial")]
use crate::plugin::driver::GenericDriver;
// use crate::plugin::driver::{Connection, GenericDriver};
// use crate::plugin::schema::{DriverType, InstrumentConfig, PluginMetadata, ScriptType};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "serial")]
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
pub fn validate_driver_config(driver: &DriverType) -> Result<(), DaqError> {
    match driver {
        #[cfg(feature = "serial")]
        DriverType::Newport1830C { port } => {
            validate_serial_port(port, "Newport 1830-C")?;
        }

        #[cfg(feature = "serial")]
        DriverType::MaiTai { port } => {
            validate_serial_port(port, "MaiTai Laser")?;
        }

        #[cfg(feature = "serial")]
        DriverType::Ell14 { port, address } => {
            validate_serial_port(port, "ELL14 Rotation Mount")?;
            validate_ell14_address(address)?;
        }

        #[cfg(feature = "serial")]
        DriverType::Esp300 { port, axis } => {
            validate_serial_port(port, "ESP300 Motion Controller")?;
            if *axis < 1 || *axis > 3 {
                return Err(DaqError::Configuration(format!(
                    "Invalid ESP300 axis: {}. Must be 1-3",
                    axis
                )));
            }
        }

        #[cfg(feature = "pvcam")]
        DriverType::Pvcam { camera_name } => {
            if camera_name.is_empty() {
                return Err(DaqError::Configuration(
                    "PVCAM camera name cannot be empty".to_string(),
                ));
            }
        }
        #[cfg(feature = "comedi")]
        DriverType::Comedi { device_path } => {
            if device_path.is_empty() {
                return Err(DaqError::Configuration(
                    "Comedi device path cannot be empty".to_string(),
                ));
            }
            // Note: We don't check device existence here since the driver may not be
            // running on the machine with the hardware (remote development)
        }
        #[cfg(feature = "comedi")]
        DriverType::ComediAnalogInput {
            device, input_mode, ..
        } => {
            if device.is_empty() {
                return Err(DaqError::Configuration(
                    "Comedi device path cannot be empty".to_string(),
                ));
            }
            // Validate input mode
            let mode_lower = input_mode.to_lowercase();
            if ![
                "rse",
                "nrse",
                "diff",
                "ground",
                "common",
                "differential",
                "single-ended",
                "single_ended",
                "other",
            ]
            .contains(&mode_lower.as_str())
            {
                return Err(DaqError::Configuration(format!(
                    "Invalid input_mode '{}'. Valid values: rse, nrse, diff",
                    input_mode
                )));
            }
        }
        #[cfg(feature = "comedi")]
        DriverType::ComediAnalogOutput { device, .. } => {
            if device.is_empty() {
                return Err(DaqError::Configuration(
                    "Comedi device path cannot be empty".to_string(),
                ));
            }
        }
        #[cfg(feature = "serial")]
        DriverType::Plugin { plugin_id, address } => {
            if plugin_id.is_empty() {
                return Err(DaqError::Configuration(
                    "Plugin ID cannot be empty".to_string(),
                ));
            }
            if address.is_empty() {
                return Err(DaqError::Configuration(
                    "Plugin address cannot be empty".to_string(),
                ));
            }
            // Address can be serial port or network address
            // Don't validate serial port here as it might be network
        }

        // Mock devices don't need validation (except basic sanity checks)
        DriverType::MockStage { .. } | DriverType::MockPowerMeter { .. } => {}
        DriverType::MockCamera { width, height } => {
            if *width == 0 || *height == 0 {
                return Err(DaqError::Configuration(format!(
                    "Invalid MockCamera resolution: {}x{}. Width/height must be > 0",
                    width, height
                )));
            }
        }
    }

    Ok(())
}

/// Validate that a serial port exists and is accessible
///
/// Provides helpful error messages listing available ports if the requested port is not found.
#[cfg(feature = "serial")]
fn validate_serial_port(port: &str, device_name: &str) -> Result<(), DaqError> {
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

        return Err(DaqError::Configuration(format!(
            "Serial port '{}' does not exist for device '{}'.\n\n{}\n\n\
             Troubleshooting:\n\
             - Verify device is connected and powered on\n\
             - Check USB cable connection\n\
             - On Linux, ensure you have permissions (add user to 'dialout' group)\n\
             - On macOS, check /dev/tty.* and /dev/cu.* devices\n\
             - Run 'ls /dev/tty*' or 'ls /dev/cu*' to list available ports",
            port, device_name, available
        )));
    }

    Ok(())
}

/// Stub validator when serial support is disabled.
#[cfg(not(feature = "serial"))]
fn validate_serial_port(_port: &str, _device_name: &str) -> Result<(), DaqError> {
    // Serial devices are not available without the instrument_serial feature enabled.
    // Validation is a no-op so builds without serialport dependency succeed.
    Ok(())
}

/// Validate ELL14 device address
///
/// ELL14 addresses must be hex digits 0-F
fn validate_ell14_address(address: &str) -> Result<(), DaqError> {
    // Use tuple pattern matching to validate exactly one hex digit
    // This avoids unwrap() and handles all cases in a single match
    let mut chars = address.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_hexdigit() => Ok(()),
        (Some(_), None) => Err(DaqError::Configuration(format!(
            "Invalid ELL14 address '{}': must be a hex digit (0-9, A-F)",
            address
        ))),
        _ => Err(DaqError::Configuration(format!(
            "Invalid ELL14 address '{}': must be a single hex digit (0-F)",
            address
        ))),
    }
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
    /// Can execute structured JSON commands - bd-cdh5.4
    Commandable,
    /// Can be staged/unstaged for acquisition sequences (Bluesky pattern) - bd-7aq6
    Stageable,
    /// Has observable parameters with subscriptions (Parameterized trait)
    Parameterized,
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
    #[cfg(feature = "serial")]
    #[serde(rename = "newport1830_c")]
    Newport1830C {
        /// Serial port path (e.g., "/dev/ttyS0")
        port: String,
    },

    /// Spectra-Physics MaiTai Ti:Sapphire Laser
    #[cfg(feature = "serial")]
    MaiTai {
        /// Serial port path (e.g., "/dev/ttyUSB5")
        port: String,
    },

    /// Thorlabs Elliptec ELL14 Rotation Mount
    #[cfg(feature = "serial")]
    Ell14 {
        /// Serial port path (e.g., "/dev/ttyUSB0")
        port: String,
        /// Device address on multidrop bus ("0"-"F", typically "2", "3", or "8")
        address: String,
    },

    /// Newport ESP300 Multi-Axis Motion Controller
    #[cfg(feature = "serial")]
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
    MockCamera {
        /// Frame width in pixels
        #[serde(default = "default_mock_camera_width")]
        width: u32,
        /// Frame height in pixels
        #[serde(default = "default_mock_camera_height")]
        height: u32,
    },

    /// Photometrics PVCAM camera
    #[cfg(feature = "pvcam")]
    Pvcam {
        /// Camera name reported by PVCAM (e.g., "PrimeBSI")
        camera_name: String,
    },
    /// Comedi DAQ board (Linux)
    #[cfg(feature = "comedi")]
    Comedi {
        /// Device path (e.g., "/dev/comedi0")
        device_path: String,
    },
    /// Comedi Analog Input (factory-based)
    #[cfg(feature = "comedi")]
    #[serde(rename = "comedi_analog_input")]
    ComediAnalogInput {
        /// Device path (e.g., "/dev/comedi0")
        device: String,
        /// Analog input channel number
        channel: u32,
        /// Voltage range index
        range_index: u32,
        /// Input reference mode: "rse", "nrse", or "diff"
        input_mode: String,
        /// Measurement units (default: "V")
        #[serde(default)]
        units: String,
    },
    /// Comedi Analog Output (factory-based)
    #[cfg(feature = "comedi")]
    #[serde(rename = "comedi_analog_output")]
    ComediAnalogOutput {
        /// Device path (e.g., "/dev/comedi0")
        device: String,
        /// Analog output channel number
        channel: u32,
        /// Voltage range index
        range_index: u32,
        /// Output units (default: "V")
        #[serde(default)]
        units: String,
    },
    /// Plugin-based device loaded from YAML configuration
    #[cfg(feature = "serial")]
    Plugin {
        /// Plugin ID from YAML metadata.id (e.g., "my-sensor-v1")
        plugin_id: String,
        /// Connection address (serial port path or TCP "host:port")
        address: String,
    },
}

fn default_mock_camera_width() -> u32 {
    640
}

fn default_mock_camera_height() -> u32 {
    480
}

impl DriverType {
    /// Get the capabilities this driver type provides
    pub fn capabilities(&self) -> Vec<Capability> {
        match self {
            #[cfg(feature = "serial")]
            DriverType::Newport1830C { .. } => {
                vec![Capability::Readable, Capability::WavelengthTunable]
            }
            #[cfg(feature = "serial")]
            DriverType::MaiTai { .. } => vec![
                Capability::Readable,
                Capability::ShutterControl,
                Capability::WavelengthTunable,
                Capability::EmissionControl,
                Capability::Parameterized,
            ],
            #[cfg(feature = "serial")]
            DriverType::Ell14 { .. } => vec![Capability::Movable],
            #[cfg(feature = "serial")]
            DriverType::Esp300 { .. } => vec![Capability::Movable],
            DriverType::MockStage { .. } => vec![Capability::Movable],
            DriverType::MockPowerMeter { .. } => vec![Capability::Readable],
            DriverType::MockCamera { .. } => vec![
                Capability::FrameProducer,
                Capability::Triggerable,
                Capability::ExposureControl,
            ],
            #[cfg(feature = "pvcam")]
            DriverType::Pvcam { .. } => vec![
                Capability::FrameProducer,
                Capability::Triggerable,
                Capability::ExposureControl,
            ],
            #[cfg(feature = "comedi")]
            DriverType::Comedi { .. } => vec![
                Capability::Readable, // Analog input
                Capability::Settable, // Analog output
            ],
            #[cfg(feature = "comedi")]
            DriverType::ComediAnalogInput { .. } => {
                vec![Capability::Readable, Capability::Parameterized]
            }
            #[cfg(feature = "comedi")]
            DriverType::ComediAnalogOutput { .. } => vec![Capability::Parameterized],
            #[cfg(feature = "serial")]
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
            #[cfg(feature = "serial")]
            DriverType::Newport1830C { .. } => "newport_1830c",
            #[cfg(feature = "serial")]
            DriverType::MaiTai { .. } => "maitai",
            #[cfg(feature = "serial")]
            DriverType::Ell14 { .. } => "ell14",
            #[cfg(feature = "serial")]
            DriverType::Esp300 { .. } => "esp300",
            DriverType::MockStage { .. } => "mock_stage",
            DriverType::MockPowerMeter { .. } => "mock_power_meter",
            DriverType::MockCamera { .. } => "mock_camera",
            #[cfg(feature = "pvcam")]
            DriverType::Pvcam { .. } => "pvcam",
            #[cfg(feature = "comedi")]
            DriverType::Comedi { .. } => "comedi",
            #[cfg(feature = "comedi")]
            DriverType::ComediAnalogInput { .. } => "comedi_analog_input",
            #[cfg(feature = "comedi")]
            DriverType::ComediAnalogOutput { .. } => "comedi_analog_output",
            #[cfg(feature = "serial")]
            DriverType::Plugin { .. } => {
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
    /// Device category for UI grouping (bd-le6k: moved from gRPC inference layer)
    ///
    /// Drivers should set this explicitly. The gRPC layer will fall back to
    /// string-based driver name inference only if this is None.
    pub category: Option<daq_core::capabilities::DeviceCategory>,
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
    /// Compute capabilities from the actual registered trait objects.
    ///
    /// This introspects which trait implementations are present rather than
    /// relying on the config's DriverType enum (which may be synthetic for
    /// factory-registered devices).
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
    /// Factories are consulted first when registering a device. If a factory
    /// matches the driver_type, it's used to build the device. Otherwise,
    /// the legacy DriverType enum matching is used as fallback.
    factories: DashMap<String, Box<dyn DriverFactory>>,

    /// Shared serial ports for ELL14 multidrop bus (interior mutability for async access)
    /// Key: port path (e.g., "/dev/ttyUSB0"), Value: shared Arc<Mutex<SerialStream>>
    #[cfg(feature = "thorlabs")]
    ell14_shared_ports: RwLock<HashMap<String, crate::drivers::ell14::SharedPort>>,

    /// Plugin factory for loading YAML-defined drivers (tokio_serial feature only)
    #[cfg(feature = "serial")]
    plugin_factory: Arc<RwLock<crate::plugin::registry::PluginFactory>>,

    /// Registration failures for debugging (device_id, driver_type, error_message)
    registration_failures: DashMap<DeviceId, RegistrationFailure>,
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
        lifecycle: &Option<Arc<dyn DeviceLifecycle>>,
    ) -> Result<(), DaqError> {
        if let Some(hook) = lifecycle {
            hook.on_register().await.map_err(|e| {
                DaqError::Driver(daq_core::error::DriverError::new(
                    driver_type,
                    daq_core::error::DriverErrorKind::Initialization,
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
        lifecycle: &Option<Arc<dyn DeviceLifecycle>>,
    ) -> Result<(), DaqError> {
        if let Some(hook) = lifecycle {
            hook.on_unregister().await.map_err(|e| {
                DaqError::Driver(daq_core::error::DriverError::new(
                    driver_type,
                    daq_core::error::DriverErrorKind::Shutdown,
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
            #[cfg(feature = "thorlabs")]
            ell14_shared_ports: RwLock::new(HashMap::new()),
            #[cfg(feature = "serial")]
            plugin_factory: Arc::new(RwLock::new(crate::plugin::registry::PluginFactory::new())),
            registration_failures: DashMap::new(),
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
            #[cfg(feature = "thorlabs")]
            ell14_shared_ports: RwLock::new(HashMap::new()),
            plugin_factory,
            registration_failures: DashMap::new(),
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
    /// `build()` method will be called instead of the legacy DriverType matching.
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
    /// use daq_core::driver::DriverFactory;
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
            DaqError::Driver(daq_core::error::DriverError::new(
                driver_type,
                daq_core::error::DriverErrorKind::Configuration,
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
            DaqError::Driver(daq_core::error::DriverError::new(
                driver_type,
                daq_core::error::DriverErrorKind::Initialization,
                format!(
                    "Factory build failed for device '{}' ({}): {}",
                    device_id, driver_type, e
                ),
            ))
        })?;

        if let Err(err) = self
            .run_on_register(device_id, driver_type, &components.lifecycle)
            .await
        {
            let _ = self
                .run_on_unregister(device_id, driver_type, &components.lifecycle)
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
        // Create a synthetic DeviceConfig for the legacy structure
        // Note: We use MockStage as a placeholder since the actual driver type
        // doesn't matter once the device is instantiated.
        let config = DeviceConfig {
            id: device_id,
            name: device_name,
            driver: DriverType::MockStage {
                initial_position: 0.0,
            },
        };

        // Convert daq_core::driver::DeviceMetadata to local DeviceMetadata
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
            shutter_control: components.shutter_control,
            emission_control: components.emission_control,
            wavelength_tunable: components.wavelength_tunable,
            lifecycle: components.lifecycle,
            metadata,
        }
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
    ///
    /// # Thread Safety (bd-pf31)
    /// This method is thread-safe and can be called concurrently. Registration of
    /// the same device ID from multiple threads will fail for all but one caller.
    pub async fn register(&self, config: DeviceConfig) -> Result<(), DaqError> {
        if self.devices.contains_key(&config.id) {
            return Err(DaqError::Configuration(format!(
                "Device '{}' is already registered",
                config.id
            )));
        }

        let driver_type = config.driver.driver_name().to_string();

        // Validate configuration before attempting to instantiate
        validate_driver_config(&config.driver).map_err(|e| {
            DaqError::Configuration(format!(
                "Configuration validation failed for device '{}' ({}): {}",
                config.id,
                config.driver.driver_name(),
                e
            ))
        })?;

        let registered = self.instantiate_device(config).await.map_err(|e| {
            DaqError::Driver(daq_core::error::DriverError::new(
                &driver_type,
                daq_core::error::DriverErrorKind::Initialization,
                e.to_string(),
            ))
        })?;
        if let Err(err) = self
            .run_on_register(&registered.config.id, &driver_type, &registered.lifecycle)
            .await
        {
            let _ = self
                .run_on_unregister(&registered.config.id, &driver_type, &registered.lifecycle)
                .await;
            return Err(err);
        }
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
                DaqError::Driver(daq_core::error::DriverError::new(
                    "plugin",
                    daq_core::error::DriverErrorKind::Initialization,
                    e.to_string(),
                ))
            })?;
        let driver_type = registered.driver_type.clone();
        if let Err(err) = self
            .run_on_register(&registered.config.id, &driver_type, &registered.lifecycle)
            .await
        {
            let _ = self
                .run_on_unregister(&registered.config.id, &driver_type, &registered.lifecycle)
                .await;
            return Err(err);
        }
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
    ///
    /// # Thread Safety (bd-pf31)
    /// This method is thread-safe and can be called concurrently.
    pub async fn unregister(&self, id: &str) -> Result<bool, DaqError> {
        if let Some((_, device)) = self.devices.remove(id) {
            let driver_type = device.driver_type.clone();
            self.run_on_unregister(&device.config.id, &driver_type, &device.lifecycle)
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
                    // Use introspected capabilities from actual trait objects,
                    // not the config's DriverType enum (which may be synthetic)
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

    /// Get device info by ID
    pub fn get_device_info(&self, id: &str) -> Option<DeviceInfo> {
        self.devices.get(id).map(|d| DeviceInfo {
            id: d.config.id.clone(),
            name: d.config.name.clone(),
            driver_type: d.driver_type.clone(),
            // Use introspected capabilities from actual trait objects,
            // not the config's DriverType enum (which may be synthetic)
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
            .filter(|entry| {
                entry
                    .value()
                    .config
                    .driver
                    .capabilities()
                    .contains(&capability)
            })
            .map(|entry| entry.key().clone())
            .collect()
    }

    // =========================================================================
    // Device Instantiation (Private)
    // =========================================================================

    /// Instantiate a device from configuration
    async fn instantiate_device(&self, config: DeviceConfig) -> Result<RegisteredDevice> {
        // Clone driver before matching to avoid borrow issues
        let driver_type_name = config.driver.driver_name().to_string();
        let driver = config.driver.clone();

        match driver {
            DriverType::MockStage { initial_position } => {
                let driver = Arc::new(crate::drivers::mock::MockStage::with_position(
                    initial_position,
                ));
                Ok(RegisteredDevice {
                    config,
                    driver_type: driver_type_name.clone(),
                    movable: Some(driver.clone()),
                    readable: None,
                    triggerable: None,
                    frame_producer: None,
                    source_frame: None,
                    exposure_control: None,
                    settable: None,
                    stageable: None,
                    commandable: None,
                    parameterized: Some(driver.clone()),
                    shutter_control: None,
                    emission_control: None,
                    wavelength_tunable: None,
                    lifecycle: None,
                    metadata: DeviceMetadata {
                        position_units: Some("mm".to_string()),
                        min_position: Some(-100.0),
                        max_position: Some(100.0),
                        ..Default::default()
                    },
                })
            }

            DriverType::MockPowerMeter { reading } => {
                let driver = Arc::new(crate::drivers::mock::MockPowerMeter::new(reading));
                Ok(RegisteredDevice {
                    config,
                    driver_type: driver_type_name.clone(),
                    movable: None,
                    readable: Some(driver.clone()),
                    triggerable: None,
                    frame_producer: None,
                    source_frame: None,
                    exposure_control: None,
                    settable: None,
                    stageable: None,
                    commandable: None,
                    parameterized: Some(driver.clone()),
                    shutter_control: None,
                    emission_control: None,
                    wavelength_tunable: None,
                    lifecycle: None,
                    metadata: DeviceMetadata {
                        measurement_units: Some("W".to_string()),
                        ..Default::default()
                    },
                })
            }

            DriverType::MockCamera { width, height } => {
                let driver = Arc::new(crate::drivers::mock::MockCamera::new(width, height));
                Ok(RegisteredDevice {
                    config,
                    driver_type: driver_type_name.clone(),
                    movable: None,
                    readable: None,
                    triggerable: Some(driver.clone()),
                    frame_producer: Some(driver.clone()),
                    source_frame: Some(driver.clone()),
                    exposure_control: Some(driver.clone()),
                    settable: None,
                    stageable: Some(driver.clone()),
                    commandable: None,
                    parameterized: Some(driver.clone()),
                    shutter_control: None,
                    emission_control: None,
                    wavelength_tunable: None,
                    lifecycle: None,
                    metadata: DeviceMetadata {
                        frame_width: Some(width),
                        frame_height: Some(height),
                        ..Default::default()
                    },
                })
            }

            #[cfg(feature = "serial")]
            DriverType::Plugin { plugin_id, address } => {
                self.instantiate_plugin_device(config, &plugin_id, &address)
                    .await
            }

            #[cfg(feature = "pvcam")]
            DriverType::Pvcam { camera_name } => {
                let driver = Arc::new(
                    crate::drivers::pvcam::PvcamDriver::new_async(camera_name.clone()).await?,
                );
                let (width, height) = driver.resolution();
                Ok(RegisteredDevice {
                    config,
                    driver_type: driver_type_name.clone(),
                    movable: None,
                    readable: None,
                    triggerable: Some(driver.clone()),
                    frame_producer: Some(driver.clone()),
                    source_frame: Some(driver.clone()),
                    exposure_control: Some(driver.clone()),
                    settable: None,
                    stageable: None,
                    commandable: Some(driver.clone()),
                    parameterized: Some(driver.clone()),
                    shutter_control: None,
                    emission_control: None,
                    wavelength_tunable: None,
                    lifecycle: None,
                    metadata: DeviceMetadata {
                        frame_width: Some(width),
                        frame_height: Some(height),
                        ..Default::default()
                    },
                })
            }

            #[cfg(feature = "comedi")]
            DriverType::Comedi { device_path } => {
                let device =
                    crate::drivers::comedi::ComediDevice::open(&device_path).map_err(|e| {
                        DaqError::Instrument(format!("Failed to open Comedi device: {}", e))
                    })?;
                let _info = device.info().map_err(|e| {
                    DaqError::Instrument(format!("Failed to get Comedi device info: {}", e))
                })?;
                // Note: Comedi doesn't implement full HAL traits directly;
                // subsystems (AnalogInput, AnalogOutput, etc.) need to be accessed separately.
                // For registry purposes, we register the device as having Readable/Settable capabilities.
                // TODO: Implement HAL trait wrappers for Comedi subsystems
                Ok(RegisteredDevice {
                    config,
                    driver_type: driver_type_name.clone(),
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
                    shutter_control: None,
                    emission_control: None,
                    wavelength_tunable: None,
                    lifecycle: None,
                    metadata: DeviceMetadata {
                        measurement_units: Some("V".to_string()), // Voltage
                        ..Default::default()
                    },
                })
            }

            #[cfg(feature = "thorlabs")]
            DriverType::Ell14 { port, address } => {
                // Use shared port for multidrop bus - multiple ELL14 devices share one serial connection
                let shared_port = {
                    // Check if port already exists
                    let read_guard = self.ell14_shared_ports.read().await;
                    if let Some(existing) = read_guard.get(&port) {
                        existing.clone()
                    } else {
                        drop(read_guard); // Release read lock before acquiring write lock
                        let new_port = crate::drivers::ell14::Ell14Driver::open_shared_port(&port)?;
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

                // Use with_shared_port_calibrated() to validate device responds and get calibration
                let driver = Arc::new(
                    crate::drivers::ell14::Ell14Driver::with_shared_port_calibrated(
                        shared_port,
                        &address,
                    )
                    .await?,
                );
                Ok(RegisteredDevice {
                    config,
                    driver_type: driver_type_name.clone(),
                    movable: Some(driver.clone()),
                    readable: None,
                    triggerable: None,
                    frame_producer: None,
                    source_frame: None,
                    exposure_control: None,
                    settable: None,
                    stageable: None,
                    commandable: None,
                    parameterized: Some(driver.clone()),
                    shutter_control: None,
                    emission_control: None,
                    wavelength_tunable: None,
                    lifecycle: None,
                    metadata: DeviceMetadata {
                        position_units: Some("degrees".to_string()),
                        min_position: Some(0.0),
                        max_position: Some(360.0),
                        ..Default::default()
                    },
                })
            }

            #[cfg(feature = "newport")]
            DriverType::Newport1830C { port } => {
                // Use new_async() to validate device responds correctly on connection
                let driver = Arc::new(
                    crate::drivers::newport_1830c::Newport1830CDriver::new_async(&port).await?,
                );
                // Newport1830C implements WavelengthTunable (bd-3xw2.5)
                let wavelength_range = driver.wavelength_range();
                Ok(RegisteredDevice {
                    config,
                    driver_type: driver_type_name.clone(),
                    movable: None,
                    readable: Some(driver.clone()),
                    triggerable: None,
                    frame_producer: None,
                    source_frame: None,
                    exposure_control: None,
                    settable: None,
                    stageable: None,
                    commandable: None,
                    parameterized: Some(driver.clone()),
                    shutter_control: None,
                    emission_control: None,
                    wavelength_tunable: Some(driver),
                    lifecycle: None,
                    metadata: DeviceMetadata {
                        measurement_units: Some("W".to_string()),
                        min_wavelength_nm: Some(wavelength_range.0),
                        max_wavelength_nm: Some(wavelength_range.1),
                        ..Default::default()
                    },
                })
            }

            #[cfg(feature = "spectra_physics")]
            DriverType::MaiTai { port } => {
                // Use new_async() to validate device identity on connection
                let driver = Arc::new(
                    daq_driver_spectra_physics::MaiTaiDriver::new_async_default(&port).await?,
                );
                Ok(RegisteredDevice {
                    config,
                    driver_type: driver_type_name.clone(),
                    movable: None,
                    readable: Some(driver.clone()),
                    triggerable: None,
                    frame_producer: None,
                    source_frame: None,
                    exposure_control: None,
                    settable: None,
                    stageable: None,
                    commandable: None,
                    parameterized: Some(driver.clone()),
                    shutter_control: Some(driver.clone()),
                    emission_control: Some(driver.clone()),
                    wavelength_tunable: Some(driver),
                    lifecycle: None,
                    metadata: DeviceMetadata {
                        measurement_units: Some("W".to_string()),
                        ..Default::default()
                    },
                })
            }

            #[cfg(feature = "newport")]
            DriverType::Esp300 { port, axis } => {
                // Use new_async() to validate device responds correctly on connection
                let driver =
                    Arc::new(crate::drivers::esp300::Esp300Driver::new_async(&port, axis).await?);
                Ok(RegisteredDevice {
                    config,
                    driver_type: driver_type_name.clone(),
                    movable: Some(driver.clone()),
                    readable: None,
                    triggerable: None,
                    frame_producer: None,
                    source_frame: None,
                    exposure_control: None,
                    settable: None,
                    stageable: None,
                    commandable: None,
                    parameterized: Some(driver),
                    shutter_control: None,
                    emission_control: None,
                    wavelength_tunable: None,
                    lifecycle: None,
                    metadata: DeviceMetadata {
                        position_units: Some("mm".to_string()),
                        min_position: Some(-25.0), // Typical ESP300 stage range
                        max_position: Some(25.0),
                        ..Default::default()
                    },
                })
            }

            // Handle disabled features
            #[cfg(all(not(feature = "thorlabs"), feature = "serial"))]
            DriverType::Ell14 { .. } => Err(anyhow!("ELL14 driver requires 'thorlabs' feature")),

            #[cfg(all(not(feature = "newport"), feature = "serial"))]
            DriverType::Newport1830C { .. } => {
                Err(anyhow!("Newport 1830-C driver requires 'newport' feature"))
            }

            #[cfg(all(not(feature = "spectra_physics"), feature = "serial"))]
            DriverType::MaiTai { .. } => {
                Err(anyhow!("MaiTai driver requires 'spectra_physics' feature"))
            }

            #[cfg(all(not(feature = "newport"), feature = "serial"))]
            DriverType::Esp300 { .. } => Err(anyhow!("ESP300 driver requires 'newport' feature")),

            // Factory-based Comedi drivers (must use factory registration)
            #[cfg(feature = "comedi")]
            DriverType::ComediAnalogInput { .. } => Err(anyhow!(
                "ComediAnalogInput devices must be registered via DriverFactory system"
            )),

            #[cfg(feature = "comedi")]
            DriverType::ComediAnalogOutput { .. } => Err(anyhow!(
                "ComediAnalogOutput devices must be registered via DriverFactory system"
            )),
        }
    }

    /// Instantiate a plugin-based device
    #[cfg(feature = "serial")]
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
    #[cfg(feature = "serial")]
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
                ));
            }
        };
        let driver_type_name = config.driver.driver_name().to_string();

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
                metadata.measurement_units = first_readable.unit.clone();
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
            parameterized: Some(driver.clone()), // bd-plb6: Wire Parameterized for plugin devices
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

        for entry in self.devices.iter() {
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
) -> Result<DeviceRegistry, DaqError> {
    let registry = DeviceRegistry::new();

    // Register all available driver factories BEFORE loading devices
    register_all_factories(&registry, None).await?;

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
        return Err(DaqError::Configuration(format!(
            "Hardware configuration validation failed:\n  - {}",
            validation_errors.join("\n  - ")
        )));
    }

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

    // Register all configured devices
    let mut success_count = 0;
    let mut failure_count = 0;

    for device_config in &config.devices {
        tracing::info!(
            device_id = %device_config.id,
            device_name = %device_config.name,
            driver_type = %device_config.driver.driver_name(),
            "Registering device"
        );

        let driver_type = device_config.driver.driver_name();

        // Check if a factory is registered for this driver type
        let result = if registry.has_factory(driver_type) {
            // Use factory-based registration
            match toml::Value::try_from(&device_config.driver) {
                Ok(toml_config) => {
                    registry
                        .register_from_toml(
                            &device_config.id,
                            &device_config.name,
                            driver_type,
                            toml_config,
                        )
                        .await
                }
                Err(e) => Err(DaqError::Configuration(format!(
                    "Failed to convert driver config to TOML: {}",
                    e
                ))),
            }
        } else {
            // Use legacy DriverType enum registration
            registry.register(device_config.clone()).await
        };

        if let Err(e) = result {
            failure_count += 1;
            registry.record_registration_failure(RegistrationFailure {
                device_id: device_config.id.clone(),
                device_name: device_config.name.clone(),
                driver_type: device_config.driver.driver_name().to_string(),
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
#[cfg(feature = "serial")]
pub async fn create_lab_registry() -> Result<DeviceRegistry, DaqError> {
    let registry = DeviceRegistry::new();

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

    // Prime BSI Camera (PVCAM)
    #[cfg(feature = "pvcam")]
    if let Err(e) = registry
        .register(DeviceConfig {
            id: "prime_bsi".into(),
            name: "Teledyne Prime BSI sCMOS".into(),
            driver: DriverType::Pvcam {
                camera_name: "PMUSBCam00".into(),
            },
        })
        .await
    {
        tracing::warn!("Prime BSI camera registration failed: {}", e);
    }

    Ok(registry)
}

#[cfg(not(feature = "serial"))]
pub async fn create_lab_registry() -> Result<DeviceRegistry, DaqError> {
    Ok(DeviceRegistry::new())
}

/// Create a DeviceRegistry with mock devices for testing
pub async fn create_mock_registry() -> Result<DeviceRegistry, DaqError> {
    let registry = DeviceRegistry::new();

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

    registry
        .register(DeviceConfig {
            id: "mock_camera".into(),
            name: "Mock Camera".into(),
            driver: DriverType::MockCamera {
                width: 640,
                height: 480,
            },
        })
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
    use daq_driver_mock::{MockCameraFactory, MockPowerMeterFactory, MockStageFactory};

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
        use daq_driver_thorlabs::Ell14Factory;
        registry.register_factory(Box::new(Ell14Factory));
    }

    // Register Newport factories
    #[cfg(feature = "newport")]
    {
        use daq_driver_newport::{Esp300Factory, Newport1830CFactory};
        registry.register_factory(Box::new(Esp300Factory));
        registry.register_factory(Box::new(Newport1830CFactory));
    }

    // Register Spectra-Physics factories
    #[cfg(feature = "spectra_physics")]
    {
        use daq_driver_spectra_physics::MaiTaiFactory;
        registry.register_factory(Box::new(MaiTaiFactory));
    }

    // Register Red Pitaya factories
    #[cfg(feature = "red_pitaya")]
    {
        use daq_driver_red_pitaya::RedPitayaPidFactory;
        registry.register_factory(Box::new(RedPitayaPidFactory));
    }

    // Register Comedi factories (NI DAQ, etc.)
    #[cfg(feature = "comedi")]
    {
        use daq_driver_comedi::{ComediAnalogInputFactory, ComediAnalogOutputFactory};
        registry.register_factory(Box::new(ComediAnalogInputFactory));
        registry.register_factory(Box::new(ComediAnalogOutputFactory));
    }

    // Load and register config-driven factories from TOML files
    #[cfg(feature = "serial")]
    if let Some(dir) = config_dir {
        if dir.exists() {
            match crate::factory::load_all_factories(dir) {
                Ok(factories) => {
                    for factory in factories {
                        let driver_type = factory.driver_type().to_string();
                        registry.register_factory(Box::new(factory));
                        tracing::debug!(driver_type = %driver_type, "Registered config factory");
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

    #[test]
    fn test_mock_camera_deserializes_with_default_resolution() {
        let driver: DriverType = serde_json::from_value(serde_json::json!({
            "type": "mock_camera"
        }))
        .unwrap();

        match driver {
            DriverType::MockCamera { width, height } => {
                assert_eq!(width, 640);
                assert_eq!(height, 480);
            }
            _ => panic!("Expected MockCamera driver"),
        }
    }

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
        let registry = create_registry_from_config(&config).await.unwrap();

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

    impl daq_core::driver::DeviceLifecycle for TestLifecycle {
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
        lifecycle: std::sync::Arc<dyn daq_core::driver::DeviceLifecycle>,
    }

    impl daq_core::driver::DriverFactory for TestFactory {
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

    impl daq_core::driver::DeviceLifecycle for FailingLifecycle {
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

    #[test]
    fn test_ell14_driver_capabilities() {
        // Verify ELL14 driver type has the Movable capability
        let driver = DriverType::Ell14 {
            port: "/dev/ttyUSB0".to_string(),
            address: "2".to_string(),
        };

        let capabilities = driver.capabilities();
        assert!(
            capabilities.contains(&Capability::Movable),
            "ELL14 should have Movable capability, got: {:?}",
            capabilities
        );
        assert_eq!(
            capabilities.len(),
            1,
            "ELL14 should have exactly one capability (Movable)"
        );

        // Verify driver name
        assert_eq!(driver.driver_name(), "ell14");
    }

    #[test]
    fn test_ell14_address_validation() {
        // Valid addresses: 0-9, A-F
        for addr in ["0", "1", "9", "A", "F", "a", "f"] {
            let result = validate_ell14_address(addr);
            assert!(result.is_ok(), "Address '{}' should be valid", addr);
        }

        // Invalid addresses
        for addr in ["", "00", "G", "z", "10", "FF"] {
            let result = validate_ell14_address(addr);
            assert!(result.is_err(), "Address '{}' should be invalid", addr);
        }
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

    /*
    #[tokio::test]
    async fn test_validate_driver_config_empty_pvcam_name() {
        let driver = DriverType::Pvcam {
            camera_name: "".to_string(),
        };

        let result = validate_driver_config(&driver);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }
    */

    #[tokio::test]
    async fn test_validate_driver_config_mock_devices_always_valid() {
        // Mock devices should always pass validation
        assert!(validate_driver_config(&DriverType::MockStage {
            initial_position: 0.0
        })
        .is_ok());

        assert!(validate_driver_config(&DriverType::MockPowerMeter { reading: 1e-6 }).is_ok());

        assert!(validate_driver_config(&DriverType::MockCamera {
            width: 640,
            height: 480
        })
        .is_ok());
    }

    #[tokio::test]
    async fn test_register_fails_on_invalid_config() {
        let registry = DeviceRegistry::new();

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
}
