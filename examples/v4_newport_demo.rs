//! V4 Newport 1830-C vertical slice demonstration
//!
//! Shows complete flow: Actor → Trait → Arrow data
//! Requires 'v4' and 'storage_arrow' features

#![cfg(all(feature = "v4", feature = "storage_arrow"))]

use anyhow::Result;
use kameo::actor::spawn;
use rust_daq::actors::Newport1830C;
use rust_daq::traits::{PowerMeter, PowerUnit, Wavelength};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Spawn Newport 1830-C actor with supervision
    let power_meter = spawn(Newport1830C::new());

    println!("🔬 V4 Newport 1830-C Vertical Slice Demo\n");

    // Configure instrument
    println!("Setting wavelength to 780 nm...");
    power_meter.set_wavelength(Wavelength { nm: 780.0 }).await?;

    println!("Setting unit to milliwatts...");
    power_meter.set_unit(PowerUnit::MilliWatts).await?;

    // Take measurements
    println!("\nTaking 5 measurements:");
    let mut measurements = Vec::new();
    for i in 1..=5 {
        let measurement = power_meter.read_power().await?;
        println!(
            "  {}. Power: {:.3} {:?} @ {} nm",
            i,
            measurement.power,
            measurement.unit,
            measurement.wavelength.map(|w| w.nm).unwrap_or(0.0)
        );
        measurements.push(measurement);
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // Convert to Arrow format
    println!("\nConverting to Apache Arrow format...");
    let arrow_batch = power_meter.to_arrow(&measurements)?;
    println!("  Schema: {:?}", arrow_batch.schema());
    println!("  Rows: {}", arrow_batch.num_rows());
    println!("  Columns: {}", arrow_batch.num_columns());

    // Display Arrow batch contents
    println!("\nArrow Batch Contents:");
    for i in 0..arrow_batch.num_rows() {
        let timestamp = arrow_batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::TimestampNanosecondArray>()
            .unwrap()
            .value(i);
        let power = arrow_batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::Float64Array>()
            .unwrap()
            .value(i);
        let wavelength = arrow_batch
            .column(2)
            .as_any()
            .downcast_ref::<arrow::array::Float64Array>()
            .unwrap()
            .value(i);

        println!(
            "  Row {}: timestamp={}, power={:.6}, wavelength={:.1}",
            i, timestamp, power, wavelength
        );
    }

    // Graceful shutdown
    power_meter.kill().await;
    println!("\n✅ Actor stopped gracefully");

    Ok(())
}

#[cfg(not(all(feature = "v4", feature = "storage_arrow")))]
fn main() {
    println!("This example requires both 'v4' and 'storage_arrow' features");
    println!("Run with: cargo run --example v4_newport_demo --features v4,storage_arrow");
}
