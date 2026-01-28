//! Switchable and Settable trait implementations for digital I/O.

use anyhow::Result;
use async_trait::async_trait;
use common::capabilities::{Settable, Switchable};
use parking_lot::RwLock;
use serde_json::Value;

use crate::subsystem::digital_io::{DigitalIO, DioDirection};

/// A wrapper that implements [`Switchable`] and [`Settable`] for Comedi digital I/O.
///
/// This allows digital I/O channels to be controlled generically through
/// the DAQ framework's capability system.
///
/// # Channel Naming
///
/// Channels can be addressed by:
/// - Number: `"0"`, `"1"`, `"2"`, etc.
/// - Named: `"dio0"`, `"dio1"`, `"pin0"`, `"pin1"`, etc.
/// - Port: `"port"` for 32-bit bitfield operations
///
/// # Example
///
/// ```rust,ignore
/// use common::capabilities::Switchable;
/// use daq_driver_comedi::hal::SwitchableDigitalIO;
///
/// let dio = device.digital_io(0)?;
/// let mut switchable = SwitchableDigitalIO::new(dio);
///
/// // Configure channel 0 as output and turn it on
/// switchable.configure_output(0)?;
/// switchable.turn_on("0").await?;
///
/// // Check state
/// let is_high = switchable.is_on("0").await?;
/// ```
pub struct SwitchableDigitalIO {
    /// The underlying digital I/O subsystem
    inner: DigitalIO,
    /// Cache of configured directions
    directions: RwLock<Vec<Option<DioDirection>>>,
}

impl SwitchableDigitalIO {
    /// Create a new switchable digital I/O wrapper.
    pub fn new(inner: DigitalIO) -> Self {
        let n_channels = inner.n_channels() as usize;
        Self {
            inner,
            directions: RwLock::new(vec![None; n_channels]),
        }
    }

    /// Get the underlying digital I/O subsystem.
    pub fn inner(&self) -> &DigitalIO {
        &self.inner
    }

    /// Configure a channel as input.
    pub fn configure_input(&self, channel: u32) -> Result<()> {
        self.inner
            .configure(channel, DioDirection::Input)
            .map_err(|e| anyhow::anyhow!("Failed to configure input: {}", e))?;

        if let Some(dir) = self.directions.write().get_mut(channel as usize) {
            *dir = Some(DioDirection::Input);
        }
        Ok(())
    }

    /// Configure a channel as output.
    pub fn configure_output(&self, channel: u32) -> Result<()> {
        self.inner
            .configure(channel, DioDirection::Output)
            .map_err(|e| anyhow::anyhow!("Failed to configure output: {}", e))?;

        if let Some(dir) = self.directions.write().get_mut(channel as usize) {
            *dir = Some(DioDirection::Output);
        }
        Ok(())
    }

    /// Configure a range of channels with the same direction.
    pub fn configure_range(&self, start: u32, count: u32, direction: DioDirection) -> Result<()> {
        self.inner
            .configure_range(start, count, direction)
            .map_err(|e| anyhow::anyhow!("Failed to configure range: {}", e))?;

        let mut dirs = self.directions.write();
        for i in start..(start + count) {
            if let Some(dir) = dirs.get_mut(i as usize) {
                *dir = Some(direction);
            }
        }
        Ok(())
    }

    /// Parse channel number from name.
    ///
    /// Supported formats:
    /// - "0", "1", "2" - direct channel number
    /// - "dio0", "dio1" - dio prefix
    /// - "pin0", "pin1" - pin prefix
    /// - "ch0", "ch1" - channel prefix
    fn parse_channel(name: &str) -> Result<u32> {
        // Try direct number first
        if let Ok(n) = name.parse::<u32>() {
            return Ok(n);
        }

        // Try prefixed formats
        for prefix in &["dio", "pin", "ch", "channel"] {
            if let Some(suffix) = name.strip_prefix(prefix) {
                if let Ok(n) = suffix.parse::<u32>() {
                    return Ok(n);
                }
            }
        }

        anyhow::bail!(
            "Invalid channel name '{}'. Use number or prefix (dio0, pin0, ch0)",
            name
        )
    }
}

#[async_trait]
impl Switchable for SwitchableDigitalIO {
    /// Turn on (set high) a digital output channel.
    async fn turn_on(&mut self, name: &str) -> Result<()> {
        let channel = Self::parse_channel(name)?;

        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || inner.set_high(channel))
            .await
            .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
            .map_err(|e| anyhow::anyhow!("Set high error: {}", e))?;

        Ok(())
    }

    /// Turn off (set low) a digital output channel.
    async fn turn_off(&mut self, name: &str) -> Result<()> {
        let channel = Self::parse_channel(name)?;

        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || inner.set_low(channel))
            .await
            .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
            .map_err(|e| anyhow::anyhow!("Set low error: {}", e))?;

        Ok(())
    }

    /// Query the on/off state of a digital channel.
    async fn is_on(&self, name: &str) -> Result<bool> {
        let channel = Self::parse_channel(name)?;

        let inner = self.inner.clone();
        let is_high = tokio::task::spawn_blocking(move || inner.read(channel))
            .await
            .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
            .map_err(|e| anyhow::anyhow!("Read error: {}", e))?;

        Ok(is_high)
    }
}

#[async_trait]
impl Settable for SwitchableDigitalIO {
    /// Set a digital I/O parameter.
    ///
    /// # Supported Parameters
    ///
    /// - `"dio_N"` or `"N"`: Set channel N to value (bool or 0/1)
    /// - `"port"`: Set 32-bit port value (u32)
    /// - `"direction_N"`: Configure direction ("input" or "output")
    async fn set_value(&self, name: &str, value: Value) -> Result<()> {
        // Handle port-wide operations
        if name == "port" {
            let port_value = value
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("port value must be an unsigned integer"))?
                as u32;

            let inner = self.inner.clone();
            tokio::task::spawn_blocking(move || inner.write_port(0, 0xFFFF_FFFF, port_value))
                .await
                .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
                .map_err(|e| anyhow::anyhow!("Write port error: {}", e))?;

            return Ok(());
        }

        // Handle direction configuration
        if let Some(suffix) = name.strip_prefix("direction_") {
            let channel = Self::parse_channel(suffix)?;
            let direction = match value.as_str() {
                Some("input") | Some("in") => DioDirection::Input,
                Some("output") | Some("out") => DioDirection::Output,
                _ => anyhow::bail!("direction must be 'input' or 'output'"),
            };

            let inner = self.inner.clone();
            tokio::task::spawn_blocking(move || inner.configure(channel, direction))
                .await
                .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
                .map_err(|e| anyhow::anyhow!("Configure error: {}", e))?;

            return Ok(());
        }

        // Handle individual channel writes
        let channel = Self::parse_channel(name)?;
        let bit_value = match &value {
            Value::Bool(b) => *b,
            Value::Number(n) => n.as_u64().map(|v| v != 0).unwrap_or(false),
            _ => anyhow::bail!("channel value must be bool or number"),
        };

        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || inner.write(channel, bit_value))
            .await
            .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
            .map_err(|e| anyhow::anyhow!("Write error: {}", e))?;

        Ok(())
    }

    /// Get a digital I/O parameter.
    ///
    /// # Supported Parameters
    ///
    /// - `"dio_N"` or `"N"`: Read channel N state (bool)
    /// - `"port"`: Read 32-bit port value (u32)
    /// - `"n_channels"`: Number of DIO channels
    async fn get_value(&self, name: &str) -> Result<Value> {
        if name == "port" {
            let inner = self.inner.clone();
            let port_value = tokio::task::spawn_blocking(move || inner.read_port(0))
                .await
                .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
                .map_err(|e| anyhow::anyhow!("Read port error: {}", e))?;

            return Ok(Value::from(port_value));
        }

        if name == "n_channels" {
            return Ok(Value::from(self.inner.n_channels()));
        }

        // Read individual channel
        let channel = Self::parse_channel(name)?;
        let inner = self.inner.clone();
        let is_high = tokio::task::spawn_blocking(move || inner.read(channel))
            .await
            .map_err(|e| anyhow::anyhow!("Task join error: {}", e))?
            .map_err(|e| anyhow::anyhow!("Read error: {}", e))?;

        Ok(Value::Bool(is_high))
    }
}

impl std::fmt::Debug for SwitchableDigitalIO {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwitchableDigitalIO")
            .field("n_channels", &self.inner.n_channels())
            .finish()
    }
}

// Send + Sync are derived from the inner types:
// - DigitalIO contains ComediDevice which is Send + Sync (via Arc<DeviceInner> with Mutex)
// - RwLock is Send + Sync
// No unsafe impl needed.
