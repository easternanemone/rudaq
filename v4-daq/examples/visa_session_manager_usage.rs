//! Example: Using VisaSessionManager for safe VISA instrument access
//!
//! This example demonstrates how to use the VisaSessionManager to safely access
//! VISA instruments from multiple concurrent actors without conflicts.
//!
//! The VisaSessionManager solves VISA's single-session limitation by:
//! - Queueing all commands FIFO
//! - Executing them sequentially in a dedicated task
//! - Supporting per-command timeouts
//! - Allowing multiple actors to share the same session safely

use std::sync::Arc;
use std::time::Duration;
use v4_daq::hardware::VisaSessionManager;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // Create the VisaSessionManager (can be shared globally)
    let manager = Arc::new(VisaSessionManager::new());
    let resource = "TCPIP0::192.168.1.100::INSTR";

    println!("=== VISA Session Manager Example ===\n");

    // Get or create a session (idempotent - returns existing if already created)
    let handle = manager
        .get_or_create_session(resource)
        .await?;

    println!("Created VISA session: {}", handle.resource_name());
    println!("Active sessions: {}\n", manager.session_count().await);

    // Example 1: Query command
    println!("--- Query Command Example ---");
    match handle.query("*IDN?", Duration::from_secs(2)).await {
        Ok(response) => println!("Instrument ID: {}", response),
        Err(e) => println!("Query failed: {}", e),
    }

    // Example 2: Write command
    println!("\n--- Write Command Example ---");
    match handle.write("*RST").await {
        Ok(_) => println!("Reset command sent successfully"),
        Err(e) => println!("Write failed: {}", e),
    }

    // Example 3: Multiple commands in sequence
    println!("\n--- Sequential Commands ---");
    let commands = vec!["*IDN?", "*OPC?", ":VOLT?"];

    for cmd in commands {
        match handle.query(cmd, Duration::from_secs(1)).await {
            Ok(response) => println!("Command '{}': {}", cmd, response),
            Err(e) => println!("Command '{}' failed: {}", cmd, e),
        }
    }

    // Example 4: Concurrent access from multiple "actors"
    println!("\n--- Concurrent Actor Access ---");

    let mut tasks = vec![];

    // Simulate 3 concurrent actors
    for actor_id in 0..3 {
        let h = handle.clone();
        let task = tokio::spawn(async move {
            let cmd = format!("MEAS{}?", actor_id);
            match h.query(&cmd, Duration::from_secs(1)).await {
                Ok(response) => {
                    println!("Actor {} - Command '{}': {}", actor_id, cmd, response);
                }
                Err(e) => {
                    println!("Actor {} - Command '{}' failed: {}", actor_id, cmd, e);
                }
            }
        });
        tasks.push(task);
    }

    // Wait for all actors to complete
    for task in tasks {
        let _ = task.await;
    }

    // Example 5: Handle cloning for sharing across subsystems
    println!("\n--- Handle Cloning (Share Across V2 and V4) ---");

    let handle_v2 = handle.clone();
    let handle_v4 = handle.clone();

    // V2 subsystem uses handle_v2
    println!("V2 subsystem can use: {}", handle_v2.resource_name());

    // V4 subsystem uses handle_v4
    println!("V4 subsystem can use: {}", handle_v4.resource_name());

    println!("\nBoth subsystems safely share the same VISA session!");

    // Example 6: Multiple independent sessions
    println!("\n--- Multiple Independent Sessions ---");

    let handle2 = manager
        .get_or_create_session("TCPIP0::192.168.1.101::INSTR")
        .await?;

    let handle3 = manager
        .get_or_create_session("GPIB0::1::INSTR")
        .await?;

    println!("Session 1: {}", handle.resource_name());
    println!("Session 2: {}", handle2.resource_name());
    println!("Session 3: {}", handle3.resource_name());
    println!("Total active sessions: {}", manager.session_count().await);

    // Clean up (optional in this example)
    let _ = manager.close_session(resource).await;
    println!("\nClosed session for {}", resource);
    println!("Remaining sessions: {}", manager.session_count().await);

    println!("\n=== Example Complete ===");
    Ok(())
}
