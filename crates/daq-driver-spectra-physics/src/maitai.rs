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
//! # Usage
//!
//! ```rust,ignore
//! use daq_driver_spectra_physics::MaiTaiFactory;
//! use daq_core::driver::DriverFactory;
//!
//! // Register the factory
//! registry.register_factory(Box::new(MaiTaiFactory));
//!
//! // Create via config
//! let config = toml::toml! {
//!     port = "/dev/ttyUSB5"
//! };
//! let components = factory.build(config.into()).await?;
//! ```

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use daq_core::capabilities::{
    EmissionControl, Parameterized, Readable, ShutterControl, WavelengthTunable,
};
use daq_core::driver::{Capability, DeviceComponents, DriverFactory};
use daq_core::error::DaqError;
use daq_core::observable::ParameterSet;
use daq_core::parameter::Parameter;
use futures::future::BoxFuture;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tokio::task::spawn_blocking;
use tokio_serial::SerialPortBuilderExt;
use tracing::instrument;

// =============================================================================
// MaiTaiFactory - DriverFactory implementation
// =============================================================================

/// Configuration for MaiTai driver
#[derive(Debug, Clone, Deserialize)]
pub struct MaiTaiConfig {
    /// Serial port path (e.g., "/dev/ttyUSB5")
    pub port: String,
    /// Optional initial wavelength in nm (default: 800)
    #[serde(default)]
    pub wavelength_nm: Option<f64>,
    /// USB-to-USB connection uses 115200, RS-232 uses 9600
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
}

fn default_baud_rate() -> u32 {
    115200
}

/// Factory for creating MaiTai driver instances.
pub struct MaiTaiFactory;

/// Static capabilities for MaiTai laser
static MAITAI_CAPABILITIES: &[Capability] = &[
    Capability::Readable,
    Capability::WavelengthTunable,
    Capability::ShutterControl,
    Capability::EmissionControl,
    Capability::Parameterized,
];

impl DriverFactory for MaiTaiFactory {
    fn driver_type(&self) -> &'static str {
        "maitai"
    }

    fn name(&self) -> &'static str {
        "Spectra-Physics MaiTai Ti:Sapphire Laser"
    }

    fn capabilities(&self) -> &'static [Capability] {
        MAITAI_CAPABILITIES
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        let cfg: MaiTaiConfig = config.clone().try_into()?;
        if let Some(wl) = cfg.wavelength_nm {
            if !(690.0..=1040.0).contains(&wl) {
                return Err(anyhow!(
                    "Wavelength {} nm out of MaiTai tuning range (690-1040 nm)",
                    wl
                ));
            }
        }
        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let cfg: MaiTaiConfig = config.try_into().context("Invalid MaiTai config")?;

            // Create driver with validation
            let driver = Arc::new(MaiTaiDriver::new_async(&cfg.port, cfg.baud_rate).await?);

            // Set initial wavelength if specified
            if let Some(wl) = cfg.wavelength_nm {
                driver.set_wavelength(wl).await?;
            }

            Ok(DeviceComponents {
                readable: Some(driver.clone()),
                wavelength_tunable: Some(driver.clone()),
                shutter_control: Some(driver.clone()),
                emission_control: Some(driver.clone()),
                parameterized: Some(driver),
                ..Default::default()
            })
        })
    }
}

// =============================================================================
// MaiTaiDriver
// =============================================================================

pub trait SerialPortIO: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> SerialPortIO for T {}
type DynSerial = Box<dyn SerialPortIO>;
type SharedPort = Arc<Mutex<BufReader<DynSerial>>>;

/// Driver for Spectra-Physics MaiTai tunable Ti:Sapphire laser
///
/// Implements Readable, WavelengthTunable, ShutterControl, and EmissionControl
/// capability traits. Uses MaiTai's ASCII protocol for hardware communication.
pub struct MaiTaiDriver {
    /// Serial port protected by Mutex for exclusive access
    port: SharedPort,
    /// Command timeout duration
    timeout: Duration,
    /// Current wavelength setting
    wavelength_nm: Parameter<f64>,
    /// Parameter registry
    params: Arc<ParameterSet>,
}

impl MaiTaiDriver {
    /// Create a new MaiTai driver instance asynchronously with device validation
    ///
    /// This is the **preferred constructor** for production use.
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyUSB5", "COM3")
    /// * `baud_rate` - Baud rate (115200 for USB-to-USB, 9600 for RS-232)
    ///
    /// # Errors
    /// Returns error if:
    /// - Serial port cannot be opened
    /// - Device doesn't respond to identity query
    /// - Device identity doesn't contain "MaiTai"
    pub async fn new_async(port_path: &str, baud_rate: u32) -> Result<Self> {
        Self::new_async_with_baud(port_path, baud_rate).await
    }

    /// Create a new MaiTai driver with default baud rate (115200).
    ///
    /// This is a convenience method for USB-to-USB connections.
    /// Use `new_async` for RS-232 connections that need 9600 baud.
    pub async fn new_async_default(port_path: &str) -> Result<Self> {
        Self::new_async_with_baud(port_path, default_baud_rate()).await
    }

    /// Internal constructor with baud rate.
    async fn new_async_with_baud(port_path: &str, baud_rate: u32) -> Result<Self> {
        let port_path_owned = port_path.to_string();

        // Use spawn_blocking to avoid blocking the async runtime
        let port = spawn_blocking(move || {
            tokio_serial::new(&port_path_owned, baud_rate)
                .data_bits(tokio_serial::DataBits::Eight)
                .parity(tokio_serial::Parity::None)
                .stop_bits(tokio_serial::StopBits::One)
                .flow_control(tokio_serial::FlowControl::None)
                .open_native_async()
                .context(format!(
                    "Failed to open MaiTai serial port: {}",
                    port_path_owned
                ))
        })
        .await
        .context("spawn_blocking for MaiTai port opening failed")??;

        let driver = Self::build(Arc::new(Mutex::new(BufReader::new(Box::new(port)))));

        // Validate device identity
        match driver.identify().await {
            Ok(identity) => {
                if !identity.to_uppercase().contains("MAITAI") {
                    return Err(anyhow!(
                        "MaiTai validation failed: device identity '{}' doesn't indicate a MaiTai laser",
                        identity
                    ));
                }
                tracing::info!("MaiTai laser validated: {}", identity);
            }
            Err(e) => {
                return Err(anyhow!(
                    "MaiTai validation failed: no response to identity query (*IDN?). Error: {}",
                    e
                ));
            }
        }

        Ok(driver)
    }

    fn build(port: SharedPort) -> Self {
        let mut params = ParameterSet::new();
        let mut wavelength_nm = Parameter::new("wavelength_nm", 800.0)
            .with_description("Tunable laser wavelength")
            .with_unit("nm")
            .with_range(690.0, 1040.0);

        // Attach hardware write callback
        Self::attach_wavelength_callbacks(&mut wavelength_nm, port.clone());

        params.register(wavelength_nm.clone());

        Self {
            port,
            timeout: Duration::from_secs(5),
            wavelength_nm,
            params: Arc::new(params),
        }
    }

    /// Attach hardware callbacks to wavelength parameter.
    fn attach_wavelength_callbacks(wavelength: &mut Parameter<f64>, port: SharedPort) {
        wavelength.connect_to_hardware_write(move |target: f64| {
            let port = port.clone();
            Box::pin(async move {
                let cmd = format!("WAVELENGTH:{}\r\n", target);
                let mut guard = port.lock().await;
                guard
                    .get_mut()
                    .write_all(cmd.as_bytes())
                    .await
                    .context("Failed to write wavelength command")
                    .map_err(|e| DaqError::Instrument(e.to_string()))?;
                guard
                    .get_mut()
                    .flush()
                    .await
                    .context("Failed to flush wavelength command")
                    .map_err(|e| DaqError::Instrument(e.to_string()))?;
                tokio::time::sleep(Duration::from_millis(50)).await;

                // Read and discard any response/echo
                let mut response = String::new();
                match tokio::time::timeout(
                    Duration::from_millis(500),
                    guard.read_line(&mut response),
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
        });
    }

    #[cfg(test)]
    pub(crate) fn with_test_port(port: SharedPort) -> Self {
        Self::build(port)
    }

    /// Query laser identity
    pub async fn identify(&self) -> Result<String> {
        self.query("*IDN?").await
    }

    /// Set shutter state
    ///
    /// MaiTai uses `SHUTter:1` and `SHUTter:0` with colon separator
    pub async fn set_shutter(&self, open: bool) -> Result<()> {
        let cmd = if open { "SHUTter:1" } else { "SHUTter:0" };
        self.send_command(cmd).await
    }

    /// Get shutter state
    pub async fn shutter(&self) -> Result<bool> {
        let response = self.query("SHUTTER?").await?;
        let state: i32 = response
            .trim()
            .parse()
            .context(format!("Failed to parse shutter state from '{}'", response))?;

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
    /// Safety: Refuses to enable emission if shutter is open
    pub async fn set_emission(&self, on: bool) -> Result<()> {
        log::info!("MaiTai: set_emission({})", on);
        if on {
            let shutter_result = self.shutter().await;
            log::info!("MaiTai: shutter query result = {:?}", shutter_result);
            let shutter_open = shutter_result.as_ref().map(|&v| v).unwrap_or(true);
            if shutter_open {
                log::warn!("SAFETY: Emission enable refused - shutter is open or state unknown");
                return Err(anyhow!(
                    "Refusing to enable emission: shutter is open or state unknown. Close shutter first."
                ));
            }
        }
        let cmd = if on { "ON" } else { "OFF" };
        log::info!("MaiTai: sending command '{}'", cmd);
        self.send_command(cmd).await
    }

    /// Query current emission state via status byte
    ///
    /// The MaiTai uses *STB? to query the product status byte.
    /// Bit 0 indicates laser on (1) or off (0).
    pub async fn emission(&self) -> Result<bool> {
        log::info!("MaiTai: querying emission state via *STB?");
        let response = self.query("*STB?").await?;
        log::info!("MaiTai: *STB? response = {:?}", response);

        let status: i32 = response
            .trim()
            .parse()
            .context(format!("Failed to parse status byte from '{}'", response))?;

        // Bit 0 indicates laser on state
        let is_on = (status & 1) != 0;
        log::info!("MaiTai: status byte = {}, bit0 = {}, is_on = {}", status, status & 1, is_on);
        Ok(is_on)
    }

    /// Query current wavelength setting
    pub async fn query_wavelength(&self) -> Result<f64> {
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
        let _ = self.wavelength_nm.inner().set(wavelength);

        Ok(wavelength)
    }

    /// Query power measurement
    async fn query_power(&self) -> Result<f64> {
        let response = self.query("POWER?").await?;
        // Response format may include units
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

        let cmd = format!("{}\r\n", command);
        port.get_mut()
            .write_all(cmd.as_bytes())
            .await
            .context("MaiTai write failed")?;
        port.get_mut()
            .flush()
            .await
            .context("MaiTai flush failed")?;

        tokio::time::sleep(Duration::from_millis(50)).await;

        let mut response = String::new();
        tokio::time::timeout(self.timeout, port.read_line(&mut response))
            .await
            .context("MaiTai read timeout")??;

        Ok(response.trim().to_string())
    }

    /// Send command and read any response
    async fn send_command(&self, command: &str) -> Result<()> {
        let mut port = self.port.lock().await;

        let cmd = format!("{}\r\n", command);
        port.get_mut()
            .write_all(cmd.as_bytes())
            .await
            .context("MaiTai write failed")?;
        port.get_mut()
            .flush()
            .await
            .context("MaiTai flush failed")?;

        tokio::time::sleep(Duration::from_millis(50)).await;

        // Read and discard any response/echo
        let mut response = String::new();
        match tokio::time::timeout(Duration::from_millis(500), port.read_line(&mut response)).await
        {
            Ok(Ok(_)) => {
                log::debug!("MaiTai command '{}' response: {}", command, response.trim());
            }
            Ok(Err(_)) | Err(_) => {
                // No response or timeout - OK for set commands
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
        self.wavelength_nm.set(wavelength_nm).await
    }

    #[instrument(skip(self), err)]
    async fn get_wavelength(&self) -> Result<f64> {
        self.query_wavelength().await
    }

    fn wavelength_range(&self) -> (f64, f64) {
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

    #[instrument(skip(self), err)]
    async fn is_emission_enabled(&self) -> Result<bool> {
        self.emission().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[test]
    fn test_factory_driver_type() {
        let factory = MaiTaiFactory;
        assert_eq!(factory.driver_type(), "maitai");
        assert_eq!(factory.name(), "Spectra-Physics MaiTai Ti:Sapphire Laser");
    }

    #[test]
    fn test_factory_capabilities() {
        let factory = MaiTaiFactory;
        let caps = factory.capabilities();
        assert!(caps.contains(&Capability::Readable));
        assert!(caps.contains(&Capability::WavelengthTunable));
        assert!(caps.contains(&Capability::ShutterControl));
        assert!(caps.contains(&Capability::EmissionControl));
        assert!(caps.contains(&Capability::Parameterized));
    }

    #[tokio::test]
    async fn test_factory_validate_config() {
        let factory = MaiTaiFactory;

        // Valid config
        let valid_config = toml::Value::Table(toml::toml! {
            port = "/dev/ttyUSB5"
        });
        assert!(factory.validate(&valid_config).is_ok());

        // Valid config with wavelength
        let valid_with_wl = toml::Value::Table(toml::toml! {
            port = "/dev/ttyUSB5"
            wavelength_nm = 800.0
        });
        assert!(factory.validate(&valid_with_wl).is_ok());

        // Invalid wavelength (out of MaiTai range)
        let invalid_wl = toml::Value::Table(toml::toml! {
            port = "/dev/ttyUSB5"
            wavelength_nm = 1100.0
        });
        assert!(factory.validate(&invalid_wl).is_err());

        // Missing port
        let missing_port = toml::Value::Table(toml::toml! {
            wavelength_nm = 800.0
        });
        assert!(factory.validate(&missing_port).is_err());
    }

    #[test]
    fn test_wavelength_range() {
        let min = 690.0;
        let max = 1040.0;

        assert!((min..=max).contains(&800.0));
        assert!((min..=max).contains(&690.0));
        assert!((min..=max).contains(&1040.0));
        assert!(!(min..=max).contains(&689.0));
        assert!(!(min..=max).contains(&1041.0));
    }

    #[test]
    fn test_parse_wavelength_response() {
        let test_cases = vec![
            ("820nm", 820.0),
            ("750NM", 750.0),
            (" 800nm \n", 800.0),
            ("1000", 1000.0),
        ];

        for (input, expected) in test_cases {
            let clean = input.trim().trim_end_matches("nm").trim_end_matches("NM");
            let wavelength: f64 = clean.parse().unwrap();
            assert_eq!(wavelength, expected, "Failed to parse: {}", input);
        }
    }

    #[test]
    fn test_parse_power_response() {
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
        assert_eq!("0".trim().parse::<i32>().unwrap(), 0);
        assert_eq!("1".trim().parse::<i32>().unwrap(), 1);
        assert_eq!(" 0 \n".trim().parse::<i32>().unwrap(), 0);
    }

    #[tokio::test]
    async fn wavelength_parameter_writes_command() -> Result<()> {
        let (mut host, device) = tokio::io::duplex(64);
        let port: SharedPort = Arc::new(Mutex::new(BufReader::new(Box::new(device))));

        let driver = MaiTaiDriver::with_test_port(port);

        driver.set_wavelength(800.0).await?;

        let mut buf = vec![0u8; 64];
        let n = host.read(&mut buf).await?;
        let sent = String::from_utf8_lossy(&buf[..n]);

        assert!(sent.contains("WAVELENGTH:800"));

        Ok(())
    }
}
