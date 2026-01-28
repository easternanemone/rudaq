//! Readable trait implementation for counter/timer.

use anyhow::Result;
use async_trait::async_trait;
use common::capabilities::{Readable, Settable};
use serde_json::Value;

use crate::subsystem::counter::Counter;

/// A wrapper that implements [`Readable`] for a Comedi counter channel.
///
/// This allows counter channels to be read generically through the DAQ
/// framework's capability system. The counter value is returned as a
/// floating-point number for compatibility with the `Readable` trait.
///
/// # Thread Safety
///
/// Counter operations are synchronized through the underlying Comedi
/// device handle.
///
/// # Example
///
/// ```rust,ignore
/// use common::capabilities::Readable;
/// use daq_driver_comedi::hal::ReadableCounter;
///
/// let counter = device.counter(0)?;
/// let readable = ReadableCounter::new(counter, 0);
///
/// // Read counter value
/// let count = readable.read().await?;
/// println!("Count: {}", count as u32);
/// ```
pub struct ReadableCounter {
    /// The underlying counter subsystem
    inner: Counter,
    /// Channel to read from
    channel: u32,
}

impl ReadableCounter {
    /// Create a new readable counter wrapper.
    ///
    /// # Arguments
    ///
    /// * `inner` - The counter subsystem
    /// * `channel` - Counter channel to read from
    pub fn new(inner: Counter, channel: u32) -> Self {
        Self { inner, channel }
    }

    /// Set the channel to read from.
    pub fn set_channel(&mut self, channel: u32) {
        self.channel = channel;
    }

    /// Get the current channel.
    pub fn channel(&self) -> u32 {
        self.channel
    }

    /// Get the underlying counter subsystem.
    pub fn inner(&self) -> &Counter {
        &self.inner
    }

    /// Get the maximum count value.
    pub fn maxdata(&self) -> u32 {
        self.inner.maxdata()
    }

    /// Get the bit width of the counter.
    pub fn bit_width(&self) -> u32 {
        self.inner.bit_width()
    }
}

#[async_trait]
impl Readable for ReadableCounter {
    /// Read the current count value.
    ///
    /// Returns the count as f64 for compatibility with the Readable trait.
    /// Cast to u32 if integer precision is needed.
    async fn read(&self) -> Result<f64> {
        let inner = self.inner.clone();
        let channel = self.channel;

        let count = tokio::task::spawn_blocking(move || inner.read(channel))
            .await
            .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
            .map_err(|e| anyhow::anyhow!("Read error: {}", e))?;

        Ok(count as f64)
    }
}

#[async_trait]
impl Settable for ReadableCounter {
    /// Set a counter parameter.
    ///
    /// # Supported Parameters
    ///
    /// - `"value"` or `"count"`: Load a count value
    /// - `"reset"`: Reset counter to zero (value is ignored)
    /// - `"reset_all"`: Reset all counters to zero (value is ignored)
    async fn set_value(&self, name: &str, value: Value) -> Result<()> {
        match name {
            "value" | "count" => {
                let count = value
                    .as_u64()
                    .ok_or_else(|| anyhow::anyhow!("count must be an unsigned integer"))?
                    as u32;

                let inner = self.inner.clone();
                let channel = self.channel;

                tokio::task::spawn_blocking(move || inner.write(channel, count))
                    .await
                    .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
                    .map_err(|e| anyhow::anyhow!("Write error: {}", e))?;

                Ok(())
            }

            "reset" => {
                let inner = self.inner.clone();
                let channel = self.channel;

                tokio::task::spawn_blocking(move || inner.reset(channel))
                    .await
                    .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
                    .map_err(|e| anyhow::anyhow!("Reset error: {}", e))?;

                Ok(())
            }

            "reset_all" => {
                let inner = self.inner.clone();

                tokio::task::spawn_blocking(move || inner.reset_all())
                    .await
                    .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
                    .map_err(|e| anyhow::anyhow!("Reset all error: {}", e))?;

                Ok(())
            }

            _ => anyhow::bail!("Unknown parameter: {}", name),
        }
    }

    /// Get a counter parameter.
    ///
    /// # Supported Parameters
    ///
    /// - `"value"` or `"count"`: Current count value
    /// - `"maxdata"`: Maximum count value
    /// - `"bit_width"`: Counter bit width
    /// - `"n_channels"`: Number of counter channels
    async fn get_value(&self, name: &str) -> Result<Value> {
        match name {
            "value" | "count" => {
                let inner = self.inner.clone();
                let channel = self.channel;

                let count = tokio::task::spawn_blocking(move || inner.read(channel))
                    .await
                    .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
                    .map_err(|e| anyhow::anyhow!("Read error: {}", e))?;

                Ok(Value::from(count))
            }

            "maxdata" => Ok(Value::from(self.inner.maxdata())),
            "bit_width" => Ok(Value::from(self.inner.bit_width())),
            "n_channels" => Ok(Value::from(self.inner.n_channels())),

            _ => anyhow::bail!("Unknown parameter: {}", name),
        }
    }
}

impl std::fmt::Debug for ReadableCounter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadableCounter")
            .field("channel", &self.channel)
            .field("bit_width", &self.inner.bit_width())
            .field("n_channels", &self.inner.n_channels())
            .finish()
    }
}

// Send + Sync are derived from the inner types:
// - Counter contains ComediDevice which is Send + Sync (via Arc<DeviceInner> with Mutex)
// No unsafe impl needed.
