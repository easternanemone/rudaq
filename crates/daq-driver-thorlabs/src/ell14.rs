//! Thorlabs Elliptec ELL14 Rotation Mount Driver
//!
//! Protocol: RS-485 multidrop bus, 9600 baud, ASCII encoded
//! Reference: ELLx modules protocol manual Issue 10
//!
//! # Features
//!
//! - **Velocity Control**: Set motor velocity (0-100%) for speed vs precision tradeoff
//! - **Cached Settings**: Non-blocking access to device settings via [`Ell14CachedSettings`]
//! - **RS-485 Multidrop**: Multiple devices share a single serial port via address (0-F)
//! - **Auto-calibration**: Queries pulses-per-degree from device firmware on init
//!
//! # Velocity Control
//!
//! The ELL14 supports velocity control from 0-100%. Higher velocities enable faster
//! scans at the cost of positioning precision. When using [`Ell14Driver::with_shared_port_calibrated`],
//! velocity is automatically set to maximum (100%) for fastest operation.
//!
//! ```rust,ignore
//! // Velocity is set to max during calibrated init
//! let driver = Ell14Driver::with_shared_port_calibrated(port, "2").await?;
//!
//! // Manual velocity control
//! driver.set_velocity(50).await?;  // 50% speed
//! let vel = driver.get_velocity().await?;  // Query from hardware
//! let cached = driver.cached_velocity().await;  // Fast read from cache
//! ```
//!
//! # Cached Settings
//!
//! The driver maintains a cache of device settings ([`Ell14CachedSettings`]) that can be
//! read without blocking on serial I/O. Call [`Ell14Driver::refresh_cached_settings`]
//! to update the cache from hardware.
//!
//! # Usage
//!
//! ```rust,ignore
//! use daq_driver_thorlabs::Ell14Factory;
//! use common::driver::DriverFactory;
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
use common::capabilities::{Movable, Parameterized};
use common::driver::{Capability, DeviceComponents, DriverFactory};
use common::error::DaqError;
use common::observable::ParameterSet;
use common::parameter::Parameter;
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

/// Cached device settings for quick reference without hardware queries.
///
/// This cache is populated during initialization and can be refreshed
/// periodically. Provides fast access to settings like velocity without
/// blocking on serial I/O.
#[derive(Debug, Clone, Default)]
pub struct Ell14CachedSettings {
    /// Forward velocity as percentage (0-100)
    pub velocity_percent: u8,
    /// Backward velocity as percentage (0-100)
    pub backward_velocity_percent: u8,
    /// Last known position in degrees (may be stale)
    pub last_position_deg: Option<f64>,
    /// Timestamp of last cache update (Unix millis)
    pub last_updated_ms: u64,
}

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
    /// Cached settings for quick reference without hardware queries
    cached_settings: Arc<tokio::sync::RwLock<Ell14CachedSettings>>,
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
            cached_settings: Arc::new(tokio::sync::RwLock::new(Ell14CachedSettings::default())),
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

                // Aggressive buffer draining (same as transaction_once)
                let mut discard = [0u8; 256];
                let clear_deadline = tokio::time::Instant::now() + Duration::from_millis(50);
                let mut total_discarded = 0usize;
                let mut zero_byte_count = 0u32;

                while tokio::time::Instant::now() < clear_deadline {
                    match tokio::time::timeout(Duration::from_millis(5), guard.read(&mut discard))
                        .await
                    {
                        Ok(Ok(0)) => {
                            zero_byte_count += 1;
                            if zero_byte_count >= 3 {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(2)).await;
                        }
                        Ok(Ok(n)) => {
                            total_discarded += n;
                            zero_byte_count = 0;
                        }
                        Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            zero_byte_count += 1;
                            if zero_byte_count >= 3 {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(2)).await;
                        }
                        // Timeout means silence - count it towards the 3-consecutive check
                        Err(_) => {
                            zero_byte_count += 1;
                            if zero_byte_count >= 3 {
                                break;
                            }
                        }
                        // Real IO error - abort drain
                        Ok(Err(_)) => break,
                    }
                }
                if total_discarded > 0 {
                    tracing::trace!(
                        discarded = total_discarded,
                        "Cleared pending data before ELL14 move_abs"
                    );
                }

                guard.write_all(cmd.as_bytes()).await?;
                guard.flush().await?;

                // Read and consume the PO response to avoid polluting subsequent commands
                // Use pattern scanning to handle leading garbage (RS-485 bus noise)
                let mut response = Vec::with_capacity(64);
                let mut buf = [0u8; 64];
                let deadline = tokio::time::Instant::now() + Duration::from_millis(500);

                // Initial wait for device to start responding
                tokio::time::sleep(Duration::from_millis(15)).await;

                loop {
                    if tokio::time::Instant::now() > deadline {
                        break;
                    }
                    // Use timeout to prevent indefinite blocking on serial read
                    match tokio::time::timeout(Duration::from_millis(50), guard.read(&mut buf))
                        .await
                    {
                        Ok(Ok(0)) => break,
                        Ok(Ok(n)) => {
                            response.extend_from_slice(&buf[..n]);
                            // Scan for PO response anywhere in buffer (handles leading garbage)
                            let resp_str = String::from_utf8_lossy(&response);
                            if let Some(start_idx) = resp_str.find(&expected_prefix) {
                                let after_prefix = &resp_str[start_idx..];
                                // PO response: {addr}PO{8 hex}\r\n - need at least 11 chars after finding prefix start
                                if after_prefix.len() >= 11
                                    && (after_prefix.contains('\r') || after_prefix.contains('\n'))
                                {
                                    break;
                                }
                            }
                        }
                        Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            continue;
                        }
                        Ok(Err(e)) if e.kind() == std::io::ErrorKind::TimedOut => {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            continue;
                        }
                        Ok(Err(_)) => break,
                        Err(_elapsed) => {
                            // tokio::time::timeout elapsed - no data available within 50ms
                            // Continue loop and re-check deadline
                            continue;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
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
                let expected_prefix = format!("{}PO", addr);

                let mut guard = port.lock().await;

                // Aggressive buffer draining
                let mut discard = [0u8; 256];
                let clear_deadline = tokio::time::Instant::now() + Duration::from_millis(50);
                let mut zero_byte_count = 0u32;

                while tokio::time::Instant::now() < clear_deadline {
                    match tokio::time::timeout(Duration::from_millis(5), guard.read(&mut discard))
                        .await
                    {
                        Ok(Ok(0)) => {
                            zero_byte_count += 1;
                            if zero_byte_count >= 3 {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(2)).await;
                        }
                        Ok(Ok(_)) => {
                            zero_byte_count = 0;
                        }
                        Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            zero_byte_count += 1;
                            if zero_byte_count >= 3 {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(2)).await;
                        }
                        // Timeout means silence - count it towards the 3-consecutive check
                        Err(_) => {
                            zero_byte_count += 1;
                            if zero_byte_count >= 3 {
                                break;
                            }
                        }
                        // Real IO error - abort drain
                        Ok(Err(_)) => break,
                    }
                }

                guard.write_all(cmd.as_bytes()).await?;
                guard.flush().await?;

                // Read response with pattern scanning
                let mut response = Vec::with_capacity(64);
                let mut buf = [0u8; 64];
                let deadline = tokio::time::Instant::now() + Duration::from_millis(500);

                tokio::time::sleep(Duration::from_millis(15)).await;

                loop {
                    if tokio::time::Instant::now() > deadline {
                        break;
                    }
                    // Use timeout to prevent indefinite blocking on serial read
                    match tokio::time::timeout(Duration::from_millis(50), guard.read(&mut buf))
                        .await
                    {
                        Ok(Ok(0)) => break,
                        Ok(Ok(n)) => {
                            response.extend_from_slice(&buf[..n]);
                            let resp_str = String::from_utf8_lossy(&response);
                            if let Some(start_idx) = resp_str.find(&expected_prefix) {
                                let after_prefix = &resp_str[start_idx..];
                                // PO response: {addr}PO{8 hex}\r\n
                                if after_prefix.len() >= 11
                                    && (after_prefix.contains('\r') || after_prefix.contains('\n'))
                                {
                                    // Extract and parse position
                                    if let Some(hex) = after_prefix.get(3..11) {
                                        if let Ok(pulses) = u32::from_str_radix(hex, 16) {
                                            return Ok((pulses as i32) as f64 / ppd);
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                        Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            continue;
                        }
                        Ok(Err(e)) if e.kind() == std::io::ErrorKind::TimedOut => {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            continue;
                        }
                        Ok(Err(_)) => break,
                        Err(_elapsed) => {
                            // tokio::time::timeout elapsed - continue and re-check deadline
                            continue;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }

                // Fallback: try to parse what we have
                let resp = String::from_utf8_lossy(&response);
                if let Some(idx) = resp.find("PO") {
                    let hex_str = &resp[idx + 2..].trim();
                    if let Some(hex) = hex_str.get(..8) {
                        if let Ok(pulses) = u32::from_str_radix(hex, 16) {
                            return Ok((pulses as i32) as f64 / ppd);
                        }
                    }
                }
                Err(DaqError::Instrument(format!(
                    "Failed to parse position response: {}",
                    resp
                )))
            })
        });
    }

    /// Create driver with calibration queried from device.
    pub async fn with_shared_port_calibrated(port: SharedPort, address: &str) -> Result<Self> {
        // Query device info to get calibration
        let cmd = format!("{}in", address);
        let expected_prefix = format!("{}IN", address.to_uppercase());

        let pulses_per_degree = {
            let mut guard = port.lock().await;

            // Aggressive buffer draining (same pattern as transaction_once)
            let mut discard = [0u8; 256];
            let clear_deadline = tokio::time::Instant::now() + Duration::from_millis(50);
            let mut total_discarded = 0usize;
            let mut zero_byte_count = 0u32;

            while tokio::time::Instant::now() < clear_deadline {
                match tokio::time::timeout(Duration::from_millis(5), guard.read(&mut discard)).await
                {
                    Ok(Ok(0)) => {
                        zero_byte_count += 1;
                        if zero_byte_count >= 3 {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(2)).await;
                    }
                    Ok(Ok(n)) => {
                        total_discarded += n;
                        zero_byte_count = 0;
                    }
                    Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        zero_byte_count += 1;
                        if zero_byte_count >= 3 {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(2)).await;
                    }
                    // Timeout means silence - count it towards the 3-consecutive check
                    Err(_) => {
                        zero_byte_count += 1;
                        if zero_byte_count >= 3 {
                            break;
                        }
                    }
                    // Real IO error - abort drain
                    Ok(Err(_)) => break,
                }
            }
            if total_discarded > 0 {
                tracing::trace!(
                    discarded = total_discarded,
                    "Cleared pending data before IN query"
                );
            }

            guard.write_all(cmd.as_bytes()).await?;
            guard.flush().await?;

            // Initial wait for device to respond
            tokio::time::sleep(Duration::from_millis(15)).await;

            // Read complete response with pattern scanning
            // Response format: {addr}IN{32 chars}\r\n (total ~35 bytes)
            let mut response = Vec::with_capacity(128);
            let mut buf = [0u8; 64];
            let deadline = tokio::time::Instant::now() + Duration::from_millis(500);

            loop {
                if tokio::time::Instant::now() > deadline {
                    break;
                }
                // Use timeout to prevent indefinite blocking on serial read
                match tokio::time::timeout(Duration::from_millis(50), guard.read(&mut buf)).await {
                    Ok(Ok(0)) => break,
                    Ok(Ok(n)) => {
                        response.extend_from_slice(&buf[..n]);
                        // Scan for IN response anywhere in buffer (handles leading garbage)
                        let resp_str = String::from_utf8_lossy(&response);
                        if let Some(start_idx) = resp_str.find(&expected_prefix) {
                            let after_prefix = &resp_str[start_idx..];
                            // IN response needs: {addr}IN{30 chars}\r\n - at least 33 chars
                            if after_prefix.len() >= 33
                                && (after_prefix.contains('\r') || after_prefix.contains('\n'))
                            {
                                break;
                            }
                        }
                    }
                    Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        continue;
                    }
                    Ok(Err(e)) if e.kind() == std::io::ErrorKind::TimedOut => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        continue;
                    }
                    Ok(Err(_)) => break,
                    Err(_elapsed) => {
                        // tokio::time::timeout elapsed - continue and re-check deadline
                        continue;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }

            let resp = String::from_utf8_lossy(&response);
            tracing::debug!(address = %address, response = %resp, "Device info response");

            // Parse pulses_per_unit from IN response using pattern scanning
            // Format: {addr}IN{type}{serial}{year}{fw}{travel}{pulses_per_unit}
            // Expected: "2IN0E1140051720231701016800023000" (33 chars total)
            // addr(1) + IN(2) + type(2) + serial(8) + year(4) + fw(4) + travel(4) + ppu(8) = 33
            // The last 8 chars are pulses_per_unit in hex (00023000 = 143360 pulses)
            if let Some(start_idx) = resp.find(&expected_prefix) {
                let after_prefix = &resp[start_idx..];
                // Find terminator
                let end_idx = after_prefix.find('\r').or_else(|| after_prefix.find('\n'));
                let clean_resp = match end_idx {
                    Some(idx) => &after_prefix[..idx],
                    None => after_prefix,
                };
                let trimmed = clean_resp.trim();

                if trimmed.len() >= 33 {
                    // Extract last 8 hex chars (pulses_per_unit)
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
                            tracing::warn!(
                                address = %address,
                                ppu_hex = ppu_hex,
                                "Failed to parse ppu hex, using default calibration"
                            );
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
            } else {
                tracing::warn!(
                    address = %address,
                    response = %resp,
                    "No IN prefix found in response, using default calibration"
                );
                Self::DEFAULT_PULSES_PER_DEGREE
            }
        };

        tracing::info!(
            address = %address,
            pulses_per_degree = pulses_per_degree,
            "Calibrated ELL14 driver"
        );

        let driver = Self::with_calibration(port, address, pulses_per_degree);

        // Set maximum velocity for fastest operation
        if let Err(e) = driver.set_max_velocity().await {
            tracing::warn!(
                address = %address,
                error = %e,
                "Failed to set max velocity during init, using default"
            );
        }

        // Populate cached settings
        if let Err(e) = driver.refresh_cached_settings().await {
            tracing::warn!(
                address = %address,
                error = %e,
                "Failed to refresh cached settings during init"
            );
        }

        Ok(driver)
    }

    /// Get the device address.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Get the calibration value.
    pub fn pulses_per_degree(&self) -> f64 {
        self.pulses_per_degree
    }

    /// Send a command and read response with retry support.
    ///
    /// Wraps `transaction_once` with up to 3 retries and linear backoff.
    #[instrument(skip(self))]
    async fn transaction(&self, cmd: &str) -> Result<String> {
        const MAX_RETRIES: u32 = 3;
        const BASE_BACKOFF_MS: u64 = 50;

        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                let backoff = Duration::from_millis(BASE_BACKOFF_MS * (attempt as u64));
                tracing::debug!(
                    address = %self.address,
                    cmd = %cmd,
                    attempt,
                    backoff_ms = backoff.as_millis(),
                    "Retrying ELL14 transaction after backoff"
                );
                tokio::time::sleep(backoff).await;
            }

            match self.transaction_once(cmd).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    tracing::debug!(
                        address = %self.address,
                        cmd = %cmd,
                        attempt,
                        error = %e,
                        "ELL14 transaction attempt failed"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| anyhow!("Transaction failed after {} retries", MAX_RETRIES)))
    }

    /// Send a command and read response (single attempt).
    ///
    /// ELL14 responses are terminated with CR LF. At 9600 baud, a typical 17-byte
    /// response takes ~18ms to transmit. We use multiple short reads to collect
    /// the complete response.
    ///
    /// On RS-485 multidrop bus, multiple devices share the same port. We:
    /// 1. Aggressively drain the buffer until we see 0 bytes
    /// 2. Send command
    /// 3. Scan response buffer for our address prefix (handles leading garbage)
    /// 4. Extract message between prefix and CR/LF terminator
    async fn transaction_once(&self, cmd: &str) -> Result<String> {
        let full_cmd = format!("{}{}", self.address, cmd);
        let expected_prefix = &self.address;

        let mut guard = self.port.lock().await;

        // Aggressive buffer draining: read until we get 0 bytes or hit safety limit
        // This is critical for RS-485 buses where other devices may have sent data
        let mut discard = [0u8; 256];
        let clear_deadline = tokio::time::Instant::now() + Duration::from_millis(50);
        let mut total_discarded = 0usize;
        let mut zero_byte_count = 0u32;

        while tokio::time::Instant::now() < clear_deadline {
            match tokio::time::timeout(Duration::from_millis(5), guard.read(&mut discard)).await {
                Ok(Ok(0)) => {
                    zero_byte_count += 1;
                    // Require 3 consecutive zero-byte reads to confirm buffer is empty
                    if zero_byte_count >= 3 {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                Ok(Ok(n)) => {
                    total_discarded += n;
                    zero_byte_count = 0; // Reset counter
                }
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    zero_byte_count += 1;
                    if zero_byte_count >= 3 {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                Ok(Err(_)) | Err(_) => {
                    // Timeout or other error - buffer is likely empty
                    break;
                }
            }
        }
        if total_discarded > 0 {
            tracing::trace!(
                discarded = total_discarded,
                cmd = %full_cmd,
                "Cleared pending data before ELL14 command"
            );
        }

        // Send command
        guard.write_all(full_cmd.as_bytes()).await?;
        guard.flush().await?;

        // Collect response with multiple reads
        // Use pattern scanning to handle leading garbage bytes (common on RS-485)
        let mut response = Vec::with_capacity(128);
        let mut buf = [0u8; 64];
        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);

        // First wait: give device time to start responding (10ms at 9600 baud ~= 9.6 bytes)
        tokio::time::sleep(Duration::from_millis(15)).await;

        loop {
            // Check timeout
            if tokio::time::Instant::now() > deadline {
                tracing::trace!(
                    cmd = %full_cmd,
                    response = %String::from_utf8_lossy(&response),
                    "ELL14 transaction timed out waiting for response"
                );
                break;
            }

            // Try to read available data with timeout to prevent indefinite blocking
            match tokio::time::timeout(Duration::from_millis(50), guard.read(&mut buf)).await {
                Ok(Ok(0)) => break, // EOF
                Ok(Ok(n)) => {
                    response.extend_from_slice(&buf[..n]);

                    // Scan for valid frame: look for address prefix ANYWHERE in buffer
                    // This handles leading garbage bytes from RS-485 bus noise
                    let resp_str = String::from_utf8_lossy(&response);
                    if let Some(start_idx) = resp_str.find(expected_prefix) {
                        // Found start marker, look for terminator after it
                        let after_prefix = &resp_str[start_idx..];
                        if let Some(end_offset) =
                            after_prefix.find('\r').or_else(|| after_prefix.find('\n'))
                        {
                            // Extract valid message, discarding leading garbage
                            let valid_msg = after_prefix[..end_offset].to_string();
                            tracing::debug!(cmd = %full_cmd, response = %valid_msg, "ELL14 transaction");
                            return Ok(valid_msg);
                        }
                    }

                    // Prevent OOM if buffer grows too large without a match
                    if response.len() > 512 {
                        tracing::warn!(
                            cmd = %full_cmd,
                            buffer_len = response.len(),
                            "ELL14 response buffer overflow, discarding old data"
                        );
                        response.drain(0..256);
                    }
                }
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No data yet, wait a bit and retry
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    continue;
                }
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::TimedOut => {
                    // Wait a bit and retry until deadline
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    continue;
                }
                Ok(Err(e)) => return Err(e.into()),
                Err(_elapsed) => {
                    // tokio::time::timeout elapsed - no data available within 50ms
                    // Continue loop and re-check deadline
                    continue;
                }
            }

            // Small delay before next read attempt to avoid spinning
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        // If we timed out without finding a complete response, return what we have
        // (caller can decide if it's valid)
        let resp = String::from_utf8_lossy(&response).to_string();
        if resp.is_empty() {
            return Err(anyhow!(
                "ELL14 transaction timeout: no response received for command '{}'",
                full_cmd
            ));
        }
        tracing::debug!(cmd = %full_cmd, response = %resp, incomplete = true, "ELL14 transaction (incomplete)");
        Ok(resp)
    }

    /// Home the device to mechanical zero.
    pub async fn home(&self) -> Result<()> {
        // IMPORTANT: Do NOT retry home commands! The device executes the home even if it
        // doesn't respond in time (it's busy moving). Retrying causes repeated homing.
        match self.transaction_once("ho").await {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("timeout") => {
                tracing::debug!(
                    address = %self.address,
                    "Home command sent (no response - device is homing)"
                );
                Ok(()) // Command was sent, device is just busy
            }
            Err(e) => Err(e),
        }
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

    /// Set forward velocity as percentage of maximum (0-100).
    ///
    /// Higher values = faster movement. Default is ~50%, max is 100%.
    /// Call this during initialization for fastest scans.
    #[instrument(skip(self), fields(address = %self.address))]
    pub async fn set_velocity(&self, percent: u8) -> Result<()> {
        let percent = percent.min(100); // Clamp to valid range
                                        // ELL14 velocity is 2 hex digits representing 0-100%
        let hex = format!("sv{:02X}", percent);
        let _ = self.transaction(&hex).await?;
        tracing::info!(
            address = %self.address,
            percent,
            "ELL14 velocity set"
        );
        Ok(())
    }

    /// Get current forward velocity as percentage (0-100).
    #[instrument(skip(self), fields(address = %self.address))]
    pub async fn get_velocity(&self) -> Result<u8> {
        let resp = self.transaction("gv").await?;
        // Response format: {addr}GV{2 hex digits}
        if let Some(idx) = resp.find("GV") {
            let hex_str = &resp[idx + 2..].trim();
            if let Some(hex) = hex_str.get(..2) {
                if let Ok(percent) = u8::from_str_radix(hex, 16) {
                    tracing::trace!(address = %self.address, percent, "ELL14 velocity read");
                    return Ok(percent);
                }
            }
        }
        tracing::debug!(address = %self.address, response = %resp, "ELL14 get_velocity failed to parse");
        Err(anyhow!("Failed to parse velocity response: {}", resp))
    }

    /// Set velocity to maximum (100%) for fastest operation.
    pub async fn set_max_velocity(&self) -> Result<()> {
        self.set_velocity(100).await
    }

    /// Get backward velocity as percentage (0-100).
    #[instrument(skip(self), fields(address = %self.address))]
    pub async fn get_backward_velocity(&self) -> Result<u8> {
        let resp = self.transaction("bv").await?;
        // Response format: {addr}BV{2 hex digits}
        if let Some(idx) = resp.find("BV") {
            let hex_str = &resp[idx + 2..].trim();
            if let Some(hex) = hex_str.get(..2) {
                if let Ok(percent) = u8::from_str_radix(hex, 16) {
                    tracing::trace!(address = %self.address, percent, "ELL14 backward velocity read");
                    return Ok(percent);
                }
            }
        }
        tracing::debug!(address = %self.address, response = %resp, "ELL14 get_backward_velocity failed to parse");
        Err(anyhow!(
            "Failed to parse backward velocity response: {}",
            resp
        ))
    }

    /// Refresh cached settings by querying hardware.
    ///
    /// Call this periodically if you need up-to-date settings without
    /// making individual queries. The cache is non-blocking to read.
    #[instrument(skip(self), fields(address = %self.address))]
    pub async fn refresh_cached_settings(&self) -> Result<()> {
        let velocity = self.get_velocity().await.unwrap_or(50);
        let backward_velocity = self.get_backward_velocity().await.unwrap_or(50);
        let position = self.position().await.ok();

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let mut cache = self.cached_settings.write().await;
        cache.velocity_percent = velocity;
        cache.backward_velocity_percent = backward_velocity;
        cache.last_position_deg = position;
        cache.last_updated_ms = now_ms;

        tracing::debug!(
            address = %self.address,
            velocity,
            backward_velocity,
            position = ?cache.last_position_deg,
            "Cached settings refreshed"
        );

        Ok(())
    }

    /// Get cached settings snapshot (non-blocking read).
    ///
    /// Returns a clone of the current cached settings. May be stale
    /// if `refresh_cached_settings()` hasn't been called recently.
    pub async fn get_cached_settings(&self) -> Ell14CachedSettings {
        self.cached_settings.read().await.clone()
    }

    /// Get cached velocity without hardware query (may be stale).
    pub async fn cached_velocity(&self) -> u8 {
        self.cached_settings.read().await.velocity_percent
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

        tracing::debug!(
            address = %self.address,
            distance_deg,
            pulses_per_degree = self.pulses_per_degree,
            pulses,
            hex_pulses = %hex_pulses,
            full_cmd = %format!("{}mr{}", self.address, hex_pulses),
            "ELL14 move_rel: sending relative move command"
        );

        // IMPORTANT: Do NOT retry move commands! The device executes moves even if it
        // doesn't respond in time (it's busy moving). Retrying causes double movement.
        // Use transaction_once and treat timeout as success (command was sent).
        match self.transaction_once(&cmd).await {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("timeout") => {
                tracing::debug!(
                    address = %self.address,
                    cmd = %cmd,
                    "Move command sent (no response - device is moving)"
                );
                Ok(()) // Command was sent, device is just busy moving
            }
            Err(e) => Err(e),
        }
    }

    #[instrument(skip(self), fields(address = %self.address))]
    async fn position(&self) -> Result<f64> {
        let resp = self.transaction("gp").await?;
        if let Some(idx) = resp.find("PO") {
            let hex_str = &resp[idx + 2..].trim();
            if let Some(hex) = hex_str.get(..8) {
                if let Ok(pulses) = u32::from_str_radix(hex, 16) {
                    return Ok((pulses as i32) as f64 / self.pulses_per_degree);
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

            tokio::time::sleep(Duration::from_millis(25)).await; // Reduced from 50ms for faster response
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

    // =========================================================================
    // Cached Settings Tests
    // =========================================================================

    #[test]
    fn test_cached_settings_default() {
        let settings = Ell14CachedSettings::default();
        assert_eq!(settings.velocity_percent, 0);
        assert_eq!(settings.backward_velocity_percent, 0);
        assert!(settings.last_position_deg.is_none());
        assert_eq!(settings.last_updated_ms, 0);
    }

    #[test]
    fn test_cached_settings_clone() {
        let settings = Ell14CachedSettings {
            velocity_percent: 100,
            backward_velocity_percent: 80,
            last_position_deg: Some(45.0),
            last_updated_ms: 1234567890,
        };
        let cloned = settings.clone();
        assert_eq!(cloned.velocity_percent, 100);
        assert_eq!(cloned.backward_velocity_percent, 80);
        assert_eq!(cloned.last_position_deg, Some(45.0));
        assert_eq!(cloned.last_updated_ms, 1234567890);
    }

    // =========================================================================
    // Velocity Command Format Tests
    // =========================================================================

    #[test]
    fn test_velocity_command_format() {
        // Velocity commands use 2 hex digits (0-100 as 00-64)
        assert_eq!(format!("sv{:02X}", 0u8), "sv00");
        assert_eq!(format!("sv{:02X}", 50u8), "sv32");
        assert_eq!(format!("sv{:02X}", 100u8), "sv64");
    }

    #[test]
    fn test_velocity_response_parsing() {
        // Response format: {addr}GV{2 hex digits}
        let response = "2GV64"; // Address 2, velocity 100%
        if let Some(idx) = response.find("GV") {
            let hex_str = &response[idx + 2..].trim();
            if let Some(hex) = hex_str.get(..2) {
                let percent = u8::from_str_radix(hex, 16).unwrap();
                assert_eq!(percent, 100);
            }
        }

        let response = "2GV32"; // Address 2, velocity 50%
        if let Some(idx) = response.find("GV") {
            let hex_str = &response[idx + 2..].trim();
            if let Some(hex) = hex_str.get(..2) {
                let percent = u8::from_str_radix(hex, 16).unwrap();
                assert_eq!(percent, 50);
            }
        }
    }

    #[test]
    fn test_velocity_clamping() {
        // Velocity should be clamped to 0-100
        let percent: u8 = 150;
        let clamped = percent.min(100);
        assert_eq!(clamped, 100);

        let percent: u8 = 50;
        let clamped = percent.min(100);
        assert_eq!(clamped, 50);
    }

    // =========================================================================
    // Motion Command No-Retry Tests
    // =========================================================================
    //
    // IMPORTANT: Motion commands (move_rel, home) must NOT retry on timeout!
    //
    // The ELL14 device executes motion commands immediately upon receiving them,
    // but may not respond in time because it's busy moving. If we retry a move
    // command that timed out, the device will execute the move AGAIN, causing
    // double (or triple) the expected movement.
    //
    // This was a real bug: clicking +90° caused 180° or 270° movement because
    // the driver retried the command 2-3 times when the device didn't respond.
    //
    // The fix: move_rel() and home() use transaction_once() directly instead of
    // transaction() which has retry logic. Timeouts are treated as success
    // because the command was sent and the device is just busy moving.

    #[test]
    fn test_move_rel_pulse_calculation() {
        // Verify pulse calculation for move_rel commands
        // ELL14 uses pulses_per_degree to convert degrees to motor pulses
        let ppd = Ell14Driver::DEFAULT_PULSES_PER_DEGREE; // ~398.22

        // 90 degrees should be ~35840 pulses
        let distance_deg = 90.0;
        let pulses = (distance_deg * ppd).round() as i32;
        assert_eq!(pulses, 35840);

        // Hex format for ELL14 protocol: 8 uppercase hex digits
        let hex_pulses = format!("{:08X}", pulses as u32);
        assert_eq!(hex_pulses, "00008C00");

        // Full command format: mr{8 hex digits}
        let cmd = format!("mr{}", hex_pulses);
        assert_eq!(cmd, "mr00008C00");
    }

    #[test]
    fn test_move_rel_negative_pulse_calculation() {
        // Verify negative (CCW) movement pulse calculation
        let ppd = Ell14Driver::DEFAULT_PULSES_PER_DEGREE;

        // -90 degrees should be negative pulses, but formatted as u32 (two's complement)
        let distance_deg = -90.0;
        let pulses = (distance_deg * ppd).round() as i32;
        assert_eq!(pulses, -35840);

        // When cast to u32, negative becomes two's complement
        let hex_pulses = format!("{:08X}", pulses as u32);
        assert_eq!(hex_pulses, "FFFF7400");
    }

    #[test]
    fn test_move_command_format() {
        // Document the expected command format for relative moves
        // This ensures the protocol is correctly implemented

        // Small positive movement (1 degree)
        let ppd = Ell14Driver::DEFAULT_PULSES_PER_DEGREE;
        let pulses = (1.0 * ppd).round() as i32;
        let hex = format!("{:08X}", pulses as u32);
        assert_eq!(hex, "0000018E"); // ~398 pulses

        // Medium movement (10 degrees)
        let pulses = (10.0 * ppd).round() as i32;
        let hex = format!("{:08X}", pulses as u32);
        assert_eq!(hex, "00000F8E"); // ~3982 pulses
    }

    /// This test documents the critical no-retry behavior for motion commands.
    ///
    /// Motion commands MUST use transaction_once() (no retry) because:
    /// 1. The ELL14 executes move commands immediately upon receipt
    /// 2. The device doesn't respond while it's busy moving
    /// 3. If we retry a timed-out move, the device executes it AGAIN
    /// 4. This causes 2x or 3x the expected movement (the original bug)
    ///
    /// The implementation treats timeout as success for motion commands because
    /// the command was successfully sent - the device is just busy moving.
    #[test]
    fn test_motion_commands_no_retry_documentation() {
        // This test serves as documentation that motion commands must not retry.
        // The actual behavior is tested by the real hardware, but this test
        // ensures developers understand the design decision.
        //
        // If you're changing move_rel() or home() to use transaction() instead
        // of transaction_once(), STOP! You will cause double-movement bugs.
        //
        // See commit b5fa5e9 for the fix that resolved this issue.

        // Verify the timeout error message format that we check for
        let timeout_msg = "ELL14 transaction timeout: no response received for command 'test'";
        assert!(timeout_msg.contains("timeout"));
    }
}
