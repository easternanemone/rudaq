//! Counter/timer subsystem.
//!
//! This module provides safe access to counter/timer channels on a Comedi device.

use tracing::debug;

use comedi_sys::lsampl_t;

use crate::device::{ComediDevice, SubdeviceType};
use crate::error::{ComediError, Result, SubdeviceTypeError};

/// Counter operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CounterMode {
    /// Simple event counting (count edges on input)
    #[default]
    EventCount,
    /// Frequency measurement (count events per time period)
    FrequencyMeasurement,
    /// Period measurement (measure time between events)
    PeriodMeasurement,
    /// Pulse generation (output pulses at specified rate)
    PulseGeneration,
    /// Quadrature encoder input (A/B phase decoding)
    QuadratureEncoder,
    /// Pulse width measurement
    PulseWidth,
}

/// Counter configuration.
#[derive(Debug, Clone)]
pub struct CounterConfig {
    /// Channel/counter number
    pub channel: u32,
    /// Operating mode
    pub mode: CounterMode,
    /// Initial count value
    pub initial_count: u32,
}

impl Default for CounterConfig {
    fn default() -> Self {
        Self {
            channel: 0,
            mode: CounterMode::EventCount,
            initial_count: 0,
        }
    }
}

/// Counter/timer subsystem accessor.
///
/// Provides methods to configure and read counter/timer channels.
#[derive(Clone)]
pub struct Counter {
    device: ComediDevice,
    subdevice: u32,
    n_channels: u32,
    maxdata: lsampl_t,
}

impl Counter {
    /// Create a new counter accessor for the given subdevice.
    pub(crate) fn new(device: ComediDevice, subdevice: u32) -> Result<Self> {
        // Verify subdevice type
        let subdev_type = device.subdevice_type(subdevice)?;
        match subdev_type {
            SubdeviceType::Counter | SubdeviceType::Timer => {}
            _ => {
                return Err(ComediError::SubdeviceTypeMismatch {
                    subdevice,
                    expected: SubdeviceTypeError::Counter,
                    actual: subdev_type.to_error_type(),
                });
            }
        }

        let n_channels =
            unsafe { comedi_sys::comedi_get_n_channels(device.handle(), subdevice) as u32 };

        let maxdata = unsafe { comedi_sys::comedi_get_maxdata(device.handle(), subdevice, 0) };

        debug!(
            subdevice = subdevice,
            n_channels = n_channels,
            maxdata = maxdata,
            "Created counter accessor"
        );

        Ok(Self {
            device,
            subdevice,
            n_channels,
            maxdata,
        })
    }

    /// Get the number of counter channels.
    pub fn n_channels(&self) -> u32 {
        self.n_channels
    }

    /// Get the maximum count value (determines bit width).
    pub fn maxdata(&self) -> lsampl_t {
        self.maxdata
    }

    /// Get the counter bit width.
    pub fn bit_width(&self) -> u32 {
        if self.maxdata == 0 {
            0
        } else {
            (self.maxdata as f64 + 1.0).log2() as u32
        }
    }

    /// Read the current count value.
    pub fn read(&self, channel: u32) -> Result<lsampl_t> {
        self.validate_channel(channel)?;

        let mut data: lsampl_t = 0;

        // Use comedi_data_read for counter values
        // Range and aref are typically ignored for counters
        let result = unsafe {
            comedi_sys::comedi_data_read(
                self.device.handle(),
                self.subdevice,
                channel,
                0, // range (ignored for counters)
                0, // aref (ignored for counters)
                &mut data,
            )
        };

        if result < 0 {
            return Err(unsafe { ComediError::from_errno() });
        }

        Ok(data)
    }

    /// Write/load a count value (for preloading counters).
    pub fn write(&self, channel: u32, value: lsampl_t) -> Result<()> {
        self.validate_channel(channel)?;

        let result = unsafe {
            comedi_sys::comedi_data_write(
                self.device.handle(),
                self.subdevice,
                channel,
                0, // range
                0, // aref
                value,
            )
        };

        if result < 0 {
            return Err(unsafe { ComediError::from_errno() });
        }

        Ok(())
    }

    /// Reset counter to zero.
    pub fn reset(&self, channel: u32) -> Result<()> {
        self.write(channel, 0)
    }

    /// Reset all counters to zero.
    pub fn reset_all(&self) -> Result<()> {
        for ch in 0..self.n_channels {
            self.reset(ch)?;
        }
        Ok(())
    }

    /// Read all counter values.
    pub fn read_all(&self) -> Result<Vec<lsampl_t>> {
        (0..self.n_channels).map(|ch| self.read(ch)).collect()
    }

    fn validate_channel(&self, channel: u32) -> Result<()> {
        if channel >= self.n_channels {
            return Err(ComediError::InvalidChannel {
                subdevice: self.subdevice,
                channel,
                max: self.n_channels,
            });
        }
        Ok(())
    }
}

impl std::fmt::Debug for Counter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Counter")
            .field("subdevice", &self.subdevice)
            .field("n_channels", &self.n_channels)
            .field("bit_width", &self.bit_width())
            .finish()
    }
}
