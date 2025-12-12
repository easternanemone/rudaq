//! Hardware drivers and capability traits for laboratory instruments
//!
//! This module provides a unified interface for controlling various scientific instruments
//! including motion controllers, cameras, lasers, and power meters.
//!
//! # Testing Strategy
//!
//! ## Mock Serial Port Testing
//!
//! All serial-based hardware drivers can be tested using the `mock_serial` module, which
//! provides a drop-in replacement for `serial2_tokio::SerialPort` that works entirely
//! in-memory without requiring physical hardware.
//!
//! ### Architecture
//!
//! - **MockSerialPort**: Implements `AsyncRead` + `AsyncWrite`, given to driver code
//! - **MockDeviceHarness**: Controls mock from test, simulates device behavior
//! - **Channels**: Unbounded mpsc channels connect the two for bidirectional communication
//!
//! ### Key Testing Capabilities
//!
//! 1. **Command/Response Sequences**: Script exact device behavior
//! 2. **Timeout Testing**: Simulate non-responsive devices by not sending responses
//! 3. **Flow Control**: Test rapid command sequences with realistic delays
//! 4. **Error Handling**: Simulate malformed responses, partial data, broken pipes
//! 5. **Protocol Validation**: Assert exact byte sequences sent/received
//!
//! ### Example Test Pattern
//!
//! ```rust,ignore
//! use rust_daq::hardware::mock_serial;
//! use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
//!
//! #[tokio::test]
//! async fn test_laser_power_query() {
//!     let (port, mut harness) = mock_serial::new();
//!     let mut reader = BufReader::new(port);
//!
//!     let app_task = tokio::spawn(async move {
//!         reader.write_all(b"POWER?\r").await.unwrap();
//!         let mut response = String::new();
//!         reader.read_line(&mut response).await.unwrap();
//!         response
//!     });
//!
//!     harness.expect_write(b"POWER?\r").await;
//!     harness.send_response(b"POWER:2.5\r\n").unwrap();
//!
//!     assert_eq!(app_task.await.unwrap(), "POWER:2.5\r\n");
//! }
//! ```
//!
//! See `tests/hardware_serial_tests.rs` for comprehensive integration tests.

// Re-export capabilities, registry, resource_pool
pub use daq_hardware::drivers::mock;
pub use daq_hardware::{capabilities, registry, resource_pool};

#[cfg(feature = "tokio_serial")]
pub use daq_hardware::plugin;

// Common capability imports
// pub use capabilities::{ExposureControl, FrameProducer, Movable, Readable, Triggerable}; (Removed duplicate)

// Real Hardware Drivers
#[cfg(feature = "instrument_thorlabs")]
pub use daq_hardware::drivers::ell14;

#[cfg(feature = "instrument_newport")]
pub use daq_hardware::drivers::esp300;

#[cfg(all(feature = "instrument_photometrics", feature = "pvcam_hardware"))]
pub use daq_hardware::drivers::pvcam;

#[cfg(all(feature = "instrument_photometrics", feature = "pvcam_hardware", feature = "arrow"))]
pub use daq_hardware::drivers::pvcam::PvcamDriver;

#[cfg(feature = "instrument_spectra_physics")]
pub use daq_hardware::drivers::maitai;

#[cfg(feature = "instrument_newport_power_meter")]
pub use daq_hardware::drivers::newport_1830c;

// Configure daq-hardware mock serial for tests
/// Mock serial port support for testing without hardware
#[cfg(feature = "instrument_serial")]
pub mod mock_serial {
    pub use daq_hardware::drivers::mock_serial::*;
}

// Re-export core capability traits
pub use capabilities::{ExposureControl, FrameProducer, Movable, Readable, Triggerable};

// =============================================================================
// Data Types
// =============================================================================

/// Thread-safe frame reference for camera/image data
///
/// Uses daq_core::data types for interoperability.
pub use daq_core::data::{Frame, FrameRef};

// =============================================================================
// Useful Types Migrated from core_v3
// =============================================================================

/// Region of Interest for camera acquisition
///
/// Defines a rectangular crop region within sensor area.
/// Used by cameras that support ROI to reduce readout time and data volume.
pub use daq_core::core::Roi;

// Frame struct removed (using daq_core::data::Frame)
