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

pub mod adapter;
pub mod capabilities;
pub mod mock;

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

// =============================================================================
// Data Types
// =============================================================================

/// Zero-copy frame reference for camera/image data
///
/// This struct provides a view into frame buffer data without copying.
/// The lifetime of the data is managed by the producer (camera driver).
///
/// # Safety
/// - `data_ptr` must remain valid for the lifetime of this struct
/// - Producer must ensure buffer is not freed while FrameRef exists
/// - Consider using Arc<Vec<u8>> or similar for safer lifetime management
///
/// # Example
/// ```rust,ignore
/// let frame = FrameRef {
///     width: 1024,
///     height: 1024,
///     data_ptr: buffer.as_ptr(),
///     stride: 1024,
/// };
///
/// // Access pixel at (x, y)
/// unsafe {
///     let offset = y * frame.stride + x;
///     let pixel = *frame.data_ptr.add(offset);
/// }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct FrameRef {
    /// Frame width in pixels
    pub width: u32,

    /// Frame height in pixels
    pub height: u32,

    /// Pointer to raw pixel data (row-major order)
    ///
    /// Data format depends on camera (typically u8 or u16 per pixel).
    /// For multi-byte pixels, assume little-endian.
    pub data_ptr: *const u8,

    /// Number of bytes per row (may include padding)
    ///
    /// For images without padding: stride = width * bytes_per_pixel
    /// For padded images: stride >= width * bytes_per_pixel
    pub stride: usize,
}

// Manual Send/Sync implementation (since raw pointers are !Send by default)
// SAFETY: The producer must ensure buffer is valid across threads
unsafe impl Send for FrameRef {}
unsafe impl Sync for FrameRef {}

impl FrameRef {
    /// Create a new frame reference
    ///
    /// # Safety
    /// - `data_ptr` must point to valid memory for at least `height * stride` bytes
    /// - Memory must remain valid for lifetime of FrameRef
    pub unsafe fn new(width: u32, height: u32, data_ptr: *const u8, stride: usize) -> Self {
        Self {
            width,
            height,
            data_ptr,
            stride,
        }
    }

    /// Get pixel data as slice (if you trust the lifetime)
    ///
    /// # Safety
    /// - Caller must ensure buffer is still valid
    /// - Returned slice lifetime is unconstrained (use carefully)
    pub unsafe fn as_slice(&self) -> &[u8] {
        std::slice::from_raw_parts(self.data_ptr, self.height as usize * self.stride)
    }

    /// Calculate total bytes in frame
    pub fn total_bytes(&self) -> usize {
        self.height as usize * self.stride
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
