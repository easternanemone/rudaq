//! Trigger On Position (TOP) example
//!
//! This example demonstrates how to configure an external digital edge trigger
//! that responds to a signal on PFI0 (or another digital input).
//!
//! # Hardware Setup
//!
//! 1. Connect a digital signal source to /Dev1/PFI0
//! 2. Connect the counter output (e.g., /Dev1/Ctr0Out) to your target device
//! 3. When a rising edge is detected on PFI0, a pulse will be generated
//!
//! # Usage
//!
//! ```bash
//! # Mock mode (no hardware required)
//! cargo run --example trigger_on_position
//!
//! # With hardware (requires NI-DAQmx and Python nidaqmx)
//! cargo run --example trigger_on_position --features hardware
//! ```

use driver_nidaqmx::{NiDaqTrigger, Triggerable, TriggerMode};
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("NI-DAQmx Trigger On Position Example");
    println!("======================================\n");

    // Create a trigger that responds to external digital edge
    println!("Creating trigger-on-position...");
    let trigger = NiDaqTrigger::new(
        TriggerMode::TriggerOnPosition {
            trigger_source: "/Dev1/PFI0".to_string(),
            rising_edge: true,
            retriggerable: false,
        },
        0.001, // 1ms high time
        0.001, // 1ms low time
        1,     // single pulse per trigger
    ).await?;

    println!("Trigger created successfully");
    println!("  Trigger source: /Dev1/PFI0");
    println!("  Edge: Rising");
    println!("  Retriggerable: No");

    // Arm the trigger
    println!("\nArming trigger...");
    trigger.arm().await?;
    println!("Trigger armed and waiting for edge on PFI0");

    // In hardware mode, the trigger would now wait for an edge on PFI0
    // In mock mode, we simulate by calling trigger()
    println!("\nWaiting for external trigger signal...");
    println!("(In mock mode, simulating trigger execution)");

    trigger.trigger().await?;
    println!("Pulse generated in response to trigger");

    // Disarm
    println!("\nDisarming trigger...");
    trigger.disarm().await?;
    println!("Trigger disarmed");

    println!("\nExample completed successfully!");
    println!("\nNote: In hardware mode, connect a signal to /Dev1/PFI0");
    println!("      to trigger pulse generation automatically.");

    Ok(())
}
