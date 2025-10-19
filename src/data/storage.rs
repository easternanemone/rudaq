//! Example data storage writers.
use crate::{
    config::Settings,
    core::{DataPoint, StorageWriter},
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
            // TODO: Implement metadata writing for HDF5
            Ok(())
        }
        #[cfg(not(feature = "storage_hdf5"))]
        Err(DaqError::FeatureNotEnabled("storage_hdf5".to_string()).into())
    }
    async fn write(&mut self, _data: &[Arc<Measurement>]) -> Result<()> {
        #[cfg(feature = "storage_hdf5")]
        {
            // TODO: Implement data writing for HDF5
            // HDF5 can handle Scalar, Spectrum, and Image data natively
            // Pattern match on Measurement variants and write to appropriate datasets
            Ok(())
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
            // TODO: Implement Arrow initialization
            Ok(())
        }
        #[cfg(not(feature = "storage_arrow"))]
        Err(DaqError::FeatureNotEnabled("storage_arrow".to_string()).into())
    }
    async fn set_metadata(&mut self, _metadata: &Metadata) -> Result<()> {
        #[cfg(feature = "storage_arrow")]
        {
            // TODO: Implement metadata writing for Arrow
            Ok(())
        }
        #[cfg(not(feature = "storage_arrow"))]
        Err(DaqError::FeatureNotEnabled("storage_arrow".to_string()).into())
    }
    async fn write(&mut self, _data: &[Arc<Measurement>]) -> Result<()> {
        #[cfg(feature = "storage_arrow")]
        {
            // TODO: Implement data writing for Arrow
            // Arrow can handle Scalar, Spectrum, and Image data with columnar storage
            // Pattern match on Measurement variants and write to appropriate record batches
            Ok(())
        }
        #[cfg(not(feature = "storage_arrow"))]
        Err(DaqError::FeatureNotEnabled("storage_arrow".to_string()).into())
    }
    async fn shutdown(&mut self) -> Result<()> {
        #[cfg(feature = "storage_arrow")]
        {
            // TODO: Implement Arrow shutdown
            Ok(())
        }
        #[cfg(not(feature = "storage_arrow"))]
        Err(DaqError::FeatureNotEnabled("storage_arrow".to_string()).into())
    }
}
