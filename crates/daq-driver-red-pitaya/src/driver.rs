//! Red Pitaya PID Controller Driver
//!
//! This module implements the driver for Red Pitaya STEMlab boards running
//! custom PID FPGA bitstreams for laser power stabilization.
//!
//! # Parameters
//!
//! The driver exposes the following parameters:
//!
//! - `power` (read-only) - Current measured power in mW
//! - `setpoint` - Target power setpoint in mW
//! - `error` (read-only) - Current error signal (setpoint - measured)
//! - `pid_output` (read-only) - Current PID output value
//! - `kp` - Proportional gain
//! - `ki` - Integral gain
//! - `kd` - Derivative gain
//! - `output_min` - Minimum output limit
//! - `output_max` - Maximum output limit
//! - `enabled` - PID loop enable/disable

use crate::scpi::{MockScpiClient, ScpiClient, DEFAULT_PORT};
use anyhow::{Context, Result};
use async_trait::async_trait;
use common::capabilities::{Parameterized, Readable};
use common::driver::{Capability, DeviceComponents, DeviceMetadata, DriverFactory};
use common::error::DaqError;
use common::observable::ParameterSet;
use common::parameter::Parameter;
use futures::future::BoxFuture;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for Red Pitaya PID driver
#[derive(Debug, Clone, Deserialize)]
pub struct RedPitayaPidConfig {
    /// Hostname or IP address of the Red Pitaya
    pub host: String,

    /// SCPI port (default: 5000)
    #[serde(default = "default_port")]
    pub port: u16,

    /// Enable mock mode for testing without hardware
    #[serde(default)]
    pub mock: bool,

    /// Initial setpoint in mW (optional)
    #[serde(default)]
    pub initial_setpoint: Option<f64>,

    /// Initial Kp gain (optional)
    #[serde(default)]
    pub initial_kp: Option<f64>,

    /// Initial Ki gain (optional)
    #[serde(default)]
    pub initial_ki: Option<f64>,

    /// Initial Kd gain (optional)
    #[serde(default)]
    pub initial_kd: Option<f64>,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

// =============================================================================
// SCPI Client Abstraction
// =============================================================================

/// Trait for SCPI client operations (allows mock injection)
#[async_trait]
trait ScpiOps: Send + Sync {
    async fn write(&self, command: &str) -> Result<()>;
    async fn query_f64(&self, query: &str) -> Result<f64>;
    async fn query_bool(&self, query: &str) -> Result<bool>;
}

#[async_trait]
impl ScpiOps for ScpiClient {
    async fn write(&self, command: &str) -> Result<()> {
        self.write(command).await
    }
    async fn query_f64(&self, query: &str) -> Result<f64> {
        self.query_f64(query).await
    }
    async fn query_bool(&self, query: &str) -> Result<bool> {
        self.query_bool(query).await
    }
}

#[async_trait]
impl ScpiOps for MockScpiClient {
    async fn write(&self, command: &str) -> Result<()> {
        self.write(command).await
    }
    async fn query_f64(&self, query: &str) -> Result<f64> {
        self.query_f64(query).await
    }
    async fn query_bool(&self, query: &str) -> Result<bool> {
        self.query_bool(query).await
    }
}

// =============================================================================
// RedPitayaPidDriver
// =============================================================================

/// Driver for Red Pitaya FPGA-based PID controller.
///
/// This driver communicates with Red Pitaya devices running custom PID FPGA
/// bitstreams via SCPI over TCP. It is designed for laser power stabilization
/// feedback loops.
///
/// # Example
///
/// ```rust,ignore
/// let driver = RedPitayaPidDriver::new_async("192.168.1.100", 5000, false).await?;
///
/// // Read current power
/// let power = driver.read().await?;
///
/// // Set new setpoint via parameter
/// driver.setpoint().set(1.5).await?;
///
/// // Enable PID loop
/// driver.enabled().set(true).await?;
/// ```
pub struct RedPitayaPidDriver {
    /// SCPI client for communication
    client: Arc<dyn ScpiOps>,

    /// Parameter registry
    params: Arc<ParameterSet>,

    // Parameters with hardware callbacks
    /// Current power reading (read-only, updated by read())
    power: Parameter<f64>,
    /// Target setpoint in mW
    setpoint: Parameter<f64>,
    /// Current error signal (read-only)
    error: Parameter<f64>,
    /// Current PID output (read-only)
    pid_output: Parameter<f64>,
    /// Proportional gain
    kp: Parameter<f64>,
    /// Integral gain
    ki: Parameter<f64>,
    /// Derivative gain
    kd: Parameter<f64>,
    /// Minimum output limit
    output_min: Parameter<f64>,
    /// Maximum output limit
    output_max: Parameter<f64>,
    /// PID enable state
    enabled: Parameter<bool>,

    /// Cache for last power reading (for Readable trait)
    _last_power: Arc<RwLock<f64>>,
}

impl RedPitayaPidDriver {
    /// Create a new driver with hardware connection.
    ///
    /// # Arguments
    /// * `host` - Hostname or IP address
    /// * `port` - SCPI port (typically 5000)
    /// * `mock` - If true, use mock client for testing
    ///
    /// # Returns
    /// * `Ok(Arc<Self>)` on successful connection
    /// * `Err` if connection fails
    pub async fn new_async(host: &str, port: u16, mock: bool) -> Result<Arc<Self>> {
        let client: Arc<dyn ScpiOps> = if mock {
            tracing::info!("Creating mock Red Pitaya PID driver");
            Arc::new(MockScpiClient::new())
        } else {
            tracing::info!("Connecting to Red Pitaya at {}:{}", host, port);
            Arc::new(ScpiClient::connect(host, port).await?)
        };

        let driver = Self::build(client);
        let driver = Arc::new(driver);

        // Validate connection by querying device
        driver.validate_connection().await?;

        tracing::info!("Red Pitaya PID driver initialized (mock={})", mock);

        Ok(driver)
    }

    /// Build driver with given SCPI client.
    fn build(client: Arc<dyn ScpiOps>) -> Self {
        let mut params = ParameterSet::new();
        let last_power = Arc::new(RwLock::new(0.0));

        // Create parameters
        let power = Parameter::new("power", 0.0)
            .with_description("Current measured power")
            .with_unit("mW");

        let error = Parameter::new("error", 0.0)
            .with_description("Current error signal (setpoint - measured)")
            .with_unit("mW");

        let pid_output =
            Parameter::new("pid_output", 0.0).with_description("Current PID controller output");

        let mut setpoint = Parameter::new("setpoint", 1.0)
            .with_description("Target power setpoint")
            .with_unit("mW")
            .with_range(0.0, 100.0);

        let mut kp = Parameter::new("kp", 1.0)
            .with_description("Proportional gain")
            .with_range(0.0, 100.0);

        let mut ki = Parameter::new("ki", 0.1)
            .with_description("Integral gain")
            .with_range(0.0, 100.0);

        let mut kd = Parameter::new("kd", 0.0)
            .with_description("Derivative gain")
            .with_range(0.0, 100.0);

        let mut output_min = Parameter::new("output_min", 0.0)
            .with_description("Minimum output limit")
            .with_range(-10.0, 10.0);

        let mut output_max = Parameter::new("output_max", 1.0)
            .with_description("Maximum output limit")
            .with_range(-10.0, 10.0);

        let mut enabled =
            Parameter::new("enabled", false).with_description("PID loop enable state");

        // Attach hardware callbacks
        Self::attach_setpoint_callback(&mut setpoint, client.clone());
        Self::attach_kp_callback(&mut kp, client.clone());
        Self::attach_ki_callback(&mut ki, client.clone());
        Self::attach_kd_callback(&mut kd, client.clone());
        Self::attach_output_min_callback(&mut output_min, client.clone());
        Self::attach_output_max_callback(&mut output_max, client.clone());
        Self::attach_enabled_callback(&mut enabled, client.clone());

        // Register parameters
        params.register(power.clone());
        params.register(setpoint.clone());
        params.register(error.clone());
        params.register(pid_output.clone());
        params.register(kp.clone());
        params.register(ki.clone());
        params.register(kd.clone());
        params.register(output_min.clone());
        params.register(output_max.clone());
        params.register(enabled.clone());

        Self {
            client,
            params: Arc::new(params),
            power,
            setpoint,
            error,
            pid_output,
            kp,
            ki,
            kd,
            output_min,
            output_max,
            enabled,
            _last_power: last_power,
        }
    }

    /// Validate connection by querying the device.
    ///
    /// This reads a value from the device to verify communication works.
    /// Unlike some other drivers, we don't sync all parameters at startup
    /// because the Parameter callbacks would trigger hardware writes.
    async fn validate_connection(&self) -> Result<()> {
        // Query power to verify connection
        let power = self.client.query_f64("PID:INP?").await?;
        tracing::debug!("Validated connection: power={}", power);
        Ok(())
    }

    /// Read current power from hardware.
    ///
    /// Note: This only returns the power value. The error and pid_output
    /// parameters are not automatically updated - use the query methods
    /// directly if those values are needed.
    async fn read_power(&self) -> Result<f64> {
        self.client.query_f64("PID:INP?").await
    }

    /// Query the current error signal from hardware.
    pub async fn query_error(&self) -> Result<f64> {
        self.client.query_f64("PID:ERR?").await
    }

    /// Query the current PID output from hardware.
    pub async fn query_pid_output(&self) -> Result<f64> {
        self.client.query_f64("PID:OUT?").await
    }

    /// Query the current setpoint from hardware.
    pub async fn query_setpoint(&self) -> Result<f64> {
        self.client.query_f64("PID:SETP?").await
    }

    /// Query the current Kp gain from hardware.
    pub async fn query_kp(&self) -> Result<f64> {
        self.client.query_f64("PID:KP?").await
    }

    /// Query the current Ki gain from hardware.
    pub async fn query_ki(&self) -> Result<f64> {
        self.client.query_f64("PID:KI?").await
    }

    /// Query the current Kd gain from hardware.
    pub async fn query_kd(&self) -> Result<f64> {
        self.client.query_f64("PID:KD?").await
    }

    /// Query the output minimum limit from hardware.
    pub async fn query_output_min(&self) -> Result<f64> {
        self.client.query_f64("PID:OMIN?").await
    }

    /// Query the output maximum limit from hardware.
    pub async fn query_output_max(&self) -> Result<f64> {
        self.client.query_f64("PID:OMAX?").await
    }

    /// Query the enabled state from hardware.
    pub async fn query_enabled(&self) -> Result<bool> {
        self.client.query_bool("PID:EN?").await
    }

    // =========================================================================
    // Hardware Callbacks
    // =========================================================================

    fn attach_setpoint_callback(param: &mut Parameter<f64>, client: Arc<dyn ScpiOps>) {
        param.connect_to_hardware_write(move |value: f64| {
            let client = client.clone();
            Box::pin(async move {
                client
                    .write(&format!("PID:SETP {}", value))
                    .await
                    .map_err(|e| DaqError::Instrument(e.to_string()))?;
                Ok(())
            })
        });
    }

    fn attach_kp_callback(param: &mut Parameter<f64>, client: Arc<dyn ScpiOps>) {
        param.connect_to_hardware_write(move |value: f64| {
            let client = client.clone();
            Box::pin(async move {
                client
                    .write(&format!("PID:KP {}", value))
                    .await
                    .map_err(|e| DaqError::Instrument(e.to_string()))?;
                Ok(())
            })
        });
    }

    fn attach_ki_callback(param: &mut Parameter<f64>, client: Arc<dyn ScpiOps>) {
        param.connect_to_hardware_write(move |value: f64| {
            let client = client.clone();
            Box::pin(async move {
                client
                    .write(&format!("PID:KI {}", value))
                    .await
                    .map_err(|e| DaqError::Instrument(e.to_string()))?;
                Ok(())
            })
        });
    }

    fn attach_kd_callback(param: &mut Parameter<f64>, client: Arc<dyn ScpiOps>) {
        param.connect_to_hardware_write(move |value: f64| {
            let client = client.clone();
            Box::pin(async move {
                client
                    .write(&format!("PID:KD {}", value))
                    .await
                    .map_err(|e| DaqError::Instrument(e.to_string()))?;
                Ok(())
            })
        });
    }

    fn attach_output_min_callback(param: &mut Parameter<f64>, client: Arc<dyn ScpiOps>) {
        param.connect_to_hardware_write(move |value: f64| {
            let client = client.clone();
            Box::pin(async move {
                client
                    .write(&format!("PID:OMIN {}", value))
                    .await
                    .map_err(|e| DaqError::Instrument(e.to_string()))?;
                Ok(())
            })
        });
    }

    fn attach_output_max_callback(param: &mut Parameter<f64>, client: Arc<dyn ScpiOps>) {
        param.connect_to_hardware_write(move |value: f64| {
            let client = client.clone();
            Box::pin(async move {
                client
                    .write(&format!("PID:OMAX {}", value))
                    .await
                    .map_err(|e| DaqError::Instrument(e.to_string()))?;
                Ok(())
            })
        });
    }

    fn attach_enabled_callback(param: &mut Parameter<bool>, client: Arc<dyn ScpiOps>) {
        param.connect_to_hardware_write(move |value: bool| {
            let client = client.clone();
            Box::pin(async move {
                let cmd = if value { "PID:EN ON" } else { "PID:EN OFF" };
                client
                    .write(cmd)
                    .await
                    .map_err(|e| DaqError::Instrument(e.to_string()))?;
                Ok(())
            })
        });
    }

    // =========================================================================
    // Parameter Accessors
    // =========================================================================

    /// Get power parameter (read-only).
    pub fn power(&self) -> &Parameter<f64> {
        &self.power
    }

    /// Get setpoint parameter.
    pub fn setpoint(&self) -> &Parameter<f64> {
        &self.setpoint
    }

    /// Get error parameter (read-only).
    pub fn error(&self) -> &Parameter<f64> {
        &self.error
    }

    /// Get PID output parameter (read-only).
    pub fn pid_output(&self) -> &Parameter<f64> {
        &self.pid_output
    }

    /// Get Kp parameter.
    pub fn kp(&self) -> &Parameter<f64> {
        &self.kp
    }

    /// Get Ki parameter.
    pub fn ki(&self) -> &Parameter<f64> {
        &self.ki
    }

    /// Get Kd parameter.
    pub fn kd(&self) -> &Parameter<f64> {
        &self.kd
    }

    /// Get output minimum limit parameter.
    pub fn output_min(&self) -> &Parameter<f64> {
        &self.output_min
    }

    /// Get output maximum limit parameter.
    pub fn output_max(&self) -> &Parameter<f64> {
        &self.output_max
    }

    /// Get enabled parameter.
    pub fn enabled(&self) -> &Parameter<bool> {
        &self.enabled
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Parameterized for RedPitayaPidDriver {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl Readable for RedPitayaPidDriver {
    async fn read(&self) -> Result<f64> {
        self.read_power().await
    }
}

// =============================================================================
// RedPitayaPidFactory
// =============================================================================

/// Factory for creating Red Pitaya PID driver instances.
///
/// Register this factory with the DeviceRegistry to enable automatic
/// device creation from TOML configuration.
pub struct RedPitayaPidFactory;

/// Static capabilities for Red Pitaya PID driver
static RED_PITAYA_PID_CAPABILITIES: &[Capability] =
    &[Capability::Readable, Capability::Parameterized];

impl DriverFactory for RedPitayaPidFactory {
    fn driver_type(&self) -> &'static str {
        "red_pitaya_pid"
    }

    fn name(&self) -> &'static str {
        "Red Pitaya PID Controller"
    }

    fn capabilities(&self) -> &'static [Capability] {
        RED_PITAYA_PID_CAPABILITIES
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        let cfg: RedPitayaPidConfig = config
            .clone()
            .try_into()
            .context("Invalid Red Pitaya PID config")?;

        // Validate host is not empty
        if cfg.host.is_empty() {
            anyhow::bail!("'host' field cannot be empty");
        }

        // Validate port range
        if cfg.port == 0 {
            anyhow::bail!("'port' field cannot be 0");
        }

        // Validate initial values if provided
        if let Some(kp) = cfg.initial_kp {
            if kp < 0.0 {
                anyhow::bail!("'initial_kp' cannot be negative");
            }
        }
        if let Some(ki) = cfg.initial_ki {
            if ki < 0.0 {
                anyhow::bail!("'initial_ki' cannot be negative");
            }
        }
        if let Some(kd) = cfg.initial_kd {
            if kd < 0.0 {
                anyhow::bail!("'initial_kd' cannot be negative");
            }
        }

        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let cfg: RedPitayaPidConfig =
                config.try_into().context("Invalid Red Pitaya PID config")?;

            let driver = RedPitayaPidDriver::new_async(&cfg.host, cfg.port, cfg.mock).await?;

            // Apply initial values if specified
            if let Some(setpoint) = cfg.initial_setpoint {
                driver.setpoint().set(setpoint).await?;
            }
            if let Some(kp) = cfg.initial_kp {
                driver.kp().set(kp).await?;
            }
            if let Some(ki) = cfg.initial_ki {
                driver.ki().set(ki).await?;
            }
            if let Some(kd) = cfg.initial_kd {
                driver.kd().set(kd).await?;
            }

            Ok(DeviceComponents {
                readable: Some(driver.clone()),
                parameterized: Some(driver),
                metadata: DeviceMetadata {
                    measurement_units: Some("mW".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            })
        })
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_factory_driver_type() {
        let factory = RedPitayaPidFactory;
        assert_eq!(factory.driver_type(), "red_pitaya_pid");
        assert_eq!(factory.name(), "Red Pitaya PID Controller");
    }

    #[test]
    fn test_factory_capabilities() {
        let factory = RedPitayaPidFactory;
        let caps = factory.capabilities();
        assert!(caps.contains(&Capability::Readable));
        assert!(caps.contains(&Capability::Parameterized));
    }

    #[tokio::test]
    async fn test_factory_validate_config() {
        let factory = RedPitayaPidFactory;

        // Valid config
        let valid_config = toml::Value::Table(toml::toml! {
            host = "192.168.1.100"
        });
        assert!(factory.validate(&valid_config).is_ok());

        // Valid config with all options
        let valid_full = toml::Value::Table(toml::toml! {
            host = "192.168.1.100"
            port = 5000
            mock = true
            initial_setpoint = 1.5
            initial_kp = 2.0
            initial_ki = 0.5
            initial_kd = 0.1
        });
        assert!(factory.validate(&valid_full).is_ok());

        // Empty host
        let empty_host = toml::Value::Table(toml::toml! {
            host = ""
        });
        assert!(factory.validate(&empty_host).is_err());

        // Missing host
        let missing_host = toml::Value::Table(toml::toml! {
            port = 5000
        });
        assert!(factory.validate(&missing_host).is_err());

        // Negative Kp
        let negative_kp = toml::Value::Table(toml::toml! {
            host = "192.168.1.100"
            initial_kp = -1.0
        });
        assert!(factory.validate(&negative_kp).is_err());
    }

    #[tokio::test]
    async fn test_factory_build_mock() {
        let factory = RedPitayaPidFactory;

        let config = toml::Value::Table(toml::toml! {
            host = "192.168.1.100"
            mock = true
        });

        let result = factory.build(config).await;
        assert!(result.is_ok());

        let components = result.unwrap();
        assert!(components.readable.is_some());
        assert!(components.parameterized.is_some());
    }

    #[tokio::test]
    async fn test_driver_read_power() {
        // Use mock mode
        let driver = RedPitayaPidDriver::new_async("localhost", 5000, true)
            .await
            .unwrap();

        let power = driver.read().await.unwrap();
        assert!(power >= 0.0);
    }

    #[tokio::test]
    async fn test_driver_set_parameters() {
        let driver = RedPitayaPidDriver::new_async("localhost", 5000, true)
            .await
            .unwrap();

        // Set setpoint
        driver.setpoint().set(2.5).await.unwrap();
        assert!((driver.setpoint().get() - 2.5).abs() < 0.001);

        // Set PID gains
        driver.kp().set(1.5).await.unwrap();
        assert!((driver.kp().get() - 1.5).abs() < 0.001);

        driver.ki().set(0.25).await.unwrap();
        assert!((driver.ki().get() - 0.25).abs() < 0.001);

        driver.kd().set(0.05).await.unwrap();
        assert!((driver.kd().get() - 0.05).abs() < 0.001);

        // Set output limits
        driver.output_min().set(-0.5).await.unwrap();
        driver.output_max().set(0.8).await.unwrap();

        // Enable PID
        driver.enabled().set(true).await.unwrap();
        assert!(driver.enabled().get());
    }

    #[tokio::test]
    async fn test_driver_parameters_registered() {
        let driver = RedPitayaPidDriver::new_async("localhost", 5000, true)
            .await
            .unwrap();

        let params = driver.parameters();
        let names = params.names();

        assert!(names.contains(&"power"));
        assert!(names.contains(&"setpoint"));
        assert!(names.contains(&"error"));
        assert!(names.contains(&"pid_output"));
        assert!(names.contains(&"kp"));
        assert!(names.contains(&"ki"));
        assert!(names.contains(&"kd"));
        assert!(names.contains(&"output_min"));
        assert!(names.contains(&"output_max"));
        assert!(names.contains(&"enabled"));
    }
}
