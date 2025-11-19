//! Example demonstrating V4 configuration loading
//!
//! This example shows how to load and validate configuration from TOML files.
//!
//! To run:
//! ```bash
//! cargo run --example test_config_load
//! ```

use v4_daq::config::V4Config;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("V4 Configuration System Test\n");
    println!("========================================\n");

    // Try to load configuration
    match V4Config::load() {
        Ok(config) => {
            println!("Successfully loaded configuration!");
            println!("\nApplication Settings:");
            println!("  Name: {}", config.application.name);
            println!("  Log Level: {}", config.application.log_level);
            if let Some(data_dir) = &config.application.data_dir {
                println!("  Data Directory: {}", data_dir.display());
            }

            println!("\nActor System Settings:");
            println!("  Mailbox Capacity: {}", config.actors.default_mailbox_capacity);
            println!("  Spawn Timeout (ms): {}", config.actors.spawn_timeout_ms);
            println!("  Shutdown Timeout (ms): {}", config.actors.shutdown_timeout_ms);

            println!("\nStorage Settings:");
            println!("  Backend: {}", config.storage.default_backend);
            println!("  Output Directory: {}", config.storage.output_dir.display());
            println!("  Compression Level: {}", config.storage.compression_level);
            println!("  Auto-flush Interval (s): {}", config.storage.auto_flush_interval_secs);

            println!("\nInstruments:");
            for instrument in &config.instruments {
                println!("  {} (type: {})", instrument.id, instrument.r#type);
                println!("    Enabled: {}", instrument.enabled);
            }

            println!("\nEnabled Instruments:");
            for instrument in config.enabled_instruments() {
                println!("  - {} ({})", instrument.id, instrument.r#type);
            }

            println!("\n========================================");
            println!("Configuration validation: PASSED");
            Ok(())
        }
        Err(e) => {
            eprintln!("Failed to load configuration: {}", e);
            eprintln!("\nNote: If you see 'file not found' error above, this is expected.");
            eprintln!("Please ensure config/config.v4.toml exists in the project root.");
            Ok(())
        }
    }
}
