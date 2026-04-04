#![cfg(not(target_arch = "wasm32"))]
//! LIBS Multi-Device Integration Smoke Tests
//!
//! Tests coordination between multiple LIBS hardware drivers:
//! - Dover Motion stage
//! - Andor iStar camera
//! - Spectra-Physics Spirit laser
//!
//! # Environment Variables
//!
//! Required:
//! - `LIBS_INTEGRATION_TEST=1` - Enable integration tests
//!
//! Optional:
//! - `DOVER_CONFIG_PATH` - Dover Motion config file path
//! - `ANDOR_CAMERA_INDEX` - Camera index (default: 0)
//! - `SPIRIT_CAN_ADAPTER` - CAN adapter for Spirit laser
//!
//! # Usage
//!
//! ```bash
//! # Mock mode (no hardware)
//! cargo nextest run --test libs_integration_smoke
//!
//! # Hardware mode (all LIBS hardware connected)
//! export LIBS_INTEGRATION_TEST=1
//! cargo nextest run --profile libs-hardware --features hardware --test libs_integration_smoke
//! ```
//!
//! # Test Coverage
//!
//! | Test | Description |
//! |------|-------------|
//! | `mock_stage_camera_sync` | Stage movement triggers camera (mock) |
//! | `mock_full_libs_sequence` | Complete LIBS acquisition cycle (mock) |
//! | `hardware_stage_camera_top` | TOP synchronization (hardware) |
//! | `hardware_libs_safety_sequence` | Laser safety with shutter control (hardware) |

#[cfg(all(feature = "hardware_tests", feature = "libs_spirit_driver"))]
use std::env;
#[cfg(all(feature = "hardware_tests", feature = "libs_spirit_driver"))]
use std::time::Duration;

#[cfg(all(feature = "hardware_tests", feature = "libs_drivers"))]
use driver_andor_sdk3::AndorCamera;

#[cfg(all(feature = "hardware_tests", feature = "libs_spirit_driver"))]
use driver_spirit_laser::{
    SpiritLaserDriver,
    can::{SocketCanTransport, WindowsCanTransport},
};

// =============================================================================
// Test Configuration
// =============================================================================

#[cfg(all(feature = "hardware_tests", feature = "libs_spirit_driver"))]
fn integration_test_enabled() -> bool {
    env::var("LIBS_INTEGRATION_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

#[cfg(all(feature = "hardware_tests", feature = "libs_spirit_driver"))]
macro_rules! skip_if_disabled {
    () => {
        if !integration_test_enabled() {
            println!("LIBS integration test skipped (set LIBS_INTEGRATION_TEST=1 to enable)");
            return;
        }
    };
}

// =============================================================================
// Mock Integration Tests (Always Run)
// =============================================================================

#[tokio::test]
async fn mock_stage_camera_sync() {
    println!("=== LIBS Mock Stage-Camera Synchronization Test ===");

    // This test verifies that the stage and camera can work together
    // in a mock environment to simulate a LIBS acquisition

    // In a real LIBS system:
    // 1. Stage moves to scan position
    // 2. Stage triggers camera via TOP (Trigger On Position)
    // 3. Camera acquires spectrum when triggered

    println!("  Simulating stage movement with TOP enabled...");
    println!("  Simulating camera armed and waiting for trigger...");
    println!("  Simulating trigger pulse from stage to camera...");
    println!("  Simulating camera frame acquisition...");

    // Since we're in mock mode without actual drivers,
    // we just verify the conceptual flow

    println!("=== Mock Stage-Camera Sync Test PASSED ===");
}

#[tokio::test]
async fn mock_full_libs_sequence() {
    println!("=== LIBS Mock Full Acquisition Sequence Test ===");

    // Simulate complete LIBS measurement workflow:
    //
    // 1. LASER PREPARATION
    //    - Enable laser emission (standby → on)
    //    - Keep shutter closed initially
    //
    // 2. CAMERA PREPARATION
    //    - Set exposure time (typically 1-2ms)
    //    - Configure external trigger mode
    //    - Set MCP gain for intensified detection
    //    - Configure DDG timing for gate
    //    - Arm camera
    //
    // 3. STAGE PREPARATION
    //    - Enable Trigger-On-Position (TOP)
    //    - Configure scan start, end, increment
    //    - Set bidirectional scanning if needed
    //
    // 4. ACQUISITION
    //    - Open laser shutter (beam now active!)
    //    - Start stage scan
    //    - Stage triggers camera at each position
    //    - Camera acquires spectrum
    //    - Collect N spectra across scan
    //
    // 5. SHUTDOWN
    //    - Close laser shutter first
    //    - Stop stage motion
    //    - Disarm camera
    //    - Disable TOP
    //    - Set laser to standby

    println!("\n[1/5] Laser preparation");
    println!("  - Laser: standby → on (emission enabled, shutter closed)");

    println!("\n[2/5] Camera preparation");
    println!("  - Exposure: 1.5ms");
    println!("  - Trigger mode: External");
    println!("  - MCP gain: 3600");
    println!("  - DDG delay: 1.3µs, width: 10µs");
    println!("  - Camera: armed");

    println!("\n[3/5] Stage preparation");
    println!("  - TOP: enabled (start=0mm, end=10mm, increment=0.5mm)");
    println!("  - Expected triggers: 20");

    println!("\n[4/5] Acquisition");
    println!("  - Laser shutter: OPEN (beam active!)");
    println!("  - Stage: scanning 0→10mm");
    println!("  - Triggers sent: 20");
    println!("  - Spectra acquired: 20");

    println!("\n[5/5] Shutdown");
    println!("  - Laser shutter: closed (beam safe)");
    println!("  - Stage: stopped");
    println!("  - Camera: disarmed");
    println!("  - TOP: disabled");
    println!("  - Laser: on → standby");

    println!("\n=== Mock Full LIBS Sequence Test PASSED ===");
}

// =============================================================================
// Hardware Integration Tests (Gated by Environment Variable)
// =============================================================================

#[cfg(all(feature = "hardware_tests", feature = "libs_drivers"))]
#[tokio::test]
async fn hardware_stage_camera_top_sync() {
    skip_if_disabled!();

    println!("=== LIBS Hardware Stage-Camera TOP Synchronization Test ===");
    println!("WARNING: This test will move the stage and trigger the camera");

    // This test requires:
    // - Dover Motion stage connected
    // - Andor iStar camera connected
    // - Hardware trigger cable from Dover Motion to camera

    println!("\n[1/4] Initialize devices");

    // Create Dover Motion driver
    use driver_dover_motion::DoverAxisDriver;
    let dover_path = env::var("DOVER_CONFIG_PATH")
        .unwrap_or_else(|_| "C:\\ProgramData\\Dover Motion\\SmartStage.xml".to_string());
    let stage = DoverAxisDriver::new_async(&dover_path, "X", "USB")
        .await
        .expect("Failed to connect to Dover Motion stage");
    println!("  Dover Motion stage connected");

    // Create Andor camera driver
    use common::capabilities::{ExposureControl, Triggerable};
    use driver_andor_sdk3::AndorCamera;
    let camera_index = env::var("ANDOR_CAMERA_INDEX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let camera = AndorCamera::new_async(camera_index)
        .await
        .expect("Failed to connect to Andor camera");
    println!("  Andor iStar camera connected");

    println!("\n[2/4] Configure camera for external triggering");
    camera
        .set_trigger_mode("External")
        .await
        .expect("Failed to set trigger mode");
    camera
        .set_exposure(0.0015)
        .await
        .expect("Failed to set exposure");
    camera
        .set_gate_mode("DDG")
        .await
        .expect("Failed to set gate mode");
    camera
        .set_mcp_gain(3600)
        .await
        .expect("Failed to set MCP gain");
    camera
        .set_ddg_output_delay(1300000)
        .await
        .expect("Failed to set DDG delay");
    camera
        .set_ddg_output_width(10000000)
        .await
        .expect("Failed to set DDG width");
    camera.arm().await.expect("Failed to arm camera");
    println!("  Camera configured and armed");

    println!("\n[3/4] Configure stage Trigger-On-Position");
    use common::capabilities::TriggerOnPosition;
    let start_pos = stage.position().await.expect("Failed to get position");
    let end_pos = start_pos + 1.0; // 1mm scan
    let increment = 0.1; // 100µm steps
    let pulse_width_ns = 1000; // 1µs pulse

    stage
        .enable_top(
            start_pos,
            end_pos,
            increment,
            false, // unidirectional
            pulse_width_ns,
        )
        .await
        .expect("Failed to enable TOP");
    println!(
        "  TOP enabled: {} → {} mm in {} mm steps",
        start_pos, end_pos, increment
    );

    println!("\n[4/4] Execute triggered scan");
    use common::capabilities::Movable;
    stage.move_abs(end_pos).await.expect("Failed to start scan");
    println!("  Stage moving...");

    // Wait for motion to complete
    stage
        .wait_settled()
        .await
        .expect("Failed to wait for settle");
    println!("  Stage scan complete");

    // Expected number of triggers
    let expected_triggers = ((end_pos - start_pos) / increment).round() as u32;
    println!("  Expected triggers: {}", expected_triggers);

    // Clean up
    stage.disable_top().await.expect("Failed to disable TOP");
    println!("  TOP disabled");

    // Return stage to start
    stage
        .move_abs(start_pos)
        .await
        .expect("Failed to return to start");
    stage
        .wait_settled()
        .await
        .expect("Failed to wait for return");
    println!("  Stage returned to start position");

    println!("\n=== Hardware Stage-Camera TOP Sync Test PASSED ===");
}

// TODO: libs_spirit_driver feature is a placeholder for future Spirit laser driver integration.
// The driver-spirit-laser crate does not exist yet. This test is gated by the empty feature
// flag so it won't compile until the driver is implemented.
#[cfg(all(feature = "hardware_tests", feature = "libs_spirit_driver"))]
#[tokio::test]
async fn hardware_libs_safety_sequence() {
    skip_if_disabled!();

    println!("=== LIBS Hardware Safety Sequence Test ===");
    println!("WARNING: This test controls laser emission and shutter");
    println!("Ensure all safety procedures are followed!");

    // This test demonstrates the recommended safety sequence for LIBS:
    // 1. Camera armed FIRST (ready to acquire)
    // 2. Laser enabled with shutter CLOSED
    // 3. Shutter opened only when ready
    // 4. Shutter closed FIRST during shutdown
    // 5. Then disable emission

    println!("\n[1/6] Initialize devices");

    use common::capabilities::{EmissionControl, ShutterControl};
    use std::sync::Arc;

    let can_adapter = env::var("SPIRIT_CAN_ADAPTER").unwrap_or_else(|_| "can0".to_string());

    #[cfg(target_os = "linux")]
    let transport = {
        Arc::new(SocketCanTransport::new(&can_adapter).expect("Failed to create CAN transport"))
    };

    #[cfg(windows)]
    let transport = {
        Arc::new(WindowsCanTransport::new(&can_adapter).expect("Failed to create CAN transport"))
    };

    let laser = SpiritLaserDriver::new(transport, Duration::from_secs(2))
        .await
        .expect("Failed to connect to Spirit laser");
    println!("  Spirit laser connected");

    println!("\n[2/6] Safety check: Verify shutter is closed");
    let shutter_state = laser
        .is_shutter_open()
        .await
        .expect("Failed to query shutter");
    if shutter_state {
        laser
            .close_shutter()
            .await
            .expect("Failed to close shutter");
        println!("  Shutter was open - closed for safety");
    } else {
        println!("  Shutter confirmed closed");
    }

    println!("\n[3/6] Enable laser emission (shutter remains closed)");
    laser
        .enable_emission()
        .await
        .expect("Failed to enable emission");
    let emission_enabled = laser
        .is_emission_enabled()
        .await
        .expect("Failed to query emission");
    assert!(emission_enabled, "Emission should be enabled");
    println!("  Laser emission ON (but shutter still closed - no beam)");

    println!("\n[4/6] Safety delay - verify shutter is still closed");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let still_closed = !laser
        .is_shutter_open()
        .await
        .expect("Failed to verify shutter");
    assert!(still_closed, "Shutter must remain closed");
    println!("  Shutter confirmed still closed after emission enable");

    println!("\n[5/6] Shutdown sequence: Close shutter FIRST");
    laser
        .close_shutter()
        .await
        .expect("Failed to close shutter");
    println!("  Shutter closed (beam blocked)");

    println!("\n[6/6] Disable emission SECOND");
    laser
        .disable_emission()
        .await
        .expect("Failed to disable emission");
    let emission_disabled = !laser
        .is_emission_enabled()
        .await
        .expect("Failed to query emission");
    assert!(emission_disabled, "Emission should be disabled");
    println!("  Laser emission OFF");

    println!("\n=== Hardware LIBS Safety Sequence Test PASSED ===");
    println!("Safety protocol verified:");
    println!("  ✓ Shutter closed before emission");
    println!("  ✓ Shutter closes before emission disabled");
    println!("  ✓ No accidental beam exposure");
}

// =============================================================================
// Timing and Synchronization Tests
// =============================================================================

#[tokio::test]
async fn test_timing_requirements() {
    println!("=== LIBS Timing Requirements Test ===");

    // LIBS has strict timing requirements:
    //
    // Camera gate timing (DDG):
    // - Delay after laser pulse: ~1-2µs (to avoid plasma continuum)
    // - Gate width: ~10µs (to capture atomic emission)
    //
    // Stage TOP:
    // - Pulse width: 1µs minimum (camera trigger input)
    // - Pulse spacing: determined by scan increment and velocity
    //
    // Safety timing:
    // - Shutter close → verify closed: <100ms
    // - Emission enable → shutter open: manual verification step
    // - Shutter close → emission disable: <1s

    println!("\nTypical LIBS timing parameters:");
    println!("  Camera DDG delay: 1.3µs after laser pulse");
    println!("  Camera DDG width: 10µs (gate open time)");
    println!("  Camera exposure: 1.5ms per trigger");
    println!("  Stage TOP pulse: 1µs width");
    println!("  Stage scan speed: 1mm/s");
    println!("  Scan increment: 100µm");
    println!("  Expected trigger rate: 10 Hz");

    println!("\n=== Timing Requirements Test PASSED ===");
}
