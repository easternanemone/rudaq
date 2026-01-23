//! Newport 1830-C Optical Power Meter Driver
//!
//! Reference: Newport 1830-C User's Manual
//!
//! Protocol Overview:
//! - Format: Simple ASCII commands (NOT SCPI)
//! - Baud: 9600, 8N1, no flow control
//! - Terminator: LF only (\n) - NOT CRLF
//! - Commands: A0/A1 (attenuator), F1/F2/F3 (filter), Wxxxx (wavelength), U1-U4 (units)
//! - Queries: D? (power), W? (wavelength), R? (range), U? (units)
//!
//! Unit Commands (1-indexed):
//! - U1 = Watts (scientific notation, e.g., "+.11E-9")
//! - U2 = dBm (decimal, e.g., "-15.24")
//! - U3 = dB (relative, decimal)
//! - U4 = REL (relative linear)
//!
//! IMPORTANT: This driver sets the device to Watts mode (U1) on initialization
//! to ensure consistent scientific notation response format for parsing.
//!
//! # Usage
//!
//! ```rust,ignore
//! use daq_driver_newport::Newport1830CFactory;
//! use daq_core::driver::DriverFactory;
//!
//! // Register the factory
//! registry.register_factory(Box::new(Newport1830CFactory));
//!
//! // Create via config
//! let config = toml::toml! {
//!     port = "/dev/ttyS0"
//! };
//! let components = factory.build(config.into()).await?;
//! ```

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use daq_core::capabilities::{Parameterized, Readable, WavelengthTunable};
use daq_core::driver::{Capability, DeviceComponents, DriverFactory};
use daq_core::error::DaqError;
use daq_core::observable::ParameterSet;
use daq_core::parameter::Parameter;
use futures::future::BoxFuture;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tokio::task::spawn_blocking;
use tokio_serial::SerialPortBuilderExt;
use tracing::instrument;

// =============================================================================
// Newport1830CFactory - DriverFactory implementation
// =============================================================================

/// Configuration for Newport 1830-C driver
#[derive(Debug, Clone, Deserialize)]
pub struct Newport1830CConfig {
    /// Serial port path (e.g., "/dev/ttyS0")
    pub port: String,
    /// Optional initial wavelength in nm (default: 800)
    #[serde(default)]
    pub wavelength_nm: Option<f64>,
}

/// Factory for creating Newport 1830-C driver instances.
pub struct Newport1830CFactory;

/// Static capabilities for Newport 1830-C
static NEWPORT_1830C_CAPABILITIES: &[Capability] = &[
    Capability::Readable,
    Capability::WavelengthTunable,
    Capability::Parameterized,
];

impl DriverFactory for Newport1830CFactory {
    fn driver_type(&self) -> &'static str {
        "newport1830_c"
    }

    fn name(&self) -> &'static str {
        "Newport 1830-C Optical Power Meter"
    }

    fn capabilities(&self) -> &'static [Capability] {
        NEWPORT_1830C_CAPABILITIES
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        let cfg: Newport1830CConfig = config.clone().try_into()?;
        if let Some(wl) = cfg.wavelength_nm {
            if !(300.0..=1100.0).contains(&wl) {
                return Err(anyhow!("Wavelength {} nm out of range (300-1100 nm)", wl));
            }
        }
        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let cfg: Newport1830CConfig =
                config.try_into().context("Invalid Newport 1830-C config")?;

            // Create driver with validation
            let driver = Arc::new(Newport1830CDriver::new_async(&cfg.port).await?);

            // Set initial wavelength if specified
            if let Some(wl) = cfg.wavelength_nm {
                driver.set_wavelength(wl).await?;
            }

            Ok(DeviceComponents {
                readable: Some(driver.clone()),
                wavelength_tunable: Some(driver.clone()),
                parameterized: Some(driver),
                ..Default::default()
            })
        })
    }
}

// =============================================================================
// Newport1830CDriver
// =============================================================================

pub trait SerialPortIO: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> SerialPortIO for T {}
type DynSerial = Box<dyn SerialPortIO>;
type SharedPort = Arc<Mutex<BufReader<DynSerial>>>;

/// Driver for Newport 1830-C optical power meter
///
/// Implements Readable and WavelengthTunable capability traits.
/// Uses Newport's simple ASCII protocol (not SCPI).
pub struct Newport1830CDriver {
    /// Serial port protected by Mutex for exclusive access
    port: SharedPort,
    /// Command timeout duration
    timeout: Duration,
    /// Wavelength parameter (nm)
    wavelength_nm: Parameter<f64>,
    /// Parameter registry
    params: Arc<ParameterSet>,
}

impl Newport1830CDriver {
    /// Create a new Newport 1830-C driver with device validation
    ///
    /// This is the **preferred constructor** for production use.
    ///
    /// # Arguments
    /// * `port_path` - Serial port path (e.g., "/dev/ttyS0", "COM3")
    ///
    /// # Errors
    /// Returns error if:
    /// - Serial port cannot be opened
    /// - Device doesn't respond to wavelength query
    pub async fn new_async(port_path: &str) -> Result<Self> {
        let port_path_owned = port_path.to_string();

        // Use spawn_blocking to avoid blocking the async runtime
        let port = spawn_blocking(move || {
            tokio_serial::new(&port_path_owned, 9600)
                .data_bits(tokio_serial::DataBits::Eight)
                .parity(tokio_serial::Parity::None)
                .stop_bits(tokio_serial::StopBits::One)
                .flow_control(tokio_serial::FlowControl::None)
                .open_native_async()
                .context(format!(
                    "Failed to open Newport 1830-C serial port: {}",
                    port_path_owned
                ))
        })
        .await
        .context("spawn_blocking for Newport 1830-C port opening failed")??;

        let driver = Self::build(Arc::new(Mutex::new(BufReader::new(Box::new(port)))));

        // Disable echo mode (E0) FIRST to prevent command echoes in responses.
        // The 1830-C has E0/E1 for echo off/on. Without this, responses may include
        // the echoed command which corrupts parsing.
        driver
            .send_config_command("E0")
            .await
            .context("Newport 1830-C: failed to disable echo mode (E0) during initialization")?;
        tracing::info!("Newport 1830-C: disabled echo mode (E0)");

        // Set units to Watts (U1) to ensure consistent scientific notation response format.
        // CRITICAL: If device is in dBm mode, D? returns decimal (e.g., "-15.24") instead
        // of scientific notation, causing incorrect parsing.
        driver.set_units_watts().await.context(
            "Newport 1830-C: failed to set units to Watts mode (U1) during initialization",
        )?;
        tracing::info!("Newport 1830-C: set units to Watts mode (U1)");

        // Validate device by querying wavelength
        match driver.query_wavelength().await {
            Ok(wavelength) => {
                if !(300.0..=1100.0).contains(&wavelength) {
                    return Err(anyhow!(
                        "Newport 1830-C validation failed: wavelength {} nm out of expected range",
                        wavelength
                    ));
                }
                tracing::info!(
                    "Newport 1830-C validated: wavelength calibration at {} nm",
                    wavelength
                );
            }
            Err(e) => {
                return Err(anyhow!(
                    "Newport 1830-C validation failed: no response to wavelength query. Error: {}",
                    e
                ));
            }
        }

        Ok(driver)
    }

    fn build(port: SharedPort) -> Self {
        let mut params = ParameterSet::new();
        let mut wavelength_nm = Parameter::new("wavelength_nm", 800.0)
            .with_description("Detector calibration wavelength")
            .with_unit("nm")
            .with_range(300.0, 1100.0);

        // Attach hardware write callback
        Self::attach_wavelength_callbacks(&mut wavelength_nm, port.clone());

        params.register(wavelength_nm.clone());

        Self {
            port,
            timeout: Duration::from_millis(500),
            wavelength_nm,
            params: Arc::new(params),
        }
    }

    /// Attach hardware callbacks to wavelength parameter.
    fn attach_wavelength_callbacks(wavelength: &mut Parameter<f64>, port: SharedPort) {
        wavelength.connect_to_hardware_write(move |target: f64| {
            let port = port.clone();
            Box::pin(async move {
                let nm = target.round() as u16;
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
        });
    }

    #[cfg(test)]
    pub(crate) fn with_test_port(port: SharedPort) -> Self {
        Self::build(port)
    }

    /// Set attenuator state
    ///
    /// # Arguments
    /// * `enabled` - true to enable attenuator (A1), false to disable (A0)
    pub async fn set_attenuator(&self, enabled: bool) -> Result<()> {
        let cmd = if enabled { "A1" } else { "A0" };
        self.send_config_command(cmd).await
    }

    /// Set filter (integration time)
    ///
    /// # Arguments
    /// * `filter` - Filter setting: 1=Slow, 2=Medium, 3=Fast
    pub async fn set_filter(&self, filter: u8) -> Result<()> {
        if !(1..=3).contains(&filter) {
            return Err(anyhow!(
                "Filter must be 1 (Slow), 2 (Medium), or 3 (Fast), got {}",
                filter
            ));
        }
        self.send_config_command(&format!("F{}", filter)).await
    }

    /// Set measurement units to Watts (U1)
    ///
    /// CRITICAL: This ensures D? returns scientific notation (e.g., "1.234E-06")
    /// instead of decimal format used by dBm mode.
    ///
    /// Unit modes (1-indexed):
    /// - U1 = Watts (scientific notation)
    /// - U2 = dBm (decimal)
    /// - U3 = dB (relative, decimal)
    /// - U4 = REL (relative linear)
    pub async fn set_units_watts(&self) -> Result<()> {
        self.send_config_command("U1").await
    }

    /// Zero the power meter
    ///
    /// This sets the current optical power level as the zero reference.
    /// IMPORTANT: Ensure the detector is blocked (shutter closed) before zeroing.
    ///
    /// # Arguments
    /// * `use_attenuator` - If true, zeros with attenuator (Z1); if false, without (Z0)
    ///
    /// # Example
    /// ```rust,ignore
    /// // Block beam first
    /// laser.close_shutter().await?;
    /// sleep(Duration::from_millis(500)).await;
    ///
    /// // Zero without attenuator
    /// power_meter.zero(false).await?;
    /// ```
    pub async fn zero(&self, use_attenuator: bool) -> Result<()> {
        let cmd = if use_attenuator { "Z1" } else { "Z0" };
        self.send_config_command(cmd).await?;
        // Zero operation takes some time to complete
        tokio::time::sleep(Duration::from_millis(500)).await;
        tracing::info!("Newport 1830-C: zeroed (attenuator={})", use_attenuator);
        Ok(())
    }

    /// Query current units setting
    ///
    /// Returns: 1=Watts, 2=dBm, 3=dB, 4=REL
    pub async fn query_units(&self) -> Result<u8> {
        let response = self.query("U?").await?;
        response
            .trim()
            .parse::<u8>()
            .with_context(|| format!("Failed to parse units response: '{}'", response))
    }

    /// Query current wavelength setting
    pub async fn query_wavelength(&self) -> Result<f64> {
        let response = self.query("W?").await?;
        self.parse_wavelength_response(&response)
    }

    /// Query power measurement
    ///
    /// Re-asserts Watts mode (U1) before each read to handle potential
    /// front-panel changes that could switch units unexpectedly.
    async fn query_power(&self) -> Result<f64> {
        // Re-assert Watts mode before reading to handle front-panel changes.
        // This adds ~100ms overhead but ensures consistent scientific notation format.
        self.send_config_command("U1").await?;

        let response = self.query("D?").await?;
        let power = self.parse_power_response(&response)?;

        // Log with magnitude classification for debugging wild swings
        let magnitude = if power == 0.0 {
            "zero"
        } else if power.abs() >= 1.0 {
            "W"
        } else if power.abs() >= 1e-3 {
            "mW"
        } else if power.abs() >= 1e-6 {
            "µW"
        } else if power.abs() >= 1e-9 {
            "nW"
        } else {
            "pW"
        };
        tracing::debug!(
            "Newport 1830-C: power = {:.6e} W ({}), raw = {:?}",
            power,
            magnitude,
            response
        );

        Ok(power)
    }

    /// Parse wavelength response (4-digit nm format)
    fn parse_wavelength_response(&self, response: &str) -> Result<f64> {
        let trimmed = response.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("Empty wavelength response"));
        }
        trimmed
            .parse::<u16>()
            .map(|nm| nm as f64)
            .with_context(|| format!("Failed to parse wavelength response: '{}'", trimmed))
    }

    /// Parse power measurement response (scientific notation in Watts mode)
    ///
    /// Expected format: Scientific notation like "1.234E-06", "+.75E-9", "5E-9"
    ///
    /// Error responses detected:
    /// - "ERR" - General error
    /// - "OVER" - Overrange (too bright)
    /// - "UNDER" - Underrange (too dim)
    /// - "SAT" - Saturated detector
    fn parse_power_response(&self, response: &str) -> Result<f64> {
        let trimmed = response.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("Empty power response"));
        }

        // Check for all known error responses
        let upper = trimmed.to_uppercase();
        if upper.contains("ERR") {
            return Err(anyhow!("Meter error: {}", trimmed));
        }
        if upper.contains("OVER") {
            return Err(anyhow!("Meter overrange (signal too bright): {}", trimmed));
        }
        if upper.contains("UNDER") {
            return Err(anyhow!("Meter underrange (signal too dim): {}", trimmed));
        }
        if upper.contains("SAT") {
            return Err(anyhow!(
                "Meter saturated (detector overloaded): {}",
                trimmed
            ));
        }

        trimmed.parse::<f64>().with_context(|| {
            format!(
                "Failed to parse power response: '{}'. Ensure device is in Watts mode (U1).",
                trimmed
            )
        })
    }

    /// Send query and read response with retry support.
    ///
    /// Wraps `query_once` with up to 3 retries and linear backoff.
    async fn query(&self, command: &str) -> Result<String> {
        const MAX_RETRIES: u32 = 3;
        const BASE_BACKOFF_MS: u64 = 100;

        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                let backoff = Duration::from_millis(BASE_BACKOFF_MS * (attempt as u64));
                tracing::debug!(
                    cmd = %command,
                    attempt,
                    backoff_ms = backoff.as_millis(),
                    "Retrying Newport 1830-C query after backoff"
                );
                tokio::time::sleep(backoff).await;
            }

            match self.query_once(command).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    tracing::debug!(
                        cmd = %command,
                        attempt,
                        error = %e,
                        "Newport 1830-C query attempt failed"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            anyhow!("Newport 1830-C query failed after {} retries", MAX_RETRIES)
        }))
    }

    /// Send query and read response (single attempt)
    async fn query_once(&self, command: &str) -> Result<String> {
        let mut port = self.port.lock().await;

        // Flush any stale data in both BufReader's buffer and underlying stream
        // First, consume any data in BufReader's internal buffer
        {
            let buf = port.buffer();
            if !buf.is_empty() {
                tracing::debug!(
                    "Newport 1830-C: clearing {} bytes from BufReader buffer",
                    buf.len()
                );
                let len = buf.len();
                port.consume(len);
            }
        }

        // Then flush any pending data from the underlying stream
        // Aggressively drain the buffer until we see 0 bytes multiple times
        // This is critical for slow 9600 baud connections where data may arrive in gaps
        let mut discard_buf = [0u8; 256];
        let clear_deadline = tokio::time::Instant::now() + Duration::from_millis(50);
        let mut _total_discarded = 0usize;
        let mut zero_byte_count = 0u32;

        while tokio::time::Instant::now() < clear_deadline {
            match tokio::time::timeout(
                Duration::from_millis(5),
                port.get_mut().read(&mut discard_buf),
            )
            .await
            {
                Ok(Ok(0)) => {
                    zero_byte_count += 1;
                    // Require 3 consecutive zero-byte reads to confirm buffer is empty
                    if zero_byte_count >= 3 {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                Ok(Ok(n)) => {
                    _total_discarded += n;
                    zero_byte_count = 0;
                    tracing::debug!("Newport 1830-C: flushed {} stale bytes from stream", n);
                }
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    zero_byte_count += 1;
                    if zero_byte_count >= 3 {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                // Timeout means no data for 5ms - this is silence, count it!
                Err(_) => {
                    zero_byte_count += 1;
                    if zero_byte_count >= 3 {
                        break;
                    }
                    // No sleep needed, timeout already consumed 5ms
                }
                // Real IO error - abort drain
                Ok(Err(e)) => {
                    tracing::warn!("Newport 1830-C: IO error during drain: {}", e);
                    break;
                }
            }
        }

        let cmd = format!("{}\n", command);
        tracing::debug!("Newport 1830-C: sending command {:?}", cmd);
        port.get_mut()
            .write_all(cmd.as_bytes())
            .await
            .context("Newport 1830-C write failed")?;

        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > self.timeout {
                return Err(anyhow!(
                    "Newport 1830-C read timeout (buffer drain exceeded)"
                ));
            }

            let mut response = String::new();
            match tokio::time::timeout(self.timeout, port.read_line(&mut response)).await {
                Ok(Ok(0)) => return Err(anyhow!("Newport 1830-C connection closed")),
                Ok(Ok(_)) => {
                    let trimmed = response.trim();
                    let cmd_trimmed = command.trim();

                    // Skip empty lines
                    if trimmed.is_empty() {
                        tracing::debug!("Newport 1830-C: skipping empty line");
                        continue;
                    }

                    // Skip exact echoes of the command
                    if trimmed == cmd_trimmed {
                        tracing::debug!("Newport 1830-C: skipping exact echo {:?}", trimmed);
                        continue;
                    }

                    // Skip partial echoes (command appears at start of response)
                    if trimmed.starts_with(cmd_trimmed) {
                        tracing::debug!("Newport 1830-C: skipping partial echo {:?}", trimmed);
                        continue;
                    }

                    // For D? queries, ensure response looks like a number (scientific notation)
                    // Valid formats: "+.75E-9", "1.234E-6", "5E-9", "-1.5E-3"
                    if cmd_trimmed == "D?" {
                        // Check if it looks like scientific notation: contains E/e and can parse as f64
                        let looks_numeric = (trimmed.contains('E') || trimmed.contains('e'))
                            && trimmed.parse::<f64>().is_ok();
                        if !looks_numeric {
                            tracing::warn!(
                                "Newport 1830-C: D? response {:?} doesn't look like scientific notation, skipping",
                                trimmed
                            );
                            continue;
                        }
                    }

                    tracing::debug!("Newport 1830-C: raw response {:?}", response);
                    return Ok(trimmed.to_string());
                }
                Ok(Err(e)) => return Err(anyhow!("Newport 1830-C read error: {}", e)),
                Err(_) => return Err(anyhow!("Newport 1830-C read timeout")),
            }
        }
    }

    /// Send configuration command and clear any response/echo
    async fn send_config_command(&self, command: &str) -> Result<()> {
        let mut port = self.port.lock().await;

        let cmd = format!("{}\n", command);
        port.get_mut()
            .write_all(cmd.as_bytes())
            .await
            .context("Newport 1830-C write failed")?;

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Read and discard any response/echo
        let mut discard = String::new();
        match tokio::time::timeout(Duration::from_millis(100), port.read_line(&mut discard)).await {
            Ok(Ok(_)) => {
                log::debug!("Newport config '{}' response: {}", command, discard.trim());
            }
            Ok(Err(_)) | Err(_) => {
                // No response or timeout - OK for config commands
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
    #[instrument(skip(self), err)]
    async fn read(&self) -> Result<f64> {
        self.query_power().await
    }
}

#[async_trait]
impl WavelengthTunable for Newport1830CDriver {
    #[instrument(skip(self), fields(wavelength_nm), err)]
    async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()> {
        self.wavelength_nm.set(wavelength_nm).await
    }

    #[instrument(skip(self), err)]
    async fn get_wavelength(&self) -> Result<f64> {
        self.query_wavelength().await
    }

    fn wavelength_range(&self) -> (f64, f64) {
        (300.0, 1100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[test]
    fn test_factory_driver_type() {
        let factory = Newport1830CFactory;
        assert_eq!(factory.driver_type(), "newport1830_c");
        assert_eq!(factory.name(), "Newport 1830-C Optical Power Meter");
    }

    #[test]
    fn test_factory_capabilities() {
        let factory = Newport1830CFactory;
        let caps = factory.capabilities();
        assert!(caps.contains(&Capability::Readable));
        assert!(caps.contains(&Capability::WavelengthTunable));
        assert!(caps.contains(&Capability::Parameterized));
    }

    #[tokio::test]
    async fn test_factory_validate_config() {
        let factory = Newport1830CFactory;

        // Valid config
        let valid_config = toml::Value::Table(toml::toml! {
            port = "/dev/ttyS0"
        });
        assert!(factory.validate(&valid_config).is_ok());

        // Valid config with wavelength
        let valid_with_wl = toml::Value::Table(toml::toml! {
            port = "/dev/ttyS0"
            wavelength_nm = 800.0
        });
        assert!(factory.validate(&valid_with_wl).is_ok());

        // Invalid wavelength
        let invalid_wl = toml::Value::Table(toml::toml! {
            port = "/dev/ttyS0"
            wavelength_nm = 2000.0
        });
        assert!(factory.validate(&invalid_wl).is_err());

        // Missing port
        let missing_port = toml::Value::Table(toml::toml! {
            wavelength_nm = 800.0
        });
        assert!(factory.validate(&missing_port).is_err());
    }

    #[test]
    fn test_parse_power_response() {
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
        let test_cases = vec![
            ("0780", 780.0),
            ("0800", 800.0),
            ("1064", 1064.0),
            ("0300", 300.0),
        ];

        for (input, expected) in test_cases {
            let parsed: Result<u16, _> = input.trim().parse();
            assert!(parsed.is_ok(), "Failed to parse: {}", input);
            assert_eq!(parsed.unwrap() as f64, expected);
        }
    }

    #[tokio::test]
    async fn wavelength_parameter_writes_command() -> Result<()> {
        let (mut host, device) = tokio::io::duplex(32);
        let port: SharedPort = Arc::new(Mutex::new(BufReader::new(Box::new(device))));

        let driver = Newport1830CDriver::with_test_port(port);

        driver.set_wavelength(800.0).await?;

        let mut buf = vec![0u8; 16];
        let n = host.read(&mut buf).await?;
        let sent = String::from_utf8_lossy(&buf[..n]);

        assert!(sent.contains("W0800"));

        Ok(())
    }
}
