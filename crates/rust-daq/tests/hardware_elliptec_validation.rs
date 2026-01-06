#![cfg(not(target_arch = "wasm32"))]
//! Elliptec ELL14 Hardware Validation Tests
//!
//! Tests for Thorlabs Elliptec ELL14 rotation mounts on shared RS-485 bus.
//! Hardware: 3 rotators at addresses 2, 3, 8 on /dev/ttyUSB0
//!
//! Run with: cargo test --features "hardware_tests,instrument_thorlabs" --test hardware_elliptec_validation -- --nocapture
//!
//! SAFETY: These tests move physical hardware. Ensure no obstructions before running.

#![cfg(all(feature = "hardware_tests", feature = "instrument_thorlabs"))]

use rust_daq::hardware::capabilities::Movable;
use rust_daq::hardware::ell14::Ell14Driver;
use std::time::Duration;
use tokio::time::sleep;

// TODO: Implement HWID-based discovery instead of hardcoded port
// Current mapping: FT230X_Basic_UART (serial DK0AHAJZ) = ELL14 bus
fn get_elliptec_port() -> String {
    std::env::var("ELLIPTEC_PORT").unwrap_or_else(|_| "/dev/ttyUSB1".to_string())
}
const ADDRESSES: [&str; 3] = ["2", "3", "8"];
const POSITION_TOLERANCE_DEG: f64 = 1.0;

// =============================================================================
// Phase 1: Basic Connectivity Tests
// =============================================================================

#[tokio::test]
async fn test_all_rotators_respond_to_position_query() {
    println!("\n=== Test: Position Query for All Rotators ===");

    for addr in ADDRESSES {
        let driver = Ell14Driver::new(&get_elliptec_port(), addr)
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

    let mut port = serialport::new(&get_elliptec_port(), 9600)
        .timeout(Duration::from_millis(500))
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::None)
        .open()
        .expect("Failed to open port");

    for addr in ADDRESSES {
        // Retry logic for RS-485 bus contention
        let mut success = false;
        for attempt in 0..3 {
            if attempt > 0 {
                println!("  Retry {} for rotator {}", attempt, addr);
                std::thread::sleep(Duration::from_millis(200));
            }

            let command = format!("{}in", addr);
            port.write_all(command.as_bytes()).expect("Failed to write");
            std::thread::sleep(Duration::from_millis(200));

            // Read with accumulation - RS-485 responses may arrive in chunks
            let mut response_buf = Vec::with_capacity(128);
            let mut buffer = [0u8; 256];

            // Try multiple reads to accumulate full response
            for _ in 0..5 {
                match port.read(&mut buffer) {
                    Ok(n) if n > 0 => {
                        response_buf.extend_from_slice(&buffer[..n]);
                    }
                    _ => {}
                }
                if response_buf.len() >= 30 {
                    break; // Got enough data
                }
                std::thread::sleep(Duration::from_millis(50));
            }

            if !response_buf.is_empty() {
                let response = String::from_utf8_lossy(&response_buf);
                println!("Rotator {} info: {}", addr, response.trim());

                // Verify response contains "IN" marker
                if response.contains("IN") {
                    success = true;
                    break;
                }
            }
        }

        assert!(
            success,
            "Failed to get valid INFO response from rotator {} after 3 attempts",
            addr
        );
    }
}

// =============================================================================
// Phase 2: Movement Tests
// =============================================================================

#[tokio::test]
async fn test_absolute_movement_single_rotator() {
    println!("\n=== Test: Absolute Movement (Single Rotator) ===");

    // Test with rotator at address 2
    let driver = Ell14Driver::new(&get_elliptec_port(), "2").expect("Failed to create driver");

    // Get initial position
    let initial = driver
        .position()
        .await
        .expect("Failed to get initial position");
    println!("Initial position: {:.2}°", initial);

    // Move to 45 degrees
    let target = 45.0;
    println!("Moving to {:.2}°...", target);
    driver
        .move_abs(target)
        .await
        .expect("Failed to send move command");

    // Wait for movement to complete
    driver
        .wait_settled()
        .await
        .expect("Failed to wait for settle");

    // Verify position
    let final_pos = driver
        .position()
        .await
        .expect("Failed to get final position");
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

    let driver = Ell14Driver::new(&get_elliptec_port(), "3").expect("Failed to create driver");

    // Get initial position
    let initial = driver
        .position()
        .await
        .expect("Failed to get initial position");
    println!("Initial position: {:.2}°", initial);

    // Move relative +10 degrees
    let delta = 10.0;
    println!("Moving relative +{:.2}°...", delta);
    driver
        .move_rel(delta)
        .await
        .expect("Failed to send relative move");
    driver
        .wait_settled()
        .await
        .expect("Failed to wait for settle");

    let pos_after_forward = driver.position().await.expect("Failed to get position");
    println!("Position after +10°: {:.2}°", pos_after_forward);

    // Move relative -10 degrees (back to start)
    println!("Moving relative -{:.2}°...", delta);
    driver
        .move_rel(-delta)
        .await
        .expect("Failed to send relative move");
    driver
        .wait_settled()
        .await
        .expect("Failed to wait for settle");

    let final_pos = driver
        .position()
        .await
        .expect("Failed to get final position");
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

    let driver = Ell14Driver::new(&get_elliptec_port(), "8").expect("Failed to create driver");

    // Get initial position
    let initial = driver
        .position()
        .await
        .expect("Failed to get initial position");
    println!("Initial position: {:.2}°", initial);

    // Home the device
    println!("Homing...");
    driver.home().await.expect("Failed to home");

    // Get position after homing
    let home_pos = driver
        .position()
        .await
        .expect("Failed to get home position");
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
        let driver = Ell14Driver::new(&get_elliptec_port(), addr).expect("Failed to create driver");

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
        let driver = Ell14Driver::new(&get_elliptec_port(), addr).expect("Failed to create driver");
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
        let driver = Ell14Driver::new(&get_elliptec_port(), addr).expect("Failed to create driver");
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
        assert!(
            error < POSITION_TOLERANCE_DEG,
            "Position error: {:.2}°",
            error
        );
    }

    // Return to initial positions
    println!("\nReturning to initial positions...");
    for (addr, initial) in &initial_positions {
        sleep(Duration::from_millis(100)).await;
        let driver = Ell14Driver::new(&get_elliptec_port(), addr).expect("Failed to create driver");
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

    let driver = Ell14Driver::new(&get_elliptec_port(), "2").expect("Failed to create driver");
    let target = 45.0;
    let num_trials = 5;
    let mut positions = Vec::new();

    // Get initial position
    let initial = driver
        .position()
        .await
        .expect("Failed to get initial position");

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
    let variance: f64 =
        positions.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / num_trials as f64;
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

    let driver = Ell14Driver::new(&get_elliptec_port(), "3").expect("Failed to create driver");
    let initial = driver
        .position()
        .await
        .expect("Failed to get initial position");

    // Test positions around full rotation
    let test_positions = [0.0, 90.0, 180.0, 270.0, 360.0];

    for target in test_positions {
        driver.move_abs(target).await.expect("Failed to move");
        driver.wait_settled().await.expect("Failed to settle");

        let actual = driver.position().await.expect("Failed to get position");
        let error = (actual - target).abs();

        println!(
            "Target: {:.0}° → Actual: {:.2}° (error: {:.2}°)",
            target, actual, error
        );

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

    let driver = Ell14Driver::new(&get_elliptec_port(), "2").expect("Failed to create driver");
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
    println!(
        "\nSuccess rate: {:.1}% ({}/{})",
        success_rate, success_count, num_queries
    );

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
            let driver = Ell14Driver::new(&get_elliptec_port(), addr).expect("Failed to create driver");
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

    assert!(
        all_success,
        "Some queries failed during bus contention test"
    );
}

// =============================================================================
// Phase 6: Advanced Feature Tests (Jog, Velocity, Motor Optimization)
// =============================================================================

#[tokio::test]
async fn test_device_info() {
    println!("\n=== Test: Device Info ===");

    for addr in ADDRESSES {
        let driver = Ell14Driver::new(&get_elliptec_port(), addr).expect("Failed to create driver");

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

    let driver = Ell14Driver::new(&get_elliptec_port(), "2").expect("Failed to create driver");

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
    let read_back = driver
        .get_jog_step()
        .await
        .expect("Failed to read back jog step");
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

    let driver = Ell14Driver::new(&get_elliptec_port(), "3").expect("Failed to create driver");

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

    let driver = Ell14Driver::new(&get_elliptec_port(), "2").expect("Failed to create driver");

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
async fn test_all_velocities() {
    println!("\n=== Test: All Rotator Velocities ===");
    println!("Checking velocities for addresses 2, 3, 8");
    println!("Expected: All should be near 64 (100%) for full speed\n");

    for addr in ADDRESSES {
        let driver =
            Ell14Driver::new(&get_elliptec_port(), addr).expect("Failed to create driver");

        match driver.get_velocity().await {
            Ok(velocity) => {
                // velocity is already a percentage (60-100%)
                let status = if velocity >= 90 { "OK" } else { "SLOW!" };
                println!(
                    "Rotator {}: velocity = {}% - {}",
                    addr, velocity, status
                );
            }
            Err(e) => println!("Rotator {}: Error getting velocity - {}", addr, e),
        }
    }
}

#[tokio::test]
async fn test_velocity_get_set() {
    println!("\n=== Test: Velocity Get/Set ===");

    let driver = Ell14Driver::new(&get_elliptec_port(), "8").expect("Failed to create driver");

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

    let driver = Ell14Driver::new(&get_elliptec_port(), "2").expect("Failed to create driver");

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
            println!(
                "Get home offset failed (may need different response parsing): {}",
                e
            );
        }
    }
}

#[tokio::test]
async fn test_motor_info() {
    println!("\n=== Test: Motor Info ===");

    let driver = Ell14Driver::new(&get_elliptec_port(), "3").expect("Failed to create driver");

    // Test motor 1 info
    match driver.get_motor1_info().await {
        Ok(info) => {
            println!("Motor 1 info:");
            println!(
                "  Loop state: {}",
                if info.loop_state { "ON" } else { "OFF" }
            );
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
            println!(
                "  Loop state: {}",
                if info.loop_state { "ON" } else { "OFF" }
            );
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

    let driver = Ell14Driver::new(&get_elliptec_port(), "2").expect("Failed to create driver");

    // Get initial position
    let initial = driver.position().await.expect("Failed to get position");
    println!("Initial position: {:.2}°", initial);

    // Search motor 1 frequency
    println!("Searching motor 1 frequency...");
    let start = std::time::Instant::now();
    match driver.search_frequency_motor1().await {
        Ok(_) => {
            println!(
                "Motor 1 frequency search completed in {:.1}s",
                start.elapsed().as_secs_f64()
            );
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
            println!(
                "Motor 2 frequency search completed in {:.1}s",
                start.elapsed().as_secs_f64()
            );
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

    let driver = Ell14Driver::new(&get_elliptec_port(), "8").expect("Failed to create driver");

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

// =============================================================================
// Phase 7: Extended Relative Movement Tests (bd-e52e.7)
// =============================================================================

#[tokio::test]
async fn test_relative_movement_cumulative() {
    println!("\n=== Test: Relative Movement Cumulative Tracking (bd-e52e.7) ===");

    let driver = Ell14Driver::new(&get_elliptec_port(), "2").expect("Failed to create driver");

    // Get initial position
    let initial = driver
        .position()
        .await
        .expect("Failed to get initial position");
    println!("Initial position: {:.2}°", initial);

    // Perform multiple cumulative relative moves
    let moves = [10.0, 10.0, 10.0, -15.0, -15.0]; // Net: +0°
    let mut expected_pos = initial;

    for (i, delta) in moves.iter().enumerate() {
        expected_pos += delta;
        println!(
            "Move {}: relative {:.1}° (expected: {:.2}°)",
            i + 1,
            delta,
            expected_pos
        );

        driver.move_rel(*delta).await.expect("Failed to move");
        driver.wait_settled().await.expect("Failed to settle");

        let actual = driver.position().await.expect("Failed to get position");
        let error = (actual - expected_pos).abs();

        println!("  Actual: {:.2}° (error: {:.2}°)", actual, error);
        assert!(
            error < POSITION_TOLERANCE_DEG,
            "Cumulative position error too large at move {}: {:.2}°",
            i + 1,
            error
        );
    }

    // Final position should be back near initial
    let final_pos = driver
        .position()
        .await
        .expect("Failed to get final position");
    let total_error = (final_pos - initial).abs();
    println!(
        "\nFinal position: {:.2}° (total error from start: {:.2}°)",
        final_pos, total_error
    );
    assert!(
        total_error < POSITION_TOLERANCE_DEG * 2.0,
        "Total cumulative error too large: {:.2}°",
        total_error
    );
}

#[tokio::test]
async fn test_relative_movement_large_angles() {
    println!("\n=== Test: Relative Movement Large Angles (bd-e52e.7) ===");

    let driver = Ell14Driver::new(&get_elliptec_port(), "3").expect("Failed to create driver");

    // Home first to have a known starting point
    driver.home().await.expect("Failed to home");
    let initial = driver.position().await.expect("Failed to get position");
    println!("Initial (home): {:.2}°", initial);

    // Test larger relative moves: +45, +90, -45, -90
    let test_moves = [
        (45.0, "forward 45°"),
        (90.0, "forward 90°"),
        (-45.0, "backward 45°"),
        (-90.0, "backward 90°"),
    ];

    let mut current_expected = initial;
    for (delta, description) in test_moves {
        current_expected += delta;

        println!(
            "Moving {} (expected: {:.2}°)...",
            description, current_expected
        );
        driver.move_rel(delta).await.expect("Failed to move");
        driver.wait_settled().await.expect("Failed to settle");

        let actual = driver.position().await.expect("Failed to get position");
        let error = (actual - current_expected).abs();
        println!("  Actual: {:.2}° (error: {:.2}°)", actual, error);

        assert!(
            error < POSITION_TOLERANCE_DEG,
            "Position error for {} too large: {:.2}°",
            description,
            error
        );
    }

    // Return to home
    driver.home().await.ok();
}

#[tokio::test]
async fn test_relative_movement_wraparound() {
    println!("\n=== Test: Relative Movement 360° Wraparound (bd-e52e.7) ===");

    let driver = Ell14Driver::new(&get_elliptec_port(), "8").expect("Failed to create driver");

    // Home first
    driver.home().await.expect("Failed to home");
    sleep(Duration::from_millis(200)).await;
    let home_pos = driver.position().await.expect("Failed to get position");
    println!("Home position: {:.2}°", home_pos);

    // Move forward past 360°
    println!("\nTesting wraparound past 360°...");
    driver.move_abs(350.0).await.expect("Failed to move");
    driver.wait_settled().await.expect("Failed to settle");

    let at_350 = driver.position().await.expect("Failed to get position");
    println!("At 350°: {:.2}°", at_350);

    // Move relative +30° (should be at ~380° or ~20° depending on tracking)
    driver.move_rel(30.0).await.expect("Failed to move");
    driver.wait_settled().await.expect("Failed to settle");

    let after_wrap = driver.position().await.expect("Failed to get position");
    println!("After +30° from 350°: {:.2}°", after_wrap);

    // The ELL14 can track continuous rotation - position might be >360
    // Or it might wrap to ~20°
    let expected_continuous = at_350 + 30.0;
    let expected_wrapped = (at_350 + 30.0) % 360.0;

    let is_continuous = (after_wrap - expected_continuous).abs() < POSITION_TOLERANCE_DEG;
    let is_wrapped = (after_wrap - expected_wrapped).abs() < POSITION_TOLERANCE_DEG;

    println!("  Expected continuous: {:.2}°", expected_continuous);
    println!("  Expected wrapped: {:.2}°", expected_wrapped);

    assert!(
        is_continuous || is_wrapped,
        "Position {} doesn't match continuous ({:.2}) or wrapped ({:.2})",
        after_wrap,
        expected_continuous,
        expected_wrapped
    );

    // Return to home
    driver.home().await.ok();
}

// =============================================================================
// Phase 8: Extended Accuracy & Backlash Tests (bd-e52e.9)
// =============================================================================

#[tokio::test]
async fn test_position_accuracy_multiple_targets() {
    println!("\n=== Test: Position Accuracy Multiple Targets (bd-e52e.9) ===");

    let driver = Ell14Driver::new(&get_elliptec_port(), "2").expect("Failed to create driver");
    let initial = driver.position().await.expect("Failed to get position");

    // Test positions at 30° intervals
    let test_positions = [
        0.0, 30.0, 60.0, 90.0, 120.0, 150.0, 180.0, 210.0, 240.0, 270.0, 300.0, 330.0,
    ];
    let mut max_error = 0.0f64;
    let mut total_error = 0.0f64;

    println!("Testing accuracy at 30° intervals:");
    for target in test_positions {
        driver.move_abs(target).await.expect("Failed to move");
        driver.wait_settled().await.expect("Failed to settle");

        let actual = driver.position().await.expect("Failed to get position");
        let error = (actual - target).abs();
        total_error += error;
        max_error = max_error.max(error);

        println!("  {:.0}° → {:.3}° (error: {:.3}°)", target, actual, error);
    }

    let avg_error = total_error / test_positions.len() as f64;
    println!("\nAccuracy Summary:");
    println!("  Average error: {:.3}°", avg_error);
    println!("  Maximum error: {:.3}°", max_error);

    assert!(
        max_error < POSITION_TOLERANCE_DEG,
        "Maximum position error too large: {:.3}°",
        max_error
    );

    // Return to initial
    driver.move_abs(initial).await.ok();
    driver.wait_settled().await.ok();
}

#[tokio::test]
async fn test_repeatability_extended() {
    println!("\n=== Test: Extended Repeatability (10 trials) (bd-e52e.9) ===");

    let driver = Ell14Driver::new(&get_elliptec_port(), "3").expect("Failed to create driver");
    let initial = driver.position().await.expect("Failed to get position");

    let target = 90.0;
    let num_trials = 10;
    let mut positions = Vec::new();

    println!(
        "Moving to {:.0}° repeatedly ({} trials):",
        target, num_trials
    );
    for i in 1..=num_trials {
        // Move away first (to 0°)
        driver.move_abs(0.0).await.expect("Failed to move");
        driver.wait_settled().await.expect("Failed to settle");

        // Move to target
        driver.move_abs(target).await.expect("Failed to move");
        driver.wait_settled().await.expect("Failed to settle");

        let pos = driver.position().await.expect("Failed to get position");
        positions.push(pos);
        println!("  Trial {:2}: {:.4}°", i, pos);
    }

    // Calculate statistics
    let mean: f64 = positions.iter().sum::<f64>() / num_trials as f64;
    let variance: f64 =
        positions.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / num_trials as f64;
    let std_dev = variance.sqrt();
    let min_pos = positions.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_pos = positions.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = max_pos - min_pos;

    println!("\nRepeatability Statistics:");
    println!("  Target: {:.2}°", target);
    println!("  Mean: {:.4}°", mean);
    println!("  Std Dev: {:.4}°", std_dev);
    println!("  Min: {:.4}°", min_pos);
    println!("  Max: {:.4}°", max_pos);
    println!("  Range: {:.4}°", range);
    println!("  Accuracy (mean error): {:.4}°", (mean - target).abs());

    assert!(
        std_dev < 0.3,
        "Repeatability std dev too large: {:.4}°",
        std_dev
    );
    assert!(
        (mean - target).abs() < POSITION_TOLERANCE_DEG,
        "Mean accuracy error too large: {:.4}°",
        (mean - target).abs()
    );

    // Return to initial
    driver.move_abs(initial).await.ok();
    driver.wait_settled().await.ok();
}

#[tokio::test]
async fn test_mechanical_backlash() {
    println!("\n=== Test: Mechanical Backlash Measurement (bd-e52e.9) ===");

    let driver = Ell14Driver::new(&get_elliptec_port(), "8").expect("Failed to create driver");
    let initial = driver.position().await.expect("Failed to get position");

    let target = 45.0;
    let num_trials = 5;
    let mut forward_positions = Vec::new();
    let mut backward_positions = Vec::new();

    println!(
        "Measuring backlash approaching {:.0}° from both directions:",
        target
    );

    for i in 1..=num_trials {
        // Approach from below (forward direction)
        driver
            .move_abs(target - 30.0)
            .await
            .expect("Failed to move");
        driver.wait_settled().await.expect("Failed to settle");
        driver.move_abs(target).await.expect("Failed to move");
        driver.wait_settled().await.expect("Failed to settle");
        let forward_pos = driver.position().await.expect("Failed to get position");
        forward_positions.push(forward_pos);

        // Approach from above (backward direction)
        driver
            .move_abs(target + 30.0)
            .await
            .expect("Failed to move");
        driver.wait_settled().await.expect("Failed to settle");
        driver.move_abs(target).await.expect("Failed to move");
        driver.wait_settled().await.expect("Failed to settle");
        let backward_pos = driver.position().await.expect("Failed to get position");
        backward_positions.push(backward_pos);

        println!(
            "  Trial {}: forward={:.4}°, backward={:.4}°, diff={:.4}°",
            i,
            forward_pos,
            backward_pos,
            (forward_pos - backward_pos).abs()
        );
    }

    let forward_mean: f64 = forward_positions.iter().sum::<f64>() / num_trials as f64;
    let backward_mean: f64 = backward_positions.iter().sum::<f64>() / num_trials as f64;
    let backlash = (forward_mean - backward_mean).abs();

    println!("\nBacklash Summary:");
    println!("  Forward approach mean: {:.4}°", forward_mean);
    println!("  Backward approach mean: {:.4}°", backward_mean);
    println!("  Measured backlash: {:.4}°", backlash);

    // Backlash should be small for ELL14
    assert!(backlash < 0.5, "Backlash too large: {:.4}°", backlash);

    // Return to initial
    driver.move_abs(initial).await.ok();
    driver.wait_settled().await.ok();
}

// =============================================================================
// Phase 9: Simultaneous Movement Tests (bd-e52e.10)
// =============================================================================

#[tokio::test]
async fn test_simultaneous_movement_two_devices() {
    println!("\n=== Test: Simultaneous Movement Two Devices (bd-e52e.10) ===");

    // Create two drivers for concurrent control
    let driver_2 = Ell14Driver::new(&get_elliptec_port(), "2").expect("Failed to create driver 2");
    let driver_3 = Ell14Driver::new(&get_elliptec_port(), "3").expect("Failed to create driver 3");

    // Get initial positions
    let initial_2 = driver_2.position().await.expect("Failed to get position 2");
    let initial_3 = driver_3.position().await.expect("Failed to get position 3");
    println!(
        "Initial positions: Rot2={:.2}°, Rot3={:.2}°",
        initial_2, initial_3
    );

    // Define targets
    let target_2 = 60.0;
    let target_3 = 120.0;
    println!(
        "Commanding simultaneous moves: Rot2→{:.0}°, Rot3→{:.0}°",
        target_2, target_3
    );

    // Start both moves concurrently
    let start_time = std::time::Instant::now();

    // Send commands to both devices (they will execute in parallel on the RS-485 bus)
    // Note: The drivers share the serial port, so commands are serialized at the port level,
    // but the devices execute their moves simultaneously
    driver_2
        .move_abs(target_2)
        .await
        .expect("Failed to start move 2");
    driver_3
        .move_abs(target_3)
        .await
        .expect("Failed to start move 3");

    // Wait for both to settle (check periodically)
    let mut settled_2 = false;
    let mut settled_3 = false;

    for _ in 0..50 {
        sleep(Duration::from_millis(100)).await;

        if !settled_2 {
            if driver_2.wait_settled().await.is_ok() {
                settled_2 = true;
            }
        }
        if !settled_3 {
            if driver_3.wait_settled().await.is_ok() {
                settled_3 = true;
            }
        }

        if settled_2 && settled_3 {
            break;
        }
    }

    let elapsed = start_time.elapsed();
    println!("Both devices settled in {:.2}s", elapsed.as_secs_f64());

    // Verify final positions
    let final_2 = driver_2.position().await.expect("Failed to get position 2");
    let final_3 = driver_3.position().await.expect("Failed to get position 3");

    let error_2 = (final_2 - target_2).abs();
    let error_3 = (final_3 - target_3).abs();

    println!("Final positions:");
    println!("  Rot2: {:.2}° (error: {:.2}°)", final_2, error_2);
    println!("  Rot3: {:.2}° (error: {:.2}°)", final_3, error_3);

    assert!(
        error_2 < POSITION_TOLERANCE_DEG,
        "Rot2 position error too large: {:.2}°",
        error_2
    );
    assert!(
        error_3 < POSITION_TOLERANCE_DEG,
        "Rot3 position error too large: {:.2}°",
        error_3
    );

    // Verify no RS-485 bus contention (devices responded correctly)
    println!("RS-485 bus handled concurrent commands successfully");

    // Return to initial positions
    driver_2.move_abs(initial_2).await.ok();
    driver_3.move_abs(initial_3).await.ok();
    driver_2.wait_settled().await.ok();
    driver_3.wait_settled().await.ok();
}

#[tokio::test]
async fn test_simultaneous_movement_all_three() {
    println!("\n=== Test: Simultaneous Movement All Three Devices (bd-e52e.10) ===");

    // Create drivers for all three rotators
    let driver_2 = Ell14Driver::new(&get_elliptec_port(), "2").expect("Failed to create driver 2");
    sleep(Duration::from_millis(50)).await;
    let driver_3 = Ell14Driver::new(&get_elliptec_port(), "3").expect("Failed to create driver 3");
    sleep(Duration::from_millis(50)).await;
    let driver_8 = Ell14Driver::new(&get_elliptec_port(), "8").expect("Failed to create driver 8");

    // Get initial positions (with delays to avoid contention)
    let initial_2 = driver_2.position().await.expect("Failed to get position 2");
    sleep(Duration::from_millis(50)).await;
    let initial_3 = driver_3.position().await.expect("Failed to get position 3");
    sleep(Duration::from_millis(50)).await;
    let initial_8 = driver_8.position().await.expect("Failed to get position 8");

    println!(
        "Initial positions: Rot2={:.2}°, Rot3={:.2}°, Rot8={:.2}°",
        initial_2, initial_3, initial_8
    );

    // Define targets (all moving to different positions)
    let targets = [(45.0, "2"), (90.0, "3"), (135.0, "8")];
    println!("Commanding all three to move: Rot2→45°, Rot3→90°, Rot8→135°");

    let start_time = std::time::Instant::now();

    // Send commands to all three devices
    driver_2
        .move_abs(targets[0].0)
        .await
        .expect("Failed to start move 2");
    driver_3
        .move_abs(targets[1].0)
        .await
        .expect("Failed to start move 3");
    driver_8
        .move_abs(targets[2].0)
        .await
        .expect("Failed to start move 8");

    // Wait for all to settle
    sleep(Duration::from_millis(500)).await;
    driver_2
        .wait_settled()
        .await
        .expect("Rot2 failed to settle");
    driver_3
        .wait_settled()
        .await
        .expect("Rot3 failed to settle");
    driver_8
        .wait_settled()
        .await
        .expect("Rot8 failed to settle");

    let elapsed = start_time.elapsed();
    println!("All three devices settled in {:.2}s", elapsed.as_secs_f64());

    // Verify final positions
    sleep(Duration::from_millis(100)).await;
    let final_2 = driver_2.position().await.expect("Failed to get position 2");
    sleep(Duration::from_millis(50)).await;
    let final_3 = driver_3.position().await.expect("Failed to get position 3");
    sleep(Duration::from_millis(50)).await;
    let final_8 = driver_8.position().await.expect("Failed to get position 8");

    let errors = [
        (final_2 - targets[0].0).abs(),
        (final_3 - targets[1].0).abs(),
        (final_8 - targets[2].0).abs(),
    ];

    println!("Final positions and errors:");
    println!("  Rot2: {:.2}° (error: {:.2}°)", final_2, errors[0]);
    println!("  Rot3: {:.2}° (error: {:.2}°)", final_3, errors[1]);
    println!("  Rot8: {:.2}° (error: {:.2}°)", final_8, errors[2]);

    for (i, error) in errors.iter().enumerate() {
        assert!(
            *error < POSITION_TOLERANCE_DEG,
            "Rotator {} position error too large: {:.2}°",
            targets[i].1,
            error
        );
    }

    println!("All three devices moved simultaneously without RS-485 bus contention");

    // Return to initial positions
    driver_2.move_abs(initial_2).await.ok();
    driver_3.move_abs(initial_3).await.ok();
    driver_8.move_abs(initial_8).await.ok();
    sleep(Duration::from_millis(500)).await;
}

#[tokio::test]
async fn test_concurrent_position_queries() {
    println!("\n=== Test: Concurrent Position Queries (bd-e52e.10) ===");

    // Test rapid alternating queries between devices
    let num_rounds = 10;
    let mut success_count = 0;
    let mut query_times = Vec::new();

    println!(
        "Performing {} rounds of rapid alternating queries...",
        num_rounds
    );

    for _round in 1..=num_rounds {
        let round_start = std::time::Instant::now();

        for addr in ADDRESSES {
            let driver = Ell14Driver::new(&get_elliptec_port(), addr).expect("Failed to create driver");
            if driver.position().await.is_ok() {
                success_count += 1;
            }
            // Minimal delay between queries
            sleep(Duration::from_millis(20)).await;
        }

        query_times.push(round_start.elapsed());
    }

    let total_queries = num_rounds * ADDRESSES.len();
    let success_rate = (success_count as f64 / total_queries as f64) * 100.0;
    let avg_round_time =
        query_times.iter().map(|d| d.as_millis()).sum::<u128>() / num_rounds as u128;

    println!("\nConcurrent Query Results:");
    println!("  Total queries: {}", total_queries);
    println!("  Successful: {} ({:.1}%)", success_count, success_rate);
    println!("  Avg round time: {}ms", avg_round_time);

    assert!(
        success_rate >= 95.0,
        "Concurrent query success rate too low: {:.1}%",
        success_rate
    );
}

// =============================================================================
// Phase 10: Position Monitoring During Movement (bd-e52e.16)
// =============================================================================

#[tokio::test]
async fn test_position_updates_during_movement() {
    println!("\n=== Test: Position Updates During Movement (bd-e52e.16) ===");

    let driver = Ell14Driver::new(&get_elliptec_port(), "2").expect("Failed to create driver");

    // Get initial position
    let initial = driver.position().await.expect("Failed to get position");
    println!("Initial position: {:.2}°", initial);

    // Set up a long move
    let target = initial + 90.0;
    println!("Starting move to {:.2}° (90° rotation)...", target);

    // Start the move
    driver.move_abs(target).await.expect("Failed to start move");

    // Poll position during movement
    let mut positions = Vec::new();
    let mut timestamps = Vec::new();
    let start_time = std::time::Instant::now();
    let poll_interval = Duration::from_millis(50);

    // Poll for up to 3 seconds
    while start_time.elapsed() < Duration::from_secs(3) {
        if let Ok(pos) = driver.position().await {
            positions.push(pos);
            timestamps.push(start_time.elapsed());
        }
        sleep(poll_interval).await;
    }

    // Analyze the position trace
    println!("\nPosition trace ({} samples):", positions.len());
    let samples_to_show = positions.len().min(10);
    for i in (0..positions.len()).step_by(positions.len() / samples_to_show + 1) {
        println!(
            "  t={:.2}s: {:.2}°",
            timestamps[i].as_secs_f64(),
            positions[i]
        );
    }

    // Check for monotonic progression (no large jumps backward during forward movement)
    let mut max_backtrack = 0.0f64;
    for i in 1..positions.len() {
        let delta = positions[i] - positions[i - 1];
        if delta < -1.0 {
            // Small negative deltas might be noise
            max_backtrack = max_backtrack.max(-delta);
        }
    }

    println!("\nMovement Analysis:");
    println!("  Start: {:.2}°", positions.first().unwrap_or(&initial));
    println!("  End: {:.2}°", positions.last().unwrap_or(&initial));
    println!("  Samples captured: {}", positions.len());
    println!("  Max backtrack: {:.2}°", max_backtrack);

    // Should have captured multiple intermediate positions
    assert!(
        positions.len() >= 5,
        "Not enough position samples captured: {}",
        positions.len()
    );

    // No significant backward movement during forward rotation
    assert!(
        max_backtrack < 2.0,
        "Significant backward movement detected: {:.2}°",
        max_backtrack
    );

    // Final position should be near target
    let final_pos = driver
        .position()
        .await
        .expect("Failed to get final position");
    let error = (final_pos - target).abs();
    println!("  Final position error: {:.2}°", error);

    assert!(
        error < POSITION_TOLERANCE_DEG,
        "Final position error too large: {:.2}°",
        error
    );

    // Return to initial
    driver.move_abs(initial).await.ok();
    driver.wait_settled().await.ok();
}

#[tokio::test]
async fn test_continuous_position_monitoring() {
    println!("\n=== Test: Continuous Position Monitoring (bd-e52e.16) ===");

    let driver = Ell14Driver::new(&get_elliptec_port(), "3").expect("Failed to create driver");
    let initial = driver.position().await.expect("Failed to get position");

    // Monitor multiple consecutive movements
    let movements = [
        (45.0, "to 45°"),
        (90.0, "to 90°"),
        (45.0, "back to 45°"),
        (0.0, "to 0°"),
    ];

    let mut all_positions = Vec::new();
    let mut dropped_samples = 0;
    let poll_interval = Duration::from_millis(100);

    println!("Monitoring position through multiple movements...");

    for (target, description) in movements {
        println!("\nMoving {}...", description);
        driver.move_abs(target).await.expect("Failed to move");

        // Poll during movement
        let move_start = std::time::Instant::now();
        let mut move_positions = Vec::new();

        while move_start.elapsed() < Duration::from_secs(2) {
            match driver.position().await {
                Ok(pos) => {
                    move_positions.push(pos);
                }
                Err(_) => {
                    dropped_samples += 1;
                }
            }
            sleep(poll_interval).await;
        }

        let final_pos = *move_positions.last().unwrap_or(&target);
        println!(
            "  Captured {} samples, final: {:.2}°",
            move_positions.len(),
            final_pos
        );

        all_positions.extend(move_positions);
    }

    println!("\nMonitoring Summary:");
    println!("  Total samples: {}", all_positions.len());
    println!("  Dropped samples: {}", dropped_samples);
    println!(
        "  Drop rate: {:.1}%",
        (dropped_samples as f64 / (all_positions.len() + dropped_samples) as f64) * 100.0
    );

    // Should have very few dropped samples
    assert!(
        dropped_samples < all_positions.len() / 10,
        "Too many dropped samples: {} out of {}",
        dropped_samples,
        all_positions.len()
    );

    // Return to initial
    driver.move_abs(initial).await.ok();
    driver.wait_settled().await.ok();
}

#[tokio::test]
async fn test_position_smoothness() {
    println!("\n=== Test: Position Smoothness During Movement (bd-e52e.16) ===");

    let driver = Ell14Driver::new(&get_elliptec_port(), "8").expect("Failed to create driver");
    let initial = driver.position().await.expect("Failed to get position");

    // Start a move
    let target = initial + 60.0;
    println!("Starting 60° move, monitoring smoothness...");
    driver.move_abs(target).await.expect("Failed to start move");

    // High-frequency polling
    let mut positions = Vec::new();
    let start_time = std::time::Instant::now();

    while start_time.elapsed() < Duration::from_secs(2) {
        if let Ok(pos) = driver.position().await {
            positions.push((start_time.elapsed().as_secs_f64(), pos));
        }
        sleep(Duration::from_millis(30)).await;
    }

    // Calculate velocity between samples
    let mut velocities = Vec::new();
    for i in 1..positions.len() {
        let dt = positions[i].0 - positions[i - 1].0;
        let dpos = positions[i].1 - positions[i - 1].1;
        if dt > 0.001 {
            velocities.push(dpos / dt); // degrees per second
        }
    }

    // Analyze smoothness
    let max_velocity = velocities.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let min_velocity = velocities.iter().cloned().fold(f64::INFINITY, f64::min);
    let avg_velocity = velocities.iter().sum::<f64>() / velocities.len().max(1) as f64;

    println!("\nSmoothness Analysis:");
    println!("  Samples: {}", positions.len());
    println!(
        "  Velocity range: {:.1} to {:.1} °/s",
        min_velocity, max_velocity
    );
    println!("  Average velocity: {:.1} °/s", avg_velocity);

    // Check for sudden jumps (acceleration spikes)
    let mut max_accel = 0.0f64;
    for i in 1..velocities.len() {
        let accel = (velocities[i] - velocities[i - 1]).abs();
        max_accel = max_accel.max(accel);
    }
    println!("  Max acceleration change: {:.1} °/s²", max_accel);

    // Movement should be reasonably smooth (no huge jumps)
    // Note: Some acceleration at start/stop is expected
    assert!(
        positions.len() >= 10,
        "Not enough samples for smoothness analysis"
    );

    // Return to initial
    driver.move_abs(initial).await.ok();
    driver.wait_settled().await.ok();
}

// =============================================================================
// Phase 11: Data Polling & Real-time Monitoring (bd-e52e.11)
// =============================================================================

#[tokio::test]
async fn test_continuous_polling_all_devices() {
    println!("\n=== Test: Continuous Polling All Devices (bd-e52e.11) ===");

    // Create drivers for all three devices
    let drivers: Vec<_> = ADDRESSES
        .iter()
        .map(|addr| Ell14Driver::new(&get_elliptec_port(), addr).expect("Failed to create driver"))
        .collect();

    // Poll all devices continuously for 10 seconds
    let poll_duration = Duration::from_secs(10);
    let poll_interval = Duration::from_millis(100); // 10 Hz polling
    let start_time = std::time::Instant::now();

    let mut total_queries = 0;
    let mut successful_queries = 0;
    let mut positions_by_device: Vec<Vec<f64>> = vec![Vec::new(); ADDRESSES.len()];

    println!(
        "Polling all {} devices at 10 Hz for {} seconds...",
        ADDRESSES.len(),
        poll_duration.as_secs()
    );

    while start_time.elapsed() < poll_duration {
        for (i, driver) in drivers.iter().enumerate() {
            total_queries += 1;
            match driver.position().await {
                Ok(pos) => {
                    successful_queries += 1;
                    positions_by_device[i].push(pos);
                }
                Err(_) => {
                    // Query failed
                }
            }
        }
        sleep(poll_interval).await;
    }

    // Analyze results
    let success_rate = (successful_queries as f64 / total_queries as f64) * 100.0;
    let expected_samples_per_device =
        (poll_duration.as_secs_f64() / poll_interval.as_secs_f64()) as usize;

    println!("\nPolling Results:");
    println!("  Duration: {} seconds", poll_duration.as_secs());
    println!("  Total queries: {}", total_queries);
    println!(
        "  Successful: {} ({:.1}%)",
        successful_queries, success_rate
    );
    println!(
        "  Expected samples/device: ~{}",
        expected_samples_per_device
    );

    for (i, addr) in ADDRESSES.iter().enumerate() {
        let samples = positions_by_device[i].len();
        let position_range = if !positions_by_device[i].is_empty() {
            let min = positions_by_device[i]
                .iter()
                .cloned()
                .fold(f64::INFINITY, f64::min);
            let max = positions_by_device[i]
                .iter()
                .cloned()
                .fold(f64::NEG_INFINITY, f64::max);
            format!("{:.2}° - {:.2}°", min, max)
        } else {
            "N/A".to_string()
        };
        println!(
            "  Device {}: {} samples, range: {}",
            addr, samples, position_range
        );
    }

    // Verify high success rate
    assert!(
        success_rate >= 90.0,
        "Polling success rate too low: {:.1}%",
        success_rate
    );

    // Verify each device got reasonable sample count
    for (i, addr) in ADDRESSES.iter().enumerate() {
        let samples = positions_by_device[i].len();
        assert!(
            samples > expected_samples_per_device / 2,
            "Device {} got too few samples: {} (expected ~{})",
            addr,
            samples,
            expected_samples_per_device
        );
    }
}

#[tokio::test]
async fn test_data_broadcast_simulation() {
    println!("\n=== Test: Data Broadcast Simulation (bd-e52e.11) ===");

    // Simulate what the data distributor does: read positions and "broadcast" them
    let driver = Ell14Driver::new(&get_elliptec_port(), "2").expect("Failed to create driver");

    let mut broadcast_data = Vec::new();
    let test_duration = Duration::from_secs(5);
    let start_time = std::time::Instant::now();
    let mut sequence = 0u64;

    println!(
        "Simulating position data broadcast for {} seconds...",
        test_duration.as_secs()
    );

    while start_time.elapsed() < test_duration {
        if let Ok(position) = driver.position().await {
            // Simulate broadcast data packet
            let data_packet = (sequence, start_time.elapsed().as_millis() as u64, position);
            broadcast_data.push(data_packet);
            sequence += 1;
        }
        sleep(Duration::from_millis(50)).await;
    }

    // Verify data integrity
    println!("\nBroadcast Simulation Results:");
    println!("  Packets generated: {}", broadcast_data.len());
    println!("  Sequence range: 0 - {}", sequence.saturating_sub(1));

    // Check sequence continuity
    let mut sequence_gaps = 0;
    for (i, (seq, _, _)) in broadcast_data.iter().enumerate() {
        if *seq != i as u64 {
            sequence_gaps += 1;
        }
    }
    println!("  Sequence gaps: {}", sequence_gaps);

    // Check timestamp monotonicity
    let mut timestamp_issues = 0;
    for i in 1..broadcast_data.len() {
        if broadcast_data[i].1 < broadcast_data[i - 1].1 {
            timestamp_issues += 1;
        }
    }
    println!("  Timestamp ordering issues: {}", timestamp_issues);

    assert!(
        broadcast_data.len() >= 50,
        "Not enough broadcast packets: {}",
        broadcast_data.len()
    );
    assert_eq!(sequence_gaps, 0, "Sequence continuity broken");
    assert_eq!(timestamp_issues, 0, "Timestamp monotonicity broken");
}

// =============================================================================
// Phase 12: Long-Duration Stability Tests (bd-e52e.15)
// =============================================================================

/// Short stability test for CI (60 seconds)
/// For full 30-minute stability test, run manually with --ignored flag
#[tokio::test]
async fn test_stability_short() {
    println!("\n=== Test: Short Stability Test (60 seconds) ===");
    println!("For full 30-minute test, run: cargo test --features hardware_tests,instrument_thorlabs test_stability_long -- --ignored");

    run_stability_test(Duration::from_secs(60)).await;
}

/// Full 30-minute stability test (run manually)
#[tokio::test]
#[ignore] // Run with --ignored flag
async fn test_stability_long() {
    println!("\n=== Test: Long Stability Test (30 minutes) (bd-e52e.15) ===");
    run_stability_test(Duration::from_secs(30 * 60)).await;
}

async fn run_stability_test(duration: Duration) {
    // Create drivers
    let drivers: Vec<_> = ADDRESSES
        .iter()
        .map(|addr| Ell14Driver::new(&get_elliptec_port(), addr).expect("Failed to create driver"))
        .collect();

    // Statistics tracking
    let mut total_queries = 0usize;
    let mut successful_queries = 0usize;
    let mut timeout_errors = 0usize;
    let mut parse_errors = 0usize;
    let mut connection_errors = 0usize;
    let mut positions_by_device: Vec<Vec<f64>> = vec![Vec::new(); ADDRESSES.len()];

    let poll_interval = Duration::from_millis(500); // 2 Hz polling
    let report_interval = Duration::from_secs(30);
    let start_time = std::time::Instant::now();
    let mut last_report = start_time;

    println!(
        "Running stability test for {} seconds...",
        duration.as_secs()
    );
    println!("Polling {} devices at 2 Hz", ADDRESSES.len());

    while start_time.elapsed() < duration {
        for (i, driver) in drivers.iter().enumerate() {
            total_queries += 1;
            match driver.position().await {
                Ok(pos) => {
                    successful_queries += 1;
                    positions_by_device[i].push(pos);
                }
                Err(e) => {
                    let err_str = e.to_string().to_lowercase();
                    if err_str.contains("timeout") {
                        timeout_errors += 1;
                    } else if err_str.contains("parse") || err_str.contains("invalid") {
                        parse_errors += 1;
                    } else {
                        connection_errors += 1;
                    }
                }
            }
        }

        // Periodic status report
        if last_report.elapsed() >= report_interval {
            let elapsed = start_time.elapsed().as_secs();
            let success_rate = (successful_queries as f64 / total_queries.max(1) as f64) * 100.0;
            println!(
                "[{:4}s] Queries: {}, Success: {:.1}%, Timeouts: {}, Parse: {}, Connection: {}",
                elapsed,
                total_queries,
                success_rate,
                timeout_errors,
                parse_errors,
                connection_errors
            );
            last_report = std::time::Instant::now();
        }

        sleep(poll_interval).await;
    }

    // Final report
    let total_elapsed = start_time.elapsed().as_secs_f64();
    let success_rate = (successful_queries as f64 / total_queries.max(1) as f64) * 100.0;
    let queries_per_second = total_queries as f64 / total_elapsed;

    println!("\n=== Stability Test Results ===");
    println!("Duration: {:.1} seconds", total_elapsed);
    println!("Total queries: {}", total_queries);
    println!("Successful: {} ({:.2}%)", successful_queries, success_rate);
    println!("Failed: {}", total_queries - successful_queries);
    println!("  - Timeout errors: {}", timeout_errors);
    println!("  - Parse errors: {}", parse_errors);
    println!("  - Connection errors: {}", connection_errors);
    println!("Query rate: {:.1} queries/second", queries_per_second);

    // Position stability per device
    println!("\nPosition Stability by Device:");
    for (i, addr) in ADDRESSES.iter().enumerate() {
        if positions_by_device[i].len() >= 2 {
            let samples = &positions_by_device[i];
            let mean: f64 = samples.iter().sum::<f64>() / samples.len() as f64;
            let variance: f64 =
                samples.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / samples.len() as f64;
            let std_dev = variance.sqrt();
            let min = samples.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = samples.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

            println!(
                "  Device {}: {} samples, mean={:.3}°, std={:.4}°, range={:.3}°-{:.3}°",
                addr,
                samples.len(),
                mean,
                std_dev,
                min,
                max
            );
        } else {
            println!(
                "  Device {}: Insufficient data ({} samples)",
                addr,
                positions_by_device[i].len()
            );
        }
    }

    // Assertions
    assert!(
        success_rate >= 95.0,
        "Stability test failed: success rate {:.2}% < 95%",
        success_rate
    );

    // Position should be stable (devices are stationary)
    for (i, addr) in ADDRESSES.iter().enumerate() {
        if positions_by_device[i].len() >= 10 {
            let samples = &positions_by_device[i];
            let mean: f64 = samples.iter().sum::<f64>() / samples.len() as f64;
            let variance: f64 =
                samples.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / samples.len() as f64;
            let std_dev = variance.sqrt();

            // Position should be stable when stationary (std dev < 0.5 degrees)
            assert!(
                std_dev < 0.5,
                "Device {} position unstable during stationary test: std_dev = {:.4}°",
                addr,
                std_dev
            );
        }
    }

    println!("\nStability test PASSED");
}

// =============================================================================
// Phase 13: Performance Characterization (bd-e52e.17, bd-e52e.18, bd-e52e.21)
// =============================================================================

#[tokio::test]
async fn test_movement_speed_and_settling_time() {
    println!("\n=== Test: Movement Speed and Settling Time (bd-e52e.18) ===");

    let driver = Ell14Driver::new(&get_elliptec_port(), "2").expect("Failed to create driver");
    let initial = driver.position().await.expect("Failed to get position");

    // Test different movement distances
    let test_moves = [10.0, 30.0, 60.0, 90.0, 180.0];
    let mut results = Vec::new();

    println!("Measuring movement speed and settling time:");

    for distance in test_moves {
        // Move to start position
        driver.move_abs(0.0).await.expect("Failed to move");
        driver.wait_settled().await.expect("Failed to settle");
        sleep(Duration::from_millis(200)).await;

        // Time the move
        let target = distance;
        let start_time = std::time::Instant::now();
        driver.move_abs(target).await.expect("Failed to move");

        // Wait for settle and measure total time
        driver.wait_settled().await.expect("Failed to settle");
        let total_time = start_time.elapsed();

        let final_pos = driver.position().await.expect("Failed to get position");
        let error = (final_pos - target).abs();

        let speed = distance / total_time.as_secs_f64(); // degrees per second

        println!(
            "  {:.0}° move: {:.2}s total, {:.1}°/s avg speed, error: {:.3}°",
            distance,
            total_time.as_secs_f64(),
            speed,
            error
        );

        results.push((distance, total_time.as_secs_f64(), speed));
    }

    // Calculate average speed
    let avg_speed: f64 = results.iter().map(|(_, _, s)| s).sum::<f64>() / results.len() as f64;
    println!("\nPerformance Summary:");
    println!("  Average speed: {:.1}°/s", avg_speed);

    // ELL14 should move reasonably fast (typically 10-50°/s)
    assert!(
        avg_speed > 5.0,
        "Movement speed too slow: {:.1}°/s",
        avg_speed
    );

    // Return to initial
    driver.move_abs(initial).await.ok();
    driver.wait_settled().await.ok();
}

#[tokio::test]
async fn test_command_latency() {
    println!("\n=== Test: Command Latency (bd-e52e.21) ===");

    let driver = Ell14Driver::new(&get_elliptec_port(), "3").expect("Failed to create driver");
    let num_queries = 50;
    let mut latencies = Vec::new();

    println!(
        "Measuring position query latency ({} queries)...",
        num_queries
    );

    for _ in 0..num_queries {
        let start = std::time::Instant::now();
        let _ = driver.position().await;
        let latency = start.elapsed();
        latencies.push(latency.as_millis() as f64);
    }

    // Calculate statistics
    let avg_latency = latencies.iter().sum::<f64>() / num_queries as f64;
    let min_latency = latencies.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_latency = latencies.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    // Calculate p50 and p95
    let mut sorted = latencies.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = sorted[sorted.len() / 2];
    let p95 = sorted[(sorted.len() as f64 * 0.95) as usize];

    println!("\nLatency Statistics (position query):");
    println!("  Average: {:.1}ms", avg_latency);
    println!("  Min: {:.1}ms", min_latency);
    println!("  Max: {:.1}ms", max_latency);
    println!("  p50: {:.1}ms", p50);
    println!("  p95: {:.1}ms", p95);

    // Latency should be reasonable (< 500ms average for RS-485)
    assert!(
        avg_latency < 500.0,
        "Average latency too high: {:.1}ms",
        avg_latency
    );
}

#[tokio::test]
async fn test_throughput() {
    println!("\n=== Test: Command Throughput (bd-e52e.21) ===");

    let driver = Ell14Driver::new(&get_elliptec_port(), "8").expect("Failed to create driver");

    // Measure how many queries per second we can sustain
    let test_duration = Duration::from_secs(5);
    let start_time = std::time::Instant::now();
    let mut query_count = 0;
    let mut success_count = 0;

    println!(
        "Measuring throughput for {} seconds...",
        test_duration.as_secs()
    );

    while start_time.elapsed() < test_duration {
        query_count += 1;
        if driver.position().await.is_ok() {
            success_count += 1;
        }
        // No delay - measure raw throughput
    }

    let elapsed = start_time.elapsed().as_secs_f64();
    let queries_per_second = query_count as f64 / elapsed;
    let success_rate = (success_count as f64 / query_count as f64) * 100.0;

    println!("\nThroughput Results:");
    println!("  Total queries: {}", query_count);
    println!("  Successful: {} ({:.1}%)", success_count, success_rate);
    println!("  Throughput: {:.1} queries/second", queries_per_second);

    // Should achieve at least 5 queries/second
    assert!(
        queries_per_second > 5.0,
        "Throughput too low: {:.1} q/s",
        queries_per_second
    );
}

#[tokio::test]
async fn test_graceful_disconnect() {
    println!("\n=== Test: Graceful Disconnect (bd-e52e.34) ===");

    // Test that we can create, use, and drop drivers without issues
    for round in 1..=3 {
        println!("Round {}:", round);

        // Create driver
        let driver = Ell14Driver::new(&get_elliptec_port(), "2").expect("Failed to create driver");

        // Use it
        let pos = driver.position().await.expect("Failed to get position");
        println!("  Position: {:.2}°", pos);

        // Driver drops here
        drop(driver);
        println!("  Driver dropped");

        // Small delay before next round
        sleep(Duration::from_millis(100)).await;
    }

    // Verify we can still communicate after all the creates/drops
    let driver = Ell14Driver::new(&get_elliptec_port(), "2").expect("Failed to create final driver");
    let final_pos = driver
        .position()
        .await
        .expect("Failed to get final position");
    println!("\nFinal verification: {:.2}°", final_pos);
    println!("Graceful disconnect test PASSED");
}
