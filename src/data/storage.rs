//! Data storage writers with clean feature flag handling.
use crate::{
    config::Settings,
    core::{DataPoint, StorageWriter},
    error::DaqError,
    metadata::Metadata,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::sync::Arc;

// ============================================================================
// CSV Writer
// ============================================================================

#[cfg(feature = "storage_csv")]
mod csv_enabled {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;

    pub struct CsvWriter {
        path: PathBuf,
        writer: Option<csv::Writer<File>>,
    }

    impl Default for CsvWriter {
        fn default() -> Self {
            Self::new()
        }
    }

    impl CsvWriter {
        pub fn new() -> Self {
            Self {
                path: PathBuf::new(),
                writer: None,
            }
        }
    }

    #[async_trait]
    impl StorageWriter for CsvWriter {
        async fn init(&mut self, settings: &Arc<Settings>) -> Result<()> {
            let file_name = format!(
                "session_{}.csv",
                chrono::Utc::now().format("%Y%m%d_%H%M%S")
            );
            let path = PathBuf::from(&settings.storage.default_path);
            if !path.exists() {
                std::fs::create_dir_all(&path)
                    .with_context(|| format!("Failed to create storage directory at {:?}", path))?;
            }
            self.path = path.join(file_name);
            log::info!("CSV Writer initialized at '{}'.", self.path.display());
            Ok(())
        }

        async fn set_metadata(&mut self, metadata: &Metadata) -> Result<()> {
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
                .write_record(["timestamp", "channel", "value", "unit", "metadata"])
                .context("Failed to write CSV header")?;

            self.writer = Some(writer);
            Ok(())
        }

        async fn write(&mut self, data: &[DataPoint]) -> Result<()> {
            if let Some(writer) = self.writer.as_mut() {
                for dp in data {
                    let metadata_str = dp
                        .metadata
                        .as_ref()
                        .map_or(String::new(), |v| v.to_string());
                    writer
                        .write_record(&[
                            dp.timestamp.to_rfc3339(),
                            dp.channel.clone(),
                            dp.value.to_string(),
                            dp.unit.clone(),
                            metadata_str,
                        ])
                        .context("Failed to write data point to CSV file")?;
                }
            }
            Ok(())
        }

        async fn shutdown(&mut self) -> Result<()> {
            if let Some(mut writer) = self.writer.take() {
                writer.flush().context("Failed to flush CSV writer")?;
            }
            log::info!("CSV Writer shut down.");
            Ok(())
        }
    }
}

#[cfg(not(feature = "storage_csv"))]
mod csv_disabled {
    use super::*;

    pub struct CsvWriter;

    impl CsvWriter {
        pub fn new() -> Self {
            Self
        }
    }

    impl Default for CsvWriter {
        fn default() -> Self {
            Self::new()
        }
    }

    #[async_trait]
    impl StorageWriter for CsvWriter {
        async fn init(&mut self, _settings: &Arc<Settings>) -> Result<()> {
            Err(DaqError::FeatureNotEnabled("storage_csv".to_string()).into())
        }

        async fn set_metadata(&mut self, _metadata: &Metadata) -> Result<()> {
            Err(DaqError::FeatureNotEnabled("storage_csv".to_string()).into())
        }

        async fn write(&mut self, _data: &[DataPoint]) -> Result<()> {
            Err(DaqError::FeatureNotEnabled("storage_csv".to_string()).into())
        }

        async fn shutdown(&mut self) -> Result<()> {
            Err(DaqError::FeatureNotEnabled("storage_csv".to_string()).into())
        }
    }
}

#[cfg(feature = "storage_csv")]
pub use csv_enabled::CsvWriter;

#[cfg(not(feature = "storage_csv"))]
pub use csv_disabled::CsvWriter;

// ============================================================================
// HDF5 Writer
// ============================================================================

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

#[cfg(not(feature = "storage_hdf5"))]
#[async_trait]
impl StorageWriter for Hdf5Writer {
    async fn init(&mut self, _settings: &Arc<Settings>) -> Result<()> {
        Err(DaqError::FeatureNotEnabled("storage_hdf5".to_string()).into())
    }

    async fn set_metadata(&mut self, _metadata: &Metadata) -> Result<()> {
        Err(DaqError::FeatureNotEnabled("storage_hdf5".to_string()).into())
    }

    async fn write(&mut self, _data: &[DataPoint]) -> Result<()> {
        Err(DaqError::FeatureNotEnabled("storage_hdf5".to_string()).into())
    }

    async fn shutdown(&mut self) -> Result<()> {
        Err(DaqError::FeatureNotEnabled("storage_hdf5".to_string()).into())
    }
}

// ============================================================================
// Arrow Writer
// ============================================================================

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

#[cfg(not(feature = "storage_arrow"))]
#[async_trait]
impl StorageWriter for ArrowWriter {
    async fn init(&mut self, _settings: &Arc<Settings>) -> Result<()> {
        Err(DaqError::FeatureNotEnabled("storage_arrow".to_string()).into())
    }

    async fn set_metadata(&mut self, _metadata: &Metadata) -> Result<()> {
        Err(DaqError::FeatureNotEnabled("storage_arrow".to_string()).into())
    }

    async fn write(&mut self, _data: &[DataPoint]) -> Result<()> {
        Err(DaqError::FeatureNotEnabled("storage_arrow".to_string()).into())
    }

    async fn shutdown(&mut self) -> Result<()> {
        Err(DaqError::FeatureNotEnabled("storage_arrow".to_string()).into())
    }
}
