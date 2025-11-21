//! Core traits and data types for the DAQ application.
//!
//! This module defines the foundational abstractions for the entire data acquisition system,
//! providing trait-based interfaces for instruments, data processors, and storage backends.
//!
//! # Architecture Overview
//!
//! The core architecture follows a plugin-based design with three primary traits:
//!
//! - [`Instrument`]: Represents any physical or virtual data acquisition device
//! - [`DataProcessor`]: Transforms data points in real-time processing pipelines
//! - [`StorageWriter`]: Persists data to various storage backends (CSV, HDF5, etc.)
//!
//! # Data Flow
//!
//! ```text
//! Instrument --[DataPoint]--> DataProcessor --[DataPoint]--> StorageWriter
//!     ↓                            ↓                              ↓
//! broadcast::channel        Ring buffer cache              CSV/HDF5 file
//! ```
//!
//! # Command System
//!
//! Instruments are controlled via [`InstrumentCommand`] messages sent through
//! async channels, enabling non-blocking parameter updates and graceful shutdown.
//!
//! # Thread Safety
//!
//! All traits require `Send + Sync` to enable safe concurrent access across
//! async tasks and threads. Data streaming uses Tokio's `broadcast` channels
//! for multi-consumer patterns.
//!
//! # Examples
//!
//! ## Implementing an Instrument
//!
//!
pub use crate::core_v3::Measurement;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

/// A single data point captured from an instrument.
///
/// `DataPoint` is the fundamental unit of data in the DAQ system, representing
/// a single measurement at a specific time. All data flowing through the system
/// uses this structure, enabling uniform processing and storage.
///
/// # Fields
///
/// * `timestamp` - UTC timestamp when the measurement was captured. Uses `chrono::DateTime`
///   for nanosecond precision and timezone awareness.
/// * `channel` - Unique identifier for the data source (e.g., "laser_power", "stage_x_position").
///   Channel naming convention: `{instrument_id}_{parameter_name}`
/// * `value` - The measured value as a 64-bit float. All measurements are normalized to f64
///   regardless of the instrument's native data type.
/// * `unit` - Physical unit of the measurement (e.g., "W", "nm", "deg", "V"). Should follow
///   SI unit conventions or common scientific notation.
/// * `metadata` - Optional JSON metadata for instrument-specific information. Serialized
///   only when present. Use for context like device address, calibration coefficients, etc.
///
/// # Memory Layout
///
/// Size: ~96 bytes (timestamp: 12, channel: 24, value: 8, unit: 24, metadata: 24, padding: 4)
///
/// # Examples
///
/// ```rust
/// use rust_daq::core::DataPoint;
/// use chrono::Utc;
///
/// let dp = DataPoint {
///     timestamp: Utc::now(),
///     channel: "power_meter_1_power".to_string(),
///     value: 0.125,
///     unit: "W".to_string(),
///     metadata: Some(serde_json::json!({"wavelength": 1550.0})),
/// };
/// ```
///
/// # Serialization
///
/// DataPoint implements `Serialize`/`Deserialize` for efficient storage and transmission.
/// The metadata field is skipped during serialization if `None`, reducing storage overhead
/// for high-rate data streams.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataPoint {
    /// UTC timestamp with nanosecond precision
    pub timestamp: DateTime<Utc>,
    /// Instrument identifier (e.g., "maitai", "esp300")
    pub instrument_id: String,
    /// Channel identifier (format: `{parameter}` e.g., "power", "wavelength")
    pub channel: String,
    /// Measured value (all measurements normalized to f64)
    pub value: f64,
    /// Physical unit (SI notation recommended)
    pub unit: String,
    /// Optional instrument-specific metadata (JSON)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

// Note: Removed daq_core conversions - daq_core crate has been deleted
// All data now uses local types (core::DataPoint, core_v3::Measurement)

/// Represents a frequency bin in a spectrum measurement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrequencyBin {
    /// Frequency in Hz
    pub frequency: f64,
    /// Magnitude in dB or linear units
    pub magnitude: f64,
}

/// Memory-efficient pixel buffer supporting multiple bit depths.
///
/// `PixelBuffer` stores image data in its native format to avoid unnecessary
/// type conversions and memory bloat. Camera sensors typically output 8-bit
/// or 16-bit unsigned integers, but were previously upcast to 64-bit floats,
/// wasting 4-8× memory.
///
/// # Memory Savings
///
/// For a 2048×2048 camera frame:
/// - U8: 4 MB (1 byte/pixel)
/// - U16: 8.4 MB (2 bytes/pixel)  
/// - F64: 33.6 MB (8 bytes/pixel) ← previous implementation
///
/// Using U16 instead of F64 saves 25 MB per frame. At 10 Hz acquisition,
/// this eliminates 250 MB/s of wasted allocation and transfer.
///
/// # Variants
///
/// * `U8` - 8-bit unsigned integer pixels (0-255)
/// * `U16` - 16-bit unsigned integer pixels (0-65535)
/// * `F64` - 64-bit floating point pixels (for computed images)
///
/// # Examples
///
/// ```rust
/// use rust_daq::core::PixelBuffer;
///
/// // Camera data in native u16 format
/// let raw_frame: Vec<u16> = vec![1024, 2048, 4096];
/// let buffer = PixelBuffer::U16(raw_frame);
///
/// // Get as f64 for processing (zero-copy for F64 variant)
/// let pixels_f64 = buffer.as_f64();
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PixelBuffer {
    /// 8-bit unsigned integer pixels (1 byte/pixel)
    U8(Vec<u8>),
    /// 16-bit unsigned integer pixels (2 bytes/pixel)
    U16(Vec<u16>),
    /// 64-bit floating point pixels (8 bytes/pixel)
    F64(Vec<f64>),
}

impl PixelBuffer {
    /// Returns pixel data as f64 slice, using zero-copy for F64 variant.
    ///
    /// For U8 and U16 variants, this allocates a new Vec and converts each
    /// pixel. For F64 variant, this returns a borrowed reference with no allocation.
    ///
    /// # Performance
    ///
    /// - F64: O(1) - zero-copy borrow
    /// - U8/U16: O(n) - allocation + type conversion
    ///
    /// GUI code can use this for rendering without needing to match on variants.
    pub fn as_f64(&self) -> std::borrow::Cow<'_, [f64]> {
        use std::borrow::Cow;
        match self {
            PixelBuffer::U8(data) => Cow::Owned(data.iter().map(|&v| v as f64).collect()),
            PixelBuffer::U16(data) => Cow::Owned(data.iter().map(|&v| v as f64).collect()),
            PixelBuffer::F64(data) => Cow::Borrowed(data.as_slice()),
        }
    }

    /// Returns the number of pixels in the buffer.
    pub fn len(&self) -> usize {
        match self {
            PixelBuffer::U8(data) => data.len(),
            PixelBuffer::U16(data) => data.len(),
            PixelBuffer::F64(data) => data.len(),
        }
    }

    /// Returns true if the buffer contains no pixels.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the memory size in bytes.
    pub fn memory_bytes(&self) -> usize {
        match self {
            PixelBuffer::U8(data) => data.len(),
            PixelBuffer::U16(data) => data.len() * 2,
            PixelBuffer::F64(data) => data.len() * 8,
        }
    }
}

// Note: Removed daq_core::PixelBuffer conversion - daq_core crate deleted

/// Represents spectrum data from FFT or other frequency analysis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpectrumData {
    /// UTC timestamp when spectrum was captured
    pub timestamp: DateTime<Utc>,
    /// Channel identifier (format: `{instrument_id}_{parameter}`)
    pub channel: String,
    /// Physical unit for magnitude values
    pub unit: String,
    /// Frequency bins containing the spectrum
    pub bins: Vec<FrequencyBin>,
    /// Optional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

// Note: Removed daq_core::SpectrumData conversion - daq_core crate deleted

/// Represents image data from cameras or 2D sensors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageData {
    /// UTC timestamp when image was captured
    pub timestamp: DateTime<Utc>,
    /// Channel identifier (format: `{instrument_id}_{parameter}`)
    pub channel: String,
    /// Image width in pixels
    pub width: usize,
    /// Image height in pixels
    pub height: usize,
    /// Pixel data in native format (row-major order)
    pub pixels: PixelBuffer,
    /// Physical unit for pixel values
    pub unit: String,
    /// Optional metadata (exposure time, gain, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl ImageData {
    /// Returns pixel data as f64 for compatibility with existing code.
    ///
    /// This is a convenience method that delegates to PixelBuffer::as_f64().
    /// Use PixelBuffer directly when possible to avoid unnecessary allocations.
    ///
    /// # Performance
    ///
    /// - For F64 PixelBuffer: Zero-copy borrow
    /// - For U8/U16 PixelBuffer: Allocation + conversion
    pub fn pixels_as_f64(&self) -> std::borrow::Cow<'_, [f64]> {
        self.pixels.as_f64()
    }

    /// Returns the total number of pixels (width × height).
    pub fn pixel_count(&self) -> usize {
        self.width * self.height
    }

    /// Returns the memory size of the pixel buffer in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.pixels.memory_bytes()
    }
}

// Note: Removed daq_core::ImageData conversion - daq_core crate deleted

/// A measurement from an instrument, supporting different data types.
///
/// `Measurement` replaces the scalar-only `DataPoint` design with an extensible
/// enum that can represent scalar values, frequency spectra, images, and other
/// measurement types. This eliminates the need for JSON metadata workarounds
/// and provides type-safe access to structured data.
///
/// # Variants
///
/// * `Scalar(DataPoint)` - Traditional scalar measurement (temperature, voltage, etc.)
/// * `Spectrum(SpectrumData)` - Frequency spectrum from FFT or spectral analysis
/// * `Image(ImageData)` - 2D image data from cameras or imaging sensors
///
/// # Migration from DataPoint
///
/// Existing code using `DataPoint` can be wrapped in `Measurement::Scalar(datapoint)`.
/// New processors can emit strongly-typed variants instead of encoding data in JSON metadata.
///
/// # Examples
///
/// ```rust
/// use rust_daq::core::{Measurement, DataPoint, SpectrumData, FrequencyBin};
/// use chrono::Utc;
///
/// // Scalar measurement (traditional)
/// let scalar = Measurement::Scalar(DataPoint {
///     timestamp: Utc::now(),
///     channel: "sensor1_temperature".to_string(),
///     value: 23.5,
///     unit: "°C".to_string(),
///     metadata: None,
/// });
///
/// // Spectrum measurement (FFT output)
/// let spectrum = Measurement::Spectrum(SpectrumData {
///     timestamp: Utc::now(),
///     channel: "mic1_fft".to_string(),
///     unit: "dB".to_string(),
///     bins: vec![
///         FrequencyBin { frequency: 0.0, magnitude: -60.0 },
///         FrequencyBin { frequency: 1000.0, magnitude: -20.0 },
///     ],
///     metadata: None,
/// });
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Data {
    /// Scalar measurement (traditional DataPoint)
    Scalar(DataPoint),
    /// Frequency spectrum from FFT or spectral analysis
    Spectrum(SpectrumData),
    /// 2D image data from cameras or imaging sensors
    Image(ImageData),
}

impl Data {
    /// Returns the timestamp of this measurement.
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Data::Scalar(dp) => dp.timestamp,
            Data::Spectrum(sd) => sd.timestamp,
            Data::Image(id) => id.timestamp,
        }
    }

    /// Returns the channel identifier of this measurement.
    pub fn channel(&self) -> &str {
        match self {
            Data::Scalar(dp) => &dp.channel,
            Data::Spectrum(sd) => &sd.channel,
            Data::Image(id) => &id.channel,
        }
    }

    /// Returns the unit of this measurement.
    pub fn unit(&self) -> &str {
        match self {
            Data::Scalar(dp) => &dp.unit,
            Data::Spectrum(sd) => &sd.unit,
            Data::Image(id) => &id.unit,
        }
    }

    /// Returns the metadata of this measurement, if any.
    pub fn metadata(&self) -> Option<&serde_json::Value> {
        match self {
            Data::Scalar(dp) => dp.metadata.as_ref(),
            Data::Spectrum(sd) => sd.metadata.as_ref(),
            Data::Image(id) => id.metadata.as_ref(),
        }
    }
}

// Note: Removed Data to daq_core::Measurement conversion - daq_core crate deleted

/// Strongly-typed argument for capability operations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ParameterValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    FloatArray(Vec<f64>),
    IntArray(Vec<i64>),
    Array(Vec<ParameterValue>),
    Object(HashMap<String, ParameterValue>),
    Null,
}

impl fmt::Display for ParameterValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParameterValue::Bool(b) => write!(f, "{}", b),
            ParameterValue::Int(i) => write!(f, "{}", i),
            ParameterValue::Float(fl) => write!(f, "{}", fl),
            ParameterValue::String(s) => write!(f, "{}", s),
            ParameterValue::FloatArray(arr) => write!(f, "{:?}", arr),
            ParameterValue::IntArray(arr) => write!(f, "{:?}", arr),
            ParameterValue::Array(arr) => write!(f, "{:?}", arr),
            ParameterValue::Object(obj) => write!(f, "{:?}", obj),
            ParameterValue::Null => write!(f, "null"),
        }
    }
}

impl ParameterValue {
    /// Extract value as a string, parsing from various types
    pub fn as_string(&self) -> Option<String> {
        match self {
            ParameterValue::String(s) => Some(s.clone()),
            ParameterValue::Bool(b) => Some(b.to_string()),
            ParameterValue::Int(i) => Some(i.to_string()),
            ParameterValue::Float(f) => Some(f.to_string()),
            _ => None,
        }
    }

    /// Extract value as f64
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ParameterValue::Float(f) => Some(*f),
            ParameterValue::Int(i) => Some(*i as f64),
            ParameterValue::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    /// Extract value as i64
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            ParameterValue::Int(i) => Some(*i),
            ParameterValue::Float(f) => Some(*f as i64),
            ParameterValue::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    /// Extract value as bool
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ParameterValue::Bool(b) => Some(*b),
            ParameterValue::String(s) => s.parse().ok(),
            _ => None,
        }
    }
}

impl From<bool> for ParameterValue {
    fn from(value: bool) -> Self {
        ParameterValue::Bool(value)
    }
}

impl From<i64> for ParameterValue {
    fn from(value: i64) -> Self {
        ParameterValue::Int(value)
    }
}

impl From<u64> for ParameterValue {
    fn from(value: u64) -> Self {
        ParameterValue::Int(value as i64)
    }
}

impl From<u32> for ParameterValue {
    fn from(value: u32) -> Self {
        ParameterValue::Int(value as i64)
    }
}

impl From<u8> for ParameterValue {
    fn from(value: u8) -> Self {
        ParameterValue::Int(value as i64)
    }
}

impl From<f64> for ParameterValue {
    fn from(value: f64) -> Self {
        ParameterValue::Float(value)
    }
}

impl From<&str> for ParameterValue {
    fn from(value: &str) -> Self {
        ParameterValue::String(value.to_string())
    }
}

impl From<String> for ParameterValue {
    fn from(value: String) -> Self {
        ParameterValue::String(value)
    }
}

impl From<Vec<f64>> for ParameterValue {
    fn from(value: Vec<f64>) -> Self {
        ParameterValue::FloatArray(value)
    }
}

impl From<Vec<i64>> for ParameterValue {
    fn from(value: Vec<i64>) -> Self {
        ParameterValue::IntArray(value)
    }
}

impl From<Vec<i32>> for ParameterValue {
    fn from(value: Vec<i32>) -> Self {
        ParameterValue::IntArray(value.into_iter().map(|x| x as i64).collect())
    }
}

/// Trait for processing measurements with support for different data types.
///
/// `MeasurementProcessor` is the next-generation processor interface that works
/// with the `Measurement` enum instead of scalar-only `DataPoint`. This enables
/// processors to emit and consume structured data like frequency spectra and images
/// without JSON metadata workarounds.
///
/// # Design Philosophy
///
/// - **Type Safety**: Processors declare the specific measurement types they work with
/// - **Composability**: Processors can transform one measurement type to another
/// - **Efficiency**: Structured data avoids serialization/deserialization overhead
/// - **Extensibility**: New measurement types can be added without breaking existing code
///
/// # Examples
///
/// ```rust
/// use rust_daq::core::{MeasurementProcessor, Measurement, DataPoint, SpectrumData, FrequencyBin};
/// use chrono::Utc;
///
/// struct FFTProcessor;
///
/// impl MeasurementProcessor for FFTProcessor {
///     fn process_measurements(&mut self, data: &[Measurement]) -> Vec<Measurement> {
///         let mut spectra = Vec::new();
///         for measurement in data {
///             if let Measurement::Scalar(_dp) = measurement {
///                 // Convert scalar time-series to spectrum (simplified)
///                 let spectrum_data = SpectrumData {
///                     timestamp: Utc::now(),
///                     channel: "example_fft".to_string(),
///                     unit: "dB".to_string(),
///                     bins: vec![FrequencyBin { frequency: 1000.0, magnitude: -20.0 }],
///                     metadata: None,
///                 };
///                 spectra.push(Measurement::Spectrum(spectrum_data));
///             }
///         }
///         spectra
///     }
/// }
/// ```
///
/// # Migration Path
///
/// Existing `DataProcessor` implementations can be wrapped:
///
/// ```rust
/// # use rust_daq::core::{MeasurementProcessor, Measurement, DataPoint, DataProcessor};
/// # struct LegacyFilter;
/// # impl DataProcessor for LegacyFilter {
/// #     fn process(&mut self, data: &[DataPoint]) -> Vec<DataPoint> { data.to_vec() }
/// # }
/// impl MeasurementProcessor for LegacyFilter {
///     fn process_measurements(&mut self, data: &[Arc<Measurement>]) -> Vec<Arc<Measurement>> {
///         // Extract scalar data points and process with legacy DataProcessor
///         let scalars: Vec<DataPoint> = data.iter()
///             .filter_map(|m| if let Measurement::Scalar(dp) = m.as_ref() { Some(dp.clone()) } else { None })
///             .collect();
///         let filtered = self.process(&scalars); // Call legacy DataProcessor::process
///         filtered.into_iter().map(|dp| {
///             Arc::new(Measurement::Scalar(dp))
///         }).collect()
///     }
/// }
/// ```
pub trait MeasurementProcessor: Send + Sync {
    /// Processes a batch of measurements and returns transformed measurements.
    ///
    /// # Arguments
    ///
    /// * `data` - Input slice of Arc-wrapped measurements to process. May contain mixed types.
    ///
    /// # Returns
    ///
    /// Vector of processed Arc-wrapped measurements. The processor may:
    /// - Filter input measurements (e.g., only process Scalar measurements)
    /// - Transform measurement types (e.g., Scalar → Spectrum via FFT)
    /// - Combine multiple measurements into one (e.g., stereo → mono)
    /// - Generate multiple outputs from one input (e.g., image → histogram + stats)
    ///
    /// # Type Conversions
    ///
    /// Common patterns:
    /// - `Scalar → Scalar`: Traditional filtering, calibration
    /// - `Scalar → Spectrum`: FFT, spectral analysis
    /// - `Image → Scalar`: Statistics (mean, max, etc.)
    /// - `Spectrum → Scalar`: Peak detection, power calculation
    ///
    /// # Performance
    ///
    /// Arc-wrapping enables zero-copy sharing of measurements between processors,
    /// storage, and GUI without cloning large data arrays.
    fn process_measurements(&mut self, data: &[Arc<Measurement>]) -> Vec<Arc<Measurement>>;
}

//==============================================================================
// Phase 2 Complete: V2 Infrastructure Ready (bd-62)
//==============================================================================
//
// V2InstrumentAdapter removed in Phase 2 (bd-62). The app infrastructure now
// uses Arc<Measurement> natively, supporting Scalar, Spectrum, and Image data.
//
// V2 instruments (MockInstrumentV2, etc.) will be integrated in Phase 3 (bd-51)
// via a native V2 InstrumentRegistry that works directly with daq_core::Instrument.
//
// Migration path:
// - Phase 1 (bd-49): Created V2InstrumentAdapter as temporary bridge
// - Phase 2 (bd-62): Updated infrastructure for Arc<Measurement>, removed adapter
// - Phase 3 (bd-51): Implement native V2 instrument support
//
//==============================================================================
