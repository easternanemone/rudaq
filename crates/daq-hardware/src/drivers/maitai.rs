//! Spectra-Physics MaiTai Ti:Sapphire Laser Driver
//!
//! Reference: MaiTai HP/MaiTai XF User's Manual
//!
//! Protocol Overview:
//! - Format: ASCII command/response over RS-232 or USB-to-USB
//! - Baud: 115200 (USB-to-USB) or 9600 (RS-232), 8N1, NO flow control
//! - Command terminator: CR+LF (\r\n)
//! - Response terminator: LF (\n)
//! - Commands: WAVELENGTH:xxx, SHUTTER:x, ON/OFF
//! - Queries: WAVELENGTH?, POWER?, SHUTTER?
//!
//! Response Formats (actual observed from hardware):
//! - WAVELENGTH? -> "820nm\n" (value with "nm" suffix)
//! - SHUTTER? -> "0\n" or "1\n" (0=closed, 1=open)
//! - POWER? -> value with units
//!
//! # Example Usage
//!
//! ```no_run
//! use rust_daq::hardware::maitai::MaiTaiDriver;
//! use rust_daq::hardware::capabilities::Readable;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let laser = MaiTaiDriver::new("/dev/ttyUSB0")?;
//!
//!     // Set wavelength
//!     laser.set_wavelength(800.0).await?;
//!
//!     // Open shutter
//!     laser.set_shutter(true).await?;
//!
//!     // Read power
//!     let power_watts = laser.read().await?;
//!     println!("Power: {:.3} W", power_watts);
//!
//!     Ok(())
//! }
//! ```

use crate::capabilities::{
    EmissionControl, Parameterized, Readable, ShutterControl, WavelengthTunable,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use daq_core::error::DaqError;
use daq_core::observable::ParameterSet;
use daq_core::parameter::Parameter;
use futures::future::BoxFuture;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tokio::task::spawn_blocking;
use tokio_serial::{SerialPortBuilderExt, SerialStream};
use tracing::instrument;

/// Driver for Spectra-Physics MaiTai tunable Ti:Sapphire laser
///
/// Implements Readable capability trait for power measurement.
/// Uses MaiTai's ASCII protocol for hardware communication.
pub struct MaiTaiDriver {
    /// Serial port protected by Mutex for exclusive access
    port: Arc<Mutex<BufReader<SerialStream>>>,
    /// Command timeout duration
    timeout: Duration,
    /// Current wavelength setting (cached for reference)
    wavelength_nm: Parameter<f64>,
    /// Parameter registry
    params: ParameterSet,
}

impl MaiTaiDriver {
    /// Create a new MaiTai driver instance
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyUSB0", "COM3")
    ///
    /// # Errors
    /// Returns error if serial port cannot be opened
    ///
    /// # Note
    /// This constructor may block the async runtime during serial port opening.
    /// For non-blocking construction, use [`new_async`] instead.
    pub fn new(port_path: &str) -> Result<Self> {
        // Configure serial settings for USB-to-USB connection (115200, 8N1, no flow control)
        // Note: RS-232 connections may require 9600 baud - adjust if using serial adapter
        let port = tokio_serial::new(port_path, 115200)
            .data_bits(tokio_serial::DataBits::Eight)
            .parity(tokio_serial::Parity::None)
            .stop_bits(tokio_serial::StopBits::One)
            .flow_control(tokio_serial::FlowControl::None)
            .open_native_async()
            .context(format!("Failed to open MaiTai serial port: {}", port_path))?;

        let port_mutex = Arc::new(Mutex::new(BufReader::new(port)));

        // Create wavelength parameter with metadata and hardware callback
        let mut params = ParameterSet::new();
        let mut wavelength = Parameter::new("wavelength_nm", 800.0)
            .with_description("Tunable laser wavelength")
            .with_unit("nm")
            .with_range(690.0, 1040.0); // MaiTai tuning range

        wavelength.connect_to_hardware_write({
            let port = port_mutex.clone();
            move |wavelength: f64| -> BoxFuture<'static, Result<(), DaqError>> {
                let port = port.clone();
                Box::pin(async move {
                    let mut p = port.lock().await;
                    let cmd = format!("WAVELENGTH:{}\r\n", wavelength);
                    p.get_mut()
                        .write_all(cmd.as_bytes())
                        .await
                        .context("Failed to write wavelength command")
                        .map_err(|e| DaqError::Instrument(e.to_string()))?;
                    p.get_mut()
                        .flush()
                        .await
                        .context("Failed to flush wavelength command")
                        .map_err(|e| DaqError::Instrument(e.to_string()))?;
                    tokio::time::sleep(Duration::from_millis(500)).await;

                    // Read and discard any response/echo to clear the buffer
                    let mut response = String::new();
                    match tokio::time::timeout(
                        Duration::from_millis(500),
                        p.read_line(&mut response),
                    )
                    .await
                    {
                        Ok(Ok(_)) => {
                            log::debug!("MaiTai wavelength response: {}", response.trim())
                        }
                        Ok(Err(e)) => {
                            log::debug!("MaiTai wavelength read error (may be OK): {}", e)
                        }
                        Err(_) => log::debug!("MaiTai wavelength no response (may be OK)"),
                    }

                    Ok(())
                })
            }
        });

        // Register parameter
        params.register(wavelength.clone());

        Ok(Self {
            port: port_mutex,
            timeout: Duration::from_secs(5),
            wavelength_nm: wavelength,
            params,
        })
    }

    /// Create a new MaiTai driver instance asynchronously
    ///
    /// This is the preferred constructor as it uses `spawn_blocking` to avoid
    /// blocking the async runtime during serial port opening.
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyUSB0", "COM3")
    ///
    /// # Errors
    /// Returns error if serial port cannot be opened
    pub async fn new_async(port_path: &str) -> Result<Self> {
        let port_path = port_path.to_string();

        // Use spawn_blocking to avoid blocking the async runtime
        // USB-to-USB connection: 115200 baud, 8N1, no flow control
        let port = spawn_blocking(move || {
            tokio_serial::new(&port_path, 115200)
                .data_bits(tokio_serial::DataBits::Eight)
                .parity(tokio_serial::Parity::None)
                .stop_bits(tokio_serial::StopBits::One)
                .flow_control(tokio_serial::FlowControl::None)
                .open_native_async()
                .context(format!("Failed to open MaiTai serial port: {}", port_path))
        })
        .await
        .context("spawn_blocking for MaiTai port opening failed")??;

        let port_mutex = Arc::new(Mutex::new(BufReader::new(port)));

        // Create wavelength parameter with metadata and hardware callback
        let mut params = ParameterSet::new();
        let mut wavelength = Parameter::new("wavelength_nm", 800.0)
            .with_description("Tunable laser wavelength")
            .with_unit("nm")
            .with_range(690.0, 1040.0); // MaiTai tuning range

        wavelength.connect_to_hardware_write({
            let port = port_mutex.clone();
            move |wavelength: f64| -> BoxFuture<'static, Result<(), DaqError>> {
                let port = port.clone();
                Box::pin(async move {
                    let mut p = port.lock().await;
                    let cmd = format!("WAVELENGTH:{}\r\n", wavelength);
                    p.get_mut()
                        .write_all(cmd.as_bytes())
                        .await
                        .context("Failed to write wavelength command")
                        .map_err(|e| DaqError::Instrument(e.to_string()))?;
                    p.get_mut()
                        .flush()
                        .await
                        .context("Failed to flush wavelength command")
                        .map_err(|e| DaqError::Instrument(e.to_string()))?;
                    tokio::time::sleep(Duration::from_millis(500)).await;

                    // Read and discard any response/echo to clear the buffer
                    let mut response = String::new();
                    match tokio::time::timeout(
                        Duration::from_millis(500),
                        p.read_line(&mut response),
                    )
                    .await
                    {
                        Ok(Ok(_)) => {
                            log::debug!("MaiTai wavelength response: {}", response.trim())
                        }
                        Ok(Err(e)) => {
                            log::debug!("MaiTai wavelength read error (may be OK): {}", e)
                        }
                        Err(_) => log::debug!("MaiTai wavelength no response (may be OK)"),
                    }

                    Ok(())
                })
            }
        });

        // Register parameter
        params.register(wavelength.clone());

        Ok(Self {
            port: port_mutex,
            timeout: Duration::from_secs(5),
            wavelength_nm: wavelength,
            params,
        })
    }

    /// Set wavelength
    ///
    /// # Arguments
    /// * `wavelength_nm` - Target wavelength in nanometers (typically 700-1000 nm)
    ///
    /// # Errors
    /// Returns error if wavelength is out of range or command fails
    #[instrument(skip(self), fields(wavelength_nm), err)]
    pub async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()> {
        self.wavelength_nm.set(wavelength_nm).await
    }

    /// Get current wavelength setting
    ///
    /// # Returns
    /// Wavelength in nanometers
    #[instrument(skip(self), err)]
    pub async fn wavelength(&self) -> Result<f64> {
        let response = self.query("WAVELENGTH?").await?;
        // Response format: "820nm" - strip "nm" suffix if present
        let clean = response
            .trim()
            .trim_end_matches("nm")
            .trim_end_matches("NM");
        let wavelength: f64 = clean
            .parse()
            .context(format!("Failed to parse wavelength from '{}'", response))?;

        // Update cached value
        self.wavelength_nm.set(wavelength).await?;

        Ok(wavelength)
    }

    /// Set shutter state
    ///
    /// # Arguments
    /// * `open` - true to open shutter, false to close
    ///
    /// # Command Format
    /// MaiTai uses `SHUTter:1` and `SHUTter:0` with colon separator
    /// (verified from hardware test examples)
    #[instrument(skip(self), fields(open), err)]
    pub async fn set_shutter(&self, open: bool) -> Result<()> {
        let cmd = if open { "SHUTter:1" } else { "SHUTter:0" };
        self.send_command(cmd).await
    }

    /// Get shutter state
    ///
    /// # Returns
    /// true if shutter is open, false if closed
    ///
    /// # Note
    /// Both `SHUTTER?` (uppercase) and `SHUTter?` (mixed case) work for queries.
    /// Using uppercase for consistency with other query commands.
    #[instrument(skip(self), err)]
    pub async fn shutter(&self) -> Result<bool> {
        let response = self.query("SHUTTER?").await?;
        // Response format: "0" or "1" (no prefix)
        let state: i32 = response
            .trim()
            .parse()
            .context(format!("Failed to parse shutter state from '{}'", response))?;

        // Validate expected values - only 0 (closed) or 1 (open) are valid
        match state {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(anyhow!(
                "Unexpected shutter state '{}' (expected 0 or 1)",
                state
            )),
        }
    }

    /// Turn laser emission on/off
    ///
    /// # Arguments
    /// * `on` - true to enable emission, false to disable
    ///
    /// # Safety
    /// This method will refuse to enable emission if the shutter is open or
    /// if shutter state cannot be determined. This prevents accidental laser
    /// exposure. Always close the shutter before enabling emission.
    #[instrument(skip(self), fields(on), err)]
    pub async fn set_emission(&self, on: bool) -> Result<()> {
        // Safety: never enable emission with shutter open
        if on {
            // Query shutter state; treat unknown state as "open" (fail-safe)
            let shutter_result = self.shutter().await;
            let shutter_open = shutter_result.as_ref().map(|&v| v).unwrap_or(true);
            if shutter_open {
                // Audit log: emission refusal for safety traceability
                log::warn!(
                    "SAFETY: Emission enable refused - shutter_state={}, shutter_query_result={:?}",
                    if shutter_open {
                        "open/unknown"
                    } else {
                        "closed"
                    },
                    shutter_result
                        .as_ref()
                        .map(|v| *v)
                        .map_err(|e| e.to_string())
                );
                return Err(anyhow!(
                    "Refusing to enable emission: shutter is open or state unknown. Close shutter first."
                ));
            }
        }
        let cmd = if on { "ON" } else { "OFF" };
        self.send_command(cmd).await
    }

    /// Query laser identity
    ///
    /// # Returns
    /// Laser model and serial number string
    #[instrument(skip(self), err)]
    pub async fn identify(&self) -> Result<String> {
        self.query("*IDN?").await
    }

    /// Query power (used by Readable trait)
    async fn query_power(&self) -> Result<f64> {
        let response = self.query("POWER?").await?;
        // Response format may include units - strip common suffixes (case-insensitive)
        let clean = response.trim().to_lowercase();
        let clean = clean
            .trim_end_matches("mw")
            .trim_end_matches("w")
            .trim_end_matches("%")
            .trim();
        clean
            .parse::<f64>()
            .context(format!("Failed to parse power from '{}'", response))
    }

    /// Send query and read response
    async fn query(&self, command: &str) -> Result<String> {
        let mut port = self.port.lock().await;

        // Write command with CR+LF terminator (per MaiTai protocol)
        let cmd = format!("{}\r\n", command);
        port.get_mut()
            .write_all(cmd.as_bytes())
            .await
            .context("MaiTai write failed")?;

        // Flush to ensure command is sent immediately
        port.get_mut()
            .flush()
            .await
            .context("MaiTai flush failed")?;

        // Small delay for device to process command
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Read response with timeout
        let mut response = String::new();
        tokio::time::timeout(self.timeout, port.read_line(&mut response))
            .await
            .context("MaiTai read timeout")??;

        Ok(response.trim().to_string())
    }

    /// Send command and read any response
    ///
    /// MaiTai may send responses even for "set" commands, so we read to:
    /// 1. Clear the serial buffer
    /// 2. Get command acknowledgment/echo
    /// 3. Ensure device is ready for the next command
    async fn send_command(&self, command: &str) -> Result<()> {
        let mut port = self.port.lock().await;

        // Use CR+LF terminator (per MaiTai protocol - validated in hardware tests)
        let cmd = format!("{}\r\n", command);
        port.get_mut()
            .write_all(cmd.as_bytes())
            .await
            .context("MaiTai write failed")?;

        // Flush to ensure command is sent immediately
        port.get_mut()
            .flush()
            .await
            .context("MaiTai flush failed")?;

        // Wait for device to process command
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Read and discard any response/echo to clear the buffer
        // Device may send acknowledgment, echo, or error
        let mut response = String::new();
        match tokio::time::timeout(Duration::from_millis(500), port.read_line(&mut response)).await
        {
            Ok(Ok(_)) => {
                log::debug!(
                    "MaiTai set command '{}' response: {}",
                    command,
                    response.trim()
                );
            }
            Ok(Err(e)) => {
                log::debug!(
                    "MaiTai set command '{}' read error (may be OK): {}",
                    command,
                    e
                );
            }
            Err(_) => {
                log::debug!("MaiTai set command '{}' no response (may be OK)", command);
            }
        }

        Ok(())
    }
}

impl Parameterized for MaiTaiDriver {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl Readable for MaiTaiDriver {
    #[instrument(skip(self), err)]
    async fn read(&self) -> Result<f64> {
        self.query_power().await
    }
}

#[async_trait]
impl WavelengthTunable for MaiTaiDriver {
    #[instrument(skip(self), fields(wavelength_nm), err)]
    async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()> {
        // Just delegate to parameter - callback handles hardware
        self.wavelength_nm.set(wavelength_nm).await
    }

    #[instrument(skip(self), err)]
    async fn get_wavelength(&self) -> Result<f64> {
        // Query hardware for actual wavelength
        self.wavelength().await
    }

    fn wavelength_range(&self) -> (f64, f64) {
        // MaiTai Ti:Sapphire tuning range
        (690.0, 1040.0)
    }
}

#[async_trait]
impl ShutterControl for MaiTaiDriver {
    #[instrument(skip(self), err)]
    async fn open_shutter(&self) -> Result<()> {
        self.set_shutter(true).await
    }

    #[instrument(skip(self), err)]
    async fn close_shutter(&self) -> Result<()> {
        self.set_shutter(false).await
    }

    #[instrument(skip(self), err)]
    async fn is_shutter_open(&self) -> Result<bool> {
        self.shutter().await
    }
}

#[async_trait]
impl EmissionControl for MaiTaiDriver {
    #[instrument(skip(self), err)]
    async fn enable_emission(&self) -> Result<()> {
        self.set_emission(true).await
    }

    #[instrument(skip(self), err)]
    async fn disable_emission(&self) -> Result<()> {
        self.set_emission(false).await
    }

    // Note: MaiTai doesn't provide emission state query,
    // so we use the default implementation which returns an error
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_wavelength_range() {
        // MaiTai Ti:Sapphire tuning range is 690-1040 nm
        let min = 690.0;
        let max = 1040.0;

        // Test valid wavelengths
        assert!((min..=max).contains(&800.0));
        assert!((min..=max).contains(&690.0));
        assert!((min..=max).contains(&1040.0));

        // Test invalid wavelengths
        assert!(!(min..=max).contains(&689.0));
        assert!(!(min..=max).contains(&1041.0));
    }

    #[test]
    fn test_parse_wavelength_response() {
        // Test parsing "820nm" format
        let response = "820nm";
        let clean = response
            .trim()
            .trim_end_matches("nm")
            .trim_end_matches("NM");
        let wavelength: f64 = clean.parse().unwrap();
        assert_eq!(wavelength, 820.0);

        // Test parsing with whitespace
        let response = " 750nm \n";
        let clean = response
            .trim()
            .trim_end_matches("nm")
            .trim_end_matches("NM");
        let wavelength: f64 = clean.parse().unwrap();
        assert_eq!(wavelength, 750.0);
    }

    #[test]
    fn test_parse_power_response() {
        // Test parsing power with various unit formats
        let test_cases = vec![
            ("3.00W", 3.0),
            ("3.00w", 3.0),
            ("100mW", 100.0),
            ("100mw", 100.0),
            ("50%", 50.0),
            (" 2.5W \n", 2.5),
        ];

        for (response, expected) in test_cases {
            let clean = response.trim().to_lowercase();
            let clean = clean
                .trim_end_matches("mw")
                .trim_end_matches("w")
                .trim_end_matches("%")
                .trim();
            let power: f64 = clean.parse().unwrap();
            assert_eq!(power, expected, "Failed to parse '{}'", response);
        }
    }

    #[test]
    fn test_parse_shutter_response() {
        // Test parsing shutter state
        assert_eq!("0".trim().parse::<i32>().unwrap(), 0);
        assert_eq!("1".trim().parse::<i32>().unwrap(), 1);
        assert_eq!(" 0 \n".trim().parse::<i32>().unwrap(), 0);
    }
}
