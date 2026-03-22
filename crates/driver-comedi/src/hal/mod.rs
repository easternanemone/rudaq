//! Hardware Abstraction Layer (HAL) trait implementations.
//!
//! This module implements common capability traits for Comedi subsystems,
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
//! use common::capabilities::Readable;
//! use driver_comedi::hal::ReadableAnalogInput;
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
mod device_introspect;
mod digital_io;
mod range_introspect;

pub use analog_input::ReadableAnalogInput;
pub use analog_output::SettableAnalogOutput;
pub use counter::ReadableCounter;
pub use device_introspect::IntrospectableDevice;
pub use digital_io::SwitchableDigitalIO;
pub use range_introspect::{RangeIntrospectableAnalogInput, RangeIntrospectableAnalogOutput};
