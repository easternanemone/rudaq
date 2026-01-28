use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;

/// Extended metadata for frames (bd-183h).
///
/// Stored as `Option<Box<FrameMetadata>>` to avoid allocation overhead
/// for frames that don't need extended metadata.
#[derive(Debug, Clone, Default)]
pub struct FrameMetadata {
    /// Sensor temperature at capture time (Celsius)
    pub temperature_c: Option<f64>,

    /// Gain mode name (e.g., "HDR", "High Sensitivity")
    pub gain_mode: Option<String>,

    /// Readout speed (e.g., "100 MHz")
    pub readout_speed: Option<String>,

    /// Binning (x, y)
    pub binning: Option<(u16, u16)>,

    /// Trigger mode (e.g., "Timed", "Trigger First")
    pub trigger_mode: Option<String>,

    /// Extensible key-value metadata for driver-specific properties
    pub extra: HashMap<String, String>,
}

/// Represents a single image frame.
///
/// Designed to be flexible for FFI (C-compatible memory layout) and efficient storage.
///
/// # Storage (bd-0dax.4)
///
/// Data is stored as `bytes::Bytes` for zero-copy sharing and pool integration.
/// - 8-bit images: 1 byte per pixel.
/// - 12/16-bit images: 2 bytes per pixel, Little Endian.
///
/// Use `as_u16_slice()` to access 16-bit data safely.
///
/// # Zero-Copy Design
///
/// The `Bytes` type enables zero-allocation frame handling:
/// - `Bytes::clone()` is O(1) - just increments a reference count
/// - When the last reference is dropped, the buffer returns to its pool
/// - `Bytes` implements `Deref<Target=[u8]>` for slice-like access
///
/// For mutating pixel data, use `data.to_vec()` to get an owned copy.
///
/// # Metadata (bd-183h)
/// Frames include timing and acquisition metadata for end-to-end traceability:
/// - `frame_number`: Driver-provided sequence number
/// - `timestamp_ns`: Capture timestamp from driver (nanoseconds since epoch)
/// - `exposure_ms`: Exposure time used for this frame
/// - `roi_x`, `roi_y`: ROI origin offset in sensor coordinates
/// - `metadata`: Optional extended metadata (temperature, gain, etc.)
#[derive(Debug, Clone)]
pub struct Frame {
    /// Width in pixels (of the captured ROI, not full sensor)
    pub width: u32,

    /// Height in pixels (of the captured ROI, not full sensor)
    pub height: u32,

    /// Bits per pixel (e.g., 8, 12, 16)
    pub bit_depth: u32,

    /// Raw pixel data (zero-copy via bytes::Bytes, bd-0dax.4)
    pub data: Bytes,

    // === Timing & Identification (bd-183h) ===
    /// Driver-provided frame sequence number (monotonically increasing)
    pub frame_number: u64,

    /// Capture timestamp from driver (nanoseconds since UNIX epoch)
    ///
    /// This is the time when the frame was captured by the camera hardware,
    /// not when it was received by the server. This enables accurate timing
    /// analysis without network jitter artifacts.
    pub timestamp_ns: u64,

    // === Acquisition Parameters (bd-183h) ===
    /// Exposure time used for this frame (milliseconds)
    pub exposure_ms: Option<f64>,

    /// ROI X offset in sensor coordinates (0 = left edge of sensor)
    pub roi_x: u32,

    /// ROI Y offset in sensor coordinates (0 = top edge of sensor)
    pub roi_y: u32,

    // === Extended Metadata (bd-183h) ===
    /// Optional extended metadata (temperature, gain, etc.)
    ///
    /// Boxed to avoid allocation overhead for frames without metadata.
    /// Use `with_metadata()` builder to set.
    pub metadata: Option<Box<FrameMetadata>>,
}

impl Frame {
    /// Create a new frame from 16-bit pixel data.
    ///
    /// Copies the data into a byte buffer.
    /// Frame metadata fields default to zero/None; use builder methods to set.
    pub fn from_u16(width: u32, height: u32, pixels: &[u16]) -> Self {
        // Convert u16 pixels to u8 bytes (Little Endian)
        let mut data = Vec::with_capacity(pixels.len() * 2);
        for pixel in pixels {
            data.extend_from_slice(&pixel.to_le_bytes());
        }

        Self {
            width,
            height,
            bit_depth: 16,
            data: Bytes::from(data),
            frame_number: 0,
            timestamp_ns: 0,
            exposure_ms: None,
            roi_x: 0,
            roi_y: 0,
            metadata: None,
        }
    }

    /// Create a new frame from 8-bit pixel data (Vec<u8>).
    ///
    /// Takes ownership of the vector.
    /// Frame metadata fields default to zero/None; use builder methods to set.
    pub fn from_u8(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            bit_depth: 8,
            data: Bytes::from(data),
            frame_number: 0,
            timestamp_ns: 0,
            exposure_ms: None,
            roi_x: 0,
            roi_y: 0,
            metadata: None,
        }
    }

    /// Create a frame from raw byte data (Vec<u8>) with explicit bit depth.
    ///
    /// Takes ownership of the vector.
    /// The caller must ensure the buffer length matches the expected size for the bit depth.
    /// Frame metadata fields default to zero/None; use builder methods to set.
    pub fn from_vec(width: u32, height: u32, bit_depth: u32, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            bit_depth,
            data: Bytes::from(data),
            frame_number: 0,
            timestamp_ns: 0,
            exposure_ms: None,
            roi_x: 0,
            roi_y: 0,
            metadata: None,
        }
    }

    /// Create a frame from `bytes::Bytes` with explicit bit depth (zero-copy).
    ///
    /// This is the preferred constructor for zero-allocation frame handling.
    /// Use this when you have a `Bytes` from a buffer pool.
    ///
    /// Frame metadata fields default to zero/None; use builder methods to set.
    pub fn from_bytes(width: u32, height: u32, bit_depth: u32, data: Bytes) -> Self {
        Self {
            width,
            height,
            bit_depth,
            data,
            frame_number: 0,
            timestamp_ns: 0,
            exposure_ms: None,
            roi_x: 0,
            roi_y: 0,
            metadata: None,
        }
    }

    // === Builder Methods (bd-183h) ===

    /// Set frame number (builder pattern).
    #[must_use]
    pub fn with_frame_number(mut self, frame_number: u64) -> Self {
        self.frame_number = frame_number;
        self
    }

    /// Set capture timestamp (builder pattern).
    #[must_use]
    pub fn with_timestamp(mut self, timestamp_ns: u64) -> Self {
        self.timestamp_ns = timestamp_ns;
        self
    }

    /// Set exposure time in milliseconds (builder pattern).
    #[must_use]
    pub fn with_exposure(mut self, exposure_ms: f64) -> Self {
        self.exposure_ms = Some(exposure_ms);
        self
    }

    /// Set ROI origin offset (builder pattern).
    #[must_use]
    pub fn with_roi_offset(mut self, roi_x: u32, roi_y: u32) -> Self {
        self.roi_x = roi_x;
        self.roi_y = roi_y;
        self
    }

    /// Set extended metadata (builder pattern).
    #[must_use]
    pub fn with_metadata(mut self, metadata: FrameMetadata) -> Self {
        self.metadata = Some(Box::new(metadata));
        self
    }

    /// Create timestamp from current system time.
    ///
    /// Utility for drivers that don't have hardware timestamps.
    pub fn timestamp_now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }

    /// Get pixel value at (x, y) as u32 (handling bit depth conversion).
    pub fn get(&self, x: u32, y: u32) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }

        let idx = (y * self.width + x) as usize;

        match self.bit_depth {
            8 => self.data.get(idx).map(|&v| v as u32),
            12 | 16 => {
                let start = idx * 2;
                if start + 1 < self.data.len() {
                    let bytes = [self.data[start], self.data[start + 1]];
                    Some(u16::from_le_bytes(bytes) as u32)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Access data as u16 slice (if applicable).
    ///
    /// This uses `bytemuck` style casting which is safe given alignment,
    /// but for simplicity without deps: this requires the machine is Little Endian (standard for standard x86/ARM).
    ///
    /// Returns None if bit_depth is 8 or data length is invalid.
    pub fn as_u16_slice(&self) -> Option<&[u16]> {
        if self.bit_depth <= 8 {
            return None;
        }
        if !self.data.len().is_multiple_of(2) {
            return None;
        }

        // SAFETY: Casting [u8] to [u16] is valid if alignment is respected.
        // Vec<u8> is not guaranteed to be u16 aligned, so we rely on `align_to`.
        // Ideally we would use `bytemuck::cast_slice`, but we want to avoid deps if possible.
        // For now, we will perform a check-and-cast.
        #[allow(unsafe_code)]
        let (prefix, mid, suffix) = unsafe { self.data.align_to::<u16>() };

        if !prefix.is_empty() || !suffix.is_empty() {
            // Alignment mismatch (bd-hnim). Log this so we know if it happens in production.
            // In practice, allocators usually return aligned memory enough for u16,
            // so this should be rare. If it happens often, change storage to Vec<u16>.
            tracing::warn!(
                prefix_len = prefix.len(),
                suffix_len = suffix.len(),
                data_len = self.data.len(),
                "Frame::as_u16_slice alignment mismatch - returning None (bd-hnim)"
            );
            return None;
        }

        Some(mid)
    }

    /// Calculate mean pixel value.
    pub fn mean(&self) -> f64 {
        match self.bit_depth {
            8 => {
                if self.data.is_empty() {
                    return 0.0;
                }
                let sum: u64 = self.data.iter().map(|&v| v as u64).sum();
                sum as f64 / self.data.len() as f64
            }
            16 => {
                let slice = self.as_u16_slice().unwrap_or(&[]);
                if slice.is_empty() {
                    return 0.0;
                }
                let sum: u64 = slice.iter().map(|&v| v as u64).sum();
                sum as f64 / slice.len() as f64
            }
            _ => 0.0,
        }
    }
}

/// Thread-safe frame reference for zero-copy sharing.
#[derive(Debug, Clone)]
pub struct FrameRef {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    data: Arc<[u8]>,
}

impl FrameRef {
    pub fn new(width: u32, height: u32, data: Vec<u8>, stride: usize) -> Self {
        Self {
            width,
            height,
            stride,
            data: data.into(),
        }
    }

    pub fn from_arc(width: u32, height: u32, data: Arc<[u8]>, stride: usize) -> Self {
        Self {
            width,
            height,
            stride,
            data,
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    pub fn data_arc(&self) -> Arc<[u8]> {
        Arc::clone(&self.data)
    }
}

// ============================================================================
// FrameView - Zero-Copy Frame Reference (bd-gtvu)
// ============================================================================

/// A borrowed view into frame data for zero-allocation observation.
///
/// `FrameView` provides read-only access to frame data without copying.
/// It's designed for the `FrameObserver` pattern where observers need to
/// inspect frame data but don't need ownership.
///
/// # Zero-Copy Design (bd-gtvu)
///
/// Unlike `Frame` which owns its pixel data via `bytes::Bytes`, `FrameView`
/// borrows the pixel slice. This eliminates the allocation overhead when
/// adapting internal frame types for external observers.
///
/// | Type | Pixel Storage | Allocation per Observer |
/// |------|---------------|------------------------|
/// | `&Frame` | `Bytes` (owned) | ~8MB copy at 4K resolution |
/// | `&FrameView` | `&[u8]` (borrowed) | 0 bytes |
///
/// # Lifetime
///
/// The `FrameView` is only valid for the duration of the observer callback.
/// Do not attempt to store or extend the lifetime of a `FrameView`.
///
/// # Example
///
/// ```rust,ignore
/// use common::data::FrameView;
///
/// fn process_frame(view: &FrameView<'_>) {
///     println!("Frame {} is {}x{}", view.frame_number, view.width, view.height);
///
///     // Access pixel data without allocation
///     let pixels = view.pixels();
///     let first_pixel = pixels.get(0).copied().unwrap_or(0);
/// }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct FrameView<'a> {
    /// Width in pixels
    pub width: u32,

    /// Height in pixels
    pub height: u32,

    /// Bits per pixel (e.g., 8, 12, 16)
    pub bit_depth: u32,

    /// Borrowed pixel data (zero-copy)
    pixels: &'a [u8],

    /// Frame sequence number
    pub frame_number: u64,

    /// Capture timestamp (nanoseconds since UNIX epoch)
    pub timestamp_ns: u64,

    /// Exposure time (milliseconds)
    pub exposure_ms: Option<f64>,

    /// ROI X offset in sensor coordinates
    pub roi_x: u32,

    /// ROI Y offset in sensor coordinates
    pub roi_y: u32,

    /// Sensor temperature at capture time (Celsius)
    pub temperature_c: Option<f64>,

    /// Binning (x, y)
    pub binning: Option<(u16, u16)>,
}

impl<'a> FrameView<'a> {
    /// Create a new FrameView from raw components.
    #[must_use]
    pub fn new(
        width: u32,
        height: u32,
        bit_depth: u32,
        pixels: &'a [u8],
        frame_number: u64,
        timestamp_ns: u64,
    ) -> Self {
        Self {
            width,
            height,
            bit_depth,
            pixels,
            frame_number,
            timestamp_ns,
            exposure_ms: None,
            roi_x: 0,
            roi_y: 0,
            temperature_c: None,
            binning: None,
        }
    }

    /// Create a FrameView from a Frame (borrows the data).
    #[must_use]
    pub fn from_frame(frame: &'a Frame) -> Self {
        Self {
            width: frame.width,
            height: frame.height,
            bit_depth: frame.bit_depth,
            pixels: &frame.data,
            frame_number: frame.frame_number,
            timestamp_ns: frame.timestamp_ns,
            exposure_ms: frame.exposure_ms,
            roi_x: frame.roi_x,
            roi_y: frame.roi_y,
            temperature_c: frame.metadata.as_ref().and_then(|m| m.temperature_c),
            binning: frame.metadata.as_ref().and_then(|m| m.binning),
        }
    }

    /// Set exposure time (builder pattern).
    #[must_use]
    pub fn with_exposure(mut self, exposure_ms: f64) -> Self {
        self.exposure_ms = Some(exposure_ms);
        self
    }

    /// Set ROI offset (builder pattern).
    #[must_use]
    pub fn with_roi_offset(mut self, roi_x: u32, roi_y: u32) -> Self {
        self.roi_x = roi_x;
        self.roi_y = roi_y;
        self
    }

    /// Set temperature (builder pattern).
    #[must_use]
    pub fn with_temperature(mut self, temperature_c: f64) -> Self {
        self.temperature_c = Some(temperature_c);
        self
    }

    /// Set binning (builder pattern).
    #[must_use]
    pub fn with_binning(mut self, binning: (u16, u16)) -> Self {
        self.binning = Some(binning);
        self
    }

    /// Get the raw pixel data as a byte slice.
    #[inline]
    #[must_use]
    pub fn pixels(&self) -> &'a [u8] {
        self.pixels
    }

    /// Get the number of bytes in the pixel data.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.pixels.len()
    }

    /// Check if pixel data is empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pixels.is_empty()
    }

    /// Get pixel value at (x, y) as u32 (handling bit depth conversion).
    #[must_use]
    pub fn get(&self, x: u32, y: u32) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }

        let idx = (y * self.width + x) as usize;

        match self.bit_depth {
            8 => self.pixels.get(idx).map(|&v| v as u32),
            12 | 16 => {
                let start = idx * 2;
                if start + 1 < self.pixels.len() {
                    let bytes = [self.pixels[start], self.pixels[start + 1]];
                    Some(u16::from_le_bytes(bytes) as u32)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Access data as u16 slice (if applicable).
    ///
    /// Returns None if bit_depth is 8 or alignment is wrong.
    #[must_use]
    pub fn as_u16_slice(&self) -> Option<&[u16]> {
        if self.bit_depth <= 8 {
            return None;
        }
        if !self.pixels.len().is_multiple_of(2) {
            return None;
        }

        // SAFETY: Casting [u8] to [u16] requires alignment check
        #[allow(unsafe_code)]
        let (prefix, mid, suffix) = unsafe { self.pixels.align_to::<u16>() };

        if !prefix.is_empty() || !suffix.is_empty() {
            // Alignment mismatch - log for debugging (consistency with Frame::as_u16_slice)
            tracing::warn!(
                prefix_len = prefix.len(),
                suffix_len = suffix.len(),
                pixels_len = self.pixels.len(),
                "FrameView::as_u16_slice alignment mismatch - returning None"
            );
            return None;
        }

        Some(mid)
    }

    /// Calculate the total number of pixels.
    #[inline]
    #[must_use]
    pub fn pixel_count(&self) -> usize {
        (self.width as usize) * (self.height as usize)
    }

    /// Calculate mean pixel value.
    #[must_use]
    pub fn mean(&self) -> f64 {
        match self.bit_depth {
            8 => {
                if self.pixels.is_empty() {
                    return 0.0;
                }
                let sum: u64 = self.pixels.iter().map(|&v| v as u64).sum();
                sum as f64 / self.pixels.len() as f64
            }
            12 | 16 => {
                if let Some(u16_slice) = self.as_u16_slice() {
                    if u16_slice.is_empty() {
                        return 0.0;
                    }
                    let sum: u64 = u16_slice.iter().map(|&v| v as u64).sum();
                    sum as f64 / u16_slice.len() as f64
                } else {
                    0.0
                }
            }
            _ => 0.0,
        }
    }
}
