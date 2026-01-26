//! Mock DAQ analog output implementation (Comedi-like).

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use daq_core::capabilities::{Parameterized, Settable};
use daq_core::driver::{Capability, DeviceComponents, DriverFactory};
use daq_core::observable::ParameterSet;
use daq_core::parameter::Parameter;
use futures::future::BoxFuture;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;

// =============================================================================
// MockDAQOutputFactory - DriverFactory implementation
// =============================================================================

/// Configuration for MockDAQOutput driver
#[derive(Debug, Clone, Deserialize)]
pub struct MockDAQOutputConfig {
    /// Channel number (default: 0)
    #[serde(default)]
    pub channel: u32,

    /// Voltage range (default: Bipolar10V)
    #[serde(default)]
    pub range: VoltageRangeConfig,

    /// Initial voltage value (default: 0.0)
    #[serde(default)]
    pub initial_voltage: f64,
}

/// Voltage range configuration (for deserialization)
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VoltageRangeConfig {
    #[default]
    Bipolar10V,
    Bipolar5V,
    Unipolar10V,
    Unipolar5V,
}


impl From<VoltageRangeConfig> for VoltageRange {
    fn from(config: VoltageRangeConfig) -> Self {
        match config {
            VoltageRangeConfig::Bipolar10V => VoltageRange::Bipolar10V,
            VoltageRangeConfig::Bipolar5V => VoltageRange::Bipolar5V,
            VoltageRangeConfig::Unipolar10V => VoltageRange::Unipolar10V,
            VoltageRangeConfig::Unipolar5V => VoltageRange::Unipolar5V,
        }
    }
}

impl Default for MockDAQOutputConfig {
    fn default() -> Self {
        Self {
            channel: 0,
            range: VoltageRangeConfig::default(),
            initial_voltage: 0.0,
        }
    }
}

/// Factory for creating MockDAQOutput instances.
pub struct MockDAQOutputFactory;

/// Static capabilities for MockDAQOutput
static MOCK_DAQ_OUTPUT_CAPABILITIES: &[Capability] =
    &[Capability::Settable, Capability::Parameterized];

impl DriverFactory for MockDAQOutputFactory {
    fn driver_type(&self) -> &'static str {
        "mock_daq_output"
    }

    fn name(&self) -> &'static str {
        "Mock DAQ Analog Output"
    }

    fn capabilities(&self) -> &'static [Capability] {
        MOCK_DAQ_OUTPUT_CAPABILITIES
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        let cfg: MockDAQOutputConfig = config.clone().try_into()?;

        let range: VoltageRange = cfg.range.into();

        // Validate initial voltage is within range
        if !range.contains(cfg.initial_voltage) {
            return Err(anyhow!(
                "Initial voltage {} V outside range {}",
                cfg.initial_voltage,
                range.description()
            ));
        }

        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let cfg: MockDAQOutputConfig = config.try_into().unwrap_or_default();

            let output = Arc::new(MockDAQOutput::with_config(cfg));

            Ok(DeviceComponents {
                settable: Some(output.clone()),
                parameterized: Some(output),
                ..Default::default()
            })
        })
    }
}

// =============================================================================
// VoltageRange
// =============================================================================

/// Voltage range for analog output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VoltageRange {
    /// -10V to +10V
    Bipolar10V,
    /// -5V to +5V
    Bipolar5V,
    /// 0V to +10V
    Unipolar10V,
    /// 0V to +5V
    Unipolar5V,
}

impl VoltageRange {
    /// Get the minimum voltage for this range.
    pub fn min(&self) -> f64 {
        match self {
            Self::Bipolar10V => -10.0,
            Self::Bipolar5V => -5.0,
            Self::Unipolar10V => 0.0,
            Self::Unipolar5V => 0.0,
        }
    }

    /// Get the maximum voltage for this range.
    pub fn max(&self) -> f64 {
        match self {
            Self::Bipolar10V => 10.0,
            Self::Bipolar5V => 5.0,
            Self::Unipolar10V => 10.0,
            Self::Unipolar5V => 5.0,
        }
    }

    /// Check if a voltage is within this range.
    pub fn contains(&self, voltage: f64) -> bool {
        voltage >= self.min() && voltage <= self.max()
    }

    /// Get a human-readable description of this range.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Bipolar10V => "±10V",
            Self::Bipolar5V => "±5V",
            Self::Unipolar10V => "0-10V",
            Self::Unipolar5V => "0-5V",
        }
    }
}

// =============================================================================
// MockDAQOutput - Simulated Analog Output
// =============================================================================

/// Mock DAQ analog output with voltage range validation.
///
/// Simulates a Comedi-like analog output with:
/// - Configurable voltage ranges (bipolar/unipolar)
/// - Multiple channels per device
/// - Range validation
/// - Settable trait implementation
///
/// # Example
///
/// ```rust,ignore
/// let output = MockDAQOutput::new(0, VoltageRange::Bipolar10V);
/// output.set_voltage(5.0).await?; // Set to 5V
/// assert_eq!(output.voltage().await?, 5.0);
/// ```
pub struct MockDAQOutput {
    /// Channel number
    channel: u32,

    /// Current voltage value
    current_value: RwLock<f64>,

    /// Voltage range
    range: VoltageRange,

    /// Parameter registry
    params: Arc<ParameterSet>,
}

impl MockDAQOutput {
    /// Create a new MockDAQOutput with default configuration.
    pub fn new() -> Self {
        Self::with_config(MockDAQOutputConfig::default())
    }

    /// Create a new MockDAQOutput for a specific channel and range.
    pub fn new_with_range(channel: u32, range: VoltageRange) -> Self {
        Self::with_config(MockDAQOutputConfig {
            channel,
            range: match range {
                VoltageRange::Bipolar10V => VoltageRangeConfig::Bipolar10V,
                VoltageRange::Bipolar5V => VoltageRangeConfig::Bipolar5V,
                VoltageRange::Unipolar10V => VoltageRangeConfig::Unipolar10V,
                VoltageRange::Unipolar5V => VoltageRangeConfig::Unipolar5V,
            },
            initial_voltage: 0.0,
        })
    }

    /// Create a new MockDAQOutput with custom configuration.
    pub fn with_config(config: MockDAQOutputConfig) -> Self {
        let range: VoltageRange = config.range.into();

        let mut params = ParameterSet::new();

        let voltage_param = Parameter::new("voltage", config.initial_voltage)
            .with_description(format!("Channel {} voltage", config.channel))
            .with_unit("V")
            .with_range(range.min(), range.max());

        params.register(voltage_param);

        Self {
            channel: config.channel,
            current_value: RwLock::new(config.initial_voltage),
            range,
            params: Arc::new(params),
        }
    }

    /// Get the channel number.
    pub fn channel(&self) -> u32 {
        self.channel
    }

    /// Get the voltage range.
    pub fn range(&self) -> VoltageRange {
        self.range
    }

    /// Set the output voltage.
    ///
    /// Returns an error if the voltage is outside the configured range.
    pub async fn set_voltage(&self, voltage: f64) -> Result<()> {
        if !self.range.contains(voltage) {
            return Err(anyhow!(
                "Voltage {} V outside range {} for channel {}",
                voltage,
                self.range.description(),
                self.channel
            ));
        }

        *self.current_value.write().await = voltage;

        Ok(())
    }

    /// Get the current output voltage.
    pub async fn voltage(&self) -> Result<f64> {
        Ok(*self.current_value.read().await)
    }
}

impl Default for MockDAQOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl Parameterized for MockDAQOutput {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl Settable for MockDAQOutput {
    async fn set_value(&self, name: &str, value: serde_json::Value) -> Result<()> {
        match name {
            "voltage" => {
                let voltage = value
                    .as_f64()
                    .ok_or_else(|| anyhow!("voltage must be a number"))?;

                self.set_voltage(voltage).await
            }

            "zero" => {
                // Zero the output (value is ignored)
                self.set_voltage(0.0).await
            }

            _ => Err(anyhow!(
                "Unknown parameter '{}' for MockDAQOutput channel {}",
                name,
                self.channel
            )),
        }
    }

    async fn get_value(&self, name: &str) -> Result<serde_json::Value> {
        match name {
            "voltage" => {
                let voltage = self.voltage().await?;
                Ok(serde_json::json!(voltage))
            }

            "channel" => Ok(serde_json::json!(self.channel)),

            "range" => Ok(serde_json::json!(self.range.description())),

            _ => Err(anyhow!(
                "Unknown parameter '{}' for MockDAQOutput channel {}",
                name,
                self.channel
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_factory_driver_type() {
        let factory = MockDAQOutputFactory;
        assert_eq!(factory.driver_type(), "mock_daq_output");
        assert_eq!(factory.name(), "Mock DAQ Analog Output");
    }

    #[tokio::test]
    async fn test_factory_capabilities() {
        let factory = MockDAQOutputFactory;
        let caps = factory.capabilities();
        assert!(caps.contains(&Capability::Settable));
        assert!(caps.contains(&Capability::Parameterized));
    }

    #[tokio::test]
    async fn test_factory_validate_config() {
        let factory = MockDAQOutputFactory;

        // Valid config
        let valid = toml::toml! {
            channel = 0
            range = "bipolar10v"
            initial_voltage = 5.0
        };
        assert!(factory.validate(&toml::Value::Table(valid)).is_ok());

        // Invalid voltage (out of range)
        let invalid = toml::toml! {
            channel = 0
            range = "bipolar5v"
            initial_voltage = 10.0
        };
        assert!(factory.validate(&toml::Value::Table(invalid)).is_err());
    }

    #[tokio::test]
    async fn test_voltage_range_bipolar10v() -> Result<()> {
        let output = MockDAQOutput::new_with_range(0, VoltageRange::Bipolar10V);

        // Valid voltages
        output.set_voltage(-10.0).await?;
        output.set_voltage(0.0).await?;
        output.set_voltage(10.0).await?;

        // Invalid voltages
        assert!(output.set_voltage(-10.1).await.is_err());
        assert!(output.set_voltage(10.1).await.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_voltage_range_bipolar5v() -> Result<()> {
        let output = MockDAQOutput::new_with_range(0, VoltageRange::Bipolar5V);

        // Valid voltages
        output.set_voltage(-5.0).await?;
        output.set_voltage(0.0).await?;
        output.set_voltage(5.0).await?;

        // Invalid voltages
        assert!(output.set_voltage(-5.1).await.is_err());
        assert!(output.set_voltage(5.1).await.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_voltage_range_unipolar10v() -> Result<()> {
        let output = MockDAQOutput::new_with_range(0, VoltageRange::Unipolar10V);

        // Valid voltages
        output.set_voltage(0.0).await?;
        output.set_voltage(5.0).await?;
        output.set_voltage(10.0).await?;

        // Invalid voltages
        assert!(output.set_voltage(-0.1).await.is_err());
        assert!(output.set_voltage(10.1).await.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_voltage_range_unipolar5v() -> Result<()> {
        let output = MockDAQOutput::new_with_range(0, VoltageRange::Unipolar5V);

        // Valid voltages
        output.set_voltage(0.0).await?;
        output.set_voltage(2.5).await?;
        output.set_voltage(5.0).await?;

        // Invalid voltages
        assert!(output.set_voltage(-0.1).await.is_err());
        assert!(output.set_voltage(5.1).await.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_settable_interface() -> Result<()> {
        let output = MockDAQOutput::new_with_range(0, VoltageRange::Bipolar10V);

        // Set via Settable trait
        output.set_value("voltage", serde_json::json!(3.5)).await?;
        assert_eq!(output.voltage().await?, 3.5);

        // Get via Settable trait
        let value = output.get_value("voltage").await?;
        assert_eq!(value.as_f64().unwrap(), 3.5);

        // Zero command
        output.set_value("zero", serde_json::json!(null)).await?;
        assert_eq!(output.voltage().await?, 0.0);

        Ok(())
    }

    #[tokio::test]
    async fn test_settable_get_metadata() -> Result<()> {
        let output = MockDAQOutput::new_with_range(5, VoltageRange::Bipolar5V);

        // Get channel
        let channel = output.get_value("channel").await?;
        assert_eq!(channel.as_u64().unwrap(), 5);

        // Get range
        let range = output.get_value("range").await?;
        assert_eq!(range.as_str().unwrap(), "±5V");

        Ok(())
    }

    #[tokio::test]
    async fn test_multiple_channels() -> Result<()> {
        // Simulate multiple channels with different ranges
        let ch0 = MockDAQOutput::new_with_range(0, VoltageRange::Bipolar10V);
        let ch1 = MockDAQOutput::new_with_range(1, VoltageRange::Unipolar5V);

        ch0.set_voltage(5.0).await?;
        ch1.set_voltage(2.5).await?;

        assert_eq!(ch0.voltage().await?, 5.0);
        assert_eq!(ch1.voltage().await?, 2.5);

        assert_eq!(ch0.channel(), 0);
        assert_eq!(ch1.channel(), 1);

        Ok(())
    }

    #[tokio::test]
    async fn test_voltage_persistence() -> Result<()> {
        let output = MockDAQOutput::new_with_range(0, VoltageRange::Bipolar10V);

        // Set multiple times
        output.set_voltage(1.0).await?;
        assert_eq!(output.voltage().await?, 1.0);

        output.set_voltage(2.0).await?;
        assert_eq!(output.voltage().await?, 2.0);

        output.set_voltage(-5.0).await?;
        assert_eq!(output.voltage().await?, -5.0);

        Ok(())
    }

    #[test]
    fn test_voltage_range_descriptions() {
        assert_eq!(VoltageRange::Bipolar10V.description(), "±10V");
        assert_eq!(VoltageRange::Bipolar5V.description(), "±5V");
        assert_eq!(VoltageRange::Unipolar10V.description(), "0-10V");
        assert_eq!(VoltageRange::Unipolar5V.description(), "0-5V");
    }

    #[test]
    fn test_voltage_range_limits() {
        assert_eq!(VoltageRange::Bipolar10V.min(), -10.0);
        assert_eq!(VoltageRange::Bipolar10V.max(), 10.0);

        assert_eq!(VoltageRange::Bipolar5V.min(), -5.0);
        assert_eq!(VoltageRange::Bipolar5V.max(), 5.0);

        assert_eq!(VoltageRange::Unipolar10V.min(), 0.0);
        assert_eq!(VoltageRange::Unipolar10V.max(), 10.0);

        assert_eq!(VoltageRange::Unipolar5V.min(), 0.0);
        assert_eq!(VoltageRange::Unipolar5V.max(), 5.0);
    }
}
