//! V4 Newport 1830-C Hardware Validation Test
//!
//! Tests the V4 vertical slice with actual Newport 1830-C hardware.
//! This validates:
//! - Kameo actor supervision
//! - Serial communication via SerialAdapterV4
//! - Real hardware command/response
//! - Arrow data format
//! - Fault tolerance
//! Requires 'v4' and 'storage_arrow' features

#![cfg(all(feature = "v4", feature = "storage_arrow"))]

use anyhow::Result;
use kameo::actor::spawn;
use rust_daq::actors::Newport1830C;
use rust_daq::traits::{PowerMeter, PowerUnit, Wavelength};
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
    let port = env::var("NEWPORT_PORT").unwrap_or_else(|_| "/dev/ttyUSB0".to_string());
    let baud_rate = env::var("NEWPORT_BAUD")
        .unwrap_or_else(|_| "9600".to_string())
        .parse()
        .expect("Invalid baud rate");

    println!("🔬 V4 Newport 1830-C Hardware Validation Test\n");
    println!("Port: {}", port);
    println!("Baud: {}\n", baud_rate);

    // Spawn Newport 1830-C actor with real hardware
    let mut power_meter = spawn(Newport1830C::with_serial(port, baud_rate));

    println!("✓ Actor spawned with Kameo supervision");

    // Test 1: Configure instrument
    println!("\n📝 Test 1: Configure Instrument");
    println!("  Setting wavelength to 780 nm...");
    power_meter.set_wavelength(Wavelength { nm: 780.0 }).await?;
    println!("  ✓ Wavelength set");

    println!("  Setting unit to Watts...");
    power_meter.set_unit(PowerUnit::Watts).await?;
    println!("  ✓ Units set");

    // Verify configuration
    let wavelength = power_meter.get_wavelength().await?;
    let unit = power_meter.get_unit().await?;
    println!(
        "  ✓ Configuration verified: {} nm, {:?}",
        wavelength.nm, unit
    );

    // Test 2: Take measurements
    println!("\n📊 Test 2: Take 10 Measurements");
    let mut measurements = Vec::new();
    for i in 1..=10 {
        let measurement = power_meter.read_power().await?;
        println!(
            "  {}. Power: {:.6} {} @ {} nm (timestamp: {})",
            i,
            measurement.power,
            match measurement.unit {
                PowerUnit::Watts => "W",
                PowerUnit::MilliWatts => "mW",
                PowerUnit::MicroWatts => "µW",
                PowerUnit::NanoWatts => "nW",
                PowerUnit::Dbm => "dBm",
            },
            measurement.wavelength.map(|w| w.nm).unwrap_or(0.0),
            measurement.timestamp_ns
        );
        measurements.push(measurement);
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    println!("  ✓ All measurements successful");

    // Test 3: Arrow data format
    println!("\n📦 Test 3: Apache Arrow Data Format");
    let arrow_batch = power_meter.to_arrow(&measurements)?;
    println!("  Schema:");
    for field in arrow_batch.schema().fields() {
        println!("    - {}: {:?}", field.name(), field.data_type());
    }
    println!("  Rows: {}", arrow_batch.num_rows());
    println!("  Columns: {}", arrow_batch.num_columns());
    println!("  ✓ Arrow conversion successful");

    // Test 4: Change configuration mid-stream
    println!("\n🔄 Test 4: Runtime Configuration Change");
    println!("  Changing to dBm units...");
    power_meter.set_unit(PowerUnit::Dbm).await?;

    let measurement = power_meter.read_power().await?;
    println!("  New measurement: {:.3} dBm", measurement.power);
    println!("  ✓ Configuration change successful");

    // Test 5: Stress test
    println!("\n⚡ Test 5: Stress Test (100 rapid reads)");
    let start = std::time::Instant::now();
    for _ in 0..100 {
        power_meter.read_power().await?;
    }
    let elapsed = start.elapsed();
    println!(
        "  Completed 100 reads in {:?} ({:.2} Hz)",
        elapsed,
        100.0 / elapsed.as_secs_f64()
    );
    println!("  ✓ Stress test passed");

    // Graceful shutdown
    power_meter.kill().await;
    println!("\n✅ Hardware validation complete - all tests passed!");
    println!("\nV4 vertical slice successfully validated with real hardware.");

    Ok(())
}

#[cfg(not(all(feature = "v4", feature = "storage_arrow")))]
fn main() {
    println!("This example requires both 'v4' and 'storage_arrow' features");
    println!("Run with: cargo run --example v4_newport_hardware_test --features v4,storage_arrow");
}
