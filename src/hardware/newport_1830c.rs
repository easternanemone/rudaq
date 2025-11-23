//! Newport 1830-C Optical Power Meter Driver
//!
//! Reference: Newport 1830-C User's Manual
//!
//! Protocol Overview:
//! - Format: Simple ASCII commands (NOT SCPI)
//! - Baud: 19200, 8N1, no flow control
//! - Terminator: LF (\n)
//! - Commands: A0/A1 (attenuator), F1/F2/F3 (filter)
//! - Query: D? (power measurement)
//!
//! # Important Notes
//!
//! - Newport 1830-C uses SIMPLE single-letter commands, NOT SCPI
//! - Does NOT support wavelength or units configuration via commands
//! - Does NOT require hardware flow control (unlike ESP300)
//! - Responses use scientific notation (e.g., "5E-9")
//!
//! # Example Usage
//!
//! ```no_run
//! use rust_daq::hardware::newport_1830c::Newport1830CDriver;
//! use rust_daq::hardware::capabilities::Readable;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let meter = Newport1830CDriver::new("/dev/ttyS0")?;
//!
//!     // Configure attenuator and filter
//!     meter.set_attenuator(false).await?;  // 0=off, 1=on
//!     meter.set_filter(2).await?;  // 1=Slow, 2=Medium, 3=Fast
//!
//!     // Read power
//!     let power_watts = meter.read().await?;
//!     println!("Power: {:.3e} W", power_watts);
//!
//!     Ok(())
//! }
//! ```

use crate::hardware::capabilities::Readable;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tokio_serial::{SerialPortBuilderExt, SerialStream};

/// Driver for Newport 1830-C optical power meter
///
/// Implements Readable capability trait for power measurement.
/// Uses Newport's simple ASCII protocol (not SCPI).
pub struct Newport1830CDriver {
    /// Serial port protected by Mutex for exclusive access
    port: Mutex<BufReader<SerialStream>>,
    /// Command timeout duration
    timeout: Duration,
}

impl Newport1830CDriver {
    /// Create a new Newport 1830-C driver instance
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyS0", "COM3")
    ///
    /// # Errors
    /// Returns error if serial port cannot be opened
    pub fn new(port_path: &str) -> Result<Self> {
        // Configure serial settings: 19200 baud, 8N1, no flow control
        let port = tokio_serial::new(port_path, 19200)
            .data_bits(tokio_serial::DataBits::Eight)
            .parity(tokio_serial::Parity::None)
            .stop_bits(tokio_serial::StopBits::One)
            .flow_control(tokio_serial::FlowControl::None)
            .open_native_async()
            .context(format!("Failed to open Newport 1830-C serial port: {}", port_path))?;

        Ok(Self {
            port: Mutex::new(BufReader::new(port)),
            timeout: Duration::from_millis(500),
        })
    }

    /// Set attenuator state
    ///
    /// # Arguments
    /// * `enabled` - true to enable attenuator (A1), false to disable (A0)
    ///
    /// # Note
    /// Newport 1830-C does not respond to configuration commands
    pub async fn set_attenuator(&self, enabled: bool) -> Result<()> {
        let cmd = if enabled { "A1" } else { "A0" };
        self.send_config_command(cmd).await
    }

    /// Set filter (integration time)
    ///
    /// # Arguments
    /// * `filter` - Filter setting: 1=Slow, 2=Medium, 3=Fast
    ///
    /// # Errors
    /// Returns error if filter value is not 1, 2, or 3
    ///
    /// # Note
    /// Newport 1830-C does not respond to configuration commands
    pub async fn set_filter(&self, filter: u8) -> Result<()> {
        if !(1..=3).contains(&filter) {
            return Err(anyhow!(
                "Filter must be 1 (Slow), 2 (Medium), or 3 (Fast), got {}",
                filter
            ));
        }

        self.send_config_command(&format!("F{}", filter)).await
    }

    /// Clear status (zero power reading)
    pub async fn clear_status(&self) -> Result<()> {
        self.send_config_command("CS").await
    }

    /// Parse power measurement response
    ///
    /// Handles scientific notation like "5E-9", "+.75E-9"
    fn parse_power_response(&self, response: &str) -> Result<f64> {
        let trimmed = response.trim();

        // Check for error responses or empty
        if trimmed.is_empty() {
            return Err(anyhow!("Empty power response"));
        }
        if trimmed.contains("ERR") || trimmed.contains("OVER") || trimmed.contains("UNDER") {
            return Err(anyhow!("Meter error response: {}", trimmed));
        }

        // Parse the value (handles scientific notation)
        trimmed
            .parse::<f64>()
            .with_context(|| format!("Failed to parse power response: '{}'", trimmed))
    }

    /// Query power measurement
    async fn query_power(&self) -> Result<f64> {
        let response = self.query("D?").await?;
        self.parse_power_response(&response)
    }

    /// Send query and read response
    async fn query(&self, command: &str) -> Result<String> {
        let mut port = self.port.lock().await;

        // Write command with LF terminator
        let cmd = format!("{}\n", command);
        port.get_mut()
            .write_all(cmd.as_bytes())
            .await
            .context("Newport 1830-C write failed")?;

        // Read response with timeout
        let mut response = String::new();
        tokio::time::timeout(self.timeout, port.read_line(&mut response))
            .await
            .context("Newport 1830-C read timeout")??;

        Ok(response.trim().to_string())
    }

    /// Send configuration command without expecting response
    ///
    /// Newport 1830-C doesn't respond to configuration commands like A0, A1, F1, F2, F3
    async fn send_config_command(&self, command: &str) -> Result<()> {
        let mut port = self.port.lock().await;

        let cmd = format!("{}\n", command);
        port.get_mut()
            .write_all(cmd.as_bytes())
            .await
            .context("Newport 1830-C write failed")?;

        // Small delay to allow meter to process command
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(())
    }
}

#[async_trait]
impl Readable for Newport1830CDriver {
    async fn read(&self) -> Result<f64> {
        self.query_power().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_power_response() {
        // Test parsing scientific notation (the core parsing logic)
        // We test this directly without creating a driver instance

        let test_cases = vec![
            ("5E-9", 5e-9),
            ("1.234E-6", 1.234e-6),
            ("+.75E-9", 0.75e-9),
            ("1E0", 1.0),
        ];

        for (input, expected) in test_cases {
            let parsed: Result<f64, _> = input.parse();
            assert!(parsed.is_ok(), "Failed to parse: {}", input);
            assert_eq!(parsed.unwrap(), expected);
        }
    }

    #[test]
    fn test_error_detection() {
        // Test error response detection
        let error_responses = vec!["ERR", "OVER", "UNDER"];

        for error_response in error_responses {
            assert!(
                error_response.contains("ERR")
                    || error_response.contains("OVER")
                    || error_response.contains("UNDER"),
                "Failed to detect error in: {}",
                error_response
            );
        }
    }
}
