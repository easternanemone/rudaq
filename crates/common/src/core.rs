//! Core data types for the DAQ application.
//!
//! This module defines foundational data types used throughout the data acquisition system:
//!
//! - [`PixelBuffer`]: Memory-efficient pixel storage supporting multiple bit depths
//! - [`Roi`]: Region of Interest for camera acquisition
//! - [`ImageMetadata`]: Camera-specific metadata (exposure, gain, temperature)
//! - [`Measurement`]: Unified measurement type supporting scalars, vectors, images, spectra
//! - [`ParameterValue`]: Strongly-typed argument for capability operations
//!
//! # Data Flow
//!
//! ```text
//! Instrument --[Measurement]--> broadcast::channel ---> Processors/Storage/GUI
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;

// =============================================================================
// Basic Data Types
// =============================================================================

/// Memory-efficient pixel buffer supporting multiple bit depths.
///
/// `PixelBuffer` stores image data in its native format to avoid unnecessary
/// type conversions and memory bloat. Camera sensors typically output 8-bit
/// or 16-bit unsigned integers.
///
/// # Memory Savings
///
/// For a 2048x2048 camera frame:
/// - U8: 4 MB (1 byte/pixel)
/// - U16: 8.4 MB (2 bytes/pixel)
/// - F64: 33.6 MB (8 bytes/pixel)
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
            PixelBuffer::U8(data) => Cow::Owned(data.iter().map(|&v| f64::from(v)).collect()),
            PixelBuffer::U16(data) => Cow::Owned(data.iter().map(|&v| f64::from(v)).collect()),
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

/// Image metadata (exposure, gain, etc.)
#[derive(Clone, Debug, Default)]
pub struct ImageMetadata {
    /// Exposure time in milliseconds.
    ///
    /// `None` if exposure is unknown or not applicable (e.g., pre-captured images).
    pub exposure_ms: Option<f64>,
    /// Camera gain multiplier (unitless).
    ///
    /// `None` if gain is not applicable or not set. Common range: 1.0-16.0.
    pub gain: Option<f64>,
    /// Binning factors (horizontal, vertical).
    ///
    /// `None` if no binning is applied. (1, 1) represents no binning, (2, 2) bins 2x2 pixels.
    pub binning: Option<(u32, u32)>,
    /// Sensor temperature in degrees Celsius.
    ///
    /// `None` if temperature reading is unavailable. Negative values indicate cooling below ambient.
    pub temperature_c: Option<f64>,
    /// Hardware timestamp from camera in microseconds.
    ///
    /// `None` if camera does not provide hardware timestamps. Used for precise inter-frame timing.
    pub hardware_timestamp_us: Option<i64>,
    /// Frame readout duration in milliseconds.
    ///
    /// `None` if readout time is unknown. Represents time from exposure end to data availability.
    pub readout_ms: Option<f64>,
    /// ROI origin (x, y) in full sensor coordinates.
    ///
    /// `None` if ROI matches full sensor area. Useful for reconstructing position in full frame.
    pub roi_origin: Option<(u32, u32)>,
    /// Driver-provided frame sequence number.
    ///
    /// `None` if the source does not expose a stable frame counter.
    pub frame_number: Option<u64>,
    /// Number of dropped frames detected since the previous image from the same source.
    ///
    /// `None` if no gap was observed or the source does not provide frame numbers.
    pub sequence_gap_from_previous: Option<u64>,
    /// Number of raw frames summed into this output frame (host-side summing).
    ///
    /// `None` or `Some(1)` means no summing was applied. `Some(N)` where N > 1 means
    /// N raw frames were accumulated on the host before emission. Downstream consumers
    /// should divide pixel values by this count to recover per-frame intensity.
    pub summing_count: Option<u32>,
}

#[derive(Serialize, Deserialize)]
struct HumanReadableImageMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    exposure_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gain: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    binning: Option<(u32, u32)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature_c: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hardware_timestamp_us: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    readout_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    roi_origin: Option<(u32, u32)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_number: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence_gap_from_previous: Option<u64>,
}

#[derive(Serialize, Deserialize)]
struct BinaryImageMetadata {
    exposure_ms: Option<f64>,
    gain: Option<f64>,
    binning: Option<(u32, u32)>,
    temperature_c: Option<f64>,
    hardware_timestamp_us: Option<i64>,
    readout_ms: Option<f64>,
    roi_origin: Option<(u32, u32)>,
    frame_number: Option<u64>,
    sequence_gap_from_previous: Option<u64>,
}

impl From<&ImageMetadata> for HumanReadableImageMetadata {
    fn from(value: &ImageMetadata) -> Self {
        Self {
            exposure_ms: value.exposure_ms,
            gain: value.gain,
            binning: value.binning,
            temperature_c: value.temperature_c,
            hardware_timestamp_us: value.hardware_timestamp_us,
            readout_ms: value.readout_ms,
            roi_origin: value.roi_origin,
            frame_number: value.frame_number,
            sequence_gap_from_previous: value.sequence_gap_from_previous,
        }
    }
}

impl From<&ImageMetadata> for BinaryImageMetadata {
    fn from(value: &ImageMetadata) -> Self {
        Self {
            exposure_ms: value.exposure_ms,
            gain: value.gain,
            binning: value.binning,
            temperature_c: value.temperature_c,
            hardware_timestamp_us: value.hardware_timestamp_us,
            readout_ms: value.readout_ms,
            roi_origin: value.roi_origin,
            frame_number: value.frame_number,
            sequence_gap_from_previous: value.sequence_gap_from_previous,
        }
    }
}

impl From<HumanReadableImageMetadata> for ImageMetadata {
    fn from(value: HumanReadableImageMetadata) -> Self {
        Self {
            exposure_ms: value.exposure_ms,
            gain: value.gain,
            binning: value.binning,
            temperature_c: value.temperature_c,
            hardware_timestamp_us: value.hardware_timestamp_us,
            readout_ms: value.readout_ms,
            roi_origin: value.roi_origin,
            frame_number: value.frame_number,
            sequence_gap_from_previous: value.sequence_gap_from_previous,
        }
    }
}

impl From<BinaryImageMetadata> for ImageMetadata {
    fn from(value: BinaryImageMetadata) -> Self {
        Self {
            exposure_ms: value.exposure_ms,
            gain: value.gain,
            binning: value.binning,
            temperature_c: value.temperature_c,
            hardware_timestamp_us: value.hardware_timestamp_us,
            readout_ms: value.readout_ms,
            roi_origin: value.roi_origin,
            frame_number: value.frame_number,
            sequence_gap_from_previous: value.sequence_gap_from_previous,
        }
    }
}

impl Serialize for ImageMetadata {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            HumanReadableImageMetadata::from(self).serialize(serializer)
        } else {
            BinaryImageMetadata::from(self).serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for ImageMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            HumanReadableImageMetadata::deserialize(deserializer).map(Into::into)
        } else {
            BinaryImageMetadata::deserialize(deserializer).map(Into::into)
        }
    }
}

/// Unified measurement representation.
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
    #[allow(clippy::cast_precision_loss)]
    // SAFETY: precision loss acceptable for metrics/display
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ParameterValue::Float(f) => Some(*f),
            ParameterValue::Int(i) => Some(*i as f64),
            ParameterValue::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    /// Extract value as i64
    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: value is bounded and fits in target type
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
    #[allow(clippy::cast_possible_wrap)]
    // SAFETY: value fits in target type range
    fn from(value: u64) -> Self {
        ParameterValue::Int(value as i64)
    }
}

impl From<u32> for ParameterValue {
    fn from(value: u32) -> Self {
        ParameterValue::Int(i64::from(value))
    }
}

impl From<u8> for ParameterValue {
    fn from(value: u8) -> Self {
        ParameterValue::Int(i64::from(value))
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
        ParameterValue::IntArray(value.into_iter().map(i64::from).collect())
    }
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
                frame_number: None,
                sequence_gap_from_previous: None,
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
                frame_number: None,
                sequence_gap_from_previous: None,
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
                frame_number: None,
                sequence_gap_from_previous: None,
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
                    frame_number: None,
                    sequence_gap_from_previous: None,
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
