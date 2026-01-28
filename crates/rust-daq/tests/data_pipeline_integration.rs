#![cfg(not(target_arch = "wasm32"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    clippy::panic,
    deprecated,
    unsafe_code,
    unused_mut,
    unused_imports,
    missing_docs
)]
//! Data pipeline integration tests
//!
//! Tests for HDF5, Arrow, and ring buffer data flow.
//!
//! # Test Coverage
//!
//! - Ring buffer operations (verified working in unit tests)
//! - HDF5 writer integration (when storage_hdf5 enabled)
//! - Arrow writer integration (when storage_arrow enabled)
//! - Ring buffer to HDF5 background writer flow
//! - High-throughput pipeline stress tests
//!
//! # Feature Gates
//!
//! Tests are conditionally compiled based on enabled features:
//! - `storage_hdf5` - HDF5 file format tests
//! - `storage_arrow` - Apache Arrow IPC format tests
//!
//! # Running Tests
//!
//! ```bash
//! # Test with Arrow support (HDF5 tests will be skipped if library not installed)
//! cargo test data_pipeline --features storage_arrow
//!
//! # Test with HDF5 support (requires HDF5 library: brew install hdf5)
//! cargo test data_pipeline --features storage_hdf5,storage_arrow
//!
//! # Test with both (if HDF5 available)
//! cargo test data_pipeline --features storage_hdf5,storage_arrow
//! ```
//!
//! **Note**: HDF5 tests require the HDF5 library to be installed:
//! - macOS: `brew install hdf5`
//! - Ubuntu: `sudo apt-get install libhdf5-dev`
//! - If HDF5 is not available, those tests will be skipped automatically.

use common::core::Measurement;
use std::sync::Arc;
use tempfile::TempDir;

// =============================================================================
// Test Helper Functions
// =============================================================================

/// Create a scalar measurement for testing
#[allow(dead_code)]
fn create_test_scalar(name: &str, value: f64) -> Measurement {
    Measurement::Scalar {
        name: name.to_string(),
        value,
        unit: "V".to_string(),
        timestamp: chrono::Utc::now(),
    }
}

/// Create a vector measurement for testing
#[allow(dead_code)]
fn create_test_vector(name: &str, values: Vec<f64>) -> Measurement {
    Measurement::Vector {
        name: name.to_string(),
        values,
        unit: "V".to_string(),
        timestamp: chrono::Utc::now(),
    }
}

/// Create a spectrum measurement for testing
#[allow(dead_code)]
fn create_test_spectrum(name: &str, n_bins: usize) -> Measurement {
    let frequencies: Vec<f64> = (0..n_bins).map(|i| i as f64 * 100.0).collect();
    let amplitudes: Vec<f64> = frequencies
        .iter()
        .map(|f| 1.0 / (1.0 + f / 1000.0))
        .collect();
    Measurement::Spectrum {
        name: name.to_string(),
        frequencies,
        amplitudes,
        frequency_unit: Some("Hz".to_string()),
        amplitude_unit: Some("dB".to_string()),
        metadata: None,
        timestamp: chrono::Utc::now(),
    }
}

/// Create an image measurement for testing
#[allow(dead_code)]
fn _create_test_image(name: &str, width: u32, height: u32) -> Measurement {
    use common::core::{ImageMetadata, PixelBuffer};

    let pixel_count = (width * height) as usize;
    let pixels: Vec<u16> = (0..pixel_count).map(|i| (i % 65536) as u16).collect();

    Measurement::Image {
        name: name.to_string(),
        width,
        height,
        buffer: PixelBuffer::U16(pixels),
        unit: "counts".to_string(),
        metadata: ImageMetadata {
            exposure_ms: Some(100.0),
            gain: Some(1.0),
            binning: Some((1, 1)),
            temperature_c: Some(-20.0),
            hardware_timestamp_us: None,
            readout_ms: Some(10.0),
            roi_origin: Some((0, 0)),
        },
        timestamp: chrono::Utc::now(),
    }
}

// =============================================================================
// HDF5 Writer Integration Tests
// =============================================================================

#[cfg(feature = "storage_hdf5")]
mod hdf5_tests {
    use super::*;
    use daq_storage::hdf5_writer::HDF5Writer;
    use daq_storage::ring_buffer::RingBuffer;
    use std::path::Path;

    #[tokio::test]
    async fn test_hdf5_write_scalar_measurements() {
        let temp_dir = TempDir::new().unwrap();
        let ring_path = temp_dir.path().join("test_ring.buf");
        let hdf5_path = temp_dir.path().join("test_scalar.h5");

        // Create ring buffer and writer
        let ring = Arc::new(RingBuffer::create(&ring_path, 10).unwrap());
        let writer = HDF5Writer::new(&hdf5_path, ring.clone()).unwrap();

        // Create test measurements
        let measurements = vec![
            create_test_scalar("power", 100.0),
            create_test_scalar("voltage", 5.0),
            create_test_scalar("temperature", 25.5),
        ];

        // Convert to Arrow and write to ring buffer
        #[cfg(feature = "storage_arrow")]
        {
            let batches = Measurement::into_arrow_batches(&measurements).unwrap();
            if let Some(batch) = batches.scalars {
                ring.write_arrow_batch(&batch).unwrap();
            }
        }

        // Flush to HDF5
        writer.flush_to_disk().await.unwrap();

        // Verify file was created
        assert!(hdf5_path.exists(), "HDF5 file should exist");

        // Verify file structure
        let file = hdf5::File::open(&hdf5_path).unwrap();
        assert!(
            file.group("measurements").is_ok(),
            "measurements group should exist"
        );
    }

    #[tokio::test]
    async fn test_hdf5_write_vector_measurements() {
        let temp_dir = TempDir::new().unwrap();
        let ring_path = temp_dir.path().join("test_ring.buf");
        let hdf5_path = temp_dir.path().join("test_vector.h5");

        let ring = Arc::new(RingBuffer::create(&ring_path, 10).unwrap());
        let writer = HDF5Writer::new(&hdf5_path, ring.clone()).unwrap();

        let measurements = vec![
            create_test_vector("waveform_1", vec![1.0, 2.0, 3.0, 4.0, 5.0]),
            create_test_vector("waveform_2", vec![5.0, 4.0, 3.0, 2.0, 1.0]),
        ];

        #[cfg(feature = "storage_arrow")]
        {
            let batches = Measurement::into_arrow_batches(&measurements).unwrap();
            if let Some(batch) = batches.vectors {
                ring.write_arrow_batch(&batch).unwrap();
            }
        }

        writer.flush_to_disk().await.unwrap();
        assert!(hdf5_path.exists(), "HDF5 file should exist");
    }

    #[tokio::test]
    async fn test_hdf5_write_spectrum_measurements() {
        let temp_dir = TempDir::new().unwrap();
        let ring_path = temp_dir.path().join("test_ring.buf");
        let hdf5_path = temp_dir.path().join("test_spectrum.h5");

        let ring = Arc::new(RingBuffer::create(&ring_path, 10).unwrap());
        let writer = HDF5Writer::new(&hdf5_path, ring.clone()).unwrap();

        let measurements = vec![
            create_test_spectrum("fft_1", 256),
            create_test_spectrum("fft_2", 512),
        ];

        #[cfg(feature = "storage_arrow")]
        {
            let batches = Measurement::into_arrow_batches(&measurements).unwrap();
            if let Some(batch) = batches.spectra {
                ring.write_arrow_batch(&batch).unwrap();
            }
        }

        writer.flush_to_disk().await.unwrap();
        assert!(hdf5_path.exists(), "HDF5 file should exist");
    }

    #[tokio::test]
    async fn test_hdf5_metadata_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let ring_path = temp_dir.path().join("test_ring.buf");
        let hdf5_path = temp_dir.path().join("test_metadata.h5");

        let ring = Arc::new(RingBuffer::create(&ring_path, 10).unwrap());
        let writer = HDF5Writer::new(&hdf5_path, ring.clone()).unwrap();

        let measurement = create_test_scalar("test_param", 42.0);

        #[cfg(feature = "storage_arrow")]
        {
            let batches = Measurement::into_arrow_batches(&[measurement]).unwrap();
            if let Some(batch) = batches.scalars {
                ring.write_arrow_batch(&batch).unwrap();
            }
        }

        writer.flush_to_disk().await.unwrap();

        // Verify metadata attributes exist
        let file = hdf5::File::open(&hdf5_path).unwrap();
        let measurements_group = file.group("measurements").unwrap();
        let batch = measurements_group.group("batch_000000").unwrap();

        // Check for metadata attributes
        assert!(
            batch.attr("ring_tail").is_ok(),
            "ring_tail attribute should exist"
        );
        assert!(
            batch.attr("timestamp_ns").is_ok(),
            "timestamp_ns attribute should exist"
        );
    }

    #[tokio::test]
    async fn test_hdf5_streaming_append() {
        let temp_dir = TempDir::new().unwrap();
        let ring_path = temp_dir.path().join("test_ring.buf");
        let hdf5_path = temp_dir.path().join("test_streaming.h5");

        let ring = Arc::new(RingBuffer::create(&ring_path, 10).unwrap());
        let writer = HDF5Writer::new(&hdf5_path, ring.clone()).unwrap();

        // Write multiple batches
        for i in 0..5 {
            let measurement = create_test_scalar(&format!("param_{}", i), i as f64);

            #[cfg(feature = "storage_arrow")]
            {
                let batches = Measurement::into_arrow_batches(&[measurement]).unwrap();
                if let Some(batch) = batches.scalars {
                    ring.write_arrow_batch(&batch).unwrap();
                }
            }

            writer.flush_to_disk().await.unwrap();
        }

        // Verify multiple batches were created
        assert_eq!(writer.batch_count(), 5, "Should have 5 batches");

        // Verify file structure
        let file = hdf5::File::open(&hdf5_path).unwrap();
        let measurements_group = file.group("measurements").unwrap();

        // Check that multiple batch groups exist
        assert!(measurements_group.group("batch_000000").is_ok());
        assert!(measurements_group.group("batch_000001").is_ok());
        assert!(measurements_group.group("batch_000002").is_ok());
        assert!(measurements_group.group("batch_000003").is_ok());
        assert!(measurements_group.group("batch_000004").is_ok());
    }

    #[tokio::test]
    async fn test_hdf5_error_handling() {
        let temp_dir = TempDir::new().unwrap();
        let ring_path = temp_dir.path().join("test_ring.buf");

        // Try to write to a read-only path (should handle gracefully)
        let hdf5_path = Path::new("/nonexistent/directory/test.h5");

        let ring = Arc::new(RingBuffer::create(&ring_path, 10).unwrap());
        let writer = HDF5Writer::new(hdf5_path, ring.clone()).unwrap();

        let measurement = create_test_scalar("test", 1.0);

        #[cfg(feature = "storage_arrow")]
        {
            let batches = Measurement::into_arrow_batches(&[measurement]).unwrap();
            if let Some(batch) = batches.scalars {
                ring.write_arrow_batch(&batch).unwrap();
            }
        }

        // Flush should return error but not panic
        let result = writer.flush_to_disk().await;
        assert!(result.is_err(), "Should return error for invalid path");
    }

    #[tokio::test]
    async fn test_hdf5_manifest_persistence() {
        use daq_experiment::document::ExperimentManifest;
        use std::collections::HashMap;

        let temp_dir = TempDir::new().unwrap();
        let ring_path = temp_dir.path().join("test_ring_manifest.buf");
        let hdf5_path = temp_dir.path().join("test_manifest.h5");

        let ring = Arc::new(RingBuffer::create(&ring_path, 10).unwrap());
        let writer = HDF5Writer::new(&hdf5_path, ring.clone()).unwrap();

        // Create a test manifest with mock device parameters
        let mut parameters = HashMap::new();
        let mut stage_params = HashMap::new();
        stage_params.insert("position".to_string(), serde_json::json!(10.5));
        stage_params.insert("velocity".to_string(), serde_json::json!(1.0));
        parameters.insert("stage1".to_string(), stage_params);

        let mut camera_params = HashMap::new();
        camera_params.insert("exposure".to_string(), serde_json::json!(0.1));
        camera_params.insert("gain".to_string(), serde_json::json!(2));
        parameters.insert("camera1".to_string(), camera_params);

        let manifest =
            ExperimentManifest::new("test-run-uid", "test_plan", "Test Plan", parameters);

        // Write manifest to HDF5
        writer.write_manifest(&manifest).await.unwrap();

        // Verify HDF5 file was created and contains manifest group
        assert!(hdf5_path.exists(), "HDF5 file should be created");

        // Verify manifest structure using hdf5 crate
        use hdf5::File;
        let file = File::open(&hdf5_path).unwrap();

        // Check manifest group exists
        assert!(
            file.group("manifest").is_ok(),
            "Manifest group should exist"
        );
        let manifest_group = file.group("manifest").unwrap();

        // Check basic attributes
        let run_uid_attr = manifest_group.attr("run_uid").unwrap();
        let run_uid: hdf5::types::VarLenUnicode = run_uid_attr.read_scalar().unwrap();
        assert_eq!(run_uid.as_str(), "test-run-uid");

        let plan_type_attr = manifest_group.attr("plan_type").unwrap();
        let plan_type: hdf5::types::VarLenUnicode = plan_type_attr.read_scalar().unwrap();
        assert_eq!(plan_type.as_str(), "test_plan");

        // Check parameters subgroup
        assert!(
            manifest_group.group("parameters").is_ok(),
            "Parameters group should exist"
        );
        let params_group = manifest_group.group("parameters").unwrap();

        // Check stage1 parameters
        assert!(
            params_group.group("stage1").is_ok(),
            "stage1 group should exist"
        );
        let stage1_group = params_group.group("stage1").unwrap();
        let position_attr = stage1_group.attr("position").unwrap();
        let position_json: hdf5::types::VarLenUnicode = position_attr.read_scalar().unwrap();
        assert!(position_json.as_str().contains("10.5"));

        // Check system info
        assert!(
            manifest_group.group("system").is_ok(),
            "System group should exist"
        );
        let system_group = manifest_group.group("system").unwrap();
        let version_attr = system_group.attr("software_version").unwrap();
        let version: hdf5::types::VarLenUnicode = version_attr.read_scalar().unwrap();
        assert!(!version.as_str().is_empty(), "Version should not be empty");
    }
}

// =============================================================================
// Arrow Writer Integration Tests
// =============================================================================

#[cfg(feature = "storage_arrow")]
mod arrow_tests {
    use super::*;
    use arrow::ipc::reader::FileReader;
    use arrow::ipc::writer::FileWriter;
    use std::fs::File;

    #[tokio::test]
    async fn test_arrow_write_scalars() {
        let temp_dir = TempDir::new().unwrap();
        let arrow_path = temp_dir.path().join("test_scalars.arrow");

        let measurements = vec![
            create_test_scalar("power", 100.0),
            create_test_scalar("voltage", 5.0),
            create_test_scalar("current", 0.5),
        ];

        let batches = Measurement::into_arrow_batches(&measurements).unwrap();
        assert!(batches.scalars.is_some(), "Should have scalar batch");

        // Write to Arrow IPC file
        let file = File::create(&arrow_path).unwrap();
        let batch = batches.scalars.unwrap();
        let mut writer = FileWriter::try_new(file, &batch.schema()).unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        // Verify file was created
        assert!(arrow_path.exists(), "Arrow file should exist");

        // Verify we can read it back
        let file = File::open(&arrow_path).unwrap();
        let reader = FileReader::try_new(file, None).unwrap();
        assert_eq!(reader.schema().fields().len(), 4, "Should have 4 fields");
    }

    #[tokio::test]
    async fn test_arrow_write_vectors() {
        let temp_dir = TempDir::new().unwrap();
        let arrow_path = temp_dir.path().join("test_vectors.arrow");

        let measurements = vec![
            create_test_vector("waveform_1", vec![1.0, 2.0, 3.0]),
            create_test_vector("waveform_2", vec![4.0, 5.0, 6.0]),
        ];

        let batches = Measurement::into_arrow_batches(&measurements).unwrap();
        assert!(batches.vectors.is_some(), "Should have vector batch");

        let file = File::create(&arrow_path).unwrap();
        let batch = batches.vectors.unwrap();
        let mut writer = FileWriter::try_new(file, &batch.schema()).unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        assert!(arrow_path.exists(), "Arrow file should exist");
    }

    #[tokio::test]
    async fn test_arrow_write_spectra() {
        let temp_dir = TempDir::new().unwrap();
        let arrow_path = temp_dir.path().join("test_spectra.arrow");

        let measurements = vec![
            create_test_spectrum("fft_1", 128),
            create_test_spectrum("fft_2", 256),
        ];

        let batches = Measurement::into_arrow_batches(&measurements).unwrap();
        assert!(batches.spectra.is_some(), "Should have spectra batch");

        let file = File::create(&arrow_path).unwrap();
        let batch = batches.spectra.unwrap();
        let mut writer = FileWriter::try_new(file, &batch.schema()).unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        assert!(arrow_path.exists(), "Arrow file should exist");
    }

    #[tokio::test]
    async fn test_arrow_metadata_in_schema() {
        let measurements = vec![create_test_scalar("test", 42.0)];

        let batches = Measurement::into_arrow_batches(&measurements).unwrap();
        let batch = batches.scalars.unwrap();

        // Check schema fields
        let schema = batch.schema();
        assert_eq!(schema.fields().len(), 4, "Should have 4 fields");

        // Verify field names
        assert_eq!(schema.field(0).name(), "name");
        assert_eq!(schema.field(1).name(), "value");
        assert_eq!(schema.field(2).name(), "unit");
        assert_eq!(schema.field(3).name(), "timestamp_ns");
    }

    #[tokio::test]
    async fn test_arrow_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let arrow_path = temp_dir.path().join("test_roundtrip.arrow");

        // Create test data
        let original_measurements = vec![
            create_test_scalar("power", 100.0),
            create_test_scalar("voltage", 5.0),
        ];

        // Write to Arrow
        let batches = Measurement::into_arrow_batches(&original_measurements).unwrap();
        let file = File::create(&arrow_path).unwrap();
        let batch = batches.scalars.unwrap();
        let mut writer = FileWriter::try_new(file, &batch.schema()).unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        // Read back from Arrow
        let file = File::open(&arrow_path).unwrap();
        let mut reader = FileReader::try_new(file, None).unwrap();
        let read_batch = reader.next().unwrap().unwrap();

        // Verify data integrity
        assert_eq!(read_batch.num_rows(), 2, "Should have 2 rows");
        assert_eq!(read_batch.num_columns(), 4, "Should have 4 columns");

        // Check values
        use arrow::array::Float64Array;
        let value_column = read_batch
            .column(1)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();

        assert_eq!(value_column.value(0), 100.0, "First value should match");
        assert_eq!(value_column.value(1), 5.0, "Second value should match");
    }

    #[tokio::test]
    async fn test_arrow_mixed_types() {
        let measurements = vec![
            create_test_scalar("power", 100.0),
            create_test_vector("waveform", vec![1.0, 2.0, 3.0]),
            create_test_spectrum("fft", 64),
        ];

        let batches = Measurement::into_arrow_batches(&measurements).unwrap();

        // Verify all types are represented
        assert!(batches.scalars.is_some(), "Should have scalars");
        assert!(batches.vectors.is_some(), "Should have vectors");
        assert!(batches.spectra.is_some(), "Should have spectra");

        // Verify counts
        assert_eq!(batches.scalars.unwrap().num_rows(), 1);
        assert_eq!(batches.vectors.unwrap().num_rows(), 1);
        assert_eq!(batches.spectra.unwrap().num_rows(), 1);
    }
}

// =============================================================================
// Ring Buffer Integration Tests
// =============================================================================

mod ringbuffer_tests {
    use super::*;
    use daq_storage::ring_buffer::RingBuffer;

    #[tokio::test]
    async fn test_ringbuffer_basic_operations() {
        let temp_dir = TempDir::new().unwrap();
        let ring_path = temp_dir.path().join("test_ring.buf");

        let ring = RingBuffer::create(&ring_path, 1).unwrap();

        // Write test data
        let test_data = b"Hello, ring buffer!";
        ring.write(test_data).unwrap();

        // Read back
        let snapshot = ring.read_snapshot();
        assert_eq!(snapshot, test_data, "Data should match");

        // Verify positions
        assert_eq!(ring.write_head(), test_data.len() as u64);
        assert_eq!(ring.read_tail(), 0);

        // Advance tail
        ring.advance_tail(snapshot.len() as u64);
        assert_eq!(ring.read_tail(), test_data.len() as u64);
    }

    #[cfg(feature = "storage_arrow")]
    #[tokio::test]
    async fn test_ringbuffer_arrow_integration() {
        let temp_dir = TempDir::new().unwrap();
        let ring_path = temp_dir.path().join("test_ring_arrow.buf");

        let ring = RingBuffer::create(&ring_path, 10).unwrap();

        // Create Arrow batch
        use arrow::array::{Float64Array, StringArray};
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow::record_batch::RecordBatch;
        use std::sync::Arc as StdArc;

        let schema = Schema::new(vec![
            Field::new("name", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]);

        let names = StringArray::from(vec!["power", "voltage", "current"]);
        let values = Float64Array::from(vec![100.0, 5.0, 0.5]);

        let batch = RecordBatch::try_new(
            StdArc::new(schema),
            vec![StdArc::new(names), StdArc::new(values)],
        )
        .unwrap();

        // Write to ring buffer
        ring.write_arrow_batch(&batch).unwrap();

        // Verify data was written
        assert!(ring.write_head() > 0, "Data should be written");

        // Read snapshot
        let snapshot = ring.read_snapshot();
        assert!(!snapshot.is_empty(), "Snapshot should not be empty");
    }

    #[tokio::test]
    async fn test_ringbuffer_circular_wrap() {
        let temp_dir = TempDir::new().unwrap();
        let ring_path = temp_dir.path().join("test_ring_wrap.buf");

        // Create small buffer to force wrapping
        let ring = RingBuffer::create(&ring_path, 1).unwrap(); // 1 MB
        let capacity = ring.capacity();

        // Write data that exceeds capacity
        let chunk_size = 512 * 1024; // 512 KB
        let test_data = vec![0xAA_u8; chunk_size];

        // Write 3 chunks (1.5 MB total, exceeds 1 MB capacity)
        for _ in 0..3 {
            ring.write(&test_data).unwrap();
        }

        // Verify buffer wrapped correctly
        let snapshot = ring.read_snapshot();
        assert!(
            snapshot.len() as u64 <= capacity,
            "Snapshot should not exceed capacity"
        );
    }

    #[tokio::test]
    async fn test_ringbuffer_concurrent_access() {
        let temp_dir = TempDir::new().unwrap();
        let ring_path = temp_dir.path().join("test_ring_concurrent.buf");

        let ring = Arc::new(RingBuffer::create(&ring_path, 10).unwrap());

        // Spawn writer task
        let ring_writer = ring.clone();
        let writer = tokio::spawn(async move {
            for i in 0..100 {
                let data = format!("Message {}", i);
                ring_writer.write(data.as_bytes()).unwrap();
                tokio::time::sleep(tokio::time::Duration::from_micros(100)).await;
            }
        });

        // Spawn reader task
        let ring_reader = ring.clone();
        let reader = tokio::spawn(async move {
            let mut read_count = 0;
            while read_count < 50 {
                let snapshot = ring_reader.read_snapshot();
                if !snapshot.is_empty() {
                    read_count += 1;
                    ring_reader.advance_tail(snapshot.len() as u64);
                }
                tokio::time::sleep(tokio::time::Duration::from_micros(500)).await;
            }
        });

        // Wait for both tasks
        writer.await.unwrap();
        reader.await.unwrap();
    }
}

// =============================================================================
// Ring Buffer to HDF5 Integration Tests
// =============================================================================

#[cfg(feature = "storage_hdf5")]
mod ringbuffer_hdf5_integration {
    use super::*;
    use daq_storage::hdf5_writer::HDF5Writer;
    use daq_storage::ring_buffer::RingBuffer;

    #[tokio::test]
    async fn test_ringbuffer_to_hdf5_flow() {
        let temp_dir = TempDir::new().unwrap();
        let ring_path = temp_dir.path().join("test_ring_hdf5.buf");
        let hdf5_path = temp_dir.path().join("test_flow.h5");

        // Create ring buffer and HDF5 writer
        let ring = Arc::new(RingBuffer::create(&ring_path, 10).unwrap());
        let writer = HDF5Writer::new(&hdf5_path, ring.clone()).unwrap();

        // Write measurements to ring buffer
        #[cfg(feature = "storage_arrow")]
        {
            let measurements = vec![
                create_test_scalar("sensor_1", 42.0),
                create_test_scalar("sensor_2", 84.0),
            ];

            let batches = Measurement::into_arrow_batches(&measurements).unwrap();
            if let Some(batch) = batches.scalars {
                ring.write_arrow_batch(&batch).unwrap();
            }
        }

        // Flush to HDF5
        writer.flush_to_disk().await.unwrap();

        // Verify HDF5 file was created and contains data
        assert!(hdf5_path.exists(), "HDF5 file should exist");
        assert!(writer.batch_count() > 0, "Should have written batches");

        // Verify ring buffer tail was advanced
        assert!(ring.read_tail() > 0, "Ring buffer tail should advance");
    }

    #[tokio::test]
    async fn test_high_throughput_pipeline() {
        let temp_dir = TempDir::new().unwrap();
        let ring_path = temp_dir.path().join("test_ring_throughput.buf");
        let hdf5_path = temp_dir.path().join("test_throughput.h5");

        let ring = Arc::new(RingBuffer::create(&ring_path, 100).unwrap()); // 100 MB buffer
        let writer = HDF5Writer::new(&hdf5_path, ring.clone()).unwrap();

        // Write 1000 measurements
        #[cfg(feature = "storage_arrow")]
        {
            for batch_num in 0..10 {
                let mut batch_measurements = Vec::new();
                for i in 0..100 {
                    let name = format!("measurement_{}_{}", batch_num, i);
                    batch_measurements.push(create_test_scalar(&name, i as f64));
                }

                let batches = Measurement::into_arrow_batches(&batch_measurements).unwrap();
                if let Some(batch) = batches.scalars {
                    ring.write_arrow_batch(&batch).unwrap();
                }

                // Flush periodically
                if batch_num % 2 == 0 {
                    writer.flush_to_disk().await.unwrap();
                }
            }
        }

        // Final flush
        writer.flush_to_disk().await.unwrap();

        // Verify throughput
        assert!(writer.batch_count() >= 5, "Should have multiple batches");
        assert!(hdf5_path.exists(), "HDF5 file should exist");
    }

    #[tokio::test]
    async fn test_background_writer_async() {
        let temp_dir = TempDir::new().unwrap();
        let ring_path = temp_dir.path().join("test_ring_async.buf");
        let hdf5_path = temp_dir.path().join("test_async.h5");

        let ring = Arc::new(RingBuffer::create(&ring_path, 10).unwrap());
        let writer = Arc::new(HDF5Writer::new(&hdf5_path, ring.clone()).unwrap());

        // Spawn background writer task
        let writer_clone = writer.clone();
        let writer_task = tokio::spawn(async move {
            // Run for a short time
            for _ in 0..5 {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                let _ = writer_clone.flush_to_disk().await;
            }
        });

        // Write data while background task is running
        #[cfg(feature = "storage_arrow")]
        {
            for i in 0..10 {
                let measurement = create_test_scalar(&format!("async_test_{}", i), i as f64);
                let batches = Measurement::into_arrow_batches(&[measurement]).unwrap();
                if let Some(batch) = batches.scalars {
                    ring.write_arrow_batch(&batch).unwrap();
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }
        }

        // Wait for background task
        writer_task.await.unwrap();

        // Verify writes occurred
        assert!(
            writer.batch_count() > 0,
            "Background writer should have written batches"
        );
    }
}

// =============================================================================
// Performance Benchmarks
// =============================================================================

#[cfg(all(test, feature = "storage_arrow"))]
mod performance_tests {
    use super::*;
    use daq_storage::ring_buffer::RingBuffer;

    #[tokio::test]
    async fn test_arrow_batch_creation_performance() {
        let start = std::time::Instant::now();
        let iterations = 1000;

        for _ in 0..iterations {
            let measurements = vec![create_test_scalar("test", 42.0)];
            let _batches = Measurement::into_arrow_batches(&measurements).unwrap();
        }

        let elapsed = start.elapsed();
        let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();

        println!("Arrow batch creation: {:.0} ops/sec", ops_per_sec);
        assert!(ops_per_sec > 1000.0, "Should create batches quickly");
    }

    #[tokio::test]
    async fn test_ringbuffer_write_performance() {
        let temp_dir = TempDir::new().unwrap();
        let ring_path = temp_dir.path().join("test_ring_perf.buf");

        let ring = RingBuffer::create(&ring_path, 100).unwrap();
        let test_data = vec![0u8; 1024]; // 1 KB per write

        let start = std::time::Instant::now();
        let iterations = 10_000;

        for _ in 0..iterations {
            ring.write(&test_data).unwrap();
        }

        let elapsed = start.elapsed();
        let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();

        println!("Ring buffer write: {:.0} ops/sec", ops_per_sec);
        assert!(ops_per_sec > 5_000.0, "Should write quickly");
    }
}
