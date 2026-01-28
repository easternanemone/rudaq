//! Settable trait implementation for analog output.

use anyhow::Result;
use async_trait::async_trait;
use common::capabilities::Settable;
use parking_lot::RwLock;
use serde_json::Value;

use crate::subsystem::analog_output::AnalogOutput;
use crate::subsystem::Range;

/// A wrapper that implements [`Settable`] for a Comedi analog output subsystem.
///
/// This allows analog output channels to be configured and controlled
/// generically through the DAQ framework's parameter system.
///
/// # Parameter Names
///
/// The following parameter names are supported:
///
/// - `"voltage"` or `"voltage_<n>"` - Set voltage on channel n (default: channel 0)
/// - `"raw"` or `"raw_<n>"` - Set raw DAC value on channel n
/// - `"channel"` - Set the default channel for subsequent operations
/// - `"range"` - Set the default range index
///
/// # Example
///
/// ```rust,ignore
/// use common::capabilities::Settable;
/// use daq_driver_comedi::hal::SettableAnalogOutput;
/// use serde_json::json;
///
/// let ao = device.analog_output(0)?;
/// let settable = SettableAnalogOutput::new(ao, 0);
///
/// // Set voltage on channel 0
/// settable.set_value("voltage", json!(2.5)).await?;
///
/// // Set voltage on specific channel
/// settable.set_value("voltage_1", json!(1.0)).await?;
/// ```
pub struct SettableAnalogOutput {
    /// The underlying analog output subsystem
    inner: AnalogOutput,
    /// Default range index
    range_index: u32,
    /// Cached range info
    range: RwLock<Option<Range>>,
}

impl SettableAnalogOutput {
    /// Create a new settable analog output wrapper.
    ///
    /// # Arguments
    ///
    /// * `inner` - The analog output subsystem
    /// * `range_index` - Default range index to use
    pub fn new(inner: AnalogOutput, range_index: u32) -> Self {
        Self {
            inner,
            range_index,
            range: RwLock::new(None),
        }
    }

    /// Get the underlying analog output subsystem.
    pub fn inner(&self) -> &AnalogOutput {
        &self.inner
    }

    /// Get or cache the range info.
    fn get_range(&self) -> Result<Range> {
        if let Some(ref range) = *self.range.read() {
            return Ok(*range);
        }

        let range = self
            .inner
            .range_info(0, self.range_index)
            .map_err(|e| anyhow::anyhow!("Failed to get range info: {}", e))?;

        *self.range.write() = Some(range);
        Ok(range)
    }

    /// Parse channel number from parameter name.
    ///
    /// Returns (base_name, channel) tuple.
    fn parse_channel_param(name: &str) -> (&str, Option<u32>) {
        if let Some(idx) = name.rfind('_') {
            let suffix = &name[idx + 1..];
            if let Ok(ch) = suffix.parse::<u32>() {
                return (&name[..idx], Some(ch));
            }
        }
        (name, None)
    }

    /// Set voltage on a channel (sync version, used internally).
    #[allow(dead_code)]
    fn set_voltage(&self, channel: u32, voltage: f64) -> Result<()> {
        let range = self.get_range()?;
        self.inner
            .write_voltage(channel, voltage, range)
            .map_err(|e| anyhow::anyhow!("Failed to write voltage: {}", e))
    }

    /// Set raw DAC value on a channel (sync version, used internally).
    #[allow(dead_code)]
    fn set_raw(&self, channel: u32, raw: u32) -> Result<()> {
        use crate::subsystem::AnalogReference;
        self.inner
            .write_raw(channel, self.range_index, AnalogReference::Ground, raw)
            .map_err(|e| anyhow::anyhow!("Failed to write raw value: {}", e))
    }
}

#[async_trait]
impl Settable for SettableAnalogOutput {
    /// Set a parameter value.
    ///
    /// # Supported Parameters
    ///
    /// - `"voltage"` or `"voltage_N"`: Set voltage on channel (N or 0)
    /// - `"raw"` or `"raw_N"`: Set raw DAC value on channel (N or 0)
    /// - `"zero_all"`: Set all channels to 0V (value is ignored)
    async fn set_value(&self, name: &str, value: Value) -> Result<()> {
        let (base_name, channel) = Self::parse_channel_param(name);
        let channel = channel.unwrap_or(0);

        match base_name {
            "voltage" => {
                let voltage = value
                    .as_f64()
                    .ok_or_else(|| anyhow::anyhow!("voltage must be a number"))?;

                let inner = self.inner.clone();
                let range = self.get_range()?;

                tokio::task::spawn_blocking(move || inner.write_voltage(channel, voltage, range))
                    .await
                    .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
                    .map_err(|e| anyhow::anyhow!("Write error: {}", e))?;

                Ok(())
            }

            "raw" => {
                let raw = value
                    .as_u64()
                    .ok_or_else(|| anyhow::anyhow!("raw must be an unsigned integer"))?
                    as u32;

                let inner = self.inner.clone();
                let range_index = self.range_index;

                tokio::task::spawn_blocking(move || {
                    use crate::subsystem::AnalogReference;
                    inner.write_raw(channel, range_index, AnalogReference::Ground, raw)
                })
                .await
                .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
                .map_err(|e| anyhow::anyhow!("Write error: {}", e))?;

                Ok(())
            }

            "zero_all" => {
                let inner = self.inner.clone();
                let range = self.get_range()?;

                tokio::task::spawn_blocking(move || inner.zero_all(range))
                    .await
                    .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
                    .map_err(|e| anyhow::anyhow!("Zero error: {}", e))?;

                Ok(())
            }

            _ => anyhow::bail!("Unknown parameter: {}", name),
        }
    }

    /// Get a parameter value.
    ///
    /// # Supported Parameters
    ///
    /// - `"n_channels"`: Number of channels
    /// - `"maxdata"`: Maximum raw value
    /// - `"resolution_bits"`: Resolution in bits
    async fn get_value(&self, name: &str) -> Result<Value> {
        match name {
            "n_channels" => Ok(Value::from(self.inner.n_channels())),
            "maxdata" => Ok(Value::from(self.inner.maxdata())),
            "resolution_bits" => Ok(Value::from(self.inner.resolution_bits())),
            _ => anyhow::bail!("Unknown parameter: {}", name),
        }
    }
}

impl std::fmt::Debug for SettableAnalogOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettableAnalogOutput")
            .field("range_index", &self.range_index)
            .field("n_channels", &self.inner.n_channels())
            .finish()
    }
}

// Send + Sync are derived from the inner types:
// - AnalogOutput contains ComediDevice which is Send + Sync (via Arc<DeviceInner> with Mutex)
// - RwLock<Option<Range>> is Send + Sync
// No unsafe impl needed.
