//! Arrow/Parquet Document Writers - Persist RunEngine Documents to Arrow/Parquet
//!
//! This module provides writers that consume `RunEngine` documents
//! (Start, Descriptor, Event, Stop) and write them to Arrow IPC or Parquet files.
//!
//! # Architecture
//!
//! - **Start**: Creates a new file and initializes schema
//! - **Descriptor**: Defines the schema for data columns
//! - **Event**: Buffers data into Arrow RecordBatches
//! - **Stop**: Flushes remaining data and finalizes the file
//!
//! # Format Comparison
//!
//! | Format | Use Case | Pros | Cons |
//! |--------|----------|------|------|
//! | Arrow IPC | Streaming, IPC | Fast read/write, streaming | Larger file size |
//! | Parquet | Analytics, Archive | Columnar compression, analytics | Write-once |
//!
//! # Feature Flags
//!
//! - `storage_arrow`: Enables Arrow IPC writer
//! - `storage_parquet`: Enables Parquet writer (includes Arrow)

#[cfg(feature = "storage_arrow")]
use std::collections::HashMap;
#[cfg(feature = "storage_arrow")]
use std::path::PathBuf;
#[cfg(feature = "storage_arrow")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "storage_arrow")]
use anyhow::Result;
#[cfg(feature = "storage_arrow")]
use arrow::array::{ArrayRef, Float64Builder, StringBuilder, UInt64Builder};
#[cfg(feature = "storage_arrow")]
use arrow::datatypes::{DataType, Field, Schema};
#[cfg(feature = "storage_arrow")]
use arrow::record_batch::RecordBatch;

/// Internal state for an active run
#[cfg(feature = "storage_arrow")]
struct ActiveArrowRun {
    run_uid: String,
    file_path: PathBuf,
    /// Schema derived from descriptor
    #[allow(dead_code)]
    schema: Option<Arc<Schema>>,
    /// Buffered events (converted to columns on flush)
    event_buffer: Vec<BufferedEvent>,
    /// Data key info from descriptor
    data_keys: HashMap<String, DataKeyInfo>,
    /// Metadata from start document
    metadata: HashMap<String, String>,
}

#[cfg(feature = "storage_arrow")]
#[derive(Clone)]
struct DataKeyInfo {
    dtype: String,
    #[allow(dead_code)]
    source: String,
}

#[cfg(feature = "storage_arrow")]
struct BufferedEvent {
    seq_num: u64,
    time_ns: u64,
    data: HashMap<String, f64>,
}

/// Arrow IPC Writer for RunEngine Documents
///
/// Writes documents to Arrow IPC format, suitable for streaming and
/// inter-process communication.
///
/// # Example
///
/// ```ignore
/// use daq_storage::arrow_writer::ArrowDocumentWriter;
/// use std::path::PathBuf;
///
/// let writer = ArrowDocumentWriter::new(PathBuf::from("/data/runs"));
/// writer.write(Document::Start(start_doc)).await?;
/// // ... write descriptor, events, stop
/// ```
#[cfg(feature = "storage_arrow")]
pub struct ArrowDocumentWriter {
    base_path: PathBuf,
    active_run: Arc<Mutex<Option<ActiveArrowRun>>>,
    /// Flush threshold (number of events before auto-flush)
    flush_threshold: usize,
}

#[cfg(feature = "storage_arrow")]
impl ArrowDocumentWriter {
    /// Create a new ArrowDocumentWriter
    pub fn new(base_path: PathBuf) -> Self {
        Self {
            base_path,
            active_run: Arc::new(Mutex::new(None)),
            flush_threshold: 1000,
        }
    }

    /// Set the flush threshold (number of events before auto-flush)
    pub fn with_flush_threshold(mut self, threshold: usize) -> Self {
        self.flush_threshold = threshold;
        self
    }

    /// Write a document to Arrow IPC storage
    pub async fn write(&self, doc: Document) -> Result<()> {
        let active_run = self.active_run.clone();
        let base_path = self.base_path.clone();
        let flush_threshold = self.flush_threshold;

        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut guard = active_run.lock().map_err(|_| anyhow!("Mutex poisoned"))?;

            match doc {
                Document::Start(start) => {
                    let filename = format!("{}_{}.arrow", start.uid, start.time_ns);
                    let file_path = base_path.join(filename);

                    let mut metadata = start.metadata.clone();
                    metadata.insert("plan_type".to_string(), start.plan_type.clone());
                    metadata.insert("plan_name".to_string(), start.plan_name.clone());
                    metadata.insert("run_uid".to_string(), start.uid.clone());

                    *guard = Some(ActiveArrowRun {
                        run_uid: start.uid,
                        file_path,
                        schema: None,
                        event_buffer: Vec::new(),
                        data_keys: HashMap::new(),
                        metadata,
                    });
                }
                Document::Descriptor(desc) => {
                    if let Some(run) = guard.as_mut() {
                        if run.run_uid != desc.run_uid {
                            return Err(anyhow!("Descriptor run_uid mismatch"));
                        }

                        // Build schema from data keys
                        let mut fields = vec![
                            Field::new("seq_num", DataType::UInt64, false),
                            Field::new("time_ns", DataType::UInt64, false),
                        ];

                        for (key, meta) in &desc.data_keys {
                            let dtype = match meta.dtype.as_str() {
                                "number" | "float64" | "f64" => DataType::Float64,
                                "string" => DataType::Utf8,
                                // For now, store other types as strings (serialized)
                                _ => DataType::Float64,
                            };
                            fields.push(Field::new(key, dtype, true));

                            run.data_keys.insert(
                                key.clone(),
                                DataKeyInfo {
                                    dtype: meta.dtype.clone(),
                                    source: meta.source.clone(),
                                },
                            );
                        }

                        run.schema = Some(Arc::new(Schema::new(fields)));
                    }
                }
                Document::Event(event) => {
                    if let Some(run) = guard.as_mut() {
                        run.event_buffer.push(BufferedEvent {
                            seq_num: event.seq_num as u64,
                            time_ns: event.time_ns,
                            data: event.data.clone(),
                        });

                        // Auto-flush if threshold reached
                        if run.event_buffer.len() >= flush_threshold {
                            flush_arrow_buffer(run)?;
                        }
                    }
                }
                Document::Stop(stop) => {
                    if let Some(run) = guard.as_mut() {
                        if run.run_uid == stop.run_uid {
                            // Final flush
                            flush_arrow_buffer(run)?;

                            // Clear active run
                            *guard = None;
                        }
                    }
                }
                Document::Manifest(_) => {
                    // Manifests are not written to data files
                }
            }
            Ok(())
        })
        .await??;

        Ok(())
    }
}

/// Flush buffered events to Arrow IPC file
#[cfg(feature = "storage_arrow")]
fn flush_arrow_buffer(run: &mut ActiveArrowRun) -> Result<()> {
    use arrow::ipc::writer::FileWriter;
    use std::fs::OpenOptions;

    if run.event_buffer.is_empty() {
        return Ok(());
    }

    let schema = run
        .schema
        .as_ref()
        .ok_or_else(|| anyhow!("No schema defined"))?;

    // Build arrays from buffered events
    let mut seq_num_builder = UInt64Builder::new();
    let mut time_ns_builder = UInt64Builder::new();

    // Build builders for each data key
    let mut data_builders: HashMap<String, Float64Builder> = HashMap::new();
    for key in run.data_keys.keys() {
        data_builders.insert(key.clone(), Float64Builder::new());
    }

    for event in &run.event_buffer {
        seq_num_builder.append_value(event.seq_num);
        time_ns_builder.append_value(event.time_ns);

        for (key, builder) in &mut data_builders {
            if let Some(value) = event.data.get(key) {
                builder.append_value(*value);
            } else {
                builder.append_null();
            }
        }
    }

    // Build arrays
    let mut columns: Vec<ArrayRef> = vec![
        Arc::new(seq_num_builder.finish()),
        Arc::new(time_ns_builder.finish()),
    ];

    // Add data columns in schema order
    for field in schema.fields().iter().skip(2) {
        if let Some(builder) = data_builders.get_mut(field.name()) {
            columns.push(Arc::new(builder.finish()));
        }
    }

    let batch = RecordBatch::try_new(schema.clone(), columns)?;

    // Write to file (append if exists)
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(&run.file_path)?;

    let mut writer = FileWriter::try_new(file, schema)?;
    writer.write(&batch)?;
    writer.finish()?;

    // Clear buffer
    run.event_buffer.clear();

    Ok(())
}

/// Parquet Writer for RunEngine Documents
///
/// Writes documents to Parquet format, suitable for analytics and
/// long-term archival storage.
///
/// # Example
///
/// ```ignore
/// use daq_storage::arrow_writer::ParquetDocumentWriter;
/// use std::path::PathBuf;
///
/// let writer = ParquetDocumentWriter::new(PathBuf::from("/data/runs"));
/// writer.write(Document::Start(start_doc)).await?;
/// // ... write descriptor, events, stop
/// ```
#[cfg(feature = "storage_parquet")]
pub struct ParquetDocumentWriter {
    base_path: PathBuf,
    active_run: Arc<Mutex<Option<ActiveArrowRun>>>,
    /// Flush threshold (number of events before auto-flush)
    flush_threshold: usize,
}

#[cfg(feature = "storage_parquet")]
impl ParquetDocumentWriter {
    /// Create a new ParquetDocumentWriter
    pub fn new(base_path: PathBuf) -> Self {
        Self {
            base_path,
            active_run: Arc::new(Mutex::new(None)),
            flush_threshold: 10000, // Larger batches for Parquet efficiency
        }
    }

    /// Set the flush threshold
    pub fn with_flush_threshold(mut self, threshold: usize) -> Self {
        self.flush_threshold = threshold;
        self
    }

    /// Write a document to Parquet storage
    pub async fn write(&self, doc: Document) -> Result<()> {
        let active_run = self.active_run.clone();
        let base_path = self.base_path.clone();
        let flush_threshold = self.flush_threshold;

        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut guard = active_run.lock().map_err(|_| anyhow!("Mutex poisoned"))?;

            match doc {
                Document::Start(start) => {
                    let filename = format!("{}_{}.parquet", start.uid, start.time_ns);
                    let file_path = base_path.join(filename);

                    let mut metadata = start.metadata.clone();
                    metadata.insert("plan_type".to_string(), start.plan_type.clone());
                    metadata.insert("plan_name".to_string(), start.plan_name.clone());
                    metadata.insert("run_uid".to_string(), start.uid.clone());

                    *guard = Some(ActiveArrowRun {
                        run_uid: start.uid,
                        file_path,
                        schema: None,
                        event_buffer: Vec::new(),
                        data_keys: HashMap::new(),
                        metadata,
                    });
                }
                Document::Descriptor(desc) => {
                    if let Some(run) = guard.as_mut() {
                        if run.run_uid != desc.run_uid {
                            return Err(anyhow!("Descriptor run_uid mismatch"));
                        }

                        // Build schema from data keys (same as Arrow)
                        let mut fields = vec![
                            Field::new("seq_num", DataType::UInt64, false),
                            Field::new("time_ns", DataType::UInt64, false),
                        ];

                        for (key, meta) in &desc.data_keys {
                            let dtype = match meta.dtype.as_str() {
                                "number" | "float64" | "f64" => DataType::Float64,
                                "string" => DataType::Utf8,
                                _ => DataType::Float64,
                            };
                            fields.push(Field::new(key, dtype, true));

                            run.data_keys.insert(
                                key.clone(),
                                DataKeyInfo {
                                    dtype: meta.dtype.clone(),
                                    source: meta.source.clone(),
                                },
                            );
                        }

                        run.schema = Some(Arc::new(Schema::new(fields)));
                    }
                }
                Document::Event(event) => {
                    if let Some(run) = guard.as_mut() {
                        run.event_buffer.push(BufferedEvent {
                            seq_num: event.seq_num as u64,
                            time_ns: event.time_ns,
                            data: event.data.clone(),
                        });

                        // Auto-flush if threshold reached
                        if run.event_buffer.len() >= flush_threshold {
                            flush_parquet_buffer(run)?;
                        }
                    }
                }
                Document::Stop(stop) => {
                    if let Some(run) = guard.as_mut() {
                        if run.run_uid == stop.run_uid {
                            // Final flush
                            flush_parquet_buffer(run)?;
                            *guard = None;
                        }
                    }
                }
                Document::Manifest(_) => {}
            }
            Ok(())
        })
        .await??;

        Ok(())
    }
}

/// Flush buffered events to Parquet file
#[cfg(feature = "storage_parquet")]
fn flush_parquet_buffer(run: &mut ActiveArrowRun) -> Result<()> {
    use arrow::array::{ArrayRef, Float64Builder, UInt64Builder};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use parquet::arrow::ArrowWriter;
    use parquet::basic::Compression;
    use parquet::file::properties::WriterProperties;
    use std::fs::File;

    if run.event_buffer.is_empty() {
        return Ok(());
    }

    let schema = run
        .schema
        .as_ref()
        .ok_or_else(|| anyhow!("No schema defined"))?;

    // Build arrays from buffered events
    let mut seq_num_builder = UInt64Builder::new();
    let mut time_ns_builder = UInt64Builder::new();

    let mut data_builders: HashMap<String, Float64Builder> = HashMap::new();
    for key in run.data_keys.keys() {
        data_builders.insert(key.clone(), Float64Builder::new());
    }

    for event in &run.event_buffer {
        seq_num_builder.append_value(event.seq_num);
        time_ns_builder.append_value(event.time_ns);

        for (key, builder) in &mut data_builders {
            if let Some(value) = event.data.get(key) {
                builder.append_value(*value);
            } else {
                builder.append_null();
            }
        }
    }

    let mut columns: Vec<ArrayRef> = vec![
        Arc::new(seq_num_builder.finish()),
        Arc::new(time_ns_builder.finish()),
    ];

    for field in schema.fields().iter().skip(2) {
        if let Some(builder) = data_builders.get_mut(field.name()) {
            columns.push(Arc::new(builder.finish()));
        }
    }

    let batch = RecordBatch::try_new(schema.clone(), columns)?;

    // Write to Parquet with compression
    let file = File::create(&run.file_path)?;
    let props = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();

    let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(props))?;
    writer.write(&batch)?;
    writer.close()?;

    run.event_buffer.clear();

    Ok(())
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[tokio::test]
    #[cfg(feature = "storage_arrow")]
    async fn test_arrow_writer_basic() {
        use daq_core::experiment::document::{DataKey, DescriptorDoc, EventDoc, StartDoc, StopDoc};
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let writer = ArrowDocumentWriter::new(temp_dir.path().to_path_buf());

        // Start
        let start = StartDoc {
            uid: "test_run".to_string(),
            time_ns: 1000,
            plan_type: "count".to_string(),
            plan_name: "Count".to_string(),
            plan_args: HashMap::new(),
            metadata: HashMap::new(),
            hints: vec![],
        };
        writer.write(Document::Start(start)).await.unwrap();

        // Descriptor
        let mut data_keys = HashMap::new();
        data_keys.insert(
            "det1".to_string(),
            DataKey {
                source: "det1".to_string(),
                dtype: "number".to_string(),
                shape: vec![],
                units: "".to_string(),
                precision: None,
                lower_limit: None,
                upper_limit: None,
            },
        );

        let descriptor = DescriptorDoc {
            run_uid: "test_run".to_string(),
            uid: "desc_1".to_string(),
            name: "primary".to_string(),
            data_keys,
            configuration: HashMap::new(),
            time_ns: 0,
        };
        writer
            .write(Document::Descriptor(descriptor))
            .await
            .unwrap();

        // Events
        for i in 0..10 {
            let mut data = HashMap::new();
            data.insert("det1".to_string(), i as f64 * 1.5);

            let event = EventDoc {
                descriptor_uid: "desc_1".to_string(),
                seq_num: i,
                data,
                arrays: HashMap::new(),
                timestamps: HashMap::new(),
                metadata: HashMap::new(),
                run_uid: "test_run".to_string(),
                time_ns: 1_000_000_000 + i as u64 * 100_000,
                uid: format!("event_{}", i),
                positions: HashMap::new(),
            };
            writer.write(Document::Event(event)).await.unwrap();
        }

        // Stop
        let stop = StopDoc {
            uid: "stop_1".to_string(),
            run_uid: "test_run".to_string(),
            time_ns: 2_000_000_000,
            exit_status: "success".to_string(),
            reason: "".to_string(),
            num_events: 10,
        };
        writer.write(Document::Stop(stop)).await.unwrap();

        // Verify file exists
        let file_path = temp_dir.path().join("test_run_1000.arrow");
        assert!(file_path.exists());
    }

    #[tokio::test]
    #[cfg(feature = "storage_parquet")]
    async fn test_parquet_writer_basic() {
        use daq_core::experiment::document::{DataKey, DescriptorDoc, EventDoc, StartDoc, StopDoc};
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let writer = ParquetDocumentWriter::new(temp_dir.path().to_path_buf());

        // Start
        let start = StartDoc {
            uid: "test_run".to_string(),
            time_ns: 1000,
            plan_type: "count".to_string(),
            plan_name: "Count".to_string(),
            plan_args: HashMap::new(),
            metadata: HashMap::new(),
            hints: vec![],
        };
        writer.write(Document::Start(start)).await.unwrap();

        // Descriptor
        let mut data_keys = HashMap::new();
        data_keys.insert(
            "det1".to_string(),
            DataKey {
                source: "det1".to_string(),
                dtype: "number".to_string(),
                shape: vec![],
                units: "".to_string(),
                precision: None,
                lower_limit: None,
                upper_limit: None,
            },
        );

        let descriptor = DescriptorDoc {
            run_uid: "test_run".to_string(),
            uid: "desc_1".to_string(),
            name: "primary".to_string(),
            data_keys,
            configuration: HashMap::new(),
            time_ns: 0,
        };
        writer
            .write(Document::Descriptor(descriptor))
            .await
            .unwrap();

        // Events
        for i in 0..10 {
            let mut data = HashMap::new();
            data.insert("det1".to_string(), i as f64 * 1.5);

            let event = EventDoc {
                descriptor_uid: "desc_1".to_string(),
                seq_num: i,
                data,
                arrays: HashMap::new(),
                timestamps: HashMap::new(),
                metadata: HashMap::new(),
                run_uid: "test_run".to_string(),
                time_ns: 1_000_000_000 + i as u64 * 100_000,
                uid: format!("event_{}", i),
                positions: HashMap::new(),
            };
            writer.write(Document::Event(event)).await.unwrap();
        }

        // Stop
        let stop = StopDoc {
            uid: "stop_1".to_string(),
            run_uid: "test_run".to_string(),
            time_ns: 2_000_000_000,
            exit_status: "success".to_string(),
            reason: "".to_string(),
            num_events: 10,
        };
        writer.write(Document::Stop(stop)).await.unwrap();

        // Verify file exists
        let file_path = temp_dir.path().join("test_run_1000.parquet");
        assert!(file_path.exists());
    }
}
