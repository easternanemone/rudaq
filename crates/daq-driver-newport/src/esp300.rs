//! Newport ESP300 Multi-Axis Motion Controller Driver
//!
//! Reference: ESP300 Universal Motion Controller/Driver User's Manual
//!
//! Protocol Overview:
//! - Format: ASCII command/response over RS-232
//! - Baud: 19200, 8N1, no flow control
//! - Commands: {Axis}{Command}{Value}
//! - Example: "1PA5.0" (axis 1, position absolute, 5.0mm)
//!
//! # Usage
//!
//! ```rust,ignore
//! use daq_driver_newport::Esp300Factory;
//! use daq_core::driver::DriverFactory;
//!
//! // Register the factory
//! registry.register_factory(Box::new(Esp300Factory));
//!
//! // Create via config
//! let config = toml::toml! {
//!     port = "/dev/ttyUSB0"
//!     axis = 1
//! };
//! let components = factory.build(config.into()).await?;
//! ```

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use daq_core::capabilities::{Movable, Parameterized};
use daq_core::driver::{Capability, DeviceComponents, DriverFactory};
use daq_core::error::DaqError;
use daq_core::observable::ParameterSet;
use daq_core::parameter::Parameter;
use daq_core::serial::{open_serial_async, wrap_shared, SharedPort};
use futures::future::BoxFuture;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tracing::instrument;

// =============================================================================
// Esp300Factory - DriverFactory implementation
// =============================================================================

/// Configuration for ESP300 driver
#[derive(Debug, Clone, Deserialize)]
pub struct Esp300Config {
    /// Serial port path (e.g., "/dev/ttyUSB0")
    pub port: String,
    /// Axis number (1-3)
    pub axis: u8,
    /// Optional custom timeout in seconds (default: 5)
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

/// Factory for creating ESP300 driver instances.
pub struct Esp300Factory;

/// Static capabilities for ESP300
static ESP300_CAPABILITIES: &[Capability] = &[Capability::Movable, Capability::Parameterized];

impl DriverFactory for Esp300Factory {
    fn driver_type(&self) -> &'static str {
        "esp300"
    }

    fn name(&self) -> &'static str {
        "Newport ESP300 Motion Controller"
    }

    fn capabilities(&self) -> &'static [Capability] {
        ESP300_CAPABILITIES
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        let cfg: Esp300Config = config.clone().try_into()?;
        if !(1..=3).contains(&cfg.axis) {
            return Err(anyhow!("ESP300 axis must be 1-3, got {}", cfg.axis));
        }
        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let cfg: Esp300Config = config.try_into().context("Invalid ESP300 config")?;
            let timeout = Duration::from_secs(cfg.timeout_secs.unwrap_or(5));

            // Create driver with validation
            let driver =
                Arc::new(Esp300Driver::new_async_with_timeout(&cfg.port, cfg.axis, timeout).await?);

            Ok(DeviceComponents {
                movable: Some(driver.clone()),
                parameterized: Some(driver),
                ..Default::default()
            })
        })
    }
}

// =============================================================================
// Esp300Driver
// =============================================================================

/// Driver for Newport ESP300 Universal Motion Controller
///
/// Supports up to 3 axes. Each axis is controlled independently via
/// a separate driver instance.
pub struct Esp300Driver {
    /// Serial port protected by Mutex for exclusive access
    port: SharedPort,
    /// Axis number (1-3)
    axis: u8,
    /// Command timeout duration
    timeout: Duration,
    /// Stage position parameter (mm)
    position_mm: Parameter<f64>,
    /// Parameter registry
    params: Arc<ParameterSet>,
}

impl Esp300Driver {
    /// Create a new ESP300 driver instance asynchronously with device validation.
    ///
    /// Uses default timeout of 5 seconds.
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyUSB0", "COM3")
    /// * `axis` - Axis number (1-3)
    ///
    /// # Errors
    /// Returns error if:
    /// - Serial port cannot be opened
    /// - Axis is invalid (not 1-3)
    /// - Device doesn't respond to version query
    pub async fn new_async(port_path: &str, axis: u8) -> Result<Self> {
        Self::new_async_with_timeout(port_path, axis, Duration::from_secs(5)).await
    }

    /// Create a new ESP300 driver instance asynchronously with custom timeout.
    ///
    /// This is the **preferred constructor** for production use.
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyUSB0", "COM3")
    /// * `axis` - Axis number (1-3)
    /// * `timeout` - Command timeout duration
    ///
    /// # Errors
    /// Returns error if:
    /// - Serial port cannot be opened
    /// - Axis is invalid (not 1-3)
    /// - Device doesn't respond to version query
    pub async fn new_async_with_timeout(
        port_path: &str,
        axis: u8,
        timeout: Duration,
    ) -> Result<Self> {
        if !(1..=3).contains(&axis) {
            return Err(anyhow!("ESP300 axis must be 1-3, got {}", axis));
        }

        // Use shared serial port opening utility
        let port = open_serial_async(port_path, 19200, "ESP300").await?;
        let shared = wrap_shared(Box::new(port));

        let driver = Self::build(shared, axis, timeout);

        // Validate device identity by querying version
        match driver.query("VE").await {
            Ok(version) => {
                if !version.to_uppercase().contains("ESP") {
                    return Err(anyhow!(
                        "ESP300 validation failed: version response '{}' doesn't indicate an ESP controller.",
                        version
                    ));
                }
                tracing::info!("ESP300 axis {} validated: {}", axis, version);
            }
            Err(e) => {
                return Err(anyhow!(
                    "ESP300 validation failed: no response to version query (VE). Error: {}",
                    e
                ));
            }
        }

        Ok(driver)
    }

    fn build(port: SharedPort, axis: u8, timeout: Duration) -> Self {
        let mut params = ParameterSet::new();

        let mut position = Parameter::new("position", 0.0)
            .with_description("Stage position")
            .with_unit("mm");

        // Attach hardware write callback
        Self::attach_position_callbacks(&mut position, port.clone(), axis);

        params.register(position.clone());

        Self {
            port,
            axis,
            timeout,
            position_mm: position,
            params: Arc::new(params),
        }
    }

    /// Attach hardware callbacks to position parameter.
    fn attach_position_callbacks(position: &mut Parameter<f64>, port: SharedPort, axis: u8) {
        position.connect_to_hardware_write(move |target: f64| {
            let port = port.clone();
            Box::pin(async move {
                let mut guard = port.lock().await;
                let cmd = format!("{}PA{:.6}\r\n", axis, target);
                let writer = guard.get_mut();
                writer
                    .write_all(cmd.as_bytes())
                    .await
                    .context("ESP300 position write failed")
                    .map_err(|e| DaqError::Instrument(e.to_string()))?;
                writer
                    .flush()
                    .await
                    .context("ESP300 position flush failed")
                    .map_err(|e| DaqError::Instrument(e.to_string()))?;

                tokio::time::sleep(Duration::from_millis(10)).await;
                Ok(())
            })
        });
    }

    #[cfg(test)]
    pub(crate) fn with_test_port(port: SharedPort, axis: u8) -> Self {
        Self::build(port, axis, Duration::from_secs(5))
    }

    /// Get the axis number.
    pub fn axis(&self) -> u8 {
        self.axis
    }

    /// Set velocity for this axis
    #[instrument(skip(self), fields(axis = self.axis, velocity), err)]
    pub async fn set_velocity(&self, velocity: f64) -> Result<()> {
        self.send_command(&format!("{}VA{:.6}", self.axis, velocity))
            .await
    }

    /// Set acceleration for this axis
    #[instrument(skip(self), fields(axis = self.axis, acceleration), err)]
    pub async fn set_acceleration(&self, acceleration: f64) -> Result<()> {
        self.send_command(&format!("{}AC{:.6}", self.axis, acceleration))
            .await
    }

    /// Home this axis (find mechanical zero)
    #[instrument(skip(self), fields(axis = self.axis), err)]
    pub async fn home(&self) -> Result<()> {
        self.send_command(&format!("{}OR", self.axis)).await?;
        self.wait_settled().await
    }

    /// Send query and read response
    async fn query(&self, command: &str) -> Result<String> {
        let mut port = self.port.lock().await;

        let cmd = format!("{}\r\n", command);
        let writer = port.get_mut();
        writer
            .write_all(cmd.as_bytes())
            .await
            .context("ESP300 write failed")?;
        writer.flush().await.context("ESP300 flush failed")?;

        let mut response = String::new();
        tokio::time::timeout(self.timeout, port.read_line(&mut response))
            .await
            .context("ESP300 read timeout")?
            .context("ESP300 read error")?;

        Ok(response.trim().to_string())
    }

    /// Send command without expecting response
    async fn send_command(&self, command: &str) -> Result<()> {
        let mut port = self.port.lock().await;

        let cmd = format!("{}\r\n", command);
        let writer = port.get_mut();
        writer
            .write_all(cmd.as_bytes())
            .await
            .context("ESP300 write failed")?;
        writer.flush().await.context("ESP300 flush failed")?;

        tokio::time::sleep(Duration::from_millis(10)).await;
        Ok(())
    }

    /// Check if axis is in motion
    async fn is_moving(&self) -> Result<bool> {
        let response = self.query(&format!("{}MD?", self.axis)).await?;
        Ok(response.trim() != "0")
    }
}

impl Parameterized for Esp300Driver {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl Movable for Esp300Driver {
    #[instrument(skip(self), fields(axis = self.axis, position), err)]
    async fn move_abs(&self, position: f64) -> Result<()> {
        self.position_mm.set(position).await
    }

    #[instrument(skip(self), fields(axis = self.axis, distance), err)]
    async fn move_rel(&self, distance: f64) -> Result<()> {
        self.send_command(&format!("{}PR{:.6}", self.axis, distance))
            .await
    }

    #[instrument(skip(self), fields(axis = self.axis), err)]
    async fn position(&self) -> Result<f64> {
        let response = self.query(&format!("{}TP?", self.axis)).await?;
        let pos = response
            .trim()
            .parse::<f64>()
            .context("Failed to parse position")?;

        let _ = self.position_mm.inner().set(pos);
        Ok(pos)
    }

    #[instrument(skip(self), fields(axis = self.axis), err)]
    async fn wait_settled(&self) -> Result<()> {
        let timeout = Duration::from_secs(60);
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow!("ESP300 wait_settled timed out after 60 seconds"));
            }

            if !self.is_moving().await? {
                return Ok(());
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[instrument(skip(self), fields(axis = self.axis), err)]
    async fn stop(&self) -> Result<()> {
        self.send_command(&format!("{}ST", self.axis)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[test]
    fn test_factory_driver_type() {
        let factory = Esp300Factory;
        assert_eq!(factory.driver_type(), "esp300");
        assert_eq!(factory.name(), "Newport ESP300 Motion Controller");
    }

    #[test]
    fn test_factory_capabilities() {
        let factory = Esp300Factory;
        let caps = factory.capabilities();
        assert!(caps.contains(&Capability::Movable));
        assert!(caps.contains(&Capability::Parameterized));
    }

    #[tokio::test]
    async fn test_factory_validate_config() {
        let factory = Esp300Factory;

        // Valid config
        let valid_config = toml::Value::Table(toml::toml! {
            port = "/dev/ttyUSB0"
            axis = 1
        });
        assert!(factory.validate(&valid_config).is_ok());

        // Invalid axis
        let invalid_config = toml::Value::Table(toml::toml! {
            port = "/dev/ttyUSB0"
            axis = 4
        });
        assert!(factory.validate(&invalid_config).is_err());

        // Missing port
        let missing_port = toml::Value::Table(toml::toml! {
            axis = 1
        });
        assert!(factory.validate(&missing_port).is_err());
    }

    #[tokio::test]
    async fn move_abs_uses_parameter_and_writes_command() -> Result<()> {
        let (mut host, device) = tokio::io::duplex(64);
        let port: SharedPort = Arc::new(Mutex::new(BufReader::new(Box::new(device))));

        let driver = Esp300Driver::with_test_port(port, 1);

        driver.move_abs(12.5).await?;

        let mut buf = vec![0u8; 64];
        let n = host.read(&mut buf).await?;
        let sent = String::from_utf8_lossy(&buf[..n]);

        assert!(sent.contains("1PA12.500000\r\n"));

        Ok(())
    }
}
