//! Driver Factory - Creates config-driven drivers from TOML files.
//!
//! This module provides the [`DriverFactory`] which creates drivers from
//! device configuration files. The factory returns a [`ConfiguredDriver`]
//! enum that uses enum_dispatch for zero-overhead polymorphism.
//!
//! # Architecture
//!
//! The factory pattern enables:
//! - Config-driven device instantiation without code changes
//! - Type-safe capability trait access via enum_dispatch
//! - Unified interface for both config-based and hand-coded drivers
//!
//! # Example
//!
//! ```rust,ignore
//! use daq_hardware::factory::{DriverFactory, ConfiguredDriver};
//! use daq_hardware::capabilities::Movable;
//! use std::path::Path;
//!
//! // Create driver from config file
//! let driver = DriverFactory::create_from_file(
//!     Path::new("config/devices/ell14.toml"),
//!     shared_port,
//!     "2"
//! )?;
//!
//! // Use via Movable trait (zero-overhead dispatch)
//! driver.move_abs(45.0).await?;
//! ```

// Traits used by enum_dispatch macro expansion
#[allow(unused_imports)]
use crate::capabilities::{Movable, Readable, ShutterControl, WavelengthTunable};
use crate::config::load_device_config;
use crate::config::schema::DeviceConfig;
use crate::drivers::generic_serial::{GenericSerialDriver, SharedPort};
use anyhow::{anyhow, Context, Result};
use daq_core::driver::{
    Capability as CoreCapability, DeviceComponents, DriverFactory as DriverFactoryTrait,
};
use enum_dispatch::enum_dispatch;
use futures::future::BoxFuture;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

// =============================================================================
// ConfiguredDriver Enum
// =============================================================================

/// Unified driver type for config-driven devices.
///
/// This enum wraps all config-driven drivers, enabling trait dispatch
/// without dynamic dispatch overhead via enum_dispatch.
///
/// # Trait Implementation
///
/// The enum automatically implements all traits listed in the
/// `#[enum_dispatch(...)]` attribute by delegating to the inner driver.
///
/// # Adding New Device Types
///
/// To add a new device type:
/// 1. Create a TOML config in `config/devices/`
/// 2. (Optional) Add a named variant for type distinction
/// 3. Update the factory to recognize the device protocol
#[enum_dispatch(Movable, Readable, WavelengthTunable, ShutterControl)]
#[derive(Clone)]
pub enum ConfiguredDriver {
    /// ELL14 rotation mount (from config)
    Ell14(GenericSerialDriver),
    /// ESP300 motion controller (from config)
    Esp300(GenericSerialDriver),
    /// Newport 1830-C power meter (from config)
    Newport1830C(GenericSerialDriver),
    /// MaiTai tunable laser (from config)
    MaiTai(GenericSerialDriver),
    /// Generic device (any protocol)
    Generic(GenericSerialDriver),
}

impl ConfiguredDriver {
    /// Get the device protocol name
    pub fn protocol(&self) -> &str {
        match self {
            ConfiguredDriver::Ell14(d) => d.config().device.protocol.as_str(),
            ConfiguredDriver::Esp300(d) => d.config().device.protocol.as_str(),
            ConfiguredDriver::Newport1830C(d) => d.config().device.protocol.as_str(),
            ConfiguredDriver::MaiTai(d) => d.config().device.protocol.as_str(),
            ConfiguredDriver::Generic(d) => d.config().device.protocol.as_str(),
        }
    }

    /// Get the device name
    pub fn name(&self) -> &str {
        match self {
            ConfiguredDriver::Ell14(d) => d.config().device.name.as_str(),
            ConfiguredDriver::Esp300(d) => d.config().device.name.as_str(),
            ConfiguredDriver::Newport1830C(d) => d.config().device.name.as_str(),
            ConfiguredDriver::MaiTai(d) => d.config().device.name.as_str(),
            ConfiguredDriver::Generic(d) => d.config().device.name.as_str(),
        }
    }

    /// Get the device address
    pub fn address(&self) -> &str {
        match self {
            ConfiguredDriver::Ell14(d) => d.address(),
            ConfiguredDriver::Esp300(d) => d.address(),
            ConfiguredDriver::Newport1830C(d) => d.address(),
            ConfiguredDriver::MaiTai(d) => d.address(),
            ConfiguredDriver::Generic(d) => d.address(),
        }
    }

    /// Get the underlying GenericSerialDriver
    pub fn inner(&self) -> &GenericSerialDriver {
        match self {
            ConfiguredDriver::Ell14(d) => d,
            ConfiguredDriver::Esp300(d) => d,
            ConfiguredDriver::Newport1830C(d) => d,
            ConfiguredDriver::MaiTai(d) => d,
            ConfiguredDriver::Generic(d) => d,
        }
    }

    /// Get the underlying GenericSerialDriver (mutable)
    pub fn inner_mut(&mut self) -> &mut GenericSerialDriver {
        match self {
            ConfiguredDriver::Ell14(d) => d,
            ConfiguredDriver::Esp300(d) => d,
            ConfiguredDriver::Newport1830C(d) => d,
            ConfiguredDriver::MaiTai(d) => d,
            ConfiguredDriver::Generic(d) => d,
        }
    }
}

// =============================================================================
// Driver Factory
// =============================================================================

/// Factory for creating config-driven drivers.
///
/// The factory loads device configurations from TOML files and creates
/// appropriate driver instances wrapped in [`ConfiguredDriver`].
pub struct DriverFactory;

impl DriverFactory {
    /// Create a driver from a device configuration.
    ///
    /// # Arguments
    /// * `config` - Device configuration (typically loaded from TOML)
    /// * `port` - Shared serial port for communication
    /// * `address` - Device address (for RS-485 multidrop protocols)
    ///
    /// # Returns
    /// A [`ConfiguredDriver`] wrapping the appropriate driver type.
    ///
    /// # Errors
    /// Returns error if driver creation fails.
    pub fn create(
        config: DeviceConfig,
        port: SharedPort,
        address: &str,
    ) -> Result<ConfiguredDriver> {
        let protocol = config.device.protocol.to_lowercase();
        let driver = GenericSerialDriver::new(config, port, address)?;

        // Map protocol to enum variant
        let configured = match protocol.as_str() {
            "elliptec" | "ell14" => ConfiguredDriver::Ell14(driver),
            "esp300" | "newport_esp300" => ConfiguredDriver::Esp300(driver),
            "newport_1830c" | "newport1830c" => ConfiguredDriver::Newport1830C(driver),
            "maitai" | "mai_tai" => ConfiguredDriver::MaiTai(driver),
            _ => ConfiguredDriver::Generic(driver),
        };

        Ok(configured)
    }

    /// Create a driver from a TOML configuration file.
    ///
    /// **Note:** This is a synchronous version that does NOT run the init sequence.
    /// For production use, prefer [`create_from_file_async`] which validates the
    /// device by running the init sequence.
    ///
    /// # Arguments
    /// * `config_path` - Path to the TOML configuration file
    /// * `port` - Shared serial port for communication
    /// * `address` - Device address (for RS-485 multidrop protocols)
    pub fn create_from_file(
        config_path: &Path,
        port: SharedPort,
        address: &str,
    ) -> Result<ConfiguredDriver> {
        let config = load_device_config(config_path)
            .with_context(|| format!("Failed to load config: {}", config_path.display()))?;

        Self::create(config, port, address)
    }

    /// Create a driver with device validation via init sequence.
    ///
    /// This is the **preferred method** for production use. It:
    /// 1. Creates the driver from the config
    /// 2. Runs the `init_sequence` defined in the TOML config
    /// 3. Validates responses match expected patterns
    ///
    /// # Arguments
    /// * `config` - Loaded device configuration
    /// * `port` - Shared serial port for communication
    /// * `address` - Device address (for RS-485 multidrop protocols)
    ///
    /// # Errors
    /// Returns error if:
    /// - Driver creation fails
    /// - Init sequence fails (device doesn't respond correctly)
    pub async fn create_async(
        config: DeviceConfig,
        port: SharedPort,
        address: &str,
    ) -> Result<ConfiguredDriver> {
        let protocol = config.device.protocol.to_lowercase();
        let driver = GenericSerialDriver::new(config, port, address)?;

        // Run init sequence to validate device responds correctly
        driver.run_init_sequence().await.with_context(|| {
            format!(
                "Device validation failed for protocol '{}' at address '{}'. \
                 Check that the correct device is connected.",
                protocol, address
            )
        })?;

        // Map protocol to enum variant
        let configured = match protocol.as_str() {
            "elliptec" | "ell14" => ConfiguredDriver::Ell14(driver),
            "esp300" | "newport_esp300" => ConfiguredDriver::Esp300(driver),
            "newport_1830c" | "newport1830c" => ConfiguredDriver::Newport1830C(driver),
            "maitai" | "mai_tai" => ConfiguredDriver::MaiTai(driver),
            _ => ConfiguredDriver::Generic(driver),
        };

        Ok(configured)
    }

    /// Create a driver from a TOML file with device validation.
    ///
    /// This is the **preferred method** for production use. It runs the
    /// `init_sequence` defined in the TOML config to validate the device.
    ///
    /// # Arguments
    /// * `config_path` - Path to the TOML configuration file
    /// * `port` - Shared serial port for communication
    /// * `address` - Device address (for RS-485 multidrop protocols)
    ///
    /// # Example
    /// ```rust,ignore
    /// let driver = DriverFactory::create_from_file_async(
    ///     Path::new("config/devices/ell14.toml"),
    ///     shared_port,
    ///     "2"
    /// ).await?;
    /// ```
    pub async fn create_from_file_async(
        config_path: &Path,
        port: SharedPort,
        address: &str,
    ) -> Result<ConfiguredDriver> {
        let config = load_device_config(config_path)
            .with_context(|| format!("Failed to load config: {}", config_path.display()))?;

        Self::create_async(config, port, address).await
    }

    /// Create a driver with custom calibration parameter and device validation.
    ///
    /// Useful for devices that need runtime calibration override.
    /// Runs the init sequence to validate the device responds correctly.
    ///
    /// # Arguments
    /// * `config_path` - Path to the TOML configuration file
    /// * `port` - Shared serial port for communication
    /// * `address` - Device address
    /// * `pulses_per_degree` - Custom calibration factor
    ///
    /// # Errors
    /// Returns error if:
    /// - Config loading or driver creation fails
    /// - Init sequence fails (device doesn't respond correctly)
    pub async fn create_calibrated(
        config_path: &Path,
        port: SharedPort,
        address: &str,
        pulses_per_degree: f64,
    ) -> Result<ConfiguredDriver> {
        let config = load_device_config(config_path)
            .with_context(|| format!("Failed to load config: {}", config_path.display()))?;

        let protocol = config.device.protocol.to_lowercase();
        let driver = GenericSerialDriver::new(config, port, address)?;

        // Run init sequence to validate device responds correctly
        driver.run_init_sequence().await.with_context(|| {
            format!(
                "Device validation failed for protocol '{}' at address '{}'. \
                 Check that the correct device is connected.",
                protocol, address
            )
        })?;

        // Set custom calibration
        driver
            .set_parameter("pulses_per_degree", pulses_per_degree)
            .await;

        let configured = match protocol.as_str() {
            "elliptec" | "ell14" => ConfiguredDriver::Ell14(driver),
            "esp300" | "newport_esp300" => ConfiguredDriver::Esp300(driver),
            "newport_1830c" | "newport1830c" => ConfiguredDriver::Newport1830C(driver),
            "maitai" | "mai_tai" => ConfiguredDriver::MaiTai(driver),
            _ => ConfiguredDriver::Generic(driver),
        };

        Ok(configured)
    }
}

// =============================================================================
// Config-Based Bus (equivalent to Ell14Bus)
// =============================================================================

/// Config-driven bus for RS-485 multidrop devices.
///
/// This is the config-based equivalent of [`Ell14Bus`], providing a
/// bus-centric API for managing multiple devices on a shared serial port.
///
/// # Example
///
/// ```rust,ignore
/// use daq_hardware::factory::ConfiguredBus;
/// use std::path::Path;
///
/// // Open bus with ELL14 config
/// let bus = ConfiguredBus::open(
///     "/dev/ttyUSB1",
///     Path::new("config/devices/ell14.toml")
/// ).await?;
///
/// // Get device handles
/// let rotator_2 = bus.device("2")?;
/// let rotator_3 = bus.device("3")?;
///
/// // Use via Movable trait
/// rotator_2.move_abs(45.0).await?;
/// ```
#[derive(Clone)]
pub struct ConfiguredBus {
    port: SharedPort,
    config: DeviceConfig,
    port_path: String,
}

impl ConfiguredBus {
    /// Open a bus connection using a device configuration.
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyUSB1")
    /// * `config_path` - Path to device TOML configuration
    #[cfg(feature = "serial")]
    pub async fn open(port_path: &str, config_path: &Path) -> Result<Self> {
        use crate::port_resolver::resolve_port;
        use tokio_serial::SerialPortBuilderExt;

        let config = load_device_config(config_path)?;

        // Resolve port path
        let resolved_path = resolve_port(port_path)
            .map_err(|e| anyhow!("Failed to resolve port '{}': {}", port_path, e))?;

        // Open port with config settings
        let port_path_clone = resolved_path.clone();
        let baud = config.connection.baud_rate;

        let port = tokio::task::spawn_blocking(move || {
            tokio_serial::new(&port_path_clone, baud)
                .data_bits(tokio_serial::DataBits::Eight)
                .parity(tokio_serial::Parity::None)
                .stop_bits(tokio_serial::StopBits::One)
                .flow_control(tokio_serial::FlowControl::None)
                .open_native_async()
                .context("Failed to open serial port")
        })
        .await
        .context("spawn_blocking failed")??;

        let boxed_port: crate::drivers::generic_serial::DynSerial = Box::new(port);

        Ok(Self {
            port: std::sync::Arc::new(tokio::sync::Mutex::new(boxed_port)),
            config,
            port_path: resolved_path,
        })
    }

    /// Get a device handle for an address on this bus.
    pub fn device(&self, address: &str) -> Result<ConfiguredDriver> {
        DriverFactory::create(self.config.clone(), self.port.clone(), address)
    }

    /// Get the serial port path
    pub fn port_path(&self) -> &str {
        &self.port_path
    }

    /// Get the device configuration
    pub fn config(&self) -> &DeviceConfig {
        &self.config
    }

    /// Get the shared port (for advanced use)
    pub fn shared_port(&self) -> SharedPort {
        self.port.clone()
    }
}

// =============================================================================
// GenericSerialDriverFactory - Implements DriverFactory Trait
// =============================================================================

/// Configuration for GenericSerialDriver instances.
///
/// This config is passed to `build()` and contains instance-specific
/// settings like port path and device address.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct GenericSerialInstanceConfig {
    /// Serial port path (e.g., "/dev/ttyUSB0")
    pub port: String,

    /// Device address on the bus (for RS-485 multidrop protocols)
    #[serde(default = "default_address")]
    pub address: String,

    /// Baud rate override (uses config default if not specified)
    pub baud_rate: Option<u32>,
}

fn default_address() -> String {
    "0".to_string()
}

/// Factory for creating GenericSerialDriver instances from TOML device configs.
///
/// This factory bridges the TOML-based config system with the DriverFactory trait,
/// enabling config-driven devices to be registered via the plugin architecture.
///
/// # Example
///
/// ```rust,ignore
/// use daq_hardware::factory::GenericSerialDriverFactory;
/// use daq_hardware::config::load_device_config;
/// use std::path::Path;
///
/// // Load device definition TOML
/// let device_config = load_device_config(Path::new("config/devices/ell14.toml"))?;
///
/// // Create factory with embedded device config
/// let factory = GenericSerialDriverFactory::new(device_config);
///
/// // Register with DeviceRegistry
/// registry.register_factory(Box::new(factory));
///
/// // Later, register device instances with port/address config
/// let instance_config = toml::from_str(r#"
///     port = "/dev/ttyUSB1"
///     address = "2"
/// "#)?;
/// registry.register_from_toml("rotator_2", "ELL14 Rotator #2", "ell14", instance_config).await?;
/// ```
pub struct GenericSerialDriverFactory {
    /// The device configuration (commands, responses, conversions, etc.)
    device_config: DeviceConfig,

    /// Cached capabilities derived from device config
    capabilities: Vec<CoreCapability>,

    /// Driver type string (from device protocol)
    driver_type: String,

    /// Human-readable name
    name: String,

    /// Optional shared port cache for RS-485 bus sharing
    /// Map from port path -> shared port
    #[cfg(feature = "serial")]
    port_cache: Arc<std::sync::Mutex<std::collections::HashMap<String, SharedPort>>>,
}

impl GenericSerialDriverFactory {
    /// Create a new factory with the given device configuration.
    ///
    /// The factory extracts the driver type from the device protocol
    /// and capabilities from the device config.
    pub fn new(device_config: DeviceConfig) -> Self {
        let driver_type = device_config.device.protocol.to_lowercase();
        let name = device_config.device.name.clone();

        // Convert device config capabilities to CoreCapability
        use crate::config::schema::CapabilityType;
        let capabilities = device_config
            .device
            .capabilities
            .iter()
            .filter_map(|cap| match cap {
                CapabilityType::Movable => Some(CoreCapability::Movable),
                CapabilityType::Readable => Some(CoreCapability::Readable),
                CapabilityType::WavelengthTunable => Some(CoreCapability::WavelengthTunable),
                CapabilityType::ShutterControl => Some(CoreCapability::ShutterControl),
                CapabilityType::EmissionControl => Some(CoreCapability::EmissionControl),
                CapabilityType::Triggerable => Some(CoreCapability::Triggerable),
                CapabilityType::ExposureControl => Some(CoreCapability::ExposureControl),
                CapabilityType::Settable => Some(CoreCapability::Settable),
                CapabilityType::Stageable => Some(CoreCapability::Stageable),
                CapabilityType::Commandable => Some(CoreCapability::Commandable),
                CapabilityType::Parameterized => Some(CoreCapability::Parameterized),
                _ => None,
            })
            .collect();

        Self {
            device_config,
            capabilities,
            driver_type,
            name,
            #[cfg(feature = "serial")]
            port_cache: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// Create a new factory from a TOML configuration file.
    pub fn from_file(config_path: &Path) -> Result<Self> {
        let device_config = load_device_config(config_path)
            .with_context(|| format!("Failed to load config: {}", config_path.display()))?;
        Ok(Self::new(device_config))
    }

    /// Get the embedded device configuration.
    pub fn device_config(&self) -> &DeviceConfig {
        &self.device_config
    }
}

impl DriverFactoryTrait for GenericSerialDriverFactory {
    fn driver_type(&self) -> &'static str {
        // Leak the string to get 'static lifetime
        // This is acceptable since factories are long-lived
        Box::leak(self.driver_type.clone().into_boxed_str())
    }

    fn name(&self) -> &'static str {
        Box::leak(self.name.clone().into_boxed_str())
    }

    fn capabilities(&self) -> &'static [CoreCapability] {
        // Leak the slice to get 'static lifetime
        Box::leak(self.capabilities.clone().into_boxed_slice())
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        // Try to deserialize as instance config
        let _: GenericSerialInstanceConfig = config.clone().try_into().map_err(|e| {
            anyhow!(
                "Invalid instance config for '{}': {}. \
                 Expected 'port' (string) and optional 'address' (string)",
                self.driver_type,
                e
            )
        })?;
        Ok(())
    }

    #[cfg(feature = "serial")]
    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        let device_config = self.device_config.clone();
        let port_cache = self.port_cache.clone();
        let driver_type = self.driver_type.clone();

        Box::pin(async move {
            let instance: GenericSerialInstanceConfig = config.try_into().map_err(|e| {
                anyhow!(
                    "Failed to parse instance config for '{}': {}",
                    driver_type,
                    e
                )
            })?;

            // Determine baud rate
            let baud_rate = instance
                .baud_rate
                .unwrap_or(device_config.connection.baud_rate);

            // Get or open the shared port
            let resolved_path = crate::port_resolver::resolve_port(&instance.port)
                .map_err(|e| anyhow!("Failed to resolve port '{}': {}", instance.port, e))?;

            let shared_port = {
                let cache = port_cache.lock().unwrap_or_else(|p| p.into_inner());
                cache.get(&resolved_path).cloned()
            };

            let shared_port = match shared_port {
                Some(port) => port,
                None => {
                    use tokio_serial::SerialPortBuilderExt;

                    let path_clone = resolved_path.clone();
                    let port = tokio::task::spawn_blocking(move || {
                        tokio_serial::new(&path_clone, baud_rate)
                            .data_bits(tokio_serial::DataBits::Eight)
                            .parity(tokio_serial::Parity::None)
                            .stop_bits(tokio_serial::StopBits::One)
                            .flow_control(tokio_serial::FlowControl::None)
                            .open_native_async()
                            .context("Failed to open serial port")
                    })
                    .await
                    .context("spawn_blocking failed")??;

                    let boxed: crate::drivers::generic_serial::DynSerial = Box::new(port);
                    let shared: SharedPort = Arc::new(Mutex::new(boxed));

                    // Cache it
                    {
                        let mut cache = port_cache.lock().unwrap_or_else(|p| p.into_inner());
                        cache.insert(resolved_path, shared.clone());
                    }

                    shared
                }
            };

            // Create the driver
            let driver = GenericSerialDriver::new(device_config, shared_port, &instance.address)?;

            // Run init sequence to validate device
            driver.run_init_sequence().await.with_context(|| {
                format!(
                    "Device validation failed for '{}' at address '{}'. \
                     Check that the correct device is connected.",
                    driver_type, instance.address
                )
            })?;

            // Build DeviceComponents with capabilities
            // GenericSerialDriver implements all the capability traits directly
            let driver_arc = Arc::new(driver);

            Ok(DeviceComponents {
                movable: Some(driver_arc.clone() as Arc<dyn crate::capabilities::Movable>),
                readable: Some(driver_arc.clone() as Arc<dyn crate::capabilities::Readable>),
                wavelength_tunable: Some(
                    driver_arc.clone() as Arc<dyn crate::capabilities::WavelengthTunable>
                ),
                shutter_control: Some(driver_arc as Arc<dyn crate::capabilities::ShutterControl>),
                ..Default::default()
            })
        })
    }

    #[cfg(not(feature = "serial"))]
    fn build(&self, _config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            Err(anyhow!(
                "Serial feature not enabled. Cannot build GenericSerialDriver."
            ))
        })
    }
}

/// Load all device configs from a directory and create factories.
///
/// This is useful for loading all device definitions at startup.
///
/// # Example
///
/// ```rust,ignore
/// use daq_hardware::factory::load_all_factories;
/// use std::path::Path;
///
/// let factories = load_all_factories(Path::new("config/devices"))?;
/// for factory in factories {
///     registry.register_factory(Box::new(factory));
/// }
/// ```
pub fn load_all_factories(dir: &Path) -> Result<Vec<GenericSerialDriverFactory>> {
    use crate::config::load_all_devices;

    let configs = load_all_devices(dir)?;
    Ok(configs
        .into_iter()
        .map(GenericSerialDriverFactory::new)
        .collect())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_device_config_from_str;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    const TEST_CONFIG: &str = r#"
[device]
name = "Test ELL14"
protocol = "elliptec"
capabilities = ["Movable"]

[connection]
type = "serial"
timeout_ms = 500

[parameters.pulses_per_degree]
type = "float"
default = 398.2222

[commands.move_absolute]
template = "${address}ma${position_pulses:08X}"
parameters = { position_pulses = "int32" }

[commands.get_position]
template = "${address}gp"
response = "position"

[responses.position]
pattern = "^(?P<addr>[0-9A-Fa-f])PO(?P<pulses>[0-9A-Fa-f]{1,8})$"

[responses.position.fields.addr]
type = "string"

[responses.position.fields.pulses]
type = "hex_i32"
signed = true

[conversions.degrees_to_pulses]
formula = "round(degrees * pulses_per_degree)"

[conversions.pulses_to_degrees]
formula = "pulses / pulses_per_degree"

[trait_mapping.Movable.move_abs]
command = "move_absolute"
input_conversion = "degrees_to_pulses"
input_param = "position_pulses"
from_param = "position"

[trait_mapping.Movable.position]
command = "get_position"
output_conversion = "pulses_to_degrees"
output_field = "pulses"
"#;

    /// Mock serial port for testing
    struct MockPort;

    impl tokio::io::AsyncRead for MockPort {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    impl tokio::io::AsyncWrite for MockPort {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::task::Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    impl Unpin for MockPort {}

    fn mock_port() -> SharedPort {
        Arc::new(Mutex::new(
            Box::new(MockPort) as crate::drivers::generic_serial::DynSerial
        ))
    }

    #[test]
    fn test_factory_create_ell14() {
        let config = load_device_config_from_str(TEST_CONFIG).unwrap();
        let port = mock_port();

        let driver = DriverFactory::create(config, port, "2").unwrap();

        // Should be Ell14 variant due to "elliptec" protocol
        assert!(matches!(driver, ConfiguredDriver::Ell14(_)));
        assert_eq!(driver.protocol(), "elliptec");
        assert_eq!(driver.address(), "2");
    }

    #[test]
    fn test_factory_create_generic() {
        let config_str = r#"
[device]
name = "Generic Device"
protocol = "custom_protocol"

[connection]
type = "serial"
"#;
        let config = load_device_config_from_str(config_str).unwrap();
        let port = mock_port();

        let driver = DriverFactory::create(config, port, "0").unwrap();

        // Should be Generic variant due to unknown protocol
        assert!(matches!(driver, ConfiguredDriver::Generic(_)));
        assert_eq!(driver.protocol(), "custom_protocol");
    }

    #[test]
    fn test_configured_driver_name() {
        let config = load_device_config_from_str(TEST_CONFIG).unwrap();
        let port = mock_port();

        let driver = DriverFactory::create(config, port, "2").unwrap();

        assert_eq!(driver.name(), "Test ELL14");
    }
}
