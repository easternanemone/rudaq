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
//! # Tensor Support
//!
//! N-dimensional arrays (images, spectra, spectral cubes) are stored using Arrow's
//! `FixedSizeList<Float64>` type with tensor shape metadata. Each field carrying
//! tensor data has the following custom metadata entries:
//!
//! - `ARROW:tensor:shape` - JSON array of dimension sizes, e.g. `[480,640]` for a
//!   480-row by 640-column image.
//! - `ARROW:tensor:dim_names` (optional) - JSON array of dimension labels, e.g.
//!   `["height","width"]`.
//!
//! This convention allows downstream readers (Python, Julia, etc.) to reconstruct
//! the original N-dimensional layout from the flat FixedSizeList storage.
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
use anyhow::{anyhow, Context, Result};
#[cfg(feature = "storage_arrow")]
use arrow::array::{ArrayRef, Float64Builder, UInt64Builder};
#[cfg(feature = "storage_arrow")]
use arrow::datatypes::{DataType, Field, Schema};
#[cfg(feature = "storage_arrow")]
use arrow::record_batch::RecordBatch;
#[cfg(feature = "storage_arrow")]
use bytes::Bytes;
#[cfg(feature = "storage_arrow")]
use common::experiment::document::Document;

// ── Tensor metadata constants ────────────────────────────────────────────────

/// Arrow field metadata key for tensor shape (JSON array of dimension sizes).
#[cfg(feature = "storage_arrow")]
pub const TENSOR_SHAPE_KEY: &str = "ARROW:tensor:shape";

/// Arrow field metadata key for tensor dimension names (optional JSON array).
#[cfg(feature = "storage_arrow")]
pub const TENSOR_DIM_NAMES_KEY: &str = "ARROW:tensor:dim_names";

// ── Tensor metadata helpers ──────────────────────────────────────────────────

/// Extract the tensor shape from an Arrow field's custom metadata.
///
/// Returns `None` if the field has no tensor shape metadata.
/// Returns `Some(vec![...])` with the dimension sizes if present.
#[cfg(feature = "storage_arrow")]
pub fn read_tensor_shape(field: &Field) -> Option<Vec<i32>> {
    field
        .metadata()
        .get(TENSOR_SHAPE_KEY)
        .and_then(|s| serde_json::from_str(s).ok())
}

/// Check whether a `DataKey` shape describes an N-dimensional tensor (ndim >= 1).
#[cfg(feature = "storage_arrow")]
fn is_tensor_shape(shape: &[i32]) -> bool {
    !shape.is_empty() && shape.iter().all(|&d| d > 0)
}

/// Compute the total number of elements in a tensor from its shape.
#[cfg(feature = "storage_arrow")]
#[allow(clippy::cast_sign_loss)]
// SAFETY: value is non-negative at this point
fn tensor_num_elements(shape: &[i32]) -> usize {
    shape.iter().map(|&d| d as usize).product()
}

// ── Internal state ───────────────────────────────────────────────────────────

/// Internal state for an active run
#[cfg(feature = "storage_arrow")]
struct ActiveArrowRun {
    run_uid: String,
    file_path: PathBuf,
    /// Schema derived from descriptor
    schema: Option<Arc<Schema>>,
    /// Buffered events (converted to columns on flush)
    event_buffer: Vec<BufferedEvent>,
    /// Data key info from descriptor
    data_keys: HashMap<String, DataKeyInfo>,
    /// Metadata from start document
    #[allow(dead_code)]
    metadata: HashMap<String, String>,
}

#[cfg(feature = "storage_arrow")]
#[derive(Clone)]
struct DataKeyInfo {
    dtype: String,
    #[allow(dead_code)]
    source: String,
    /// Tensor dimensions (empty for scalars)
    shape: Vec<i32>,
}

#[cfg(feature = "storage_arrow")]
struct BufferedEvent {
    seq_num: u64,
    time_ns: u64,
    /// Scalar data (field name -> value)
    data: HashMap<String, f64>,
    /// Array/tensor data (field name -> raw bytes, little-endian f64)
    arrays: HashMap<String, Bytes>,
}

// ── Schema construction helpers ──────────────────────────────────────────────

/// Build an Arrow `Field` for a descriptor data key.
///
/// Scalars map to `Float64` (or `Utf8` for strings).
/// Tensors map to `FixedSizeList<Float64>` with shape metadata.
#[cfg(feature = "storage_arrow")]
fn build_arrow_field(key: &str, info: &DataKeyInfo) -> Field {
    if is_tensor_shape(&info.shape) {
        let num_elements = tensor_num_elements(&info.shape);
        let inner_type = match info.dtype.as_str() {
            "uint16" | "uint32" | "int16" | "int32" => DataType::Float64, // promote to f64
            _ => DataType::Float64,
        };
        let inner_field = Arc::new(Field::new("item", inner_type, true));
        let list_type =
            DataType::FixedSizeList(inner_field, i32::try_from(num_elements).unwrap_or(i32::MAX));

        // Attach tensor metadata
        let mut metadata = HashMap::new();
        metadata.insert(
            TENSOR_SHAPE_KEY.to_string(),
            serde_json::to_string(&info.shape).unwrap_or_default(),
        );

        Field::new(key, list_type, true).with_metadata(metadata)
    } else {
        // Scalar field
        let dtype = match info.dtype.as_str() {
            "number" | "float64" | "f64" => DataType::Float64,
            "string" => DataType::Utf8,
            _ => DataType::Float64,
        };
        Field::new(key, dtype, true)
    }
}

// ── Arrow IPC Writer ─────────────────────────────────────────────────────────

/// Arrow IPC Writer for RunEngine Documents
///
/// Writes documents to Arrow IPC format, suitable for streaming and
/// inter-process communication. Supports scalar, 1-D array, 2-D (image),
/// and 3-D (spectral cube) tensor data.
///
/// # Example
///
/// ```ignore
/// use storage::arrow_writer::ArrowDocumentWriter;
/// use storage::config::StorageConfig;
///
/// let config = StorageConfig::from_env();
/// let writer = ArrowDocumentWriter::new(config.data_subdir("runs"));
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
    /// Create a new `ArrowDocumentWriter`
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

                        // Fixed columns: seq_num, time_ns
                        let mut fields = vec![
                            Field::new("seq_num", DataType::UInt64, false),
                            Field::new("time_ns", DataType::UInt64, false),
                        ];

                        for (key, meta) in &desc.data_keys {
                            let info = DataKeyInfo {
                                dtype: meta.dtype.clone(),
                                source: meta.source.clone(),
                                shape: meta.shape.clone(),
                            };

                            fields.push(build_arrow_field(key, &info));
                            run.data_keys.insert(key.clone(), info);
                        }

                        run.schema = Some(Arc::new(Schema::new(fields)));
                    }
                }
                Document::Event(event) => {
                    if let Some(run) = guard.as_mut() {
                        run.event_buffer.push(BufferedEvent {
                            seq_num: u64::from(event.seq_num),
                            time_ns: event.time_ns,
                            data: event.data.clone(),
                            arrays: event.arrays.clone(),
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

// ── Flush helpers ────────────────────────────────────────────────────────────

/// Build a `FixedSizeListArray` column from buffered tensor byte data.
///
/// Each event's bytes for this key are interpreted as little-endian `f64` values.
/// The resulting array has one fixed-size list entry per event, with `num_elements`
/// f64 values per entry.
#[cfg(feature = "storage_arrow")]
fn build_tensor_column(
    key: &str,
    info: &DataKeyInfo,
    events: &[BufferedEvent],
) -> Result<ArrayRef> {
    use arrow::array::FixedSizeListBuilder;

    let num_elements = tensor_num_elements(&info.shape);
    let list_size = i32::try_from(num_elements).context("tensor element count exceeds i32")?;
    let mut builder = FixedSizeListBuilder::new(Float64Builder::new(), list_size);

    for event in events {
        if let Some(raw) = event.arrays.get(key) {
            let expected_f64_bytes = num_elements * 8;
            let expected_u16_bytes = num_elements * 2;

            if raw.len() == expected_f64_bytes {
                // Data is already f64 little-endian
                let values = builder.values();
                for chunk in raw.chunks_exact(8) {
                    let val = f64::from_le_bytes([
                        chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6],
                        chunk[7],
                    ]);
                    values.append_value(val);
                }
                builder.append(true);
            } else if info.dtype == "uint16" && raw.len() == expected_u16_bytes {
                // uint16 raw data: 2 bytes per element, promote to f64
                let values = builder.values();
                for chunk in raw.chunks_exact(2) {
                    let val = f64::from(u16::from_le_bytes([chunk[0], chunk[1]]));
                    values.append_value(val);
                }
                builder.append(true);
            } else {
                // Size mismatch - append null entry (pad inner values for alignment)
                let values = builder.values();
                for _ in 0..num_elements {
                    values.append_null();
                }
                builder.append(false);
            }
        } else {
            // No array data for this event - null entry
            let values = builder.values();
            for _ in 0..num_elements {
                values.append_null();
            }
            builder.append(false);
        }
    }

    Ok(Arc::new(builder.finish()))
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

    // Build fixed columns
    let mut seq_num_builder = UInt64Builder::new();
    let mut time_ns_builder = UInt64Builder::new();

    // Build scalar-data builders (only for non-tensor keys)
    let mut scalar_builders: HashMap<String, Float64Builder> = HashMap::new();
    for (key, info) in &run.data_keys {
        if !is_tensor_shape(&info.shape) {
            scalar_builders.insert(key.clone(), Float64Builder::new());
        }
    }

    for event in &run.event_buffer {
        seq_num_builder.append_value(event.seq_num);
        time_ns_builder.append_value(event.time_ns);

        for (key, builder) in &mut scalar_builders {
            if let Some(value) = event.data.get(key) {
                builder.append_value(*value);
            } else {
                builder.append_null();
            }
        }
    }

    // Assemble columns in schema order
    let mut columns: Vec<ArrayRef> = vec![
        Arc::new(seq_num_builder.finish()),
        Arc::new(time_ns_builder.finish()),
    ];

    for field in schema.fields().iter().skip(2) {
        let key = field.name();
        if let Some(info) = run.data_keys.get(key) {
            if is_tensor_shape(&info.shape) {
                columns.push(build_tensor_column(key, info, &run.event_buffer)?);
            } else if let Some(builder) = scalar_builders.get_mut(key) {
                columns.push(Arc::new(builder.finish()));
            }
        }
    }

    let batch = RecordBatch::try_new(schema.clone(), columns)?;

    // Write to file (append if exists)
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&run.file_path)?;

    let mut writer = FileWriter::try_new(file, schema)?;
    writer.write(&batch)?;
    writer.finish()?;

    // Clear buffer
    run.event_buffer.clear();

    Ok(())
}

// ── Parquet Writer ───────────────────────────────────────────────────────────

/// Parquet Writer for RunEngine Documents
///
/// Writes documents to Parquet format, suitable for analytics and
/// long-term archival storage.
///
/// # Example
///
/// ```ignore
/// use storage::arrow_writer::ParquetDocumentWriter;
/// use storage::config::StorageConfig;
///
/// let config = StorageConfig::from_env();
/// let writer = ParquetDocumentWriter::new(config.data_subdir("runs"));
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
    /// Create a new `ParquetDocumentWriter`
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
                            let info = DataKeyInfo {
                                dtype: meta.dtype.clone(),
                                source: meta.source.clone(),
                                shape: meta.shape.clone(),
                            };

                            fields.push(build_arrow_field(key, &info));
                            run.data_keys.insert(key.clone(), info);
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
                            arrays: event.arrays.clone(),
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

    // Build fixed columns
    let mut seq_num_builder = UInt64Builder::new();
    let mut time_ns_builder = UInt64Builder::new();

    let mut scalar_builders: HashMap<String, Float64Builder> = HashMap::new();
    for (key, info) in &run.data_keys {
        if !is_tensor_shape(&info.shape) {
            scalar_builders.insert(key.clone(), Float64Builder::new());
        }
    }

    for event in &run.event_buffer {
        seq_num_builder.append_value(event.seq_num);
        time_ns_builder.append_value(event.time_ns);

        for (key, builder) in &mut scalar_builders {
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
        let key = field.name();
        if let Some(info) = run.data_keys.get(key) {
            if is_tensor_shape(&info.shape) {
                columns.push(build_tensor_column(key, info, &run.event_buffer)?);
            } else if let Some(builder) = scalar_builders.get_mut(key) {
                columns.push(Arc::new(builder.finish()));
            }
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

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[tokio::test]
    #[cfg(feature = "storage_arrow")]
    async fn test_arrow_writer_basic() {
        use common::experiment::document::{DataKey, DescriptorDoc, EventDoc, StartDoc, StopDoc};
        use tempfile::TempDir;

        let temp_dir = TempDir::new().expect("create temp dir");
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
        writer
            .write(Document::Start(start))
            .await
            .expect("write start");

        // Descriptor
        let mut data_keys = HashMap::new();
        data_keys.insert(
            "det1".to_string(),
            DataKey {
                source: "det1".to_string(),
                dtype: "number".to_string(),
                shape: vec![],
                units: String::new(),
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
            .expect("write descriptor");

        // Events
        for i in 0..10 {
            let mut data = HashMap::new();
            data.insert("det1".to_string(), f64::from(i) * 1.5);

            let event = EventDoc {
                descriptor_uid: "desc_1".to_string(),
                seq_num: i,
                data,
                arrays: HashMap::new(),
                timestamps: HashMap::new(),
                metadata: HashMap::new(),
                run_uid: "test_run".to_string(),
                time_ns: 1_000_000_000 + u64::from(i) * 100_000,
                uid: format!("event_{i}"),
                positions: HashMap::new(),
            };
            writer
                .write(Document::Event(event))
                .await
                .expect("write event");
        }

        // Stop
        let stop = StopDoc {
            uid: "stop_1".to_string(),
            run_uid: "test_run".to_string(),
            time_ns: 2_000_000_000,
            exit_status: "success".to_string(),
            reason: String::new(),
            num_events: 10,
        };
        writer
            .write(Document::Stop(stop))
            .await
            .expect("write stop");

        // Verify file exists
        let file_path = temp_dir.path().join("test_run_1000.arrow");
        assert!(file_path.exists());
    }

    /// Round-trip test for a 2D tensor (image: 4x8 f64).
    #[tokio::test]
    #[cfg(feature = "storage_arrow")]
    async fn test_arrow_tensor_2d_roundtrip() {
        use arrow::array::AsArray;
        use arrow::ipc::reader::FileReader;
        use common::experiment::document::{DataKey, DescriptorDoc, EventDoc, StartDoc, StopDoc};
        use std::fs::File;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().expect("create temp dir");
        let writer = ArrowDocumentWriter::new(temp_dir.path().to_path_buf());

        let height: i32 = 4;
        let width: i32 = 8;
        #[allow(clippy::cast_sign_loss)]
        // SAFETY: value is non-negative at this point
        let num_elements = (height * width) as usize;

        // Start
        let start = StartDoc {
            uid: "tensor2d".to_string(),
            time_ns: 2000,
            plan_type: "count".to_string(),
            plan_name: "Tensor2D".to_string(),
            plan_args: HashMap::new(),
            metadata: HashMap::new(),
            hints: vec![],
        };
        writer
            .write(Document::Start(start))
            .await
            .expect("write start");

        // Descriptor with scalar + 2D tensor
        let mut data_keys = HashMap::new();
        data_keys.insert(
            "intensity".to_string(),
            DataKey {
                source: "det".to_string(),
                dtype: "number".to_string(),
                shape: vec![],
                units: "counts".to_string(),
                precision: None,
                lower_limit: None,
                upper_limit: None,
            },
        );
        data_keys.insert(
            "image".to_string(),
            DataKey {
                source: "cam1".to_string(),
                dtype: "float64".to_string(),
                shape: vec![height, width],
                units: String::new(),
                precision: None,
                lower_limit: None,
                upper_limit: None,
            },
        );

        let descriptor = DescriptorDoc {
            run_uid: "tensor2d".to_string(),
            uid: "desc_2d".to_string(),
            name: "primary".to_string(),
            data_keys,
            configuration: HashMap::new(),
            time_ns: 0,
        };
        writer
            .write(Document::Descriptor(descriptor))
            .await
            .expect("write descriptor");

        // Write 3 events with image data
        let mut expected_images: Vec<Vec<f64>> = Vec::new();
        for seq in 0u32..3 {
            let mut data = HashMap::new();
            data.insert("intensity".to_string(), f64::from(seq) * 10.0);

            // Create image: pixel value = seq * 100 + pixel_index
            #[allow(clippy::cast_precision_loss)]
            // SAFETY: precision loss acceptable for metrics/display
            let image_data: Vec<f64> = (0..num_elements)
                .map(|i| f64::from(seq) * 100.0 + i as f64)
                .collect();
            expected_images.push(image_data.clone());

            let raw_bytes: Vec<u8> = image_data.iter().flat_map(|v| v.to_le_bytes()).collect();
            let mut arrays = HashMap::new();
            arrays.insert("image".to_string(), Bytes::from(raw_bytes));

            let event = EventDoc {
                descriptor_uid: "desc_2d".to_string(),
                seq_num: seq,
                data,
                arrays,
                timestamps: HashMap::new(),
                metadata: HashMap::new(),
                run_uid: "tensor2d".to_string(),
                time_ns: 3_000_000_000 + u64::from(seq) * 100_000,
                uid: format!("ev2d_{seq}"),
                positions: HashMap::new(),
            };
            writer
                .write(Document::Event(event))
                .await
                .expect("write event");
        }

        // Stop
        let stop = StopDoc {
            uid: "stop_2d".to_string(),
            run_uid: "tensor2d".to_string(),
            time_ns: 4_000_000_000,
            exit_status: "success".to_string(),
            reason: String::new(),
            num_events: 3,
        };
        writer
            .write(Document::Stop(stop))
            .await
            .expect("write stop");

        // ── Read back and verify ──
        let file_path = temp_dir.path().join("tensor2d_2000.arrow");
        assert!(file_path.exists(), "Arrow file should exist");

        let file = File::open(&file_path).expect("open arrow file");
        let reader = FileReader::try_new(file, None).expect("create FileReader");
        let schema = reader.schema();

        // Check tensor shape metadata on the image field
        let image_field = schema
            .field_with_name("image")
            .expect("image field should exist");
        #[allow(clippy::cast_sign_loss)]
        let shape = read_tensor_shape(image_field).expect("should have tensor shape metadata");
        assert_eq!(shape, vec![height, width]);

        // Check the FixedSizeList type
        match image_field.data_type() {
            DataType::FixedSizeList(_, size) => {
                assert_eq!(*size as usize, num_elements);
            }
            other => panic!("expected FixedSizeList, got {other:?}"),
        }

        // Read all batches and verify data
        let mut total_rows = 0usize;
        for batch_result in reader {
            let batch = batch_result.expect("read batch");
            let image_col = batch.column_by_name("image").expect("image column");
            let intensity_col = batch.column_by_name("intensity").expect("intensity column");

            let fsl = image_col.as_fixed_size_list();
            let intensities = intensity_col
                .as_any()
                .downcast_ref::<arrow::array::Float64Array>()
                .expect("intensity f64");

            for row in 0..batch.num_rows() {
                #[allow(clippy::cast_precision_loss)]
                // SAFETY: precision loss acceptable for metrics/display
                #[allow(clippy::cast_precision_loss)]
                let event_idx = total_rows + row;

                // Verify scalar
                assert!(
                    (intensities.value(row) - event_idx as f64 * 10.0).abs() < 1e-12,
                    "scalar mismatch at event {event_idx}"
                );

                // Verify tensor values
                let list_value = fsl.value(row);
                let values = list_value
                    .as_any()
                    .downcast_ref::<arrow::array::Float64Array>()
                    .expect("inner f64 array");
                assert_eq!(values.len(), num_elements);

                for (i, &expected) in expected_images[event_idx].iter().enumerate() {
                    assert!(
                        (values.value(i) - expected).abs() < 1e-12,
                        "tensor value mismatch at event {event_idx}, element {i}"
                    );
                }
            }
            total_rows += batch.num_rows();
        }
        assert_eq!(total_rows, 3, "should have 3 events");
    }

    /// Round-trip test for a 3D tensor (spectral cube: 2x3x4 f64).
    #[tokio::test]
    #[cfg(feature = "storage_arrow")]
    async fn test_arrow_tensor_3d_roundtrip() {
        use arrow::array::AsArray;
        use arrow::ipc::reader::FileReader;
        use common::experiment::document::{DataKey, DescriptorDoc, EventDoc, StartDoc, StopDoc};
        use std::fs::File;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().expect("create temp dir");
        let writer = ArrowDocumentWriter::new(temp_dir.path().to_path_buf());

        let depth: i32 = 2;
        let height: i32 = 3;
        let width: i32 = 4;
        #[allow(clippy::cast_sign_loss)]
        // SAFETY: value is non-negative at this point
        let num_elements = (depth * height * width) as usize; // 24

        // Start
        let start = StartDoc {
            uid: "tensor3d".to_string(),
            time_ns: 5000,
            plan_type: "count".to_string(),
            plan_name: "Tensor3D".to_string(),
            plan_args: HashMap::new(),
            metadata: HashMap::new(),
            hints: vec![],
        };
        writer
            .write(Document::Start(start))
            .await
            .expect("write start");

        // Descriptor with 3D tensor
        let mut data_keys = HashMap::new();
        data_keys.insert(
            "cube".to_string(),
            DataKey {
                source: "spectrometer".to_string(),
                dtype: "float64".to_string(),
                shape: vec![depth, height, width],
                units: String::new(),
                precision: None,
                lower_limit: None,
                upper_limit: None,
            },
        );

        let descriptor = DescriptorDoc {
            run_uid: "tensor3d".to_string(),
            uid: "desc_3d".to_string(),
            name: "primary".to_string(),
            data_keys,
            configuration: HashMap::new(),
            time_ns: 0,
        };
        writer
            .write(Document::Descriptor(descriptor))
            .await
            .expect("write descriptor");

        // Write 2 events
        let mut expected_cubes: Vec<Vec<f64>> = Vec::new();
        for seq in 0u32..2 {
            #[allow(clippy::cast_precision_loss)]
            // SAFETY: precision loss acceptable for metrics/display
            let cube_data: Vec<f64> = (0..num_elements)
                .map(|i| f64::from(seq) * 1000.0 + i as f64 * 0.5)
                .collect();
            expected_cubes.push(cube_data.clone());

            let raw_bytes: Vec<u8> = cube_data.iter().flat_map(|v| v.to_le_bytes()).collect();
            let mut arrays = HashMap::new();
            arrays.insert("cube".to_string(), Bytes::from(raw_bytes));

            let event = EventDoc {
                descriptor_uid: "desc_3d".to_string(),
                seq_num: seq,
                data: HashMap::new(),
                arrays,
                timestamps: HashMap::new(),
                metadata: HashMap::new(),
                run_uid: "tensor3d".to_string(),
                time_ns: 6_000_000_000 + u64::from(seq) * 100_000,
                uid: format!("ev3d_{seq}"),
                positions: HashMap::new(),
            };
            writer
                .write(Document::Event(event))
                .await
                .expect("write event");
        }

        // Stop
        let stop = StopDoc {
            uid: "stop_3d".to_string(),
            run_uid: "tensor3d".to_string(),
            time_ns: 7_000_000_000,
            exit_status: "success".to_string(),
            reason: String::new(),
            num_events: 2,
        };
        writer
            .write(Document::Stop(stop))
            .await
            .expect("write stop");

        // ── Read back and verify ──
        let file_path = temp_dir.path().join("tensor3d_5000.arrow");
        assert!(file_path.exists(), "Arrow file should exist");

        let file = File::open(&file_path).expect("open arrow file");
        let reader = FileReader::try_new(file, None).expect("create FileReader");
        let schema = reader.schema();

        // Verify shape metadata
        let cube_field = schema
            .field_with_name("cube")
            .expect("cube field should exist");
        let shape = read_tensor_shape(cube_field).expect("should have tensor shape metadata");
        assert_eq!(shape, vec![depth, height, width]);

        // Verify data
        let mut total_rows = 0usize;
        for batch_result in reader {
            let batch = batch_result.expect("read batch");
            let cube_col = batch.column_by_name("cube").expect("cube column");
            let fsl = cube_col.as_fixed_size_list();

            for row in 0..batch.num_rows() {
                let event_idx = total_rows + row;
                let list_value = fsl.value(row);
                let values = list_value
                    .as_any()
                    .downcast_ref::<arrow::array::Float64Array>()
                    .expect("inner f64");
                assert_eq!(values.len(), num_elements);

                for (i, &expected) in expected_cubes[event_idx].iter().enumerate() {
                    assert!(
                        (values.value(i) - expected).abs() < 1e-12,
                        "cube value mismatch at event {event_idx}, element {i}"
                    );
                }
            }
            total_rows += batch.num_rows();
        }
        assert_eq!(total_rows, 2);
    }

    /// Test that uint16 tensor data (camera frames) is correctly promoted to f64.
    #[tokio::test]
    #[cfg(feature = "storage_arrow")]
    async fn test_arrow_tensor_uint16_promotion() {
        use arrow::array::AsArray;
        use arrow::ipc::reader::FileReader;
        use common::experiment::document::{DataKey, DescriptorDoc, EventDoc, StartDoc, StopDoc};
        use std::fs::File;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().expect("create temp dir");
        let writer = ArrowDocumentWriter::new(temp_dir.path().to_path_buf());

        let height: i32 = 3;
        let width: i32 = 4;
        #[allow(clippy::cast_sign_loss)]
        // SAFETY: value is non-negative at this point
        let num_elements = (height * width) as usize;

        let start = StartDoc {
            uid: "u16test".to_string(),
            time_ns: 8000,
            plan_type: "count".to_string(),
            plan_name: "U16Test".to_string(),
            plan_args: HashMap::new(),
            metadata: HashMap::new(),
            hints: vec![],
        };
        writer
            .write(Document::Start(start))
            .await
            .expect("write start");

        let mut data_keys = HashMap::new();
        data_keys.insert(
            "frame".to_string(),
            DataKey {
                source: "cam".to_string(),
                dtype: "uint16".to_string(),
                shape: vec![height, width],
                units: String::new(),
                precision: None,
                lower_limit: None,
                upper_limit: None,
            },
        );

        let descriptor = DescriptorDoc {
            run_uid: "u16test".to_string(),
            uid: "desc_u16".to_string(),
            name: "primary".to_string(),
            data_keys,
            configuration: HashMap::new(),
            time_ns: 0,
        };
        writer
            .write(Document::Descriptor(descriptor))
            .await
            .expect("write descriptor");

        // Write uint16 frame data (2 bytes per element)
        #[allow(clippy::cast_possible_truncation)]
        // SAFETY: value is bounded and fits in target type
        let u16_data: Vec<u16> = (0..num_elements).map(|i| (i * 100) as u16).collect();
        let raw_bytes: Vec<u8> = u16_data.iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut arrays = HashMap::new();
        arrays.insert("frame".to_string(), Bytes::from(raw_bytes));

        let event = EventDoc {
            descriptor_uid: "desc_u16".to_string(),
            seq_num: 0,
            data: HashMap::new(),
            arrays,
            timestamps: HashMap::new(),
            metadata: HashMap::new(),
            run_uid: "u16test".to_string(),
            time_ns: 9_000_000_000,
            uid: "ev_u16_0".to_string(),
            positions: HashMap::new(),
        };
        writer
            .write(Document::Event(event))
            .await
            .expect("write event");

        let stop = StopDoc {
            uid: "stop_u16".to_string(),
            run_uid: "u16test".to_string(),
            time_ns: 10_000_000_000,
            exit_status: "success".to_string(),
            reason: String::new(),
            num_events: 1,
        };
        writer
            .write(Document::Stop(stop))
            .await
            .expect("write stop");

        // Read back
        let file_path = temp_dir.path().join("u16test_8000.arrow");
        assert!(file_path.exists());

        let file = File::open(&file_path).expect("open arrow file");
        let reader = FileReader::try_new(file, None).expect("create FileReader");

        for batch_result in reader {
            let batch = batch_result.expect("read batch");
            let frame_col = batch.column_by_name("frame").expect("frame column");
            let fsl = frame_col.as_fixed_size_list();

            let list_value = fsl.value(0);
            let values = list_value
                .as_any()
                .downcast_ref::<arrow::array::Float64Array>()
                .expect("inner f64");
            assert_eq!(values.len(), num_elements);

            for (i, &expected_u16) in u16_data.iter().enumerate() {
                assert!(
                    (values.value(i) - f64::from(expected_u16)).abs() < 1e-12,
                    "uint16 promotion mismatch at element {i}"
                );
            }
        }
    }

    /// Verify that mixed scalar + tensor events work correctly together.
    #[tokio::test]
    #[cfg(feature = "storage_arrow")]
    async fn test_arrow_mixed_scalar_and_tensor() {
        use arrow::array::AsArray;
        use arrow::ipc::reader::FileReader;
        use common::experiment::document::{DataKey, DescriptorDoc, EventDoc, StartDoc, StopDoc};
        use std::fs::File;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().expect("create temp dir");
        let writer = ArrowDocumentWriter::new(temp_dir.path().to_path_buf());

        let start = StartDoc {
            uid: "mixed".to_string(),
            time_ns: 11000,
            plan_type: "scan".to_string(),
            plan_name: "Mixed".to_string(),
            plan_args: HashMap::new(),
            metadata: HashMap::new(),
            hints: vec![],
        };
        writer
            .write(Document::Start(start))
            .await
            .expect("write start");

        // 1 scalar + 1 tensor (1D spectrum with 5 elements)
        let mut data_keys = HashMap::new();
        data_keys.insert(
            "power".to_string(),
            DataKey {
                source: "pm".to_string(),
                dtype: "number".to_string(),
                shape: vec![],
                units: "W".to_string(),
                precision: None,
                lower_limit: None,
                upper_limit: None,
            },
        );
        data_keys.insert(
            "spectrum".to_string(),
            DataKey {
                source: "spec".to_string(),
                dtype: "float64".to_string(),
                shape: vec![5],
                units: String::new(),
                precision: None,
                lower_limit: None,
                upper_limit: None,
            },
        );

        let descriptor = DescriptorDoc {
            run_uid: "mixed".to_string(),
            uid: "desc_mix".to_string(),
            name: "primary".to_string(),
            data_keys,
            configuration: HashMap::new(),
            time_ns: 0,
        };
        writer
            .write(Document::Descriptor(descriptor))
            .await
            .expect("write descriptor");

        // 5 events: each has scalar power and a 5-element spectrum
        for seq in 0u32..5 {
            let mut data = HashMap::new();
            data.insert("power".to_string(), f64::from(seq) * 0.1);

            let spectrum: Vec<f64> = (0..5)
                .map(|i| f64::from(seq) + f64::from(i) * 0.01)
                .collect();
            let raw: Vec<u8> = spectrum.iter().flat_map(|v| v.to_le_bytes()).collect();
            let mut arrays = HashMap::new();
            arrays.insert("spectrum".to_string(), Bytes::from(raw));

            let event = EventDoc {
                descriptor_uid: "desc_mix".to_string(),
                seq_num: seq,
                data,
                arrays,
                timestamps: HashMap::new(),
                metadata: HashMap::new(),
                run_uid: "mixed".to_string(),
                time_ns: 12_000_000_000 + u64::from(seq) * 100_000,
                uid: format!("ev_mix_{seq}"),
                positions: HashMap::new(),
            };
            writer
                .write(Document::Event(event))
                .await
                .expect("write event");
        }

        let stop = StopDoc {
            uid: "stop_mix".to_string(),
            run_uid: "mixed".to_string(),
            time_ns: 13_000_000_000,
            exit_status: "success".to_string(),
            reason: String::new(),
            num_events: 5,
        };
        writer
            .write(Document::Stop(stop))
            .await
            .expect("write stop");

        // Read back
        let file_path = temp_dir.path().join("mixed_11000.arrow");
        assert!(file_path.exists());

        let file = File::open(&file_path).expect("open arrow file");
        let reader = FileReader::try_new(file, None).expect("create FileReader");

        let mut total_rows = 0usize;
        for batch_result in reader {
            let batch = batch_result.expect("read batch");
            let power_col = batch.column_by_name("power").expect("power column");
            let spectrum_col = batch.column_by_name("spectrum").expect("spectrum column");

            let powers = power_col
                .as_any()
                .downcast_ref::<arrow::array::Float64Array>()
                .expect("power f64");
            let spectra = spectrum_col.as_fixed_size_list();

            for row in 0..batch.num_rows() {
                #[allow(clippy::cast_precision_loss)]
                // SAFETY: precision loss acceptable for metrics/display
                #[allow(clippy::cast_precision_loss)]
                let seq = total_rows + row;
                assert!(
                    (powers.value(row) - seq as f64 * 0.1).abs() < 1e-12,
                    "power mismatch at event {seq}"
                );

                let list_val = spectra.value(row);
                let vals = list_val
                    .as_any()
                    .downcast_ref::<arrow::array::Float64Array>()
                    .expect("spectrum f64");
                assert_eq!(vals.len(), 5);
                for i in 0..5 {
                    #[allow(clippy::cast_precision_loss)]
                    // SAFETY: precision loss acceptable for metrics/display
                    let expected = seq as f64 + i as f64 * 0.01;
                    assert!(
                        (vals.value(i) - expected).abs() < 1e-12,
                        "spectrum mismatch at event {seq}, element {i}"
                    );
                }
            }
            total_rows += batch.num_rows();
        }
        assert_eq!(total_rows, 5);
    }

    #[tokio::test]
    #[cfg(feature = "storage_parquet")]
    async fn test_parquet_writer_basic() {
        use common::experiment::document::{DataKey, DescriptorDoc, EventDoc, StartDoc, StopDoc};
        use tempfile::TempDir;

        let temp_dir = TempDir::new().expect("create temp dir");
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
        writer
            .write(Document::Start(start))
            .await
            .expect("write start");

        // Descriptor
        let mut data_keys = HashMap::new();
        data_keys.insert(
            "det1".to_string(),
            DataKey {
                source: "det1".to_string(),
                dtype: "number".to_string(),
                shape: vec![],
                units: String::new(),
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
            .expect("write descriptor");

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
                uid: format!("event_{i}"),
                positions: HashMap::new(),
            };
            writer
                .write(Document::Event(event))
                .await
                .expect("write event");
        }

        // Stop
        let stop = StopDoc {
            uid: "stop_1".to_string(),
            run_uid: "test_run".to_string(),
            time_ns: 2_000_000_000,
            exit_status: "success".to_string(),
            reason: String::new(),
            num_events: 10,
        };
        writer
            .write(Document::Stop(stop))
            .await
            .expect("write stop");

        // Verify file exists
        let file_path = temp_dir.path().join("test_run_1000.parquet");
        assert!(file_path.exists());
    }
}
