//! Core traits and data types for the DAQ application (V5 Architecture).
//!
//! This module defines the foundational abstractions for the entire data acquisition system,
//! providing trait-based interfaces for instruments, data processors, and storage backends.
//!
//! # Architecture Overview
//!
//! The V5 architecture uses capability-based traits:
//!
//! - [`Instrument`]: Base trait for all instruments with lifecycle management
//! - [`Camera`], [`Stage`], [`Laser`], etc.: Capability traits for specific functionality
//! - [`Measurement`]: Unified measurement type supporting scalars, vectors, images, spectra
//! - [`MeasurementProcessor`]: Transform measurements in real-time processing pipelines
//!
//! # Data Flow
//!
//! ```text
//! Instrument --[Measurement]--> broadcast::channel ---> Processors/Storage/GUI
//! ```
//!
//! # Thread Safety
//!
//! All traits require `Send + Sync` to enable safe concurrent access across
//! async tasks and threads. Data streaming uses Tokio's `broadcast` channels
//! for multi-consumer patterns.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

// =============================================================================
// Basic Data Types
// =============================================================================

/// A single data point captured from an instrument (legacy V1 structure).
///
/// `DataPoint` is maintained for backwards compatibility but new code should
/// use the `Measurement` enum which supports structured data types.
///
/// # Fields
///
/// * `timestamp` - UTC timestamp when the measurement was captured
/// * `instrument_id` - Instrument identifier (e.g., "maitai", "esp300")
/// * `channel` - Channel identifier (e.g., "power", "wavelength")
/// * `value` - Measured value (all measurements normalized to f64)
/// * `unit` - Physical unit (SI notation recommended)
/// * `metadata` - Optional instrument-specific metadata (JSON)
#[deprecated(since = "0.5.0", note = "Use Measurement enum instead")]
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
/// or 16-bit unsigned integers.
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

/// Region of Interest for camera acquisition
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Roi {
    /// X-coordinate of top-left corner in pixels
    pub x: u32,
    /// Y-coordinate of top-left corner in pixels
    pub y: u32,
    /// Width of ROI in pixels
    pub width: u32,
    /// Height of ROI in pixels
    pub height: u32,
}

impl PartialOrd for Roi {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Roi {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Compare by area first, then by position
        let self_area = self.width * self.height;
        let other_area = other.width * other.height;

        match self_area.cmp(&other_area) {
            std::cmp::Ordering::Equal => {
                // If equal area, compare by top-left position
                match self.x.cmp(&other.x) {
                    std::cmp::Ordering::Equal => self.y.cmp(&other.y),
                    other => other,
                }
            }
            other => other,
        }
    }
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

/// Image metadata (exposure, gain, etc.)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageMetadata {
    /// Exposure time in milliseconds.
    ///
    /// `None` if exposure is unknown or not applicable (e.g., pre-captured images).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exposure_ms: Option<f64>,
    /// Camera gain multiplier (unitless).
    ///
    /// `None` if gain is not applicable or not set. Common range: 1.0-16.0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gain: Option<f64>,
    /// Binning factors (horizontal, vertical).
    ///
    /// `None` if no binning is applied. (1, 1) represents no binning, (2, 2) bins 2×2 pixels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binning: Option<(u32, u32)>,
    /// Sensor temperature in degrees Celsius.
    ///
    /// `None` if temperature reading is unavailable. Negative values indicate cooling below ambient.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_c: Option<f64>,
    /// Hardware timestamp from camera in microseconds.
    ///
    /// `None` if camera does not provide hardware timestamps. Used for precise inter-frame timing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hardware_timestamp_us: Option<i64>,
    /// Frame readout duration in milliseconds.
    ///
    /// `None` if readout time is unknown. Represents time from exposure end to data availability.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readout_ms: Option<f64>,
    /// ROI origin (x, y) in full sensor coordinates.
    ///
    /// `None` if ROI matches full sensor area. Useful for reconstructing position in full frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roi_origin: Option<(u32, u32)>,
}

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

/// Unified measurement representation (V5 architecture).
///
/// All instruments emit this enum directly. Supports scalars, vectors, images, and spectra.
///
/// # Variants
///
/// * `Scalar` - Single scalar value (temperature, voltage, power, etc.)
/// * `Vector` - Array of values (spectrum, time series)
/// * `Image` - 2D image data with zero-copy optimization
/// * `Spectrum` - Frequency spectrum with frequency/amplitude pairs
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Measurement {
    /// Single scalar value with metadata
    Scalar {
        /// Measurement name/identifier (e.g., "power", "temperature")
        name: String,
        /// Measured value
        value: f64,
        /// Physical unit (SI notation recommended, e.g., "W", "K", "V")
        unit: String,
        /// UTC timestamp when measurement was captured
        timestamp: DateTime<Utc>,
    },

    /// Vector of values (e.g., spectrum, time series)
    Vector {
        /// Measurement name/identifier (e.g., "waveform", "time_series")
        name: String,
        /// Array of measured values
        values: Vec<f64>,
        /// Physical unit for all values (SI notation recommended)
        unit: String,
        /// UTC timestamp when measurement was captured
        timestamp: DateTime<Utc>,
    },

    /// 2D image data with zero-copy optimization
    Image {
        /// Measurement name/identifier (e.g., "camera_frame", "thermal_image")
        name: String,
        /// Image width in pixels
        width: u32,
        /// Image height in pixels
        height: u32,
        /// Pixel data in native format (row-major order)
        buffer: PixelBuffer,
        /// Physical unit for pixel values (e.g., "counts", "photons", "ADU")
        unit: String,
        /// Camera-specific metadata (exposure, gain, temperature, etc.)
        metadata: ImageMetadata,
        /// UTC timestamp when image was captured
        timestamp: DateTime<Utc>,
    },

    /// Spectrum with frequency/amplitude pairs
    Spectrum {
        /// Measurement name/identifier (e.g., "fft", "absorption_spectrum")
        name: String,
        /// Frequency values for each spectral bin
        frequencies: Vec<f64>,
        /// Amplitude values corresponding to each frequency
        amplitudes: Vec<f64>,
        /// Physical unit for frequency values (e.g., "Hz", "nm").
        ///
        /// `None` if unit is unknown or not applicable.
        frequency_unit: Option<String>,
        /// Physical unit for amplitude values (e.g., "dB", "V", "W").
        ///
        /// `None` if unit is unknown or not applicable.
        amplitude_unit: Option<String>,
        /// Optional spectrum-specific metadata (resolution, window function, etc.).
        ///
        /// `None` if no additional metadata is available.
        metadata: Option<Value>,
        /// UTC timestamp when spectrum was captured
        timestamp: DateTime<Utc>,
    },
}

impl Measurement {
    /// Extract timestamp regardless of variant
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Measurement::Scalar { timestamp, .. } => *timestamp,
            Measurement::Vector { timestamp, .. } => *timestamp,
            Measurement::Image { timestamp, .. } => *timestamp,
            Measurement::Spectrum { timestamp, .. } => *timestamp,
        }
    }

    /// Extract name regardless of variant
    pub fn name(&self) -> &str {
        match self {
            Measurement::Scalar { name, .. } => name,
            Measurement::Vector { name, .. } => name,
            Measurement::Image { name, .. } => name,
            Measurement::Spectrum { name, .. } => name,
        }
    }
}

/// Arrow RecordBatch conversion for zero-copy batch processing.
///
/// This implementation provides efficient columnar storage for measurements,
/// enabling high-throughput data pipelines with Arrow-based analytics.
#[cfg(feature = "storage_arrow")]
impl Measurement {
    /// Convert a batch of measurements to Arrow RecordBatch for efficient columnar storage.
    ///
    /// Returns separate RecordBatches for each measurement type since they have different schemas.
    /// Measurements are grouped by type and converted to columnar format.
    ///
    /// # Arguments
    /// * `measurements` - Slice of measurements to convert
    ///
    /// # Returns
    /// A tuple of optional RecordBatches: (scalars, vectors, spectra)
    /// Images are not included as they require special handling for binary data.
    pub fn into_arrow_batches(
        measurements: &[Measurement],
    ) -> Result<ArrowBatches, arrow::error::ArrowError> {
        use arrow::array::{
            ArrayRef, Float64Array, Float64Builder, ListBuilder, StringBuilder,
            TimestampNanosecondArray,
        };
        use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
        use arrow::record_batch::RecordBatch;
        use std::sync::Arc;

        // Separate measurements by type
        let mut scalars: Vec<&Measurement> = Vec::new();
        let mut vectors: Vec<&Measurement> = Vec::new();
        let mut spectra: Vec<&Measurement> = Vec::new();
        let mut images: Vec<&Measurement> = Vec::new();

        for m in measurements {
            match m {
                Measurement::Scalar { .. } => scalars.push(m),
                Measurement::Vector { .. } => vectors.push(m),
                Measurement::Spectrum { .. } => spectra.push(m),
                Measurement::Image { .. } => images.push(m),
            }
        }

        // Build scalar batch
        let scalar_batch = if !scalars.is_empty() {
            let schema = Arc::new(Schema::new(vec![
                Field::new("name", DataType::Utf8, false),
                Field::new("value", DataType::Float64, false),
                Field::new("unit", DataType::Utf8, false),
                Field::new(
                    "timestamp_ns",
                    DataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into())),
                    false,
                ),
            ]));

            let names: Vec<&str> = scalars
                .iter()
                .filter_map(|m| match m {
                    Measurement::Scalar { name, .. } => Some(name.as_str()),
                    _ => None,
                })
                .collect();
            let values: Vec<f64> = scalars
                .iter()
                .filter_map(|m| match m {
                    Measurement::Scalar { value, .. } => Some(*value),
                    _ => None,
                })
                .collect();
            let units: Vec<&str> = scalars
                .iter()
                .filter_map(|m| match m {
                    Measurement::Scalar { unit, .. } => Some(unit.as_str()),
                    _ => None,
                })
                .collect();
            let timestamps: Vec<i64> = scalars
                .iter()
                .filter_map(|m| match m {
                    Measurement::Scalar { timestamp, .. } => {
                        Some(timestamp.timestamp_nanos_opt().unwrap_or(0))
                    }
                    _ => None,
                })
                .collect();

            let columns: Vec<ArrayRef> = vec![
                Arc::new(arrow::array::StringArray::from(names)),
                Arc::new(Float64Array::from(values)),
                Arc::new(arrow::array::StringArray::from(units)),
                Arc::new(TimestampNanosecondArray::from(timestamps).with_timezone("UTC")),
            ];

            Some(RecordBatch::try_new(schema, columns)?)
        } else {
            None
        };

        // Build vector batch (values stored as List<Float64>)
        let vector_batch = if !vectors.is_empty() {
            let schema = Arc::new(Schema::new(vec![
                Field::new("name", DataType::Utf8, false),
                Field::new(
                    "values",
                    DataType::List(Arc::new(Field::new("item", DataType::Float64, true))),
                    false,
                ),
                Field::new("unit", DataType::Utf8, false),
                Field::new(
                    "timestamp_ns",
                    DataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into())),
                    false,
                ),
            ]));

            let mut name_builder = StringBuilder::new();
            let mut values_builder = ListBuilder::new(Float64Builder::new());
            let mut unit_builder = StringBuilder::new();
            let mut timestamp_builder: Vec<i64> = Vec::new();

            for m in &vectors {
                if let Measurement::Vector {
                    name,
                    values,
                    unit,
                    timestamp,
                } = m
                {
                    name_builder.append_value(name);
                    for v in values {
                        values_builder.values().append_value(*v);
                    }
                    values_builder.append(true);
                    unit_builder.append_value(unit);
                    timestamp_builder.push(timestamp.timestamp_nanos_opt().unwrap_or(0));
                }
            }

            let columns: Vec<ArrayRef> = vec![
                Arc::new(name_builder.finish()),
                Arc::new(values_builder.finish()),
                Arc::new(unit_builder.finish()),
                Arc::new(TimestampNanosecondArray::from(timestamp_builder).with_timezone("UTC")),
            ];

            Some(RecordBatch::try_new(schema, columns)?)
        } else {
            None
        };

        // Build spectrum batch
        let spectrum_batch = if !spectra.is_empty() {
            let schema = Arc::new(Schema::new(vec![
                Field::new("name", DataType::Utf8, false),
                Field::new(
                    "frequencies",
                    DataType::List(Arc::new(Field::new("item", DataType::Float64, true))),
                    false,
                ),
                Field::new(
                    "amplitudes",
                    DataType::List(Arc::new(Field::new("item", DataType::Float64, true))),
                    false,
                ),
                Field::new("frequency_unit", DataType::Utf8, true),
                Field::new("amplitude_unit", DataType::Utf8, true),
                Field::new(
                    "timestamp_ns",
                    DataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into())),
                    false,
                ),
            ]));

            let mut name_builder = StringBuilder::new();
            let mut freq_builder = ListBuilder::new(Float64Builder::new());
            let mut amp_builder = ListBuilder::new(Float64Builder::new());
            let mut freq_unit_builder = StringBuilder::new();
            let mut amp_unit_builder = StringBuilder::new();
            let mut timestamp_builder: Vec<i64> = Vec::new();

            for m in &spectra {
                if let Measurement::Spectrum {
                    name,
                    frequencies,
                    amplitudes,
                    frequency_unit,
                    amplitude_unit,
                    timestamp,
                    ..
                } = m
                {
                    name_builder.append_value(name);
                    for f in frequencies {
                        freq_builder.values().append_value(*f);
                    }
                    freq_builder.append(true);
                    for a in amplitudes {
                        amp_builder.values().append_value(*a);
                    }
                    amp_builder.append(true);
                    match frequency_unit {
                        Some(u) => freq_unit_builder.append_value(u),
                        None => freq_unit_builder.append_null(),
                    }
                    match amplitude_unit {
                        Some(u) => amp_unit_builder.append_value(u),
                        None => amp_unit_builder.append_null(),
                    }
                    timestamp_builder.push(timestamp.timestamp_nanos_opt().unwrap_or(0));
                }
            }

            let columns: Vec<ArrayRef> = vec![
                Arc::new(name_builder.finish()),
                Arc::new(freq_builder.finish()),
                Arc::new(amp_builder.finish()),
                Arc::new(freq_unit_builder.finish()),
                Arc::new(amp_unit_builder.finish()),
                Arc::new(TimestampNanosecondArray::from(timestamp_builder).with_timezone("UTC")),
            ];

            Some(RecordBatch::try_new(schema, columns)?)
        } else {
            None
        };

        // Build image batch (pixel data stored as LargeBinary, metadata as strings/integers)
        let image_batch = if !images.is_empty() {
            let schema = Arc::new(Schema::new(vec![
                Field::new("name", DataType::Utf8, false),
                Field::new("width", DataType::UInt32, false),
                Field::new("height", DataType::UInt32, false),
                Field::new("unit", DataType::Utf8, false),
                Field::new("dtype", DataType::Utf8, false), // "U8", "U16", or "F64"
                Field::new("pixels", DataType::LargeBinary, false),
                Field::new("exposure_ms", DataType::Float64, true),
                Field::new("gain", DataType::Float64, true),
                Field::new(
                    "timestamp_ns",
                    DataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into())),
                    false,
                ),
            ]));

            let mut name_builder = StringBuilder::new();
            let mut width_builder: Vec<u32> = Vec::new();
            let mut height_builder: Vec<u32> = Vec::new();
            let mut unit_builder = StringBuilder::new();
            let mut dtype_builder = StringBuilder::new();
            let mut pixels_builder = arrow::array::LargeBinaryBuilder::new();
            let mut exposure_builder = Float64Builder::new();
            let mut gain_builder = Float64Builder::new();
            let mut timestamp_builder: Vec<i64> = Vec::new();

            for m in &images {
                if let Measurement::Image {
                    name,
                    width,
                    height,
                    buffer,
                    unit,
                    metadata,
                    timestamp,
                } = m
                {
                    name_builder.append_value(name);
                    width_builder.push(*width);
                    height_builder.push(*height);
                    unit_builder.append_value(unit);

                    // Store pixel data as binary blob with dtype tag
                    match buffer {
                        PixelBuffer::U8(data) => {
                            dtype_builder.append_value("U8");
                            pixels_builder.append_value(data);
                        }
                        PixelBuffer::U16(data) => {
                            dtype_builder.append_value("U16");
                            // Convert u16 slice to bytes (little-endian)
                            let bytes: Vec<u8> =
                                data.iter().flat_map(|&v| v.to_le_bytes()).collect();
                            pixels_builder.append_value(&bytes);
                        }
                        PixelBuffer::F64(data) => {
                            dtype_builder.append_value("F64");
                            // Convert f64 slice to bytes (little-endian)
                            let bytes: Vec<u8> =
                                data.iter().flat_map(|&v| v.to_le_bytes()).collect();
                            pixels_builder.append_value(&bytes);
                        }
                    }

                    // Store optional metadata
                    if let Some(exp) = metadata.exposure_ms {
                        exposure_builder.append_value(exp);
                    } else {
                        exposure_builder.append_null();
                    }

                    if let Some(g) = metadata.gain {
                        gain_builder.append_value(g);
                    } else {
                        gain_builder.append_null();
                    }

                    timestamp_builder.push(timestamp.timestamp_nanos_opt().unwrap_or(0));
                }
            }

            let columns: Vec<ArrayRef> = vec![
                Arc::new(name_builder.finish()),
                Arc::new(arrow::array::UInt32Array::from(width_builder)),
                Arc::new(arrow::array::UInt32Array::from(height_builder)),
                Arc::new(unit_builder.finish()),
                Arc::new(dtype_builder.finish()),
                Arc::new(pixels_builder.finish()),
                Arc::new(exposure_builder.finish()),
                Arc::new(gain_builder.finish()),
                Arc::new(TimestampNanosecondArray::from(timestamp_builder).with_timezone("UTC")),
            ];

            Some(RecordBatch::try_new(schema, columns)?)
        } else {
            None
        };

        Ok(ArrowBatches {
            scalars: scalar_batch,
            vectors: vector_batch,
            spectra: spectrum_batch,
            images: image_batch,
        })
    }
}

/// Container for Arrow RecordBatches organized by measurement type.
#[cfg(feature = "storage_arrow")]
#[derive(Debug)]
pub struct ArrowBatches {
    /// RecordBatch containing scalar measurements
    pub scalars: Option<arrow::record_batch::RecordBatch>,
    /// RecordBatch containing vector measurements
    pub vectors: Option<arrow::record_batch::RecordBatch>,
    /// RecordBatch containing spectrum measurements
    pub spectra: Option<arrow::record_batch::RecordBatch>,
    /// RecordBatch containing image measurements
    pub images: Option<arrow::record_batch::RecordBatch>,
}

/// Legacy Data enum (being replaced by Measurement).
///
/// Kept for backwards compatibility during migration.
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

/// Strongly-typed argument for capability operations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ParameterValue {
    /// Boolean value (true/false)
    Bool(bool),
    /// 64-bit signed integer
    Int(i64),
    /// 64-bit floating point number
    Float(f64),
    /// UTF-8 string
    String(String),
    /// Array of 64-bit floats (e.g., position array, calibration data)
    FloatArray(Vec<f64>),
    /// Array of 64-bit signed integers (e.g., pixel counts, bin indices)
    IntArray(Vec<i64>),
    /// Nested array of parameter values (for complex structures)
    Array(Vec<ParameterValue>),
    /// Key-value map of parameter values (for structured configuration)
    Object(HashMap<String, ParameterValue>),
    /// Null/None value (represents absence of data)
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

// =============================================================================
// Instrument State and Commands (V5)
// =============================================================================

/// Instrument lifecycle state
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstrumentState {
    /// Instrument object created but not yet initialized
    Uninitialized,
    /// Instrument is not connected to hardware
    Disconnected,
    /// Instrument is in the process of connecting
    Connecting,
    /// Instrument is connected and ready to operate
    Connected,
    /// Connected and ready (alias for Connected for V2 compatibility)
    Idle,
    /// Currently acquiring/operating
    Running,
    /// Paused (can resume)
    Paused,
    /// Error state (see error message)
    Error,
    /// Shutting down
    ShuttingDown,
}

/// Generic command envelope for instrument control
///
/// Replaces the complex InstrumentCommand enum. Instruments handle
/// commands via their trait methods instead.
#[derive(Clone, Debug)]
pub enum Command {
    /// Start acquisition/operation
    Start,
    /// Stop acquisition/operation
    Stop,
    /// Pause acquisition/operation
    Pause,
    /// Resume from pause
    Resume,
    /// Request current state
    GetState,
    /// Request parameter value
    GetParameter(String),
    /// Set parameter value (parameter name, JSON value)
    SetParameter(String, serde_json::Value),
    /// Configure multiple parameters at once
    Configure {
        /// Map of parameter names to values for batch configuration
        params: HashMap<String, ParameterValue>,
    },
    /// Instrument-specific command (for specialized operations)
    Custom(String, serde_json::Value),
}

/// Response to command execution
#[derive(Clone, Debug)]
pub enum Response {
    /// Command completed successfully
    Ok,
    /// Command completed with state update
    State(InstrumentState),
    /// Command completed with parameter value
    Parameter(serde_json::Value),
    /// Command completed with custom data
    Custom(serde_json::Value),
    /// Command failed with error message
    Error(String),
}

// =============================================================================
// Parameter Base Trait (for dynamic access)
// =============================================================================

/// Base trait for all parameters (enables heterogeneous collections)
///
/// Concrete parameters use `Parameter<T>` (see parameter.rs).
pub trait ParameterBase: Send + Sync {
    /// Parameter name
    fn name(&self) -> &str;

    /// Get current value as JSON
    fn value_json(&self) -> serde_json::Value;

    /// Set value from JSON
    fn set_json(&mut self, value: serde_json::Value) -> Result<()>;

    /// Get parameter constraints as JSON
    fn constraints_json(&self) -> serde_json::Value;
}

// =============================================================================
// Core Instrument Trait (V5 Unified Architecture)
// =============================================================================

/// Base trait for all instruments (V5 unified architecture).
///
/// All instruments implement this trait directly. No wrapper types needed.
/// Instruments run in their own Tokio tasks and communicate via channels.
///
/// # Data Flow
///
/// ```text
/// Instrument Task → data_channel() → broadcast::Receiver<Measurement>
///                                    ↓
///                                   GUI/Storage/Processors subscribe directly
/// ```
///
/// # Command Flow
///
/// ```text
/// Manager → execute(cmd) → Instrument implementation
/// ```
#[async_trait]
pub trait Instrument: Send + Sync {
    /// Unique instrument identifier
    fn id(&self) -> &str;

    /// Current lifecycle state
    fn state(&self) -> InstrumentState;

    /// Initialize hardware connection
    ///
    /// Called once before instrument can be used. Should establish
    /// hardware connection, verify communication, and prepare for operation.
    async fn initialize(&mut self) -> Result<()>;

    /// Shutdown hardware connection gracefully
    ///
    /// Called during application shutdown or instrument removal.
    /// Should release hardware resources and clean up.
    async fn shutdown(&mut self) -> Result<()>;

    /// Subscribe to data stream
    ///
    /// Returns a broadcast receiver for measurements. Multiple subscribers
    /// can receive the same data stream independently.
    fn data_channel(&self) -> broadcast::Receiver<Measurement>;

    /// Execute command (direct async call, no message passing)
    ///
    /// Replaces the old InstrumentCommand enum with direct method dispatch.
    /// Instruments can implement custom command handling as needed.
    async fn execute(&mut self, cmd: Command) -> Result<Response>;

    /// Access instrument parameters
    ///
    /// Returns reference to parameter collection for introspection and
    /// dynamic access (e.g., GUI parameter editors).
    fn parameters(&self) -> &HashMap<String, Box<dyn ParameterBase>>;

    /// Get mutable access to parameters (for setting)
    fn parameters_mut(&mut self) -> &mut HashMap<String, Box<dyn ParameterBase>>;
}

// =============================================================================
// Capability Traits (V5 Meta Instrument Pattern)
// =============================================================================

/// Camera capability trait
///
/// Modules that require camera functionality should work with this trait
/// instead of concrete camera types. This enables hardware-agnostic
/// experiment logic.
#[async_trait]
pub trait Camera: Instrument {
    /// Set exposure time in milliseconds
    async fn set_exposure(&mut self, ms: f64) -> Result<()>;

    /// Set region of interest
    async fn set_roi(&mut self, roi: Roi) -> Result<()>;

    /// Get current ROI
    async fn roi(&self) -> Roi;

    /// Set binning (horizontal, vertical)
    async fn set_binning(&mut self, h: u32, v: u32) -> Result<()>;

    /// Start continuous acquisition
    async fn start_acquisition(&mut self) -> Result<()>;

    /// Stop acquisition
    async fn stop_acquisition(&mut self) -> Result<()>;

    /// Arm camera for triggered acquisition
    async fn arm_trigger(&mut self) -> Result<()>;

    /// Software trigger (if supported)
    async fn trigger(&mut self) -> Result<()>;
}

/// Stage/positioner capability trait
///
/// Modules that control motion should work with this trait for
/// hardware-agnostic positioning logic.
#[async_trait]
pub trait Stage: Instrument {
    /// Move to absolute position in mm
    async fn move_absolute(&mut self, position_mm: f64) -> Result<()>;

    /// Move relative to current position in mm
    async fn move_relative(&mut self, distance_mm: f64) -> Result<()>;

    /// Get current position in mm
    async fn position(&self) -> Result<f64>;

    /// Stop motion immediately
    async fn stop_motion(&mut self) -> Result<()>;

    /// Check if stage is currently moving
    async fn is_moving(&self) -> Result<bool>;

    /// Home stage (find reference position)
    async fn home(&mut self) -> Result<()>;

    /// Set velocity in mm/s
    async fn set_velocity(&mut self, mm_per_sec: f64) -> Result<()>;

    /// Wait for motion to settle (with timeout)
    async fn wait_settled(&self, timeout: std::time::Duration) -> Result<()> {
        let start = std::time::Instant::now();
        loop {
            if !self.is_moving().await? {
                return Ok(());
            }
            if start.elapsed() > timeout {
                return Err(anyhow::anyhow!("Timeout waiting for motion to settle"));
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
}

/// Spectrometer capability trait
#[async_trait]
pub trait Spectrometer: Instrument {
    /// Set integration time in milliseconds
    async fn set_integration_time(&mut self, ms: f64) -> Result<()>;

    /// Get wavelength range
    fn wavelength_range(&self) -> (f64, f64);

    /// Start spectrum acquisition
    async fn start_acquisition(&mut self) -> Result<()>;

    /// Stop acquisition
    async fn stop_acquisition(&mut self) -> Result<()>;
}

/// Power meter capability trait
#[async_trait]
pub trait PowerMeter: Instrument {
    /// Set wavelength for calibration (nm)
    async fn set_wavelength(&mut self, nm: f64) -> Result<()>;

    /// Set measurement range (watts)
    async fn set_range(&mut self, watts: f64) -> Result<()>;

    /// Zero/calibrate sensor
    async fn zero(&mut self) -> Result<()>;
}

/// Laser capability trait
///
/// V5 Design: Control methods for tunable lasers with wavelength/power control.
/// Power/wavelength readings are broadcast via Instrument::data_channel().
#[async_trait]
pub trait Laser: Instrument {
    /// Set wavelength in nanometers (for tunable lasers)
    async fn set_wavelength(&mut self, nm: f64) -> Result<()>;

    /// Get current wavelength setting in nanometers
    async fn wavelength(&self) -> Result<f64>;

    /// Set output power in watts
    async fn set_power(&mut self, watts: f64) -> Result<()>;

    /// Get current power output in watts
    async fn power(&self) -> Result<f64>;

    /// Enable shutter (allow laser emission)
    async fn enable_shutter(&mut self) -> Result<()>;

    /// Disable shutter (block laser emission)
    async fn disable_shutter(&mut self) -> Result<()>;

    /// Check if shutter is enabled (laser can emit)
    async fn is_enabled(&self) -> Result<bool>;
}

// =============================================================================
// Instrument Handle (Direct Management)
// =============================================================================

/// Handle for managing instrument lifecycle and communication
///
/// Replaces the actor-based management. Each instrument runs in a task,
/// and the handle provides direct access to channels and lifecycle control.
pub struct InstrumentHandle {
    /// Instrument identifier
    pub id: String,

    /// Tokio task handle (for monitoring and cancellation)
    pub task: JoinHandle<Result<()>>,

    /// Shutdown signal sender
    pub shutdown_tx: oneshot::Sender<()>,

    /// Data broadcast receiver (subscribe to get measurements)
    pub data_rx: broadcast::Receiver<Measurement>,

    /// Command channel for instrument control
    pub command_tx: mpsc::Sender<Command>,

    /// Reference to instrument (for capability downcasting)
    pub instrument: Arc<tokio::sync::Mutex<Box<dyn Instrument>>>,
}

impl std::fmt::Debug for InstrumentHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstrumentHandle")
            .field("id", &self.id)
            .field("task", &"<JoinHandle>")
            .field("shutdown_tx", &"<oneshot::Sender>")
            .field("data_rx", &"<broadcast::Receiver>")
            .field("command_tx", &"<mpsc::Sender>")
            .field("instrument", &"<Arc<Mutex<Box<dyn Instrument>>>>")
            .finish()
    }
}

impl InstrumentHandle {
    /// Send command and wait for response
    pub async fn send_command(&self, cmd: Command) -> Result<Response> {
        self.command_tx.send(cmd).await?;
        // Response will come via oneshot channel in actual implementation
        // This is simplified for Phase 1
        Ok(Response::Ok)
    }

    /// Subscribe to data stream
    pub fn subscribe(&self) -> broadcast::Receiver<Measurement> {
        self.data_rx.resubscribe()
    }

    /// Request graceful shutdown
    pub async fn shutdown(self) -> Result<()> {
        let _ = self.shutdown_tx.send(());
        self.task.await??;
        Ok(())
    }

    /// Check if instrument implements Camera trait
    pub async fn as_camera(&self) -> Option<Arc<tokio::sync::Mutex<Box<dyn Camera>>>> {
        let _guard = self.instrument.lock().await;
        // Attempt downcast (simplified for Phase 1)
        // Full implementation would use proper trait object casting
        None
    }

    /// Check if instrument implements Stage trait
    pub async fn as_stage(&self) -> Option<Arc<tokio::sync::Mutex<Box<dyn Stage>>>> {
        None
    }

    /// Check if instrument implements Spectrometer trait
    pub async fn as_spectrometer(&self) -> Option<Arc<tokio::sync::Mutex<Box<dyn Spectrometer>>>> {
        None
    }
}

// =============================================================================
// Processor Traits
// =============================================================================

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
    fn process_measurements(&mut self, data: &[Arc<Measurement>]) -> Vec<Arc<Measurement>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_measurement_accessors() {
        let m = Measurement::Scalar {
            name: "test".to_string(),
            value: 42.0,
            unit: "mW".to_string(),
            timestamp: Utc::now(),
        };

        assert_eq!(m.name(), "test");
        assert!(m.timestamp() <= Utc::now());
    }

    #[test]
    fn test_instrument_state_transitions() {
        assert_ne!(InstrumentState::Connected, InstrumentState::Running);
        assert_eq!(InstrumentState::Connected, InstrumentState::Connected);
    }

    #[test]
    fn test_command_types() {
        let cmd = Command::Start;
        assert!(matches!(cmd, Command::Start));

        let cmd = Command::SetParameter("exposure".to_string(), serde_json::json!(100.0));
        assert!(matches!(cmd, Command::SetParameter(_, _)));
    }

    #[test]
    fn test_roi_ordering() {
        let roi1 = Roi {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        let roi2 = Roi {
            x: 0,
            y: 0,
            width: 200,
            height: 200,
        };
        assert!(roi1 < roi2);
    }

    #[test]
    #[cfg(feature = "storage_arrow")]
    fn test_arrow_batches_scalar() -> Result<(), arrow::error::ArrowError> {
        let measurements = vec![
            Measurement::Scalar {
                name: "power".to_string(),
                value: 100.0,
                unit: "mW".to_string(),
                timestamp: Utc::now(),
            },
            Measurement::Scalar {
                name: "temperature".to_string(),
                value: 25.5,
                unit: "C".to_string(),
                timestamp: Utc::now(),
            },
        ];

        let batches = Measurement::into_arrow_batches(&measurements)?;
        assert!(batches.scalars.is_some());
        assert!(batches.vectors.is_none());
        assert!(batches.spectra.is_none());

        let scalar_batch = batches.scalars.unwrap();
        assert_eq!(scalar_batch.num_rows(), 2);
        assert_eq!(scalar_batch.num_columns(), 4);
        Ok(())
    }

    #[test]
    #[cfg(feature = "storage_arrow")]
    fn test_arrow_batches_mixed() -> Result<(), arrow::error::ArrowError> {
        let measurements = vec![
            Measurement::Scalar {
                name: "power".to_string(),
                value: 100.0,
                unit: "mW".to_string(),
                timestamp: Utc::now(),
            },
            Measurement::Vector {
                name: "spectrum".to_string(),
                values: vec![1.0, 2.0, 3.0, 4.0, 5.0],
                unit: "V".to_string(),
                timestamp: Utc::now(),
            },
            Measurement::Spectrum {
                name: "fft".to_string(),
                frequencies: vec![100.0, 200.0, 300.0],
                amplitudes: vec![0.5, 0.3, 0.1],
                frequency_unit: Some("Hz".to_string()),
                amplitude_unit: Some("dB".to_string()),
                metadata: None,
                timestamp: Utc::now(),
            },
        ];

        let batches = Measurement::into_arrow_batches(&measurements)?;
        assert!(batches.scalars.is_some());
        assert!(batches.vectors.is_some());
        assert!(batches.spectra.is_some());

        assert_eq!(batches.scalars.unwrap().num_rows(), 1);
        assert_eq!(batches.vectors.unwrap().num_rows(), 1);
        assert_eq!(batches.spectra.unwrap().num_rows(), 1);
        Ok(())
    }

    #[test]
    #[cfg(feature = "storage_arrow")]
    fn test_arrow_batches_images() -> Result<(), arrow::error::ArrowError> {
        // Test U8 image
        let img_u8 = Measurement::Image {
            name: "camera_u8".to_string(),
            width: 640,
            height: 480,
            buffer: PixelBuffer::U8(vec![128u8; 640 * 480]),
            unit: "counts".to_string(),
            metadata: ImageMetadata {
                exposure_ms: Some(50.0),
                gain: Some(2.0),
                binning: None,
                temperature_c: None,
                hardware_timestamp_us: None,
                readout_ms: None,
                roi_origin: None,
            },
            timestamp: Utc::now(),
        };

        // Test U16 image
        let img_u16 = Measurement::Image {
            name: "camera_u16".to_string(),
            width: 1024,
            height: 1024,
            buffer: PixelBuffer::U16(vec![32768u16; 1024 * 1024]),
            unit: "ADU".to_string(),
            metadata: ImageMetadata {
                exposure_ms: Some(100.0),
                gain: None,
                binning: Some((2, 2)),
                temperature_c: Some(-20.0),
                hardware_timestamp_us: None,
                readout_ms: None,
                roi_origin: None,
            },
            timestamp: Utc::now(),
        };

        // Test F64 image
        let img_f64 = Measurement::Image {
            name: "processed_f64".to_string(),
            width: 512,
            height: 512,
            buffer: PixelBuffer::F64(vec![0.5; 512 * 512]),
            unit: "normalized".to_string(),
            metadata: ImageMetadata {
                exposure_ms: None,
                gain: None,
                binning: None,
                temperature_c: None,
                hardware_timestamp_us: None,
                readout_ms: None,
                roi_origin: None,
            },
            timestamp: Utc::now(),
        };

        let measurements = vec![img_u8, img_u16, img_f64];
        let batches = Measurement::into_arrow_batches(&measurements)?;

        assert!(batches.images.is_some());
        let image_batch = batches.images.unwrap();
        assert_eq!(image_batch.num_rows(), 3);
        assert_eq!(image_batch.num_columns(), 9); // name, width, height, unit, dtype, pixels, exposure_ms, gain, timestamp_ns

        // Verify schema
        let schema = image_batch.schema();
        assert_eq!(schema.field(0).name(), "name");
        assert_eq!(schema.field(1).name(), "width");
        assert_eq!(schema.field(2).name(), "height");
        assert_eq!(schema.field(3).name(), "unit");
        assert_eq!(schema.field(4).name(), "dtype");
        assert_eq!(schema.field(5).name(), "pixels");
        assert_eq!(schema.field(6).name(), "exposure_ms");
        assert_eq!(schema.field(7).name(), "gain");
        assert_eq!(schema.field(8).name(), "timestamp_ns");

        Ok(())
    }

    #[test]
    #[cfg(feature = "storage_arrow")]
    fn test_arrow_batches_mixed_with_images() -> Result<(), arrow::error::ArrowError> {
        let measurements = vec![
            Measurement::Scalar {
                name: "power".to_string(),
                value: 100.0,
                unit: "mW".to_string(),
                timestamp: Utc::now(),
            },
            Measurement::Image {
                name: "camera_frame".to_string(),
                width: 128,
                height: 128,
                buffer: PixelBuffer::U8(vec![255u8; 128 * 128]),
                unit: "counts".to_string(),
                metadata: ImageMetadata {
                    exposure_ms: Some(10.0),
                    gain: Some(1.5),
                    binning: None,
                    temperature_c: None,
                    hardware_timestamp_us: None,
                    readout_ms: None,
                    roi_origin: None,
                },
                timestamp: Utc::now(),
            },
            Measurement::Vector {
                name: "waveform".to_string(),
                values: vec![1.0, 2.0, 3.0],
                unit: "V".to_string(),
                timestamp: Utc::now(),
            },
        ];

        let batches = Measurement::into_arrow_batches(&measurements)?;
        assert!(batches.scalars.is_some());
        assert!(batches.vectors.is_some());
        assert!(batches.images.is_some());
        assert!(batches.spectra.is_none());

        assert_eq!(batches.scalars.unwrap().num_rows(), 1);
        assert_eq!(batches.vectors.unwrap().num_rows(), 1);
        assert_eq!(batches.images.unwrap().num_rows(), 1);
        Ok(())
    }
}
