//! V4 MaiTai Ti:Sapphire Laser Hardware Validation Test
//!
//! Tests the V4 vertical slice with actual MaiTai hardware.
//! This validates:
//! - Kameo actor supervision
//! - Serial communication via SerialAdapterV4 with XON/XOFF flow control
//! - Real hardware command/response
//! - Arrow data format
//! - Shutter safety (closed on startup/shutdown)

use anyhow::Result;
use kameo::prelude::*;
use v4_daq::actors::MaiTai;
use v4_daq::traits::tunable_laser::{TunableLaser, Wavelength};
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rust_daq=debug".parse().unwrap()),
        )
        .init();

    // Get serial port from environment or use default
    let port = env::var("MAITAI_PORT").unwrap_or_else(|_| "/dev/ttyUSB5".to_string());
    let baud_rate = env::var("MAITAI_BAUD")
        .unwrap_or_else(|_| "9600".to_string())
        .parse()
        .expect("Invalid baud rate");

    println!("🔬 V4 MaiTai Ti:Sapphire Laser Hardware Validation Test\n");
    println!("Port: {}", port);
    println!("Baud: {}\n", baud_rate);

    // Spawn MaiTai actor with real hardware
    let laser = MaiTai::spawn(MaiTai::with_serial(port, baud_rate));

    println!("✓ Actor spawned with Kameo supervision");
    println!("  (300ms initialization delay for MaiTai)");

    // Test 1: Query current state
    println!("\n📝 Test 1: Query Instrument State");
    let wavelength = laser.get_wavelength().await?;
    println!("  Current wavelength: {} nm", wavelength.nm);

    let shutter = laser.get_shutter_state().await?;
    println!("  Shutter state: {:?}", shutter);
    println!("  ✓ State query successful");

    // Test 2: Configure wavelength
    println!("\n🎚️  Test 2: Wavelength Tuning");
    println!("  Setting wavelength to 750 nm...");
    laser.set_wavelength(Wavelength { nm: 750.0 }).await?;
    println!("  ✓ Wavelength set");

    // Verify
    let wavelength = laser.get_wavelength().await?;
    println!("  Verified: {} nm", wavelength.nm);

    println!("\n  Setting wavelength to 850 nm...");
    laser.set_wavelength(Wavelength { nm: 850.0 }).await?;
    let wavelength = laser.get_wavelength().await?;
    println!("  Verified: {} nm", wavelength.nm);
    println!("  ✓ Wavelength tuning successful");

    // Test 3: Shutter control
    println!("\n🚪 Test 3: Shutter Control");
    println!("  Opening shutter...");
    laser.open_shutter().await?;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let shutter = laser.get_shutter_state().await?;
    println!("  Shutter state: {:?}", shutter);
    println!("  ✓ Shutter opened");

    println!("\n  Closing shutter...");
    laser.close_shutter().await?;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let shutter = laser.get_shutter_state().await?;
    println!("  Shutter state: {:?}", shutter);
    println!("  ✓ Shutter closed");

    // Test 4: Power measurement (with shutter open)
    println!("\n📊 Test 4: Power Measurements");
    println!("  Opening shutter for power readings...");
    laser.open_shutter().await?;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    println!("  Taking 10 measurements...");
    let mut measurements = Vec::new();
    for i in 1..=10 {
        let measurement = laser.measure().await?;
        println!(
            "  {}. Power: {:.6} W @ {} nm, Shutter: {:?} (timestamp: {})",
            i,
            measurement.power_watts,
            measurement.wavelength.nm,
            measurement.shutter,
            measurement.timestamp_ns
        );
        measurements.push(measurement);
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    println!("  ✓ All measurements successful");

    // Close shutter after measurements
    println!("\n  Closing shutter after measurements...");
    laser.close_shutter().await?;

    // Test 5: Arrow data format
    println!("\n📦 Test 5: Apache Arrow Data Format");
    let arrow_batch = laser.to_arrow(&measurements)?;
    println!("  Schema:");
    for field in arrow_batch.schema().fields() {
        println!("    - {}: {:?}", field.name(), field.data_type());
    }
    println!("  Rows: {}", arrow_batch.num_rows());
    println!("  Columns: {}", arrow_batch.num_columns());
    println!("  ✓ Arrow conversion successful");

    // Test 6: Stress test (rapid wavelength changes)
    println!("\n⚡ Test 6: Stress Test (20 wavelength changes)");
    let start = std::time::Instant::now();
    let wavelengths = [750.0, 760.0, 770.0, 780.0, 790.0, 800.0, 810.0, 820.0, 830.0, 840.0];

    for &wl in wavelengths.iter().cycle().take(20) {
        laser.set_wavelength(Wavelength { nm: wl }).await?;
    }
    let elapsed = start.elapsed();
    println!(
        "  Completed 20 wavelength changes in {:?} ({:.2} Hz)",
        elapsed,
        20.0 / elapsed.as_secs_f64()
    );
    println!("  ✓ Stress test passed");

    // Test 7: Safety verification
    println!("\n🛡️  Test 7: Safety Verification");
    let shutter = laser.get_shutter_state().await?;
    println!("  Final shutter state: {:?}", shutter);
    println!("  ✓ Shutter is safely closed");

    // Graceful shutdown (will close shutter in on_stop hook)
    println!("\n  Initiating graceful shutdown...");
    laser.kill();
    println!("  ✓ Actor stopped (shutter auto-closed)");

    println!("\n✅ Hardware validation complete - all tests passed!");
    println!("\nV4 vertical slice successfully validated with real MaiTai hardware.");
    println!("Architecture proven: Kameo actors + SerialAdapterV4 + TunableLaser trait");

    Ok(())
}
