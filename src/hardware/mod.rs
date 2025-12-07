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

pub mod capabilities;
pub mod mock;
pub mod registry;
#[cfg(feature = "tokio_serial")]
pub mod resource_pool;

// Plugin system for YAML-defined instrument drivers
#[cfg(feature = "tokio_serial")]
pub mod plugin;

// Mock serial port for testing (always available)
pub mod mock_serial;

// =============================================================================
// Real Hardware Drivers
// =============================================================================

// Thorlabs Elliptec rotation mount
#[cfg(feature = "instrument_thorlabs")]
pub mod ell14;

// Newport ESP300 multi-axis motion controller
#[cfg(feature = "instrument_newport")]
pub mod esp300;

// Photometrics PVCAM cameras (Prime BSI, Prime 95B)
#[cfg(feature = "instrument_photometrics")]
pub mod pvcam;

// Spectra-Physics MaiTai Ti:Sapphire laser
#[cfg(feature = "instrument_spectra_physics")]
pub mod maitai;

// Newport 1830-C optical power meter
#[cfg(feature = "instrument_newport_power_meter")]
pub mod newport_1830c;

// Re-export core capability traits
pub use capabilities::{ExposureControl, FrameProducer, Movable, Readable, Triggerable};

use std::sync::Arc;

// =============================================================================
// Data Types
// =============================================================================

/// Thread-safe frame reference for camera/image data
///
/// This struct provides shared access to frame buffer data using reference counting.
/// The data is automatically freed when all references are dropped.
///
/// # Thread Safety
/// Uses `Arc<[u8]>` internally for safe shared ownership across threads.
/// No manual lifetime management required - the Arc handles it automatically.
///
/// # Example
/// ```rust,ignore
/// let data: Vec<u8> = vec![0u8; 1024 * 1024];
/// let frame = FrameRef::new(1024, 1024, data, 1024);
///
/// // Access pixel at (x, y)
/// let offset = y * frame.stride + x;
/// let pixel = frame.as_slice()[offset];
///
/// // Safe to clone and share across threads
/// let frame2 = frame.clone();
/// ```
#[derive(Debug, Clone)]
pub struct FrameRef {
    /// Frame width in pixels
    pub width: u32,

    /// Frame height in pixels
    pub height: u32,

    /// Shared reference to pixel data (row-major order)
    ///
    /// Data format depends on camera (typically u8 or u16 per pixel).
    /// For multi-byte pixels, assume little-endian.
    data: Arc<[u8]>,

    /// Number of bytes per row (may include padding)
    ///
    /// For images without padding: stride = width * bytes_per_pixel
    /// For padded images: stride >= width * bytes_per_pixel
    pub stride: usize,
}

// Note: No manual Send/Sync needed - Arc<[u8]> is automatically Send+Sync

impl FrameRef {
    /// Create a new frame reference from owned data
    ///
    /// The data is moved into an Arc for shared ownership.
    pub fn new(width: u32, height: u32, data: Vec<u8>, stride: usize) -> Self {
        Self {
            width,
            height,
            data: data.into(),
            stride,
        }
    }

    /// Create a new frame reference from an existing Arc
    ///
    /// Useful when the data is already in an Arc (e.g., from a ring buffer).
    pub fn from_arc(width: u32, height: u32, data: Arc<[u8]>, stride: usize) -> Self {
        Self {
            width,
            height,
            data,
            stride,
        }
    }

    /// Get pixel data as slice
    ///
    /// Returns a reference to the underlying pixel data.
    /// The slice is valid as long as any FrameRef to this data exists.
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    /// Calculate total bytes in frame
    pub fn total_bytes(&self) -> usize {
        self.height as usize * self.stride
    }

    /// Get the Arc for efficient cloning without copying data
    pub fn data_arc(&self) -> Arc<[u8]> {
        Arc::clone(&self.data)
    }
}

// =============================================================================
// Useful Types Migrated from core_v3
// =============================================================================

/// Region of Interest for camera acquisition
///
/// Defines a rectangular crop region within sensor area.
/// Used by cameras that support ROI to reduce readout time and data volume.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Roi {
    /// Left edge of ROI (pixels from left of sensor)
    pub x: u32,

    /// Top edge of ROI (pixels from top of sensor)
    pub y: u32,

    /// Width of ROI in pixels
    pub width: u32,

    /// Height of ROI in pixels
    pub height: u32,
}

impl Default for Roi {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 1024,
            height: 1024,
        }
    }
}

impl Roi {
    /// Calculate area in pixels
    pub fn area(&self) -> u32 {
        self.width * self.height
    }

    /// Check if ROI is valid for given sensor size
    pub fn is_valid_for(&self, sensor_width: u32, sensor_height: u32) -> bool {
        self.x + self.width <= sensor_width && self.y + self.height <= sensor_height
    }
}

/// Owned frame data from camera acquisition
///
/// Unlike FrameRef (zero-copy), this struct owns the pixel data.
/// Used by camera drivers that return owned buffers (e.g., PVCAM).
#[derive(Clone, Debug)]
pub struct Frame {
    /// Frame width in pixels
    pub width: u32,

    /// Frame height in pixels
    pub height: u32,

    /// Pixel data buffer (u16 values for scientific cameras)
    ///
    /// Data is stored in row-major order (scan line by line).
    /// Length should equal width * height.
    pub buffer: Vec<u16>,
}

impl Frame {
    /// Create a new frame with given dimensions and buffer
    pub fn new(width: u32, height: u32, buffer: Vec<u16>) -> Self {
        Self {
            width,
            height,
            buffer,
        }
    }

    /// Get pixel value at (x, y)
    pub fn get_pixel(&self, x: u32, y: u32) -> Option<u16> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = (y * self.width + x) as usize;
        self.buffer.get(index).copied()
    }

    /// Calculate mean pixel value
    pub fn mean(&self) -> f64 {
        if self.buffer.is_empty() {
            return 0.0;
        }
        let sum: u64 = self.buffer.iter().map(|&v| v as u64).sum();
        sum as f64 / self.buffer.len() as f64
    }
}
