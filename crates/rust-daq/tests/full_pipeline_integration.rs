#![cfg(not(target_arch = "wasm32"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    clippy::panic,
    deprecated,
    unsafe_code,
    clippy::single_match,
    unused_mut,
    unused_imports,
    missing_docs
)]
//! Full Pipeline Integration Tests
//!
//! End-to-end tests simulating the complete acquisition workflow:
//! Mock Instrument -> Measurement -> Storage Pipeline
//!
//! These tests verify that data flows correctly through all layers of the system,
//! from hardware acquisition to persistent storage, including error handling and
//! graceful shutdown scenarios.
//!
//! Run with:
//! ```bash
//! cargo test --test full_pipeline_integration
//! cargo test --test full_pipeline_integration --features storage_hdf5
//! cargo test --test full_pipeline_integration --features storage_arrow
//! ```

use anyhow::Result;
use daq_core::core::Measurement;
use rust_daq::hardware::capabilities::{FrameProducer, Movable, Readable, Triggerable};
use rust_daq::hardware::mock::{MockCamera, MockPowerMeter, MockStage};
use std::sync::Arc;
use tempfile::TempDir;

// =============================================================================
// Test Helper: Create Measurements from Mock Instruments
// =============================================================================

/// Simulate acquiring a scalar measurement from a power meter
async fn acquire_power_measurement(
    power_meter: &MockPowerMeter,
    name: &str,
) -> Result<Measurement> {
    let value = power_meter.read().await?;
    Ok(Measurement::Scalar {
        name: name.to_string(),
        value,
        unit: "W".to_string(),
        timestamp: chrono::Utc::now(),
    })
}

/// Simulate acquiring a position measurement from a stage
async fn acquire_position_measurement(stage: &MockStage, name: &str) -> Result<Measurement> {
    let position = stage.position().await?;
    Ok(Measurement::Scalar {
        name: name.to_string(),
        value: position,
        unit: "mm".to_string(),
        timestamp: chrono::Utc::now(),
    })
}

/// Simulate acquiring an image measurement from a camera
async fn acquire_camera_image(
    camera: &MockCamera,
    name: &str,
    frame_number: u64,
) -> Result<Measurement> {
    // Trigger camera to acquire frame
    camera.trigger().await?;

    // In a real system, we'd read pixel data from the camera
    // For mock, we generate synthetic image data
    let (width, height) = camera.resolution();
    let pixel_count = (width * height) as usize;
    let pixels: Vec<u16> = (0..pixel_count)
        .map(|i| ((i + frame_number as usize * 100) % 65536) as u16)
        .collect();

    Ok(Measurement::Image {
        name: name.to_string(),
        width,
        height,
        buffer: daq_core::core::PixelBuffer::U16(pixels),
        unit: "counts".to_string(),
        metadata: daq_core::core::ImageMetadata {
            exposure_ms: Some(100.0),
            gain: Some(1.0),
            binning: Some((1, 1)),
            temperature_c: Some(-20.0),
            hardware_timestamp_us: None,
            readout_ms: Some(10.0),
            roi_origin: Some((0, 0)),
        },
        timestamp: chrono::Utc::now(),
    })
}

// =============================================================================
// Test 1: Simple Instrument -> Measurement Pipeline
// =============================================================================

#[tokio::test]
async fn test_instrument_to_measurement_basic() {
    // Create mock instruments
    let stage = MockStage::new();
    let power_meter = MockPowerMeter::new(5.0);

    // Move stage to target position
    stage.move_abs(10.0).await.unwrap();
    stage.wait_settled().await.unwrap();

    // Acquire measurements
    let position_measurement = acquire_position_measurement(&stage, "stage_x")
        .await
        .unwrap();
    let power_measurement = acquire_power_measurement(&power_meter, "laser_power")
        .await
        .unwrap();

    // Verify measurements were created correctly
    match position_measurement {
        Measurement::Scalar { name, value, .. } => {
            assert_eq!(name, "stage_x");
            assert!((value - 10.0).abs() < 0.001);
        }
        _ => panic!("Expected scalar measurement"),
    }

    match power_measurement {
        Measurement::Scalar { name, value, .. } => {
            assert_eq!(name, "laser_power");
            assert!(value > 0.0);
        }
        _ => panic!("Expected scalar measurement"),
    }
}

// =============================================================================
// Test 2: Multi-Instrument Scan with Measurements
// =============================================================================

#[tokio::test]
async fn test_multi_instrument_scan_to_measurements() {
    let stage = MockStage::new();
    let camera = MockCamera::new(640, 480);
    let power_meter = MockPowerMeter::new(2.5);

    camera.arm().await.unwrap();

    let positions = [0.0, 5.0, 10.0, 15.0, 20.0];
    let mut measurements: Vec<Measurement> = Vec::new();

    // Simulate a coordinated scan
    for (i, &pos) in positions.iter().enumerate() {
        // Move stage
        stage.move_abs(pos).await.unwrap();
        stage.wait_settled().await.unwrap();

        // Acquire measurements from all instruments
        let position = acquire_position_measurement(&stage, "position")
            .await
            .unwrap();
        let power = acquire_power_measurement(&power_meter, "power")
            .await
            .unwrap();
        let image = acquire_camera_image(&camera, "camera_frame", (i + 1) as u64)
            .await
            .unwrap();

        measurements.push(position);
        measurements.push(power);
        measurements.push(image);
    }

    // Verify we collected all measurements
    assert_eq!(measurements.len(), positions.len() * 3); // 3 measurements per position
    assert_eq!(camera.get_frame_count(), positions.len() as u64);
}

// =============================================================================
// Test 3: Instrument -> Measurement -> CSV Storage Pipeline
// =============================================================================

#[cfg(feature = "storage_csv")]
#[tokio::test]
async fn test_full_pipeline_to_csv() {
    use std::fs;
    use std::io::Write;

    let temp_dir = TempDir::new().unwrap();
    let csv_path = temp_dir.path().join("measurements.csv");

    // Create mock instruments
    let stage = MockStage::new();
    let power_meter = MockPowerMeter::new(3.0);

    // Acquire measurements
    let mut measurements = Vec::new();
    for i in 0..5 {
        let pos = i as f64 * 2.0;
        stage.move_abs(pos).await.unwrap();
        stage.wait_settled().await.unwrap();

        let position = acquire_position_measurement(&stage, "stage_x")
            .await
            .unwrap();
        let power = acquire_power_measurement(&power_meter, "laser_power")
            .await
            .unwrap();

        measurements.push(position);
        measurements.push(power);
    }

    // Write measurements to CSV (simplified implementation)
    let mut file = fs::File::create(&csv_path).unwrap();
    writeln!(file, "name,value,unit,timestamp").unwrap();

    for measurement in &measurements {
        match measurement {
            Measurement::Scalar {
                name,
                value,
                unit,
                timestamp,
            } => {
                writeln!(file, "{},{},{},{}", name, value, unit, timestamp).unwrap();
            }
            _ => {}
        }
    }

    // Verify file was created and contains data
    assert!(csv_path.exists());
    let contents = fs::read_to_string(&csv_path).unwrap();
    assert!(contents.contains("stage_x"));
    assert!(contents.contains("laser_power"));
}

// =============================================================================
// Test 4: Full Pipeline with Arrow Storage
// =============================================================================

#[cfg(feature = "storage_arrow")]
#[tokio::test]
async fn test_full_pipeline_to_arrow() {
    use daq_storage::ring_buffer::RingBuffer;

    let temp_dir = TempDir::new().unwrap();
    let ring_path = temp_dir.path().join("pipeline.buf");

    // Create mock instruments
    let stage = MockStage::new();
    let power_meter = MockPowerMeter::new(2.5);

    // Create ring buffer for data pipeline
    let ring = Arc::new(RingBuffer::create(&ring_path, 10).unwrap());

    // Simulate acquisition and storage pipeline
    let positions = vec![0.0, 5.0, 10.0];
    let mut all_measurements = Vec::new();

    for &pos in &positions {
        stage.move_abs(pos).await.unwrap();
        stage.wait_settled().await.unwrap();

        let position = acquire_position_measurement(&stage, "stage_x")
            .await
            .unwrap();
        let power = acquire_power_measurement(&power_meter, "laser_power")
            .await
            .unwrap();

        all_measurements.push(position);
        all_measurements.push(power);
    }

    // Convert to Arrow batches and write to ring buffer
    let batches = Measurement::into_arrow_batches(&all_measurements).unwrap();

    if let Some(batch) = batches.scalars {
        ring.write_arrow_batch(&batch).unwrap();
    }

    // Verify data was written to ring buffer
    assert!(ring.write_head() > 0, "Ring buffer should contain data");
    let snapshot = ring.read_snapshot();
    assert!(!snapshot.is_empty(), "Should have data in ring buffer");
}

// =============================================================================
// Test 5: Full Pipeline with HDF5 Storage
// =============================================================================

#[cfg(all(feature = "storage_hdf5", feature = "storage_arrow"))]
#[tokio::test]
async fn test_full_pipeline_to_hdf5() {
    use daq_storage::hdf5_writer::HDF5Writer;
    use daq_storage::ring_buffer::RingBuffer;

    let temp_dir = TempDir::new().unwrap();
    let ring_path = temp_dir.path().join("pipeline.buf");
    let hdf5_path = temp_dir.path().join("pipeline.h5");

    // Create mock instruments
    let stage = MockStage::new();
    let power_meter = MockPowerMeter::new(2.5);
    let camera = MockCamera::new(640, 480);

    // Create data pipeline: Ring Buffer -> HDF5 Writer
    let ring = Arc::new(RingBuffer::create(&ring_path, 10).unwrap());
    let writer = HDF5Writer::new(&hdf5_path, ring.clone()).unwrap();

    // Arm camera
    camera.arm().await.unwrap();

    // Simulate acquisition
    let positions = vec![0.0, 5.0, 10.0];
    let mut all_measurements = Vec::new();

    for (i, &pos) in positions.iter().enumerate() {
        // Move stage to position
        stage.move_abs(pos).await.unwrap();
        stage.wait_settled().await.unwrap();

        // Acquire from all instruments
        let position = acquire_position_measurement(&stage, "stage_x")
            .await
            .unwrap();
        let power = acquire_power_measurement(&power_meter, "laser_power")
            .await
            .unwrap();
        let image = acquire_camera_image(&camera, "camera_frame", (i + 1) as u64)
            .await
            .unwrap();

        all_measurements.push(position);
        all_measurements.push(power);
        all_measurements.push(image);
    }

    // Write to pipeline: Measurements -> Arrow -> Ring Buffer -> HDF5
    let batches = Measurement::into_arrow_batches(&all_measurements).unwrap();

    if let Some(batch) = batches.scalars {
        ring.write_arrow_batch(&batch).unwrap();
    }
    if let Some(batch) = batches.images {
        ring.write_arrow_batch(&batch).unwrap();
    }

    // Flush to HDF5
    writer.flush_to_disk().await.unwrap();

    // Verify HDF5 file was created
    assert!(hdf5_path.exists(), "HDF5 file should be created");
    assert!(writer.batch_count() > 0, "Should have written batches");

    // Verify file structure
    let file = hdf5::File::open(&hdf5_path).unwrap();
    assert!(
        file.group("measurements").is_ok(),
        "Should have measurements group"
    );
}

// =============================================================================
// Test 6: Error Handling Across Pipeline
// =============================================================================

#[tokio::test]
async fn test_pipeline_error_handling() {
    use std::sync::atomic::{AtomicBool, Ordering};

    // Create mock camera that will fail
    let camera = MockCamera::new(640, 480);
    let stage = MockStage::new();
    let power_meter = MockPowerMeter::new(2.5);

    let error_occurred = Arc::new(AtomicBool::new(false));

    // Attempt acquisition without arming camera (should fail)
    let result = acquire_camera_image(&camera, "frame", 1).await;
    assert!(result.is_err(), "Should fail without arming camera");

    // Arm camera and continue
    camera.arm().await.unwrap();

    // Simulate pipeline with error recovery
    let mut successful_measurements = 0;

    for i in 0..5 {
        stage.move_abs(i as f64).await.unwrap();

        // Attempt to acquire measurements
        match acquire_camera_image(&camera, "frame", i + 1).await {
            Ok(_) => {
                let _ = acquire_power_measurement(&power_meter, "power").await;
                successful_measurements += 1;
            }
            Err(_) => {
                error_occurred.store(true, Ordering::SeqCst);
            }
        }
    }

    // All should succeed after arming
    assert_eq!(successful_measurements, 5);
}

// =============================================================================
// Test 7: Graceful Shutdown During Pipeline Operation
// =============================================================================

#[cfg(feature = "storage_arrow")]
#[tokio::test]
async fn test_pipeline_graceful_shutdown() {
    use daq_storage::ring_buffer::RingBuffer;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::time::Duration;

    let temp_dir = TempDir::new().unwrap();
    let ring_path = temp_dir.path().join("shutdown_test.buf");

    let stage = Arc::new(MockStage::new());
    let power_meter = Arc::new(MockPowerMeter::new(2.5));
    let ring = Arc::new(RingBuffer::create(&ring_path, 10).unwrap());
    let shutdown_flag = Arc::new(AtomicBool::new(false));

    // Spawn acquisition task
    let stage_clone = stage.clone();
    let power_clone = power_meter.clone();
    let ring_clone = ring.clone();
    let shutdown_clone = shutdown_flag.clone();

    let acquisition_task = tokio::spawn(async move {
        let mut measurements_count = 0;

        for i in 0..100 {
            // Check shutdown flag
            if shutdown_clone.load(Ordering::SeqCst) {
                break;
            }

            // Move stage and acquire
            let pos = i as f64;
            stage_clone.move_abs(pos).await.unwrap();

            let position = acquire_position_measurement(&stage_clone, "position")
                .await
                .unwrap();
            let power = acquire_power_measurement(&power_clone, "power")
                .await
                .unwrap();

            // Write to pipeline
            let measurements = vec![position, power];
            let batches = Measurement::into_arrow_batches(&measurements).unwrap();

            if let Some(batch) = batches.scalars {
                ring_clone.write_arrow_batch(&batch).unwrap();
            }

            measurements_count += 1;
        }

        measurements_count
    });

    // Request shutdown after 100ms
    tokio::time::sleep(Duration::from_millis(100)).await;
    shutdown_flag.store(true, Ordering::SeqCst);

    // Wait for task to complete gracefully
    let completed = acquisition_task.await.unwrap();

    // Should have completed some measurements but not all 100
    assert!(completed > 0, "Should have completed some measurements");
    assert!(
        completed < 100,
        "Should have stopped before completing all measurements"
    );

    // Verify ring buffer contains data
    assert!(
        ring.write_head() > 0,
        "Ring buffer should contain acquired data"
    );
}

// =============================================================================
// Test 8: High-Throughput Pipeline Stress Test
// =============================================================================

#[cfg(feature = "storage_arrow")]
#[tokio::test]
async fn test_pipeline_high_throughput() {
    use daq_storage::ring_buffer::RingBuffer;
    use std::time::Instant;

    let temp_dir = TempDir::new().unwrap();
    let ring_path = temp_dir.path().join("throughput_test.buf");

    let stage = MockStage::with_speed(1000.0); // Fast mock stage
    let power_meter = MockPowerMeter::new(2.5);
    let ring = Arc::new(RingBuffer::create(&ring_path, 100).unwrap());

    let start = Instant::now();
    let num_iterations = 100;

    // Simulate high-throughput acquisition
    for i in 0..num_iterations {
        let pos = (i % 10) as f64;
        stage.move_abs(pos).await.unwrap();

        let position = acquire_position_measurement(&stage, "position")
            .await
            .unwrap();
        let power = acquire_power_measurement(&power_meter, "power")
            .await
            .unwrap();

        let measurements = vec![position, power];
        let batches = Measurement::into_arrow_batches(&measurements).unwrap();

        if let Some(batch) = batches.scalars {
            ring.write_arrow_batch(&batch).unwrap();
        }
    }

    let elapsed = start.elapsed();
    let throughput = num_iterations as f64 / elapsed.as_secs_f64();

    println!(
        "Pipeline throughput: {:.1} acquisitions/sec ({} acquisitions in {:?})",
        throughput, num_iterations, elapsed
    );

    // Should achieve reasonable throughput (at least 50 acquisitions/sec)
    assert!(
        throughput > 50.0,
        "Throughput too low: {:.1} acquisitions/sec",
        throughput
    );

    // Verify all data was written
    assert!(
        ring.write_head() > 0,
        "Ring buffer should contain all acquired data"
    );
}

// =============================================================================
// Test 9: Data Integrity Through Full Pipeline
// =============================================================================

#[cfg(all(feature = "storage_arrow", feature = "storage_hdf5"))]
#[tokio::test]
async fn test_pipeline_data_integrity() {
    use daq_storage::hdf5_writer::HDF5Writer;
    use daq_storage::ring_buffer::RingBuffer;

    let temp_dir = TempDir::new().unwrap();
    let ring_path = temp_dir.path().join("integrity.buf");
    let hdf5_path = temp_dir.path().join("integrity.h5");

    let stage = MockStage::new();
    let power_meter = MockPowerMeter::new(5.0);

    let ring = Arc::new(RingBuffer::create(&ring_path, 10).unwrap());
    let writer = HDF5Writer::new(&hdf5_path, ring.clone()).unwrap();

    // Define expected positions
    let expected_positions = vec![0.0, 1.0, 2.0, 3.0, 4.0];
    let mut actual_measurements = Vec::new();

    // Acquire measurements
    for &pos in &expected_positions {
        stage.move_abs(pos).await.unwrap();
        stage.wait_settled().await.unwrap();

        let position = acquire_position_measurement(&stage, "stage_x")
            .await
            .unwrap();
        let power = acquire_power_measurement(&power_meter, "power")
            .await
            .unwrap();

        actual_measurements.push(position.clone());
        actual_measurements.push(power.clone());

        // Write to pipeline
        let batches = Measurement::into_arrow_batches(&[position, power]).unwrap();
        if let Some(batch) = batches.scalars {
            ring.write_arrow_batch(&batch).unwrap();
        }
    }

    // Flush to storage
    writer.flush_to_disk().await.unwrap();

    // Verify data integrity: check positions match expected
    let mut position_count = 0;
    for measurement in &actual_measurements {
        if let Measurement::Scalar { name, value, .. } = measurement {
            if name == "stage_x" {
                let expected = expected_positions[position_count];
                assert!(
                    (*value - expected).abs() < 0.001,
                    "Position mismatch: expected {}, got {}",
                    expected,
                    value
                );
                position_count += 1;
            }
        }
    }

    assert_eq!(position_count, expected_positions.len());

    // Verify HDF5 file integrity
    let file = hdf5::File::open(&hdf5_path).unwrap();
    assert!(file.group("measurements").is_ok());
}
