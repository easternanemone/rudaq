// =============================================================================
// HDF5 Writer Integration Tests
// =============================================================================

use super::helpers::*;
use common::core::Measurement;
use std::path::Path;
use std::sync::Arc;
use storage::hdf5_writer::HDF5Writer;
use storage::ring_buffer::RingBuffer;
use tempfile::TempDir;

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
    use common::experiment::document::ExperimentManifest;
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

    let manifest = ExperimentManifest::new("test-run-uid", "test_plan", "Test Plan", parameters);

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
