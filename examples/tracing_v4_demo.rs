//! Demonstration of V4 Tracing Infrastructure
//!
//! This example shows how to initialize and use the V4 tracing system.
//!
//! Run with:
//! ```bash
//! cargo run --example tracing_v4_demo
//! ```
//!
//! Try different log levels:
//! ```bash
//! RUST_DAQ_APPLICATION_LOG_LEVEL=debug cargo run --example tracing_v4_demo
//! RUST_DAQ_APPLICATION_LOG_LEVEL=trace cargo run --example tracing_v4_demo
//! ```
//!
//! Use RUST_LOG for fine-grained filtering:
//! ```bash
//! RUST_LOG=rust_daq=debug cargo run --example tracing_v4_demo
//! ```

use rust_daq::{config_v4::V4Config, tracing_v4};
use tracing::{debug, error, info, info_span, warn};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Rust DAQ V4 Tracing Demo ===\n");

    // Load configuration from config.v4.toml
    println!("Loading configuration...");
    let config = V4Config::load()?;
    println!(
        "✓ Configuration loaded (log level: {})\n",
        config.application.log_level
    );

    // Initialize tracing from configuration
    println!("Initializing tracing...");
    tracing_v4::init_from_config(&config)?;
    println!("✓ Tracing initialized!\n");

    println!("=== Demonstrating Log Levels ===\n");

    // Demonstrate different log levels
    error!("This is an ERROR message - always visible");
    warn!("This is a WARN message - visible at warn level and below");
    info!("This is an INFO message - visible at info level and below");
    debug!("This is a DEBUG message - only visible at debug level");
    tracing::trace!("This is a TRACE message - only visible at trace level");

    println!("\n=== Demonstrating Structured Logging ===\n");

    // Structured fields
    info!(
        instrument = "mock_power_meter",
        reading = 42.5,
        unit = "mW",
        "Instrument reading received"
    );

    warn!(
        component = "storage",
        backend = "hdf5",
        error_count = 3,
        "Storage backend experiencing issues"
    );

    println!("\n=== Demonstrating Spans (Async Context) ===\n");

    // Spans provide context for operations
    let span = info_span!("data_acquisition", session_id = "demo-001");
    let _enter = span.enter();

    info!("Starting data acquisition session");

    simulate_instrument_reading();
    simulate_data_processing();

    info!("Data acquisition session complete");

    println!("\n=== Demonstrating Error Logging ===\n");

    // Logging errors with context
    let simulated_error = std::io::Error::new(std::io::ErrorKind::NotFound, "device not found");
    error!(
        error = ?simulated_error,
        device_path = "/dev/ttyUSB0",
        "Failed to open serial device"
    );

    println!("\n=== Environment Variable Options ===");
    println!("Override log level:");
    println!("  RUST_DAQ_APPLICATION_LOG_LEVEL=debug cargo run --example tracing_v4_demo");
    println!("\nFine-grained filtering:");
    println!("  RUST_LOG=rust_daq=debug cargo run --example tracing_v4_demo");
    println!("  RUST_LOG=rust_daq::instrument=trace cargo run --example tracing_v4_demo");

    Ok(())
}

fn simulate_instrument_reading() {
    let span = info_span!("instrument_reading", instrument = "mock_sensor");
    let _enter = span.enter();

    debug!("Connecting to instrument");
    debug!("Sending measurement command");
    info!(value = 123.45, unit = "V", "Measurement received");
}

fn simulate_data_processing() {
    let span = info_span!("data_processing", processor = "iir_filter");
    let _enter = span.enter();

    debug!("Applying IIR filter");
    debug!(cutoff_hz = 10.0, "Filter configuration");
    info!(samples_processed = 1000, "Data processing complete");
}
