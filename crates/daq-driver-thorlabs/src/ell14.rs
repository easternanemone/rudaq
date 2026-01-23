//! Thorlabs Elliptec ELL14 Rotation Mount Driver
//!
//! Protocol: RS-485 multidrop bus, 9600 baud, ASCII encoded
//! Reference: ELLx modules protocol manual Issue 10
//!
//! # Usage
//!
//! ```rust,ignore
//! use daq_driver_thorlabs::Ell14Factory;
//! use daq_core::driver::DriverFactory;
//!
//! // Register the factory
//! registry.register_factory(Box::new(Ell14Factory));
//!
//! // Create via config
//! let config = toml::toml! {
//!     port = "/dev/ttyUSB1"
//!     address = "2"
//! };
//! let components = factory.build(config.into()).await?;
//! ```

use crate::shared_ports::{get_or_open_port, get_or_open_port_with_timeout, SharedPort};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use daq_core::capabilities::{Movable, Parameterized};
use daq_core::driver::{Capability, DeviceComponents, DriverFactory};
use daq_core::error::DaqError;
use daq_core::observable::ParameterSet;
use daq_core::parameter::Parameter;
use futures::future::BoxFuture;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::instrument;

// =============================================================================
// Ell14Factory - DriverFactory implementation
// =============================================================================

/// Configuration for ELL14 driver
#[derive(Debug, Clone, Deserialize)]
pub struct Ell14Config {
    /// Serial port path (e.g., "/dev/ttyUSB1")
    pub port: String,
    /// Device address on RS-485 bus (0-9, A-F)
    pub address: String,
    /// Optional custom calibration (pulses per degree)
    #[serde(default)]
    pub pulses_per_degree: Option<f64>,
    /// Optional port timeout in milliseconds (default: 500)
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Factory for creating ELL14 driver instances.
pub struct Ell14Factory;

/// Static capabilities for ELL14
static ELL14_CAPABILITIES: &[Capability] = &[Capability::Movable, Capability::Parameterized];

impl DriverFactory for Ell14Factory {
    fn driver_type(&self) -> &'static str {
        "ell14"
    }

    fn name(&self) -> &'static str {
        "Thorlabs ELL14 Rotation Mount"
    }

    fn capabilities(&self) -> &'static [Capability] {
        ELL14_CAPABILITIES
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        let _: Ell14Config = config.clone().try_into()?;
        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let cfg: Ell14Config = config.try_into().context("Invalid ELL14 config")?;

            // Get or create shared port for this path with optional custom timeout
            let port = if let Some(timeout_ms) = cfg.timeout_ms {
                get_or_open_port_with_timeout(&cfg.port, Duration::from_millis(timeout_ms)).await?
            } else {
                get_or_open_port(&cfg.port).await?
            };

            // Create driver with calibration
            let driver = if let Some(ppd) = cfg.pulses_per_degree {
                Arc::new(Ell14Driver::with_calibration(port, &cfg.address, ppd))
            } else {
                // Query device for calibration
                Arc::new(Ell14Driver::with_shared_port_calibrated(port, &cfg.address).await?)
            };

            Ok(DeviceComponents {
                movable: Some(driver.clone()),
                parameterized: Some(driver),
                ..Default::default()
            })
        })
    }
}

// =============================================================================
// ELL14 Status Codes
// =============================================================================

/// ELL14 status/error codes returned in GS responses
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Ell14StatusCode {
    Ok = 0x00,
    CommunicationTimeout = 0x01,
    MechanicalTimeout = 0x02,
    CommandError = 0x03,
    ValueOutOfRange = 0x04,
    ModuleIsolated = 0x05,
    ModuleOutOfIsolation = 0x06,
    InitializationError = 0x07,
    ThermalError = 0x08,
    Busy = 0x09,
    SensorError = 0x0A,
    MotorError = 0x0B,
    OutOfRange = 0x0C,
    OverCurrentError = 0x0D,
    Unknown = 0xFF,
}

impl Ell14StatusCode {
    pub fn from_hex(hex: &str) -> Self {
        match u8::from_str_radix(hex, 16) {
            Ok(code) => Self::from_u8(code),
            Err(_) => Self::Unknown,
        }
    }

    pub fn from_u8(code: u8) -> Self {
        match code {
            0x00 => Self::Ok,
            0x01 => Self::CommunicationTimeout,
            0x02 => Self::MechanicalTimeout,
            0x03 => Self::CommandError,
            0x04 => Self::ValueOutOfRange,
            0x05 => Self::ModuleIsolated,
            0x06 => Self::ModuleOutOfIsolation,
            0x07 => Self::InitializationError,
            0x08 => Self::ThermalError,
            0x09 => Self::Busy,
            0x0A => Self::SensorError,
            0x0B => Self::MotorError,
            0x0C => Self::OutOfRange,
            0x0D => Self::OverCurrentError,
            _ => Self::Unknown,
        }
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Ok => "No error",
            Self::CommunicationTimeout => "Communication timeout",
            Self::MechanicalTimeout => "Mechanical timeout",
            Self::CommandError => "Command error",
            Self::ValueOutOfRange => "Value out of range",
            Self::ModuleIsolated => "Module isolated",
            Self::ModuleOutOfIsolation => "Module out of isolation",
            Self::InitializationError => "Initialization error",
            Self::ThermalError => "Thermal error",
            Self::Busy => "Busy",
            Self::SensorError => "Sensor error",
            Self::MotorError => "Motor error",
            Self::OutOfRange => "Position out of range",
            Self::OverCurrentError => "Over current error",
            Self::Unknown => "Unknown error",
        }
    }
}

// =============================================================================
// Ell14Driver
// =============================================================================

/// Driver for Thorlabs Elliptec ELL14 Rotation Mount.
///
/// Implements the Movable capability trait for controlling rotation.
/// Multiple drivers can share a single serial port via the shared_ports module.
#[derive(Clone)]
pub struct Ell14Driver {
    port: SharedPort,
    address: String,
    pulses_per_degree: f64,
    position_deg: Parameter<f64>,
    params: Arc<ParameterSet>,
}

impl Ell14Driver {
    /// Default calibration: 143360 pulses / 360 degrees = 398.2222 pulses/degree
    pub const DEFAULT_PULSES_PER_DEGREE: f64 = 398.22222222;

    /// Create driver with default calibration.
    pub fn with_shared_port(port: SharedPort, address: &str) -> Self {
        Self::with_calibration(port, address, Self::DEFAULT_PULSES_PER_DEGREE)
    }

    /// Create driver with custom calibration.
    pub fn with_calibration(port: SharedPort, address: &str, pulses_per_degree: f64) -> Self {
        let mut params = ParameterSet::new();

        let mut position_deg = Parameter::new("position", 0.0)
            .with_description("Rotation position")
            .with_unit("deg")
            .with_range(0.0, 360.0);

        // Attach hardware callbacks
        Self::attach_position_callbacks(
            &mut position_deg,
            port.clone(),
            address.to_string(),
            pulses_per_degree,
        );

        params.register(position_deg.clone());

        Self {
            port,
            address: address.to_string(),
            pulses_per_degree,
            position_deg,
            params: Arc::new(params),
        }
    }

    /// Attach hardware read/write callbacks to position parameter.
    fn attach_position_callbacks(
        position: &mut Parameter<f64>,
        port: SharedPort,
        address: String,
        pulses_per_degree: f64,
    ) {
        // Connect hardware write callback
        let port_for_write = port.clone();
        let addr_for_write = address.clone();
        let ppd_for_write = pulses_per_degree;

        position.connect_to_hardware_write(move |target: f64| {
            let port = port_for_write.clone();
            let addr = addr_for_write.clone();
            let ppd = ppd_for_write;
            Box::pin(async move {
                // Convert degrees to pulses and send move absolute command
                let pulses = (target * ppd).round() as u32;
                let cmd = format!("{}ma{:08X}", addr, pulses);
                let expected_prefix = format!("{}PO", addr);

                let mut guard = port.lock().await;

                // Clear any pending data first (leftover from other devices on RS-485 bus)
                let mut discard = [0u8; 64];
                let clear_deadline = tokio::time::Instant::now() + Duration::from_millis(10);
                while tokio::time::Instant::now() < clear_deadline {
                    match tokio::time::timeout(Duration::from_millis(5), guard.read(&mut discard))
                        .await
                    {
                        Ok(Ok(0)) => break,
                        Ok(Ok(n)) => {
                            tracing::trace!(
                                discarded = n,
                                "Cleared pending data before ELL14 move_abs"
                            );
                        }
                        Ok(Err(_)) | Err(_) => break,
                    }
                }

                guard.write_all(cmd.as_bytes()).await?;
                guard.flush().await?;

                // Read and consume the PO response to avoid polluting subsequent commands
                // Response format: <addr>PO<8 hex digits>\r\n (e.g., "2PO00000000\r\n" = 13 chars)
                let mut response = Vec::with_capacity(32);
                let mut buf = [0u8; 32];
                let deadline = tokio::time::Instant::now() + Duration::from_millis(500);

                loop {
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    if tokio::time::Instant::now() > deadline {
                        break;
                    }
                    match guard.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            response.extend_from_slice(&buf[..n]);
                            // Check for complete PO response: must start with our address+PO
                            // and end with CR/LF, with full position data
                            let resp_str = String::from_utf8_lossy(&response);
                            if resp_str.contains(&expected_prefix)
                                && (response.contains(&b'\r') || response.contains(&b'\n'))
                                && resp_str.len() >= 12
                            {
                                // Got complete response
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => break,
                        Err(_) => break,
                    }
                }

                tracing::trace!(
                    address = %addr,
                    cmd = %cmd,
                    response = %String::from_utf8_lossy(&response),
                    "ELL14 move_abs response consumed"
                );

                Ok(())
            })
        });

        // Connect hardware read callback
        let port_for_read = port;
        let addr_for_read = address;
        let ppd_for_read = pulses_per_degree;

        position.connect_to_hardware_read(move || {
            let port = port_for_read.clone();
            let addr = addr_for_read.clone();
            let ppd = ppd_for_read;
            Box::pin(async move {
                let cmd = format!("{}gp", addr);

                let mut guard = port.lock().await;
                guard.write_all(cmd.as_bytes()).await?;
                guard.flush().await?;

                // Read response
                let mut buf = [0u8; 32];
                tokio::time::sleep(Duration::from_millis(50)).await;
                let n = guard.read(&mut buf).await?;

                let resp = String::from_utf8_lossy(&buf[..n]);
                // Parse position from response (format: "XPOxxxxxxxx")
                if let Some(idx) = resp.find("PO") {
                    let hex_str = &resp[idx + 2..].trim();
                    if let Some(hex) = hex_str.get(..8) {
                        if let Ok(pulses) = u32::from_str_radix(hex, 16) {
                            return Ok(pulses as f64 / ppd);
                        }
                    }
                }
                Err(DaqError::Instrument(
                    "Failed to parse position response".to_string(),
                ))
            })
        });
    }

    /// Create driver with calibration queried from device.
    pub async fn with_shared_port_calibrated(port: SharedPort, address: &str) -> Result<Self> {
        // Query device info to get calibration
        let cmd = format!("{}in", address);

        let pulses_per_degree = {
            let mut guard = port.lock().await;

            // Clear any pending data in the receive buffer first
            let mut discard = [0u8; 64];
            let clear_deadline = tokio::time::Instant::now() + Duration::from_millis(10);
            while tokio::time::Instant::now() < clear_deadline {
                match tokio::time::timeout(Duration::from_millis(5), guard.read(&mut discard)).await
                {
                    Ok(Ok(0)) => break,
                    Ok(Ok(n)) => {
                        tracing::trace!(discarded = n, "Cleared pending data before IN query");
                    }
                    Ok(Err(_)) | Err(_) => break,
                }
            }

            guard.write_all(cmd.as_bytes()).await?;
            guard.flush().await?;

            // Read complete response with proper timeout
            // Response format: {addr}IN{32 chars}\r\n (total ~35 bytes)
            let mut response = Vec::with_capacity(64);
            let mut buf = [0u8; 64];
            let deadline = tokio::time::Instant::now() + Duration::from_millis(300);

            loop {
                tokio::time::sleep(Duration::from_millis(30)).await;
                if tokio::time::Instant::now() > deadline {
                    break;
                }
                match guard.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        response.extend_from_slice(&buf[..n]);
                        // Check for complete response (ends with CR/LF)
                        if response.contains(&b'\r') || response.contains(&b'\n') {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                    Err(e) if e.kind() == std::io::ErrorKind::TimedOut => break,
                    Err(_) => break,
                }
            }

            let resp = String::from_utf8_lossy(&response);
            tracing::debug!(response = %resp, "Device info response");

            // Parse pulses_per_unit from IN response
            // Format: {addr}IN{type}{serial}{year}{fw}{travel}{pulses_per_unit}
            // Expected: "2IN0E1140051720231701016800023000" (33 chars total)
            // addr(1) + IN(2) + type(2) + serial(8) + year(4) + fw(4) + travel(4) + ppu(8) = 33
            // The last 8 chars are pulses_per_unit in hex (00023000 = 143360 pulses)
            let trimmed = resp.trim();
            if trimmed.len() >= 33 {
                // Full response: address(1) + IN(2) + data(31) = 34 minimum
                if let Some(ppu_hex) = trimmed.get(trimmed.len().saturating_sub(8)..) {
                    if let Ok(ppu) = u32::from_str_radix(ppu_hex, 16) {
                        let ppd = ppu as f64 / 360.0;
                        // Sanity check: pulses_per_degree should be ~398 for ELL14
                        if ppd > 100.0 && ppd < 1000.0 {
                            ppd
                        } else {
                            tracing::warn!(
                                address = %address,
                                parsed_ppd = ppd,
                                raw_ppu_hex = ppu_hex,
                                "Invalid pulses_per_degree parsed, using default"
                            );
                            Self::DEFAULT_PULSES_PER_DEGREE
                        }
                    } else {
                        Self::DEFAULT_PULSES_PER_DEGREE
                    }
                } else {
                    Self::DEFAULT_PULSES_PER_DEGREE
                }
            } else {
                tracing::warn!(
                    address = %address,
                    response_len = trimmed.len(),
                    response = %trimmed,
                    "Incomplete IN response, using default calibration"
                );
                Self::DEFAULT_PULSES_PER_DEGREE
            }
        };

        tracing::info!(
            address = %address,
            pulses_per_degree = pulses_per_degree,
            "Calibrated ELL14 driver"
        );

        Ok(Self::with_calibration(port, address, pulses_per_degree))
    }

    /// Get the device address.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Get the calibration value.
    pub fn pulses_per_degree(&self) -> f64 {
        self.pulses_per_degree
    }

    /// Send a command and read response.
    ///
    /// ELL14 responses are terminated with CR LF. At 9600 baud, a typical 17-byte
    /// response takes ~18ms to transmit. We use multiple short reads to collect
    /// the complete response.
    ///
    /// On RS-485 multidrop bus, multiple devices share the same port. We clear
    /// any pending data before sending and validate the response address prefix.
    #[instrument(skip(self))]
    async fn transaction(&self, cmd: &str) -> Result<String> {
        let full_cmd = format!("{}{}", self.address, cmd);
        let expected_prefix = &self.address;

        let mut guard = self.port.lock().await;

        // Clear any pending data in the receive buffer (leftover from other devices)
        // Use a very short timeout to avoid blocking if no data is pending
        let mut discard = [0u8; 64];
        let clear_deadline = tokio::time::Instant::now() + Duration::from_millis(10);
        while tokio::time::Instant::now() < clear_deadline {
            match tokio::time::timeout(Duration::from_millis(5), guard.read(&mut discard)).await {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => {
                    tracing::trace!(discarded = n, "Cleared pending data before ELL14 command");
                }
                Ok(Err(_)) | Err(_) => break,
            }
        }

        // Send command
        guard.write_all(full_cmd.as_bytes()).await?;
        guard.flush().await?;

        // Collect response with multiple reads until we see our address prefix + CR/LF
        let mut response = Vec::with_capacity(64);
        let mut buf = [0u8; 64];
        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);

        loop {
            // Wait a bit for data to arrive
            tokio::time::sleep(Duration::from_millis(30)).await;

            // Check timeout
            if tokio::time::Instant::now() > deadline {
                break;
            }

            // Try to read available data
            match guard.read(&mut buf).await {
                Ok(0) => break, // EOF
                Ok(n) => {
                    response.extend_from_slice(&buf[..n]);
                    // Check for complete response: starts with our address and ends with CR/LF
                    let resp_str = String::from_utf8_lossy(&response);
                    if resp_str.starts_with(expected_prefix)
                        && (response.contains(&b'\r') || response.contains(&b'\n'))
                    {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No data yet, continue waiting
                    continue;
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    break;
                }
                Err(e) => return Err(e.into()),
            }
        }

        let resp = String::from_utf8_lossy(&response).to_string();
        tracing::debug!(cmd = %full_cmd, response = %resp, "ELL14 transaction");
        Ok(resp)
    }

    /// Home the device to mechanical zero.
    pub async fn home(&self) -> Result<()> {
        let _ = self.transaction("ho").await?;
        Ok(())
    }

    /// Stop any motion.
    pub async fn stop(&self) -> Result<()> {
        let _ = self.transaction("st").await?;
        Ok(())
    }

    /// Get device status.
    pub async fn get_status(&self) -> Result<Ell14StatusCode> {
        let resp = self.transaction("gs").await?;
        if let Some(idx) = resp.find("GS") {
            let hex_str = resp[idx + 2..].trim();
            if let Some(hex) = hex_str.get(..2) {
                let status = Ell14StatusCode::from_hex(hex);
                tracing::trace!(address = %self.address, response = %resp, hex, ?status, "ELL14 get_status parsed");
                return Ok(status);
            }
        }
        tracing::debug!(address = %self.address, response = %resp, "ELL14 get_status failed to parse GS response");
        Ok(Ell14StatusCode::Unknown)
    }
}

impl Parameterized for Ell14Driver {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl Movable for Ell14Driver {
    #[instrument(skip(self), fields(address = %self.address))]
    async fn move_abs(&self, position_deg: f64) -> Result<()> {
        self.position_deg.set(position_deg).await
    }

    #[instrument(skip(self), fields(address = %self.address))]
    async fn move_rel(&self, distance_deg: f64) -> Result<()> {
        let pulses = (distance_deg * self.pulses_per_degree).round() as i32;
        let hex_pulses = format!("{:08X}", pulses as u32);
        let cmd = format!("mr{}", hex_pulses);
        let _ = self.transaction(&cmd).await?;
        Ok(())
    }

    #[instrument(skip(self), fields(address = %self.address))]
    async fn position(&self) -> Result<f64> {
        let resp = self.transaction("gp").await?;
        if let Some(idx) = resp.find("PO") {
            let hex_str = &resp[idx + 2..].trim();
            if let Some(hex) = hex_str.get(..8) {
                if let Ok(pulses) = u32::from_str_radix(hex, 16) {
                    return Ok(pulses as f64 / self.pulses_per_degree);
                }
            }
        }
        Err(anyhow!("Failed to parse position response: {}", resp))
    }

    #[instrument(skip(self), fields(address = %self.address))]
    async fn wait_settled(&self) -> Result<()> {
        let timeout = Duration::from_secs(10);
        let start = std::time::Instant::now();
        let mut consecutive_settled = 0;

        loop {
            if start.elapsed() > timeout {
                tracing::warn!(
                    address = %self.address,
                    consecutive_settled,
                    "ELL14 wait_settled timed out"
                );
                return Err(anyhow!("ELL14 wait_settled timed out after 10 seconds"));
            }

            match self.get_status().await {
                Ok(status) if status.is_ok() => {
                    consecutive_settled += 1;
                    tracing::trace!(
                        address = %self.address,
                        consecutive_settled,
                        "ELL14 status OK"
                    );
                    if consecutive_settled >= 3 {
                        return Ok(());
                    }
                }
                Ok(status) => {
                    tracing::debug!(
                        address = %self.address,
                        ?status,
                        "ELL14 status not OK, resetting counter"
                    );
                    consecutive_settled = 0;
                }
                Err(e) => {
                    tracing::debug!(
                        address = %self.address,
                        error = %e,
                        "ELL14 get_status error (device may be in motion)"
                    );
                    // Device may not respond during motion
                }
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_code_parsing() {
        assert!(Ell14StatusCode::from_hex("00").is_ok());
        assert_eq!(
            Ell14StatusCode::from_hex("02"),
            Ell14StatusCode::MechanicalTimeout
        );
        assert_eq!(Ell14StatusCode::from_hex("FF"), Ell14StatusCode::Unknown);
    }

    #[test]
    fn test_factory_driver_type() {
        let factory = Ell14Factory;
        assert_eq!(factory.driver_type(), "ell14");
        assert_eq!(factory.name(), "Thorlabs ELL14 Rotation Mount");
    }

    #[test]
    fn test_factory_capabilities() {
        let factory = Ell14Factory;
        let caps = factory.capabilities();
        assert!(caps.contains(&Capability::Movable));
        assert!(caps.contains(&Capability::Parameterized));
    }

    #[tokio::test]
    async fn test_factory_validate_config() {
        let factory = Ell14Factory;

        // Valid config
        let valid_config = toml::Value::Table(toml::toml! {
            port = "/dev/ttyUSB1"
            address = "2"
        });
        assert!(factory.validate(&valid_config).is_ok());

        // Missing port
        let invalid_config = toml::Value::Table(toml::toml! {
            address = "2"
        });
        assert!(factory.validate(&invalid_config).is_err());
    }
}
