//! Safe Rust driver for Comedi DAQ devices.
//!
//! This crate provides a safe, ergonomic interface to Comedi (Control and
//! Measurement Device Interface) data acquisition hardware. It wraps the
//! low-level FFI bindings from `comedi-sys` with proper error handling,
//! RAII resource management, and async support.
//!
//! # Supported Hardware
//!
//! The Comedi kernel drivers support a wide variety of DAQ hardware:
//! - National Instruments PCI/PCIe cards (E-series, M-series, X-series)
//! - Measurement Computing boards
//! - Advantech cards
//! - And many more...
//!
//! This driver has been tested with:
//! - NI PCI-MIO-16XE-10 (16-ch AI, 2-ch AO, DIO, counters)
//!
//! # Architecture
//!
//! The driver is organized into subsystems matching the Comedi model:
//!
//! ## Device Access
//! - [`ComediDevice`] - Main device handle with RAII cleanup
//! - [`DeviceInfo`] / [`SubdeviceInfo`] - Device introspection
//!
//! ## Subsystems
//! - [`AnalogInput`] - Analog input subsystem with voltage/raw reading
//! - [`AnalogOutput`] - Analog output subsystem with voltage/raw writing
//! - [`DigitalIO`] - Digital I/O with per-pin direction control
//! - [`Counter`] - Counter/timer with multiple counting modes
//!
//! ## Streaming Acquisition
//! - [`StreamAcquisition`] - Hardware-timed multi-channel acquisition
//! - [`ContinuousStream`] - Indefinite streaming with multi-sink support
//! - [`StreamConfig`] - Configuration builder for streaming
//!
//! ## Timing
//! - [`TimingConfig`] - Hardware timing configuration
//! - [`TimingCapabilities`] - Query device timing limits
//! - [`ClockSource`] - Internal/external clock selection
//!
//! ## HAL Traits
//! - [`ReadableAnalogInput`] - HAL trait for analog input
//! - [`SettableAnalogOutput`] - HAL trait for analog output
//! - [`SwitchableDigitalIO`] - HAL trait for digital I/O
//! - [`ReadableCounter`] - HAL trait for counters
//!
//! # Examples
//!
//! ## Basic Single-Sample Reading
//!
//! ```no_run
//! use daq_driver_comedi::{ComediDevice, Range};
//!
//! # fn example() -> anyhow::Result<()> {
//! let device = ComediDevice::open("/dev/comedi0")?;
//!
//! println!("Board: {}", device.board_name());
//! println!("Driver: {}", device.driver_name());
//!
//! let ai = device.analog_input()?;
//! let voltage = ai.read_voltage(0, Range::default())?;
//! println!("Channel 0: {:.3} V", voltage);
//! # Ok(())
//! # }
//! ```
//!
//! ## High-Speed Streaming
//!
//! ```no_run
//! use daq_driver_comedi::{ComediDevice, StreamConfig, StreamAcquisition};
//!
//! # fn example() -> anyhow::Result<()> {
//! let device = ComediDevice::open("/dev/comedi0")?;
//!
//! let config = StreamConfig::builder()
//!     .channels(&[0, 1, 2, 3])
//!     .sample_rate(50000.0)  // 50 kS/s per channel
//!     .build()?;
//!
//! let stream = StreamAcquisition::new(&device, config)?;
//! stream.start()?;
//!
//! // Read 1000 scans (4000 samples total)
//! let data = stream.read_n_scans(1000)?;
//! println!("Acquired {} samples", data.len());
//!
//! stream.stop()?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Continuous Multi-Sink Streaming
//!
//! ```no_run
//! use daq_driver_comedi::{ComediDevice, StreamConfig, ContinuousStream};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let device = ComediDevice::open("/dev/comedi0")?;
//! let config = StreamConfig::builder()
//!     .channels(&[0, 1])
//!     .sample_rate(10000.0)
//!     .build()?;
//!
//! let stream = ContinuousStream::new(&device, config)?;
//! let mut display_rx = stream.add_sink("display", 100)?;
//! let mut storage_rx = stream.add_sink("storage", 1000)?;
//!
//! stream.start()?;
//!
//! // Process data from multiple sinks concurrently
//! while let Some(batch) = display_rx.recv().await {
//!     println!("Display: {} scans", batch.n_scans());
//! }
//! # Ok(())
//! # }
//! ```

/// Linker reference function to ensure this crate is not stripped.
///
/// This function is called by `daq_drivers::link_drivers()` to force the linker
/// to include this crate's driver registration code. Without this explicit
/// reference, the linker may optimize away driver crates that register factories
/// via constructor functions.
///
/// # Usage
///
/// This function is automatically called by `daq_drivers::link_drivers()` when
/// the `comedi` feature is enabled. You typically don't need to call it directly.
#[inline(never)]
pub fn link() {
    // Reference a type from the crate to create a dependency that the linker
    // cannot optimize away. This ensures driver factory registration code
    // (when added) will be included in the final binary.
    std::hint::black_box(std::any::TypeId::of::<ComediDevice>());
}

pub mod calibration;
pub mod continuous;
pub mod device;
pub mod error;
pub mod factory;
pub mod hal;
pub mod multi_channel;
pub mod streaming;
pub mod subsystem;
pub mod timing;

pub use continuous::{
    ContinuousStats, ContinuousStream, ContinuousStreamBuilder, SampleBatch, SinkConfig,
    SinkReceiver,
};
pub use device::{
    comedi_discover, ComediDevice, DeviceInfo, DiscoveredDevice, SubdeviceInfo, SubdeviceType,
};
pub use error::{ComediError, Result};
pub use hal::{ReadableAnalogInput, ReadableCounter, SettableAnalogOutput, SwitchableDigitalIO};
pub use multi_channel::ComediMultiChannelAcquisition;
pub use streaming::{
    ChannelSpec, SharedStreamAcquisition, StopCondition, StreamAcquisition, StreamConfig,
    StreamConfigBuilder, StreamStats, TriggerSource,
};
pub use subsystem::analog_input::{AnalogInput, AnalogInputConfig};
pub use subsystem::analog_output::{AnalogOutput, AnalogOutputConfig};
pub use subsystem::counter::{Counter, CounterMode};
pub use subsystem::digital_io::{DigitalIO, DioDirection, DioPort};
pub use subsystem::Range;
pub use timing::{
    ClockPolarity, ClockSource, TimingCapabilities, TimingConfig, TimingConfigBuilder,
};

// Factory exports for registry integration
pub use factory::{
    ComediAnalogInputConfig, ComediAnalogInputDriver, ComediAnalogInputFactory,
    ComediAnalogOutputConfig, ComediAnalogOutputDriver, ComediAnalogOutputFactory,
};

// Calibration exports
pub use calibration::{
    CalibrationData, CalibrationInfo, CalibrationManager, CalibrationPolynomial,
    CalibrationSetting, CalibrationStatus, ConversionDirection, MAX_POLYNOMIAL_COEFFICIENTS,
};
