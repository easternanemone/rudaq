//! HDF5 Storage Actor for V4 DAQ System
//!
//! Persists Arrow RecordBatch data to HDF5 files with proper metadata,
//! schema preservation, and file rotation support.

use crate::config::StorageConfig;
use anyhow::Result;
use kameo::actor::{ActorRef, WeakActorRef};
use kameo::error::BoxSendError;
use kameo::message::{Context, Message};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

#[cfg(feature = "storage_hdf5")]
use hdf5::File;

/// Storage statistics for monitoring and diagnostics
#[derive(Debug, Clone, Default, kameo::Reply)]
pub struct StorageStats {
    /// Total bytes written in current session
    pub bytes_written: u64,
    /// Total batches written
    pub batches_written: u64,
    /// Current HDF5 file path
    pub file_path: PathBuf,
    /// Current file size in bytes
    pub file_size: u64,
    /// Number of datasets (instruments) in current file
    pub num_datasets: u64,
}

/// HDF5 Storage Actor State
pub struct HDF5Storage {
    /// Base output directory from config
    output_dir: PathBuf,
    /// Current HDF5 file handle
    #[cfg(feature = "storage_hdf5")]
    file: Option<File>,
    /// Current file path
    current_file_path: PathBuf,
    /// Compression level (0-9)
    compression_level: u8,
    /// Bytes written in current session
    bytes_written: u64,
    /// Batches written in current session
    batches_written: u64,
    /// Per-instrument metadata
    metadata: HashMap<String, HashMap<String, String>>,
    /// Auto-flush interval in seconds (0 = manual only)
    auto_flush_interval_secs: u64,
    /// Last flush time
    last_flush: SystemTime,
}

impl HDF5Storage {
    /// Create new HDF5 storage actor
    pub fn new(config: &StorageConfig) -> Self {
        Self {
            output_dir: config.output_dir.clone(),
            #[cfg(feature = "storage_hdf5")]
            file: None,
            current_file_path: PathBuf::new(),
            compression_level: config.compression_level,
            bytes_written: 0,
            batches_written: 0,
            metadata: HashMap::new(),
            auto_flush_interval_secs: config.auto_flush_interval_secs,
            last_flush: SystemTime::now(),
        }
    }

    /// Initialize and open HDF5 file
    #[cfg(feature = "storage_hdf5")]
    async fn init_file(&mut self) -> Result<()> {
        // Ensure output directory exists
        if !self.output_dir.exists() {
            std::fs::create_dir_all(&self.output_dir).with_context(|| {
                format!("Failed to create output directory: {:?}", self.output_dir)
            })?;
        }

        // Create filename with timestamp
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let filename = format!("daq_session_{}.h5", timestamp);
        let file_path = self.output_dir.join(&filename);

        // Create HDF5 file
        let file = File::create(&file_path)
            .with_context(|| format!("Failed to create HDF5 file at {:?}", file_path))?;

        // Store session metadata
        self.add_root_attributes(&file)?;

        self.file = Some(file);
        self.current_file_path = file_path;

        tracing::info!("HDF5 file opened: {:?}", self.current_file_path);
        Ok(())
    }

    /// Add root-level attributes to HDF5 file
    #[cfg(feature = "storage_hdf5")]
    fn add_root_attributes(&self, file: &File) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();

        file.new_attr::<String>()
            .create("created_at")?
            .write_scalar(&chrono::Utc::now().to_rfc3339())?;

        file.new_attr::<u64>()
            .create("created_timestamp_ns")?
            .write_scalar(&(now.as_nanos() as u64))?;

        file.new_attr::<String>()
            .create("application")?
            .write_scalar(&"Rust DAQ V4".to_string())?;

        Ok(())
    }

    /// Get or create dataset group for an instrument
    #[cfg(feature = "storage_hdf5")]
    fn get_or_create_instrument_group(&mut self, instrument_id: &str) -> Result<Group> {
        let file = self
            .file
            .as_ref()
            .ok_or_else(|| anyhow!("File not initialized"))?;

        // Check if group already exists
        match file.group(instrument_id) {
            Ok(group) => Ok(group),
            Err(_) => {
                // Create new group for this instrument
                let group = file.create_group(instrument_id).with_context(|| {
                    format!("Failed to create group for instrument: {}", instrument_id)
                })?;

                // Add instrument metadata
                group
                    .new_attr::<String>()
                    .create("instrument_id")?
                    .write_scalar(&instrument_id.to_string())?;

                group
                    .new_attr::<String>()
                    .create("created_at")?
                    .write_scalar(&chrono::Utc::now().to_rfc3339())?;

                Ok(group)
            }
        }
    }

    /// Write Arrow RecordBatch to HDF5
    #[cfg(feature = "storage_hdf5")]
    async fn write_batch_hdf5(
        &mut self,
        batch: &arrow::record_batch::RecordBatch,
        instrument_id: &str,
    ) -> Result<()> {
        let group = self.get_or_create_instrument_group(instrument_id)?;

        // Get number of rows
        let num_rows = batch.num_rows() as u64;

        // Store schema as JSON attribute
        let schema = batch.schema();
        let schema_json = serde_json::to_string(&schema.to_string())?;
        group
            .new_attr::<String>()
            .create("schema")?
            .write_scalar(&schema_json)?;

        // Write each column as a dataset
        for (col_idx, field) in schema.fields().iter().enumerate() {
            let column = batch.column(col_idx);
            let col_name = field.name();

            // Store column data
            // For simplicity, convert to Vec and write as dataset
            // In production, use Arrow's native HDF5 conversion if available
            self.write_column_data(&group, col_name, column, num_rows)?;
        }

        // Update statistics
        self.batches_written += 1;

        Ok(())
    }

    /// Write Arrow array column to HDF5 dataset
    #[cfg(feature = "storage_hdf5")]
    fn write_column_data(
        &self,
        group: &Group,
        col_name: &str,
        column: &Arc<dyn arrow::array::Array>,
        num_rows: u64,
    ) -> Result<()> {
        use arrow::array::*;

        // Handle different Arrow types
        match column.data_type() {
            arrow::datatypes::DataType::Float64 => {
                let array = column
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .ok_or_else(|| anyhow!("Failed to downcast to Float64Array"))?;

                let values: Vec<f64> = (0..array.len()).map(|i| array.value(i)).collect();

                group
                    .create_dataset::<f64>(col_name)?
                    .write_slice(&values, hdf5::s![..])
                    .context("Failed to write Float64 column")?;
            }
            arrow::datatypes::DataType::Int64 => {
                let array = column
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .ok_or_else(|| anyhow!("Failed to downcast to Int64Array"))?;

                let values: Vec<i64> = (0..array.len()).map(|i| array.value(i)).collect();

                group
                    .create_dataset::<i64>(col_name)?
                    .write_slice(&values, hdf5::s![..])
                    .context("Failed to write Int64 column")?;
            }
            arrow::datatypes::DataType::Utf8 => {
                let array = column
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or_else(|| anyhow!("Failed to downcast to StringArray"))?;

                // HDF5 variable-length strings
                let values: Vec<String> = (0..array.len())
                    .map(|i| array.value(i).to_string())
                    .collect();

                group
                    .create_dataset::<String>(col_name)?
                    .write_slice(&values, hdf5::s![..])
                    .context("Failed to write String column")?;
            }
            arrow::datatypes::DataType::Int32 => {
                let array = column
                    .as_any()
                    .downcast_ref::<Int32Array>()
                    .ok_or_else(|| anyhow!("Failed to downcast to Int32Array"))?;

                let values: Vec<i32> = (0..array.len()).map(|i| array.value(i)).collect();

                group
                    .create_dataset::<i32>(col_name)?
                    .write_slice(&values, hdf5::s![..])
                    .context("Failed to write Int32 column")?;
            }
            _ => {
                tracing::warn!(
                    "Unsupported Arrow type for column {}: {:?}",
                    col_name,
                    column.data_type()
                );
            }
        }

        Ok(())
    }

    /// Flush pending writes to disk
    #[cfg(feature = "storage_hdf5")]
    async fn flush(&mut self) -> Result<()> {
        if let Some(file) = &self.file {
            file.flush().context("Failed to flush HDF5 file")?;

            self.last_flush = SystemTime::now();
            tracing::debug!("HDF5 file flushed");
        }
        Ok(())
    }

    /// Close current HDF5 file
    #[cfg(feature = "storage_hdf5")]
    async fn close_file(&mut self) -> Result<()> {
        if let Some(_file) = self.file.take() {
            self.flush().await?;
            tracing::info!("HDF5 file closed: {:?}", self.current_file_path);
        }
        Ok(())
    }
}

impl Default for HDF5Storage {
    fn default() -> Self {
        let config = StorageConfig {
            default_backend: "hdf5".to_string(),
            output_dir: PathBuf::from("./data"),
            compression_level: 6,
            auto_flush_interval_secs: 0,
        };
        Self::new(&config)
    }
}

impl kameo::Actor for HDF5Storage {
    type Args = Self;
    type Error = BoxSendError;

    async fn on_start(
        mut args: Self::Args,
        _actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        #[cfg(feature = "storage_hdf5")]
        {
            if let Err(err) = args.init_file().await {
                tracing::error!("Failed to initialize HDF5 file: {}", err);
                let error_msg: Box<dyn Any + Send> =
                    Box::new(format!("HDF5 initialization failed: {}", err));
                return Err(SendError::HandlerError(error_msg));
            }
        }

        #[cfg(not(feature = "storage_hdf5"))]
        {
            tracing::warn!("HDF5 storage feature not enabled; file operations disabled");
        }

        tracing::info!("HDF5 Storage actor started");
        Ok(args)
    }

    async fn on_stop(
        &mut self,
        _actor_ref: WeakActorRef<Self>,
        _reason: kameo::error::ActorStopReason,
    ) -> Result<(), Self::Error> {
        #[cfg(feature = "storage_hdf5")]
        {
            if let Err(err) = self.close_file().await {
                tracing::error!("Failed to close HDF5 file: {}", err);
            }
        }

        tracing::info!("HDF5 Storage actor stopped");
        Ok(())
    }
}

// Message definitions

/// Write Arrow RecordBatch to HDF5
#[derive(Debug)]
pub struct WriteBatch {
    /// Arrow RecordBatch to write
    pub batch: Option<Vec<u8>>, // Serialized Arrow IPC format for feature-gated compilation
    /// Instrument ID for grouping
    pub instrument_id: String,
}

impl Message<WriteBatch> for HDF5Storage {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: WriteBatch,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        #[cfg(feature = "storage_hdf5")]
        {
            // Note: In production, deserialize from Arrow IPC format
            // This is a placeholder that accepts data structure
            tracing::debug!("Writing batch for instrument: {}", msg.instrument_id);

            // Check if auto-flush is needed
            if self.auto_flush_interval_secs > 0 {
                if let Ok(elapsed) = self.last_flush.elapsed() {
                    if elapsed.as_secs() >= self.auto_flush_interval_secs {
                        self.flush().await?;
                    }
                }
            }

            Ok(())
        }

        #[cfg(not(feature = "storage_hdf5"))]
        {
            tracing::warn!("WriteBatch received but HDF5 feature not enabled");
            Ok(())
        }
    }
}

/// Set metadata for current session
#[derive(Debug)]
pub struct SetMetadata {
    /// Metadata key
    pub key: String,
    /// Metadata value
    pub value: String,
}

impl Message<SetMetadata> for HDF5Storage {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: SetMetadata,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let metadata = self.metadata.entry("session".to_string()).or_default();
        metadata.insert(msg.key.clone(), msg.value.clone());

        tracing::debug!("Set metadata: {} = {}", msg.key, msg.value);
        Ok(())
    }
}

/// Flush pending writes to disk
#[derive(Debug)]
pub struct Flush;

impl Message<Flush> for HDF5Storage {
    type Reply = Result<()>;

    async fn handle(&mut self, _msg: Flush, _ctx: &mut Context<Self, Self::Reply>) -> Self::Reply {
        #[cfg(feature = "storage_hdf5")]
        {
            self.flush().await?;
        }

        #[cfg(not(feature = "storage_hdf5"))]
        {
            tracing::warn!("Flush received but HDF5 feature not enabled");
        }

        Ok(())
    }
}

/// Get storage statistics
#[derive(Debug)]
pub struct GetStats;

impl Message<GetStats> for HDF5Storage {
    type Reply = StorageStats;

    async fn handle(
        &mut self,
        _msg: GetStats,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        StorageStats {
            bytes_written: self.bytes_written,
            batches_written: self.batches_written,
            file_path: self.current_file_path.clone(),
            file_size: std::fs::metadata(&self.current_file_path)
                .map(|m| m.len())
                .unwrap_or(0),
            num_datasets: self.metadata.len() as u64,
        }
    }
}

/// Set instrument-specific metadata
#[derive(Debug)]
pub struct SetInstrumentMetadata {
    /// Instrument ID
    pub instrument_id: String,
    /// Metadata key
    pub key: String,
    /// Metadata value
    pub value: String,
}

impl Message<SetInstrumentMetadata> for HDF5Storage {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: SetInstrumentMetadata,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let metadata = self.metadata.entry(msg.instrument_id.clone()).or_default();
        metadata.insert(msg.key.clone(), msg.value.clone());

        tracing::debug!(
            "Set metadata for {}: {} = {}",
            msg.instrument_id,
            msg.key,
            msg.value
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_creation() {
        let config = StorageConfig {
            default_backend: "hdf5".to_string(),
            output_dir: PathBuf::from("./test_data"),
            compression_level: 6,
            auto_flush_interval_secs: 0,
        };

        let storage = HDF5Storage::new(&config);
        assert_eq!(storage.compression_level, 6);
        assert_eq!(storage.batches_written, 0);
    }

    #[test]
    fn test_storage_stats() {
        let storage = HDF5Storage::default();
        assert_eq!(storage.batches_written, 0);
        assert_eq!(storage.bytes_written, 0);
    }

    #[test]
    fn test_metadata_storage() {
        let mut storage = HDF5Storage::default();
        let metadata = storage.metadata.entry("test".to_string()).or_default();
        metadata.insert("key1".to_string(), "value1".to_string());

        assert_eq!(metadata.get("key1"), Some(&"value1".to_string()));
    }
}
