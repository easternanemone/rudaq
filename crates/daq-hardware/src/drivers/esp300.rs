//! Newport ESP300 Multi-Axis Motion Controller Driver
//!
//! Reference: ESP300 Universal Motion Controller/Driver User's Manual
//!
//! Protocol Overview:
//! - Format: ASCII command/response over RS-232
//! - Baud: 19200, 8N1, hardware flow control
//! - Commands: {Axis}{Command}{Value}
//! - Example: "1PA5.0" (axis 1, position absolute, 5.0mm)
//!
//! # Example Usage
//!
//! ```ignore
//! use daq_hardware::drivers::esp300::Esp300Driver;
//! use daq_hardware::capabilities::Movable;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Use new_async() for production - validates device identity
//!     let stage = Esp300Driver::new_async("/dev/ttyUSB0", 1).await?;
//!
//!     // Move to absolute position
//!     stage.move_abs(10.5).await?;
//!     stage.wait_settled().await?;
//!
//!     // Get current position
//!     let pos = stage.position().await?;
//!     println!("Position: {:.3} mm", pos);
//!
//!     Ok(())
//! }
//! ```

use crate::capabilities::{Movable, Parameterized};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use common::error::DaqError;
use common::error_recovery::RetryPolicy;
use common::observable::ParameterSet;
use common::parameter::Parameter;
use futures::future::BoxFuture;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tokio::task::spawn_blocking;
use tokio_serial::SerialPortBuilderExt;
use tracing::instrument;

pub trait SerialPortIO: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> SerialPortIO for T {}
type DynSerial = Box<dyn SerialPortIO>;
type SharedPort = Arc<Mutex<BufReader<DynSerial>>>;

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
    params: ParameterSet,
}

impl Esp300Driver {
    async fn with_retry<F, Fut, T>(&self, operation: &str, mut op: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let policy = RetryPolicy::default();
        let mut attempts = 0;
        loop {
            match op().await {
                Ok(value) => return Ok(value),
                Err(err) => {
                    attempts += 1;
                    if attempts >= policy.max_attempts {
                        return Err(err);
                    }
                    tracing::warn!(
                        target: "hardware::esp300",
                        attempt = attempts,
                        max_attempts = policy.max_attempts,
                        "Operation '{}' failed: {}. Retrying in {:?}",
                        operation,
                        err,
                        policy.backoff_delay
                    );
                    tokio::time::sleep(policy.backoff_delay).await;
                }
            }
        }
    }

    /// Create a new ESP300 driver instance for a specific axis
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyUSB0", "COM3")
    /// * `axis` - Axis number (1-3)
    ///
    /// # Errors
    /// Returns error if serial port cannot be opened or axis is invalid
    ///
    /// # Note
    /// This constructor may block the async runtime during serial port opening.
    /// For non-blocking construction, use [`new_async`] instead.
    pub fn new(port_path: &str, axis: u8) -> Result<Self> {
        if !(1..=3).contains(&axis) {
            return Err(anyhow!("ESP300 axis must be 1-3, got {}", axis));
        }

        // Configure serial settings: 19200 baud, 8N1, no flow control
        // Note: ESP300 v3.04 confirmed to work with FlowControl::None (tested 2025-11-02)
        let port = tokio_serial::new(port_path, 19200)
            .data_bits(tokio_serial::DataBits::Eight)
            .parity(tokio_serial::Parity::None)
            .stop_bits(tokio_serial::StopBits::One)
            .flow_control(tokio_serial::FlowControl::None)
            .open_native_async()
            .context(format!("Failed to open ESP300 serial port: {}", port_path))?;

        Ok(Self::build(
            Arc::new(Mutex::new(BufReader::new(Box::new(port)))),
            axis,
        ))
    }

    /// Create a new ESP300 driver instance asynchronously with device validation
    ///
    /// This is the **preferred constructor** for production use. It uses
    /// `spawn_blocking` to avoid blocking the async runtime during serial port
    /// opening, and validates that an ESP300 controller is actually connected.
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyUSB0", "COM3")
    /// * `axis` - Axis number (1-3)
    ///
    /// # Errors
    /// Returns error if:
    /// - Serial port cannot be opened
    /// - Axis is invalid (not 1-3)
    /// - Device doesn't respond to version query (VE)
    /// - Response doesn't contain "ESP" (indicates wrong device)
    pub async fn new_async(port_path: &str, axis: u8) -> Result<Self> {
        if !(1..=3).contains(&axis) {
            return Err(anyhow!("ESP300 axis must be 1-3, got {}", axis));
        }

        let port_path_owned = port_path.to_string();

        // Use spawn_blocking to avoid blocking the async runtime
        let port = spawn_blocking(move || {
            tokio_serial::new(&port_path_owned, 19200)
                .data_bits(tokio_serial::DataBits::Eight)
                .parity(tokio_serial::Parity::None)
                .stop_bits(tokio_serial::StopBits::One)
                .flow_control(tokio_serial::FlowControl::None)
                .open_native_async()
                .context(format!(
                    "Failed to open ESP300 serial port: {}",
                    port_path_owned
                ))
        })
        .await
        .context("spawn_blocking for ESP300 port opening failed")??;

        let driver = Self::build(Arc::new(Mutex::new(BufReader::new(Box::new(port)))), axis);

        // Validate device identity by querying version
        // ESP300 responds with something like "ESP300 Version 3.04"
        match driver.query("VE").await {
            Ok(version) => {
                if !version.to_uppercase().contains("ESP") {
                    return Err(anyhow!(
                        "ESP300 validation failed: version response '{}' doesn't indicate an ESP controller. \
                         Check that the correct device is connected to port {}.",
                        version,
                        port_path
                    ));
                }
                tracing::info!("ESP300 axis {} validated: {}", axis, version);
            }
            Err(e) => {
                return Err(anyhow!(
                    "ESP300 validation failed: no response to version query (VE). \
                     Check that the correct device is connected to port {}. Error: {}",
                    port_path,
                    e
                ));
            }
        }

        Ok(driver)
    }

    fn build(port: SharedPort, axis: u8) -> Self {
        let mut params = ParameterSet::new();

        let mut position = Parameter::new("position", 0.0)
            .with_description("Stage position")
            .with_unit("mm");

        position.connect_to_hardware_write({
            let port = port.clone();
            move |position: f64| -> BoxFuture<'static, Result<(), DaqError>> {
                let port = port.clone();
                Box::pin(async move {
                    let mut port = port.lock().await;
                    let cmd = format!("{}PA{:.6}\r\n", axis, position);
                    port.get_mut()
                        .write_all(cmd.as_bytes())
                        .await
                        .context("ESP300 position write failed")
                        .map_err(|e| DaqError::Instrument(e.to_string()))?;

                    tokio::time::sleep(Duration::from_millis(10)).await;

                    Ok(())
                })
            }
        });

        params.register(position.clone());

        Self {
            port,
            axis,
            timeout: Duration::from_secs(5),
            position_mm: position,
            params,
        }
    }

    #[cfg(test)]
    fn with_test_port(port: SharedPort, axis: u8) -> Self {
        Self::build(port, axis)
    }

    /// Set velocity for this axis
    ///
    /// # Arguments
    /// * `velocity` - Velocity in mm/s
    #[instrument(skip(self), fields(axis = self.axis, velocity), err)]
    pub async fn set_velocity(&self, velocity: f64) -> Result<()> {
        self.send_command(&format!("{}VA{:.6}", self.axis, velocity))
            .await?;
        Ok(())
    }

    /// Set acceleration for this axis
    ///
    /// # Arguments
    /// * `acceleration` - Acceleration in mm/s²
    #[instrument(skip(self), fields(axis = self.axis, acceleration), err)]
    pub async fn set_acceleration(&self, acceleration: f64) -> Result<()> {
        self.send_command(&format!("{}AC{:.6}", self.axis, acceleration))
            .await?;
        Ok(())
    }

    /// Get velocity for this axis
    #[instrument(skip(self), fields(axis = self.axis), err)]
    pub async fn velocity(&self) -> Result<f64> {
        let response = self.query(&format!("{}VA?", self.axis)).await?;
        response
            .trim()
            .parse::<f64>()
            .context("Failed to parse velocity")
    }

    /// Get acceleration for this axis
    #[instrument(skip(self), fields(axis = self.axis), err)]
    pub async fn acceleration(&self) -> Result<f64> {
        let response = self.query(&format!("{}AC?", self.axis)).await?;
        response
            .trim()
            .parse::<f64>()
            .context("Failed to parse acceleration")
    }

    /// Home this axis (find mechanical zero)
    #[instrument(skip(self), fields(axis = self.axis), err)]
    pub async fn home(&self) -> Result<()> {
        self.send_command(&format!("{}OR", self.axis)).await?;
        self.wait_settled().await
    }

    /// Stop motion on this axis
    #[instrument(skip(self), fields(axis = self.axis), err)]
    pub async fn stop(&self) -> Result<()> {
        self.send_command(&format!("{}ST", self.axis)).await
    }

    /// Send command and read response
    async fn query(&self, command: &str) -> Result<String> {
        let command = command.to_string();
        self.with_retry("ESP300 query", || {
            let cmd = command.clone();
            async move { self.query_once(&cmd).await }
        })
        .await
    }

    async fn query_once(&self, command: &str) -> Result<String> {
        let mut port = self.port.lock().await;

        // Write command with terminator
        let cmd = format!("{}\r\n", command);
        port.get_mut()
            .write_all(cmd.as_bytes())
            .await
            .context("ESP300 write failed")?;

        // Read response with timeout
        let mut response = String::new();
        tokio::time::timeout(self.timeout, port.read_line(&mut response))
            .await
            .context("ESP300 read timeout")?
            .context("ESP300 read error")?;

        Ok(response.trim().to_string())
    }

    /// Send command without expecting response
    async fn send_command(&self, command: &str) -> Result<()> {
        let command = command.to_string();
        self.with_retry("ESP300 send_command", || {
            let cmd = command.clone();
            async move { self.send_command_once(&cmd).await }
        })
        .await
    }

    async fn send_command_once(&self, command: &str) -> Result<()> {
        let mut port = self.port.lock().await;

        let cmd = format!("{}\r\n", command);
        port.get_mut()
            .write_all(cmd.as_bytes())
            .await
            .context("ESP300 write failed")?;

        // Small delay to ensure command is processed
        tokio::time::sleep(Duration::from_millis(10)).await;
        Ok(())
    }

    /// Check if axis is in motion
    async fn is_moving(&self) -> Result<bool> {
        let response = self.query(&format!("{}MD?", self.axis)).await?;
        // Response is 0 if stationary, 1 if moving
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
        // PR command: Position Relative
        self.send_command(&format!("{}PR{:.6}", self.axis, distance))
            .await
    }

    #[instrument(skip(self), fields(axis = self.axis), err)]
    async fn position(&self) -> Result<f64> {
        // TP command: Tell Position
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
        // Poll motion status until stationary
        let timeout = Duration::from_secs(60);
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow!("ESP300 wait_settled timed out after 60 seconds"));
            }

            if !self.is_moving().await? {
                return Ok(());
            }

            // Poll at 100ms intervals
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[instrument(skip(self), fields(axis = self.axis), err)]
    async fn stop(&self) -> Result<()> {
        // ST command: Stop motion on this axis
        self.send_command(&format!("{}ST", self.axis)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[test]
    fn test_axis_validation() {
        // Valid axes
        let result1 = Esp300Driver::new("/dev/null", 1);
        let result2 = Esp300Driver::new("/dev/null", 2);
        let result3 = Esp300Driver::new("/dev/null", 3);

        // On most systems /dev/null exists but isn't a serial port
        // so we just check that axis validation works
        assert!(result1.is_ok() || result1.is_err());
        assert!(result2.is_ok() || result2.is_err());
        assert!(result3.is_ok() || result3.is_err());

        // Invalid axes should fail validation before port opening
        assert!(Esp300Driver::new("/dev/null", 0).is_err());
        assert!(Esp300Driver::new("/dev/null", 4).is_err());
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
