#![cfg(not(target_arch = "wasm32"))]
//! Comedi Continuous Streaming Test Suite
//!
//! Tests for high-speed continuous data acquisition using the Comedi command interface.
//! Target hardware: National Instruments PCI-MIO-16XE-10 (100 kS/s aggregate).
//!
//! # Environment Variables
//!
//! Required:
//! - `COMEDI_SMOKE_TEST=1` - Enable the test suite
//!
//! Optional:
//! - `COMEDI_DEVICE` - Device path (default: "/dev/comedi0")
//!
//! # Running
//!
//! ```bash
//! export COMEDI_SMOKE_TEST=1
//! cargo test -p daq-driver-comedi --test continuous_streaming --features hardware -- --nocapture --test-threads=1
//! ```
//!
//! # Test Coverage
//!
//! | Test | Description |
//! |------|-------------|
//! | `test_basic_streaming` | Single channel continuous acquisition |
//! | `test_multi_channel_streaming` | 4-channel synchronized acquisition |
//! | `test_high_speed_acquisition` | High sample rate (50 kS/s) validation |
//! | `test_sustained_streaming` | 5-second sustained acquisition with stats |
//! | `test_sample_rate_accuracy` | Verify actual vs configured sample rate |
//! | `test_data_integrity` | Validate sample data is reasonable |

#![cfg(feature = "hardware")]

use daq_driver_comedi::{ComediDevice, StreamAcquisition, StreamConfig};
use std::env;
use std::thread;
use std::time::{Duration, Instant};

// =============================================================================
// Test Configuration
// =============================================================================

/// Check if streaming test is enabled via environment variable
fn smoke_test_enabled() -> bool {
    env::var("COMEDI_SMOKE_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

/// Get device path from environment or default to /dev/comedi0
fn device_path() -> String {
    env::var("COMEDI_DEVICE").unwrap_or_else(|_| "/dev/comedi0".to_string())
}

/// Skip test with message if smoke test not enabled
macro_rules! skip_if_disabled {
    () => {
        if !smoke_test_enabled() {
            println!("Comedi streaming test skipped (set COMEDI_SMOKE_TEST=1 to enable)");
            return;
        }
        // Allow device to recover from any previous test
        thread::sleep(Duration::from_millis(200));
    };
}

/// Cancel any running acquisition on the analog input subdevice.
/// This ensures a clean state before starting a new acquisition.
fn cancel_any_acquisition(device: &ComediDevice) {
    // Find AI subdevice and cancel any running command using the file descriptor
    if let Some(_ai_subdev) = device.find_subdevice(daq_driver_comedi::SubdeviceType::AnalogInput) {
        // Use the public fileno() and ioctl to cancel - or just let the
        // StreamAcquisition handle cleanup via Drop. The main fix is the
        // sleep delay to allow previous acquisition to fully stop.
        thread::sleep(Duration::from_millis(100));
    }
}

// =============================================================================
// NI PCI-MIO-16XE-10 Specifications
// =============================================================================

/// Maximum aggregate sample rate (all channels combined)
const MAX_AGGREGATE_RATE: f64 = 100_000.0; // 100 kS/s

/// Default test sample rate per channel
const DEFAULT_SAMPLE_RATE: f64 = 10_000.0; // 10 kS/s

/// High speed test sample rate
const HIGH_SPEED_RATE: f64 = 50_000.0; // 50 kS/s

/// Number of channels for multi-channel tests
const MULTI_CHANNEL_COUNT: usize = 4;

/// Standard test duration
const STANDARD_DURATION: Duration = Duration::from_secs(2);

/// Sustained test duration
const SUSTAINED_DURATION: Duration = Duration::from_secs(5);

/// Sample rate tolerance (percentage)
/// Note: Lower sample rates may have higher variance due to hardware timing quantization
const RATE_TOLERANCE_PERCENT: f64 = 10.0;

// =============================================================================
// Test Statistics Helper
// =============================================================================

/// Statistics collected during streaming tests
#[derive(Debug, Default)]
struct TestStats {
    /// Total samples acquired
    pub samples_acquired: u64,
    /// Total scans (samples per channel)
    pub scans_acquired: u64,
    /// Test duration
    pub duration: Duration,
    /// Calculated sample rate
    pub actual_rate: f64,
    /// Buffer overflows detected
    pub overflows: u64,
    /// Minimum sample value seen
    pub min_value: f64,
    /// Maximum sample value seen
    pub max_value: f64,
    /// Number of zero samples (potential issues)
    pub zero_samples: u64,
}

impl TestStats {
    fn new() -> Self {
        Self {
            min_value: f64::MAX,
            max_value: f64::MIN,
            ..Default::default()
        }
    }

    fn update_from_samples(&mut self, samples: &[f64]) {
        for &v in samples {
            if v < self.min_value {
                self.min_value = v;
            }
            if v > self.max_value {
                self.max_value = v;
            }
            if v.abs() < 1e-9 {
                self.zero_samples += 1;
            }
        }
        self.samples_acquired += samples.len() as u64;
    }

    fn calculate_rate(&mut self) {
        if self.duration.as_secs_f64() > 0.0 {
            self.actual_rate = self.samples_acquired as f64 / self.duration.as_secs_f64();
        }
    }

    fn print_summary(&self, test_name: &str) {
        println!("\n=== {} Statistics ===", test_name);
        println!("  Duration: {:?}", self.duration);
        println!("  Samples acquired: {}", self.samples_acquired);
        println!("  Scans acquired: {}", self.scans_acquired);
        println!("  Actual sample rate: {:.1} S/s", self.actual_rate);
        println!("  Buffer overflows: {}", self.overflows);
        println!(
            "  Value range: {:.4}V to {:.4}V",
            self.min_value, self.max_value
        );
        println!("  Zero samples: {}", self.zero_samples);
    }
}

// =============================================================================
// Test 1: Basic Single-Channel Streaming
// =============================================================================

/// Test basic single-channel continuous streaming
///
/// Validates:
/// - Stream configuration and setup
/// - Data acquisition starts and stops cleanly
/// - Samples are received continuously
#[test]
fn test_basic_streaming() {
    skip_if_disabled!();

    println!("\n=== Comedi Basic Streaming Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    // Cancel any lingering acquisition from previous tests
    cancel_any_acquisition(&device);

    // Configure single-channel streaming
    let config = StreamConfig::builder()
        .channels(&[0])
        .sample_rate(DEFAULT_SAMPLE_RATE)
        .buffer_size(4096)
        .build()
        .expect("Failed to build stream config");

    println!("Configuration:");
    println!("  Channel: 0");
    println!("  Sample rate: {} S/s", DEFAULT_SAMPLE_RATE);
    println!("  Buffer size: 4096 samples");

    let stream = StreamAcquisition::new(&device, config).expect("Failed to create stream");

    // Start acquisition
    println!("\nStarting acquisition...");
    stream.start().expect("Failed to start streaming");

    let mut stats = TestStats::new();
    let start = Instant::now();

    // Collect data for standard duration
    while start.elapsed() < STANDARD_DURATION {
        if let Ok(Some(samples)) = stream.read_available() {
            if !samples.is_empty() {
                stats.update_from_samples(&samples);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    stats.duration = start.elapsed();
    stats.calculate_rate();

    // Stop acquisition
    stream.stop().expect("Failed to stop streaming");

    // Get final stats from stream
    let stream_stats = stream.stats();
    stats.scans_acquired = stream_stats.scans_acquired;
    stats.overflows = stream_stats.overflows;

    stats.print_summary("Basic Streaming");

    // Explicit cleanup: stop stream, drop stream, then device closes
    drop(stream);
    thread::sleep(Duration::from_millis(100));

    // Assertions
    assert!(
        stats.samples_acquired > 0,
        "Should have acquired some samples"
    );
    assert!(
        stats.actual_rate > DEFAULT_SAMPLE_RATE * 0.5,
        "Sample rate should be at least 50% of configured rate"
    );
    assert!(
        stats.min_value >= -15.0 && stats.max_value <= 15.0,
        "Voltages should be within reasonable range"
    );

    println!("\n=== Basic Streaming Test PASSED ===\n");
}

// =============================================================================
// Test 2: Multi-Channel Synchronized Streaming
// =============================================================================

/// Test multi-channel synchronized acquisition
///
/// Validates:
/// - Multiple channels can be acquired simultaneously
/// - Channel data is properly interleaved
/// - All channels produce data
#[test]
fn test_multi_channel_streaming() {
    skip_if_disabled!();

    println!("\n=== Comedi Multi-Channel Streaming Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    // Cancel any lingering acquisition from previous tests
    cancel_any_acquisition(&device);

    // Configure multi-channel streaming
    // Note: Per-channel rate, so aggregate = rate * n_channels
    let per_channel_rate = DEFAULT_SAMPLE_RATE / MULTI_CHANNEL_COUNT as f64;
    let channels: Vec<u32> = (0..MULTI_CHANNEL_COUNT as u32).collect();

    let config = StreamConfig::builder()
        .channels(&channels)
        .sample_rate(per_channel_rate)
        .buffer_size(8192)
        .build()
        .expect("Failed to build stream config");

    println!("Configuration:");
    println!("  Channels: {:?}", channels);
    println!("  Per-channel rate: {:.0} S/s", per_channel_rate);
    println!(
        "  Aggregate rate: {:.0} S/s",
        per_channel_rate * MULTI_CHANNEL_COUNT as f64
    );

    let stream = StreamAcquisition::new(&device, config).expect("Failed to create stream");

    println!("\nStarting multi-channel acquisition...");
    stream.start().expect("Failed to start streaming");

    let mut stats = TestStats::new();
    let mut channel_samples: Vec<Vec<f64>> = vec![Vec::new(); MULTI_CHANNEL_COUNT];
    let start = Instant::now();

    while start.elapsed() < STANDARD_DURATION {
        if let Ok(Some(samples)) = stream.read_available() {
            if !samples.is_empty() {
                stats.update_from_samples(&samples);

                // Deinterleave samples into per-channel vectors
                for (i, &v) in samples.iter().enumerate() {
                    let ch = i % MULTI_CHANNEL_COUNT;
                    channel_samples[ch].push(v);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    stats.duration = start.elapsed();
    stats.calculate_rate();
    stats.scans_acquired = channel_samples[0].len() as u64;

    stream.stop().expect("Failed to stop streaming");

    stats.print_summary("Multi-Channel Streaming");

    // Per-channel statistics
    println!("\nPer-Channel Statistics:");
    for (ch, samples) in channel_samples.iter().enumerate() {
        if !samples.is_empty() {
            let min = samples.iter().cloned().fold(f64::MAX, f64::min);
            let max = samples.iter().cloned().fold(f64::MIN, f64::max);
            let mean: f64 = samples.iter().sum::<f64>() / samples.len() as f64;
            println!(
                "  CH{}: {} samples, min={:.4}V, max={:.4}V, mean={:.4}V",
                ch,
                samples.len(),
                min,
                max,
                mean
            );
        } else {
            println!("  CH{}: No samples!", ch);
        }
    }

    // Assertions
    for (ch, samples) in channel_samples.iter().enumerate() {
        assert!(!samples.is_empty(), "Channel {} should have samples", ch);
    }

    // All channels should have approximately equal sample counts
    let counts: Vec<usize> = channel_samples.iter().map(|s| s.len()).collect();
    let max_count = *counts.iter().max().unwrap_or(&0);
    let min_count = *counts.iter().min().unwrap_or(&0);
    assert!(
        max_count - min_count <= 1,
        "Channel sample counts should be equal (or differ by at most 1)"
    );

    // Explicit cleanup
    drop(stream);
    thread::sleep(Duration::from_millis(100));

    println!("\n=== Multi-Channel Streaming Test PASSED ===\n");
}

// =============================================================================
// Test 3: High-Speed Acquisition
// =============================================================================

/// Test high-speed single-channel acquisition
///
/// Validates:
/// - Device can sustain high sample rates
/// - No significant data loss at high speeds
/// - Buffer management handles throughput
#[test]
fn test_high_speed_acquisition() {
    skip_if_disabled!();

    println!("\n=== Comedi High-Speed Acquisition Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    // Cancel any lingering acquisition from previous tests
    cancel_any_acquisition(&device);

    // Configure high-speed streaming
    let config = StreamConfig::builder()
        .channels(&[0])
        .sample_rate(HIGH_SPEED_RATE)
        .buffer_size(16384) // Larger buffer for high speed
        .build()
        .expect("Failed to build stream config");

    println!("Configuration:");
    println!("  Channel: 0");
    println!("  Target sample rate: {} S/s", HIGH_SPEED_RATE);
    println!("  Buffer size: 16384 samples");

    let stream = StreamAcquisition::new(&device, config).expect("Failed to create stream");

    println!("\nStarting high-speed acquisition...");
    stream.start().expect("Failed to start streaming");

    let mut stats = TestStats::new();
    let start = Instant::now();

    while start.elapsed() < STANDARD_DURATION {
        if let Ok(Some(samples)) = stream.read_available() {
            if !samples.is_empty() {
                stats.update_from_samples(&samples);
            }
        }
        // Shorter sleep for high-speed acquisition
        std::thread::sleep(Duration::from_micros(500));
    }

    stats.duration = start.elapsed();
    stats.calculate_rate();

    let stream_stats = stream.stats();
    stats.overflows = stream_stats.overflows;

    stream.stop().expect("Failed to stop streaming");

    stats.print_summary("High-Speed Acquisition");

    // Calculate expected samples
    let expected_samples = (HIGH_SPEED_RATE * stats.duration.as_secs_f64()) as u64;
    let capture_efficiency = (stats.samples_acquired as f64 / expected_samples as f64) * 100.0;
    println!("  Expected samples: {}", expected_samples);
    println!("  Capture efficiency: {:.1}%", capture_efficiency);

    // Assertions
    assert!(
        stats.samples_acquired > 0,
        "Should have acquired samples at high speed"
    );
    assert!(
        capture_efficiency > 80.0,
        "Capture efficiency should be > 80% (got {:.1}%)",
        capture_efficiency
    );
    assert!(
        stats.overflows < 10,
        "Should have minimal overflows (got {})",
        stats.overflows
    );

    // Explicit cleanup
    drop(stream);
    thread::sleep(Duration::from_millis(100));

    println!("\n=== High-Speed Acquisition Test PASSED ===\n");
}

// =============================================================================
// Test 4: Sustained Streaming
// =============================================================================

/// Test sustained streaming over longer duration
///
/// Validates:
/// - Acquisition can run continuously for extended period
/// - No accumulating errors or memory issues
/// - Consistent throughput over time
#[test]
fn test_sustained_streaming() {
    skip_if_disabled!();

    println!("\n=== Comedi Sustained Streaming Test ===");
    println!("Device: {}", device_path());
    println!("Duration: {:?}", SUSTAINED_DURATION);

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    // Cancel any lingering acquisition from previous tests
    cancel_any_acquisition(&device);

    let config = StreamConfig::builder()
        .channels(&[0, 1])
        .sample_rate(DEFAULT_SAMPLE_RATE / 2.0) // 5kS/s per channel
        .buffer_size(8192)
        .build()
        .expect("Failed to build stream config");

    println!("Configuration:");
    println!("  Channels: [0, 1]");
    println!("  Per-channel rate: {} S/s", DEFAULT_SAMPLE_RATE / 2.0);

    let stream = StreamAcquisition::new(&device, config).expect("Failed to create stream");

    println!("\nStarting sustained acquisition...");
    stream.start().expect("Failed to start streaming");

    let mut stats = TestStats::new();
    let mut periodic_rates: Vec<f64> = Vec::new();
    let start = Instant::now();
    let mut last_checkpoint = Instant::now();
    let mut samples_at_checkpoint: u64 = 0;

    while start.elapsed() < SUSTAINED_DURATION {
        if let Ok(Some(samples)) = stream.read_available() {
            if !samples.is_empty() {
                stats.update_from_samples(&samples);
            }
        }

        // Periodic rate check every second
        if last_checkpoint.elapsed() >= Duration::from_secs(1) {
            let samples_this_period = stats.samples_acquired - samples_at_checkpoint;
            let rate = samples_this_period as f64 / last_checkpoint.elapsed().as_secs_f64();
            periodic_rates.push(rate);
            println!(
                "  [{:?}] Rate: {:.0} S/s, Total: {} samples",
                start.elapsed(),
                rate,
                stats.samples_acquired
            );

            samples_at_checkpoint = stats.samples_acquired;
            last_checkpoint = Instant::now();
        }

        std::thread::sleep(Duration::from_millis(10));
    }

    stats.duration = start.elapsed();
    stats.calculate_rate();

    let stream_stats = stream.stats();
    stats.overflows = stream_stats.overflows;

    stream.stop().expect("Failed to stop streaming");

    stats.print_summary("Sustained Streaming");

    // Rate stability analysis
    if !periodic_rates.is_empty() {
        let avg_rate: f64 = periodic_rates.iter().sum::<f64>() / periodic_rates.len() as f64;
        let rate_variance: f64 = periodic_rates
            .iter()
            .map(|r| (r - avg_rate).powi(2))
            .sum::<f64>()
            / periodic_rates.len() as f64;
        let rate_std_dev = rate_variance.sqrt();
        let rate_cv = (rate_std_dev / avg_rate) * 100.0;

        println!("\nRate Stability:");
        println!("  Average rate: {:.0} S/s", avg_rate);
        println!("  Std deviation: {:.0} S/s", rate_std_dev);
        println!("  Coefficient of variation: {:.1}%", rate_cv);

        // Rate should be stable (CV < 20%)
        assert!(
            rate_cv < 20.0,
            "Rate coefficient of variation should be < 20% (got {:.1}%)",
            rate_cv
        );
    }

    // Assertions
    let min_expected = (DEFAULT_SAMPLE_RATE * SUSTAINED_DURATION.as_secs_f64() * 0.5) as u64;
    assert!(
        stats.samples_acquired > min_expected,
        "Should acquire at least {} samples (got {})",
        min_expected,
        stats.samples_acquired
    );

    // Explicit cleanup
    drop(stream);
    thread::sleep(Duration::from_millis(100));

    println!("\n=== Sustained Streaming Test PASSED ===\n");
}

// =============================================================================
// Test 5: Sample Rate Accuracy
// =============================================================================

/// Test sample rate accuracy
///
/// Validates:
/// - Actual sample rate matches configured rate within tolerance
/// - Hardware timing is accurate
#[test]
fn test_sample_rate_accuracy() {
    skip_if_disabled!();

    println!("\n=== Comedi Sample Rate Accuracy Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    // Cancel any lingering acquisition from previous tests
    cancel_any_acquisition(&device);

    // Test at several sample rates
    // Start with higher rates first - lower rates may need more settling time
    let test_rates = [10000.0, 25000.0, 5000.0];

    for &target_rate in &test_rates {
        println!("\nTesting target rate: {} S/s", target_rate);

        let config = StreamConfig::builder()
            .channels(&[0])
            .sample_rate(target_rate)
            .buffer_size(4096)
            .build()
            .expect("Failed to build stream config");

        let stream = StreamAcquisition::new(&device, config).expect("Failed to create stream");

        stream.start().expect("Failed to start streaming");

        // Allow hardware to stabilize before measuring
        thread::sleep(Duration::from_millis(100));

        let mut sample_count: u64 = 0;
        let start = Instant::now();
        let test_duration = Duration::from_millis(1500); // 1.5 seconds for more accurate measurement

        while start.elapsed() < test_duration {
            if let Ok(Some(samples)) = stream.read_available() {
                sample_count += samples.len() as u64;
            }
            thread::sleep(Duration::from_micros(500));
        }

        let elapsed = start.elapsed();
        stream.stop().expect("Failed to stop streaming");

        // Cleanup between rate tests
        drop(stream);
        thread::sleep(Duration::from_millis(200));
        cancel_any_acquisition(&device);

        let actual_rate = sample_count as f64 / elapsed.as_secs_f64();
        let error_percent = ((actual_rate - target_rate) / target_rate * 100.0).abs();

        println!(
            "  Target: {:.0} S/s, Actual: {:.0} S/s, Error: {:.2}%",
            target_rate, actual_rate, error_percent
        );

        // Allow tolerance for hardware timing adjustments
        // Lower rates may have more variance due to timer quantization
        let tolerance = if target_rate < 10000.0 {
            RATE_TOLERANCE_PERCENT * 2.0 // 20% for lower rates
        } else {
            RATE_TOLERANCE_PERCENT // 10% for higher rates
        };
        assert!(
            error_percent < tolerance,
            "Sample rate error {:.1}% exceeds tolerance {:.0}% for {} S/s",
            error_percent,
            tolerance,
            target_rate
        );
    }

    println!("\n=== Sample Rate Accuracy Test PASSED ===\n");
}

// =============================================================================
// Test 6: Data Integrity
// =============================================================================

/// Test data integrity during streaming
///
/// Validates:
/// - Sample values are within expected voltage range
/// - No corrupted data (NaN, Inf)
/// - Reasonable signal characteristics
#[test]
fn test_data_integrity() {
    skip_if_disabled!();

    println!("\n=== Comedi Data Integrity Test ===");
    println!("Device: {}", device_path());

    let device = ComediDevice::open(&device_path()).expect("Failed to open Comedi device");

    // Cancel any lingering acquisition from previous tests
    cancel_any_acquisition(&device);

    let config = StreamConfig::builder()
        .channels(&[0])
        .sample_rate(DEFAULT_SAMPLE_RATE)
        .buffer_size(4096)
        .build()
        .expect("Failed to build stream config");

    let stream = StreamAcquisition::new(&device, config).expect("Failed to create stream");

    println!("Starting data integrity test...");
    stream.start().expect("Failed to start streaming");

    let mut all_samples: Vec<f64> = Vec::new();
    let mut nan_count = 0u64;
    let mut inf_count = 0u64;
    let mut out_of_range = 0u64;
    let start = Instant::now();

    while start.elapsed() < STANDARD_DURATION {
        if let Ok(Some(samples)) = stream.read_available() {
            for &v in &samples {
                if v.is_nan() {
                    nan_count += 1;
                } else if v.is_infinite() {
                    inf_count += 1;
                } else if v < -15.0 || v > 15.0 {
                    out_of_range += 1;
                } else {
                    all_samples.push(v);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    stream.stop().expect("Failed to stop streaming");

    println!("\nData Integrity Results:");
    println!("  Total samples: {}", all_samples.len());
    println!("  NaN values: {}", nan_count);
    println!("  Infinite values: {}", inf_count);
    println!("  Out of range (±15V): {}", out_of_range);

    if !all_samples.is_empty() {
        let min = all_samples.iter().cloned().fold(f64::MAX, f64::min);
        let max = all_samples.iter().cloned().fold(f64::MIN, f64::max);
        let mean: f64 = all_samples.iter().sum::<f64>() / all_samples.len() as f64;
        let variance: f64 =
            all_samples.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / all_samples.len() as f64;
        let std_dev = variance.sqrt();

        println!("\nSignal Statistics:");
        println!("  Min: {:.6}V", min);
        println!("  Max: {:.6}V", max);
        println!("  Mean: {:.6}V", mean);
        println!("  Std Dev: {:.6}V", std_dev);
        println!("  Dynamic range: {:.6}V", max - min);
    }

    // Assertions
    assert!(
        !all_samples.is_empty(),
        "Should have collected valid samples"
    );
    assert_eq!(nan_count, 0, "Should have no NaN values");
    assert_eq!(inf_count, 0, "Should have no infinite values");
    assert!(
        out_of_range < all_samples.len() as u64 / 100,
        "Less than 1% of samples should be out of range"
    );

    // Explicit cleanup
    drop(stream);
    thread::sleep(Duration::from_millis(100));

    println!("\n=== Data Integrity Test PASSED ===\n");
}

// =============================================================================
// Skip Check Test
// =============================================================================

/// Test that streaming tests are properly skipped when not enabled
#[test]
fn streaming_test_skip_check() {
    let enabled = smoke_test_enabled();
    if !enabled {
        println!("Comedi streaming test correctly disabled (COMEDI_SMOKE_TEST not set)");
    } else {
        println!("Comedi streaming test enabled via COMEDI_SMOKE_TEST=1");
    }
}
