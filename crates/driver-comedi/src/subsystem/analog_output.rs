//! Analog output subsystem.
//!
//! This module provides safe access to analog output channels on a Comedi device.

use tracing::debug;

use comedi_sys::lsampl_t;

use crate::device::{ComediDevice, SubdeviceType};
use crate::error::{ComediError, Result, SubdeviceTypeError};
use crate::subsystem::{AnalogReference, Range};

/// Configuration for an analog output channel.
#[derive(Debug, Clone)]
pub struct AnalogOutputConfig {
    /// Channel number
    pub channel: u32,
    /// Voltage range
    pub range: Range,
    /// Analog reference type
    pub aref: AnalogReference,
}

impl Default for AnalogOutputConfig {
    fn default() -> Self {
        Self {
            channel: 0,
            range: Range::default(),
            aref: AnalogReference::Ground,
        }
    }
}

/// Analog output subsystem accessor.
///
/// Provides methods to write voltages to analog output channels.
#[derive(Clone)]
pub struct AnalogOutput {
    device: ComediDevice,
    subdevice: u32,
    n_channels: u32,
    maxdata: lsampl_t,
}

impl AnalogOutput {
    /// Create a new analog output accessor for the given subdevice.
    pub(crate) fn new(device: ComediDevice, subdevice: u32) -> Result<Self> {
        // Verify subdevice type
        let subdev_type = device.subdevice_type(subdevice)?;
        if subdev_type != SubdeviceType::AnalogOutput {
            return Err(ComediError::SubdeviceTypeMismatch {
                subdevice,
                expected: SubdeviceTypeError::AnalogOutput,
                actual: subdev_type.to_error_type(),
            });
        }

        let n_channels =
            unsafe { comedi_sys::comedi_get_n_channels(device.handle(), subdevice) as u32 };

        let maxdata = unsafe { comedi_sys::comedi_get_maxdata(device.handle(), subdevice, 0) };

        debug!(
            subdevice = subdevice,
            n_channels = n_channels,
            maxdata = maxdata,
            "Created analog output accessor"
        );

        Ok(Self {
            device,
            subdevice,
            n_channels,
            maxdata,
        })
    }

    /// Get the number of analog output channels.
    pub fn n_channels(&self) -> u32 {
        self.n_channels
    }

    /// Get the maximum data value (determines resolution).
    pub fn maxdata(&self) -> lsampl_t {
        self.maxdata
    }

    /// Get the resolution in bits.
    pub fn resolution_bits(&self) -> u32 {
        if self.maxdata == 0 {
            0
        } else {
            (self.maxdata as f64 + 1.0).log2() as u32
        }
    }

    /// Get the number of available ranges for a channel.
    pub fn n_ranges(&self, channel: u32) -> Result<u32> {
        self.validate_channel(channel)?;

        let n = unsafe {
            comedi_sys::comedi_get_n_ranges(self.device.handle(), self.subdevice, channel)
        };

        Ok(n as u32)
    }

    /// Get information about a specific range.
    pub fn range_info(&self, channel: u32, range_index: u32) -> Result<Range> {
        self.validate_channel(channel)?;

        let n_ranges = self.n_ranges(channel)?;
        if range_index >= n_ranges {
            return Err(ComediError::InvalidRange {
                range: range_index,
                max: n_ranges,
            });
        }

        let ptr = unsafe {
            comedi_sys::comedi_get_range(self.device.handle(), self.subdevice, channel, range_index)
        };

        unsafe { Range::from_ptr(range_index, ptr) }.ok_or_else(|| ComediError::NullPointer {
            function: "comedi_get_range".to_string(),
        })
    }

    /// Get all available ranges for a channel.
    pub fn ranges(&self, channel: u32) -> Result<Vec<Range>> {
        let n = self.n_ranges(channel)?;
        (0..n).map(|i| self.range_info(channel, i)).collect()
    }

    /// Write a raw value to a channel.
    ///
    /// The value should be in the range 0 to maxdata.
    pub fn write_raw(
        &self,
        channel: u32,
        range: u32,
        aref: AnalogReference,
        data: lsampl_t,
    ) -> Result<()> {
        self.validate_channel(channel)?;

        let result = unsafe {
            comedi_sys::comedi_data_write(
                self.device.handle(),
                self.subdevice,
                channel,
                range,
                aref.to_raw(),
                data,
            )
        };

        if result < 0 {
            return Err(unsafe { ComediError::from_errno() });
        }

        Ok(())
    }

    /// Write a voltage to a channel.
    ///
    /// The voltage is automatically converted to a raw value using the specified range.
    pub fn write_voltage(&self, channel: u32, voltage: f64, range: Range) -> Result<()> {
        let raw = self.voltage_to_raw(voltage, &range);
        self.write_raw(channel, range.index, AnalogReference::Ground, raw)
    }

    /// Write a voltage with full configuration.
    pub fn write_configured(&self, config: &AnalogOutputConfig, voltage: f64) -> Result<()> {
        let raw = self.voltage_to_raw(voltage, &config.range);
        self.write_raw(config.channel, config.range.index, config.aref, raw)
    }

    /// Set all channels to zero.
    pub fn zero_all(&self, range: Range) -> Result<()> {
        for ch in 0..self.n_channels {
            self.write_voltage(ch, 0.0, range)?;
        }
        Ok(())
    }

    /// Convert a voltage to raw DAC value.
    pub fn voltage_to_raw(&self, voltage: f64, range: &Range) -> lsampl_t {
        unsafe {
            let range_ptr =
                comedi_sys::comedi_get_range(self.device.handle(), self.subdevice, 0, range.index);

            if range_ptr.is_null() {
                // Fallback to manual calculation
                let fraction = (voltage - range.min) / range.span();
                (fraction * self.maxdata as f64).clamp(0.0, self.maxdata as f64) as lsampl_t
            } else {
                comedi_sys::comedi_from_phys(voltage, range_ptr, self.maxdata)
            }
        }
    }

    /// Convert a raw DAC value to voltage.
    pub fn raw_to_voltage(&self, raw: lsampl_t, range: &Range) -> f64 {
        unsafe {
            let range_ptr =
                comedi_sys::comedi_get_range(self.device.handle(), self.subdevice, 0, range.index);

            if range_ptr.is_null() {
                let fraction = raw as f64 / self.maxdata as f64;
                range.min + fraction * range.span()
            } else {
                comedi_sys::comedi_to_phys(raw, range_ptr, self.maxdata)
            }
        }
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

impl std::fmt::Debug for AnalogOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalogOutput")
            .field("subdevice", &self.subdevice)
            .field("n_channels", &self.n_channels)
            .field("resolution", &format!("{}-bit", self.resolution_bits()))
            .finish()
    }
}
