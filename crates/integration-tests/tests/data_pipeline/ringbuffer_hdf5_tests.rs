// =============================================================================
// Ring Buffer to HDF5 Integration Tests
// =============================================================================

use super::helpers::*;
use common::core::Measurement;
use std::sync::Arc;
use storage::hdf5_writer::HDF5Writer;
use storage::ring_buffer::RingBuffer;
use tempfile::TempDir;

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
                let name = format!("measurement_{batch_num}_{i}");
                batch_measurements.push(create_test_scalar(&name, f64::from(i)));
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
            let measurement = create_test_scalar(&format!("async_test_{i}"), f64::from(i));
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
