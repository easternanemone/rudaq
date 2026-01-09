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
use crate::config::schema::DeviceConfig;
use crate::config::load_device_config;
use crate::drivers::generic_serial::{GenericSerialDriver, SharedPort};
use anyhow::{anyhow, Context, Result};
use enum_dispatch::enum_dispatch;
use std::path::Path;

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
    /// # Arguments
    /// * `config_path` - Path to the TOML configuration file
    /// * `port` - Shared serial port for communication
    /// * `address` - Device address (for RS-485 multidrop protocols)
    ///
    /// # Example
    /// ```rust,ignore
    /// let driver = DriverFactory::create_from_file(
    ///     Path::new("config/devices/ell14.toml"),
    ///     shared_port,
    ///     "2"
    /// )?;
    /// ```
    pub fn create_from_file(
        config_path: &Path,
        port: SharedPort,
        address: &str,
    ) -> Result<ConfiguredDriver> {
        let config = load_device_config(config_path)
            .with_context(|| format!("Failed to load config: {}", config_path.display()))?;

        Self::create(config, port, address)
    }

    /// Create a driver with custom calibration parameter.
    ///
    /// Useful for devices that need runtime calibration override.
    ///
    /// # Arguments
    /// * `config_path` - Path to the TOML configuration file
    /// * `port` - Shared serial port for communication
    /// * `address` - Device address
    /// * `pulses_per_degree` - Custom calibration factor
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

        // Set custom calibration
        driver.set_parameter("pulses_per_degree", pulses_per_degree).await;

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
    #[cfg(feature = "tokio_serial")]
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
        Arc::new(Mutex::new(Box::new(MockPort) as crate::drivers::generic_serial::DynSerial))
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
