//! Example data storage writers.
use crate::{
    config::Settings,
    core::StorageWriter,
    error::DaqError,
    metadata::Metadata,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use daq_core::Measurement;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

/// A writer for CSV files.
#[cfg(feature = "storage_csv")]
pub struct CsvWriter {
    path: PathBuf,
    writer: Option<csv::Writer<File>>,
}

#[cfg(feature = "storage_csv")]
impl Default for CsvWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "storage_csv")]
impl CsvWriter {
    pub fn new() -> Self {
        Self {
            path: PathBuf::new(),
            writer: None,
        }
    }
}

#[cfg(not(feature = "storage_csv"))]
pub struct CsvWriter;

#[cfg(not(feature = "storage_csv"))]
impl CsvWriter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl StorageWriter for CsvWriter {
    async fn init(&mut self, settings: &Arc<Settings>) -> Result<()> {
        #[cfg(not(feature = "storage_csv"))]
        return Err(DaqError::FeatureNotEnabled("storage_csv".to_string()).into());

        #[cfg(feature = "storage_csv")]
        {
            let file_name = format!(
                "{}_{}.csv",
                "session",
                chrono::Utc::now().format("%Y%m%d_%H%M%S")
            );
            let path = PathBuf::from(&settings.storage.default_path);
            if !path.exists() {
                std::fs::create_dir_all(&path)
                    .with_context(|| format!("Failed to create storage directory at {:?}", path))?;
            }
            self.path = path.join(file_name);
            log::info!(
                "CSV Writer will be initialized at '{}'.",
                self.path.display()
            );
            Ok(())
        }
    }

    async fn set_metadata(&mut self, metadata: &Metadata) -> Result<()> {
        #[cfg(feature = "storage_csv")]
        {
            let mut file = File::create(&self.path)
                .with_context(|| format!("Failed to create CSV file at {:?}", self.path))?;

            let json_string = serde_json::to_string_pretty(metadata)
                .context("Failed to serialize metadata to JSON")?;

            for line in json_string.lines() {
                file.write_all(b"# ")
                    .and_then(|_| file.write_all(line.as_bytes()))
                    .and_then(|_| file.write_all(b"\n"))
                    .context("Failed to write metadata to CSV file")?;
            }

            let mut writer = csv::Writer::from_writer(file);
            writer
                .write_record(["timestamp", "channel", "value", "unit"])
                .context("Failed to write CSV header")?;

            self.writer = Some(writer);
            Ok(())
        }
        #[cfg(not(feature = "storage_csv"))]
        Ok(())
    }

    async fn write(&mut self, data: &[Arc<Measurement>]) -> Result<()> {
        #[cfg(feature = "storage_csv")]
        {
            if let Some(writer) = self.writer.as_mut() {
                for measurement in data {
                    // CSV writer only handles Scalar measurements
                    // Spectrum and Image data require HDF5/Arrow format
                    if let Measurement::Scalar(dp) = measurement.as_ref() {
                        writer
                            .write_record(&[
                                dp.timestamp.to_rfc3339(),
                                dp.channel.clone(),
                                dp.value.to_string(),
                                dp.unit.clone(),
                            ])
                            .context("Failed to write data point to CSV file")?;
                    } else {
                        // Log non-scalar measurements being skipped
                        log::trace!("CSV writer skipping non-scalar measurement (use HDF5/Arrow for Spectrum/Image data)");
                    }
                }
            }
            Ok(())
        }
        #[cfg(not(feature = "storage_csv"))]
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        #[cfg(feature = "storage_csv")]
        {
            if let Some(mut writer) = self.writer.take() {
                writer.flush().context("Failed to flush CSV writer")?;
            }
            log::info!("CSV Writer shut down.");
            Ok(())
        }
        #[cfg(not(feature = "storage_csv"))]
        Ok(())
    }
}

// Skeletons for other writers
#[cfg(feature = "storage_hdf5")]
pub struct Hdf5Writer {
    file: Option<hdf5::File>,
}

#[cfg(feature = "storage_hdf5")]
impl Default for Hdf5Writer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "storage_hdf5")]
impl Hdf5Writer {
    pub fn new() -> Self {
        Self { file: None }
    }
}

#[cfg(not(feature = "storage_hdf5"))]
pub struct Hdf5Writer;

#[cfg(not(feature = "storage_hdf5"))]
impl Default for Hdf5Writer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(feature = "storage_hdf5"))]
impl Hdf5Writer {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl StorageWriter for Hdf5Writer {
    async fn init(&mut self, settings: &Arc<Settings>) -> Result<()> {
        #[cfg(feature = "storage_hdf5")]
        {
            let file_name = format!(
                "{}_{}.h5",
                "session",
                chrono::Utc::now().format("%Y%m%d_%H%M%S")
            );
            let path = PathBuf::from(&settings.storage.default_path);
            if !path.exists() {
                std::fs::create_dir_all(&path)
                    .with_context(|| format!("Failed to create storage directory at {:?}", path))?;
            }
            let file_path = path.join(file_name);
            self.file = Some(hdf5::File::create(file_path)?);
            Ok(())
        }
        #[cfg(not(feature = "storage_hdf5"))]
        Err(DaqError::FeatureNotEnabled("storage_hdf5".to_string()).into())
    }
    async fn set_metadata(&mut self, _metadata: &Metadata) -> Result<()> {
        #[cfg(feature = "storage_hdf5")]
        {
            // HDF5 metadata writing is not yet implemented
            // To fully implement: Write metadata as HDF5 attributes on the root group
            // using self.file.as_ref().unwrap() and file.new_attr::<String>() methods
            Err(DaqError::FeatureIncomplete(
                "storage_hdf5".to_string(),
                "HDF5 metadata writing is not implemented. Use CSV storage for now.".to_string(),
            )
            .into())
        }
        #[cfg(not(feature = "storage_hdf5"))]
        Err(DaqError::FeatureNotEnabled("storage_hdf5".to_string()).into())
    }
    async fn write(&mut self, _data: &[Arc<Measurement>]) -> Result<()> {
        #[cfg(feature = "storage_hdf5")]
        {
            // HDF5 data writing is not yet implemented
            // To fully implement:
            // 1. Create datasets for each Measurement variant (scalars, spectra, images)
            // 2. Pattern match on measurement type and write to appropriate dataset
            // 3. Handle dynamic dataset resizing for streaming data
            // 4. Use chunked storage for efficient appends
            Err(DaqError::FeatureIncomplete(
                "storage_hdf5".to_string(),
                "HDF5 data writing is not implemented. Use CSV storage for scalar data."
                    .to_string(),
            )
            .into())
        }
        #[cfg(not(feature = "storage_hdf5"))]
        Err(DaqError::FeatureNotEnabled("storage_hdf5".to_string()).into())
    }
    async fn shutdown(&mut self) -> Result<()> {
        #[cfg(feature = "storage_hdf5")]
        {
            if let Some(file) = self.file.take() {
                file.close()?;
            }
            Ok(())
        }
        #[cfg(not(feature = "storage_hdf5"))]
        Err(DaqError::FeatureNotEnabled("storage_hdf5".to_string()).into())
    }
}

#[cfg(feature = "storage_arrow")]
pub struct ArrowWriter;

#[cfg(feature = "storage_arrow")]
impl Default for ArrowWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "storage_arrow")]
impl ArrowWriter {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(not(feature = "storage_arrow"))]
pub struct ArrowWriter;

#[cfg(not(feature = "storage_arrow"))]
impl Default for ArrowWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(feature = "storage_arrow"))]
impl ArrowWriter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl StorageWriter for ArrowWriter {
    async fn init(&mut self, _settings: &Arc<Settings>) -> Result<()> {
        #[cfg(feature = "storage_arrow")]
        {
            // Arrow initialization is not yet implemented
            // To fully implement:
            // 1. Create Arrow schema matching Measurement enum variants
            // 2. Initialize IPC file writer with appropriate path
            // 3. Set up record batch builders for streaming writes
            Err(DaqError::FeatureIncomplete(
                "storage_arrow".to_string(),
                "Arrow storage is not implemented. Use CSV storage for scalar data.".to_string(),
            )
            .into())
        }
        #[cfg(not(feature = "storage_arrow"))]
        Err(DaqError::FeatureNotEnabled("storage_arrow".to_string()).into())
    }
    async fn set_metadata(&mut self, _metadata: &Metadata) -> Result<()> {
        #[cfg(feature = "storage_arrow")]
        {
            // Arrow metadata writing is not yet implemented
            // To fully implement: Add metadata as Arrow schema custom metadata
            // using Schema::with_metadata() before creating the IPC writer
            Err(DaqError::FeatureIncomplete(
                "storage_arrow".to_string(),
                "Arrow metadata writing is not implemented. Use CSV storage for now.".to_string(),
            )
            .into())
        }
        #[cfg(not(feature = "storage_arrow"))]
        Err(DaqError::FeatureNotEnabled("storage_arrow".to_string()).into())
    }
    async fn write(&mut self, _data: &[Arc<Measurement>]) -> Result<()> {
        #[cfg(feature = "storage_arrow")]
        {
            // Arrow data writing is not yet implemented
            // To fully implement:
            // 1. Pattern match on Measurement variants (Scalar, Spectrum, Image)
            // 2. Build appropriate record batches for each type
            // 3. Write batches to IPC stream/file
            // 4. Handle efficient columnar storage for high-throughput data
            Err(DaqError::FeatureIncomplete(
                "storage_arrow".to_string(),
                "Arrow data writing is not implemented. Use CSV storage for scalar data."
                    .to_string(),
            )
            .into())
        }
        #[cfg(not(feature = "storage_arrow"))]
        Err(DaqError::FeatureNotEnabled("storage_arrow".to_string()).into())
    }
    async fn shutdown(&mut self) -> Result<()> {
        #[cfg(feature = "storage_arrow")]
        {
            // Arrow shutdown is not yet implemented
            // To fully implement: Flush and close the IPC writer
            Err(DaqError::FeatureIncomplete(
                "storage_arrow".to_string(),
                "Arrow shutdown is not implemented. Use CSV storage for now.".to_string(),
            )
            .into())
        }
        #[cfg(not(feature = "storage_arrow"))]
        Err(DaqError::FeatureNotEnabled("storage_arrow".to_string()).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use daq_core::{DataPoint, Measurement};

    #[tokio::test]
    async fn test_hdf5_writer_returns_proper_errors() {
        let mut writer = Hdf5Writer::new();
        let metadata = Metadata::default();

        // When storage_hdf5 feature is enabled, set_metadata should return FeatureIncomplete error
        let result = writer.set_metadata(&metadata).await;

        #[cfg(feature = "storage_hdf5")]
        {
            assert!(result.is_err());
            let err_msg = result.unwrap_err().to_string();
            assert!(err_msg.contains("not yet implemented") || err_msg.contains("not implemented"));
        }

        #[cfg(not(feature = "storage_hdf5"))]
        {
            assert!(result.is_err());
            let err_msg = result.unwrap_err().to_string();
            assert!(err_msg.contains("not enabled"));
        }
    }

    #[tokio::test]
    async fn test_arrow_writer_returns_proper_errors() {
        let mut writer = ArrowWriter::new();

        // Create a minimal Settings object for testing
        use crate::config::{ApplicationSettings, StorageSettings};
        let settings = Arc::new(crate::config::Settings {
            log_level: "info".to_string(),
            application: ApplicationSettings {
                broadcast_channel_capacity: 1024,
                command_channel_capacity: 32,
                data_distributor: Default::default(),
            },
            storage: StorageSettings {
                default_path: "/tmp".to_string(),
                default_format: "csv".to_string(),
            },
            instruments: std::collections::HashMap::new(),
            processors: None,
            instruments_v3: Vec::new(),
        });

        // When storage_arrow feature is enabled, init should return FeatureIncomplete error
        let result = writer.init(&settings).await;

        #[cfg(feature = "storage_arrow")]
        {
            assert!(result.is_err());
            let err_msg = result.unwrap_err().to_string();
            assert!(err_msg.contains("not yet implemented") || err_msg.contains("not implemented"));
        }

        #[cfg(not(feature = "storage_arrow"))]
        {
            assert!(result.is_err());
            let err_msg = result.unwrap_err().to_string();
            assert!(err_msg.contains("not enabled"));
        }
    }

    #[tokio::test]
    async fn test_no_silent_failures() {
        // Test that HDF5 write fails with error, not Ok(())
        let mut hdf5_writer = Hdf5Writer::new();
        let test_data = vec![Arc::new(Measurement::Scalar(DataPoint {
            timestamp: Utc::now(),
            channel: "test".to_string(),
            value: 1.0,
            unit: "V".to_string(),
        }))];

        let result = hdf5_writer.write(&test_data).await;
        assert!(result.is_err(), "HDF5 write should fail, not return Ok(())");

        // Test that Arrow write fails with error, not Ok(())
        let mut arrow_writer = ArrowWriter::new();
        let result = arrow_writer.write(&test_data).await;
        assert!(
            result.is_err(),
            "Arrow write should fail, not return Ok(())"
        );
    }
}
