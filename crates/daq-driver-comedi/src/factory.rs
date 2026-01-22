//! DriverFactory implementations for Comedi DAQ devices.
//!
//! This module provides registry-compatible factories for integrating Comedi
//! devices into the rust-daq ecosystem via the DriverFactory pattern.
//!
//! # Supported Drivers
//!
//! - [`ComediAnalogInputFactory`] - Multi-channel analog input for power monitoring
//! - [`ComediAnalogOutputFactory`] - Analog output for feedback control
//!
//! # Hardware Tested
//!
//! - NI PCI-MIO-16XE-10 (16-ch AI, 2-ch AO, DIO, counters) with BNC-2110
//!
//! # Input Reference Modes
//!
//! The analog input driver supports multiple input reference modes:
//!
//! | Mode | Config Value | Description |
//! |------|--------------|-------------|
//! | RSE  | `"rse"` or `"ground"` | Referenced Single-Ended (vs card ground) |
//! | NRSE | `"nrse"` or `"common"` | Non-Referenced Single-Ended (vs AISENSE) |
//! | DIFF | `"diff"` or `"differential"` | Differential (ACH0+ACH8, ACH1+ACH9, etc.) |
//!
//! For loopback testing with a BNC-2110, use `input_mode = "rse"`.
//!
//! # Example Configuration
//!
//! ```toml
//! [[devices]]
//! id = "ni_daq_ai"
//! type = "comedi_analog_input"
//! enabled = true
//!
//! [devices.config]
//! device = "/dev/comedi0"
//! channel = 0
//! range_index = 0
//! input_mode = "rse"  # or "nrse", "diff"
//! ```

use anyhow::{Context, Result};
use async_trait::async_trait;
use daq_core::capabilities::{Parameterized, Readable};
use daq_core::driver::{Capability, DeviceComponents, DeviceMetadata, DriverFactory};
use daq_core::observable::ParameterSet;
use daq_core::parameter::Parameter;
use futures::future::BoxFuture;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::device::ComediDevice;
use crate::subsystem::analog_input::AnalogInput;
use crate::subsystem::analog_output::AnalogOutput;
use crate::subsystem::{AnalogReference, Range};

// =============================================================================
// Configuration Types
// =============================================================================

/// Configuration for Comedi analog input driver.
#[derive(Debug, Clone, Deserialize)]
pub struct ComediAnalogInputConfig {
    /// Path to Comedi device (e.g., "/dev/comedi0")
    #[serde(default = "default_device")]
    pub device: String,

    /// Analog input channel number (0-15 for 16-ch cards)
    #[serde(default)]
    pub channel: u32,

    /// Voltage range index (0 = default, typically ±10V)
    #[serde(default)]
    pub range_index: u32,

    /// Input reference mode: "rse" (default), "nrse", or "diff"
    ///
    /// - **RSE** (Referenced Single-Ended): Measures vs card ground (AIGND).
    ///   Best for loopback testing and grounded signals.
    /// - **NRSE** (Non-Referenced Single-Ended): Measures vs AISENSE pin.
    ///   Use when signal has its own ground reference.
    /// - **DIFF** (Differential): Measures difference between paired channels
    ///   (ACH0+ACH8, ACH1+ACH9, etc.). Best for noise rejection.
    #[serde(default = "default_input_mode")]
    pub input_mode: String,

    /// Human-readable name for the measurement
    #[serde(default)]
    pub measurement_name: Option<String>,

    /// Measurement units (default: "V")
    #[serde(default = "default_units")]
    pub units: String,

    /// Enable mock mode for testing without hardware
    #[serde(default)]
    pub mock: bool,
}

fn default_device() -> String {
    "/dev/comedi0".to_string()
}

fn default_units() -> String {
    "V".to_string()
}

fn default_input_mode() -> String {
    "rse".to_string()
}

/// Parse input mode string to AnalogReference.
///
/// Supported values:
/// - "rse", "ground", "single-ended" → Ground (RSE)
/// - "nrse", "common" → Common (NRSE)
/// - "diff", "differential" → Differential
fn parse_input_mode(mode: &str) -> Result<AnalogReference> {
    match mode.to_lowercase().as_str() {
        "rse" | "ground" | "single-ended" | "single_ended" => Ok(AnalogReference::Ground),
        "nrse" | "common" => Ok(AnalogReference::Common),
        "diff" | "differential" => Ok(AnalogReference::Differential),
        "other" => Ok(AnalogReference::Other),
        _ => anyhow::bail!(
            "Invalid input_mode '{}'. Valid values: rse, nrse, diff (or: ground, common, differential)",
            mode
        ),
    }
}

/// Configuration for Comedi analog output driver.
#[derive(Debug, Clone, Deserialize)]
pub struct ComediAnalogOutputConfig {
    /// Path to Comedi device
    #[serde(default = "default_device")]
    pub device: String,

    /// Analog output channel number
    #[serde(default)]
    pub channel: u32,

    /// Voltage range index
    #[serde(default)]
    pub range_index: u32,

    /// Output units (default: "V")
    #[serde(default = "default_units")]
    pub units: String,

    /// Enable mock mode
    #[serde(default)]
    pub mock: bool,
}

// =============================================================================
// Mock Implementations (for testing without hardware)
// =============================================================================

/// Mock analog input for testing.
struct MockAnalogInput {
    #[allow(dead_code)]
    channel: u32,
    value: RwLock<f64>,
}

impl MockAnalogInput {
    fn new(channel: u32) -> Self {
        Self {
            channel,
            value: RwLock::new(0.5 + 0.1 * channel as f64), // Simulated offset per channel
        }
    }

    async fn read_voltage(&self) -> Result<f64> {
        // Simulate noise
        let base = *self.value.read().await;
        let noise = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos() as f64
            / 1e9
            - 0.5)
            * 0.01;
        Ok(base + noise)
    }
}

/// Mock analog output for testing.
struct MockAnalogOutput {
    #[allow(dead_code)]
    channel: u32,
    value: RwLock<f64>,
}

impl MockAnalogOutput {
    fn new(channel: u32) -> Self {
        Self {
            channel,
            value: RwLock::new(0.0),
        }
    }

    async fn write_voltage(&self, voltage: f64) -> Result<()> {
        *self.value.write().await = voltage;
        Ok(())
    }

    async fn read_voltage(&self) -> Result<f64> {
        Ok(*self.value.read().await)
    }
}

// =============================================================================
// ComediAnalogInputDriver
// =============================================================================

/// Driver for Comedi analog input channels.
///
/// This driver wraps a single analog input channel and exposes it via the
/// rust-daq `Readable` trait for integration with the experiment engine.
///
/// # Use Cases
///
/// - Power monitoring via photodiode signal
/// - Voltage measurements
/// - General analog signal acquisition
///
/// # Example
///
/// ```rust,ignore
/// let driver = ComediAnalogInputDriver::new_async("/dev/comedi0", 0, 0, false).await?;
/// let voltage = driver.read().await?;
/// println!("Channel 0: {:.3} V", voltage);
/// ```
pub struct ComediAnalogInputDriver {
    /// Real hardware (None if mock mode)
    #[allow(dead_code)]
    device: Option<ComediDevice>,
    analog_input: Option<AnalogInput>,

    /// Mock implementation (Some if mock mode)
    mock: Option<MockAnalogInput>,

    /// Channel being read
    channel: u32,

    /// Range index
    range_index: u32,

    /// Analog reference mode (RSE, NRSE, DIFF)
    aref: AnalogReference,

    /// Parameter registry
    params: Arc<ParameterSet>,

    /// Voltage reading parameter (updated on each read)
    #[allow(dead_code)]
    voltage: Parameter<f64>,

    /// Channel parameter (read-only info)
    #[allow(dead_code)]
    channel_param: Parameter<f64>,

    /// Input mode parameter (read-only info)
    #[allow(dead_code)]
    input_mode_param: Parameter<f64>,
}

impl ComediAnalogInputDriver {
    /// Create a new analog input driver.
    ///
    /// # Arguments
    /// * `device_path` - Path to Comedi device (e.g., "/dev/comedi0")
    /// * `channel` - Analog input channel number
    /// * `range_index` - Voltage range index
    /// * `aref` - Analog reference mode (RSE, NRSE, DIFF)
    /// * `mock` - If true, use mock implementation
    pub async fn new_async(
        device_path: &str,
        channel: u32,
        range_index: u32,
        aref: AnalogReference,
        mock: bool,
    ) -> Result<Arc<Self>> {
        let mut params = ParameterSet::new();

        // Create parameters
        let voltage = Parameter::new("voltage", 0.0)
            .with_description("Last voltage reading")
            .with_unit("V");

        let channel_param =
            Parameter::new("channel", channel as f64).with_description("Analog input channel");

        // Input mode as numeric for parameter (0=RSE, 1=NRSE, 2=DIFF, 3=Other)
        let input_mode_param = Parameter::new("input_mode", aref.to_raw() as f64)
            .with_description("Input reference mode (0=RSE, 1=NRSE, 2=DIFF)");

        params.register(voltage.clone());
        params.register(channel_param.clone());
        params.register(input_mode_param.clone());

        let aref_name = match aref {
            AnalogReference::Ground => "RSE",
            AnalogReference::Common => "NRSE",
            AnalogReference::Differential => "DIFF",
            AnalogReference::Other => "OTHER",
        };

        let driver = if mock {
            info!(
                "Creating mock Comedi analog input driver (channel={}, mode={})",
                channel, aref_name
            );
            Self {
                device: None,
                analog_input: None,
                mock: Some(MockAnalogInput::new(channel)),
                channel,
                range_index,
                aref,
                params: Arc::new(params),
                voltage,
                channel_param,
                input_mode_param,
            }
        } else {
            info!(
                "Opening Comedi device {} for analog input (channel={}, range={}, mode={})",
                device_path, channel, range_index, aref_name
            );

            // Open device in blocking task (FFI is blocking)
            let path = device_path.to_string();
            let device = tokio::task::spawn_blocking(move || ComediDevice::open(&path))
                .await
                .context("Task join error")?
                .context("Failed to open Comedi device")?;

            let ai = device
                .analog_input()
                .context("Failed to get analog input subsystem")?;

            // Validate channel
            let n_channels = ai.n_channels();
            if channel >= n_channels {
                anyhow::bail!(
                    "Channel {} out of range (device has {} channels)",
                    channel,
                    n_channels
                );
            }

            // Warn about differential mode channel requirements
            if aref == AnalogReference::Differential && channel >= n_channels / 2 {
                tracing::warn!(
                    "Channel {} may not support differential mode (typical max: {})",
                    channel,
                    n_channels / 2 - 1
                );
            }

            info!(
                "Opened {} ({}), channel {}/{}, {}-bit resolution, mode={}",
                device.board_name(),
                device.driver_name(),
                channel,
                n_channels,
                ai.resolution_bits(),
                aref_name
            );

            Self {
                device: Some(device),
                analog_input: Some(ai),
                mock: None,
                channel,
                range_index,
                aref,
                params: Arc::new(params),
                voltage,
                channel_param,
                input_mode_param,
            }
        };

        Ok(Arc::new(driver))
    }

    /// Read voltage from the configured channel using the configured reference mode.
    async fn read_voltage(&self) -> Result<f64> {
        if let Some(mock) = &self.mock {
            return mock.read_voltage().await;
        }

        let ai = self
            .analog_input
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No analog input subsystem"))?;

        let channel = self.channel;
        let aref = self.aref;
        // Use specified range index
        let range = Range {
            index: self.range_index,
            ..Range::default()
        };

        // Comedi FFI is blocking, run in blocking task
        let ai_clone = ai.clone();
        let voltage =
            tokio::task::spawn_blocking(move || ai_clone.read_raw(channel, range.index, aref))
                .await
                .context("Task join error")?
                .context("Failed to read voltage")?;

        // Convert raw to voltage
        let voltage = ai.raw_to_voltage(voltage, &range);

        debug!(
            "Read voltage: channel={}, aref={:?}, value={:.4}V",
            self.channel, self.aref, voltage
        );
        Ok(voltage)
    }

    /// Get the configured analog reference mode.
    pub fn analog_reference(&self) -> AnalogReference {
        self.aref
    }

    /// Get the channel number.
    pub fn channel(&self) -> u32 {
        self.channel
    }

    /// Get the device board name.
    pub fn board_name(&self) -> String {
        self.device
            .as_ref()
            .map(|d| d.board_name())
            .unwrap_or_else(|| "mock".to_string())
    }
}

impl Parameterized for ComediAnalogInputDriver {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl Readable for ComediAnalogInputDriver {
    async fn read(&self) -> Result<f64> {
        self.read_voltage().await
    }
}

// =============================================================================
// ComediAnalogOutputDriver
// =============================================================================

/// Driver for Comedi analog output channels.
///
/// This driver wraps a single analog output channel for control applications
/// such as PID feedback loops.
pub struct ComediAnalogOutputDriver {
    /// Real hardware
    #[allow(dead_code)]
    device: Option<ComediDevice>,
    analog_output: Option<AnalogOutput>,

    /// Mock implementation
    mock: Option<MockAnalogOutput>,

    /// Channel
    channel: u32,

    /// Range index
    range_index: u32,

    /// Parameter registry
    params: Arc<ParameterSet>,

    /// Output voltage parameter
    output: Parameter<f64>,
}

impl ComediAnalogOutputDriver {
    /// Create a new analog output driver.
    pub async fn new_async(
        device_path: &str,
        channel: u32,
        range_index: u32,
        mock: bool,
    ) -> Result<Arc<Self>> {
        let mut params = ParameterSet::new();

        let output = Parameter::new("output", 0.0)
            .with_description("Output voltage")
            .with_unit("V")
            .with_range(-10.0, 10.0);

        params.register(output.clone());

        let driver = if mock {
            info!("Creating mock Comedi analog output driver (channel={})", channel);
            Self {
                device: None,
                analog_output: None,
                mock: Some(MockAnalogOutput::new(channel)),
                channel,
                range_index,
                params: Arc::new(params),
                output,
            }
        } else {
            info!(
                "Opening Comedi device {} for analog output (channel={})",
                device_path, channel
            );

            let path = device_path.to_string();
            let device = tokio::task::spawn_blocking(move || ComediDevice::open(&path))
                .await
                .context("Task join error")?
                .context("Failed to open Comedi device")?;

            let ao = device
                .analog_output()
                .context("Failed to get analog output subsystem")?;

            Self {
                device: Some(device),
                analog_output: Some(ao),
                mock: None,
                channel,
                range_index,
                params: Arc::new(params),
                output,
            }
        };

        Ok(Arc::new(driver))
    }

    /// Write voltage to the output channel.
    pub async fn write_voltage(&self, voltage: f64) -> Result<()> {
        if let Some(mock) = &self.mock {
            return mock.write_voltage(voltage).await;
        }

        let ao = self
            .analog_output
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No analog output subsystem"))?;

        let channel = self.channel;
        let range = Range {
            index: self.range_index,
            ..Range::default()
        };

        let ao_clone = ao.clone();
        tokio::task::spawn_blocking(move || ao_clone.write_voltage(channel, voltage, range))
            .await
            .context("Task join error")?
            .context("Failed to write voltage")?;

        debug!("Wrote voltage: channel={}, value={:.4}V", self.channel, voltage);
        Ok(())
    }

    /// Read current output value (from cache, not hardware).
    ///
    /// Note: Comedi AO doesn't support hardware readback on all devices.
    /// This returns the cached value from the output parameter.
    pub async fn read_output(&self) -> Result<f64> {
        if let Some(mock) = &self.mock {
            return mock.read_voltage().await;
        }

        // Return cached parameter value (no hardware readback for AO)
        Ok(self.output.get())
    }

    /// Get output parameter for external control.
    pub fn output(&self) -> &Parameter<f64> {
        &self.output
    }
}

impl Parameterized for ComediAnalogOutputDriver {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

// =============================================================================
// Factories
// =============================================================================

/// Factory for Comedi analog input drivers.
///
/// Creates single-channel analog input drivers suitable for power monitoring,
/// voltage measurements, and general signal acquisition.
pub struct ComediAnalogInputFactory;

static AI_CAPABILITIES: &[Capability] = &[Capability::Readable, Capability::Parameterized];

impl DriverFactory for ComediAnalogInputFactory {
    fn driver_type(&self) -> &'static str {
        "comedi_analog_input"
    }

    fn name(&self) -> &'static str {
        "Comedi Analog Input"
    }

    fn capabilities(&self) -> &'static [Capability] {
        AI_CAPABILITIES
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        let cfg: ComediAnalogInputConfig = config
            .clone()
            .try_into()
            .context("Invalid Comedi analog input config")?;

        if cfg.device.is_empty() {
            anyhow::bail!("'device' path cannot be empty");
        }

        // Validate input_mode
        parse_input_mode(&cfg.input_mode)?;

        // Channel validation will happen at build time when we know device capabilities
        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let cfg: ComediAnalogInputConfig = config
                .try_into()
                .context("Invalid Comedi analog input config")?;

            // Parse input reference mode
            let aref = parse_input_mode(&cfg.input_mode)?;

            let driver = ComediAnalogInputDriver::new_async(
                &cfg.device,
                cfg.channel,
                cfg.range_index,
                aref,
                cfg.mock,
            )
            .await?;

            Ok(DeviceComponents {
                readable: Some(driver.clone()),
                parameterized: Some(driver),
                metadata: DeviceMetadata {
                    measurement_units: Some(cfg.units),
                    ..Default::default()
                },
                ..Default::default()
            })
        })
    }
}

/// Factory for Comedi analog output drivers.
pub struct ComediAnalogOutputFactory;

static AO_CAPABILITIES: &[Capability] = &[Capability::Parameterized];

impl DriverFactory for ComediAnalogOutputFactory {
    fn driver_type(&self) -> &'static str {
        "comedi_analog_output"
    }

    fn name(&self) -> &'static str {
        "Comedi Analog Output"
    }

    fn capabilities(&self) -> &'static [Capability] {
        AO_CAPABILITIES
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        let cfg: ComediAnalogOutputConfig = config
            .clone()
            .try_into()
            .context("Invalid Comedi analog output config")?;

        if cfg.device.is_empty() {
            anyhow::bail!("'device' path cannot be empty");
        }

        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let cfg: ComediAnalogOutputConfig = config
                .try_into()
                .context("Invalid Comedi analog output config")?;

            let driver =
                ComediAnalogOutputDriver::new_async(&cfg.device, cfg.channel, cfg.range_index, cfg.mock)
                    .await?;

            Ok(DeviceComponents {
                parameterized: Some(driver),
                metadata: DeviceMetadata {
                    measurement_units: Some(cfg.units),
                    ..Default::default()
                },
                ..Default::default()
            })
        })
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ai_factory_type() {
        let factory = ComediAnalogInputFactory;
        assert_eq!(factory.driver_type(), "comedi_analog_input");
        assert_eq!(factory.name(), "Comedi Analog Input");
    }

    #[test]
    fn test_ao_factory_type() {
        let factory = ComediAnalogOutputFactory;
        assert_eq!(factory.driver_type(), "comedi_analog_output");
        assert_eq!(factory.name(), "Comedi Analog Output");
    }

    #[test]
    fn test_ai_capabilities() {
        let factory = ComediAnalogInputFactory;
        let caps = factory.capabilities();
        assert!(caps.contains(&Capability::Readable));
        assert!(caps.contains(&Capability::Parameterized));
    }

    #[tokio::test]
    async fn test_ai_factory_validate() {
        let factory = ComediAnalogInputFactory;

        // Valid config
        let valid = toml::toml! {
            device = "/dev/comedi0"
            channel = 0
        };
        assert!(factory.validate(&toml::Value::Table(valid)).is_ok());

        // Empty device
        let empty_device = toml::toml! {
            device = ""
        };
        assert!(factory.validate(&toml::Value::Table(empty_device)).is_err());
    }

    #[tokio::test]
    async fn test_ai_mock_driver() {
        let driver =
            ComediAnalogInputDriver::new_async("/dev/comedi0", 0, 0, AnalogReference::Ground, true)
                .await
                .expect("Failed to create mock driver");

        // Read should work
        let voltage = driver.read().await.expect("Failed to read");
        assert!(voltage.is_finite());

        // Parameters should be registered
        let params = driver.parameters();
        assert!(params.names().contains(&"voltage"));
        assert!(params.names().contains(&"channel"));
        assert!(params.names().contains(&"input_mode"));

        // Check analog reference
        assert_eq!(driver.analog_reference(), AnalogReference::Ground);
    }

    #[tokio::test]
    async fn test_ai_mock_driver_differential() {
        let driver = ComediAnalogInputDriver::new_async(
            "/dev/comedi0",
            0,
            0,
            AnalogReference::Differential,
            true,
        )
        .await
        .expect("Failed to create mock driver");

        assert_eq!(driver.analog_reference(), AnalogReference::Differential);
    }

    #[tokio::test]
    async fn test_parse_input_mode() {
        // RSE variants
        assert!(matches!(
            parse_input_mode("rse"),
            Ok(AnalogReference::Ground)
        ));
        assert!(matches!(
            parse_input_mode("RSE"),
            Ok(AnalogReference::Ground)
        ));
        assert!(matches!(
            parse_input_mode("ground"),
            Ok(AnalogReference::Ground)
        ));

        // NRSE variants
        assert!(matches!(
            parse_input_mode("nrse"),
            Ok(AnalogReference::Common)
        ));
        assert!(matches!(
            parse_input_mode("common"),
            Ok(AnalogReference::Common)
        ));

        // DIFF variants
        assert!(matches!(
            parse_input_mode("diff"),
            Ok(AnalogReference::Differential)
        ));
        assert!(matches!(
            parse_input_mode("differential"),
            Ok(AnalogReference::Differential)
        ));

        // Invalid
        assert!(parse_input_mode("invalid").is_err());
    }

    #[tokio::test]
    async fn test_ao_mock_driver() {
        let driver = ComediAnalogOutputDriver::new_async("/dev/comedi0", 0, 0, true)
            .await
            .expect("Failed to create mock driver");

        // Write and read back
        driver.write_voltage(2.5).await.expect("Failed to write");
        let readback = driver.read_output().await.expect("Failed to read");
        assert!((readback - 2.5).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_ai_factory_build_mock() {
        let factory = ComediAnalogInputFactory;

        let config = toml::toml! {
            device = "/dev/comedi0"
            channel = 0
            mock = true
        };

        let result = factory.build(toml::Value::Table(config)).await;
        assert!(result.is_ok());

        let components = result.unwrap();
        assert!(components.readable.is_some());
        assert!(components.parameterized.is_some());
    }
}
