//! V4 Generic SCPI Instrument Hardware Test
//!
//! Tests the ScpiActor with a real SCPI-compliant instrument.
//! Uses VisaAdapterV4 for hardware communication.
//!
//! ## Tested Instruments
//!
//! This example should work with any SCPI-compliant instrument:
//! - Oscilloscopes (Keysight, Tektronix, R&S)
//! - Multimeters (Keysight 34401A, 34410A, etc.)
//! - Power supplies (Keysight E3631A, etc.)
//! - Function generators (Keysight 33500B, etc.)
//!
//! ## Hardware Setup
//!
//! **Option 1: TCP/IP (LAN/Ethernet)**
//! ```bash
//! export SCPI_RESOURCE="TCPIP0::192.168.1.100::INSTR"
//! cargo run --example v4_scpi_hardware_test --features instrument_visa
//! ```
//!
//! **Option 2: USB**
//! ```bash
//! export SCPI_RESOURCE="USB0::0x0957::0x2007::MY12345678::INSTR"
//! cargo run --example v4_scpi_hardware_test --features instrument_visa
//! ```
//!
//! **Option 3: GPIB**
//! ```bash
//! export SCPI_RESOURCE="GPIB0::10::INSTR"
//! cargo run --example v4_scpi_hardware_test --features instrument_visa
//! ```
//!
//! ## Mock Mode (No Hardware)
//!
//! ```bash
//! cargo run --example v4_scpi_hardware_test
//! ```

use kameo::actor::ActorRef;
use kameo::Actor;
use std::env;
use std::time::Duration;
use tracing::{error, info, warn};
use v4_daq::actors::scpi::{
    ClearErrors, Identify, Query, QueryWithTimeout, ReadError, ScpiActor, Write,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("=== V4 SCPI Hardware Test ===\n");

    // Check for VISA resource from environment
    let resource = env::var("SCPI_RESOURCE").ok();

    let actor_ref: ActorRef<ScpiActor> = if let Some(resource_str) = resource {
        info!("Hardware Mode: {}", resource_str);
        info!("Creating SCPI actor with VISA adapter...\n");

        match ScpiActor::new(
            "test_scpi".to_string(),
            resource_str,
            Duration::from_secs(2),
        )
        .await
        {
            Ok(Ok(actor)) => ScpiActor::spawn(actor),
            Ok(Err(e)) | Err(e) => {
                error!("Failed to create SCPI actor: {}", e);
                warn!("Falling back to mock mode\n");
                ScpiActor::spawn(ScpiActor::mock("test_scpi".to_string()))
            }
        }
    } else {
        info!("Mock Mode: No SCPI_RESOURCE environment variable set");
        info!("Set SCPI_RESOURCE to test with real hardware\n");
        ScpiActor::spawn(ScpiActor::mock("test_scpi".to_string()))
    };

    // Test 1: Identify instrument
    info!("Test 1: Identify Instrument (*IDN?)");
    match actor_ref.ask(Identify).await {
        Ok(Ok(idn)) => {
            info!("  Identity: {}\n", idn);
        }
        Ok(Err(e)) => {
            error!("  Failed to read identity: {}\n", e);
        }
        Err(e) => {
            error!("  Failed to send message: {}\n", e);
        }
    }

    // Test 2: Clear errors
    info!("Test 2: Clear Error Queue (*CLS)");
    match actor_ref.ask(ClearErrors).await {
        Ok(Ok(())) => {
            info!("  Errors cleared successfully\n");
        }
        Ok(Err(e)) => {
            error!("  Failed to clear errors: {}\n", e);
        }
        Err(e) => {
            error!("  Failed to send message: {}\n", e);
        }
    }

    // Test 3: Read error register
    info!("Test 3: Read Error Register (*ESR?)");
    match actor_ref.ask(ReadError).await {
        Ok(Ok(esr)) => {
            info!("  Error Register: 0x{:02X}", esr);
            if esr == 0 {
                info!("  No errors present\n");
            } else {
                warn!("  Errors present in register\n");
            }
        }
        Ok(Err(e)) => {
            error!("  Failed to read error register: {}\n", e);
        }
        Err(e) => {
            error!("  Failed to send message: {}\n", e);
        }
    }

    // Test 4: Generic query (depends on instrument type)
    info!("Test 4: Generic Query Test");
    info!("  Querying instrument status (*STB?)");
    match actor_ref
        .ask(Query {
            cmd: "*STB?".to_string(),
        })
        .await
    {
        Ok(Ok(response)) => {
            info!("  Status Byte: {}\n", response);
        }
        Ok(Err(e)) => {
            error!("  Query failed: {}\n", e);
        }
        Err(e) => {
            error!("  Failed to send message: {}\n", e);
        }
    }

    // Test 5: Query with timeout
    info!("Test 5: Query with Custom Timeout");
    match actor_ref
        .ask(QueryWithTimeout {
            cmd: "*OPC?".to_string(),
            timeout: Duration::from_millis(500),
        })
        .await
    {
        Ok(Ok(response)) => {
            info!("  Operation Complete: {}\n", response);
        }
        Ok(Err(e)) => {
            error!("  Query failed: {}\n", e);
        }
        Err(e) => {
            error!("  Failed to send message: {}\n", e);
        }
    }

    // Test 6: Write command (enable output or similar)
    info!("Test 6: Write Command Test");
    info!("  Sending *CLS (clear status)");
    match actor_ref
        .ask(Write {
            cmd: "*CLS".to_string(),
        })
        .await
    {
        Ok(Ok(())) => {
            info!("  Command sent successfully\n");
        }
        Ok(Err(e)) => {
            error!("  Write failed: {}\n", e);
        }
        Err(e) => {
            error!("  Failed to send message: {}\n", e);
        }
    }

    // Test 7: Reset instrument (commented out by default - may take time)
    /*
    info!("Test 7: Reset Instrument (*RST)");
    warn!("  Resetting instrument to factory defaults...");
    match actor_ref.ask(Reset).await {
        Ok(Ok(())) => {
            info!("  Reset complete\n");
        }
        Ok(Err(e)) | Err(e) => {
            error!("  Reset failed: {}\n", e);
        }
    }
    */

    // Test 8: Instrument-specific query (example for multimeter)
    info!("Test 8: Instrument-Specific Query (if applicable)");
    info!("  Example: MEAS:VOLT:DC? (for multimeter)");
    match actor_ref
        .ask(Query {
            cmd: "MEAS:VOLT:DC?".to_string(),
        })
        .await
    {
        Ok(Ok(response)) => {
            match response.trim().parse::<f64>() {
                Ok(voltage) => {
                    info!("  Measured Voltage: {:.6} V\n", voltage);
                }
                Err(_) => {
                    info!("  Response: {} (not a voltage measurement)\n", response);
                }
            }
        }
        Ok(Err(e)) => {
            warn!("  Command not supported or failed: {}\n", e);
        }
        Err(e) => {
            warn!("  Failed to send message: {}\n", e);
        }
    }

    info!("=== Test Complete ===");
    info!("\nTo test with real hardware:");
    info!("  export SCPI_RESOURCE=\"TCPIP0::192.168.1.100::INSTR\"");
    info!("  cargo run --example v4_scpi_hardware_test --features instrument_visa\n");

    Ok(())
}
