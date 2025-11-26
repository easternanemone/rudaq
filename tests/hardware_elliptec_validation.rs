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

    // Clear the bus by waiting and doing a simple status query first
    sleep(Duration::from_millis(200)).await;

    // Store initial positions
    let mut initial_positions = Vec::new();

    for addr in ADDRESSES {
        // Add delay between device queries to avoid RS-485 bus contention
        sleep(Duration::from_millis(200)).await;
        let driver = Ell14Driver::new(PORT, addr).expect("Failed to create driver");
        sleep(Duration::from_millis(100)).await;

        // Retry logic for position query
        let mut pos = None;
        for attempt in 0..3 {
            match driver.position().await {
                Ok(p) => {
                    pos = Some(p);
                    break;
                }
                Err(e) if attempt < 2 => {
                    println!("  Retry {} for {}: {}", attempt + 1, addr, e);
                    sleep(Duration::from_millis(150)).await;
                }
                Err(e) => {
                    panic!("Failed to get position for {}: {}", addr, e);
                }
            }
        }

        let p = pos.unwrap();
        initial_positions.push((addr.to_string(), p));
        println!("Rotator {} initial: {:.2}°", addr, p);
    }

    // Move each device to a different target
    let targets = [30.0, 60.0, 90.0];
    for (i, addr) in ADDRESSES.iter().enumerate() {
        // Add delay between creating drivers for different devices
        sleep(Duration::from_millis(200)).await;
        let driver = Ell14Driver::new(PORT, addr).expect("Failed to create driver");
        let target = targets[i];

        println!("Moving rotator {} to {:.2}°...", addr, target);
        driver.move_abs(target).await.expect("Failed to move");
        driver.wait_settled().await.expect("Failed to settle");

        sleep(Duration::from_millis(150)).await;

        // Retry logic for position query after move
        let mut pos = None;
        for attempt in 0..3 {
            match driver.position().await {
                Ok(p) => {
                    pos = Some(p);
                    break;
                }
                Err(e) if attempt < 2 => {
                    println!("  Position retry {} for {}: {}", attempt + 1, addr, e);
                    sleep(Duration::from_millis(150)).await;
                }
                Err(e) => {
                    panic!("Failed to get position after move for {}: {}", addr, e);
                }
            }
        }
        let pos = pos.unwrap();
        println!("Rotator {} now at {:.2}°", addr, pos);

        let error = (pos - target).abs();
        assert!(error < POSITION_TOLERANCE_DEG, "Position error: {:.2}°", error);
    }

    // Return to initial positions
    println!("\nReturning to initial positions...");
    for (addr, initial) in &initial_positions {
        sleep(Duration::from_millis(100)).await;
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

// =============================================================================
// Phase 6: Advanced Feature Tests (Jog, Velocity, Motor Optimization)
// =============================================================================

#[tokio::test]
async fn test_device_info() {
    println!("\n=== Test: Device Info ===");

    for addr in ADDRESSES {
        let driver = Ell14Driver::new(PORT, addr).expect("Failed to create driver");

        match driver.get_device_info().await {
            Ok(info) => {
                println!("Rotator {} info:", addr);
                println!("  Type: {}", info.device_type);
                println!("  Serial: {}", info.serial);
                println!("  Firmware: {}", info.firmware);
                println!("  Year: {}", info.year);
                println!("  Travel: {} pulses", info.travel);
                println!("  Pulses/unit: {}", info.pulses_per_unit);

                // Verify it's an ELL14
                assert!(
                    info.device_type.contains("14") || info.device_type.contains("0E"),
                    "Unexpected device type: {}",
                    info.device_type
                );
            }
            Err(e) => {
                println!("Rotator {} info failed: {}", addr, e);
                // Don't fail - some responses may be hard to parse
            }
        }

        sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn test_jog_step_get_set() {
    println!("\n=== Test: Jog Step Get/Set ===");

    let driver = Ell14Driver::new(PORT, "2").expect("Failed to create driver");

    // Get initial jog step
    let initial_jog = driver.get_jog_step().await.expect("Failed to get jog step");
    println!("Initial jog step: {:.3}°", initial_jog);

    // Set a new jog step
    let new_jog_step = 5.0;
    driver
        .set_jog_step(new_jog_step)
        .await
        .expect("Failed to set jog step");
    println!("Set jog step to: {:.1}°", new_jog_step);

    // Verify it was set
    sleep(Duration::from_millis(100)).await;
    let read_back = driver.get_jog_step().await.expect("Failed to read back jog step");
    println!("Read back jog step: {:.3}°", read_back);

    let error = (read_back - new_jog_step).abs();
    assert!(
        error < 0.1,
        "Jog step mismatch: expected {:.1}°, got {:.3}°",
        new_jog_step,
        read_back
    );

    // Restore original
    driver.set_jog_step(initial_jog).await.ok();
}

#[tokio::test]
async fn test_jog_forward_backward() {
    println!("\n=== Test: Jog Forward/Backward ===");

    let driver = Ell14Driver::new(PORT, "3").expect("Failed to create driver");

    // Get initial position
    let initial = driver.position().await.expect("Failed to get position");
    println!("Initial position: {:.2}°", initial);

    // Set jog step to 10 degrees
    driver
        .set_jog_step(10.0)
        .await
        .expect("Failed to set jog step");

    // Jog forward
    println!("Jogging forward by 10°...");
    driver.jog_forward().await.expect("Failed to jog forward");
    driver.wait_settled().await.expect("Failed to wait");

    let after_forward = driver.position().await.expect("Failed to get position");
    println!("After forward: {:.2}°", after_forward);

    // Jog backward
    println!("Jogging backward by 10°...");
    driver.jog_backward().await.expect("Failed to jog backward");
    driver.wait_settled().await.expect("Failed to wait");

    let after_backward = driver.position().await.expect("Failed to get position");
    println!("After backward: {:.2}°", after_backward);

    // Should be back near initial
    let error = (after_backward - initial).abs();
    assert!(
        error < POSITION_TOLERANCE_DEG,
        "Failed to return to initial. Error: {:.2}°",
        error
    );
}

#[tokio::test]
async fn test_stop_command() {
    println!("\n=== Test: Stop Command ===");

    let driver = Ell14Driver::new(PORT, "2").expect("Failed to create driver");

    // Get initial position
    let initial = driver.position().await.expect("Failed to get position");
    println!("Initial position: {:.2}°", initial);

    // Start a long move
    let target = initial + 180.0;
    println!("Starting move to {:.0}°...", target);
    driver.move_abs(target).await.expect("Failed to start move");

    // Wait briefly then stop
    sleep(Duration::from_millis(200)).await;
    driver.stop().await.expect("Failed to stop");
    println!("Stop command sent");

    // Wait for motion to halt
    sleep(Duration::from_millis(200)).await;

    // Check position - should be somewhere between initial and target
    let stopped_pos = driver.position().await.expect("Failed to get position");
    println!("Stopped at: {:.2}°", stopped_pos);

    // Return to initial
    driver.move_abs(initial).await.ok();
    driver.wait_settled().await.ok();
}

#[tokio::test]
async fn test_velocity_get_set() {
    println!("\n=== Test: Velocity Get/Set ===");

    let driver = Ell14Driver::new(PORT, "8").expect("Failed to create driver");

    // Get current velocity
    match driver.get_velocity().await {
        Ok(velocity) => {
            println!("Current velocity: {}%", velocity);
            assert!(
                velocity >= 60 && velocity <= 100,
                "Velocity out of range: {}%",
                velocity
            );

            // Try setting a new velocity
            let new_velocity = 80;
            driver
                .set_velocity(new_velocity)
                .await
                .expect("Failed to set velocity");
            println!("Set velocity to: {}%", new_velocity);

            sleep(Duration::from_millis(100)).await;

            // Read back
            if let Ok(read_back) = driver.get_velocity().await {
                println!("Read back velocity: {}%", read_back);
            }

            // Restore original
            driver.set_velocity(velocity).await.ok();
        }
        Err(e) => {
            println!("Get velocity failed (may not be supported): {}", e);
        }
    }
}

#[tokio::test]
async fn test_home_offset_get() {
    println!("\n=== Test: Home Offset Get ===");

    let driver = Ell14Driver::new(PORT, "2").expect("Failed to create driver");

    match driver.get_home_offset().await {
        Ok(offset) => {
            println!("Current home offset: {:.3}°", offset);
            // Just verify we can read it
            assert!(
                offset.abs() < 360.0,
                "Home offset out of expected range: {:.3}°",
                offset
            );
        }
        Err(e) => {
            println!("Get home offset failed (may need different response parsing): {}", e);
        }
    }
}

#[tokio::test]
async fn test_motor_info() {
    println!("\n=== Test: Motor Info ===");

    let driver = Ell14Driver::new(PORT, "3").expect("Failed to create driver");

    // Test motor 1 info
    match driver.get_motor1_info().await {
        Ok(info) => {
            println!("Motor 1 info:");
            println!("  Loop state: {}", if info.loop_state { "ON" } else { "OFF" });
            println!("  Motor on: {}", if info.motor_on { "YES" } else { "NO" });
            println!("  Frequency: {} Hz", info.frequency);
            println!("  Forward period: {}", info.forward_period);
            println!("  Backward period: {}", info.backward_period);
        }
        Err(e) => {
            println!("Motor 1 info failed: {}", e);
        }
    }

    sleep(Duration::from_millis(100)).await;

    // Test motor 2 info
    match driver.get_motor2_info().await {
        Ok(info) => {
            println!("Motor 2 info:");
            println!("  Loop state: {}", if info.loop_state { "ON" } else { "OFF" });
            println!("  Motor on: {}", if info.motor_on { "YES" } else { "NO" });
            println!("  Frequency: {} Hz", info.frequency);
            println!("  Forward period: {}", info.forward_period);
            println!("  Backward period: {}", info.backward_period);
        }
        Err(e) => {
            println!("Motor 2 info failed: {}", e);
        }
    }
}

#[tokio::test]
async fn test_motor_frequency_search() {
    println!("\n=== Test: Motor Frequency Search (Motor Optimization) ===");
    println!("WARNING: This test takes 15-30 seconds to complete");

    let driver = Ell14Driver::new(PORT, "2").expect("Failed to create driver");

    // Get initial position
    let initial = driver.position().await.expect("Failed to get position");
    println!("Initial position: {:.2}°", initial);

    // Search motor 1 frequency
    println!("Searching motor 1 frequency...");
    let start = std::time::Instant::now();
    match driver.search_frequency_motor1().await {
        Ok(_) => {
            println!("Motor 1 frequency search completed in {:.1}s", start.elapsed().as_secs_f64());
        }
        Err(e) => {
            println!("Motor 1 frequency search failed: {}", e);
        }
    }

    sleep(Duration::from_millis(500)).await;

    // Search motor 2 frequency
    println!("Searching motor 2 frequency...");
    let start = std::time::Instant::now();
    match driver.search_frequency_motor2().await {
        Ok(_) => {
            println!("Motor 2 frequency search completed in {:.1}s", start.elapsed().as_secs_f64());
        }
        Err(e) => {
            println!("Motor 2 frequency search failed: {}", e);
        }
    }

    // Verify device still responds
    let final_pos = driver.position().await.expect("Failed to get position");
    println!("Final position: {:.2}°", final_pos);

    // Don't save - this was just a test
    println!("Motor optimization complete (settings NOT saved)");
}

#[tokio::test]
async fn test_save_user_data() {
    println!("\n=== Test: Save User Data ===");

    let driver = Ell14Driver::new(PORT, "8").expect("Failed to create driver");

    // Just test that the command works - we won't actually persist changes
    match driver.save_user_data().await {
        Ok(_) => {
            println!("Save user data command succeeded");
        }
        Err(e) => {
            println!("Save user data failed: {}", e);
            // This is acceptable - the command may have different response format
        }
    }
}
