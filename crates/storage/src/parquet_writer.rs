//! Parquet Writer -- Zero-copy Arrow-to-Parquet storage.
//!
//! Provides a general-purpose writer that persists Arrow [`RecordBatch`]es
//! directly to Parquet files. Unlike [`super::arrow_writer::ParquetDocumentWriter`],
//! which is specialized for RunEngine documents, this writer operates at the
//! `RecordBatch` level and is suitable for any Arrow-based data pipeline.
//!
//! # Features
//!
//! - Configurable compression (Snappy, Zstd, or uncompressed)
//! - Configurable row group size (default 64K rows)
//! - Append-style writes: multiple batches accumulate in a single Parquet file
//! - Zero-copy when the Arrow arrays are already in memory
//!
//! # Example
//!
//! ```ignore
//! use arrow::array::{Float64Array, Int64Array};
//! use arrow::datatypes::{DataType, Field, Schema};
//! use arrow::record_batch::RecordBatch;
//! use storage::parquet_writer::{ParquetWriter, ParquetWriterConfig, ParquetCompression};
//! use std::sync::Arc;
//!
//! let schema = Arc::new(Schema::new(vec![
//!     Field::new("x", DataType::Float64, false),
//!     Field::new("y", DataType::Int64, false),
//! ]));
//!
//! let batch = RecordBatch::try_new(schema.clone(), vec![
//!     Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
//!     Arc::new(Int64Array::from(vec![10, 20, 30])),
//! ]).unwrap();
//!
//! let config = ParquetWriterConfig::default()
//!     .with_compression(ParquetCompression::Zstd)
//!     .with_row_group_size(1024);
//!
//! let mut writer = ParquetWriter::new("data.parquet", config).unwrap();
//! writer.write_batch(&batch).unwrap();
//! writer.close().unwrap();
//! ```

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use tracing::debug;

// ── Configuration ────────────────────────────────────────────────────────────

/// Default number of rows per row group.
const DEFAULT_ROW_GROUP_SIZE: usize = 65_536;

/// Compression algorithm for Parquet output.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ParquetCompression {
    /// No compression -- fastest writes, largest files.
    None,
    /// Snappy compression -- fast with moderate compression ratio.
    #[default]
    Snappy,
    /// Zstandard compression -- slower writes but excellent compression ratio.
    Zstd,
}

impl ParquetCompression {
    /// Convert to the parquet crate's [`Compression`] enum.
    fn to_parquet_compression(self) -> Compression {
        match self {
            Self::None => Compression::UNCOMPRESSED,
            Self::Snappy => Compression::SNAPPY,
            Self::Zstd => Compression::ZSTD(Default::default()),
        }
    }
}

/// Configuration for [`ParquetWriter`].
#[derive(Debug, Clone)]
pub struct ParquetWriterConfig {
    /// Compression algorithm. Default: Snappy.
    pub compression: ParquetCompression,
    /// Maximum number of rows per row group. Default: 64K.
    pub row_group_size: usize,
}

impl Default for ParquetWriterConfig {
    fn default() -> Self {
        Self {
            compression: ParquetCompression::default(),
            row_group_size: DEFAULT_ROW_GROUP_SIZE,
        }
    }
}

impl ParquetWriterConfig {
    /// Set the compression algorithm.
    #[must_use]
    pub fn with_compression(mut self, compression: ParquetCompression) -> Self {
        self.compression = compression;
        self
    }

    /// Set the row group size.
    #[must_use]
    pub fn with_row_group_size(mut self, row_group_size: usize) -> Self {
        self.row_group_size = row_group_size;
        self
    }

    /// Build [`WriterProperties`] from this configuration.
    fn to_writer_properties(&self) -> WriterProperties {
        WriterProperties::builder()
            .set_compression(self.compression.to_parquet_compression())
            .set_max_row_group_size(self.row_group_size)
            .build()
    }
}

// ── Writer ───────────────────────────────────────────────────────────────────

/// General-purpose Parquet writer that persists Arrow `RecordBatch`es to a file.
///
/// The writer is created with a file path and configuration. The Parquet schema
/// is inferred from the first `RecordBatch` written, so the writer can be
/// constructed before the schema is known. All subsequent batches must share the
/// same schema.
///
/// Call [`close`](ParquetWriter::close) to flush remaining data and write the
/// Parquet footer. Dropping without closing will attempt to finalize the file
/// but errors are silently ignored.
pub struct ParquetWriter {
    /// Path to the output Parquet file.
    path: PathBuf,
    /// Writer configuration (used when lazily creating the inner writer).
    config: ParquetWriterConfig,
    /// The underlying Arrow-to-Parquet writer. `None` before the first batch
    /// is written and after `close()` is called.
    inner: Option<ArrowWriter<File>>,
    /// Number of batches written so far.
    batches_written: u64,
}

impl std::fmt::Debug for ParquetWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParquetWriter")
            .field("path", &self.path)
            .field("is_open", &self.inner.is_some())
            .field("batches_written", &self.batches_written)
            .finish()
    }
}

impl ParquetWriter {
    /// Create a new `ParquetWriter`.
    ///
    /// The file is created immediately so path errors surface early, but the
    /// Parquet schema is inferred from the first [`RecordBatch`] written.
    ///
    /// # Errors
    ///
    /// Returns an error if the output path cannot be created (missing parent
    /// directory, permission error, etc.).
    pub fn new(path: impl AsRef<Path>, config: ParquetWriterConfig) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        // Eagerly validate the path by creating the file. The file will be
        // recreated (truncated) when the first batch is written.
        File::create(&path)
            .with_context(|| format!("cannot create parquet file: {}", path.display()))?;

        debug!(path = %path.display(), "parquet writer created");

        Ok(Self {
            path,
            config,
            inner: None,
            batches_written: 0,
        })
    }

    /// Write a [`RecordBatch`] to the Parquet file.
    ///
    /// The first call infers the schema from the batch. All subsequent batches
    /// must use the same schema.
    ///
    /// # Errors
    ///
    /// Returns an error if the batch schema does not match the file schema, or
    /// if a write I/O error occurs.
    pub fn write_batch(&mut self, batch: &RecordBatch) -> Result<()> {
        let writer = match self.inner.as_mut() {
            Some(w) => w,
            None => {
                // Lazily initialize the writer with the schema from the first batch.
                let schema = batch.schema();
                let file = File::create(&self.path).with_context(|| {
                    format!("cannot create parquet file: {}", self.path.display())
                })?;
                let props = self.config.to_writer_properties();
                let w = ArrowWriter::try_new(file, schema, Some(props))
                    .context("failed to initialize parquet writer")?;
                self.inner.insert(w)
            }
        };

        writer
            .write(batch)
            .context("failed to write record batch to parquet")?;
        self.batches_written += 1;

        debug!(
            path = %self.path.display(),
            batches = self.batches_written,
            rows = batch.num_rows(),
            "wrote batch to parquet"
        );

        Ok(())
    }

    /// Flush remaining data, write the Parquet footer, and close the file.
    ///
    /// After calling `close`, the writer cannot be used again. Returns the
    /// total Parquet metadata (file size in bytes written).
    ///
    /// # Errors
    ///
    /// Returns an error if the writer has already been closed, or if flushing
    /// fails.
    pub fn close(&mut self) -> Result<()> {
        let writer = self
            .inner
            .take()
            .context("parquet writer is already closed")?;

        let metadata = writer.close().context("failed to close parquet writer")?;

        debug!(
            path = %self.path.display(),
            batches = self.batches_written,
            num_row_groups = metadata.num_row_groups(),
            "parquet file finalized"
        );

        Ok(())
    }

    /// Returns the file path this writer is writing to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the number of batches written so far.
    pub fn batches_written(&self) -> u64 {
        self.batches_written
    }

    /// Returns whether the writer is still open.
    pub fn is_open(&self) -> bool {
        self.inner.is_some()
    }
}

impl Drop for ParquetWriter {
    fn drop(&mut self) {
        if let Some(writer) = self.inner.take() {
            // Best-effort finalization on drop.
            let _ = writer.close();
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Float64Array, Int64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use std::sync::Arc;
    use tempfile::TempDir;

    /// Helper: build a simple schema with (x: f64, y: i64).
    fn test_schema() -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Int64, false),
        ]))
    }

    /// Helper: build a RecordBatch with the test schema.
    fn test_batch(xs: &[f64], ys: &[i64]) -> RecordBatch {
        let schema = test_schema();
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(xs.to_vec())),
                Arc::new(Int64Array::from(ys.to_vec())),
            ],
        )
        .expect("valid test batch")
    }

    #[test]
    fn write_and_read_back() {
        let dir = TempDir::new().expect("create temp dir");
        let path = dir.path().join("test.parquet");

        let batch = test_batch(&[1.0, 2.0, 3.0], &[10, 20, 30]);

        // Write
        let mut writer =
            ParquetWriter::new(&path, ParquetWriterConfig::default()).expect("create writer");
        writer.write_batch(&batch).expect("write batch");
        assert_eq!(writer.batches_written(), 1);
        writer.close().expect("close writer");
        assert!(!writer.is_open());

        // Read back
        let file = File::open(&path).expect("open parquet file");
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .expect("create reader")
            .build()
            .expect("build reader");

        let batches: Vec<RecordBatch> = reader.map(|r| r.expect("read batch")).collect();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 3);
        assert_eq!(batches[0].num_columns(), 2);

        // Verify values
        let xs = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("x column");
        assert!((xs.value(0) - 1.0).abs() < f64::EPSILON);
        assert!((xs.value(1) - 2.0).abs() < f64::EPSILON);
        assert!((xs.value(2) - 3.0).abs() < f64::EPSILON);

        let ys = batches[0]
            .column(1)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("y column");
        assert_eq!(ys.value(0), 10);
        assert_eq!(ys.value(1), 20);
        assert_eq!(ys.value(2), 30);
    }

    #[test]
    fn write_multiple_batches() {
        let dir = TempDir::new().expect("create temp dir");
        let path = dir.path().join("multi.parquet");

        let batch1 = test_batch(&[1.0, 2.0], &[10, 20]);
        let batch2 = test_batch(&[3.0, 4.0], &[30, 40]);
        let batch3 = test_batch(&[5.0], &[50]);

        let mut writer =
            ParquetWriter::new(&path, ParquetWriterConfig::default()).expect("create writer");
        writer.write_batch(&batch1).expect("write batch 1");
        writer.write_batch(&batch2).expect("write batch 2");
        writer.write_batch(&batch3).expect("write batch 3");
        assert_eq!(writer.batches_written(), 3);
        writer.close().expect("close writer");

        // Read back and verify total row count
        let file = File::open(&path).expect("open parquet file");
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .expect("create reader")
            .build()
            .expect("build reader");

        let total_rows: usize = reader.map(|r| r.expect("read batch").num_rows()).sum();
        assert_eq!(total_rows, 5);
    }

    #[test]
    fn compression_snappy() {
        let dir = TempDir::new().expect("create temp dir");
        let path = dir.path().join("snappy.parquet");

        let batch = test_batch(&[1.0, 2.0, 3.0], &[10, 20, 30]);
        let config = ParquetWriterConfig::default().with_compression(ParquetCompression::Snappy);

        let mut writer = ParquetWriter::new(&path, config).expect("create writer");
        writer.write_batch(&batch).expect("write batch");
        writer.close().expect("close writer");

        // Verify the file is valid and readable
        let file = File::open(&path).expect("open parquet file");
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .expect("create reader")
            .build()
            .expect("build reader");

        let batches: Vec<RecordBatch> = reader.map(|r| r.expect("read batch")).collect();
        assert_eq!(batches[0].num_rows(), 3);
    }

    #[test]
    fn compression_zstd() {
        let dir = TempDir::new().expect("create temp dir");
        let path = dir.path().join("zstd.parquet");

        let batch = test_batch(&[1.0, 2.0, 3.0], &[10, 20, 30]);
        let config = ParquetWriterConfig::default().with_compression(ParquetCompression::Zstd);

        let mut writer = ParquetWriter::new(&path, config).expect("create writer");
        writer.write_batch(&batch).expect("write batch");
        writer.close().expect("close writer");

        // Verify the file is valid and readable
        let file = File::open(&path).expect("open parquet file");
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .expect("create reader")
            .build()
            .expect("build reader");

        let batches: Vec<RecordBatch> = reader.map(|r| r.expect("read batch")).collect();
        assert_eq!(batches[0].num_rows(), 3);
    }

    #[test]
    fn compression_none() {
        let dir = TempDir::new().expect("create temp dir");
        let path = dir.path().join("uncompressed.parquet");

        let batch = test_batch(&[1.0, 2.0, 3.0], &[10, 20, 30]);
        let config = ParquetWriterConfig::default().with_compression(ParquetCompression::None);

        let mut writer = ParquetWriter::new(&path, config).expect("create writer");
        writer.write_batch(&batch).expect("write batch");
        writer.close().expect("close writer");

        let file = File::open(&path).expect("open parquet file");
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .expect("create reader")
            .build()
            .expect("build reader");

        let batches: Vec<RecordBatch> = reader.map(|r| r.expect("read batch")).collect();
        assert_eq!(batches[0].num_rows(), 3);
    }

    #[test]
    fn close_and_reopen() {
        let dir = TempDir::new().expect("create temp dir");
        let path = dir.path().join("reopen.parquet");

        // Write, close
        let batch = test_batch(&[1.0, 2.0], &[10, 20]);
        let mut writer =
            ParquetWriter::new(&path, ParquetWriterConfig::default()).expect("create writer");
        writer.write_batch(&batch).expect("write batch");
        writer.close().expect("close writer");

        // Reopen the file with a new writer (overwrites)
        let batch2 = test_batch(&[3.0, 4.0, 5.0], &[30, 40, 50]);
        let mut writer2 =
            ParquetWriter::new(&path, ParquetWriterConfig::default()).expect("create writer 2");
        writer2.write_batch(&batch2).expect("write batch 2");
        writer2.close().expect("close writer 2");

        // Read back -- should only see the second write (file was overwritten)
        let file = File::open(&path).expect("open parquet file");
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .expect("create reader")
            .build()
            .expect("build reader");

        let batches: Vec<RecordBatch> = reader.map(|r| r.expect("read batch")).collect();
        let total_rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
        assert_eq!(total_rows, 3);
    }

    #[test]
    fn custom_row_group_size() {
        let dir = TempDir::new().expect("create temp dir");
        let path = dir.path().join("rowgroup.parquet");

        // Use a small row group size to force multiple row groups
        let config = ParquetWriterConfig::default().with_row_group_size(2);

        let batch = test_batch(&[1.0, 2.0, 3.0, 4.0, 5.0], &[10, 20, 30, 40, 50]);

        let mut writer = ParquetWriter::new(&path, config).expect("create writer");
        writer.write_batch(&batch).expect("write batch");
        writer.close().expect("close writer");

        // Verify all data is readable
        let file = File::open(&path).expect("open parquet file");
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .expect("create reader")
            .build()
            .expect("build reader");

        let total_rows: usize = reader.map(|r| r.expect("read batch").num_rows()).sum();
        assert_eq!(total_rows, 5);
    }

    #[test]
    fn close_twice_errors() {
        let dir = TempDir::new().expect("create temp dir");
        let path = dir.path().join("close_twice.parquet");

        let batch = test_batch(&[1.0], &[10]);
        let mut writer =
            ParquetWriter::new(&path, ParquetWriterConfig::default()).expect("create writer");
        writer.write_batch(&batch).expect("write batch");
        writer.close().expect("first close");

        // Second close should error
        let result = writer.close();
        assert!(result.is_err());
    }

    #[test]
    fn string_columns() {
        let dir = TempDir::new().expect("create temp dir");
        let path = dir.path().join("strings.parquet");

        let schema = Arc::new(Schema::new(vec![
            Field::new("name", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["alpha", "beta", "gamma"])),
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
            ],
        )
        .expect("valid batch");

        let mut writer =
            ParquetWriter::new(&path, ParquetWriterConfig::default()).expect("create writer");
        writer.write_batch(&batch).expect("write batch");
        writer.close().expect("close writer");

        let file = File::open(&path).expect("open parquet file");
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .expect("create reader")
            .build()
            .expect("build reader");

        let batches: Vec<RecordBatch> = reader.map(|r| r.expect("read batch")).collect();
        assert_eq!(batches[0].num_rows(), 3);

        let names = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("name column");
        assert_eq!(names.value(0), "alpha");
        assert_eq!(names.value(1), "beta");
        assert_eq!(names.value(2), "gamma");
    }

    #[test]
    fn default_config_values() {
        let config = ParquetWriterConfig::default();
        assert_eq!(config.compression, ParquetCompression::Snappy);
        assert_eq!(config.row_group_size, 65_536);
    }
}
