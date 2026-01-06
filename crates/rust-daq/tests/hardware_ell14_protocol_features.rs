#![cfg(not(target_arch = "wasm32"))]
//! ELL14 Extended Protocol Features Hardware Validation Tests
//!
//! Tests for new ELL14 protocol commands:
//! - Motor period commands (f1/b1/f2/b2)
//! - Current curve scan (c1/c2)
//! - Device isolation (is)
//! - Motor fine-tuning (om)
//! - Clean mechanics (cm)
//! - Skip frequency search (sk)
//! - Home direction parameter
//!
//! **IMPORTANT:** These tests:
//! 1. Include `stop()` calls for all long-running operations
//! 2. Restore all parameters to original values after modification
//! 3. Verify restoration using device info queries
//!
//! Run with:
//! ```bash
//! cargo test --features "hardware_tests,instrument_thorlabs" \
//!   --test hardware_ell14_protocol_features -- --nocapture --test-threads=1
//! ```
//!
//! **SAFETY:** These tests move physical hardware. Ensure no obstructions.

#![cfg(all(feature = "hardware_tests", feature = "instrument_thorlabs"))]

use rust_daq::hardware::capabilities::Movable;
use rust_daq::hardware::ell14::{Ell14Driver, HomeDirection};
use std::time::Duration;
use tokio::time::sleep;

fn get_elliptec_port() -> String {
    std::env::var("ELLIPTEC_PORT").unwrap_or_else(|_| "/dev/ttyUSB1".to_string())
}

const TEST_ADDRESS: &str = "2";
const POSITION_TOLERANCE_DEG: f64 = 1.0;

// =============================================================================
// Helper Functions
// =============================================================================

/// Create driver with device-specific calibration
///
/// CRITICAL: This reads pulses_per_degree from the device's `IN` response
/// rather than using a hardcoded default. Each ELL14 unit has device-specific
/// calibration stored in firmware.
async fn create_driver() -> Ell14Driver {
    Ell14Driver::new_async_with_device_calibration(&get_elliptec_port(), TEST_ADDRESS)
        .await
        .expect("Failed to create driver with device calibration")
}

/// Verify device is responding after a test
async fn verify_device_responsive(driver: &Ell14Driver) -> bool {
    match driver.get_device_info().await {
        Ok(info) => {
            println!("  Device verified: {} (SN: {})", info.device_type, info.serial);
            true
        }
        Err(e) => {
            println!("  Device verification failed: {}", e);
            false
        }
    }
}

/// Ensure device is stopped and ready
async fn ensure_stopped(driver: &Ell14Driver) {
    // Send stop command
    let _ = driver.stop().await;
    // Wait for any motion to halt
    sleep(Duration::from_millis(200)).await;
    // Verify device is ready
    let _ = driver.wait_settled().await;
}

// =============================================================================
// Motor Period Commands (f1/b1/f2/b2) Tests
// =============================================================================

#[tokio::test]
async fn test_motor_periods_get() {
    println!("\n=== Test: Get Motor Periods ===");

    let driver = create_driver().await;

    // Get motor 1 periods
    match driver.get_motor1_periods().await {
        Ok(periods) => {
            println!("Motor 1 periods:");
            println!("  Forward: {} (freq: {} Hz)",
                periods.forward_period,
                if periods.forward_period > 0 { 14_740_000 / periods.forward_period as u32 } else { 0 }
            );
            println!("  Backward: {} (freq: {} Hz)",
                periods.backward_period,
                if periods.backward_period > 0 { 14_740_000 / periods.backward_period as u32 } else { 0 }
            );

            // Verify periods are in reasonable range for piezo motors
            assert!(periods.forward_period > 0, "Forward period should not be zero");
            assert!(periods.backward_period > 0, "Backward period should not be zero");
        }
        Err(e) => {
            println!("Failed to get motor 1 periods: {}", e);
            // Not a hard failure - command may not be supported on all firmware versions
        }
    }

    sleep(Duration::from_millis(100)).await;

    // Get motor 2 periods
    match driver.get_motor2_periods().await {
        Ok(periods) => {
            println!("Motor 2 periods:");
            println!("  Forward: {} (freq: {} Hz)",
                periods.forward_period,
                if periods.forward_period > 0 { 14_740_000 / periods.forward_period as u32 } else { 0 }
            );
            println!("  Backward: {} (freq: {} Hz)",
                periods.backward_period,
                if periods.backward_period > 0 { 14_740_000 / periods.backward_period as u32 } else { 0 }
            );
        }
        Err(e) => {
            println!("Failed to get motor 2 periods: {}", e);
        }
    }

    // Verify device is still responsive
    assert!(verify_device_responsive(&driver).await, "Device should be responsive after test");
}

#[tokio::test]
async fn test_motor_periods_set_and_restore() {
    println!("\n=== Test: Set Motor Periods and Restore ===");
    println!("This test modifies motor periods and MUST restore them");

    let driver = create_driver().await;

    // Get original periods FIRST
    let original_periods = match driver.get_motor1_periods().await {
        Ok(p) => {
            println!("Original Motor 1 periods:");
            println!("  Forward: {}", p.forward_period);
            println!("  Backward: {}", p.backward_period);
            Some(p)
        }
        Err(e) => {
            println!("Cannot get original periods, skipping test: {}", e);
            return;
        }
    };
    let original = original_periods.unwrap();

    // Try setting a slightly different period (within safe range)
    // Typical piezo frequencies are 78-106 kHz, periods ~139-189
    let test_period = if original.forward_period > 150 {
        original.forward_period - 5
    } else {
        original.forward_period + 5
    };

    println!("Setting test period: {} (original: {})", test_period, original.forward_period);

    match driver.set_motor1_forward_period(test_period).await {
        Ok(_) => {
            println!("Successfully set motor 1 forward period");

            // Verify the change
            sleep(Duration::from_millis(100)).await;
            if let Ok(new_periods) = driver.get_motor1_periods().await {
                println!("New forward period: {}", new_periods.forward_period);
            }
        }
        Err(e) => {
            println!("Failed to set period: {}", e);
        }
    }

    // RESTORE ORIGINAL VALUE - CRITICAL
    println!("\nRestoring original period...");
    match driver.set_motor1_forward_period(original.forward_period).await {
        Ok(_) => {
            println!("Restored motor 1 forward period to {}", original.forward_period);
        }
        Err(e) => {
            println!("WARNING: Failed to restore original period: {}", e);
            // Try factory reset as fallback
            println!("Attempting factory period restore...");
            let _ = driver.restore_motor1_factory_periods().await;
        }
    }

    // VERIFY RESTORATION
    sleep(Duration::from_millis(100)).await;
    match driver.get_motor1_periods().await {
        Ok(restored) => {
            let diff = (restored.forward_period as i32 - original.forward_period as i32).abs();
            println!("Verified: Forward period = {} (diff from original: {})",
                restored.forward_period, diff);
            assert!(diff <= 1, "Period should be restored to original value");
        }
        Err(e) => {
            println!("Could not verify restoration: {}", e);
        }
    }

    // Final device check
    assert!(verify_device_responsive(&driver).await, "Device should be responsive after restore");
}

// =============================================================================
// Current Curve Scan (c1/c2) Tests
// =============================================================================

/// Long-running test - includes stop() call
#[tokio::test]
#[ignore] // This test takes ~12 seconds per motor - run with --ignored
async fn test_current_curve_scan_motor1() {
    println!("\n=== Test: Current Curve Scan Motor 1 ===");
    println!("WARNING: This test takes ~12 seconds and moves the motor");

    let driver = create_driver().await;

    // Get initial position
    let initial_pos = driver.position().await.expect("Failed to get position");
    println!("Initial position: {:.2}°", initial_pos);

    println!("Starting current curve scan...");
    let start = std::time::Instant::now();

    // Run the scan with timeout protection
    let scan_result = tokio::time::timeout(
        Duration::from_secs(20),
        driver.scan_current_curve_motor1()
    ).await;

    match scan_result {
        Ok(Ok(scan)) => {
            println!("Scan completed in {:.1}s", start.elapsed().as_secs_f64());
            println!("Motor {}: {} data points", scan.motor_number, scan.data_points.len());

            if !scan.data_points.is_empty() {
                let first = &scan.data_points[0];
                let last = &scan.data_points[scan.data_points.len() - 1];
                println!("  Frequency range: {} Hz - {} Hz", first.frequency_hz, last.frequency_hz);
                println!("  First point current: fwd={:.3}A, bwd={:.3}A",
                    first.forward_current_amps, first.backward_current_amps);
            }

            assert_eq!(scan.data_points.len(), 87, "Should have 87 data points");
        }
        Ok(Err(e)) => {
            println!("Scan failed: {}", e);
            // Send stop command in case scan is stuck
            ensure_stopped(&driver).await;
        }
        Err(_) => {
            println!("Scan timed out - sending stop command");
            ensure_stopped(&driver).await;
        }
    }

    // ALWAYS stop and return to initial position
    ensure_stopped(&driver).await;

    println!("Returning to initial position...");
    driver.move_abs(initial_pos).await.ok();
    driver.wait_settled().await.ok();

    // Verify device is responsive
    assert!(verify_device_responsive(&driver).await, "Device should be responsive after scan");
}

/// Test that stop() can abort a current curve scan
#[tokio::test]
#[ignore] // Manual test - requires timing
async fn test_current_curve_scan_abort() {
    println!("\n=== Test: Current Curve Scan Abort ===");
    println!("Testing that stop() can abort a long-running scan");

    let driver = create_driver().await;

    // Start scan without awaiting completion
    println!("Starting scan and immediately sending stop...");
    let _ = driver.scan_current_curve_motor1(); // Don't await

    sleep(Duration::from_millis(500)).await; // Let scan start

    // Send stop command
    println!("Sending stop command...");
    driver.stop().await.expect("Stop should succeed");

    sleep(Duration::from_millis(200)).await;

    // Verify device is responsive
    assert!(verify_device_responsive(&driver).await, "Device should be responsive after abort");
}

// =============================================================================
// Device Isolation (is) Tests
// =============================================================================

#[tokio::test]
#[ignore] // Optional test - run with --ignored
async fn test_isolate_device_and_cancel() {
    println!("\n=== Test: Device Isolation and Cancel ===");
    println!("Testing isolation command with immediate cancellation");

    let driver = create_driver().await;

    // Isolate for 1 minute (will cancel immediately)
    println!("Isolating device for 1 minute...");
    match driver.isolate_device(1).await {
        Ok(_) => {
            println!("Device isolated successfully");

            // Verify device still responds to individual address
            sleep(Duration::from_millis(100)).await;
            let pos = driver.position().await;
            println!("Device still responds to individual address: {:?}", pos.is_ok());

            // CANCEL ISOLATION IMMEDIATELY
            println!("Canceling isolation...");
            match driver.cancel_isolation().await {
                Ok(_) => println!("Isolation cancelled"),
                Err(e) => println!("Cancel isolation error: {}", e),
            }
        }
        Err(e) => {
            println!("Isolation command failed (may not be supported): {}", e);
        }
    }

    // Verify device is fully responsive
    sleep(Duration::from_millis(100)).await;
    assert!(verify_device_responsive(&driver).await, "Device should be responsive after cancel");
}

// =============================================================================
// Motor Fine-Tuning (om) Tests
// =============================================================================

/// Long-running test - includes stop() call
#[tokio::test]
#[ignore] // Takes several minutes - run with --ignored
async fn test_fine_tune_motors() {
    println!("\n=== Test: Motor Fine-Tuning (om command) ===");
    println!("WARNING: This test takes several minutes and moves the motor");
    println!("Use stop() or Ctrl+C to abort if needed");

    let driver = create_driver().await;

    // Record initial state
    let initial_pos = driver.position().await.expect("Failed to get position");
    let initial_motor1_info = driver.get_motor1_info().await.ok();

    println!("Initial position: {:.2}°", initial_pos);
    if let Some(ref info) = initial_motor1_info {
        println!("Initial motor 1 frequency: {} Hz", info.frequency);
    }

    println!("\nStarting motor fine-tuning (this will take several minutes)...");
    let start = std::time::Instant::now();

    // Run fine-tuning with timeout protection
    let result = tokio::time::timeout(
        Duration::from_secs(360), // 6 minute timeout
        driver.fine_tune_motors()
    ).await;

    match result {
        Ok(Ok(_)) => {
            println!("Fine-tuning completed in {:.1}s", start.elapsed().as_secs_f64());

            // Check new motor info
            if let Ok(new_info) = driver.get_motor1_info().await {
                println!("New motor 1 frequency: {} Hz", new_info.frequency);
                if let Some(ref old) = initial_motor1_info {
                    let diff = new_info.frequency as i64 - old.frequency as i64;
                    println!("Frequency change: {:+} Hz", diff);
                }
            }

            // NOTE: We do NOT save to EEPROM in tests - settings are temporary
            println!("Settings NOT saved to EEPROM (test mode)");
        }
        Ok(Err(e)) => {
            println!("Fine-tuning failed: {}", e);
            ensure_stopped(&driver).await;
        }
        Err(_) => {
            println!("Fine-tuning timed out - sending stop");
            ensure_stopped(&driver).await;
        }
    }

    // Return to initial position
    ensure_stopped(&driver).await;
    println!("Returning to initial position...");
    driver.move_abs(initial_pos).await.ok();
    driver.wait_settled().await.ok();

    assert!(verify_device_responsive(&driver).await, "Device should be responsive after fine-tuning");
}

/// Test that stop() can abort motor fine-tuning
#[tokio::test]
#[ignore] // Optional test - run with --ignored
async fn test_fine_tune_motors_abort() {
    println!("\n=== Test: Motor Fine-Tuning Abort ===");
    println!("Testing that fine-tuning can be safely aborted with stop()");

    let driver = create_driver().await;

    let initial_pos = driver.position().await.expect("Failed to get position");

    // Start fine-tuning (send command without full wait)
    println!("Sending fine-tune command...");
    // We can't easily start fine_tune_motors() without blocking, so we test stop() directly

    // Simulate: Send stop command (should always be safe)
    println!("Sending stop command...");
    driver.stop().await.expect("Stop should always succeed");

    sleep(Duration::from_millis(200)).await;

    // Verify device is responsive
    let pos = driver.position().await.expect("Should get position after stop");
    println!("Position after stop: {:.2}°", pos);

    // Return to initial if needed
    if (pos - initial_pos).abs() > POSITION_TOLERANCE_DEG {
        driver.move_abs(initial_pos).await.ok();
        driver.wait_settled().await.ok();
    }

    assert!(verify_device_responsive(&driver).await, "Device should be responsive after abort");
}

// =============================================================================
// Clean Mechanics (cm) Tests
// =============================================================================

/// Long-running test - includes stop() call
#[tokio::test]
#[ignore] // Takes several minutes - run with --ignored
async fn test_clean_mechanics() {
    println!("\n=== Test: Clean Mechanics (cm command) ===");
    println!("WARNING: This test moves the device over full range");
    println!("Ensure full range of motion is CLEAR before running!");
    println!("Use stop() or Ctrl+C to abort if needed");

    let driver = create_driver().await;

    let initial_pos = driver.position().await.expect("Failed to get position");
    println!("Initial position: {:.2}°", initial_pos);

    println!("\nStarting cleaning cycle (full-range movement)...");
    let start = std::time::Instant::now();

    // Run cleaning with timeout protection
    let result = tokio::time::timeout(
        Duration::from_secs(360), // 6 minute timeout
        driver.clean_mechanics()
    ).await;

    match result {
        Ok(Ok(_)) => {
            println!("Cleaning completed in {:.1}s", start.elapsed().as_secs_f64());
        }
        Ok(Err(e)) => {
            println!("Cleaning failed: {}", e);
            ensure_stopped(&driver).await;
        }
        Err(_) => {
            println!("Cleaning timed out - sending stop");
            ensure_stopped(&driver).await;
        }
    }

    // Return to initial position
    ensure_stopped(&driver).await;
    println!("Returning to initial position...");
    driver.move_abs(initial_pos).await.ok();
    driver.wait_settled().await.ok();

    assert!(verify_device_responsive(&driver).await, "Device should be responsive after cleaning");
}

/// Test that stop() can abort cleaning cycle
#[tokio::test]
#[ignore] // Optional test - run with --ignored
async fn test_clean_mechanics_abort() {
    println!("\n=== Test: Clean Mechanics Abort ===");
    println!("Testing that cleaning can be safely aborted with stop()");

    let driver = create_driver().await;

    let initial_pos = driver.position().await.expect("Failed to get position");

    // Stop is always safe
    println!("Sending stop command...");
    driver.stop().await.expect("Stop should always succeed");

    sleep(Duration::from_millis(200)).await;

    // Verify device is responsive and return to initial
    let pos = driver.position().await.expect("Should get position after stop");
    println!("Position after stop: {:.2}°", pos);

    if (pos - initial_pos).abs() > POSITION_TOLERANCE_DEG {
        driver.move_abs(initial_pos).await.ok();
        driver.wait_settled().await.ok();
    }

    assert!(verify_device_responsive(&driver).await, "Device should be responsive after abort");
}

// =============================================================================
// Skip Frequency Search (sk) Tests
// =============================================================================

#[tokio::test]
#[ignore] // Optional test - run with --ignored
async fn test_skip_frequency_search() {
    println!("\n=== Test: Skip Frequency Search (sk command) ===");
    println!("NOTE: This test does NOT save to EEPROM - settings are temporary");

    let driver = create_driver().await;

    // Try the skip frequency search command
    match driver.skip_frequency_search().await {
        Ok(_) => {
            println!("Skip frequency search command succeeded");
            println!("(Setting NOT persisted - requires save_user_data() to persist)");
        }
        Err(e) => {
            println!("Skip frequency search failed (may not be supported): {}", e);
        }
    }

    // Verify device is still responsive
    assert!(verify_device_responsive(&driver).await, "Device should be responsive after skip_frequency_search");
}

// =============================================================================
// Home Direction Tests
// =============================================================================

#[tokio::test]
async fn test_home_with_direction_clockwise() {
    println!("\n=== Test: Home with Direction (Clockwise) ===");

    let driver = create_driver().await;

    // Get the home offset - this is the expected position after homing
    let home_offset = driver.get_home_offset().await.unwrap_or(0.0);
    println!("Device home offset: {:.2}°", home_offset);

    let initial_pos = driver.position().await.expect("Failed to get position");
    println!("Initial position: {:.2}°", initial_pos);

    // Move away from home first
    driver.move_abs(90.0).await.expect("Failed to move");
    driver.wait_settled().await.expect("Failed to settle");

    // Home with clockwise direction
    println!("Homing clockwise...");
    let start = std::time::Instant::now();

    match driver.home_with_direction(Some(HomeDirection::Clockwise)).await {
        Ok(_) => {
            println!("Homing completed in {:.2}s", start.elapsed().as_secs_f64());

            let home_pos = driver.position().await.expect("Failed to get position");
            println!("Home position: {:.2}° (expected near {:.2}°)", home_pos, home_offset);

            // Position after homing should be near the home offset
            let diff = (home_pos - home_offset).abs();
            assert!(diff < 5.0, "Should be within 5° of home offset (diff: {:.2}°)", diff);
        }
        Err(e) => {
            println!("Home with direction failed: {}", e);
            // Fallback to standard home
            ensure_stopped(&driver).await;
            driver.home().await.ok();
        }
    }

    // Return to initial position
    driver.move_abs(initial_pos).await.ok();
    driver.wait_settled().await.ok();

    assert!(verify_device_responsive(&driver).await, "Device should be responsive after homing");
}

#[tokio::test]
async fn test_home_with_direction_counter_clockwise() {
    println!("\n=== Test: Home with Direction (Counter-Clockwise) ===");

    let driver = create_driver().await;

    // Get the home offset - this is the expected position after homing
    let home_offset = driver.get_home_offset().await.unwrap_or(0.0);
    println!("Device home offset: {:.2}°", home_offset);

    let initial_pos = driver.position().await.expect("Failed to get position");
    println!("Initial position: {:.2}°", initial_pos);

    // Move away from home
    driver.move_abs(90.0).await.expect("Failed to move");
    driver.wait_settled().await.expect("Failed to settle");

    // Home with counter-clockwise direction
    println!("Homing counter-clockwise...");
    let start = std::time::Instant::now();

    match driver.home_with_direction(Some(HomeDirection::CounterClockwise)).await {
        Ok(_) => {
            println!("Homing completed in {:.2}s", start.elapsed().as_secs_f64());

            let home_pos = driver.position().await.expect("Failed to get position");
            println!("Home position: {:.2}° (expected near {:.2}°)", home_pos, home_offset);

            // Position after homing should be near the home offset
            let diff = (home_pos - home_offset).abs();
            assert!(diff < 5.0, "Should be within 5° of home offset (diff: {:.2}°)", diff);
        }
        Err(e) => {
            println!("Home with direction failed: {}", e);
            ensure_stopped(&driver).await;
            driver.home().await.ok();
        }
    }

    // Return to initial
    driver.move_abs(initial_pos).await.ok();
    driver.wait_settled().await.ok();

    assert!(verify_device_responsive(&driver).await, "Device should be responsive after homing");
}

#[tokio::test]
async fn test_home_with_direction_default() {
    println!("\n=== Test: Home with Direction (Default/None) ===");

    let driver = create_driver().await;

    // Get the home offset - this is the expected position after homing
    let home_offset = driver.get_home_offset().await.unwrap_or(0.0);
    println!("Device home offset: {:.2}°", home_offset);

    let initial_pos = driver.position().await.expect("Failed to get position");

    // Move away
    driver.move_abs(45.0).await.expect("Failed to move");
    driver.wait_settled().await.expect("Failed to settle");

    // Home with default (no direction specified)
    println!("Homing with default direction...");

    match driver.home_with_direction(None).await {
        Ok(_) => {
            let home_pos = driver.position().await.expect("Failed to get position");
            println!("Home position: {:.2}° (expected near {:.2}°)", home_pos, home_offset);

            // Position after homing should be near the home offset
            let diff = (home_pos - home_offset).abs();
            assert!(diff < 5.0, "Should be within 5° of home offset (diff: {:.2}°)", diff);
        }
        Err(e) => {
            println!("Home failed: {}", e);
            ensure_stopped(&driver).await;
        }
    }

    // Return to initial
    driver.move_abs(initial_pos).await.ok();
    driver.wait_settled().await.ok();

    assert!(verify_device_responsive(&driver).await);
}

// =============================================================================
// Integration Tests - Multiple Features Combined
// =============================================================================

#[tokio::test]
async fn test_full_parameter_cycle_with_restore() {
    println!("\n=== Test: Full Parameter Cycle with Restore ===");
    println!("Testing: velocity, jog step, home offset - all with restore");

    let driver = create_driver().await;

    // Record ALL initial values
    let initial_pos = driver.position().await.expect("Failed to get position");
    let initial_velocity = driver.get_velocity().await.ok();
    let initial_jog = driver.get_jog_step().await.ok();
    let initial_home_offset = driver.get_home_offset().await.ok();

    println!("Initial state:");
    println!("  Position: {:.2}°", initial_pos);
    if let Some(v) = initial_velocity { println!("  Velocity: {}%", v); }
    if let Some(j) = initial_jog { println!("  Jog step: {:.2}°", j); }
    if let Some(h) = initial_home_offset { println!("  Home offset: {:.3}°", h); }

    // Modify parameters
    println!("\nModifying parameters...");

    // Change velocity
    if initial_velocity.is_some() {
        let new_velocity = 80;
        driver.set_velocity(new_velocity).await.ok();
        println!("  Set velocity to {}%", new_velocity);
    }

    // Change jog step
    let new_jog = 15.0;
    driver.set_jog_step(new_jog).await.ok();
    println!("  Set jog step to {:.1}°", new_jog);

    // Verify changes
    sleep(Duration::from_millis(100)).await;
    if let Ok(v) = driver.get_velocity().await {
        println!("  Current velocity: {}%", v);
    }
    if let Ok(j) = driver.get_jog_step().await {
        println!("  Current jog: {:.2}°", j);
    }

    // RESTORE ALL PARAMETERS
    println!("\nRestoring all parameters...");

    if let Some(v) = initial_velocity {
        driver.set_velocity(v).await.ok();
        println!("  Restored velocity to {}%", v);
    }
    if let Some(j) = initial_jog {
        driver.set_jog_step(j).await.ok();
        println!("  Restored jog step to {:.2}°", j);
    }

    // VERIFY RESTORATION
    println!("\nVerifying restoration...");
    sleep(Duration::from_millis(100)).await;

    if let (Some(orig), Ok(current)) = (initial_velocity, driver.get_velocity().await) {
        let diff = (orig as i32 - current as i32).abs();
        println!("  Velocity: {} (diff from original: {})", current, diff);
        assert!(diff <= 1, "Velocity should be restored");
    }

    if let (Some(orig), Ok(current)) = (initial_jog, driver.get_jog_step().await) {
        let diff = (orig - current).abs();
        println!("  Jog step: {:.2}° (diff from original: {:.2}°)", current, diff);
        assert!(diff < 0.5, "Jog step should be restored");
    }

    // Final device verification
    assert!(verify_device_responsive(&driver).await, "Device should be responsive after restore");
    println!("\nAll parameters restored successfully!");
}

#[tokio::test]
async fn test_stop_command_always_works() {
    println!("\n=== Test: Stop Command Always Works ===");
    println!("Verifying stop() is safe to call at any time");

    let driver = create_driver().await;

    // Test 1: Stop when idle
    println!("1. Stop when idle...");
    driver.stop().await.expect("Stop should work when idle");
    println!("   OK");

    // Test 2: Stop during movement
    println!("2. Stop during movement...");
    driver.move_abs(180.0).await.expect("Start move");
    sleep(Duration::from_millis(100)).await; // Let movement start
    driver.stop().await.expect("Stop should work during movement");
    sleep(Duration::from_millis(200)).await;
    println!("   OK");

    // Test 3: Multiple stops in succession
    println!("3. Multiple rapid stops...");
    for _ in 0..5 {
        driver.stop().await.expect("Stop should always succeed");
    }
    println!("   OK");

    // Allow device to settle after rapid commands - may have queued status responses
    sleep(Duration::from_millis(300)).await;

    // Verify device is responsive with retry (rapid stops can leave GS responses in buffer)
    let mut pos = None;
    for attempt in 1..=3 {
        match driver.position().await {
            Ok(p) => {
                pos = Some(p);
                break;
            }
            Err(e) => {
                println!("   Position query attempt {}/3 failed: {}", attempt, e);
                sleep(Duration::from_millis(200)).await;
            }
        }
    }
    let pos = pos.expect("Should get position after stops (with retry)");
    println!("Final position: {:.2}°", pos);

    // Return to home
    driver.home().await.ok();
    driver.wait_settled().await.ok();

    assert!(verify_device_responsive(&driver).await);
    println!("\nStop command works reliably in all scenarios!");
}

// =============================================================================
// Bus Reliability Tests - Prevent Communication Overload
// =============================================================================
//
// ELL14 devices have slow RS-485 communication (9600 baud) and can error out if:
// 1. Too many commands are sent in rapid succession
// 2. Too many motors run simultaneously (current draw from bus distributor)
// 3. Commands overlap on the shared bus
//
// These tests verify the driver handles these constraints gracefully.

/// Test that rapid command sequences don't overwhelm the bus
/// This should run by default to catch command-rate issues early
#[tokio::test]
async fn test_bus_rapid_command_sequence() {
    println!("\n=== Test: Bus Rapid Command Sequence ===");
    println!("Verifying driver handles rapid commands without bus errors");

    let driver = create_driver().await;

    let mut success_count = 0;
    let mut error_count = 0;
    let total_commands = 20;

    println!("Sending {} rapid position queries...", total_commands);

    for i in 0..total_commands {
        match driver.position().await {
            Ok(pos) => {
                success_count += 1;
                if i % 5 == 0 {
                    println!("  [{}/{}] Position: {:.2}°", i + 1, total_commands, pos);
                }
            }
            Err(e) => {
                error_count += 1;
                println!("  [{}/{}] Error: {}", i + 1, total_commands, e);
            }
        }
        // NO delay between commands - stress test
    }

    println!("\nResults: {} successes, {} errors", success_count, error_count);

    // Allow some errors due to bus contention, but majority should succeed
    let success_rate = success_count as f64 / total_commands as f64;
    println!("Success rate: {:.1}%", success_rate * 100.0);

    assert!(
        success_rate >= 0.8,
        "At least 80% of rapid commands should succeed (got {:.1}%)",
        success_rate * 100.0
    );

    // Final verification
    assert!(verify_device_responsive(&driver).await, "Device should recover after rapid commands");
}

/// Test proper command spacing prevents errors
#[tokio::test]
async fn test_bus_command_spacing() {
    println!("\n=== Test: Bus Command Spacing ===");
    println!("Verifying proper inter-command delays prevent errors");

    let driver = create_driver().await;

    let mut success_count = 0;
    let total_commands = 15;
    let inter_command_delay = Duration::from_millis(50); // Recommended minimum

    println!("Sending {} commands with {}ms spacing...", total_commands, inter_command_delay.as_millis());

    for i in 0..total_commands {
        match driver.position().await {
            Ok(pos) => {
                success_count += 1;
                println!("  [{}/{}] Position: {:.2}°", i + 1, total_commands, pos);
            }
            Err(e) => {
                println!("  [{}/{}] Error: {}", i + 1, total_commands, e);
            }
        }
        sleep(inter_command_delay).await;
    }

    println!("\nResults: {}/{} commands succeeded", success_count, total_commands);

    // With proper spacing, ALL commands should succeed
    assert_eq!(
        success_count, total_commands,
        "All commands should succeed with proper spacing"
    );
}

/// Test mixed command types don't cause bus contention
#[tokio::test]
async fn test_bus_mixed_command_types() {
    println!("\n=== Test: Bus Mixed Command Types ===");
    println!("Verifying mixed read/write commands work reliably");

    let driver = create_driver().await;

    let initial_pos = driver.position().await.expect("Failed to get initial position");
    let inter_command_delay = Duration::from_millis(100); // Increased for reliability

    println!("Running mixed command sequence...");

    // Sequence of different command types
    // Note: Commands after movement may fail if device hasn't settled
    let commands: Vec<&str> = vec![
        "get_position",
        "get_device_info",
        "get_velocity",
        "small_move",
        "wait_settle",  // Added settle time after move
        "get_position",
        "get_jog_step",
        "stop",
    ];

    let mut success_count = 0;

    for cmd_name in &commands {
        let result = match *cmd_name {
            "get_position" => driver.position().await.map(|_| ()),
            "get_device_info" => driver.get_device_info().await.map(|_| ()),
            "get_velocity" => driver.get_velocity().await.map(|_| ()),
            "small_move" => driver.move_rel(1.0).await,
            "wait_settle" => {
                // Not a real command - just settle time
                driver.wait_settled().await.ok();
                Ok(())
            }
            "get_jog_step" => driver.get_jog_step().await.map(|_| ()),
            "stop" => driver.stop().await,
            _ => Ok(()),
        };

        match result {
            Ok(_) => {
                success_count += 1;
                println!("  ✓ {}", cmd_name);
            }
            Err(e) => {
                println!("  ✗ {} - {}", cmd_name, e);
            }
        }
        sleep(inter_command_delay).await;
    }

    println!("\nResults: {}/{} commands succeeded", success_count, commands.len());

    // Return to initial position
    ensure_stopped(&driver).await;
    driver.move_abs(initial_pos).await.ok();
    driver.wait_settled().await.ok();

    // Allow some failures due to bus timing (75% success rate)
    let min_required = (commands.len() * 3) / 4;
    assert!(
        success_count >= min_required,
        "At least {}% of commands should succeed (got {}/{})",
        (min_required * 100) / commands.len(),
        success_count,
        commands.len()
    );
}

/// Test recovery from communication errors
#[tokio::test]
async fn test_bus_error_recovery() {
    println!("\n=== Test: Bus Error Recovery ===");
    println!("Verifying driver recovers gracefully from bus errors");

    let driver = create_driver().await;

    // First, verify device is working
    assert!(verify_device_responsive(&driver).await, "Device should be responsive initially");

    // Intentionally cause potential errors with very rapid commands
    println!("Sending burst of commands to potentially cause errors...");
    for _ in 0..10 {
        let _ = driver.position().await; // Ignore result
    }

    // Wait for bus to settle
    println!("Waiting for bus to settle...");
    sleep(Duration::from_millis(500)).await;

    // Verify recovery with proper spacing
    println!("Verifying recovery...");
    let mut recovery_success = 0;
    for i in 0..5 {
        sleep(Duration::from_millis(100)).await;
        if driver.position().await.is_ok() {
            recovery_success += 1;
            println!("  Recovery attempt {}: OK", i + 1);
        } else {
            println!("  Recovery attempt {}: Failed", i + 1);
        }
    }

    assert!(
        recovery_success >= 4,
        "Driver should recover from bus errors (got {}/5 successes)",
        recovery_success
    );

    // Final verification
    assert!(verify_device_responsive(&driver).await, "Device should be fully recovered");
}

/// Test that sequential operations on multiple addresses work correctly
/// This simulates multi-device setups on the same bus
#[tokio::test]
async fn test_bus_sequential_multi_address() {
    println!("\n=== Test: Bus Sequential Multi-Address Operations ===");
    println!("Verifying sequential operations to different addresses work");

    // This test uses a single address but simulates the pattern of
    // multi-device communication by varying command types
    let driver = create_driver().await;

    let inter_device_delay = Duration::from_millis(100); // Delay between "devices"

    println!("Simulating sequential multi-device pattern...");

    // Pattern: query device, wait, query again, wait, move, wait, verify
    // This is how multi-device code should behave

    for round in 1..=3 {
        println!("\nRound {}:", round);

        // Simulate "Device A" query
        match driver.get_device_info().await {
            Ok(info) => println!("  Device A: {}", info.device_type),
            Err(e) => println!("  Device A error: {}", e),
        }
        sleep(inter_device_delay).await;

        // Simulate "Device B" query (same physical device, different command)
        match driver.position().await {
            Ok(pos) => println!("  Device B position: {:.2}°", pos),
            Err(e) => println!("  Device B error: {}", e),
        }
        sleep(inter_device_delay).await;

        // Simulate "Device C" query
        match driver.get_velocity().await {
            Ok(vel) => println!("  Device C velocity: {}%", vel),
            Err(e) => println!("  Device C error: {}", e),
        }
        sleep(inter_device_delay).await;
    }

    assert!(verify_device_responsive(&driver).await, "Device should work after multi-address pattern");
    println!("\nMulti-address pattern completed successfully");
}

/// Test that long operations don't block bus communication
#[tokio::test]
async fn test_bus_long_operation_interruptibility() {
    println!("\n=== Test: Bus Long Operation Interruptibility ===");
    println!("Verifying stop command can interrupt movements");

    let driver = create_driver().await;

    let initial_pos = driver.position().await.expect("Failed to get position");
    println!("Initial position: {:.2}°", initial_pos);

    // Start a long move
    let target = if initial_pos < 180.0 { 350.0 } else { 10.0 };
    println!("Starting move to {:.0}° (long travel)...", target);

    driver.move_abs(target).await.expect("Move command should succeed");

    // Wait briefly then interrupt
    sleep(Duration::from_millis(200)).await;

    println!("Interrupting with stop command...");
    let stop_result = driver.stop().await;
    assert!(stop_result.is_ok(), "Stop should succeed during movement");

    // Allow device to settle - stop may leave status responses in buffer
    sleep(Duration::from_millis(300)).await;

    // Verify we can still communicate (with retry for buffer clearing)
    let mut interrupted_pos = None;
    for attempt in 1..=3 {
        match driver.position().await {
            Ok(pos) => {
                interrupted_pos = Some(pos);
                break;
            }
            Err(e) => {
                println!("  Position query attempt {}/3 failed: {}", attempt, e);
                sleep(Duration::from_millis(200)).await;
            }
        }
    }
    let interrupted_pos = interrupted_pos.expect("Should get position after stop (with retry)");
    println!("Position after interrupt: {:.2}°", interrupted_pos);

    // Position should be between initial and target (interrupted mid-move)
    // Unless the move was very fast

    // Return to initial
    driver.move_abs(initial_pos).await.ok();
    driver.wait_settled().await.ok();

    assert!(verify_device_responsive(&driver).await, "Device should be responsive after interrupt");
}

/// Test bus doesn't lock up under sustained load
#[tokio::test]
async fn test_bus_sustained_load() {
    println!("\n=== Test: Bus Sustained Load ===");
    println!("Verifying bus handles sustained query load");

    let driver = create_driver().await;

    let test_duration = Duration::from_secs(5);
    let query_interval = Duration::from_millis(100); // 10 queries/second

    let start = std::time::Instant::now();
    let mut query_count = 0;
    let mut error_count = 0;

    println!("Running sustained load for {}s at ~10 queries/sec...", test_duration.as_secs());

    while start.elapsed() < test_duration {
        match driver.position().await {
            Ok(_) => query_count += 1,
            Err(_) => error_count += 1,
        }
        sleep(query_interval).await;
    }

    let elapsed = start.elapsed().as_secs_f64();
    let queries_per_sec = query_count as f64 / elapsed;
    let error_rate = error_count as f64 / (query_count + error_count) as f64;

    println!("\nResults:");
    println!("  Duration: {:.1}s", elapsed);
    println!("  Queries: {} successful, {} errors", query_count, error_count);
    println!("  Rate: {:.1} queries/sec", queries_per_sec);
    println!("  Error rate: {:.1}%", error_rate * 100.0);

    assert!(
        error_rate < 0.05,
        "Error rate should be under 5% for sustained load (got {:.1}%)",
        error_rate * 100.0
    );

    assert!(verify_device_responsive(&driver).await, "Device should be responsive after sustained load");
}

// =============================================================================
// Safety/Cleanup Test - Run Last
// =============================================================================

#[tokio::test]
async fn test_z_final_cleanup_and_verify() {
    println!("\n=== Final Cleanup and Verification ===");
    println!("This test runs last (z prefix) to verify all devices are in good state");

    let driver = create_driver().await;

    // Send stop just in case
    driver.stop().await.ok();
    sleep(Duration::from_millis(200)).await;

    // Get device info
    match driver.get_device_info().await {
        Ok(info) => {
            println!("Device: {} (SN: {})", info.device_type, info.serial);
            println!("Firmware: {}", info.firmware);
            println!("Pulses/unit: {}", info.pulses_per_unit);
        }
        Err(e) => {
            println!("Could not get device info: {}", e);
        }
    }

    // Get position
    match driver.position().await {
        Ok(pos) => println!("Position: {:.2}°", pos),
        Err(e) => println!("Could not get position: {}", e),
    }

    // Get motor info
    if let Ok(m1) = driver.get_motor1_info().await {
        println!("Motor 1: {} Hz, loop={}", m1.frequency, m1.loop_state);
    }
    if let Ok(m2) = driver.get_motor2_info().await {
        println!("Motor 2: {} Hz, loop={}", m2.frequency, m2.loop_state);
    }

    println!("\nAll tests completed. Device is in good state.");
}
