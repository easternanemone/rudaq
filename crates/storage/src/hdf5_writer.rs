//! HDF5 Background Writer - The Mullet Strategy Backend
//!
//! This implements the "business in the back" of The Mullet Strategy:
//! - Protobuf in front (fast, compact, 10k+ writes/sec)
//! - HDF5 in back (compatible with Python/MATLAB/Igor)
//!
//! Scientists never see Protobuf - they only see f64/Vec<f64> and HDF5 files.
//! The background writer translates Protobuf → HDF5 at 1 Hz without blocking
//! the hardware loop.

use common::error::DaqError;
#[cfg(feature = "storage_hdf5")]
use common::error::{StorageError, StorageErrorKind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::time::{interval, Duration};

#[cfg(feature = "storage_hdf5")]
use crate::map_hdf5_err;

use super::ring_buffer::RingBuffer;
#[cfg(feature = "storage_hdf5")]
use common::observable::ParameterSet;

use common::error::AppResult as Result;

/// Background HDF5 writer that persists ring buffer data
///
/// # Architecture
///
/// ```text
/// Hardware Loop (100 Hz) → Ring Buffer (Arrow IPC)
///                              ↓
///                         HDF5 Writer (1 Hz, background)
///                              ↓
///                         experiment_data.h5
/// ```
///
/// # The Mullet Strategy
///
/// - **Party in front**: Fast Arrow writes, scientists see f64/Vec<f64>
/// - **Business in back**: HDF5 files for compatibility
/// - **Never blocking**: Async background task, 1 second flush interval
///
/// # Example
///
/// ```no_run
/// use storage::ring_buffer::RingBuffer;
/// use storage::hdf5_writer::HDF5Writer;
/// use storage::config::StorageConfig;
/// use std::sync::Arc;
/// use std::path::Path;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let config = StorageConfig::from_env();
///     let hdf5_path = config.data_path().join("experiment.h5");
///     let ring = Arc::new(RingBuffer::create(Path::new("/dev/shm/daq_ring"), 100)?);
///     let writer = HDF5Writer::new(&hdf5_path, ring.clone())?;
///
///     tokio::spawn(async move {
///         writer.run().await;
///     });
///
///     // Hardware loop continues without blocking...
///     Ok(())
/// }
/// ```
pub struct HDF5Writer {
    output_path: PathBuf,
    ring_buffer: Arc<RingBuffer>,
    flush_interval: Duration,
    /// Threshold for adaptive flushing (bytes). Default 10MB.
    adaptive_threshold: u64,
    /// Polling interval for checking adaptive threshold. Default 100ms.
    poll_interval: Duration,
    last_read_tail: AtomicU64,
    batch_counter: AtomicU64,
}

impl HDF5Writer {
    /// Create new HDF5 writer
    ///
    /// # Arguments
    ///
    /// * `output_path` - Path to HDF5 file to create/append
    /// * `ring_buffer` - Shared ring buffer to read from
    pub fn new(output_path: &Path, ring_buffer: Arc<RingBuffer>) -> Result<Self> {
        // Don't create HDF5 file yet - wait for first flush
        // This allows testing without hdf5 feature enabled

        Ok(Self {
            output_path: output_path.to_path_buf(),
            ring_buffer,
            flush_interval: Duration::from_secs(1),
            adaptive_threshold: 10 * 1024 * 1024, // 10 MB
            poll_interval: Duration::from_millis(100),
            last_read_tail: AtomicU64::new(0),
            batch_counter: AtomicU64::new(0),
        })
    }

    /// Run background writer loop
    ///
    /// This never returns - it runs continuously until the task is cancelled.
    /// Flushes data every `flush_interval` (default 1 second).
    pub async fn run(self) {
        self.run_loop().await;
    }

    /// Run the writer loop using a shared reference.
    pub async fn run_shared(self: Arc<Self>) {
        self.run_loop().await;
    }

    /// Flush interval accessor.
    pub fn flush_interval(&self) -> Duration {
        self.flush_interval
    }

    /// Override the flush interval (primarily for tests and recording control).
    pub fn set_flush_interval(&mut self, interval: Duration) {
        self.flush_interval = interval;
    }

    /// Set adaptive flush threshold (bytes).
    pub fn set_adaptive_threshold(&mut self, bytes: u64) {
        self.adaptive_threshold = bytes;
    }

    /// Set poll interval for adaptive flushing.
    ///
    /// # Errors
    ///
    /// Returns `DaqError::Configuration` if `interval` is zero
    /// (`tokio::time::interval(Duration::ZERO)` panics).
    pub fn set_poll_interval(&mut self, interval: Duration) -> Result<()> {
        if interval.is_zero() {
            return Err(DaqError::Configuration(
                "poll_interval must be non-zero".into(),
            ));
        }
        self.poll_interval = interval;
        Ok(())
    }

    /// Inject a snapshot of all parameters into the HDF5 file as attributes.
    ///
    /// Parameters are serialized to JSON and stored under a `parameters` group,
    /// one attribute per parameter name. Existing attributes are overwritten.
    #[cfg(feature = "storage_hdf5")]
    pub async fn inject_parameters(&self, params: &ParameterSet) -> Result<()> {
        type ParamSnapshotEntry = (
            String,
            serde_json::Value,
            Option<String>,
            Option<String>,
            bool,
        );

        // Capture snapshot outside blocking section
        let snapshot: Vec<ParamSnapshotEntry> = params
            .iter()
            .map(|(name, param)| {
                let value = param.get_json().map_err(|e| {
                    DaqError::Storage(StorageError::new(
                        StorageErrorKind::Serialization,
                        e.to_string(),
                    ))
                })?;
                let meta = param.metadata();
                Ok((
                    name.to_string(),
                    value,
                    meta.description.clone(),
                    meta.units.clone(),
                    meta.read_only,
                ))
            })
            .collect::<Result<_>>()?;

        let output_path = self.output_path.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            use hdf5::types::VarLenUnicode;
            use hdf5::File;

            let file = if output_path.exists() {
                File::open_rw(&output_path).map_err(map_hdf5_err)?
            } else {
                File::create(&output_path).map_err(map_hdf5_err)?
            };

            let params_group = if file.group("parameters").is_ok() {
                file.group("parameters")?
            } else {
                file.create_group("parameters")?
            };

            for (name, value, description, units, read_only) in &snapshot {
                let record = serde_json::json!({
                    "name": name,
                    "value": value,
                    "description": description,
                    "units": units,
                    "read_only": read_only,
                });

                let json_str = serde_json::to_string(&record)?;

                // Overwrite existing attribute if present
                if params_group.attr(name).is_ok() {
                    params_group
                        .attr(name)?
                        .write_scalar(&json_str.parse::<VarLenUnicode>().map_err(|e| {
                            DaqError::Storage(StorageError::new(
                                StorageErrorKind::Hdf5,
                                format!("VarLenUnicode parse: {}", e),
                            ))
                        })?)
                        .map_err(map_hdf5_err)?;
                } else {
                    params_group
                        .new_attr::<VarLenUnicode>()
                        .create(&name[..])
                        .map_err(map_hdf5_err)?
                        .write_scalar(&json_str.parse::<VarLenUnicode>().map_err(|e| {
                            DaqError::Storage(StorageError::new(
                                StorageErrorKind::Hdf5,
                                format!("VarLenUnicode parse: {}", e),
                            ))
                        })?)
                        .map_err(map_hdf5_err)?;
                }
            }

            Ok(())
        })
        .await??;

        Ok(())
    }

    async fn run_loop(&self) {
        let mut ticker = interval(self.poll_interval);
        let mut last_flush = tokio::time::Instant::now();

        loop {
            ticker.tick().await;

            // Check backlog to decide whether to flush
            let write_head = self.ring_buffer.write_head();
            let read_tail = self.last_read_tail.load(Ordering::Acquire);
            let backlog = write_head.saturating_sub(read_tail);

            // Flush if threshold exceeded OR time interval elapsed
            let should_flush =
                backlog >= self.adaptive_threshold || last_flush.elapsed() >= self.flush_interval;

            if should_flush {
                match self.flush_to_disk().await {
                    Ok(bytes) => {
                        if bytes > 0 {
                            // Only reset timer if we actually wrote data?
                            // Or reset regardless to maintain heartbeat?
                            // Standard strategy: reset timer on successful flush attempt
                            last_flush = tokio::time::Instant::now();
                        } else if last_flush.elapsed() >= self.flush_interval {
                            // If we tried to flush due to time but found no data,
                            // reset timer anyway to avoid spinning
                            last_flush = tokio::time::Instant::now();
                        }
                    }
                    Err(e) => {
                        eprintln!("HDF5 flush error: {}", e);
                        // Reset timer to back off on error
                        last_flush = tokio::time::Instant::now();
                    }
                }
            }
        }
    }

    /// Flush new data from ring buffer to HDF5 file
    ///
    /// This reads new data since last flush, decodes Protobuf messages,
    /// and writes structured datasets to HDF5.
    /// Non-blocking if no new data is available.
    #[cfg(feature = "storage_hdf5")]
    pub async fn flush_to_disk(&self) -> Result<usize> {
        // Check if there's new data by comparing write_head to our last read position
        let current_write_head = self.ring_buffer.write_head();
        let last_processed = self.last_read_tail.load(Ordering::Acquire);

        if current_write_head <= last_processed {
            // No new data since last flush
            return Ok(0);
        }

        // Clone data needed for blocking task
        let output_path = self.output_path.clone();
        let batch_id = self.batch_counter.fetch_add(1, Ordering::SeqCst);
        let ring_buffer = self.ring_buffer.clone();

        // Wrap ring buffer read AND HDF5 operations in spawn_blocking to prevent
        // executor stalls. read_snapshot() can block with progressive backoff
        // during high write contention (bd-jnfu.14).
        let bytes_processed = tokio::task::spawn_blocking(move || -> Result<usize> {
            // Read snapshot from ring buffer (can block during contention)
            let snapshot = ring_buffer.read_snapshot();
            if snapshot.is_empty() {
                return Ok(0);
            }
            use hdf5::File;
            // use prost::Message;

            // Open or create HDF5 file
            let file = if output_path.exists() {
                File::open_rw(&output_path).map_err(map_hdf5_err)?
            } else {
                File::create(&output_path).map_err(map_hdf5_err)?
            };

            // Create measurements group if it doesn't exist
            let measurements = if file.group("measurements").is_ok() {
                file.group("measurements").map_err(map_hdf5_err)?
            } else {
                file.create_group("measurements").map_err(map_hdf5_err)?
            };

            // Create batch group with unique name
            let batch_name = format!("batch_{:06}", batch_id);
            let batch_group = measurements
                .create_group(&batch_name)
                .map_err(map_hdf5_err)?;

            // Decode and write Protobuf ScanProgress messages
            // Returns bytes successfully processed to handle partial messages
            #[cfg(feature = "networking")]
            let bytes_processed =
                { Self::write_protobuf_to_hdf5_blocking(&batch_group, &snapshot)? };

            #[cfg(not(feature = "networking"))]
            let bytes_processed = {
                // Fallback: Write raw bytes when networking feature is disabled
                batch_group
                    .new_dataset::<u8>()
                    .shape(snapshot.len())
                    .create("raw_data")
                    .map_err(map_hdf5_err)?
                    .write(&snapshot)
                    .map_err(map_hdf5_err)?;
                snapshot.len()
            };

            // Add metadata
            batch_group
                .new_attr::<u64>()
                .create("ring_tail")
                .map_err(map_hdf5_err)?
                .write_scalar(&current_write_head)
                .map_err(map_hdf5_err)?;

            #[allow(clippy::cast_possible_truncation)]
            // SAFETY: Unix epoch nanos will not exceed u64::MAX until year 2554
            let timestamp_ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;
            batch_group
                .new_attr::<u64>()
                .create("timestamp_ns")
                .map_err(map_hdf5_err)?
                .write_scalar(&timestamp_ns)
                .map_err(map_hdf5_err)?;

            Ok(bytes_processed)
        })
        .await??;

        // Only advance by bytes actually processed (to preserve partial messages)
        let new_tail = last_processed + bytes_processed as u64;
        self.last_read_tail.store(new_tail, Ordering::Release);
        self.ring_buffer.advance_tail(bytes_processed as u64);

        Ok(bytes_processed)
    }

    /// Decode Protobuf ScanProgress messages and write to HDF5
    ///
    /// Returns the number of bytes successfully processed so partial messages
    /// are preserved in the ring buffer for the next flush.
    ///
    /// Converts ScanProgress messages to structured HDF5 datasets:
    /// - /scan_id (string attribute)
    /// - /timestamps (u64 array)
    /// - /point_indices (u32 array)
    /// - /axis_positions/<device_id> (f64 arrays)
    /// - /data/<device_id> (f64 arrays)
    ///
    /// NOTE: This is a blocking synchronous method called from within spawn_blocking
    #[cfg(all(feature = "storage_hdf5", feature = "networking"))]
    fn write_protobuf_to_hdf5_blocking(group: &hdf5::Group, data: &[u8]) -> Result<usize> {
        use prost::Message;
        use protocol::daq::ScanProgress;
        use std::collections::HashMap;

        // Decode length-prefixed messages from buffer
        let mut offset = 0;
        let mut last_complete_offset = 0; // Track last fully processed message
        let mut timestamps: Vec<u64> = Vec::new();
        let mut point_indices: Vec<u32> = Vec::new();
        let mut axis_positions: HashMap<String, Vec<f64>> = HashMap::new();
        let mut data_values: HashMap<String, Vec<f64>> = HashMap::new();
        let mut scan_id: Option<String> = None;

        while offset + 4 <= data.len() {
            // Read message length (4 bytes, little-endian)
            let len_bytes: [u8; 4] = data[offset..offset + 4].try_into()?;
            let msg_len = u32::from_le_bytes(len_bytes) as usize;

            if offset + 4 + msg_len > data.len() {
                // Incomplete message - stop here, don't consume partial data
                break;
            }

            // Move past length prefix
            offset += 4;

            // Decode ScanProgress message
            match ScanProgress::decode(&data[offset..offset + msg_len]) {
                Ok(progress) => {
                    // Store scan ID from first message
                    if scan_id.is_none() && !progress.scan_id.is_empty() {
                        scan_id = Some(progress.scan_id.clone());
                    }

                    timestamps.push(progress.timestamp_ns);
                    point_indices.push(progress.point_index);

                    // Store axis positions
                    for (device_id, position) in &progress.axis_positions {
                        axis_positions
                            .entry(device_id.clone())
                            .or_default()
                            .push(*position);
                    }

                    // Store data points
                    for data_point in &progress.data_points {
                        data_values
                            .entry(data_point.device_id.clone())
                            .or_default()
                            .push(data_point.value);
                    }
                }
                Err(e) => {
                    eprintln!("Failed to decode ScanProgress: {}", e);
                    // Still advance past this message even if decode fails
                }
            }

            offset += msg_len;
            last_complete_offset = offset; // Mark this as successfully processed
        }

        // Write scan_id as attribute
        if let Some(ref id) = scan_id {
            // HDF5 string attributes require special handling
            group
                .new_attr::<hdf5::types::VarLenUnicode>()
                .create("scan_id")
                .map_err(map_hdf5_err)?
                .write_scalar(&id.parse::<hdf5::types::VarLenUnicode>().map_err(|e| {
                    DaqError::Storage(StorageError::new(
                        StorageErrorKind::Hdf5,
                        format!("VarLenUnicode parse: {}", e),
                    ))
                })?)
                .map_err(map_hdf5_err)?;
        }

        // Write timestamps
        if !timestamps.is_empty() {
            group
                .new_dataset::<u64>()
                .create("timestamps")
                .map_err(map_hdf5_err)?
                .write(&timestamps)
                .map_err(map_hdf5_err)?;
        }

        // Write point indices
        if !point_indices.is_empty() {
            group
                .new_dataset::<u32>()
                .create("point_indices")
                .map_err(map_hdf5_err)?
                .write(&point_indices)
                .map_err(map_hdf5_err)?;
        }

        // Write axis positions as subgroup
        if !axis_positions.is_empty() {
            let axes_group = group.create_group("axis_positions").map_err(map_hdf5_err)?;
            for (device_id, positions) in &axis_positions {
                axes_group
                    .new_dataset::<f64>()
                    .create(device_id.as_str())
                    .map_err(map_hdf5_err)?
                    .write(positions)
                    .map_err(map_hdf5_err)?;
            }
        }

        // Write data values as subgroup
        if !data_values.is_empty() {
            let data_group = group.create_group("data").map_err(map_hdf5_err)?;
            for (device_id, values) in &data_values {
                data_group
                    .new_dataset::<f64>()
                    .create(device_id.as_str())
                    .map_err(map_hdf5_err)?
                    .write(values)
                    .map_err(map_hdf5_err)?;
            }
        }

        Ok(last_complete_offset)
    }

    /// Fallback implementation when HDF5 feature is disabled
    /// Writes scan data as CSV for basic persistence
    #[cfg(not(feature = "storage_hdf5"))]
    pub async fn flush_to_disk(&self) -> Result<usize> {
        // Check if there's new data
        let current_write_head = self.ring_buffer.write_head();
        let last_processed = self.last_read_tail.load(Ordering::Acquire);

        if current_write_head <= last_processed {
            return Ok(0);
        }

        // Clone data for blocking task
        let fallback_path = self.output_path.with_extension("bin");
        let ring_buffer = self.ring_buffer.clone();

        // Wrap ring buffer read AND file I/O in spawn_blocking to prevent
        // executor stalls. read_snapshot() can block with progressive backoff
        // during high write contention (bd-jnfu.14).
        let snapshot_len = tokio::task::spawn_blocking(move || -> Result<usize> {
            // Read snapshot from ring buffer (can block during contention)
            let snapshot = ring_buffer.read_snapshot();
            if snapshot.is_empty() {
                return Ok(0);
            }
            let snapshot_len = snapshot.len();
            use std::fs::OpenOptions;
            use std::io::Write;

            // Write raw bytes to a binary file as fallback
            // This preserves the Protobuf-encoded ScanProgress data
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&fallback_path)?;

            // Write length-prefixed message (allows decoding multiple messages)
            let len = snapshot_len as u32;
            file.write_all(&len.to_le_bytes())?;
            file.write_all(&snapshot)?;

            Ok(snapshot_len)
        })
        .await??;

        // Early return if no data was written
        if snapshot_len == 0 {
            return Ok(0);
        }

        // Advance tail to prevent buffer overflow
        self.ring_buffer.advance_tail(snapshot_len as u64);
        self.last_read_tail
            .store(current_write_head, Ordering::Release);

        Ok(snapshot_len)
    }

    /// Write ExperimentManifest to HDF5 file as attributes (bd-ib06)
    ///
    /// Stores manifest under /manifest/ group for experiment reproducibility:
    /// - /manifest/timestamp_ns
    /// - /manifest/run_uid
    /// - /manifest/plan_type
    /// - /manifest/plan_name
    /// - /manifest/parameters/<device_id>/<param_name> (JSON string)
    /// - /manifest/system/<key> (software version, hostname, etc.)
    /// - /manifest/metadata/<key> (user metadata)
    ///
    /// # Arguments
    ///
    /// * `manifest` - The ExperimentManifest to persist
    ///
    /// # Errors
    ///
    /// Returns error if HDF5 file operations fail
    #[cfg(feature = "storage_hdf5")]
    pub async fn write_manifest(
        &self,
        manifest: &common::experiment::document::ExperimentManifest,
    ) -> Result<()> {
        // Clone manifest for move into blocking task
        let manifest = manifest.clone();
        let output_path = self.output_path.clone();

        // Wrap all HDF5 operations in spawn_blocking to prevent executor stalls
        tokio::task::spawn_blocking(move || -> Result<()> {
            use hdf5::File;

            // Open or create HDF5 file
            let file = if output_path.exists() {
                File::open_rw(&output_path).map_err(map_hdf5_err)?
            } else {
                File::create(&output_path).map_err(map_hdf5_err)?
            };

            // Create manifest group if it doesn't exist
            let manifest_group = if file.group("manifest").is_ok() {
                file.group("manifest").map_err(map_hdf5_err)?
            } else {
                file.create_group("manifest").map_err(map_hdf5_err)?
            };

            // Write basic manifest attributes
            manifest_group
                .new_attr::<u64>()
                .create("timestamp_ns")
                .map_err(map_hdf5_err)?
                .write_scalar(&manifest.timestamp_ns)
                .map_err(map_hdf5_err)?;

            manifest_group
                .new_attr::<hdf5::types::VarLenUnicode>()
                .create("run_uid")
                .map_err(map_hdf5_err)?
                .write_scalar(
                    &manifest
                        .run_uid
                        .parse::<hdf5::types::VarLenUnicode>()
                        .map_err(|e| {
                            DaqError::Storage(StorageError::new(
                                StorageErrorKind::Hdf5,
                                format!("VarLenUnicode parse: {}", e),
                            ))
                        })?,
                )
                .map_err(map_hdf5_err)?;

            manifest_group
                .new_attr::<hdf5::types::VarLenUnicode>()
                .create("plan_type")
                .map_err(map_hdf5_err)?
                .write_scalar(
                    &manifest
                        .plan_type
                        .as_str()
                        .parse::<hdf5::types::VarLenUnicode>()
                        .map_err(|e| {
                            DaqError::Storage(StorageError::new(
                                StorageErrorKind::Hdf5,
                                format!("VarLenUnicode parse: {}", e),
                            ))
                        })?,
                )
                .map_err(map_hdf5_err)?;

            manifest_group
                .new_attr::<hdf5::types::VarLenUnicode>()
                .create("plan_name")
                .map_err(map_hdf5_err)?
                .write_scalar(
                    &manifest
                        .plan_name
                        .as_str()
                        .parse::<hdf5::types::VarLenUnicode>()
                        .map_err(|e| {
                            DaqError::Storage(StorageError::new(
                                StorageErrorKind::Hdf5,
                                format!("VarLenUnicode parse: {}", e),
                            ))
                        })?,
                )
                .map_err(map_hdf5_err)?;

            // Create parameters subgroup
            let params_group = if manifest_group.group("parameters").is_ok() {
                manifest_group.group("parameters").map_err(map_hdf5_err)?
            } else {
                manifest_group
                    .create_group("parameters")
                    .map_err(map_hdf5_err)?
            };

            // Write device parameters as JSON attributes
            for (device_id, params) in &manifest.parameters {
                let device_group = if params_group.group(device_id).is_ok() {
                    params_group.group(device_id).map_err(map_hdf5_err)?
                } else {
                    params_group.create_group(device_id).map_err(map_hdf5_err)?
                };

                for (param_name, param_value) in params {
                    // Serialize JSON value to string for HDF5 storage
                    let json_str = serde_json::to_string(param_value).map_err(|e| {
                        DaqError::Storage(StorageError::new(
                            StorageErrorKind::Serialization,
                            e.to_string(),
                        ))
                    })?;
                    device_group
                        .new_attr::<hdf5::types::VarLenUnicode>()
                        .create(&param_name[..])
                        .map_err(map_hdf5_err)?
                        .write_scalar(&json_str.parse::<hdf5::types::VarLenUnicode>().map_err(
                            |e| {
                                DaqError::Storage(StorageError::new(
                                    StorageErrorKind::Hdf5,
                                    format!("VarLenUnicode parse: {}", e),
                                ))
                            },
                        )?)
                        .map_err(map_hdf5_err)?;
                }
            }

            // Create system_info subgroup
            let system_group = if manifest_group.group("system").is_ok() {
                manifest_group.group("system").map_err(map_hdf5_err)?
            } else {
                manifest_group
                    .create_group("system")
                    .map_err(map_hdf5_err)?
            };

            for (key, value) in &manifest.system_info {
                system_group
                    .new_attr::<hdf5::types::VarLenUnicode>()
                    .create(&key[..])
                    .map_err(map_hdf5_err)?
                    .write_scalar(&value.parse::<hdf5::types::VarLenUnicode>().map_err(|e| {
                        DaqError::Storage(StorageError::new(
                            StorageErrorKind::Hdf5,
                            format!("VarLenUnicode parse: {}", e),
                        ))
                    })?)
                    .map_err(map_hdf5_err)?;
            }

            // Create metadata subgroup
            let metadata_group = if manifest_group.group("metadata").is_ok() {
                manifest_group.group("metadata").map_err(map_hdf5_err)?
            } else {
                manifest_group
                    .create_group("metadata")
                    .map_err(map_hdf5_err)?
            };

            for (key, value) in &manifest.metadata {
                metadata_group
                    .new_attr::<hdf5::types::VarLenUnicode>()
                    .create(&key[..])
                    .map_err(map_hdf5_err)?
                    .write_scalar(&value.parse::<hdf5::types::VarLenUnicode>().map_err(|e| {
                        DaqError::Storage(StorageError::new(
                            StorageErrorKind::Hdf5,
                            format!("VarLenUnicode parse: {}", e),
                        ))
                    })?)
                    .map_err(map_hdf5_err)?;
            }

            Ok(())
        })
        .await??;

        Ok(())
    }

    /// No-op when storage_hdf5 feature is disabled
    #[cfg(not(feature = "storage_hdf5"))]
    pub async fn write_manifest(
        &self,
        _manifest: &common::experiment::document::ExperimentManifest,
    ) -> Result<()> {
        // Gracefully degrade when HDF5 not available
        Ok(())
    }

    /// Helper to write a string as a VarLenUnicode attribute
    #[cfg(all(feature = "storage_hdf5", feature = "storage_arrow"))]
    #[allow(dead_code)] // reserved for Arrow->HDF5 bridge path
    fn write_string_attr(container: &hdf5::Container, name: &str, value: &str) -> Result<()> {
        use hdf5::types::VarLenUnicode;
        container
            .new_attr::<VarLenUnicode>()
            .create(name)
            .map_err(map_hdf5_err)?
            .write_scalar(&value.parse::<VarLenUnicode>().map_err(|e| {
                DaqError::Storage(StorageError::new(
                    StorageErrorKind::Hdf5,
                    format!("VarLenUnicode parse: {}", e),
                ))
            })?)
            .map_err(map_hdf5_err)?;
        Ok(())
    }

    /// Write Arrow RecordBatch to HDF5 group
    ///
    /// Converts Arrow columns to HDF5 datasets for compatibility with
    /// Python/MATLAB/Igor analysis tools.
    #[cfg(all(feature = "storage_hdf5", feature = "storage_arrow"))]
    #[allow(dead_code)] // reserved for Arrow->HDF5 bridge path
    fn write_arrow_to_hdf5(&self, group: &hdf5::Group, data: &[u8]) -> Result<()> {
        use arrow::ipc::reader::FileReader;
        use std::io::Cursor;

        let cursor = Cursor::new(data);
        let mut reader = FileReader::try_new(cursor, None).map_err(|e| {
            DaqError::Storage(StorageError::new(
                StorageErrorKind::Arrow,
                format!("Failed to create Arrow reader: {}", e),
            ))
        })?;

        // Read first batch
        if let Some(batch) = reader.next().transpose().map_err(|e| {
            DaqError::Storage(StorageError::new(
                StorageErrorKind::Arrow,
                format!("Failed to read Arrow batch: {}", e),
            ))
        })? {
            let schema = batch.schema();

            // Write each column as a separate dataset
            for (i, field) in schema.fields().iter().enumerate() {
                let column = batch.column(i);
                let dataset_name = field.name();

                // Convert Arrow array to HDF5-compatible format
                match column.data_type() {
                    arrow::datatypes::DataType::Float64 => {
                        use arrow::array::Float64Array;
                        let array =
                            column
                                .as_any()
                                .downcast_ref::<Float64Array>()
                                .ok_or_else(|| {
                                    DaqError::Storage(StorageError::new(
                                        StorageErrorKind::Arrow,
                                        "Failed to downcast Float64Array".to_string(),
                                    ))
                                })?;

                        let values: Vec<f64> = (0..array.len()).map(|i| array.value(i)).collect();

                        group
                            .new_dataset::<f64>()
                            .create(dataset_name.as_str())
                            .map_err(|e| {
                                DaqError::Storage(StorageError::new(
                                    StorageErrorKind::Hdf5,
                                    e.to_string(),
                                ))
                            })?
                            .write(&values)
                            .map_err(|e| {
                                DaqError::Storage(StorageError::new(
                                    StorageErrorKind::Hdf5,
                                    e.to_string(),
                                ))
                            })?;
                    }
                    arrow::datatypes::DataType::Int64 => {
                        use arrow::array::Int64Array;
                        let array =
                            column
                                .as_any()
                                .downcast_ref::<Int64Array>()
                                .ok_or_else(|| {
                                    DaqError::Storage(StorageError::new(
                                        StorageErrorKind::Arrow,
                                        "Failed to downcast Int64Array".to_string(),
                                    ))
                                })?;

                        let values: Vec<i64> = (0..array.len()).map(|i| array.value(i)).collect();

                        group
                            .new_dataset::<i64>()
                            .create(dataset_name.as_str())
                            .map_err(|e| {
                                DaqError::Storage(StorageError::new(
                                    StorageErrorKind::Hdf5,
                                    e.to_string(),
                                ))
                            })?
                            .write(&values)
                            .map_err(|e| {
                                DaqError::Storage(StorageError::new(
                                    StorageErrorKind::Hdf5,
                                    e.to_string(),
                                ))
                            })?;
                    }
                    _ => {
                        // Fallback for unsupported types - write as string
                        eprintln!(
                            "Warning: Unsupported Arrow type for HDF5: {:?}",
                            column.data_type()
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Get number of batches written so far
    pub fn batch_count(&self) -> u64 {
        self.batch_counter.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    #[cfg(feature = "storage_hdf5")]
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_hdf5_writer_create() {
        let ring_temp = NamedTempFile::new().unwrap();
        let hdf5_temp = NamedTempFile::new().unwrap();

        let ring = Arc::new(RingBuffer::create(ring_temp.path(), 1).unwrap());
        let writer = HDF5Writer::new(hdf5_temp.path(), ring.clone()).unwrap();

        assert_eq!(writer.batch_count(), 0);
        assert_eq!(writer.flush_interval(), Duration::from_secs(1));
    }

    #[tokio::test]
    async fn test_hdf5_writer_flush_empty() {
        let ring_temp = NamedTempFile::new().unwrap();
        let hdf5_temp = NamedTempFile::new().unwrap();

        let ring = Arc::new(RingBuffer::create(ring_temp.path(), 1).unwrap());
        let writer = HDF5Writer::new(hdf5_temp.path(), ring.clone()).unwrap();

        // Flush with no data should not error
        writer.flush_to_disk().await.unwrap();
        assert_eq!(writer.batch_count(), 0);
    }

    #[tokio::test]
    async fn test_hdf5_writer_non_blocking() {
        let ring_temp = NamedTempFile::new().unwrap();
        let hdf5_temp = NamedTempFile::new().unwrap();

        let ring = Arc::new(RingBuffer::create(ring_temp.path(), 1).unwrap());
        let writer = HDF5Writer::new(hdf5_temp.path(), ring.clone()).unwrap();

        // Write some data to ring buffer
        ring.write(b"Test data for non-blocking verification")
            .unwrap();

        // Start background task
        let handle = tokio::spawn(async move {
            // Run for a short time
            let mut interval = interval(Duration::from_millis(100));
            for _ in 0..5 {
                interval.tick().await;
                let _ = writer.flush_to_disk().await;
            }
        });

        // Verify main thread is not blocked
        tokio::time::sleep(Duration::from_millis(50)).await;

        handle.await.unwrap();
    }

    #[cfg(feature = "storage_hdf5")]
    #[tokio::test]
    async fn test_hdf5_writer_creates_file() {
        let ring_temp = NamedTempFile::new().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let hdf5_path = temp_dir.path().join("test_output.h5");

        let ring = Arc::new(RingBuffer::create(ring_temp.path(), 1).unwrap());
        let writer = HDF5Writer::new(&hdf5_path, ring.clone()).unwrap();

        // Write some data
        ring.write(b"HDF5 test data").unwrap();

        // Flush to disk
        writer.flush_to_disk().await.unwrap();

        // Verify file was created
        assert!(hdf5_path.exists(), "HDF5 file should be created");
    }

    #[cfg(all(feature = "storage_hdf5", feature = "storage_arrow"))]
    #[tokio::test]
    async fn test_hdf5_writer_arrow_integration() {
        use arrow::array::{Float64Array, Int64Array};
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow::record_batch::RecordBatch;
        use std::sync::Arc as StdArc;

        let ring_temp = NamedTempFile::new().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let hdf5_path = temp_dir.path().join("arrow_test.h5");

        let ring = Arc::new(RingBuffer::create(ring_temp.path(), 1).unwrap());
        let writer = HDF5Writer::new(&hdf5_path, ring.clone()).unwrap();

        // Create Arrow RecordBatch
        let schema = Schema::new(vec![
            Field::new("timestamp", DataType::Int64, false),
            Field::new("voltage", DataType::Float64, false),
        ]);

        let timestamps = Int64Array::from(vec![1, 2, 3, 4, 5]);
        let voltages = Float64Array::from(vec![1.1, 2.2, 3.3, 4.4, 5.5]);

        let batch = RecordBatch::try_new(
            StdArc::new(schema),
            vec![StdArc::new(timestamps), StdArc::new(voltages)],
        )
        .unwrap();

        // Write to ring buffer
        ring.write_arrow_batch(&batch).unwrap();

        // Flush to HDF5
        writer.flush_to_disk().await.unwrap();

        // Verify file created and contains data
        assert!(hdf5_path.exists());
        assert!(writer.batch_count() > 0);
    }

    #[cfg(feature = "storage_hdf5")]
    #[tokio::test]
    async fn test_hdf5_writer_adaptive_flush() {
        let ring_temp = NamedTempFile::new().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let hdf5_path = temp_dir.path().join("adaptive.h5");

        let ring = Arc::new(RingBuffer::create(ring_temp.path(), 1).unwrap());
        let mut writer = HDF5Writer::new(&hdf5_path, ring.clone()).unwrap();

        // Configure for adaptive flush test
        writer.set_flush_interval(Duration::from_secs(10)); // Long interval
        writer.set_adaptive_threshold(1024); // 1 KB threshold
        writer.set_poll_interval(Duration::from_millis(10)).unwrap(); // Fast polling

        // Write small data (below threshold)
        ring.write(&[0u8; 500]).unwrap();

        // Start background writer
        let writer_arc = Arc::new(writer);
        let writer_clone = writer_arc.clone();
        let handle = tokio::spawn(async move {
            writer_clone.run_shared().await;
        });

        // Poll until we confirm no flush happened for the small write.
        // 200ms is well above the 10ms poll interval — if a flush were going
        // to happen it would have by now.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(writer_arc.batch_count(), 0, "Should not flush small batch");

        // Write more data (exceeds 1KB threshold total)
        ring.write(&[0u8; 600]).unwrap(); // Total 1100 bytes

        // Poll batch_count until it advances (bounded by 2s deadline)
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while writer_arc.batch_count() == 0 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "Timed out waiting for adaptive flush"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        assert!(
            writer_arc.batch_count() > 0,
            "Should flush when threshold exceeded"
        );

        handle.abort();
    }

    #[tokio::test]
    async fn test_set_poll_interval_rejects_zero() {
        let ring_temp = NamedTempFile::new().unwrap();
        let hdf5_temp = NamedTempFile::new().unwrap();

        let ring = Arc::new(RingBuffer::create(ring_temp.path(), 1).unwrap());
        let mut writer = HDF5Writer::new(hdf5_temp.path(), ring).unwrap();

        let result = writer.set_poll_interval(Duration::ZERO);
        assert!(result.is_err(), "Duration::ZERO should be rejected");

        // Non-zero should succeed
        writer.set_poll_interval(Duration::from_millis(1)).unwrap();
    }
}
