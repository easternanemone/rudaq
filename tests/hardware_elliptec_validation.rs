//! Elliptec ELL14 Hardware Validation Tests
//!
//! Tests for Thorlabs Elliptec ELL14 rotation mounts on shared RS-485 bus.
//! Hardware: 3 rotators at addresses 2, 3, 8 on /dev/ttyUSB0
//!
//! Run with: cargo test --features "hardware_tests,instrument_thorlabs" --test hardware_elliptec_validation -- --nocapture
//!
//! SAFETY: These tests move physical hardware. Ensure no obstructions before running.

#![cfg(all(feature = "hardware_tests", feature = "instrument_thorlabs"))]

use anyhow::{Context, Result};
use rust_daq::hardware::capabilities::Movable;
use rust_daq::hardware::ell14::Ell14Driver;
use std::time::Duration;
use tokio::time::sleep;

const PORT: &str = "/dev/ttyUSB0";
const ADDRESSES: [&str; 3] = ["2", "3", "8"];
const POSITION_TOLERANCE_DEG: f64 = 1.0;

/// Helper to create drivers for all three rotators
async fn create_drivers() -> Result<Vec<(String, Ell14Driver)>> {
    let mut drivers = Vec::new();
    for addr in ADDRESSES {
        let driver = Ell14Driver::new(PORT, addr)
            .context(format!("Failed to create driver for address {}", addr))?;
        drivers.push((addr.to_string(), driver));
    }
    Ok(drivers)
}

// =============================================================================
// Phase 1: Basic Connectivity Tests
// =============================================================================

#[tokio::test]
async fn test_all_rotators_respond_to_position_query() {
    println!("\n=== Test: Position Query for All Rotators ===");

    for addr in ADDRESSES {
        let driver = Ell14Driver::new(PORT, addr)
            .expect(&format!("Failed to create driver for address {}", addr));

        let position = driver.position().await;
        match position {
            Ok(pos) => {
                // ELL14 can track continuous rotation beyond 360 degrees
                // Normalize to 0-360 for display but accept any value
                let normalized = pos % 360.0;
                let full_rotations = (pos / 360.0).floor() as i32;
                println!(
                    "Rotator {} position: {:.2}° (normalized: {:.2}°, {} full rotations)",
                    addr, pos, normalized, full_rotations
                );
                // Just verify it's a finite number
                assert!(pos.is_finite(), "Position is not finite: {}", pos);
            }
            Err(e) => {
                panic!("Rotator {} failed to respond: {}", addr, e);
            }
        }

        // Small delay between devices to avoid bus contention
        sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn test_rotator_info_responses() {
    println!("\n=== Test: Device Info Responses ===");

    // This tests the raw serial communication
    use std::io::{Read, Write};
    use std::time::Duration;

    let mut port = serialport::new(PORT, 9600)
        .timeout(Duration::from_millis(500))
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::None)
        .open()
        .expect("Failed to open port");

    for addr in ADDRESSES {
        let command = format!("{}in", addr);
        port.write_all(command.as_bytes())
            .expect("Failed to write");
        std::thread::sleep(Duration::from_millis(200));

        let mut buffer = [0u8; 256];
        match port.read(&mut buffer) {
            Ok(n) if n > 0 => {
                let response = String::from_utf8_lossy(&buffer[..n]);
                println!("Rotator {} info: {}", addr, response.trim());

                // Verify response starts with address + "IN"
                assert!(
                    response.contains("IN"),
                    "Expected INFO response, got: {}",
                    response
                );
            }
            _ => panic!("No response from rotator {}", addr),
        }
    }
}

// =============================================================================
// Phase 2: Movement Tests
// =============================================================================

#[tokio::test]
async fn test_absolute_movement_single_rotator() {
    println!("\n=== Test: Absolute Movement (Single Rotator) ===");

    // Test with rotator at address 2
    let driver = Ell14Driver::new(PORT, "2").expect("Failed to create driver");

    // Get initial position
    let initial = driver.position().await.expect("Failed to get initial position");
    println!("Initial position: {:.2}°", initial);

    // Move to 45 degrees
    let target = 45.0;
    println!("Moving to {:.2}°...", target);
    driver
        .move_abs(target)
        .await
        .expect("Failed to send move command");

    // Wait for movement to complete
    driver.wait_settled().await.expect("Failed to wait for settle");

    // Verify position
    let final_pos = driver.position().await.expect("Failed to get final position");
    println!("Final position: {:.2}°", final_pos);

    let error = (final_pos - target).abs();
    assert!(
        error < POSITION_TOLERANCE_DEG,
        "Position error too large: {:.2}° (tolerance: {:.2}°)",
        error,
        POSITION_TOLERANCE_DEG
    );

    // Return to initial position
    driver
        .move_abs(initial)
        .await
        .expect("Failed to return to initial");
    driver.wait_settled().await.ok();
}

#[tokio::test]
async fn test_relative_movement() {
    println!("\n=== Test: Relative Movement ===");

    let driver = Ell14Driver::new(PORT, "3").expect("Failed to create driver");

    // Get initial position
    let initial = driver.position().await.expect("Failed to get initial position");
    println!("Initial position: {:.2}°", initial);

    // Move relative +10 degrees
    let delta = 10.0;
    println!("Moving relative +{:.2}°...", delta);
    driver
        .move_rel(delta)
        .await
        .expect("Failed to send relative move");
    driver.wait_settled().await.expect("Failed to wait for settle");

    let pos_after_forward = driver.position().await.expect("Failed to get position");
    println!("Position after +10°: {:.2}°", pos_after_forward);

    // Move relative -10 degrees (back to start)
    println!("Moving relative -{:.2}°...", delta);
    driver
        .move_rel(-delta)
        .await
        .expect("Failed to send relative move");
    driver.wait_settled().await.expect("Failed to wait for settle");

    let final_pos = driver.position().await.expect("Failed to get final position");
    println!("Final position: {:.2}°", final_pos);

    // Should be back at initial
    let error = (final_pos - initial).abs();
    assert!(
        error < POSITION_TOLERANCE_DEG,
        "Failed to return to initial position. Error: {:.2}°",
        error
    );
}

#[tokio::test]
async fn test_home_command() {
    println!("\n=== Test: Home Command ===");

    let driver = Ell14Driver::new(PORT, "8").expect("Failed to create driver");

    // Get initial position
    let initial = driver.position().await.expect("Failed to get initial position");
    println!("Initial position: {:.2}°", initial);

    // Home the device
    println!("Homing...");
    driver.home().await.expect("Failed to home");

    // Get position after homing
    let home_pos = driver.position().await.expect("Failed to get home position");
    println!("Position after home: {:.2}°", home_pos);

    // Home position should be near 0 (mechanical zero)
    assert!(
        home_pos.abs() < 5.0,
        "Home position too far from zero: {:.2}°",
        home_pos
    );

    // Return to initial if it was different
    if (initial - home_pos).abs() > POSITION_TOLERANCE_DEG {
        driver.move_abs(initial).await.ok();
        driver.wait_settled().await.ok();
    }
}

// =============================================================================
// Phase 3: Multi-Device Tests
// =============================================================================

#[tokio::test]
async fn test_sequential_queries_all_devices() {
    println!("\n=== Test: Sequential Queries All Devices ===");

    for addr in ADDRESSES {
        let driver = Ell14Driver::new(PORT, addr).expect("Failed to create driver");

        let pos = driver.position().await.expect("Failed to get position");
        println!("Rotator {} at {:.2}°", addr, pos);

        // Verify position is valid
        assert!(
            pos >= -360.0 && pos <= 720.0,
            "Position out of expected range"
        );
    }
}

#[tokio::test]
async fn test_move_all_devices_sequentially() {
    println!("\n=== Test: Move All Devices Sequentially ===");

    // Store initial positions
    let mut initial_positions = Vec::new();

    for addr in ADDRESSES {
        let driver = Ell14Driver::new(PORT, addr).expect("Failed to create driver");
        let pos = driver.position().await.expect("Failed to get position");
        initial_positions.push((addr.to_string(), pos));
        println!("Rotator {} initial: {:.2}°", addr, pos);
    }

    // Move each device to a different target
    let targets = [30.0, 60.0, 90.0];
    for (i, addr) in ADDRESSES.iter().enumerate() {
        let driver = Ell14Driver::new(PORT, addr).expect("Failed to create driver");
        let target = targets[i];

        println!("Moving rotator {} to {:.2}°...", addr, target);
        driver.move_abs(target).await.expect("Failed to move");
        driver.wait_settled().await.expect("Failed to settle");

        let pos = driver.position().await.expect("Failed to get position");
        println!("Rotator {} now at {:.2}°", addr, pos);

        let error = (pos - target).abs();
        assert!(error < POSITION_TOLERANCE_DEG, "Position error: {:.2}°", error);
    }

    // Return to initial positions
    println!("\nReturning to initial positions...");
    for (addr, initial) in &initial_positions {
        let driver = Ell14Driver::new(PORT, addr).expect("Failed to create driver");
        driver.move_abs(*initial).await.ok();
        driver.wait_settled().await.ok();
    }
}

// =============================================================================
// Phase 4: Accuracy and Repeatability Tests
// =============================================================================

#[tokio::test]
async fn test_position_repeatability() {
    println!("\n=== Test: Position Repeatability ===");

    let driver = Ell14Driver::new(PORT, "2").expect("Failed to create driver");
    let target = 45.0;
    let num_trials = 5;
    let mut positions = Vec::new();

    // Get initial position
    let initial = driver.position().await.expect("Failed to get initial position");

    for i in 1..=num_trials {
        // Move to target
        driver.move_abs(target).await.expect("Failed to move");
        driver.wait_settled().await.expect("Failed to settle");

        let pos = driver.position().await.expect("Failed to get position");
        positions.push(pos);
        println!("Trial {}: {:.3}°", i, pos);

        // Move away
        driver.move_abs(0.0).await.expect("Failed to move");
        driver.wait_settled().await.expect("Failed to settle");
    }

    // Calculate statistics
    let mean: f64 = positions.iter().sum::<f64>() / num_trials as f64;
    let variance: f64 = positions.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / num_trials as f64;
    let std_dev = variance.sqrt();

    println!("\nRepeatability Results:");
    println!("  Target: {:.2}°", target);
    println!("  Mean: {:.3}°", mean);
    println!("  Std Dev: {:.3}°", std_dev);
    println!("  Max Error: {:.3}°", (mean - target).abs());

    assert!(
        std_dev < 0.5,
        "Repeatability too poor (std dev: {:.3}°)",
        std_dev
    );
    assert!(
        (mean - target).abs() < POSITION_TOLERANCE_DEG,
        "Mean position error too large: {:.3}°",
        (mean - target).abs()
    );

    // Return to initial
    driver.move_abs(initial).await.ok();
    driver.wait_settled().await.ok();
}

#[tokio::test]
async fn test_full_rotation_accuracy() {
    println!("\n=== Test: Full Rotation Accuracy ===");

    let driver = Ell14Driver::new(PORT, "3").expect("Failed to create driver");
    let initial = driver.position().await.expect("Failed to get initial position");

    // Test positions around full rotation
    let test_positions = [0.0, 90.0, 180.0, 270.0, 360.0];

    for target in test_positions {
        driver.move_abs(target).await.expect("Failed to move");
        driver.wait_settled().await.expect("Failed to settle");

        let actual = driver.position().await.expect("Failed to get position");
        let error = (actual - target).abs();

        println!("Target: {:.0}° → Actual: {:.2}° (error: {:.2}°)", target, actual, error);

        assert!(
            error < POSITION_TOLERANCE_DEG,
            "Position error at {:.0}° is {:.2}°",
            target,
            error
        );
    }

    // Return to initial
    driver.move_abs(initial).await.ok();
    driver.wait_settled().await.ok();
}

// =============================================================================
// Phase 5: Stress and Edge Case Tests
// =============================================================================

#[tokio::test]
async fn test_rapid_position_queries() {
    println!("\n=== Test: Rapid Position Queries ===");

    let driver = Ell14Driver::new(PORT, "2").expect("Failed to create driver");
    let num_queries = 20;
    let mut success_count = 0;

    for i in 1..=num_queries {
        match driver.position().await {
            Ok(pos) => {
                success_count += 1;
                if i % 5 == 0 {
                    println!("Query {}: {:.2}°", i, pos);
                }
            }
            Err(e) => {
                println!("Query {} failed: {}", i, e);
            }
        }
        // Minimal delay
        sleep(Duration::from_millis(50)).await;
    }

    let success_rate = (success_count as f64 / num_queries as f64) * 100.0;
    println!("\nSuccess rate: {:.1}% ({}/{})", success_rate, success_count, num_queries);

    assert!(
        success_rate >= 95.0,
        "Query success rate too low: {:.1}%",
        success_rate
    );
}

#[tokio::test]
async fn test_bus_contention_resilience() {
    println!("\n=== Test: Bus Contention Resilience ===");

    // Rapidly query all three devices
    let num_rounds = 5;
    let mut all_success = true;

    for round in 1..=num_rounds {
        println!("Round {}:", round);
        for addr in ADDRESSES {
            let driver = Ell14Driver::new(PORT, addr).expect("Failed to create driver");
            match driver.position().await {
                Ok(pos) => {
                    println!("  Rotator {}: {:.2}°", addr, pos);
                }
                Err(e) => {
                    println!("  Rotator {}: FAILED - {}", addr, e);
                    all_success = false;
                }
            }
            // Minimal inter-device delay
            sleep(Duration::from_millis(30)).await;
        }
    }

    assert!(all_success, "Some queries failed during bus contention test");
}
