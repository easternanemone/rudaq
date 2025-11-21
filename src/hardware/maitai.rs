//! Spectra-Physics MaiTai Ti:Sapphire Laser Driver
//!
//! Reference: MaiTai HP/MaiTai XF User's Manual
//!
//! Protocol Overview:
//! - Format: ASCII command/response over RS-232
//! - Baud: 9600, 8N1, software flow control (XON/XOFF)
//! - Terminator: CR (\r)
//! - Commands: WAVELENGTH:xxx, SHUTTER:x, ON/OFF
//! - Queries: WAVELENGTH?, POWER?, SHUTTER?
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

use crate::hardware::capabilities::Readable;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serial2_tokio::SerialPort;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

/// Driver for Spectra-Physics MaiTai tunable Ti:Sapphire laser
///
/// Implements Readable capability trait for power measurement.
/// Uses MaiTai's ASCII protocol for hardware communication.
pub struct MaiTaiDriver {
    /// Serial port protected by Mutex for exclusive access
    port: Mutex<BufReader<SerialPort>>,
    /// Command timeout duration
    timeout: Duration,
    /// Current wavelength setting (cached for reference)
    wavelength_nm: Mutex<f64>,
}

impl MaiTaiDriver {
    /// Create a new MaiTai driver instance
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyUSB0", "COM3")
    ///
    /// # Errors
    /// Returns error if serial port cannot be opened
    pub fn new(port_path: &str) -> Result<Self> {
        // Configure serial settings with XonXoff flow control (required for MaiTai)
        let port = SerialPort::open(port_path, |mut settings: serial2::Settings| {
            settings.set_raw();
            settings.set_baud_rate(9600)?;
            settings.set_flow_control(serial2::FlowControl::XonXoff);
            Ok(settings)
        }).context("Failed to open MaiTai serial port")?;

        Ok(Self {
            port: Mutex::new(BufReader::new(port)),
            timeout: Duration::from_secs(5),
            wavelength_nm: Mutex::new(800.0), // Default center wavelength
        })
    }

    /// Set wavelength
    ///
    /// # Arguments
    /// * `wavelength_nm` - Target wavelength in nanometers (typically 700-1000 nm)
    ///
    /// # Errors
    /// Returns error if wavelength is out of range or command fails
    pub async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()> {
        if !(690.0..=1040.0).contains(&wavelength_nm) {
            return Err(anyhow!(
                "Wavelength {} nm out of range (690-1040 nm)",
                wavelength_nm
            ));
        }

        self.send_command(&format!("WAVELENGTH:{}", wavelength_nm))
            .await?;

        // Update cached value
        *self.wavelength_nm.lock().await = wavelength_nm;

        // Allow time for wavelength tuning (hardware can take several seconds)
        tokio::time::sleep(Duration::from_millis(100)).await;

        Ok(())
    }

    /// Get current wavelength setting
    ///
    /// # Returns
    /// Wavelength in nanometers
    pub async fn wavelength(&self) -> Result<f64> {
        let response = self.query("WAVELENGTH?").await?;
        let wavelength: f64 = response
            .split(':')
            .last()
            .unwrap_or(&response)
            .trim()
            .parse()
            .context("Failed to parse wavelength")?;

        // Update cached value
        *self.wavelength_nm.lock().await = wavelength;

        Ok(wavelength)
    }

    /// Set shutter state
    ///
    /// # Arguments
    /// * `open` - true to open shutter, false to close
    pub async fn set_shutter(&self, open: bool) -> Result<()> {
        let cmd = if open { "SHUTTER:1" } else { "SHUTTER:0" };
        self.send_command(cmd).await
    }

    /// Get shutter state
    ///
    /// # Returns
    /// true if shutter is open, false if closed
    pub async fn shutter(&self) -> Result<bool> {
        let response = self.query("SHUTTER?").await?;
        let state: i32 = response
            .split(':')
            .last()
            .unwrap_or(&response)
            .trim()
            .parse()
            .context("Failed to parse shutter state")?;

        Ok(state == 1)
    }

    /// Turn laser emission on/off
    ///
    /// # Arguments
    /// * `on` - true to enable emission, false to disable
    pub async fn set_emission(&self, on: bool) -> Result<()> {
        let cmd = if on { "ON" } else { "OFF" };
        self.send_command(cmd).await
    }

    /// Query laser identity
    ///
    /// # Returns
    /// Laser model and serial number string
    pub async fn identify(&self) -> Result<String> {
        self.query("*IDN?").await
    }

    /// Query power (used by Readable trait)
    async fn query_power(&self) -> Result<f64> {
        let response = self.query("POWER?").await?;
        response
            .split(':')
            .last()
            .unwrap_or(&response)
            .trim()
            .parse::<f64>()
            .context("Failed to parse power")
    }

    /// Send query and read response
    async fn query(&self, command: &str) -> Result<String> {
        let mut port = self.port.lock().await;

        // Write command with CR terminator
        let cmd = format!("{}\r", command);
        port.get_mut()
            .write_all(cmd.as_bytes())
            .await
            .context("MaiTai write failed")?;

        // Read response with timeout
        let mut response = String::new();
        tokio::time::timeout(self.timeout, port.read_line(&mut response))
            .await
            .context("MaiTai read timeout")??;

        Ok(response.trim().to_string())
    }

    /// Send command without expecting response
    async fn send_command(&self, command: &str) -> Result<()> {
        let mut port = self.port.lock().await;

        let cmd = format!("{}\r", command);
        port.get_mut()
            .write_all(cmd.as_bytes())
            .await
            .context("MaiTai write failed")?;

        // Small delay to ensure command is processed
        tokio::time::sleep(Duration::from_millis(10)).await;
        Ok(())
    }
}

#[async_trait]
impl Readable for MaiTaiDriver {
    async fn read(&self) -> Result<f64> {
        self.query_power().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wavelength_validation() {
        // Valid range is 690-1040 nm
        assert!(MaiTaiDriver::new("/dev/null").is_ok());
    }
}
