//! CameraSensor meta-instrument trait
//!
//! Hardware-agnostic interface for camera control and data acquisition.
//! Implementations handle protocol-specific details (PVCAM SDK, GigE Vision, etc.).

use anyhow::Result;
use arrow::array::{BinaryArray, StringArray, TimestampNanosecondArray, UInt32Array, UInt64Array};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use std::sync::Arc;

/// Pixel format enumeration (extensible)
#[derive(Debug, Clone, Copy, PartialEq, Eq, kameo::Reply)]
pub enum PixelFormat {
    Mono8,   // 8-bit monochrome
    Mono12,  // 12-bit monochrome (packed)
    Mono16,  // 16-bit monochrome
    Bayer8,  // Bayer RGB (8-bit)
    Bayer16, // Bayer RGB (16-bit)
}

impl PixelFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            PixelFormat::Mono8 => "Mono8",
            PixelFormat::Mono12 => "Mono12",
            PixelFormat::Mono16 => "Mono16",
            PixelFormat::Bayer8 => "Bayer8",
            PixelFormat::Bayer16 => "Bayer16",
        }
    }
}

/// Region of Interest configuration
#[derive(Debug, Clone, kameo::Reply)]
pub struct RegionOfInterest {
    /// Top-left X coordinate (pixel)
    pub x: u32,
    /// Top-left Y coordinate (pixel)
    pub y: u32,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
}

impl RegionOfInterest {
    pub fn full_sensor(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }
}

/// Binning configuration (hardware-dependent)
#[derive(Debug, Clone, Copy, kameo::Reply)]
pub struct BinningConfig {
    /// Binning factor in X direction (1 = no binning, 2 = 2x2, etc.)
    pub x_bin: u8,
    /// Binning factor in Y direction
    pub y_bin: u8,
}

/// Trigger mode for frame acquisition
#[derive(Debug, Clone, Copy, PartialEq, Eq, kameo::Reply)]
pub enum TriggerMode {
    /// Internal trigger (free-running at configured frame rate)
    Internal,
    /// External trigger on digital input
    External,
}

/// Camera timing configuration
#[derive(Debug, Clone, kameo::Reply)]
pub struct CameraTiming {
    /// Exposure time in microseconds
    pub exposure_us: u32,
    /// Frame period (for streaming); inverse = frame rate (Hz)
    pub frame_period_ms: f64,
    /// Trigger mode
    pub trigger_mode: TriggerMode,
}

/// Single frame with pixel data and metadata
#[derive(Debug, Clone, kameo::Reply)]
pub struct Frame {
    /// Frame timestamp (nanoseconds since acquisition start)
    pub timestamp_ns: i64,
    /// Frame counter (for detecting dropped frames)
    pub frame_number: u64,
    /// Pixel format
    pub pixel_format: PixelFormat,
    /// Width in pixels (after binning)
    pub width: u32,
    /// Height in pixels (after binning)
    pub height: u32,
    /// ROI applied to this frame
    pub roi: RegionOfInterest,
    /// Pixel data (format determined by pixel_format)
    /// For Mono16: Vec<u16> cast to Vec<u8> with length = width * height * 2
    /// For Mono8: Vec<u8> with length = width * height
    pub pixel_data: Vec<u8>,
}

/// Streaming configuration
#[derive(Debug, Clone, kameo::Reply)]
pub struct CameraStreamConfig {
    /// ROI for streaming frames
    pub roi: RegionOfInterest,
    /// Binning configuration
    pub binning: BinningConfig,
    /// Timing settings
    pub timing: CameraTiming,
    /// Gain setting (0-100, hardware-dependent)
    pub gain: u8,
}

/// Camera capabilities and hardware limits
#[derive(Debug, Clone, kameo::Reply)]
pub struct CameraCapabilities {
    pub sensor_width: u32,
    pub sensor_height: u32,
    pub pixel_formats: Vec<PixelFormat>,
    pub max_binning_x: u8,
    pub max_binning_y: u8,
    pub min_exposure_us: u32,
    pub max_exposure_us: u32,
    pub max_frame_rate_hz: f64,
}

/// Camera sensor meta-instrument trait
///
/// Hardware-agnostic interface for camera control and data acquisition.
/// Implementations handle protocol-specific details (PVCAM SDK, GigE Vision, etc.).
///
/// ## Frame Queueing
/// - Uses bounded `tokio::sync::mpsc` channel (default capacity: 10 frames)
/// - Oldest frames dropped when queue full
/// - Dropped frame count tracked in metadata
///
/// ## Memory Management
/// - New `Vec<u8>` allocated for each frame (simple, safe)
/// - Memory pool optimization deferred to Phase 2
///
/// ## Synchronization
/// - Uses `SystemTime::now()` for timestamps
/// - Correlate with motion controller via post-hoc timestamp matching
#[async_trait]
pub trait CameraSensor: Send + Sync {
    /// Start continuous frame acquisition
    ///
    /// # Arguments
    /// * `config` - Stream configuration (ROI, timing, gain, etc.)
    ///
    /// # Behavior
    /// - Configures camera according to `config`
    /// - Starts internal frame acquisition thread/task
    /// - Frames are queued internally and retrieved via subscriber
    /// - Returns immediately; actual acquisition happens asynchronously
    ///
    /// # Errors
    /// - Hardware not connected
    /// - Invalid configuration (ROI out of bounds, exposure exceeds max, etc.)
    /// - Frame rate exceeds hardware capability
    async fn start_stream(&self, config: CameraStreamConfig) -> Result<()>;

    /// Stop streaming acquisition
    ///
    /// # Behavior
    /// - Stops frame acquisition task
    /// - Clears internal frame queue
    ///
    /// # Errors
    /// - Hardware communication error
    async fn stop_stream(&self) -> Result<()>;

    /// Check if streaming is active
    fn is_streaming(&self) -> bool;

    /// Acquire single frame (snapshot)
    ///
    /// # Arguments
    /// * `config` - Timing and ROI for this frame
    ///
    /// # Behavior
    /// - Takes one frame with specified settings
    /// - Blocks until frame is available (with timeout)
    /// - Returns frame with full metadata
    ///
    /// # Errors
    /// - Hardware not connected
    /// - Timeout waiting for frame
    /// - Invalid configuration
    async fn snap_frame(&self, config: &CameraTiming) -> Result<Frame>;

    /// Configure Region of Interest
    ///
    /// # Arguments
    /// * `roi` - ROI definition
    ///
    /// # Behavior
    /// - Updates ROI for future frames
    /// - If streaming, applies to next queued frame
    ///
    /// # Errors
    /// - ROI out of sensor bounds
    /// - Invalid ROI dimensions
    async fn configure_roi(&self, roi: RegionOfInterest) -> Result<()>;

    /// Configure timing parameters
    ///
    /// # Arguments
    /// * `timing` - Exposure, frame period, trigger mode
    ///
    /// # Errors
    /// - Exposure exceeds hardware maximum
    /// - Frame period too short (violates exposure + readout time)
    async fn set_timing(&self, timing: CameraTiming) -> Result<()>;

    /// Set sensor gain
    ///
    /// # Arguments
    /// * `gain` - Gain in 0-100 range (hardware interprets as appropriate scale)
    ///
    /// # Errors
    /// - Gain out of valid range
    async fn set_gain(&self, gain: u8) -> Result<()>;

    /// Configure binning
    ///
    /// # Arguments
    /// * `binning` - X and Y binning factors
    ///
    /// # Errors
    /// - Binning not supported by hardware
    /// - Binning factor exceeds hardware maximum
    async fn set_binning(&self, binning: BinningConfig) -> Result<()>;

    /// Get current camera capabilities and limits
    fn get_capabilities(&self) -> CameraCapabilities;

    /// Serialize frames to Arrow RecordBatch for storage/transfer
    ///
    /// # Arguments
    /// * `frames` - Slice of frames to serialize
    ///
    /// # Output Schema
    /// ```text
    /// timestamp (i64 ns, Timestamp(Nanosecond))
    /// frame_number (u64)
    /// pixel_format (utf8)
    /// width (u32)
    /// height (u32)
    /// roi_x (u32)
    /// roi_y (u32)
    /// roi_width (u32)
    /// roi_height (u32)
    /// pixel_data (binary, raw bytes for each frame)
    /// ```
    ///
    /// # Note
    /// Pixel data stored as binary blob; deserialization requires format context.
    fn to_arrow_frames(&self, frames: &[Frame]) -> Result<RecordBatch> {
        static SCHEMA: Lazy<Arc<Schema>> = Lazy::new(|| {
            Arc::new(Schema::new(vec![
                Field::new(
                    "timestamp",
                    DataType::Timestamp(arrow::datatypes::TimeUnit::Nanosecond, None),
                    false,
                ),
                Field::new("frame_number", DataType::UInt64, false),
                Field::new("pixel_format", DataType::Utf8, false),
                Field::new("width", DataType::UInt32, false),
                Field::new("height", DataType::UInt32, false),
                Field::new("roi_x", DataType::UInt32, false),
                Field::new("roi_y", DataType::UInt32, false),
                Field::new("roi_width", DataType::UInt32, false),
                Field::new("roi_height", DataType::UInt32, false),
                Field::new("pixel_data", DataType::Binary, false),
            ]))
        });

        let timestamps: Vec<i64> = frames.iter().map(|f| f.timestamp_ns).collect();
        let frame_numbers: Vec<u64> = frames.iter().map(|f| f.frame_number).collect();
        let pixel_formats: StringArray = frames
            .iter()
            .map(|f| Some(f.pixel_format.as_str()))
            .collect();
        let widths: Vec<u32> = frames.iter().map(|f| f.width).collect();
        let heights: Vec<u32> = frames.iter().map(|f| f.height).collect();
        let roi_xs: Vec<u32> = frames.iter().map(|f| f.roi.x).collect();
        let roi_ys: Vec<u32> = frames.iter().map(|f| f.roi.y).collect();
        let roi_widths: Vec<u32> = frames.iter().map(|f| f.roi.width).collect();
        let roi_heights: Vec<u32> = frames.iter().map(|f| f.roi.height).collect();

        // Convert pixel data to BinaryArray
        let pixel_data_array: BinaryArray = frames
            .iter()
            .map(|f| Some(f.pixel_data.as_slice()))
            .collect();

        let batch = RecordBatch::try_new(
            SCHEMA.clone(),
            vec![
                Arc::new(TimestampNanosecondArray::from(timestamps)),
                Arc::new(UInt64Array::from(frame_numbers)),
                Arc::new(pixel_formats),
                Arc::new(UInt32Array::from(widths)),
                Arc::new(UInt32Array::from(heights)),
                Arc::new(UInt32Array::from(roi_xs)),
                Arc::new(UInt32Array::from(roi_ys)),
                Arc::new(UInt32Array::from(roi_widths)),
                Arc::new(UInt32Array::from(roi_heights)),
                Arc::new(pixel_data_array),
            ],
        )?;

        Ok(batch)
    }
}
