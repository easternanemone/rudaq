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
//! MaiTai Ti:Sapphire Laser Hardware Validation Tests
//!
//! # CRITICAL SAFETY WARNING
//!
//! THIS TEST FILE CONTROLS A CLASS 4 LASER (MaiTai Ti:Sapphire)
//!
//! ## BEFORE RUNNING ANY TESTS:
//! 1. Obtain explicit authorization from Laser Safety Officer (LSO)
//! 2. Ensure all personnel have completed laser safety training
//! 3. Verify proper PPE is available (OD6+ safety glasses for 690-1040nm)
//! 4. Confirm beam path is enclosed and terminated properly
//! 5. Activate all hardware interlocks
//! 6. Post warning signs and activate warning lights
//! 7. Confirm no reflective surfaces in beam path
//!
//! ## EMERGENCY PROCEDURES:
//! - Know location of emergency stop button
//! - Know location of fire extinguisher (CO2 for electrical fires)
//! - Laser shutoff procedure: SHUTTER FIRST, then EMISSION OFF
//!
//! ## TEST CATEGORIES:
//! - `connection_*` - Can run with laser powered off (safe)
//! - `identity_*` - Communication tests, laser can be off (safe)
//! - `shutter_*` - REQUIRES SAFETY APPROVAL (laser on, shutter control)
//! - `wavelength_*` - REQUIRES SAFETY APPROVAL (may affect alignment)
//! - `power_*` - REQUIRES SAFETY APPROVAL (shutter must open)
//! - `emission_*` - HIGHEST RISK (turning laser on/off)
//!
//! Run with: cargo test --features "hardware_tests,instrument_spectra_physics"
//!           --test hardware_maitai_validation -- --test-threads=1

#![cfg(all(feature = "hardware_tests", feature = "spectra_physics"))]

use rust_daq::hardware::capabilities::{
    EmissionControl, Readable, ShutterControl, WavelengthTunable,
};
use rust_daq::hardware::maitai::MaiTaiDriver;
use std::time::Duration;
use tokio::time::sleep;

// =============================================================================
// Configuration
// =============================================================================

/// Serial port for MaiTai laser (from config/default.toml)
const PORT: &str = "/dev/ttyUSB5";

/// Default wavelength for tests (nm)
const DEFAULT_WAVELENGTH_NM: f64 = 800.0;

/// Wavelength tuning range
const MIN_WAVELENGTH_NM: f64 = 690.0;
const MAX_WAVELENGTH_NM: f64 = 1040.0;

/// Safety delay after opening shutter (ms)
const SHUTTER_SAFETY_DELAY_MS: u64 = 500;

/// Delay for wavelength tuning (ms)
const WAVELENGTH_TUNING_DELAY_MS: u64 = 2000;

// =============================================================================
// Safety Helper Functions
// =============================================================================

/// Log safety warning before any test that activates the laser
fn log_safety_warning(test_name: &str) {
    eprintln!("\n");
    eprintln!("╔══════════════════════════════════════════════════════════════════╗");
    eprintln!("║                    ⚠️  LASER SAFETY WARNING  ⚠️                    ║");
    eprintln!("╠══════════════════════════════════════════════════════════════════╣");
    eprintln!("║  Test: {:<55} ║", test_name);
    eprintln!("║                                                                  ║");
    eprintln!("║  This test will control laser shutter/emission.                  ║");
    eprintln!("║  Ensure all safety protocols are in place.                       ║");
    eprintln!("║                                                                  ║");
    eprintln!("║  Press Ctrl+C within 5 seconds to abort.                         ║");
    eprintln!("╚══════════════════════════════════════════════════════════════════╝");
    eprintln!("\n");
}

/// Ensure shutter is closed (safety critical)
async fn ensure_shutter_closed(driver: &MaiTaiDriver) -> anyhow::Result<()> {
    driver.close_shutter().await?;
    sleep(Duration::from_millis(100)).await;

    // Verify shutter is actually closed
    let is_open = driver.is_shutter_open().await?;
    if is_open {
        anyhow::bail!("SAFETY CRITICAL: Shutter failed to close!");
    }

    Ok(())
}

// =============================================================================
// Phase 1: Connection Tests (SAFE - no laser activation)
// =============================================================================

/// Test: Basic serial port connection
#[tokio::test]
async fn test_connection_basic() {
    let result = MaiTaiDriver::new(PORT);

    match result {
        Ok(_) => {
            eprintln!("[PASS] Successfully connected to MaiTai on {}", PORT);
        }
        Err(e) => {
            eprintln!("[SKIP] Could not connect to MaiTai: {}", e);
            eprintln!(
                "       Ensure laser controller is powered on and connected to {}",
                PORT
            );
            // Don't fail - hardware may not be present
        }
    }
}

/// Test: Connection with invalid port fails gracefully
#[tokio::test]
async fn test_connection_invalid_port() {
    let result = MaiTaiDriver::new("/dev/ttyNONEXISTENT");

    assert!(
        result.is_err(),
        "Should fail when connecting to non-existent port"
    );
    eprintln!("[PASS] Invalid port correctly rejected");
}

// =============================================================================
// Phase 2: Identity Tests (SAFE - query only)
// =============================================================================

/// Test: Query laser identity
#[tokio::test]
async fn test_identity_query() {
    let driver = match MaiTaiDriver::new(PORT) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    match driver.identify().await {
        Ok(idn) => {
            eprintln!("[PASS] Laser identity: {}", idn);
            assert!(!idn.is_empty(), "Identity should not be empty");
        }
        Err(e) => {
            eprintln!("[WARN] Could not query identity: {}", e);
            eprintln!("       Laser may not respond to *IDN? query");
        }
    }
}

// =============================================================================
// Phase 3: Wavelength Tests (MODERATE RISK - no beam exposure)
// =============================================================================

/// Test: Query current wavelength
#[tokio::test]
async fn test_wavelength_query() {
    let driver = match MaiTaiDriver::new(PORT) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    match driver.get_wavelength().await {
        Ok(wavelength) => {
            eprintln!("[PASS] Current wavelength: {} nm", wavelength);
            assert!(
                (MIN_WAVELENGTH_NM..=MAX_WAVELENGTH_NM).contains(&wavelength),
                "Wavelength {} nm outside expected range",
                wavelength
            );
        }
        Err(e) => {
            eprintln!("[WARN] Could not query wavelength: {}", e);
        }
    }
}

/// Test: Wavelength range validation
#[tokio::test]
async fn test_wavelength_range() {
    let driver = match MaiTaiDriver::new(PORT) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    let (min, max) = driver.wavelength_range();
    eprintln!("[INFO] Driver reports wavelength range: {}-{} nm", min, max);

    assert_eq!(min, MIN_WAVELENGTH_NM, "Min wavelength mismatch");
    assert_eq!(max, MAX_WAVELENGTH_NM, "Max wavelength mismatch");

    eprintln!("[PASS] Wavelength range matches MaiTai specifications");
}

/// Test: Set wavelength to default value
#[tokio::test]
async fn test_wavelength_set_default() {
    log_safety_warning("test_wavelength_set_default");

    let driver = match MaiTaiDriver::new(PORT) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    // Allow abort time for safety
    sleep(Duration::from_secs(5)).await;

    match driver.set_wavelength(DEFAULT_WAVELENGTH_NM).await {
        Ok(()) => {
            eprintln!("[PASS] Set wavelength to {} nm", DEFAULT_WAVELENGTH_NM);

            // Wait for tuning
            sleep(Duration::from_millis(WAVELENGTH_TUNING_DELAY_MS)).await;

            // Verify
            if let Ok(actual) = driver.get_wavelength().await {
                let tolerance = 1.0; // nm
                assert!(
                    (actual - DEFAULT_WAVELENGTH_NM).abs() < tolerance,
                    "Wavelength {} nm differs from target {} nm by more than {} nm",
                    actual,
                    DEFAULT_WAVELENGTH_NM,
                    tolerance
                );
                eprintln!("[PASS] Verified wavelength: {} nm", actual);
            }
        }
        Err(e) => {
            eprintln!("[WARN] Could not set wavelength: {}", e);
        }
    }
}

/// Test: Wavelength out-of-range rejection
#[tokio::test]
async fn test_wavelength_out_of_range() {
    let driver = match MaiTaiDriver::new(PORT) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    // Test below minimum
    let result = driver.set_wavelength(500.0).await;
    assert!(result.is_err(), "Should reject wavelength below minimum");
    eprintln!("[PASS] Correctly rejected 500 nm (below range)");

    // Test above maximum
    let result = driver.set_wavelength(1500.0).await;
    assert!(result.is_err(), "Should reject wavelength above maximum");
    eprintln!("[PASS] Correctly rejected 1500 nm (above range)");
}

/// Test: Wavelength sweep across tuning range
#[tokio::test]
#[ignore] // Run with --ignored flag
async fn test_wavelength_sweep() {
    log_safety_warning("test_wavelength_sweep");

    let driver = match MaiTaiDriver::new(PORT) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    // Allow abort time
    sleep(Duration::from_secs(5)).await;

    let wavelengths = [700.0, 750.0, 800.0, 850.0, 900.0, 950.0, 1000.0];

    eprintln!("[INFO] Beginning wavelength sweep...");

    for target in wavelengths {
        match driver.set_wavelength(target).await {
            Ok(()) => {
                sleep(Duration::from_millis(WAVELENGTH_TUNING_DELAY_MS)).await;

                if let Ok(actual) = driver.get_wavelength().await {
                    eprintln!("  {} nm -> {} nm", target, actual);
                } else {
                    eprintln!("  {} nm -> (read failed)", target);
                }
            }
            Err(e) => {
                eprintln!("  {} nm -> FAILED: {}", target, e);
            }
        }
    }

    // Return to default
    let _ = driver.set_wavelength(DEFAULT_WAVELENGTH_NM).await;
    eprintln!("[PASS] Wavelength sweep complete");
}

// =============================================================================
// Phase 4: Shutter Tests (HIGH RISK - potential beam exposure)
// =============================================================================

/// Test: Query shutter state
#[tokio::test]
async fn test_shutter_query() {
    let driver = match MaiTaiDriver::new(PORT) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    match driver.is_shutter_open().await {
        Ok(is_open) => {
            eprintln!(
                "[PASS] Shutter state: {}",
                if is_open { "OPEN" } else { "CLOSED" }
            );
        }
        Err(e) => {
            eprintln!("[WARN] Could not query shutter state: {}", e);
        }
    }
}

/// Test: Close shutter (always safe to call)
#[tokio::test]
async fn test_shutter_close() {
    let driver = match MaiTaiDriver::new(PORT) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    match ensure_shutter_closed(&driver).await {
        Ok(()) => {
            eprintln!("[PASS] Shutter closed successfully");
        }
        Err(e) => {
            eprintln!("[FAIL] Shutter close failed: {}", e);
            panic!("SAFETY CRITICAL: Could not close shutter!");
        }
    }
}

/// Test: Open and close shutter cycle
#[tokio::test]
#[ignore] // REQUIRES EXPLICIT SAFETY APPROVAL
async fn test_shutter_cycle() {
    log_safety_warning("test_shutter_cycle");

    let driver = match MaiTaiDriver::new(PORT) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    // Safety delay
    sleep(Duration::from_secs(5)).await;

    // Ensure closed first
    ensure_shutter_closed(&driver)
        .await
        .expect("Could not close shutter before test");

    // Open shutter
    eprintln!("[INFO] Opening shutter...");
    driver.open_shutter().await.expect("Failed to open shutter");
    sleep(Duration::from_millis(SHUTTER_SAFETY_DELAY_MS)).await;

    let is_open = driver
        .is_shutter_open()
        .await
        .expect("Could not query state");
    assert!(is_open, "Shutter should be open");
    eprintln!("[INFO] Shutter confirmed OPEN");

    // Close shutter
    eprintln!("[INFO] Closing shutter...");
    driver
        .close_shutter()
        .await
        .expect("Failed to close shutter");
    sleep(Duration::from_millis(100)).await;

    let is_open = driver
        .is_shutter_open()
        .await
        .expect("Could not query state");
    assert!(!is_open, "Shutter should be closed");
    eprintln!("[PASS] Shutter cycle complete - confirmed CLOSED");
}

/// Test: Rapid shutter cycling (stress test)
#[tokio::test]
#[ignore] // REQUIRES EXPLICIT SAFETY APPROVAL
async fn test_shutter_rapid_cycling() {
    log_safety_warning("test_shutter_rapid_cycling");

    let driver = match MaiTaiDriver::new(PORT) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    // Safety delay
    sleep(Duration::from_secs(5)).await;

    const CYCLES: usize = 10;
    eprintln!("[INFO] Beginning {} shutter cycles...", CYCLES);

    for i in 1..=CYCLES {
        driver.open_shutter().await.expect("Failed to open shutter");
        sleep(Duration::from_millis(100)).await;

        driver
            .close_shutter()
            .await
            .expect("Failed to close shutter");
        sleep(Duration::from_millis(100)).await;

        eprintln!("  Cycle {}/{} complete", i, CYCLES);
    }

    // Verify closed at end
    ensure_shutter_closed(&driver)
        .await
        .expect("Failed to close shutter after cycling");

    eprintln!("[PASS] {} shutter cycles completed successfully", CYCLES);
}

// =============================================================================
// Phase 5: Power Measurement Tests (HIGH RISK - shutter must open)
// =============================================================================

/// Test: Read laser power with Readable trait
#[tokio::test]
#[ignore] // REQUIRES EXPLICIT SAFETY APPROVAL
async fn test_power_read() {
    log_safety_warning("test_power_read");

    let driver = match MaiTaiDriver::new(PORT) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    // Safety delay
    sleep(Duration::from_secs(5)).await;

    match driver.read().await {
        Ok(power) => {
            eprintln!("[PASS] Power reading: {} W", power);
            assert!(power >= 0.0, "Power should be non-negative");
        }
        Err(e) => {
            eprintln!("[WARN] Could not read power: {}", e);
        }
    }
}

/// Test: Continuous power monitoring
#[tokio::test]
#[ignore] // REQUIRES EXPLICIT SAFETY APPROVAL
async fn test_power_monitoring() {
    log_safety_warning("test_power_monitoring");

    let driver = match MaiTaiDriver::new(PORT) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    // Safety delay
    sleep(Duration::from_secs(5)).await;

    const READINGS: usize = 10;
    let mut powers = Vec::with_capacity(READINGS);

    eprintln!("[INFO] Taking {} power readings...", READINGS);

    for i in 1..=READINGS {
        match driver.read().await {
            Ok(power) => {
                eprintln!("  Reading {}/{}: {} W", i, READINGS, power);
                powers.push(power);
            }
            Err(e) => {
                eprintln!("  Reading {}/{}: FAILED - {}", i, READINGS, e);
            }
        }
        sleep(Duration::from_millis(500)).await;
    }

    if !powers.is_empty() {
        let avg: f64 = powers.iter().sum::<f64>() / powers.len() as f64;
        let min = powers.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = powers.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

        eprintln!("[INFO] Power statistics:");
        eprintln!("       Min: {} W", min);
        eprintln!("       Max: {} W", max);
        eprintln!("       Avg: {} W", avg);
        eprintln!("[PASS] Power monitoring complete");
    }
}

// =============================================================================
// Phase 6: Emission Tests (HIGHEST RISK - laser on/off control)
// =============================================================================

/// Test: Disable emission (always safe to call)
#[tokio::test]
async fn test_emission_disable() {
    let driver = match MaiTaiDriver::new(PORT) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    match driver.disable_emission().await {
        Ok(()) => {
            eprintln!("[PASS] Emission disable command sent");
        }
        Err(e) => {
            eprintln!("[WARN] Emission disable failed: {}", e);
        }
    }
}

/// Test: Enable/disable emission cycle
#[tokio::test]
#[ignore] // HIGHEST RISK - REQUIRES EXPLICIT LSO APPROVAL
async fn test_emission_cycle() {
    log_safety_warning("test_emission_cycle");
    eprintln!("╔══════════════════════════════════════════════════════════════════╗");
    eprintln!("║              ⚠️  HIGHEST RISK TEST  ⚠️                             ║");
    eprintln!("║  This test will TURN ON the laser emission.                      ║");
    eprintln!("║  LSO approval is REQUIRED before running this test.              ║");
    eprintln!("╚══════════════════════════════════════════════════════════════════╝");

    let driver = match MaiTaiDriver::new(PORT) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    // Extended safety delay for emission tests
    eprintln!("[INFO] Waiting 10 seconds before enabling emission...");
    sleep(Duration::from_secs(10)).await;

    // Ensure shutter is closed before enabling emission
    ensure_shutter_closed(&driver)
        .await
        .expect("Could not close shutter before emission test");

    // Enable emission
    eprintln!("[INFO] Enabling emission...");
    match driver.enable_emission().await {
        Ok(()) => {
            eprintln!("[INFO] Emission enabled");

            // Let it stabilize
            sleep(Duration::from_secs(2)).await;

            // Disable emission
            eprintln!("[INFO] Disabling emission...");
            driver
                .disable_emission()
                .await
                .expect("CRITICAL: Failed to disable emission!");

            eprintln!("[PASS] Emission cycle complete");
        }
        Err(e) => {
            eprintln!("[WARN] Could not enable emission: {}", e);
        }
    }
}

// =============================================================================
// Phase 7: Safety Interlock Tests
// =============================================================================

/// Test: Verify shutter closes on disconnect
#[tokio::test]
#[ignore] // REQUIRES SAFETY APPROVAL
async fn test_safety_shutter_on_drop() {
    log_safety_warning("test_safety_shutter_on_drop");

    // Create driver and open shutter
    {
        let driver = match MaiTaiDriver::new(PORT) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[SKIP] Cannot create driver: {}", e);
                return;
            }
        };

        // Safety delay
        sleep(Duration::from_secs(5)).await;

        // Close shutter first
        ensure_shutter_closed(&driver)
            .await
            .expect("Could not close shutter");

        // Open shutter
        driver.open_shutter().await.expect("Failed to open shutter");
        eprintln!("[INFO] Shutter opened, dropping driver...");

        // Driver dropped here
    }

    // Small delay for hardware to respond
    sleep(Duration::from_millis(500)).await;

    // Create new connection to verify shutter state
    let driver = MaiTaiDriver::new_async(PORT).await.expect("Could not reconnect");

    // Note: MaiTai may not auto-close shutter on disconnect
    // This test documents the behavior
    match driver.is_shutter_open().await {
        Ok(is_open) => {
            if is_open {
                eprintln!("[WARN] Shutter remained open after driver drop");
                eprintln!("       Hardware interlock should be used for safety");
                // Close it now
                ensure_shutter_closed(&driver)
                    .await
                    .expect("Failed to close");
            } else {
                eprintln!("[PASS] Shutter closed after driver drop");
            }
        }
        Err(e) => {
            eprintln!("[WARN] Could not query shutter state: {}", e);
        }
    }
}

// =============================================================================
// Phase 8: Integration Tests with V5 Capability Traits
// =============================================================================

/// Test: Use WavelengthTunable trait methods
#[tokio::test]
async fn test_trait_wavelength_tunable() {
    let driver = match MaiTaiDriver::new_async(PORT).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    // Use trait method
    let wavelength_result = <MaiTaiDriver as WavelengthTunable>::get_wavelength(&driver).await;

    match wavelength_result {
        Ok(wavelength) => {
            eprintln!(
                "[PASS] WavelengthTunable::get_wavelength() returned {} nm",
                wavelength
            );
        }
        Err(e) => {
            eprintln!("[WARN] WavelengthTunable::get_wavelength() failed: {}", e);
        }
    }

    // Check trait method for range
    let (min, max) = <MaiTaiDriver as WavelengthTunable>::wavelength_range(&driver);
    eprintln!(
        "[INFO] WavelengthTunable::wavelength_range() = ({}, {})",
        min, max
    );
}

/// Test: Use ShutterControl trait methods
#[tokio::test]
async fn test_trait_shutter_control() {
    let driver = match MaiTaiDriver::new_async(PORT).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    // Use trait method to close shutter (always safe)
    match <MaiTaiDriver as ShutterControl>::close_shutter(&driver).await {
        Ok(()) => {
            eprintln!("[PASS] ShutterControl::close_shutter() succeeded");
        }
        Err(e) => {
            eprintln!("[WARN] ShutterControl::close_shutter() failed: {}", e);
        }
    }

    // Query state via trait
    match <MaiTaiDriver as ShutterControl>::is_shutter_open(&driver).await {
        Ok(is_open) => {
            eprintln!(
                "[PASS] ShutterControl::is_shutter_open() = {}",
                if is_open { "OPEN" } else { "CLOSED" }
            );
        }
        Err(e) => {
            eprintln!("[WARN] ShutterControl::is_shutter_open() failed: {}", e);
        }
    }
}

/// Test: Use Readable trait for power measurement
#[tokio::test]
#[ignore] // REQUIRES SAFETY APPROVAL
async fn test_trait_readable() {
    log_safety_warning("test_trait_readable");

    let driver = match MaiTaiDriver::new_async(PORT).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    // Safety delay
    sleep(Duration::from_secs(5)).await;

    // Use Readable trait
    match <MaiTaiDriver as Readable>::read(&driver).await {
        Ok(power) => {
            eprintln!("[PASS] Readable::read() returned {} W", power);
        }
        Err(e) => {
            eprintln!("[WARN] Readable::read() failed: {}", e);
        }
    }
}

// =============================================================================
// Phase 9: Performance Tests
// =============================================================================

/// Test: Measure command latency
#[tokio::test]
async fn test_command_latency() {
    let driver = match MaiTaiDriver::new_async(PORT).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    const ITERATIONS: usize = 10;
    let mut latencies = Vec::with_capacity(ITERATIONS);

    eprintln!(
        "[INFO] Measuring wavelength query latency ({} iterations)...",
        ITERATIONS
    );

    for _ in 0..ITERATIONS {
        let start = std::time::Instant::now();
        let _ = driver.get_wavelength().await;
        let elapsed = start.elapsed();
        latencies.push(elapsed);
    }

    let avg_ms = latencies.iter().map(|d| d.as_millis() as f64).sum::<f64>() / ITERATIONS as f64;
    let min_ms = latencies.iter().map(|d| d.as_millis()).min().unwrap_or(0);
    let max_ms = latencies.iter().map(|d| d.as_millis()).max().unwrap_or(0);

    eprintln!("[INFO] Latency statistics:");
    eprintln!("       Min: {} ms", min_ms);
    eprintln!("       Max: {} ms", max_ms);
    eprintln!("       Avg: {:.1} ms", avg_ms);
    eprintln!("[PASS] Command latency measurement complete");
}

// =============================================================================
// Phase 10: Error Recovery Tests
// =============================================================================

/// Test: Recovery from invalid command
#[tokio::test]
async fn test_error_recovery() {
    let driver = match MaiTaiDriver::new_async(PORT).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    // Try invalid wavelength
    let result = driver.set_wavelength(9999.0).await;
    assert!(result.is_err(), "Should reject invalid wavelength");

    // Verify normal operation still works
    match driver.get_wavelength().await {
        Ok(wavelength) => {
            eprintln!(
                "[PASS] Normal operation works after error: {} nm",
                wavelength
            );
        }
        Err(e) => {
            eprintln!("[WARN] Operation failed after error: {}", e);
        }
    }
}

// =============================================================================
// Phase 11: Safety Guard Tests
// =============================================================================

/// Test: Emission guard rejects when shutter is open
///
/// CRITICAL SAFETY TEST: Verifies that the MaiTai driver refuses to enable
/// emission when the shutter is open or in an unknown state. This prevents
/// accidental laser exposure.
#[tokio::test]
async fn test_emission_guard_rejects_when_shutter_open() {
    let driver = match MaiTaiDriver::new_async(PORT).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    // First ensure shutter is closed, then open it
    match driver.close_shutter().await {
        Ok(()) => eprintln!("[INFO] Shutter closed for test setup"),
        Err(e) => {
            eprintln!("[SKIP] Cannot close shutter: {}", e);
            return;
        }
    }

    // Open shutter
    match driver.open_shutter().await {
        Ok(()) => eprintln!("[INFO] Shutter opened for test"),
        Err(e) => {
            eprintln!("[SKIP] Cannot open shutter: {}", e);
            return;
        }
    }

    // Now try to enable emission - this SHOULD FAIL
    let result = driver.enable_emission().await;

    // Close shutter regardless of result (cleanup)
    let _ = driver.close_shutter().await;

    // Verify the guard worked
    match result {
        Err(e) => {
            let err_msg = e.to_string();
            assert!(
                err_msg.contains("shutter") || err_msg.contains("Refusing"),
                "[FAIL] Expected shutter-related error, got: {}",
                err_msg
            );
            eprintln!("[PASS] Emission guard correctly rejected: {}", err_msg);
        }
        Ok(()) => {
            // This is a CRITICAL FAILURE - emission was enabled with shutter open
            eprintln!("[CRITICAL FAIL] Emission was enabled with shutter open!");
            eprintln!("                This is a safety violation.");
            // Immediately disable emission
            let _ = driver.disable_emission().await;
            panic!("SAFETY VIOLATION: Emission enabled with shutter open");
        }
    }
}

/// Test: Emission allowed when shutter is closed
#[tokio::test]
#[ignore] // REQUIRES SAFETY APPROVAL - this will actually enable emission
async fn test_emission_allowed_when_shutter_closed() {
    log_safety_warning("test_emission_allowed_when_shutter_closed");

    let driver = match MaiTaiDriver::new_async(PORT).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SKIP] Cannot create driver: {}", e);
            return;
        }
    };

    // Safety delay
    sleep(Duration::from_secs(5)).await;

    // Ensure shutter is closed
    ensure_shutter_closed(&driver)
        .await
        .expect("Could not close shutter");

    // Try to enable emission - this SHOULD SUCCEED
    match driver.enable_emission().await {
        Ok(()) => {
            eprintln!("[PASS] Emission enabled with shutter closed (as expected)");
            // Immediately disable
            let _ = driver.disable_emission().await;
        }
        Err(e) => {
            eprintln!("[INFO] Emission rejected: {} (may be hardware state)", e);
        }
    }
}
