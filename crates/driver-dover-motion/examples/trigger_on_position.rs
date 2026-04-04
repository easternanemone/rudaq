//! Trigger-on-Position (TOP) example for LIBS experiments
//!
//! This example demonstrates:
//! - Configuring TOP mode for synchronized triggering
//! - Performing a scan with automatic trigger generation
//! - Disabling TOP mode after scan

use anyhow::Result;
use common::capabilities::{Movable, TriggerOnPosition};
use driver_dover_motion::DoverMockDriver;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing for debug output
    tracing_subscriber::fmt::init();

    println!("Dover Motion Trigger-on-Position Example");
    println!("=========================================\n");

    // Create mock driver (no hardware required)
    let axis = DoverMockDriver::new("X");
    println!("Created mock Dover axis 'X'\n");

    // Configure TOP for LIBS scanning
    // Trigger every 100 µm from 0 to 10 mm
    let start_pos = 0.0; // mm
    let end_pos = 10.0; // mm
    let increment = 0.1; // mm (100 µm)
    let bidirectional = true; // Trigger in both directions
    let pulse_width_ns = 1000; // 1 µs pulse

    println!("Configuring Trigger-on-Position:");
    println!("  Start: {start_pos} mm");
    println!("  End: {end_pos} mm");
    println!("  Increment: {increment} mm (100 µm)");
    println!("  Bidirectional: {bidirectional}");
    println!("  Pulse width: {pulse_width_ns} ns\n");

    axis.enable_top(start_pos, end_pos, increment, bidirectional, pulse_width_ns)
        .await?;

    // Verify TOP is enabled
    let enabled = axis.is_top_enabled().await?;
    println!("TOP enabled: {enabled}\n");

    // Perform scan - triggers will fire automatically at position intervals
    println!("Performing forward scan (0 → 10 mm)...");
    axis.move_abs(end_pos).await?;
    axis.wait_settled().await?;
    println!("  → 100 triggers generated at 100 µm intervals");

    println!("\nPerforming reverse scan (10 → 0 mm)...");
    axis.move_abs(start_pos).await?;
    axis.wait_settled().await?;
    println!("  → 100 triggers generated at 100 µm intervals");

    // Disable TOP after scan
    println!("\nDisabling TOP...");
    axis.disable_top().await?;

    let enabled = axis.is_top_enabled().await?;
    println!("TOP enabled: {enabled}");

    println!("\nDone!");
    println!("\nIn a real LIBS experiment:");
    println!("- Each trigger would fire the laser and camera");
    println!("- Spectra would be captured at precise 100 µm intervals");
    println!("- Data would be correlated with position automatically");

    Ok(())
}
