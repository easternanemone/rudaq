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
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio_serial::{SerialPortBuilderExt, SerialStream};

/// Driver for Thorlabs Elliptec ELL14 Rotation Mount
///
/// Implements the Movable capability trait for controlling rotation.
/// The ELL14 has a mechanical resolution in "pulses" that must be converted
/// to/from degrees based on device calibration.
pub struct Ell14Driver {
    /// Serial port protected by Mutex for exclusive access during transactions
    port: Mutex<SerialStream>,
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
        let port = tokio_serial::new(port_path, 9600)
            .data_bits(tokio_serial::DataBits::Eight)
            .parity(tokio_serial::Parity::None)
            .stop_bits(tokio_serial::StopBits::One)
            .flow_control(tokio_serial::FlowControl::None)
            .open_native_async()
            .context(format!("Failed to open ELL14 serial port: {}", port_path))?;

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
        // Home command doesn't return immediate response - just starts homing
        self.send_command("ho").await?;
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

        // Small delay for device to process command and start responding
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Read response with buffering - responses may arrive in chunks on shared RS-485 bus
        let mut response_buf = Vec::with_capacity(64);
        let mut buf = [0u8; 64];
        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }

            match tokio::time::timeout(remaining.min(Duration::from_millis(100)), port.read(&mut buf)).await {
                Ok(Ok(n)) if n > 0 => {
                    response_buf.extend_from_slice(&buf[..n]);
                    // Check if we have a complete response (responses end after data)
                    // Give a tiny delay for any remaining bytes
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Ok(Ok(_)) => {
                    // Zero bytes read - check if we have data already
                    if !response_buf.is_empty() {
                        break;
                    }
                }
                Ok(Err(_)) | Err(_) => {
                    // Read error or timeout - use what we have
                    if !response_buf.is_empty() {
                        break;
                    }
                }
            }

            // If we've collected some data, check if more is coming
            if !response_buf.is_empty() {
                // Brief pause then try one more read
                tokio::time::sleep(Duration::from_millis(30)).await;
                if let Ok(Ok(n)) = tokio::time::timeout(Duration::from_millis(50), port.read(&mut buf)).await {
                    if n > 0 {
                        response_buf.extend_from_slice(&buf[..n]);
                    }
                }
                break;
            }
        }

        if response_buf.is_empty() {
            return Err(anyhow!("ELL14 returned empty response"));
        }

        let response = std::str::from_utf8(&response_buf)
            .context("Invalid UTF-8 from ELL14")?
            .trim();

        Ok(response.to_string())
    }

    /// Send command without waiting for response (for move commands)
    ///
    /// Movement commands may not return a response until motion completes.
    /// Use wait_settled() to wait for motion completion.
    async fn send_command(&self, command: &str) -> Result<()> {
        let mut port = self.port.lock().await;

        let payload = format!("{}{}", self.address, command);
        port.write_all(payload.as_bytes())
            .await
            .context("ELL14 write failed")?;

        // Brief delay to let command be processed
        tokio::time::sleep(Duration::from_millis(50)).await;

        Ok(())
    }

    /// Parse position from hex string response
    ///
    /// Format: {Address}PO{Hex}
    /// Example responses: "0PO00002000", "2POF" (short hex), or "3PO" (position 0)
    fn parse_position_response(&self, response: &str) -> Result<f64> {
        // Minimum response: "XPO" = 3 chars (addr + PO, hex portion may be empty for position 0)
        if response.len() < 3 {
            return Err(anyhow!("Response too short: {}", response));
        }

        // Look for position response marker "PO"
        if let Some(idx) = response.find("PO") {
            let hex_str = response[idx + 2..].trim();

            // Empty hex string means position 0
            if hex_str.is_empty() {
                return Ok(0.0);
            }

            // Handle variable length hex strings (take first 8 chars max)
            let hex_clean = if hex_str.len() > 8 {
                &hex_str[..8]
            } else {
                hex_str
            };

            // Parse as u32 first, then reinterpret as i32 for signed positions
            // (ELL14 returns positions as 32-bit two's complement hex)
            let pulses_unsigned = u32::from_str_radix(hex_clean, 16)
                .context(format!("Failed to parse position hex: {}", hex_clean))?;
            let pulses = pulses_unsigned as i32;

            return Ok(pulses as f64 / self.pulses_per_degree);
        }

        Err(anyhow!("Unexpected position format: {}", response))
    }
}

#[async_trait]
impl Movable for Ell14Driver {
    async fn move_abs(&self, position_deg: f64) -> Result<()> {
        // Convert degrees to pulses (handle negative positions properly)
        let pulses = (position_deg * self.pulses_per_degree) as i32;

        // Format as 8-digit hex (uppercase, zero-padded)
        // For negative values, the u32 cast gives two's complement representation
        let hex_pulses = format!("{:08X}", pulses as u32);

        // Command: ma (Move Absolute)
        // Format: "0ma00002000" for device 0, position 0x00002000
        let cmd = format!("ma{}", hex_pulses);

        // Move commands may return a response after motion starts or completes
        // We try to get a response but don't fail if none comes - use wait_settled for confirmation
        let _ = self.send_command(&cmd).await;

        Ok(())
    }

    async fn move_rel(&self, distance_deg: f64) -> Result<()> {
        // Command: mr (Move Relative)
        let pulses = (distance_deg * self.pulses_per_degree) as i32;
        let hex_pulses = format!("{:08X}", pulses as u32);

        let cmd = format!("mr{}", hex_pulses);
        let _ = self.send_command(&cmd).await;

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
        let mut consecutive_settled = 0;

        loop {
            if start.elapsed() > timeout {
                return Err(anyhow!("ELL14 wait_settled timed out after 10 seconds"));
            }

            // Try to get status - device may not respond during movement
            match self.transaction("gs").await {
                Ok(resp) => {
                    // Response format: "0GS{StatusHex}"
                    if let Some(idx) = resp.find("GS") {
                        let hex_str = resp[idx + 2..].trim();

                        // Handle variable length status (could be "0", "00", etc.)
                        if hex_str.is_empty() {
                            // Empty status means stationary
                            consecutive_settled += 1;
                        } else {
                            let hex_clean = if hex_str.len() > 2 {
                                &hex_str[..2]
                            } else {
                                hex_str
                            };

                            if let Ok(status) = u32::from_str_radix(hex_clean, 16) {
                                // Check "Moving" bit (Bit 0 for ELL14)
                                let is_moving = (status & 0x01) != 0;
                                if !is_moving {
                                    consecutive_settled += 1;
                                } else {
                                    consecutive_settled = 0;
                                }
                            }
                        }

                        // Require 2 consecutive "not moving" status to confirm settled
                        if consecutive_settled >= 2 {
                            // Extra delay to let any pending responses clear
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            // Drain any remaining data from the buffer
                            let mut port = self.port.lock().await;
                            let mut drain_buf = [0u8; 256];
                            let _ = tokio::time::timeout(
                                Duration::from_millis(50),
                                port.read(&mut drain_buf),
                            )
                            .await;
                            return Ok(());
                        }
                    }
                }
                Err(_) => {
                    // Device busy, likely still moving - reset counter and retry
                    consecutive_settled = 0;
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
        let port = tokio_serial::new("/dev/null", 9600)
            .open_native_async()
            .unwrap();
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
        let port = tokio_serial::new("/dev/null", 9600)
            .open_native_async()
            .unwrap();
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
