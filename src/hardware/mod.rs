pub mod adapter;
pub mod capabilities;
pub mod mock;

// Re-export core capability traits
pub use capabilities::{
    ExposureControl, FrameProducer, Movable, Readable, Triggerable,
};

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