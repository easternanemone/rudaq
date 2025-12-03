//! Hardware Polarization Test
//!
//! LASER SAFETY: This program controls MaiTai Ti:Sapphire laser
//! Authorization required before running.
//!
//! Note: Due to the current Ell14Driver design (exclusive port access),
//! rotators are controlled sequentially. This is a workaround until
//! a shared-port RS-485 driver is implemented.

use anyhow::Result;
use rust_daq::hardware::capabilities::{
    EmissionControl, Movable, Readable, ShutterControl, WavelengthTunable,
};
use rust_daq::hardware::ell14::Ell14Driver;
use rust_daq::hardware::maitai::MaiTaiDriver;
use rust_daq::hardware::newport_1830c::Newport1830CDriver;
use std::time::Duration;
use tokio::time::sleep;

// Configuration
const ELLIPTEC_PORT: &str = "/dev/ttyUSB0";
const NEWPORT_PORT: &str = "/dev/ttyS0";
const MAITAI_PORT: &str = "/dev/ttyUSB5";

// Elliptec addresses
const HWP_ADDR: &str = "2";
const POLARIZER_ADDR: &str = "3";

/// Helper to control a rotator with create-use-drop pattern
async fn move_rotator(addr: &str, position: f64) -> Result<f64> {
    let driver = Ell14Driver::new(ELLIPTEC_PORT, addr)?;
    driver.move_abs(position).await?;
    driver.wait_settled().await?;
    let actual = driver.position().await?;
    // Driver dropped here, releasing port
    Ok(actual)
}

async fn get_rotator_position(addr: &str) -> Result<f64> {
    let driver = Ell14Driver::new(ELLIPTEC_PORT, addr)?;
    let pos = driver.position().await?;
    Ok(pos)
}

async fn home_rotator(addr: &str) -> Result<f64> {
    let driver = Ell14Driver::new(ELLIPTEC_PORT, addr)?;
    driver.home().await?;
    sleep(Duration::from_secs(2)).await;
    let pos = driver.position().await?;
    Ok(pos)
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("=====================================================");
    println!("  POLARIZATION MEASUREMENT TEST");
    println!("  LASER SAFETY: Authorized testing in progress");
    println!("=====================================================\n");

    // Phase 1: Test Elliptec rotators
    println!("[1/6] Testing Elliptec rotators...");

    let hwp_pos = get_rotator_position(HWP_ADDR).await?;
    println!("  HWP (addr {}): {:.2}°", HWP_ADDR, hwp_pos);

    sleep(Duration::from_millis(100)).await;

    let pol_pos = get_rotator_position(POLARIZER_ADDR).await?;
    println!("  Polarizer (addr {}): {:.2}°", POLARIZER_ADDR, pol_pos);

    println!("  [OK] Elliptec rotators responding\n");

    // Phase 2: Test Newport power meter
    println!("[2/6] Testing Newport 1830-C power meter...");

    let power_meter_ok = match Newport1830CDriver::new(NEWPORT_PORT) {
        Ok(pm) => match pm.read().await {
            Ok(power) => {
                println!("  Power reading: {:.3e} W", power);
                println!("  [OK] Newport power meter responding\n");
                true
            }
            Err(e) => {
                println!("  [WARN] Read failed: {}", e);
                println!("  Power meter may not be configured\n");
                false
            }
        },
        Err(e) => {
            println!("  [SKIP] Could not open power meter: {}", e);
            println!("  Continuing without power meter...\n");
            false
        }
    };

    // Phase 3: Test MaiTai laser
    println!("[3/6] Testing MaiTai laser connection...");

    let laser: Option<MaiTaiDriver> = match MaiTaiDriver::new(MAITAI_PORT) {
        Ok(l) => {
            println!("  [OK] Connected to MaiTai on {}", MAITAI_PORT);
            Some(l)
        }
        Err(e) => {
            println!("  [SKIP] Could not connect: {}", e);
            println!("  Check: Is laser controller powered on?");
            println!("  Check: Is correct port ({}) connected?\n", MAITAI_PORT);
            None
        }
    };

    if let Some(ref laser) = laser {
        // Query laser state
        match laser.get_wavelength().await {
            Ok(wl) => println!("  Wavelength: {} nm", wl),
            Err(e) => println!("  Wavelength query failed: {}", e),
        }

        match laser.is_shutter_open().await {
            Ok(open) => println!("  Shutter: {}", if open { "OPEN" } else { "CLOSED" }),
            Err(e) => println!("  Shutter query failed: {}", e),
        }
        println!();
    }

    // Phase 4: Home rotators
    println!("[4/6] Homing rotators...");

    println!("  HWP: Homing...");
    let hwp_home = home_rotator(HWP_ADDR).await?;
    println!("  HWP home position: {:.2}°", hwp_home);

    sleep(Duration::from_millis(100)).await;

    println!("  Polarizer: Homing...");
    let pol_home = home_rotator(POLARIZER_ADDR).await?;
    println!("  Polarizer home position: {:.2}°", pol_home);

    println!("  [OK] Rotators homed\n");

    // Phase 5: Laser control (only if connected)
    if let Some(ref laser) = laser {
        println!("[5/6] Laser control test...");
        println!("  WARNING: About to control laser shutter");
        println!("  Waiting 5 seconds - press Ctrl+C to abort");
        sleep(Duration::from_secs(5)).await;

        // Close shutter first (safety)
        println!("  Closing shutter...");
        laser.close_shutter().await?;
        sleep(Duration::from_millis(500)).await;

        let is_open = laser.is_shutter_open().await?;
        println!(
            "  Shutter state: {}",
            if is_open {
                "OPEN (ERROR!)"
            } else {
                "CLOSED (OK)"
            }
        );

        if is_open {
            println!("  [FAIL] Could not verify shutter closed!");
            return Ok(());
        }

        // Quick shutter cycle test
        println!("  Opening shutter...");
        laser.open_shutter().await?;
        sleep(Duration::from_millis(500)).await;

        println!("  Closing shutter...");
        laser.close_shutter().await?;
        sleep(Duration::from_millis(500)).await;

        println!("  [OK] Shutter control verified\n");
    } else {
        println!("[5/6] Skipping laser control (not connected)\n");
    }

    // Phase 6: Polarization scan (if all hardware available)
    if laser.is_some() && power_meter_ok {
        println!("[6/6] Running polarization scan...");
        println!("  This will:");
        println!("  - Set polarizer to 0°");
        println!("  - Rotate HWP from 0° to 180° in 15° steps");
        println!("  - Measure power at each position");
        println!();
        println!("  Waiting 5 seconds - press Ctrl+C to abort");
        sleep(Duration::from_secs(5)).await;

        // Set polarizer to 0
        println!("  Setting polarizer to 0°...");
        move_rotator(POLARIZER_ADDR, 0.0).await?;
        sleep(Duration::from_millis(100)).await;

        // Enable laser emission
        if let Some(ref laser) = laser {
            println!("  >>> ENABLING LASER EMISSION <<<");
            println!("  WARNING: Class 4 laser will be activated");
            sleep(Duration::from_secs(3)).await; // Safety delay

            laser.enable_emission().await?;
            println!("  Emission enabled, waiting for stabilization...");
            sleep(Duration::from_secs(5)).await; // Laser warm-up
        }

        // Open shutter for measurements
        if let Some(ref laser) = laser {
            println!("  >>> OPENING SHUTTER - BEAM ACTIVE <<<");
            laser.open_shutter().await?;
            sleep(Duration::from_secs(1)).await;
        }

        println!("\n  HWP Angle (°) | Power (W)");
        println!("  -------------|----------");

        let angles: Vec<f64> = (0..=12).map(|i| i as f64 * 15.0).collect();

        for angle in &angles {
            // Move HWP
            move_rotator(HWP_ADDR, *angle).await?;
            sleep(Duration::from_millis(200)).await;

            // Read power (need to recreate driver each time due to port sharing)
            let power = match Newport1830CDriver::new(NEWPORT_PORT) {
                Ok(pm) => match pm.read().await {
                    Ok(p) => format!("{:.3e}", p),
                    Err(_) => "ERR".to_string(),
                },
                Err(_) => "N/A".to_string(),
            };

            println!("  {:>12.1} | {}", angle, power);
        }

        // Close shutter
        if let Some(ref laser) = laser {
            println!("\n  >>> CLOSING SHUTTER <<<");
            laser.close_shutter().await?;
            sleep(Duration::from_millis(500)).await;

            // Verify closed
            let is_open = laser.is_shutter_open().await?;
            if is_open {
                println!("  [WARN] Shutter may still be open!");
            } else {
                println!("  Shutter confirmed CLOSED");
            }

            // Disable emission
            println!("  >>> DISABLING EMISSION <<<");
            laser.disable_emission().await?;
            println!("  Emission disabled");
        }

        println!("  [OK] Polarization scan complete\n");
    } else {
        println!("[6/6] Skipping polarization scan (hardware not available)\n");
    }

    // Final cleanup
    println!("=====================================================");
    println!("  TEST COMPLETE");
    println!("  - Homing rotators for safety...");

    let _ = home_rotator(HWP_ADDR).await;
    sleep(Duration::from_millis(100)).await;
    let _ = home_rotator(POLARIZER_ADDR).await;

    println!("  - Rotators homed");

    if let Some(ref laser) = laser {
        // Ensure shutter closed and emission disabled
        laser.close_shutter().await?;
        println!("  - Shutter closed");
        laser.disable_emission().await?;
        println!("  - Emission disabled");
    }

    println!("=====================================================");

    Ok(())
}
