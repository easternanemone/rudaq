//! Pure data types for the device registry.
//!
//! These types are extracted from `registry/mod.rs` so the parent file
//! can focus on the runtime registry implementation. They have no
//! dependency on registry-internal state and are safe to use from
//! anywhere in the workspace.

use super::DeviceId;
use common::capabilities::DeviceCategory;
use common::driver::{Capability, ManifestFeatureMeta};
use common::error::DaqError;
use common::health::DeviceHealth;
use serde::{Deserialize, Serialize};

// =============================================================================
// Health events
// =============================================================================

/// Event emitted when a device's health state changes.
#[derive(Debug, Clone)]
pub struct DeviceHealthEvent {
    /// The device whose health changed.
    pub device_id: String,
    /// Health state before the change.
    pub old_state: DeviceHealth,
    /// Health state after the change.
    pub new_state: DeviceHealth,
    /// Human-readable reason for the transition.
    pub reason: String,
    /// Timestamp when the transition occurred.
    pub timestamp: std::time::Instant,
    /// Number of consecutive failures at the time of the event.
    pub consecutive_failures: u32,
    /// Total restart attempts at the time of the event.
    pub restart_attempts: u32,
}

// =============================================================================
// Device configuration
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
// Device introspection
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
    pub category: Option<DeviceCategory>,
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
    /// Available command names for command-capable devices.
    pub available_commands: Vec<String>,
    /// Optional serialized UI schema/configuration for metadata-driven control panels.
    pub ui_schema_json: Option<String>,
    /// Explicit panel routing hint for the UI (see `common::panel_kind` constants).
    pub panel_kind: Option<String>,
    /// Config origin: "toml" (startup), "db" (reconciler), etc.
    pub config_source: Option<String>,
    /// Static feature metadata from device manifest (for universal/TOML-driven devices).
    ///
    /// Used as a fallback by the reconciler when `Parameterized` is not available.
    pub manifest_features: Vec<ManifestFeatureMeta>,
}

// =============================================================================
// Registration outcomes
// =============================================================================

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
    /// Available command names this driver advertises.
    pub available_commands: Vec<String>,
}

// =============================================================================
// Hardware configuration file types
// =============================================================================

/// Configuration for the safety heartbeat digital output pulse.
///
/// When enabled, a Tokio task toggles a Comedi digital output bit at a fixed
/// interval. An external hardware interlock monitors this pulse train: if the
/// daemon crashes or hangs, the pulse stops, and the interlock cuts laser power.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatConfig {
    /// Whether the heartbeat is enabled (default: true).
    #[serde(default = "default_heartbeat_enabled")]
    pub enabled: bool,
    /// Comedi device path (e.g., "/dev/comedi0").
    pub device: String,
    /// DIO subdevice index. `None` means auto-detect the first DIO subdevice.
    #[serde(default)]
    pub subdevice: Option<u32>,
    /// DIO channel number to toggle.
    pub channel: u32,
    /// Toggle interval in milliseconds (default: 100).
    #[serde(default = "default_heartbeat_interval_ms")]
    pub interval_ms: u64,
}

pub(super) fn default_heartbeat_enabled() -> bool {
    true
}

pub(super) fn default_heartbeat_interval_ms() -> u64 {
    100
}

/// Hardware configuration loaded from a TOML file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareConfig {
    /// Plugin search paths (in priority order, first = highest priority)
    /// Convention: user paths before system paths
    #[serde(default)]
    pub plugin_paths: Vec<std::path::PathBuf>,

    /// List of devices to register
    pub devices: Vec<DeviceConfig>,

    /// Optional safety heartbeat configuration.
    ///
    /// When present and enabled, the daemon toggles a Comedi DIO channel
    /// to drive an external hardware interlock.
    #[serde(default)]
    pub safety_heartbeat: Option<HeartbeatConfig>,
}

impl HardwareConfig {
    /// Load hardware configuration from a TOML file
    pub fn from_file(path: &std::path::Path) -> Result<Self, DaqError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            DaqError::Configuration(format!("Failed to read hardware config file: {e}"))
        })?;
        toml::from_str(&content).map_err(|e| {
            DaqError::Configuration(format!("Failed to parse hardware config file: {e}"))
        })
    }
}
