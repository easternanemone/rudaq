//! Comedi DAQ Storage Writer
//!
//! High-performance storage integration for NI PCI-MIO-16XE-10 and similar
//! Comedi-based DAQ cards. Supports HDF5 and Arrow IPC streaming with
//! chunked writing for continuous data acquisition.
//!
//! # Features
//!
//! - Streaming analog input data to HDF5
//! - Streaming analog input data to Arrow IPC
//! - Chunked writing for large datasets
//! - Metadata recording (voltage ranges, sample rates, channel config)
//! - Ring buffer integration for live data tapping
//! - Compression support (gzip, lz4)
//!
//! # Architecture
//!
//! ```text
//! Comedi Driver (100 kS/s) → ComediStreamWriter → Storage Backend
//!                                    ↓
//!                              Ring Buffer (taps)
//!                                    ↓
//!                              HDF5 / Arrow IPC
//! ```
//!
//! # Example
//!
//! ```no_run
//! use daq_storage::comedi_writer::{ComediStreamWriter, ChannelConfig};
//! use std::path::Path;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let channels = vec![
//!         ChannelConfig::new(0, "AI0", -10.0, 10.0),
//!         ChannelConfig::new(1, "AI1", -5.0, 5.0),
//!     ];
//!
//!     let mut writer = ComediStreamWriter::builder()
//!         .output_path(Path::new("data/experiment.h5"))
//!         .channels(channels)
//!         .sample_rate(10000.0)
//!         .chunk_size(1024)
//!         .build()?;
//!
//!     // Stream samples
//!     let samples = vec![0.1, 0.2, 0.15, 0.25]; // AI0, AI1 interleaved
//!     writer.write_samples(&samples).await?;
//!
//!     writer.finalize().await?;
//!     Ok(())
//! }
//! ```

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

#[cfg(feature = "storage_arrow")]
use arrow::array::{ArrayRef, Float64Array, Int64Array, UInt32Array};
#[cfg(feature = "storage_arrow")]
use arrow::datatypes::{DataType, Field, Schema};
#[cfg(feature = "storage_arrow")]
use arrow::record_batch::RecordBatch;

use super::ring_buffer::RingBuffer;

/// Compression algorithm for storage
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CompressionType {
    #[default]
    None,
    Gzip,
    Lz4,
}

impl CompressionType {
    /// HDF5 filter ID
    #[cfg(feature = "storage_hdf5")]
    pub fn hdf5_filter(&self) -> Option<u32> {
        match self {
            Self::None => None,
            Self::Gzip => Some(1),    // H5Z_FILTER_DEFLATE
            Self::Lz4 => Some(32004), // LZ4 filter ID
        }
    }
}

/// Storage format for streaming data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StorageFormat {
    #[default]
    Hdf5,
    ArrowIpc,
    Both,
}

/// Analog input channel configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    /// Channel number (0-15 for NI PCI-MIO-16XE-10)
    pub channel: u32,
    /// User-defined label (e.g., "AI0", "Voltage_Sensor")
    pub label: String,
    /// Minimum voltage of configured range
    pub voltage_min: f64,
    /// Maximum voltage of configured range
    pub voltage_max: f64,
    /// Units for display (default: "V")
    pub units: String,
    /// Optional description
    pub description: Option<String>,
}

impl ChannelConfig {
    /// Create a new channel configuration
    pub fn new(channel: u32, label: impl Into<String>, voltage_min: f64, voltage_max: f64) -> Self {
        Self {
            channel,
            label: label.into(),
            voltage_min,
            voltage_max,
            units: "V".to_string(),
            description: None,
        }
    }

    /// Set custom units
    pub fn with_units(mut self, units: impl Into<String>) -> Self {
        self.units = units.into();
        self
    }

    /// Set description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Voltage range span
    pub fn voltage_span(&self) -> f64 {
        self.voltage_max - self.voltage_min
    }
}

/// Acquisition metadata for storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcquisitionMetadata {
    /// Sample rate in Hz
    pub sample_rate: f64,
    /// Start timestamp (nanoseconds since epoch)
    pub start_time_ns: u64,
    /// Hardware identifier (e.g., "NI PCI-MIO-16XE-10")
    pub hardware_id: String,
    /// Comedi device path
    pub device_path: String,
    /// Subdevice number
    pub subdevice: u32,
    /// User-defined experiment metadata
    pub user_metadata: HashMap<String, String>,
}

impl Default for AcquisitionMetadata {
    fn default() -> Self {
        Self {
            sample_rate: 10000.0,
            start_time_ns: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
            hardware_id: "NI PCI-MIO-16XE-10".to_string(),
            device_path: "/dev/comedi0".to_string(),
            subdevice: 0,
            user_metadata: HashMap::new(),
        }
    }
}

/// Stream statistics
#[derive(Debug, Clone, Default)]
pub struct StreamStats {
    /// Total samples written
    pub samples_written: u64,
    /// Total bytes written to storage
    pub bytes_written: u64,
    /// Number of chunks written
    pub chunks_written: u64,
    /// Write errors encountered
    pub write_errors: u64,
    /// Current write rate (samples/sec)
    pub write_rate: f64,
}

/// Builder for ComediStreamWriter
pub struct ComediStreamWriterBuilder {
    output_path: PathBuf,
    channels: Vec<ChannelConfig>,
    metadata: AcquisitionMetadata,
    format: StorageFormat,
    compression: CompressionType,
    chunk_size: usize,
    ring_buffer_mb: Option<usize>,
    enable_taps: bool,
}

impl Default for ComediStreamWriterBuilder {
    fn default() -> Self {
        Self {
            output_path: PathBuf::from("data.h5"),
            channels: Vec::new(),
            metadata: AcquisitionMetadata::default(),
            format: StorageFormat::Hdf5,
            compression: CompressionType::None,
            chunk_size: 4096,
            ring_buffer_mb: None,
            enable_taps: false,
        }
    }
}

impl ComediStreamWriterBuilder {
    /// Set output file path
    pub fn output_path(mut self, path: &Path) -> Self {
        self.output_path = path.to_path_buf();
        self
    }

    /// Set channel configurations
    pub fn channels(mut self, channels: Vec<ChannelConfig>) -> Self {
        self.channels = channels;
        self
    }

    /// Add a single channel
    pub fn add_channel(mut self, config: ChannelConfig) -> Self {
        self.channels.push(config);
        self
    }

    /// Set sample rate
    pub fn sample_rate(mut self, rate: f64) -> Self {
        self.metadata.sample_rate = rate;
        self
    }

    /// Set hardware identifier
    pub fn hardware_id(mut self, id: impl Into<String>) -> Self {
        self.metadata.hardware_id = id.into();
        self
    }

    /// Set Comedi device path
    pub fn device_path(mut self, path: impl Into<String>) -> Self {
        self.metadata.device_path = path.into();
        self
    }

    /// Set subdevice number
    pub fn subdevice(mut self, subdevice: u32) -> Self {
        self.metadata.subdevice = subdevice;
        self
    }

    /// Set storage format
    pub fn format(mut self, format: StorageFormat) -> Self {
        self.format = format;
        self
    }

    /// Set compression type
    pub fn compression(mut self, compression: CompressionType) -> Self {
        self.compression = compression;
        self
    }

    /// Set chunk size for writing (samples per chunk)
    pub fn chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = size;
        self
    }

    /// Enable ring buffer for live tapping
    pub fn enable_ring_buffer(mut self, capacity_mb: usize) -> Self {
        self.ring_buffer_mb = Some(capacity_mb);
        self.enable_taps = true;
        self
    }

    /// Add user metadata
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.user_metadata.insert(key.into(), value.into());
        self
    }

    /// Build the ComediStreamWriter
    pub fn build(self) -> Result<ComediStreamWriter> {
        if self.channels.is_empty() {
            return Err(anyhow!("At least one channel must be configured"));
        }

        let ring_buffer = if let Some(mb) = self.ring_buffer_mb {
            let path = self.output_path.with_extension("ring");
            Some(Arc::new(RingBuffer::create(&path, mb)?))
        } else {
            None
        };

        Ok(ComediStreamWriter {
            output_path: self.output_path,
            channels: self.channels,
            metadata: self.metadata,
            format: self.format,
            compression: self.compression,
            chunk_size: self.chunk_size,
            ring_buffer,
            enable_taps: self.enable_taps,
            sample_buffer: RwLock::new(Vec::with_capacity(self.chunk_size * 4)),
            samples_written: AtomicU64::new(0),
            bytes_written: AtomicU64::new(0),
            chunks_written: AtomicU64::new(0),
            write_errors: AtomicU64::new(0),
            initialized: RwLock::new(false),
        })
    }
}

/// High-performance streaming writer for Comedi DAQ data
pub struct ComediStreamWriter {
    #[allow(dead_code)] // Used with storage_hdf5 feature
    output_path: PathBuf,
    channels: Vec<ChannelConfig>,
    #[allow(dead_code)] // Used with storage_hdf5 feature
    metadata: AcquisitionMetadata,
    format: StorageFormat,
    #[allow(dead_code)] // Used with storage_hdf5 feature
    compression: CompressionType,
    chunk_size: usize,
    ring_buffer: Option<Arc<RingBuffer>>,
    #[allow(dead_code)] // Reserved for future use
    enable_taps: bool,
    sample_buffer: RwLock<Vec<f64>>,
    samples_written: AtomicU64,
    bytes_written: AtomicU64,
    chunks_written: AtomicU64,
    write_errors: AtomicU64,
    initialized: RwLock<bool>,
}

impl ComediStreamWriter {
    /// Create a new builder
    pub fn builder() -> ComediStreamWriterBuilder {
        ComediStreamWriterBuilder::default()
    }

    /// Initialize storage (create file, write headers)
    pub async fn initialize(&self) -> Result<()> {
        let mut initialized = self.initialized.write().await;
        if *initialized {
            return Ok(());
        }

        match self.format {
            StorageFormat::Hdf5 | StorageFormat::Both => {
                self.initialize_hdf5().await?;
            }
            StorageFormat::ArrowIpc => {
                // Arrow IPC doesn't need pre-initialization
            }
        }

        *initialized = true;
        Ok(())
    }

    /// Initialize HDF5 file structure
    #[cfg(feature = "storage_hdf5")]
    async fn initialize_hdf5(&self) -> Result<()> {
        let output_path = self.output_path.clone();
        let channels = self.channels.clone();
        let metadata = self.metadata.clone();
        let chunk_size = self.chunk_size;
        let compression = self.compression;

        tokio::task::spawn_blocking(move || -> Result<()> {
            use hdf5::types::VarLenUnicode;
            use hdf5::File;

            let file = File::create(&output_path)?;

            // Write acquisition metadata
            let meta_group = file.create_group("metadata")?;
            meta_group
                .new_attr::<f64>()
                .create("sample_rate")?
                .write_scalar(&metadata.sample_rate)?;
            meta_group
                .new_attr::<u64>()
                .create("start_time_ns")?
                .write_scalar(&metadata.start_time_ns)?;
            meta_group
                .new_attr::<VarLenUnicode>()
                .create("hardware_id")?
                .write_scalar(
                    &metadata
                        .hardware_id
                        .parse::<VarLenUnicode>()
                        .expect("VarLenUnicode"),
                )?;
            meta_group
                .new_attr::<VarLenUnicode>()
                .create("device_path")?
                .write_scalar(
                    &metadata
                        .device_path
                        .parse::<VarLenUnicode>()
                        .expect("VarLenUnicode"),
                )?;
            meta_group
                .new_attr::<u32>()
                .create("subdevice")?
                .write_scalar(&metadata.subdevice)?;

            // Write user metadata
            if !metadata.user_metadata.is_empty() {
                let user_group = meta_group.create_group("user")?;
                for (key, value) in &metadata.user_metadata {
                    user_group
                        .new_attr::<VarLenUnicode>()
                        .create(&key[..])?
                        .write_scalar(&value.parse::<VarLenUnicode>().expect("VarLenUnicode"))?;
                }
            }

            // Create channels group
            let channels_group = file.create_group("channels")?;

            // Create data group with timestamp and per-channel datasets
            let data_group = file.create_group("data")?;

            // Timestamp dataset (extendable)
            let ts_builder = data_group.new_dataset::<f64>().chunk(chunk_size);
            let ts_builder = if let Some(_filter_id) = compression.hdf5_filter() {
                ts_builder.deflate(6) // Default compression level
            } else {
                ts_builder
            };
            ts_builder.shape(0..).create("timestamps")?;

            // Sample index dataset
            let idx_builder = data_group.new_dataset::<u64>().chunk(chunk_size);
            let idx_builder = if let Some(_filter_id) = compression.hdf5_filter() {
                idx_builder.deflate(6)
            } else {
                idx_builder
            };
            idx_builder.shape(0..).create("sample_index")?;

            // Per-channel datasets
            for ch in &channels {
                // Write channel config
                let ch_group = channels_group.create_group(&ch.label)?;
                ch_group
                    .new_attr::<u32>()
                    .create("channel")?
                    .write_scalar(&ch.channel)?;
                ch_group
                    .new_attr::<f64>()
                    .create("voltage_min")?
                    .write_scalar(&ch.voltage_min)?;
                ch_group
                    .new_attr::<f64>()
                    .create("voltage_max")?
                    .write_scalar(&ch.voltage_max)?;
                ch_group
                    .new_attr::<VarLenUnicode>()
                    .create("units")?
                    .write_scalar(&ch.units.parse::<VarLenUnicode>().expect("VarLenUnicode"))?;

                if let Some(ref desc) = ch.description {
                    ch_group
                        .new_attr::<VarLenUnicode>()
                        .create("description")?
                        .write_scalar(&desc.parse::<VarLenUnicode>().expect("VarLenUnicode"))?;
                }

                // Create data dataset for this channel
                let ds_builder = data_group.new_dataset::<f64>().chunk(chunk_size);
                let ds_builder = if let Some(_filter_id) = compression.hdf5_filter() {
                    ds_builder.deflate(6)
                } else {
                    ds_builder
                };
                ds_builder.shape(0..).create(ch.label.as_str())?;
            }

            Ok(())
        })
        .await??;

        Ok(())
    }

    /// Fallback when HDF5 is not available
    #[cfg(not(feature = "storage_hdf5"))]
    async fn initialize_hdf5(&self) -> Result<()> {
        Ok(())
    }

    /// Write interleaved samples (channel0_sample0, channel1_sample0, channel0_sample1, ...)
    pub async fn write_samples(&self, samples: &[f64]) -> Result<()> {
        // Auto-initialize if needed
        if !*self.initialized.read().await {
            self.initialize().await?;
        }

        let num_channels = self.channels.len();
        if !samples.len().is_multiple_of(num_channels) {
            return Err(anyhow!(
                "Sample count {} is not divisible by channel count {}",
                samples.len(),
                num_channels
            ));
        }

        // Add to buffer
        {
            let mut buffer = self.sample_buffer.write().await;
            buffer.extend_from_slice(samples);

            // Flush if buffer exceeds chunk size
            let samples_per_channel = buffer.len() / num_channels;
            if samples_per_channel >= self.chunk_size {
                let chunk_data = std::mem::take(&mut *buffer);
                drop(buffer);
                self.flush_chunk(&chunk_data).await?;
            }
        }

        // Write to ring buffer for live tapping
        if let Some(ref rb) = self.ring_buffer {
            // Serialize samples to bytes for ring buffer
            let bytes: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
            rb.write(&bytes)?;
        }

        let num_samples = samples.len() / num_channels;
        self.samples_written
            .fetch_add(num_samples as u64, Ordering::Relaxed);

        Ok(())
    }

    /// Write de-interleaved samples (all channel0, then all channel1, ...)
    pub async fn write_samples_by_channel(&self, channel_data: &[Vec<f64>]) -> Result<()> {
        if channel_data.len() != self.channels.len() {
            return Err(anyhow!(
                "Channel data count {} does not match configured channels {}",
                channel_data.len(),
                self.channels.len()
            ));
        }

        // Verify all channels have same number of samples
        let num_samples = channel_data.first().map(|v| v.len()).unwrap_or(0);
        if !channel_data.iter().all(|v| v.len() == num_samples) {
            return Err(anyhow!("All channels must have the same number of samples"));
        }

        // Interleave for standard write path
        let mut interleaved = Vec::with_capacity(num_samples * channel_data.len());
        for i in 0..num_samples {
            for ch_data in channel_data {
                interleaved.push(ch_data[i]);
            }
        }

        self.write_samples(&interleaved).await
    }

    /// Flush buffered data to storage
    async fn flush_chunk(&self, chunk_data: &[f64]) -> Result<()> {
        let num_channels = self.channels.len();
        let samples_per_channel = chunk_data.len() / num_channels;

        if samples_per_channel == 0 {
            return Ok(());
        }

        match self.format {
            StorageFormat::Hdf5 => {
                self.flush_hdf5(chunk_data, samples_per_channel).await?;
            }
            StorageFormat::ArrowIpc => {
                self.flush_arrow(chunk_data, samples_per_channel).await?;
            }
            StorageFormat::Both => {
                self.flush_hdf5(chunk_data, samples_per_channel).await?;
                self.flush_arrow(chunk_data, samples_per_channel).await?;
            }
        }

        self.chunks_written.fetch_add(1, Ordering::Relaxed);
        self.bytes_written
            .fetch_add((chunk_data.len() * 8) as u64, Ordering::Relaxed);

        Ok(())
    }

    /// Flush chunk to HDF5
    #[cfg(feature = "storage_hdf5")]
    async fn flush_hdf5(&self, chunk_data: &[f64], samples_per_channel: usize) -> Result<()> {
        let output_path = self.output_path.clone();
        let channels = self.channels.clone();
        let sample_rate = self.metadata.sample_rate;
        let current_samples = self.samples_written.load(Ordering::Relaxed);
        let chunk_data = chunk_data.to_vec();
        let num_channels = channels.len();

        tokio::task::spawn_blocking(move || -> Result<()> {
            use hdf5::File;

            let file = File::open_rw(&output_path)?;
            let data_group = file.group("data")?;

            // Extend and write timestamps
            let ts_ds = data_group.dataset("timestamps")?;
            let current_len = ts_ds.shape()[0];
            ts_ds.resize((current_len + samples_per_channel,))?;

            let timestamps: Vec<f64> = (0..samples_per_channel)
                .map(|i| (current_samples + i as u64) as f64 / sample_rate)
                .collect();
            ts_ds.write_slice(&timestamps, current_len..)?;

            // Extend and write sample indices
            let idx_ds = data_group.dataset("sample_index")?;
            idx_ds.resize((current_len + samples_per_channel,))?;
            let indices: Vec<u64> = (0..samples_per_channel)
                .map(|i| current_samples + i as u64)
                .collect();
            idx_ds.write_slice(&indices, current_len..)?;

            // De-interleave and write per-channel data
            for (ch_idx, ch) in channels.iter().enumerate() {
                let ds = data_group.dataset(&ch.label)?;
                ds.resize((current_len + samples_per_channel,))?;

                let channel_samples: Vec<f64> = (0..samples_per_channel)
                    .map(|i| chunk_data[i * num_channels + ch_idx])
                    .collect();

                ds.write_slice(&channel_samples, current_len..)?;
            }

            Ok(())
        })
        .await??;

        Ok(())
    }

    #[cfg(not(feature = "storage_hdf5"))]
    async fn flush_hdf5(&self, _chunk_data: &[f64], _samples_per_channel: usize) -> Result<()> {
        Ok(())
    }

    /// Flush chunk to Arrow IPC
    #[cfg(feature = "storage_arrow")]
    async fn flush_arrow(&self, chunk_data: &[f64], samples_per_channel: usize) -> Result<()> {
        let output_path = self.output_path.with_extension("arrow");
        let channels = self.channels.clone();
        let sample_rate = self.metadata.sample_rate;
        let current_samples = self.samples_written.load(Ordering::Relaxed);
        let chunk_data = chunk_data.to_vec();
        let num_channels = channels.len();

        tokio::task::spawn_blocking(move || -> Result<()> {
            use arrow::ipc::writer::FileWriter;
            use std::fs::OpenOptions;
            use std::sync::Arc;

            // Build schema
            let mut fields = vec![
                Field::new("timestamp", DataType::Float64, false),
                Field::new("sample_index", DataType::UInt64, false),
            ];
            for ch in &channels {
                fields.push(Field::new(&ch.label, DataType::Float64, false));
            }
            let schema = Arc::new(Schema::new(fields));

            // Build arrays
            let timestamps: Vec<f64> = (0..samples_per_channel)
                .map(|i| (current_samples + i as u64) as f64 / sample_rate)
                .collect();
            let indices: Vec<u64> = (0..samples_per_channel)
                .map(|i| current_samples + i as u64)
                .collect();

            let mut columns: Vec<ArrayRef> = vec![
                Arc::new(Float64Array::from(timestamps)),
                Arc::new(arrow::array::UInt64Array::from(indices)),
            ];

            for ch_idx in 0..channels.len() {
                let channel_samples: Vec<f64> = (0..samples_per_channel)
                    .map(|i| chunk_data[i * num_channels + ch_idx])
                    .collect();
                columns.push(Arc::new(Float64Array::from(channel_samples)));
            }

            let batch = RecordBatch::try_new(schema.clone(), columns)?;

            // Append to Arrow IPC file
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&output_path)?;

            let mut writer = FileWriter::try_new(file, &schema)?;
            writer.write(&batch)?;
            writer.finish()?;

            Ok(())
        })
        .await??;

        Ok(())
    }

    #[cfg(not(feature = "storage_arrow"))]
    async fn flush_arrow(&self, _chunk_data: &[f64], _samples_per_channel: usize) -> Result<()> {
        Ok(())
    }

    /// Finalize storage and flush remaining data
    pub async fn finalize(&self) -> Result<()> {
        // Flush any remaining buffered data
        let remaining = {
            let mut buffer = self.sample_buffer.write().await;
            std::mem::take(&mut *buffer)
        };

        if !remaining.is_empty() {
            self.flush_chunk(&remaining).await?;
        }

        Ok(())
    }

    /// Get current statistics
    pub fn stats(&self) -> StreamStats {
        StreamStats {
            samples_written: self.samples_written.load(Ordering::Relaxed),
            bytes_written: self.bytes_written.load(Ordering::Relaxed),
            chunks_written: self.chunks_written.load(Ordering::Relaxed),
            write_errors: self.write_errors.load(Ordering::Relaxed),
            write_rate: 0.0, // TODO: Calculate from time tracking
        }
    }

    /// Get reference to ring buffer for tap registration
    pub fn ring_buffer(&self) -> Option<&Arc<RingBuffer>> {
        self.ring_buffer.as_ref()
    }

    /// Register a tap consumer for live data streaming
    pub fn register_tap(
        &self,
        id: String,
        nth_frame: usize,
    ) -> Result<tokio::sync::mpsc::Receiver<Vec<u8>>> {
        if let Some(ref rb) = self.ring_buffer {
            rb.register_tap(id, nth_frame)
        } else {
            Err(anyhow!(
                "Ring buffer not enabled. Use builder.enable_ring_buffer()"
            ))
        }
    }

    /// Unregister a tap consumer
    pub fn unregister_tap(&self, id: &str) -> Result<bool> {
        if let Some(ref rb) = self.ring_buffer {
            rb.unregister_tap(id)
        } else {
            Err(anyhow!("Ring buffer not enabled"))
        }
    }
}

/// Continuous acquisition session that wraps ComediStreamWriter
pub struct ContinuousAcquisitionSession {
    writer: Arc<ComediStreamWriter>,
    start_time_ns: u64,
    total_samples: AtomicU64,
}

impl ContinuousAcquisitionSession {
    /// Create a new acquisition session
    pub fn new(writer: Arc<ComediStreamWriter>) -> Self {
        Self {
            writer,
            start_time_ns: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
            total_samples: AtomicU64::new(0),
        }
    }

    /// Get session duration in seconds
    pub fn duration_secs(&self) -> f64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        (now - self.start_time_ns) as f64 / 1e9
    }

    /// Get total samples acquired
    pub fn total_samples(&self) -> u64 {
        self.total_samples.load(Ordering::Relaxed)
    }

    /// Get effective sample rate
    pub fn effective_rate(&self) -> f64 {
        let duration = self.duration_secs();
        if duration > 0.0 {
            self.total_samples() as f64 / duration
        } else {
            0.0
        }
    }

    /// Write samples to the session
    pub async fn write(&self, samples: &[f64]) -> Result<()> {
        let num_channels = self.writer.channels.len();
        let num_samples = samples.len() / num_channels;
        self.total_samples
            .fetch_add(num_samples as u64, Ordering::Relaxed);
        self.writer.write_samples(samples).await
    }

    /// Finalize the session
    pub async fn finalize(&self) -> Result<()> {
        self.writer.finalize().await
    }

    /// Get writer statistics
    pub fn stats(&self) -> StreamStats {
        self.writer.stats()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_channel_config() {
        let ch = ChannelConfig::new(0, "AI0", -10.0, 10.0)
            .with_units("mV")
            .with_description("Voltage sensor");

        assert_eq!(ch.channel, 0);
        assert_eq!(ch.label, "AI0");
        assert_eq!(ch.voltage_min, -10.0);
        assert_eq!(ch.voltage_max, 10.0);
        assert_eq!(ch.units, "mV");
        assert_eq!(ch.description, Some("Voltage sensor".to_string()));
        assert_eq!(ch.voltage_span(), 20.0);
    }

    #[tokio::test]
    async fn test_builder() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.h5");

        let writer = ComediStreamWriter::builder()
            .output_path(&path)
            .add_channel(ChannelConfig::new(0, "AI0", -10.0, 10.0))
            .add_channel(ChannelConfig::new(1, "AI1", -5.0, 5.0))
            .sample_rate(10000.0)
            .chunk_size(1024)
            .build()
            .unwrap();

        assert_eq!(writer.channels.len(), 2);
        assert_eq!(writer.metadata.sample_rate, 10000.0);
    }

    #[tokio::test]
    async fn test_builder_no_channels_fails() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.h5");

        let result = ComediStreamWriter::builder().output_path(&path).build();

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_write_samples_validates_count() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.h5");

        let writer = ComediStreamWriter::builder()
            .output_path(&path)
            .add_channel(ChannelConfig::new(0, "AI0", -10.0, 10.0))
            .add_channel(ChannelConfig::new(1, "AI1", -5.0, 5.0))
            .format(StorageFormat::ArrowIpc) // Use Arrow to avoid HDF5 dependency
            .chunk_size(100) // Large chunk to avoid flush
            .build()
            .unwrap();

        // 3 samples is not divisible by 2 channels
        let result = writer.write_samples(&[1.0, 2.0, 3.0]).await;
        assert!(result.is_err());

        // 4 samples is divisible by 2 channels
        let result = writer.write_samples(&[1.0, 2.0, 3.0, 4.0]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_stream_stats() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.h5");

        let writer = ComediStreamWriter::builder()
            .output_path(&path)
            .add_channel(ChannelConfig::new(0, "AI0", -10.0, 10.0))
            .format(StorageFormat::ArrowIpc)
            .chunk_size(1000) // Large chunk to avoid flush
            .build()
            .unwrap();

        // Write 100 samples
        let samples: Vec<f64> = (0..100).map(|i| i as f64 * 0.1).collect();
        writer.write_samples(&samples).await.unwrap();

        let stats = writer.stats();
        assert_eq!(stats.samples_written, 100);
    }

    #[tokio::test]
    async fn test_acquisition_session() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.h5");

        let writer = Arc::new(
            ComediStreamWriter::builder()
                .output_path(&path)
                .add_channel(ChannelConfig::new(0, "AI0", -10.0, 10.0))
                .format(StorageFormat::ArrowIpc)
                .chunk_size(1000)
                .build()
                .unwrap(),
        );

        let session = ContinuousAcquisitionSession::new(writer);

        // Write some samples
        let samples: Vec<f64> = (0..100).map(|i| i as f64 * 0.1).collect();
        session.write(&samples).await.unwrap();

        assert_eq!(session.total_samples(), 100);
        assert!(session.duration_secs() > 0.0);
    }
}
