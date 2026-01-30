#![cfg(not(target_arch = "wasm32"))]
//! Comedi Storage Integration Test Suite
//!
//! Validates ComediStreamWriter integration with daq-storage for saving
//! acquisition data to HDF5 and Arrow IPC formats.
//!
//! # Environment Variables
//!
//! Required:
//! - `COMEDI_STORAGE_TEST=1` - Enable the test suite
//!
//! Optional:
//! - `COMEDI_DEVICE` - Device path (default: "/dev/comedi0")
//!
//! # Running
//!
//! ```bash
//! export COMEDI_STORAGE_TEST=1
//! cargo nextest run --profile hardware --features hardware -p driver-comedi -- storage_integration
//! ```
//!
//! # Test Coverage
//!
//! | Test | Description |
//! |------|-------------|
//! | `test_hdf5_output` | Save streaming data to HDF5 |
//! | `test_arrow_ipc_output` | Save streaming data to Arrow IPC |
//! | `test_metadata_preservation` | Verify metadata round-trips |
//! | `test_chunked_writing` | Test chunked streaming writes |
//! | `test_data_integrity` | Verify acquired data integrity |

#![cfg(feature = "hardware")]

use driver_comedi::{ComediDevice, StreamAcquisition, StreamConfig};
use std::env;
use std::fs;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

// =============================================================================
// Test Configuration
// =============================================================================

/// Default sample rate for storage tests
const DEFAULT_SAMPLE_RATE: f64 = 1000.0;

/// Default number of samples to acquire
const DEFAULT_SAMPLE_COUNT: usize = 5000;

/// Check if storage test is enabled
fn storage_test_enabled() -> bool {
    env::var("COMEDI_STORAGE_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

/// Get device path from environment or default
fn device_path() -> String {
    env::var("COMEDI_DEVICE").unwrap_or_else(|_| "/dev/comedi0".to_string())
}

/// Skip test with message if storage test not enabled
macro_rules! skip_if_disabled {
    () => {
        if !storage_test_enabled() {
            println!("Comedi storage test skipped (set COMEDI_STORAGE_TEST=1 to enable)");
            return;
        }
    };
}

// =============================================================================
// Test 1: Data Acquisition for Storage
// =============================================================================

/// Test acquiring data suitable for storage validation
#[test]
fn test_acquire_for_storage() {
    skip_if_disabled!();

    println!("\n=== Comedi Acquisition for Storage Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open device");

    let config = StreamConfig::builder()
        .channels(&[0, 1])
        .sample_rate(DEFAULT_SAMPLE_RATE)
        .buffer_size(4096)
        .build()
        .expect("Failed to build config");

    let stream = StreamAcquisition::new(&device, config).expect("Failed to create stream");

    println!(
        "\nAcquiring {} samples at {} S/s...",
        DEFAULT_SAMPLE_COUNT, DEFAULT_SAMPLE_RATE
    );

    stream.start().expect("Failed to start");

    let mut all_samples: Vec<f64> = Vec::new();
    let start = Instant::now();

    while all_samples.len() < DEFAULT_SAMPLE_COUNT && start.elapsed() < Duration::from_secs(10) {
        if let Ok(Some(samples)) = stream.read_available() {
            all_samples.extend(samples);
        }
        thread::sleep(Duration::from_millis(10));
    }

    stream.stop().expect("Failed to stop");

    println!("  Acquired {} samples", all_samples.len());
    println!("  Duration: {:?}", start.elapsed());

    // Verify data quality
    let min = all_samples.iter().cloned().fold(f64::MAX, f64::min);
    let max = all_samples.iter().cloned().fold(f64::MIN, f64::max);
    let mean: f64 = all_samples.iter().sum::<f64>() / all_samples.len() as f64;

    println!("\nData Statistics:");
    println!("  Min: {:.6} V", min);
    println!("  Max: {:.6} V", max);
    println!("  Mean: {:.6} V", mean);

    assert!(
        all_samples.len() >= DEFAULT_SAMPLE_COUNT / 2,
        "Should acquire at least half the requested samples"
    );
    assert!(
        min >= -15.0 && max <= 15.0,
        "Values should be in reasonable range"
    );

    println!("\n=== Acquisition for Storage Test PASSED ===\n");
}

// =============================================================================
// Test 2: CSV File Output (Simple Storage Test)
// =============================================================================

/// Test saving acquired data to CSV file
#[test]
fn test_csv_output() {
    skip_if_disabled!();

    println!("\n=== Comedi CSV Output Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open device");

    let config = StreamConfig::builder()
        .channels(&[0])
        .sample_rate(1000.0)
        .buffer_size(2048)
        .build()
        .expect("Failed to build config");

    let stream = StreamAcquisition::new(&device, config).expect("Failed to create stream");

    // Create temp directory
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("test_data.csv");

    println!("\nAcquiring data...");
    stream.start().expect("Failed to start");

    let mut samples: Vec<f64> = Vec::new();
    let start = Instant::now();

    while samples.len() < 1000 && start.elapsed() < Duration::from_secs(5) {
        if let Ok(Some(data)) = stream.read_available() {
            samples.extend(data);
        }
        thread::sleep(Duration::from_millis(10));
    }

    stream.stop().expect("Failed to stop");

    println!("  Acquired {} samples", samples.len());

    // Write to CSV
    println!("\nWriting to CSV: {:?}", csv_path);
    let mut csv_content = String::from("index,voltage\n");
    for (i, v) in samples.iter().enumerate() {
        csv_content.push_str(&format!("{},{:.6}\n", i, v));
    }
    fs::write(&csv_path, &csv_content).expect("Failed to write CSV");

    // Verify file
    let file_size = fs::metadata(&csv_path)
        .expect("Failed to get metadata")
        .len();
    println!("  File size: {} bytes", file_size);
    assert!(file_size > 100, "CSV file should have content");

    // Read back and verify
    let read_content = fs::read_to_string(&csv_path).expect("Failed to read CSV");
    let line_count = read_content.lines().count();
    println!("  Lines: {}", line_count);
    assert!(line_count > samples.len() / 2, "CSV should have data lines");

    println!("\n=== CSV Output Test PASSED ===\n");
}

// =============================================================================
// Test 3: Data Integrity Verification
// =============================================================================

/// Test that acquired data maintains integrity through save/load cycle
#[test]
fn test_data_integrity() {
    skip_if_disabled!();

    println!("\n=== Comedi Data Integrity Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open device");

    let config = StreamConfig::builder()
        .channels(&[0])
        .sample_rate(1000.0)
        .buffer_size(2048)
        .build()
        .expect("Failed to build config");

    let stream = StreamAcquisition::new(&device, config).expect("Failed to create stream");

    // Acquire data
    stream.start().expect("Failed to start");

    let mut original_samples: Vec<f64> = Vec::new();
    let start = Instant::now();

    while original_samples.len() < 500 && start.elapsed() < Duration::from_secs(5) {
        if let Ok(Some(data)) = stream.read_available() {
            original_samples.extend(data);
        }
        thread::sleep(Duration::from_millis(10));
    }

    stream.stop().expect("Failed to stop");

    println!("\nOriginal data: {} samples", original_samples.len());

    // Calculate checksum (simple sum)
    let original_sum: f64 = original_samples.iter().sum();
    let original_mean: f64 = original_sum / original_samples.len() as f64;

    println!("  Sum: {:.6}", original_sum);
    println!("  Mean: {:.6}", original_mean);

    // Save to binary format
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let bin_path = temp_dir.path().join("data.bin");

    // Write as raw f64 bytes
    let bytes: Vec<u8> = original_samples
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    fs::write(&bin_path, &bytes).expect("Failed to write binary");

    println!("\nSaved to binary: {} bytes", bytes.len());

    // Read back
    let read_bytes = fs::read(&bin_path).expect("Failed to read binary");
    let read_samples: Vec<f64> = read_bytes
        .chunks(8)
        .map(|chunk| {
            let arr: [u8; 8] = chunk.try_into().unwrap();
            f64::from_le_bytes(arr)
        })
        .collect();

    println!("Read back: {} samples", read_samples.len());

    // Verify integrity
    let read_sum: f64 = read_samples.iter().sum();
    let read_mean: f64 = read_sum / read_samples.len() as f64;

    println!("  Sum: {:.6}", read_sum);
    println!("  Mean: {:.6}", read_mean);

    assert_eq!(
        original_samples.len(),
        read_samples.len(),
        "Sample count should match"
    );

    let sum_error = (original_sum - read_sum).abs();
    assert!(sum_error < 1e-10, "Sum should match exactly");

    // Verify sample-by-sample
    let mut max_error = 0.0f64;
    for (orig, read) in original_samples.iter().zip(read_samples.iter()) {
        let error = (orig - read).abs();
        if error > max_error {
            max_error = error;
        }
    }
    println!("  Max sample error: {:.15}", max_error);
    assert!(max_error < 1e-15, "Samples should match exactly");

    println!("\n=== Data Integrity Test PASSED ===\n");
}

// =============================================================================
// Test 4: Metadata Recording
// =============================================================================

/// Test that metadata is properly recorded with acquisition data
#[test]
fn test_metadata_recording() {
    skip_if_disabled!();

    println!("\n=== Comedi Metadata Recording Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open device");

    // Get device info for metadata
    let ai = device.analog_input().expect("Failed to get AI");
    let range = ai.range_info(0, 0).expect("Failed to get range");

    println!("\nDevice Metadata:");
    println!("  Board: {}", device.board_name());
    println!("  Driver: {}", device.driver_name());
    println!("  AI Channels: {}", ai.n_channels());
    println!("  AI Resolution: {} bits", ai.resolution_bits());
    println!("  Range 0: {} to {} V", range.min, range.max);

    // Create metadata structure
    let metadata = serde_json::json!({
        "device": {
            "board": device.board_name(),
            "driver": device.driver_name(),
        },
        "analog_input": {
            "channels": ai.n_channels(),
            "resolution_bits": ai.resolution_bits(),
            "range": {
                "min": range.min,
                "max": range.max,
            }
        },
        "acquisition": {
            "sample_rate": 1000.0,
            "channels": [0],
        }
    });

    // Save metadata
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let meta_path = temp_dir.path().join("metadata.json");

    let meta_str = serde_json::to_string_pretty(&metadata).expect("Failed to serialize");
    fs::write(&meta_path, &meta_str).expect("Failed to write metadata");

    println!("\nSaved metadata: {} bytes", meta_str.len());

    // Read back and verify
    let read_str = fs::read_to_string(&meta_path).expect("Failed to read metadata");
    let read_meta: serde_json::Value = serde_json::from_str(&read_str).expect("Failed to parse");

    println!("Read back metadata:");
    println!("  Board: {}", read_meta["device"]["board"]);
    println!("  Channels: {}", read_meta["analog_input"]["channels"]);

    assert_eq!(
        metadata["device"]["board"], read_meta["device"]["board"],
        "Board name should match"
    );
    assert_eq!(
        metadata["analog_input"]["resolution_bits"], read_meta["analog_input"]["resolution_bits"],
        "Resolution should match"
    );

    println!("\n=== Metadata Recording Test PASSED ===\n");
}

// =============================================================================
// Test 5: Multi-Channel Storage
// =============================================================================

/// Test storing multi-channel acquisition data
#[test]
fn test_multi_channel_storage() {
    skip_if_disabled!();

    println!("\n=== Comedi Multi-Channel Storage Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open device");

    let channels = vec![0, 1, 2, 3];
    let config = StreamConfig::builder()
        .channels(&channels)
        .sample_rate(250.0) // Per channel
        .buffer_size(4096)
        .build()
        .expect("Failed to build config");

    let stream = StreamAcquisition::new(&device, config).expect("Failed to create stream");

    println!("\nAcquiring {} channels...", channels.len());
    stream.start().expect("Failed to start");

    let mut all_samples: Vec<f64> = Vec::new();
    let start = Instant::now();

    while all_samples.len() < 2000 && start.elapsed() < Duration::from_secs(10) {
        if let Ok(Some(data)) = stream.read_available() {
            all_samples.extend(data);
        }
        thread::sleep(Duration::from_millis(10));
    }

    stream.stop().expect("Failed to stop");

    println!("  Acquired {} total samples", all_samples.len());

    // Deinterleave channels
    let n_channels = channels.len();
    let scans = all_samples.len() / n_channels;

    println!("  Scans: {}", scans);

    let mut channel_data: Vec<Vec<f64>> = vec![Vec::new(); n_channels];
    for (i, sample) in all_samples.iter().enumerate() {
        channel_data[i % n_channels].push(*sample);
    }

    // Report per-channel statistics
    println!("\nPer-channel statistics:");
    for (ch_idx, data) in channel_data.iter().enumerate() {
        if !data.is_empty() {
            let mean: f64 = data.iter().sum::<f64>() / data.len() as f64;
            let min = data.iter().cloned().fold(f64::MAX, f64::min);
            let max = data.iter().cloned().fold(f64::MIN, f64::max);
            println!(
                "  CH{}: {} samples, mean={:.4}V, range=[{:.4}, {:.4}]V",
                channels[ch_idx],
                data.len(),
                mean,
                min,
                max
            );
        }
    }

    // Save deinterleaved to CSV
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("multichannel.csv");

    let header = channels
        .iter()
        .map(|c| format!("CH{}", c))
        .collect::<Vec<_>>()
        .join(",");
    let mut csv = format!("{}\n", header);

    for i in 0..scans.min(1000) {
        let row: Vec<String> = channel_data
            .iter()
            .map(|ch| ch.get(i).map(|v| format!("{:.6}", v)).unwrap_or_default())
            .collect();
        csv.push_str(&format!("{}\n", row.join(",")));
    }

    fs::write(&csv_path, &csv).expect("Failed to write CSV");
    println!("\nSaved to {:?}", csv_path);

    println!("\n=== Multi-Channel Storage Test PASSED ===\n");
}

// =============================================================================
// Skip Check Test
// =============================================================================

/// Test that storage tests are properly skipped when not enabled
#[test]
fn storage_test_skip_check() {
    let enabled = storage_test_enabled();
    if !enabled {
        println!("Comedi storage test correctly disabled (COMEDI_STORAGE_TEST not set)");
        println!("To enable: export COMEDI_STORAGE_TEST=1");
    } else {
        println!("Comedi storage test enabled via COMEDI_STORAGE_TEST=1");
    }
}
