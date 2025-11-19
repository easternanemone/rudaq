//! MaiTai Diagnostic Test - Raw Serial Communication
//!
//! Simple test to diagnose MaiTai serial responses

use anyhow::Result;
use std::env;
use v4_daq::hardware::SerialAdapterV4Builder;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rust_daq=debug".parse().unwrap()),
        )
        .init();

    let port = env::var("MAITAI_PORT").unwrap_or_else(|_| "/dev/ttyUSB5".to_string());

    println!("🔧 MaiTai Serial Diagnostic Test\n");
    println!("Port: {}\n", port);

    // Create adapter with very long timeout
    let adapter = SerialAdapterV4Builder::new(port, 9600)
        .with_line_terminator("\r".to_string())
        .with_response_delimiter('\r')
        .with_timeout(Duration::from_secs(5))
        .build();

    adapter.connect().await?;
    println!("✓ Connected to serial port");

    tokio::time::sleep(Duration::from_millis(300)).await;
    println!("✓ 300ms delay complete\n");

    // Test 1: Send wavelength command (no response expected)
    println!("Test 1: Set wavelength to 800 nm");
    adapter.send_command_no_response("WAVELENGTH:800").await?;
    println!("✓ Command sent\n");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Test 2: Try to query wavelength with extended timeout
    println!("Test 2: Query wavelength (5s timeout)");
    match adapter.send_command("WAVELENGTH?").await {
        Ok(response) => {
            println!("✓ Response received: '{}'", response);
            println!("  Response bytes: {:?}", response.as_bytes());
            println!("  Response length: {} bytes", response.len());
        }
        Err(e) => {
            println!("✗ Error: {}", e);
        }
    }

    println!("\n✅ Diagnostic complete");
    Ok(())
}
