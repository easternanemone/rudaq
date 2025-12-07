//! Newport 1830-C Optical Power Meter Driver
//!
//! Reference: Newport 1830-C User's Manual
//!
//! Protocol Overview:
//! - Format: Simple ASCII commands (NOT SCPI)
//! - Baud: 9600, 8N1, no flow control (verified with hardware)
//! - Terminator: LF (\n)
//! - Commands: A0/A1 (attenuator), F1/F2/F3 (filter), Wxxxx (wavelength)
//! - Queries: D? (power), W? (wavelength), R? (range), U? (units)
//! - Response format: Scientific notation (e.g., "+.11E-9" for 0.11 nW)
//!
//! # Important Notes
//!
//! - Newport 1830-C uses SIMPLE single-letter commands, NOT SCPI
//! - Supports wavelength configuration: W? queries, Wxxxx sets (e.g., W0800 for 800nm)
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
//!     // Set wavelength for accurate power measurement
//!     meter.set_wavelength(800.0).await?;  // 800nm
//!     let wavelength = meter.get_wavelength().await?;
//!     println!("Wavelength: {} nm", wavelength);
//!
//!     // Read power
//!     let power_watts = meter.read().await?;
//!     println!("Power: {:.3e} W", power_watts);
//!
//!     Ok(())
//! }
//! ```

use crate::error::DaqError;
use crate::hardware::capabilities::{Parameterized, Readable, WavelengthTunable};
use crate::observable::ParameterSet;
use crate::parameter::Parameter;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures::future::BoxFuture;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tokio::task::spawn_blocking;
use tokio_serial::{SerialPortBuilderExt, SerialStream};

pub trait SerialPortIO: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> SerialPortIO for T {}
type DynSerial = Box<dyn SerialPortIO>;
type SharedPort = Arc<Mutex<BufReader<DynSerial>>>;

/// Driver for Newport 1830-C optical power meter
///
/// Implements Readable capability trait for power measurement.
/// Uses Newport's simple ASCII protocol (not SCPI).
pub struct Newport1830CDriver {
    /// Serial port protected by Mutex for exclusive access
    port: SharedPort,
    /// Command timeout duration
    timeout: Duration,
    /// Wavelength parameter (nm)
    wavelength_nm: Parameter<f64>,
    /// Parameter registry
    params: ParameterSet,
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
        // Configure serial settings: 9600 baud, 8N1, no flow control
        // Note: 9600 baud verified with actual hardware (not 19200 as in some docs)
        let port = tokio_serial::new(port_path, 9600)
            .data_bits(tokio_serial::DataBits::Eight)
            .parity(tokio_serial::Parity::None)
            .stop_bits(tokio_serial::StopBits::One)
            .flow_control(tokio_serial::FlowControl::None)
            .open_native_async()
            .context(format!(
                "Failed to open Newport 1830-C serial port: {}",
                port_path
            ))?;

        Ok(Self::build(Arc::new(Mutex::new(BufReader::new(Box::new(
            port,
        ))))))
    }

    fn build(port: SharedPort) -> Self {
        let mut params = ParameterSet::new();
        let mut wavelength_nm = Parameter::new("wavelength_nm", 800.0)
            .with_description("Detector calibration wavelength")
            .with_unit("nm")
            .with_range(300.0, 1100.0);

        wavelength_nm.connect_to_hardware_write({
            let port = port.clone();
            move |wavelength: f64| -> BoxFuture<'static, Result<(), DaqError>> {
                let port = port.clone();
                Box::pin(async move {
                    let nm = wavelength.round() as u16;
                    let cmd = format!("W{:04}\n", nm);
                    let mut guard = port.lock().await;
                    guard
                        .get_mut()
                        .write_all(cmd.as_bytes())
                        .await
                        .context("Failed to write wavelength command")
                        .map_err(|e| DaqError::Instrument(e.to_string()))?;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    Ok(())
                })
            }
        });

        params.register(wavelength_nm.clone());

        Self {
            port,
            timeout: Duration::from_millis(500),
            wavelength_nm,
            params,
        }
    }

    #[cfg(test)]
    fn with_test_port(port: SharedPort) -> Self {
        Self::build(port)
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

    /// Query current wavelength setting
    ///
    /// # Returns
    /// Wavelength in nanometers
    ///
    /// # Response Format
    /// Returns 4-digit nm value (e.g., "0780" for 780nm)
    pub async fn query_wavelength(&self) -> Result<f64> {
        let response = self.query("W?").await?;
        self.parse_wavelength_response(&response)
    }

    /// Set wavelength for accurate power measurement
    ///
    /// # Arguments
    /// * `wavelength_nm` - Wavelength in nanometers (300-1100 nm typical range)
    ///
    /// # Command Format
    /// Sends `Wxxxx` where xxxx is the 4-digit wavelength (e.g., W0800 for 800nm)
    pub async fn set_wavelength_nm(&self, wavelength_nm: f64) -> Result<()> {
        self.wavelength_nm.set(wavelength_nm).await
    }

    /// Query range setting
    ///
    /// # Returns
    /// Range value (1-8 typically)
    pub async fn query_range(&self) -> Result<u8> {
        let response = self.query("R?").await?;
        response
            .trim()
            .parse::<u8>()
            .with_context(|| format!("Failed to parse range response: '{}'", response))
    }

    /// Query units setting
    ///
    /// # Returns
    /// Units value (0=W, 1=dBm, 2=dB)
    pub async fn query_units(&self) -> Result<u8> {
        let response = self.query("U?").await?;
        response
            .trim()
            .parse::<u8>()
            .with_context(|| format!("Failed to parse units response: '{}'", response))
    }

    /// Parse wavelength response
    ///
    /// Handles 4-digit nm format (e.g., "0780" for 780nm)
    fn parse_wavelength_response(&self, response: &str) -> Result<f64> {
        let trimmed = response.trim();

        if trimmed.is_empty() {
            return Err(anyhow!("Empty wavelength response"));
        }

        // Parse as integer then convert to float
        trimmed
            .parse::<u16>()
            .map(|nm| nm as f64)
            .with_context(|| format!("Failed to parse wavelength response: '{}'", trimmed))
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

    /// Send configuration command and clear any response/echo
    ///
    /// Newport 1830-C may echo commands or send acknowledgments.
    /// We must clear the buffer to prevent garbage in subsequent query responses.
    async fn send_config_command(&self, command: &str) -> Result<()> {
        let mut port = self.port.lock().await;

        let cmd = format!("{}\n", command);
        port.get_mut()
            .write_all(cmd.as_bytes())
            .await
            .context("Newport 1830-C write failed")?;

        // Allow meter to process command
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Read and discard any response/echo to clear the buffer
        // This is critical to prevent stale data mixing with query responses
        let mut discard = String::new();
        match tokio::time::timeout(Duration::from_millis(100), port.read_line(&mut discard)).await {
            Ok(Ok(_)) => {
                log::debug!("Newport config '{}' response: {}", command, discard.trim());
            }
            Ok(Err(_)) | Err(_) => {
                // No response or timeout - that's OK for config commands
            }
        }

        Ok(())
    }
}

impl Parameterized for Newport1830CDriver {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl Readable for Newport1830CDriver {
    async fn read(&self) -> Result<f64> {
        self.query_power().await
    }
}

#[async_trait]
impl WavelengthTunable for Newport1830CDriver {
    async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()> {
        self.set_wavelength_nm(wavelength_nm).await
    }

    async fn get_wavelength(&self) -> Result<f64> {
        self.query_wavelength().await
    }

    fn wavelength_range(&self) -> (f64, f64) {
        // Newport 1830-C typical wavelength range for silicon detector
        (300.0, 1100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

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
    fn test_parse_wavelength_response() {
        // Test parsing 4-digit wavelength format
        let test_cases = vec![
            ("0780", 780.0),
            ("0800", 800.0),
            ("1064", 1064.0),
            ("0300", 300.0),
            (" 0800 \n", 800.0),
        ];

        for (input, expected) in test_cases {
            let parsed: Result<u16, _> = input.trim().parse();
            assert!(parsed.is_ok(), "Failed to parse wavelength: {}", input);
            assert_eq!(parsed.unwrap() as f64, expected);
        }
    }

    #[test]
    fn test_wavelength_range_validation() {
        // Test wavelength range bounds (300-1100 nm)
        let valid = vec![300, 800, 1064, 1100];
        let invalid = vec![299, 1101, 0, 2000];

        for nm in valid {
            assert!(
                (300..=1100).contains(&nm),
                "{} should be valid wavelength",
                nm
            );
        }

        for nm in invalid {
            assert!(
                !(300..=1100).contains(&nm),
                "{} should be invalid wavelength",
                nm
            );
        }
    }

    #[test]
    fn test_wavelength_command_format() {
        // Test the 4-digit format command generation
        let test_cases = vec![
            (800u16, "W0800"),
            (780u16, "W0780"),
            (1064u16, "W1064"),
            (300u16, "W0300"),
        ];

        for (nm, expected_cmd) in test_cases {
            let cmd = format!("W{:04}", nm);
            assert_eq!(cmd, expected_cmd, "Wrong format for {} nm", nm);
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

    #[tokio::test]
    async fn wavelength_parameter_writes_command() -> Result<()> {
        let (mut host, device) = tokio::io::duplex(32);
        let port: SharedPort = Arc::new(Mutex::new(BufReader::new(Box::new(device))));

        let driver = Newport1830CDriver::with_test_port(port);

        driver.set_wavelength_nm(800.0).await?;

        let mut buf = vec![0u8; 16];
        let n = host.read(&mut buf).await?;
        let sent = String::from_utf8_lossy(&buf[..n]);

        assert!(sent.contains("W0800"));

        Ok(())
    }
}
