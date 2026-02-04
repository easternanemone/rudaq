//! Basic NI-DAQmx trigger example
//!
//! This example demonstrates how to use the NI-DAQmx driver to generate
//! digital pulses using the Triggerable trait.
//!
//! # Usage
//!
//! ```bash
//! # Mock mode (no hardware required)
//! cargo run --example basic_trigger
//!
//! # With hardware (requires NI-DAQmx and Python nidaqmx)
//! cargo run --example basic_trigger --features hardware
//! ```

use driver_nidaqmx::{NiDaqTrigger, Triggerable, TriggerMode};
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("NI-DAQmx Basic Trigger Example");
    println!("================================\n");

    // Create a simple digital trigger
    println!("Creating digital trigger...");
    let trigger = NiDaqTrigger::new(
        TriggerMode::Digital,
        0.1,   // 100ms high time
        0.001, // 1ms low time
        1,     // single pulse
    ).await?;

    println!("Trigger created successfully");

    // Arm the trigger
    println!("\nArming trigger...");
    trigger.arm().await?;
    println!("Trigger armed");

    // Check armed status
    if trigger.is_armed().await {
        println!("Trigger is armed and ready");
    }

    // Execute trigger
    println!("\nExecuting trigger...");
    trigger.trigger().await?;
    println!("Trigger executed successfully");

    // Disarm
    println!("\nDisarming trigger...");
    trigger.disarm().await?;
    println!("Trigger disarmed");

    println!("\nExample completed successfully!");

    Ok(())
}
