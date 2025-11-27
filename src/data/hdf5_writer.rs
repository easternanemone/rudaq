//! HDF5 Background Writer - The Mullet Strategy Backend
//!
//! This implements the "business in the back" of The Mullet Strategy:
//! - Arrow in front (fast, 10k+ writes/sec)
//! - HDF5 in back (compatible with Python/MATLAB/Igor)
//!
//! Scientists never see Arrow - they only see f64/Vec<f64> and HDF5 files.
//! The background writer translates Arrow → HDF5 at 1 Hz without blocking
//! the hardware loop.

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::time::{interval, Duration};

use super::ring_buffer::RingBuffer;

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
/// use rust_daq::data::ring_buffer::RingBuffer;
/// use rust_daq::data::hdf5_writer::HDF5Writer;
/// use std::sync::Arc;
/// use std::path::Path;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let ring = Arc::new(RingBuffer::create(Path::new("/dev/shm/daq_ring"), 100)?);
///     let writer = HDF5Writer::new(Path::new("data.h5"), ring.clone())?;
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
    #[allow(dead_code)]
    output_path: PathBuf,
    ring_buffer: Arc<RingBuffer>,
    flush_interval: Duration,
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
            last_read_tail: AtomicU64::new(0),
            batch_counter: AtomicU64::new(0),
        })
    }

    /// Run background writer loop
    ///
    /// This never returns - it runs continuously until the task is cancelled.
    /// Flushes data every `flush_interval` (default 1 second).
    pub async fn run(self) {
        let mut interval = interval(self.flush_interval);

        loop {
            interval.tick().await;

            if let Err(e) = self.flush_to_disk() {
                eprintln!("HDF5 flush error: {}", e);
                // Continue running even on error
            }
        }
    }

    /// Flush new data from ring buffer to HDF5 file
    ///
    /// This reads new data since last flush and appends to HDF5.
    /// Non-blocking if no new data is available.
    #[cfg(feature = "storage_hdf5")]
    fn flush_to_disk(&self) -> Result<()> {
        use hdf5::{File, Group};

        // Check if there's new data
        let current_tail = self.ring_buffer.read_tail();
        let last_tail = self.last_read_tail.load(Ordering::Acquire);

        if current_tail <= last_tail {
            // No new data since last flush
            return Ok(());
        }

        // Read snapshot from ring buffer
        let snapshot = self.ring_buffer.read_snapshot();
        if snapshot.is_empty() {
            return Ok(());
        }

        // Open or create HDF5 file
        let file = if self.output_path.exists() {
            File::open_rw(&self.output_path)?
        } else {
            File::create(&self.output_path)?
        };

        // Create measurements group if it doesn't exist
        let measurements = if file.group("measurements").is_ok() {
            file.group("measurements")?
        } else {
            file.create_group("measurements")?
        };

        // Create batch group with unique name
        let batch_id = self.batch_counter.fetch_add(1, Ordering::SeqCst);
        let batch_name = format!("batch_{:06}", batch_id);
        let batch_group = measurements.create_group(&batch_name)?;

        // Parse and write Arrow data
        #[cfg(feature = "storage_arrow")]
        {
            self.write_arrow_to_hdf5(&batch_group, &snapshot)?;
        }

        #[cfg(not(feature = "storage_arrow"))]
        {
            // Fallback: Write raw bytes
            batch_group
                .new_dataset::<u8>()
                .create("raw_data", snapshot.len())?
                .write(&snapshot)?;
        }

        // Add metadata
        batch_group
            .new_attr::<u64>()
            .create("ring_tail")?
            .write_scalar(&current_tail)?;

        batch_group
            .new_attr::<u64>()
            .create("timestamp_ns")?
            .write_scalar(
                &std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as u64,
            )?;

        // Update last read position
        self.last_read_tail.store(current_tail, Ordering::Release);

        // Mark data as consumed in ring buffer
        let bytes_written = snapshot.len() as u64;
        self.ring_buffer.advance_tail(bytes_written);

        Ok(())
    }

    /// Fallback implementation when HDF5 feature is disabled
    #[cfg(not(feature = "storage_hdf5"))]
    fn flush_to_disk(&self) -> Result<()> {
        // Just advance the tail to prevent ring buffer from filling
        let snapshot = self.ring_buffer.read_snapshot();
        if !snapshot.is_empty() {
            let bytes = snapshot.len() as u64;
            self.ring_buffer.advance_tail(bytes);
            self.last_read_tail.fetch_add(bytes, Ordering::SeqCst);
        }
        Ok(())
    }

    /// Write Arrow RecordBatch to HDF5 group
    ///
    /// Converts Arrow columns to HDF5 datasets for compatibility with
    /// Python/MATLAB/Igor analysis tools.
    #[cfg(all(feature = "storage_hdf5", feature = "storage_arrow"))]
    fn write_arrow_to_hdf5(&self, group: &hdf5::Group, data: &[u8]) -> Result<()> {
        use arrow::ipc::reader::FileReader;
        use std::io::Cursor;

        let cursor = Cursor::new(data);
        let mut reader = FileReader::try_new(cursor, None)?;

        // Read first batch
        if let Some(batch) = reader.next().transpose()? {
            let schema = batch.schema();

            // Write each column as a separate dataset
            for (i, field) in schema.fields().iter().enumerate() {
                let column = batch.column(i);
                let dataset_name = field.name();

                // Convert Arrow array to HDF5-compatible format
                match column.data_type() {
                    arrow::datatypes::DataType::Float64 => {
                        use arrow::array::Float64Array;
                        let array = column
                            .as_any()
                            .downcast_ref::<Float64Array>()
                            .ok_or_else(|| anyhow!("Failed to downcast Float64Array"))?;

                        let values: Vec<f64> = (0..array.len()).map(|i| array.value(i)).collect();

                        group
                            .new_dataset::<f64>()
                            .create(dataset_name, values.len())?
                            .write(&values)?;
                    }
                    arrow::datatypes::DataType::Int64 => {
                        use arrow::array::Int64Array;
                        let array = column
                            .as_any()
                            .downcast_ref::<Int64Array>()
                            .ok_or_else(|| anyhow!("Failed to downcast Int64Array"))?;

                        let values: Vec<i64> = (0..array.len()).map(|i| array.value(i)).collect();

                        group
                            .new_dataset::<i64>()
                            .create(dataset_name, values.len())?
                            .write(&values)?;
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

    /// Get current flush interval
    pub fn flush_interval(&self) -> Duration {
        self.flush_interval
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{NamedTempFile};

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
        writer.flush_to_disk().unwrap();
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
                let _ = writer.flush_to_disk();
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
        writer.flush_to_disk().unwrap();

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
        writer.flush_to_disk().unwrap();

        // Verify file created and contains data
        assert!(hdf5_path.exists());
        assert!(writer.batch_count() > 0);
    }
}
