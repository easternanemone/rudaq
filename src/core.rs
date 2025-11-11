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
//! ```rust
//! use rust_daq::core::{Instrument, DataPoint, InstrumentCommand};
//! use rust_daq::config::Settings;
//! use async_trait::async_trait;
//! use std::sync::Arc;
//! use tokio::sync::broadcast;
//!
//! struct MockInstrument {
//!     id: String,
//!     sender: Option<broadcast::Sender<DataPoint>>,
//! }
//!
//! #[async_trait]
//! impl Instrument for MockInstrument {
//!     fn name(&self) -> String {
//!         self.id.clone()
//!     }
//!
//!     async fn connect(&mut self, _settings: &Arc<Settings>) -> anyhow::Result<()> {
//!         let (sender, _) = broadcast::channel(1024);
//!         self.sender = Some(sender);
//!         Ok(())
//!     }
//!
//!     async fn disconnect(&mut self) -> anyhow::Result<()> {
//!         self.sender = None;
//!         Ok(())
//!     }
//!
//!     async fn data_stream(&mut self) -> anyhow::Result<broadcast::Receiver<DataPoint>> {
//!         self.sender.as_ref()
//!             .map(|s| s.subscribe())
//!             .ok_or_else(|| anyhow::anyhow!("Not connected"))
//!     }
//! }
//! ```
use crate::config::Settings;
use crate::measurement::Measure;
use crate::metadata::Metadata;
use async_trait::async_trait;
pub use daq_core::Measurement;
use serde::{Deserialize, Serialize};
use std::any::TypeId;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinHandle};

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
    pub timestamp: chrono::DateTime<chrono::Utc>,
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

// Conversion from V1 core::DataPoint to V2 daq_core::DataPoint
// This allows V1 instruments to integrate with V2 architecture
impl From<DataPoint> for daq_core::DataPoint {
    fn from(dp: DataPoint) -> Self {
        Self {
            timestamp: dp.timestamp,
            channel: dp.channel,
            value: dp.value,
            unit: dp.unit,
            // Note: instrument_id and metadata are dropped in conversion to V2
        }
    }
}

// Conversion from V1 core::DataPoint to V2 daq_core::Measurement
// This allows V1 instruments to work with the new Measurement enum architecture
impl From<DataPoint> for daq_core::Measurement {
    fn from(dp: DataPoint) -> Self {
        // Convert V1 DataPoint to V2 DataPoint first, then wrap in Scalar variant
        let v2_dp: daq_core::DataPoint = dp.into();
        daq_core::Measurement::Scalar(v2_dp)
    }
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

impl From<PixelBuffer> for daq_core::PixelBuffer {
    fn from(buffer: PixelBuffer) -> Self {
        match buffer {
            PixelBuffer::U8(data) => daq_core::PixelBuffer::U8(data),
            PixelBuffer::U16(data) => daq_core::PixelBuffer::U16(data),
            PixelBuffer::F64(data) => daq_core::PixelBuffer::F64(data),
        }
    }
}

/// Represents spectrum data from FFT or other frequency analysis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpectrumData {
    /// UTC timestamp when spectrum was captured
    pub timestamp: chrono::DateTime<chrono::Utc>,
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

impl From<SpectrumData> for daq_core::SpectrumData {
    fn from(spectrum: SpectrumData) -> Self {
        let SpectrumData {
            timestamp,
            channel,
            unit,
            bins,
            metadata,
        } = spectrum;

        let (wavelengths, intensities): (Vec<f64>, Vec<f64>) = bins
            .into_iter()
            .map(|bin| (bin.frequency, bin.magnitude))
            .unzip();

        let unit_x = metadata
            .as_ref()
            .and_then(|meta| meta.get("frequency_unit").and_then(|value| value.as_str()))
            .unwrap_or("Hz")
            .to_string();

        daq_core::SpectrumData {
            timestamp,
            channel,
            wavelengths,
            intensities,
            unit_x,
            unit_y: unit,
            metadata,
        }
    }
}

/// Represents image data from cameras or 2D sensors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageData {
    /// UTC timestamp when image was captured
    pub timestamp: chrono::DateTime<chrono::Utc>,
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

impl From<ImageData> for daq_core::ImageData {
    fn from(image: ImageData) -> Self {
        let ImageData {
            timestamp,
            channel,
            width,
            height,
            pixels,
            unit,
            metadata,
        } = image;

        daq_core::ImageData {
            timestamp,
            channel,
            width: u32::try_from(width).unwrap_or(u32::MAX),
            height: u32::try_from(height).unwrap_or(u32::MAX),
            pixels: pixels.into(),
            unit,
            metadata,
        }
    }
}

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
    pub fn timestamp(&self) -> chrono::DateTime<chrono::Utc> {
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

impl From<Data> for daq_core::Measurement {
    fn from(data: Data) -> Self {
        match data {
            Data::Scalar(dp) => daq_core::Measurement::Scalar(dp.into()),
            Data::Spectrum(spectrum) => daq_core::Measurement::Spectrum(spectrum.into()),
            Data::Image(image) => daq_core::Measurement::Image(image.into()),
        }
    }
}

/// Command that can be sent to an instrument.
///
/// `InstrumentCommand` provides a type-safe command interface for controlling
/// instruments asynchronously. Commands are sent via Tokio `mpsc` channels to
/// instrument tasks, enabling non-blocking operations.
///
/// # Variants
///
/// * `SetParameter(key, value)` - Set an instrument parameter without waiting for confirmation.
///   Example: `SetParameter("wavelength".to_string(), "800.0".to_string())`
///
/// * `QueryParameter(key)` - Request the current value of a parameter. The response
///   is typically sent via the instrument's data stream as a `DataPoint`.
///   Example: `QueryParameter("temperature".to_string())`
///
/// * `Execute(command, args)` - Execute a complex command with optional arguments.
///   Example: `Execute("calibrate".to_string(), vec![])` to calibrate instrument
///
/// * `Shutdown` - Gracefully shut down the instrument. Triggers `disconnect()` and
///   breaks the instrument task loop. The shutdown process has a 5-second timeout,
///   after which the task is forcefully terminated.
///
/// # Usage Pattern
///
/// Commands are sent through the `InstrumentHandle::command_tx` channel:
///
/// ```rust
/// use rust_daq::core::InstrumentCommand;
/// # use tokio::sync::mpsc;
/// # async fn example(command_tx: mpsc::Sender<InstrumentCommand>) -> anyhow::Result<()> {
/// // Set a parameter
/// command_tx.send(InstrumentCommand::SetParameter(
///     "power".to_string(),
///     "100".to_string()
/// )).await?;
///
/// // Execute a command
/// command_tx.send(InstrumentCommand::Execute(
///     "calibrate".to_string(),
///     vec![]
/// )).await?;
/// # Ok(())
/// # }
/// ```
///
/// # Shutdown Behavior
///
/// The `Shutdown` command is special:
/// 1. Sent to all instruments during app shutdown
/// 2. Causes instrument task loop to break
/// 3. Triggers `instrument.disconnect()` to clean up resources
/// 4. If timeout (5s) expires, task is forcefully terminated
///
/// # Thread Safety
///
/// Commands are `Clone` to support broadcasting to multiple instruments and
/// retry logic. They're also `Send` for cross-thread channel communication.
#[derive(Clone, Debug)]
pub enum InstrumentCommand {
    /// Set a parameter (key, value) - no response expected
    SetParameter(String, ParameterValue),
    /// Query a parameter (key) - response sent via data stream
    QueryParameter(String),
    /// Execute a command with optional arguments
    Execute(String, Vec<String>),
    /// Capability-scoped operation with typed parameters
    Capability {
        /// Capability identifier (`TypeId` returned by `Instrument::capabilities`)
        capability: TypeId,
        /// Operation name exposed by the capability trait
        operation: String,
        /// Typed parameter payload for the operation
        parameters: Vec<ParameterValue>,
    },
    /// Gracefully shut down the instrument (triggers disconnect, 5s timeout)
    Shutdown,
}

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

/// A handle to a running instrument task.
///
/// `InstrumentHandle` provides a safe interface for managing and communicating
/// with instrument tasks running on the Tokio runtime. Each instrument runs in
/// its own async task, isolated from other instruments and the GUI.
///
/// # Fields
///
/// * `task` - Tokio task handle for the instrument's main loop. Can be awaited
///   to get the task's result, or aborted to forcefully terminate the instrument.
///   The task returns `Result<()>` where errors indicate instrument failures.
///
/// * `command_tx` - Command channel sender for sending [`InstrumentCommand`]s
///   to the instrument. Bounded channel with capacity 32 to apply backpressure
///   if the instrument cannot keep up with commands.
///
/// # Lifecycle
///
/// 1. **Created** by `DaqAppInner::spawn_instrument()` when an instrument is registered
/// 2. **Active** during normal operation, processing commands and streaming data
/// 3. **Shutdown** when `Shutdown` command is sent, triggering graceful disconnect
/// 4. **Terminated** either by task completion or timeout + abort
///
/// # Usage Pattern
///
/// ```rust
/// use rust_daq::core::{InstrumentHandle, InstrumentCommand};
/// # use tokio::sync::mpsc;
/// # use tokio::task::JoinHandle;
/// # async fn example(handle: InstrumentHandle) -> anyhow::Result<()> {
/// // Send a command to the instrument
/// handle.command_tx.send(InstrumentCommand::SetParameter(
///     "wavelength".to_string(),
///     "800.0".to_string()
/// )).await?;
///
/// // For shutdown, send Shutdown command then await task with timeout
/// handle.command_tx.send(InstrumentCommand::Shutdown).await?;
/// tokio::time::timeout(
///     std::time::Duration::from_secs(5),
///     handle.task
/// ).await??;
/// # Ok(())
/// # }
/// ```
///
/// # Error Handling
///
/// If `command_tx.send()` fails with `SendError`, the instrument task has terminated.
/// This typically indicates a crash or panic in the instrument code. The task handle
/// should be awaited to retrieve the error details.
///
/// # Thread Safety
///
/// InstrumentHandle is `Send` but not `Sync` - ownership should be transferred
/// between threads, not shared. The application stores handles in a `HashMap`
/// protected by a `Mutex` for safe multi-threaded access.
pub struct InstrumentHandle {
    /// Tokio task handle (returns Result on completion/failure)
    pub abort_handle: AbortHandle,
    /// Command channel sender (capacity: 32, bounded for backpressure)
    pub command_tx: mpsc::Sender<InstrumentCommand>,
    /// Capabilities advertised by the instrument instance
    pub capabilities: Vec<TypeId>,
}

/// Trait for any scientific instrument.
///
/// This trait defines the common interface for all instruments, allowing them
/// to be managed and controlled in a generic way. All instruments must implement
/// this trait to be used in the DAQ system.
///
/// # Design Philosophy
///
/// The trait follows an async-first design to support non-blocking I/O operations
/// (serial, USB, network). Each instrument runs in its own Tokio task, processing
/// commands from a channel and streaming data via broadcast channels.
///
/// # Lifecycle Methods
///
/// 1. `connect()` - Initialize hardware connection and spawn data streaming task
/// 2. `data_stream()` - Provide broadcast receiver for real-time data consumption
/// 3. `handle_command()` - Process control commands (parameter changes, execution)
/// 4. `disconnect()` - Clean up resources and close hardware connection
///
/// # Threading Model
///
/// Instruments must be `Send + Sync` to enable:
/// - Transfer between async tasks (Send)
/// - Shared access via Arc (Sync, though typically not needed)
///
/// # Implementation Example
///
/// See module-level documentation for a complete example of implementing this trait.
///
/// # Error Handling
///
/// All async methods return `anyhow::Result<()>` for flexible error handling.
/// Common error scenarios:
/// - Connection failures (device not found, permission denied)
/// - Communication timeouts (no response from hardware)
/// - Invalid commands (unsupported operation)
/// - Hardware errors (device malfunction)
///
/// Errors should include context using `.context()` to aid debugging.
#[async_trait]
pub trait Instrument: Send + Sync {
    type Measure: Measure;

    fn name(&self) -> String;
    async fn connect(&mut self, id: &str, settings: &Arc<Settings>) -> anyhow::Result<()>;
    async fn disconnect(&mut self) -> anyhow::Result<()>;
    fn measure(&self) -> &Self::Measure;
    fn capabilities(&self) -> Vec<TypeId> {
        Vec::new()
    }
    async fn handle_command(&mut self, _command: InstrumentCommand) -> anyhow::Result<()> {
        Ok(())
    }

    /// Set the V2 data distributor for broadcasting original measurements.
    /// Only V2InstrumentAdapter implements this; default is no-op.
    fn set_v2_data_distributor(
        &mut self,
        _distributor: std::sync::Arc<
            crate::measurement::DataDistributor<std::sync::Arc<daq_core::Measurement>>,
        >,
    ) {
        // Default: no-op for V1 instruments
    }
}

/// Trait for a data processor.
///
/// Data processors transform streams of [`DataPoint`]s in real-time, enabling
/// signal processing, filtering, triggering, and derived measurements. Processors
/// can be chained to form multi-stage processing pipelines.
///
/// # Design Principles
///
/// - **Stateful**: Processors maintain internal state (filter coefficients, buffers, etc.)
/// - **Batch processing**: Operates on slices of data points for efficiency
/// - **Flexible output**: Can produce 0, 1, or many output points per input
/// - **Thread-safe**: Must be `Send + Sync` for concurrent access
///
/// # Common Use Cases
///
/// - **Filtering**: IIR/FIR filters, moving averages, smoothing
/// - **Signal processing**: FFT, power spectral density, correlation
/// - **Triggering**: Edge detection, threshold crossing, event detection
/// - **Derivation**: Calculating rates of change, integrals, statistics
/// - **Transformation**: Unit conversion, calibration, normalization
///
/// # Pipeline Architecture
///
/// ```text
/// DataPoint[] --[Processor 1]--> DataPoint[] --[Processor 2]--> DataPoint[]
///     Raw data        Filter              FFT             Storage
/// ```
///
/// Processors are registered in `ProcessorRegistry` and applied sequentially
/// before data reaches storage or GUI display.
///
/// # Performance Considerations
///
/// - Process data in batches to amortize per-call overhead
/// - Pre-allocate output vectors to avoid repeated allocations
/// - Use SIMD operations for bulk data processing when applicable
/// - Avoid expensive operations in hot paths (heap allocation, logging)
///
/// # Example: Simple Moving Average
///
/// ```rust
/// use rust_daq::core::{DataProcessor, DataPoint};
///
/// struct MovingAverage {
///     window_size: usize,
///     buffer: Vec<f64>,
/// }
///
/// impl DataProcessor for MovingAverage {
///     fn process(&mut self, data: &[DataPoint]) -> Vec<DataPoint> {
///         let mut output = Vec::with_capacity(data.len());
///
///         for dp in data {
///             self.buffer.push(dp.value);
///             if self.buffer.len() > self.window_size {
///                 self.buffer.remove(0);
///             }
///
///             let avg = self.buffer.iter().sum::<f64>() / self.buffer.len() as f64;
///             output.push(DataPoint {
///                 timestamp: dp.timestamp,
///                 channel: format!("{}_avg", dp.channel),
///                 value: avg,
///                 unit: dp.unit.clone(),
///                 metadata: None,
///             });
///         }
///
///         output
///     }
/// }
/// ```
pub trait DataProcessor: Send + Sync {
    /// Processes a batch of data points and returns transformed data.
    ///
    /// # Arguments
    ///
    /// * `data` - Input slice of data points to process. May be empty if no data available.
    ///
    /// # Returns
    ///
    /// Vector of processed data points. The output can be:
    /// - **Empty** (`vec![]`) if input doesn't meet processing criteria (e.g., trigger not met)
    /// - **Same length** as input for 1:1 transformations (filtering, calibration)
    /// - **Shorter** for decimation or trigger detection
    /// - **Longer** for expansion or derivative calculations
    ///
    /// # Complexity
    ///
    /// Varies by processor type:
    /// - Simple filters: O(n) where n = data.len()
    /// - FFT processors: O(n log n)
    /// - Triggered processors: O(n) with early return
    ///
    /// # Implementation Notes
    ///
    /// - Maintain state between calls (e.g., filter history, buffer accumulation)
    /// - Preserve original timestamps when possible
    /// - Use descriptive channel names: `format!("{}_filtered", input_channel)`
    /// - Clone unit strings efficiently or use `Arc<str>` for shared units
    /// - Handle edge cases: empty input, first call (uninitialized state)
    fn process(&mut self, data: &[DataPoint]) -> Vec<DataPoint>;
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
///         let scalars: Vec<DataPoint> = data.iter()
///             .filter_map(|m| if let Measurement::Scalar(dp) = m.as_ref() { Some(dp.clone()) } else { None })
///             .collect();
///         let filtered = self.process(&scalars); // Call legacy DataProcessor::process
///         filtered.into_iter().map(|dp| {
///             let daq_dp = daq_core::DataPoint {
///                 timestamp: dp.timestamp,
///                 channel: dp.channel,
///                 value: dp.value,
///                 unit: dp.unit,
///             };
///             Arc::new(Measurement::Scalar(daq_dp))
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

/// Adapter that wraps a legacy `DataProcessor` to work with `MeasurementProcessor`.
///
/// This adapter enables backward compatibility by:
/// 1. Extracting Scalar measurements from Arc<Measurement> inputs
/// 2. Calling the legacy DataProcessor::process() on DataPoints
/// 3. Wrapping results back into Arc<Measurement>
///
/// Non-scalar measurements (Spectrum, Image) are passed through unchanged.
///
/// # Example
///
/// ```rust
/// # use rust_daq::core::{MeasurementProcessor, Measurement, DataPoint, DataProcessor};
/// # struct MyFilter;
/// # impl DataProcessor for MyFilter {
/// #     fn process(&mut self, data: &[DataPoint]) -> Vec<DataPoint> { data.to_vec() }
/// # }
/// let legacy_filter = MyFilter;
/// let adapted: Box<dyn MeasurementProcessor> = Box::new(DataProcessorAdapter::new(Box::new(legacy_filter)));
/// ```
pub struct DataProcessorAdapter {
    inner: Box<dyn DataProcessor>,
}

impl DataProcessorAdapter {
    /// Creates a new adapter wrapping a DataProcessor
    pub fn new(processor: Box<dyn DataProcessor>) -> Self {
        Self { inner: processor }
    }
}

impl MeasurementProcessor for DataProcessorAdapter {
    fn process_measurements(&mut self, data: &[Arc<Measurement>]) -> Vec<Arc<Measurement>> {
        // Extract scalar measurements and convert from daq_core::DataPoint to core::DataPoint
        let scalars: Vec<DataPoint> = data
            .iter()
            .filter_map(|m| {
                if let Measurement::Scalar(dp) = m.as_ref() {
                    // Convert daq_core::DataPoint to core::DataPoint
                    Some(DataPoint {
                        timestamp: dp.timestamp,
                        instrument_id: String::new(), // daq_core doesn't have instrument_id
                        channel: dp.channel.clone(),
                        value: dp.value,
                        unit: dp.unit.clone(),
                        metadata: None, // daq_core DataPoint doesn't have metadata in this context
                    })
                } else {
                    None
                }
            })
            .collect();

        // Process with legacy processor
        let processed = self.inner.process(&scalars);

        // Wrap results back into Arc<Measurement>
        // Convert from core::DataPoint to daq_core::DataPoint
        let mut results: Vec<Arc<Measurement>> = processed
            .into_iter()
            .map(|dp| {
                let daq_dp = daq_core::DataPoint {
                    timestamp: dp.timestamp,
                    channel: dp.channel,
                    value: dp.value,
                    unit: dp.unit,
                };
                Arc::new(Measurement::Scalar(daq_dp))
            })
            .collect();

        // Pass through non-scalar measurements unchanged
        for measurement in data {
            if !matches!(measurement.as_ref(), Measurement::Scalar(_)) {
                results.push(measurement.clone());
            }
        }

        results
    }
}

/// Trait for a data storage writer.
///
/// `StorageWriter` defines the interface for persisting data to various storage
/// backends (CSV, HDF5, databases, cloud services). Writers are responsible for
/// efficient batch I/O, metadata management, and graceful resource cleanup.
///
/// # Lifecycle
///
/// 1. **init()** - Create/open storage, allocate resources
/// 2. **set_metadata()** - Write experiment metadata header
/// 3. **write()** - Append data batches (called repeatedly)
/// 4. **shutdown()** - Flush buffers, close files, finalize
///
/// # Async Design
///
/// All methods are async to support non-blocking I/O operations. Use:
/// - `tokio::fs` for file operations
/// - `tokio::spawn_blocking` for CPU-intensive operations (compression, serialization)
/// - Buffered writes to minimize syscalls
///
/// # Error Handling
///
/// Storage errors are critical - they indicate data loss. Writers should:
/// - Return detailed errors with `.context()` for debugging
/// - Log errors before returning (storage task logs to console + file)
/// - Implement retry logic for transient failures (disk space, network)
/// - Fail fast on unrecoverable errors (permission denied, corruption)
///
/// # Supported Formats
///
/// Current implementations:
/// - CSV: Human-readable, Excel-compatible, inefficient for large datasets
/// - HDF5: Binary, self-describing, optimal for numeric arrays
/// - Arrow/Parquet: Columnar format, efficient compression, ecosystem support
///
/// # Example: Simple CSV Writer
///
/// ```rust
/// use rust_daq::core::{StorageWriter, DataPoint};
/// use rust_daq::config::Settings;
/// use rust_daq::metadata::Metadata;
/// use async_trait::async_trait;
/// use std::sync::Arc;
/// use tokio::fs::File;
/// use tokio::io::AsyncWriteExt;
///
/// struct CsvWriter {
///     file: Option<File>,
/// }
///
/// #[async_trait]
/// impl StorageWriter for CsvWriter {
///     async fn init(&mut self, settings: &Arc<Settings>) -> anyhow::Result<()> {
///         let path = format!("{}/data.csv", settings.storage.default_path);
///         self.file = Some(File::create(path).await?);
///         // Write CSV header
///         if let Some(f) = &mut self.file {
///             f.write_all(b"timestamp,channel,value,unit\n").await?;
///         }
///         Ok(())
///     }
///
///     async fn set_metadata(&mut self, metadata: &Metadata) -> anyhow::Result<()> {
///         // Write metadata as CSV comment lines
///         Ok(())
///     }
///
///     async fn write(&mut self, data: &[DataPoint]) -> anyhow::Result<()> {
///         if let Some(f) = &mut self.file {
///             for dp in data {
///                 let line = format!("{},{},{},{}\n",
///                     dp.timestamp, dp.channel, dp.value, dp.unit);
///                 f.write_all(line.as_bytes()).await?;
///             }
///         }
///         Ok(())
///     }
///
///     async fn shutdown(&mut self) -> anyhow::Result<()> {
///         if let Some(mut f) = self.file.take() {
///             f.flush().await?;
///         }
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait StorageWriter: Send + Sync {
    /// Initializes the storage backend and prepares for writing.
    ///
    /// This method should:
    /// 1. Create the storage file/connection based on settings
    /// 2. Write file headers or initialize database tables
    /// 3. Allocate write buffers for batch operations
    /// 4. Set up compression if applicable
    ///
    /// # Arguments
    ///
    /// * `settings` - Application settings containing storage configuration
    ///   (path, format, compression level, buffer size)
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - File/directory creation fails (permission denied, disk full)
    /// - Database connection fails (invalid credentials, network error)
    /// - Configuration is missing or invalid
    ///
    /// # Complexity
    ///
    /// O(1) for file creation, but may include O(n) overhead for pre-allocation
    ///
    /// # Example
    ///
    /// ```rust
    /// # use rust_daq::core::StorageWriter;
    /// # use rust_daq::config::Settings;
    /// # use std::sync::Arc;
    /// # use async_trait::async_trait;
    /// # use anyhow::Result;
    /// # struct MyWriter;
    /// # #[async_trait]
    /// # impl StorageWriter for MyWriter {
    /// async fn init(&mut self, settings: &Arc<Settings>) -> Result<()> {
    ///     let path = format!("{}/experiment.h5", settings.storage.default_path);
    ///     tokio::fs::create_dir_all(&settings.storage.default_path).await?;
    ///     // Initialize HDF5 file...
    ///     Ok(())
    /// }
    /// # async fn set_metadata(&mut self, _: &rust_daq::metadata::Metadata) -> Result<()> { Ok(()) }
    /// # async fn write(&mut self, _: &[rust_daq::core::DataPoint]) -> Result<()> { Ok(()) }
    /// # async fn shutdown(&mut self) -> Result<()> { Ok(()) }
    /// # }
    /// ```
    async fn init(&mut self, settings: &Arc<Settings>) -> anyhow::Result<()>;

    /// Sets the experiment-level metadata for this storage session.
    ///
    /// Metadata includes:
    /// - Experimenter name, institution, project
    /// - Session start time, configuration snapshot
    /// - Instrument descriptions and calibration data
    /// - Custom key-value pairs
    ///
    /// This method should be called once after `init()` and before the first `write()`.
    /// The metadata is typically written to:
    /// - CSV: Comment lines at file header
    /// - HDF5: Root-level attributes
    /// - Database: Metadata table
    ///
    /// # Arguments
    ///
    /// * `metadata` - Experiment metadata structure containing session info
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - Storage is not initialized (init() not called)
    /// - Write operation fails (disk error, serialization error)
    /// - Metadata format is incompatible with storage backend
    ///
    /// # Complexity
    ///
    /// O(1) for simple attribute writes, O(n) for large custom metadata
    async fn set_metadata(&mut self, metadata: &Metadata) -> anyhow::Result<()>;

    /// Writes a batch of measurements to the storage.
    ///
    /// This is the hot path - called frequently with batches of data. Implementations
    /// should:
    /// - Buffer writes to minimize I/O syscalls
    /// - Use batch insert APIs for databases
    /// - Compress data if applicable (gzip, lz4)
    /// - Flush periodically to prevent data loss on crash
    /// - Handle all Measurement variants (Scalar, Spectrum, Image)
    ///
    /// # Arguments
    ///
    /// * `data` - Slice of measurements to write (Arc-wrapped for zero-copy). May be empty (no-op).
    ///
    /// # Batching Strategy
    ///
    /// - Typical batch size: 100-1000 measurements
    /// - Storage task accumulates measurements and calls write() periodically
    /// - Flush interval: 1 second or when batch size reached
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - Disk is full (no space left on device)
    /// - File/connection is closed (shutdown already called)
    /// - Serialization fails (invalid data format)
    /// - Network error (for remote storage)
    ///
    /// # Complexity
    ///
    /// - CSV: O(n) where n = data.len() (sequential writes)
    /// - HDF5: O(n) for appends, O(1) for pre-allocated datasets
    /// - Database: O(n) for batch inserts
    ///
    /// # Performance
    ///
    /// For high-rate data (>1kHz), consider:
    /// - Memory-mapped files for zero-copy writes
    /// - Separate write thread/task to avoid blocking
    /// - Asynchronous I/O with io_uring (Linux)
    async fn write(&mut self, data: &[Arc<Measurement>]) -> anyhow::Result<()>;

    /// Finalizes the storage and releases resources.
    ///
    /// This method should:
    /// 1. Flush any remaining buffered data
    /// 2. Write file footers or finalize indexes
    /// 3. Close file descriptors or database connections
    /// 4. Clean up temporary resources
    ///
    /// Called automatically during application shutdown. Errors are logged
    /// but typically not propagated.
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - Final flush fails (disk error)
    /// - File close fails (NFS timeout)
    /// - Finalization operations fail (index corruption)
    ///
    /// # Idempotency
    ///
    /// This method should be safe to call multiple times. Subsequent calls
    /// should be no-ops if already shut down.
    ///
    /// # Complexity
    ///
    /// O(1) for simple file close, O(n) for index finalization or compression
    async fn shutdown(&mut self) -> anyhow::Result<()>;
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
