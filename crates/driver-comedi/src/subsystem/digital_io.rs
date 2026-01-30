//! Digital I/O subsystem.
//!
//! This module provides safe access to digital I/O channels on a Comedi device.

use tracing::debug;

use crate::device::{ComediDevice, SubdeviceType};
use crate::error::{ComediError, Result, SubdeviceTypeError};

/// Direction for a digital I/O channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum DioDirection {
    /// Configure channel as input
    #[default]
    Input = comedi_sys::COMEDI_INPUT,
    /// Configure channel as output
    Output = comedi_sys::COMEDI_OUTPUT,
}

/// Digital I/O port identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DioPort {
    /// Main DIO port (typically 8 channels)
    Main,
    /// RTSI bus (Real-Time System Integration, typically 10 channels)
    Rtsi,
    /// PFI lines (Programmable Function Interface, typically 8 channels)
    Pfi,
    /// Custom port by subdevice index
    Custom(u32),
}

/// Digital I/O subsystem accessor.
///
/// Provides methods to read and write digital I/O channels.
#[derive(Clone)]
pub struct DigitalIO {
    device: ComediDevice,
    subdevice: u32,
    n_channels: u32,
}

impl DigitalIO {
    /// Create a new digital I/O accessor for the given subdevice.
    pub(crate) fn new(device: ComediDevice, subdevice: u32) -> Result<Self> {
        // Verify subdevice type (accept DIO, DI, or DO)
        let subdev_type = device.subdevice_type(subdevice)?;
        match subdev_type {
            SubdeviceType::DigitalIO
            | SubdeviceType::DigitalInput
            | SubdeviceType::DigitalOutput => {}
            _ => {
                return Err(ComediError::SubdeviceTypeMismatch {
                    subdevice,
                    expected: SubdeviceTypeError::DigitalIO,
                    actual: subdev_type.to_error_type(),
                });
            }
        }

        let n_channels =
            unsafe { comedi_sys::comedi_get_n_channels(device.handle(), subdevice) as u32 };

        debug!(
            subdevice = subdevice,
            n_channels = n_channels,
            "Created digital I/O accessor"
        );

        Ok(Self {
            device,
            subdevice,
            n_channels,
        })
    }

    /// Get the number of digital I/O channels.
    pub fn n_channels(&self) -> u32 {
        self.n_channels
    }

    /// Configure the direction of a channel.
    pub fn configure(&self, channel: u32, direction: DioDirection) -> Result<()> {
        self.validate_channel(channel)?;

        let result = unsafe {
            comedi_sys::comedi_dio_config(
                self.device.handle(),
                self.subdevice,
                channel,
                direction as u32,
            )
        };

        if result < 0 {
            return Err(unsafe { ComediError::from_errno() });
        }

        Ok(())
    }

    /// Configure multiple channels as inputs.
    pub fn configure_inputs(&self, channels: &[u32]) -> Result<()> {
        for &ch in channels {
            self.configure(ch, DioDirection::Input)?;
        }
        Ok(())
    }

    /// Configure multiple channels as outputs.
    pub fn configure_outputs(&self, channels: &[u32]) -> Result<()> {
        for &ch in channels {
            self.configure(ch, DioDirection::Output)?;
        }
        Ok(())
    }

    /// Configure all channels in a range.
    pub fn configure_range(&self, start: u32, count: u32, direction: DioDirection) -> Result<()> {
        for ch in start..(start + count) {
            self.configure(ch, direction)?;
        }
        Ok(())
    }

    /// Read a single digital channel.
    ///
    /// Returns true if the channel is high, false if low.
    pub fn read(&self, channel: u32) -> Result<bool> {
        self.validate_channel(channel)?;

        let mut bit: u32 = 0;

        let result = unsafe {
            comedi_sys::comedi_dio_read(self.device.handle(), self.subdevice, channel, &mut bit)
        };

        if result < 0 {
            return Err(unsafe { ComediError::from_errno() });
        }

        Ok(bit != 0)
    }

    /// Write a single digital channel.
    pub fn write(&self, channel: u32, value: bool) -> Result<()> {
        self.validate_channel(channel)?;

        let result = unsafe {
            comedi_sys::comedi_dio_write(
                self.device.handle(),
                self.subdevice,
                channel,
                if value { 1 } else { 0 },
            )
        };

        if result < 0 {
            return Err(unsafe { ComediError::from_errno() });
        }

        Ok(())
    }

    /// Read multiple channels as a bitmask.
    ///
    /// Reads up to 32 channels starting from `base_channel`.
    /// Returns the state as a bitmask where bit 0 corresponds to base_channel.
    pub fn read_port(&self, base_channel: u32) -> Result<u32> {
        let mut bits: u32 = 0;

        let result = unsafe {
            comedi_sys::comedi_dio_bitfield2(
                self.device.handle(),
                self.subdevice,
                0, // write_mask = 0 means read-only
                &mut bits,
                base_channel,
            )
        };

        if result < 0 {
            return Err(unsafe { ComediError::from_errno() });
        }

        Ok(bits)
    }

    /// Write multiple channels from a bitmask.
    ///
    /// Writes up to 32 channels starting from `base_channel`.
    /// Only channels with corresponding bits set in `write_mask` are modified.
    pub fn write_port(&self, base_channel: u32, write_mask: u32, values: u32) -> Result<()> {
        let mut bits = values;

        let result = unsafe {
            comedi_sys::comedi_dio_bitfield2(
                self.device.handle(),
                self.subdevice,
                write_mask,
                &mut bits,
                base_channel,
            )
        };

        if result < 0 {
            return Err(unsafe { ComediError::from_errno() });
        }

        Ok(())
    }

    /// Read all channels and return as a vector of booleans.
    pub fn read_all(&self) -> Result<Vec<bool>> {
        let mut result = Vec::with_capacity(self.n_channels as usize);
        for ch in 0..self.n_channels {
            result.push(self.read(ch)?);
        }
        Ok(result)
    }

    /// Set a channel high.
    pub fn set_high(&self, channel: u32) -> Result<()> {
        self.write(channel, true)
    }

    /// Set a channel low.
    pub fn set_low(&self, channel: u32) -> Result<()> {
        self.write(channel, false)
    }

    /// Toggle a channel (read current state, write opposite).
    pub fn toggle(&self, channel: u32) -> Result<bool> {
        let current = self.read(channel)?;
        let new_value = !current;
        self.write(channel, new_value)?;
        Ok(new_value)
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

impl std::fmt::Debug for DigitalIO {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DigitalIO")
            .field("subdevice", &self.subdevice)
            .field("n_channels", &self.n_channels)
            .finish()
    }
}
