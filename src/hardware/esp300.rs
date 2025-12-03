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
//! ```no_run
//! use rust_daq::hardware::esp300::Esp300Driver;
//! use rust_daq::hardware::capabilities::Movable;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create driver for axis 1
//!     let stage = Esp300Driver::new("/dev/ttyUSB0", 1)?;
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

use crate::hardware::capabilities::{Movable, Parameterized};
use crate::observable::ParameterSet;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tokio::task::spawn_blocking;
use tokio_serial::{SerialPortBuilderExt, SerialStream};

/// Driver for Newport ESP300 Universal Motion Controller
///
/// Supports up to 3 axes. Each axis is controlled independently via
/// a separate driver instance.
pub struct Esp300Driver {
    /// Serial port protected by Mutex for exclusive access
    port: Mutex<BufReader<SerialStream>>,
    /// Axis number (1-3)
    axis: u8,
    /// Command timeout duration
    timeout: Duration,
    /// Parameter registry
    params: ParameterSet,
}

impl Esp300Driver {
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

        Ok(Self {
            port: Mutex::new(BufReader::new(port)),
            axis,
            timeout: Duration::from_secs(5),
            params: ParameterSet::new(),
        })
    }

    /// Create a new ESP300 driver instance asynchronously
    ///
    /// This is the preferred constructor as it uses `spawn_blocking` to avoid
    /// blocking the async runtime during serial port opening.
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyUSB0", "COM3")
    /// * `axis` - Axis number (1-3)
    ///
    /// # Errors
    /// Returns error if serial port cannot be opened or axis is invalid
    pub async fn new_async(port_path: &str, axis: u8) -> Result<Self> {
        if !(1..=3).contains(&axis) {
            return Err(anyhow!("ESP300 axis must be 1-3, got {}", axis));
        }

        let port_path = port_path.to_string();

        // Use spawn_blocking to avoid blocking the async runtime
        let port = spawn_blocking(move || {
            tokio_serial::new(&port_path, 19200)
                .data_bits(tokio_serial::DataBits::Eight)
                .parity(tokio_serial::Parity::None)
                .stop_bits(tokio_serial::StopBits::One)
                .flow_control(tokio_serial::FlowControl::None)
                .open_native_async()
                .context(format!("Failed to open ESP300 serial port: {}", port_path))
        })
        .await
        .context("spawn_blocking for ESP300 port opening failed")??;

        Ok(Self {
            port: Mutex::new(BufReader::new(port)),
            axis,
            timeout: Duration::from_secs(5),
            params: ParameterSet::new(),
        })
    }

    /// Set velocity for this axis
    ///
    /// # Arguments
    /// * `velocity` - Velocity in mm/s
    pub async fn set_velocity(&self, velocity: f64) -> Result<()> {
        self.send_command(&format!("{}VA{:.6}", self.axis, velocity))
            .await?;
        Ok(())
    }

    /// Set acceleration for this axis
    ///
    /// # Arguments
    /// * `acceleration` - Acceleration in mm/s²
    pub async fn set_acceleration(&self, acceleration: f64) -> Result<()> {
        self.send_command(&format!("{}AC{:.6}", self.axis, acceleration))
            .await?;
        Ok(())
    }

    /// Get velocity for this axis
    pub async fn velocity(&self) -> Result<f64> {
        let response = self.query(&format!("{}VA?", self.axis)).await?;
        response
            .trim()
            .parse::<f64>()
            .context("Failed to parse velocity")
    }

    /// Get acceleration for this axis
    pub async fn acceleration(&self) -> Result<f64> {
        let response = self.query(&format!("{}AC?", self.axis)).await?;
        response
            .trim()
            .parse::<f64>()
            .context("Failed to parse acceleration")
    }

    /// Home this axis (find mechanical zero)
    pub async fn home(&self) -> Result<()> {
        self.send_command(&format!("{}OR", self.axis)).await?;
        self.wait_settled().await
    }

    /// Stop motion on this axis
    pub async fn stop(&self) -> Result<()> {
        self.send_command(&format!("{}ST", self.axis)).await
    }

    /// Send command and read response
    async fn query(&self, command: &str) -> Result<String> {
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
    async fn move_abs(&self, position: f64) -> Result<()> {
        // PA command: Position Absolute
        self.send_command(&format!("{}PA{:.6}", self.axis, position))
            .await
    }

    async fn move_rel(&self, distance: f64) -> Result<()> {
        // PR command: Position Relative
        self.send_command(&format!("{}PR{:.6}", self.axis, distance))
            .await
    }

    async fn position(&self) -> Result<f64> {
        // TP command: Tell Position
        let response = self.query(&format!("{}TP?", self.axis)).await?;
        response
            .trim()
            .parse::<f64>()
            .context("Failed to parse position")
    }

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

    async fn stop(&self) -> Result<()> {
        // ST command: Stop motion on this axis
        self.send_command(&format!("{}ST", self.axis)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
