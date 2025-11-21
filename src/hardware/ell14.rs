//! Thorlabs Elliptec ELL14 Rotation Mount Driver
//!
//! Reference: ELLx modules protocol manual Issue 7-6
//!
//! Protocol Overview:
//! - Format: [Address][Command][Data (optional)] (ASCII encoded)
//! - Address: 0-9, A-F (usually '0' for first device)
//! - Encoding: Positions as 32-bit integers in hex
//! - Timing: Half-duplex request-response
//!
//! # Example Usage
//!
//! ```no_run
//! use rust_daq::hardware::ell14::Ell14Driver;
//! use rust_daq::hardware::capabilities::Movable;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let driver = Ell14Driver::new("/dev/ttyUSB0", "0")?;
//!
//!     // Move to 45 degrees
//!     driver.move_abs(45.0).await?;
//!     driver.wait_settled().await?;
//!
//!     // Get current position
//!     let pos = driver.position().await?;
//!     println!("Position: {:.2}°", pos);
//!
//!     Ok(())
//! }
//! ```

use crate::hardware::capabilities::Movable;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serial2_tokio::SerialPort;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

/// Driver for Thorlabs Elliptec ELL14 Rotation Mount
///
/// Implements the Movable capability trait for controlling rotation.
/// The ELL14 has a mechanical resolution in "pulses" that must be converted
/// to/from degrees based on device calibration.
pub struct Ell14Driver {
    /// Serial port protected by Mutex for exclusive access during transactions
    port: Mutex<SerialPort>,
    /// Device address (usually "0")
    address: String,
    /// Calibration factor: Pulses per Degree
    /// Default: 398.22 (143360 pulses / 360 degrees for ELL14)
    pulses_per_degree: f64,
}

impl Ell14Driver {
    /// Create a new ELL14 driver instance
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyUSB0" on Linux, "COM3" on Windows)
    /// * `address` - Device address (usually "0")
    ///
    /// # Errors
    /// Returns error if serial port cannot be opened
    pub fn new(port_path: &str, address: &str) -> Result<Self> {
        // Configure serial settings with no flow control (per Thorlabs ELL14 spec)
        let port = SerialPort::open(port_path, |mut settings: serial2::Settings| {
            settings.set_raw();
            settings.set_baud_rate(9600)?;
            settings.set_flow_control(serial2::FlowControl::None);
            Ok(settings)
        }).context("Failed to open ELL14 serial port")?;

        Ok(Self {
            port: Mutex::new(port),
            address: address.to_string(),
            pulses_per_degree: 398.2222, // 143360 pulses / 360 degrees
        })
    }

    /// Create with custom calibration factor
    ///
    /// # Arguments
    /// * `port_path` - Serial port path
    /// * `address` - Device address
    /// * `pulses_per_degree` - Custom calibration (varies by device)
    pub fn with_calibration(
        port_path: &str,
        address: &str,
        pulses_per_degree: f64,
    ) -> Result<Self> {
        let mut driver = Self::new(port_path, address)?;
        driver.pulses_per_degree = pulses_per_degree;
        Ok(driver)
    }

    /// Send home command to find mechanical zero
    ///
    /// Should be called on initialization to establish reference position
    pub async fn home(&self) -> Result<()> {
        let _ = self.transaction("ho").await?;
        self.wait_settled().await
    }

    /// Helper to send a command and get a response
    ///
    /// ELL14 protocol is ASCII based with format: {Address}{Command}{Data}
    async fn transaction(&self, command: &str) -> Result<String> {
        let mut port = self.port.lock().await;

        // Construct packet: Address + Command
        // Example: "0gs" (Get Status for device 0)
        let payload = format!("{}{}", self.address, command);
        port.write_all(payload.as_bytes())
            .await
            .context("ELL14 write failed")?;

        // Read response with timeout
        // Responses are typically short ASCII strings
        let mut buf = [0u8; 1024];

        let read_len = tokio::time::timeout(Duration::from_millis(500), port.read(&mut buf))
            .await
            .context("ELL14 read timeout")?
            .context("ELL14 read error")?;

        if read_len == 0 {
            return Err(anyhow!("ELL14 returned empty response"));
        }

        let response = std::str::from_utf8(&buf[..read_len])
            .context("Invalid UTF-8 from ELL14")?
            .trim();

        Ok(response.to_string())
    }

    /// Parse position from hex string response
    ///
    /// Format: {Address}{Command}{8-char Hex}
    /// Example response to 'gp': "0PO00002000"
    fn parse_position_response(&self, response: &str) -> Result<f64> {
        if response.len() < 5 {
            return Err(anyhow!("Response too short: {}", response));
        }

        // Look for position response marker "PO"
        if let Some(idx) = response.find("PO") {
            let hex_str = &response[idx + 2..].trim();

            // Handle variable length hex strings
            let hex_clean = if hex_str.len() > 8 {
                &hex_str[..8]
            } else {
                hex_str
            };

            let pulses = i32::from_str_radix(hex_clean, 16)
                .context(format!("Failed to parse position hex: {}", hex_clean))?;

            return Ok(pulses as f64 / self.pulses_per_degree);
        }

        Err(anyhow!("Unexpected position format: {}", response))
    }
}

#[async_trait]
impl Movable for Ell14Driver {
    async fn move_abs(&self, position_deg: f64) -> Result<()> {
        // Convert degrees to pulses
        let pulses = (position_deg * self.pulses_per_degree) as i32;

        // Format as 8-digit hex (uppercase, zero-padded)
        let hex_pulses = format!("{:08X}", pulses);

        // Command: ma (Move Absolute)
        // Format: "0ma00002000" for device 0, position 0x00002000
        let cmd = format!("ma{}", hex_pulses);
        let _ = self.transaction(&cmd).await?;

        Ok(())
    }

    async fn move_rel(&self, distance_deg: f64) -> Result<()> {
        // Command: mr (Move Relative)
        let pulses = (distance_deg * self.pulses_per_degree) as i32;
        let hex_pulses = format!("{:08X}", pulses);

        let cmd = format!("mr{}", hex_pulses);
        let _ = self.transaction(&cmd).await?;

        Ok(())
    }

    async fn position(&self) -> Result<f64> {
        // Command: gp (Get Position)
        let resp = self.transaction("gp").await?;
        self.parse_position_response(&resp)
    }

    async fn wait_settled(&self) -> Result<()> {
        // Poll 'gs' (Get Status) until motion stops
        // Status byte logic from manual:
        // Bit 0: Moving (1=Moving, 0=Stationary)

        let timeout = Duration::from_secs(10);
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow!("ELL14 wait_settled timed out after 10 seconds"));
            }

            let resp = self.transaction("gs").await?;

            // Response format: "0GS{StatusHex}"
            if let Some(idx) = resp.find("GS") {
                let hex_str = &resp[idx + 2..].trim();

                // Handle variable length status
                let hex_clean = if hex_str.len() > 2 {
                    &hex_str[..2]
                } else {
                    hex_str
                };

                let status = u32::from_str_radix(hex_clean, 16)
                    .context(format!("Failed to parse status hex: {}", hex_clean))?;

                // Check "Moving" bit (Bit 0 for ELL14)
                let is_moving = (status & 0x01) != 0;

                if !is_moving {
                    return Ok(());
                }
            }

            // Poll at 50ms intervals
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_position_response() {
        // Create a mock driver for testing parse logic
        let port = SerialPort::open("/dev/null", 9600).unwrap();
        let driver = Ell14Driver {
            port: Mutex::new(port),
            address: "0".to_string(),
            pulses_per_degree: 398.2222,
        };

        // Test typical response
        let response = "0PO00002000";
        let position = driver.parse_position_response(response).unwrap();

        // 0x2000 = 8192 pulses / 398.2222 pulses/deg ≈ 20.57°
        assert!((position - 20.57).abs() < 0.1);
    }

    #[test]
    fn test_position_conversion() {
        // Create a mock driver for testing conversion logic
        let port = SerialPort::open("/dev/null", 9600).unwrap();
        let driver = Ell14Driver {
            port: Mutex::new(port),
            address: "0".to_string(),
            pulses_per_degree: 398.2222,
        };

        // Test 45 degrees
        let pulses = (45.0 * driver.pulses_per_degree) as i32;
        assert_eq!(pulses, 17920); // 398.2222 * 45

        // Test 90 degrees
        let pulses = (90.0 * driver.pulses_per_degree) as i32;
        assert_eq!(pulses, 35840); // 398.2222 * 90
    }
}
