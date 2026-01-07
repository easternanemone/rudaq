//! Hardware Abstraction Layer (HAL) trait implementations.
//!
//! This module implements daq-core capability traits for Comedi subsystems,
//! enabling them to integrate with the unified hardware abstraction layer.
//!
//! # Implemented Traits
//!
//! - `Readable` for analog input channels
//! - `Settable` for analog output channels
//! - `Switchable` for digital I/O channels
//! - `Readable` for counter/timer channels
//!
//! # Example
//!
//! ```rust,ignore
//! use daq_core::capabilities::Readable;
//! use daq_driver_comedi::hal::ReadableAnalogInput;
//!
//! let device = ComediDevice::open("/dev/comedi0")?;
//! let ai = device.analog_input(0)?;
//! let readable = ReadableAnalogInput::new(ai, 0, 0);
//!
//! // Now use the Readable trait
//! let voltage = readable.read().await?;
//! ```

mod analog_input;
mod analog_output;
mod counter;
mod digital_io;

pub use analog_input::ReadableAnalogInput;
pub use analog_output::SettableAnalogOutput;
pub use counter::ReadableCounter;
pub use digital_io::SwitchableDigitalIO;
