//! Readable trait implementation for analog input.

use anyhow::Result;
use async_trait::async_trait;
use common::capabilities::Readable;
use parking_lot::RwLock;

use crate::subsystem::analog_input::AnalogInput;
use crate::subsystem::{AnalogReference, Range};

/// A wrapper that implements [`Readable`] for a Comedi analog input channel.
///
/// This allows analog input channels to be used generically in the DAQ framework
/// alongside other readable devices like power meters and temperature sensors.
///
/// # Thread Safety
///
/// Uses interior mutability via `RwLock` to allow configuration changes
/// while maintaining the immutable `&self` interface required by `Readable`.
///
/// # Example
///
/// ```rust,ignore
/// use common::capabilities::Readable;
/// use daq_driver_comedi::hal::ReadableAnalogInput;
///
/// let ai = device.analog_input(0)?;
/// let readable = ReadableAnalogInput::new(ai, 0, 0);
///
/// // Read voltage asynchronously
/// let voltage = readable.read().await?;
/// println!("Channel 0: {:.4} V", voltage);
/// ```
pub struct ReadableAnalogInput {
    /// The underlying analog input subsystem
    inner: AnalogInput,
    /// Channel to read from
    channel: u32,
    /// Range index to use
    range_index: u32,
    /// Analog reference type
    aref: AnalogReference,
    /// Cached range info for voltage conversion
    range: RwLock<Option<Range>>,
}

impl ReadableAnalogInput {
    /// Create a new readable analog input wrapper.
    ///
    /// # Arguments
    ///
    /// * `inner` - The analog input subsystem
    /// * `channel` - Channel number to read from
    /// * `range_index` - Range index to use for readings
    pub fn new(inner: AnalogInput, channel: u32, range_index: u32) -> Self {
        Self {
            inner,
            channel,
            range_index,
            aref: AnalogReference::Ground,
            range: RwLock::new(None),
        }
    }

    /// Create with a specific analog reference.
    pub fn with_aref(mut self, aref: AnalogReference) -> Self {
        self.aref = aref;
        self
    }

    /// Set the channel to read from.
    pub fn set_channel(&mut self, channel: u32) {
        self.channel = channel;
        // Invalidate cached range since it may differ per channel
        *self.range.write() = None;
    }

    /// Set the range index.
    pub fn set_range(&mut self, range_index: u32) {
        self.range_index = range_index;
        *self.range.write() = None;
    }

    /// Get the current channel.
    pub fn channel(&self) -> u32 {
        self.channel
    }

    /// Get the range index.
    pub fn range_index(&self) -> u32 {
        self.range_index
    }

    /// Get the underlying analog input subsystem.
    pub fn inner(&self) -> &AnalogInput {
        &self.inner
    }

    /// Get or cache the range info.
    fn get_range(&self) -> Result<Range> {
        // Check cache first
        if let Some(ref range) = *self.range.read() {
            return Ok(*range);
        }

        // Fetch and cache range info
        let range = self
            .inner
            .range_info(self.channel, self.range_index)
            .map_err(|e| anyhow::anyhow!("Failed to get range info: {}", e))?;

        *self.range.write() = Some(range);
        Ok(range)
    }
}

#[async_trait]
impl Readable for ReadableAnalogInput {
    /// Read voltage from the configured channel.
    ///
    /// Returns the voltage in the units defined by the range (typically volts).
    async fn read(&self) -> Result<f64> {
        // Get range for voltage conversion
        let range = self.get_range()?;

        // Read voltage (blocking operation wrapped in spawn_blocking)
        let inner = self.inner.clone();
        let channel = self.channel;

        let voltage = tokio::task::spawn_blocking(move || inner.read_voltage(channel, range))
            .await
            .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
            .map_err(|e| anyhow::anyhow!("Read error: {}", e))?;

        Ok(voltage)
    }
}

impl std::fmt::Debug for ReadableAnalogInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadableAnalogInput")
            .field("channel", &self.channel)
            .field("range_index", &self.range_index)
            .field("aref", &self.aref)
            .finish()
    }
}

// Send + Sync are derived from the inner types:
// - AnalogInput contains ComediDevice which is Send + Sync (via Arc<DeviceInner> with Mutex)
// - RwLock<Option<Range>> is Send + Sync
// No unsafe impl needed.
