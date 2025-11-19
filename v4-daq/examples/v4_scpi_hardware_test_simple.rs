//! V4 Generic SCPI Instrument Hardware Test (Simplified)

use kameo::Actor;
use std::env;
use std::time::Duration;
use v4_daq::actors::scpi::{Identify, Query, ScpiActor};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    println!("=== V4 SCPI Hardware Test ===\n");

    let actor = ScpiActor::spawn(ScpiActor::mock("test_scpi".to_string()));

    // Test 1: Identify
    println!("Test 1: Identify (*IDN?)");
    let idn = actor.ask(Identify).await.map_err(|e| anyhow::anyhow!("Send failed: {}", e))?;
    println!("  Identity: {}\n", idn);

    // Test 2: Query
    println!("Test 2: Query (*STB?)");
    let response = actor.ask(Query { cmd: "*STB?".to_string() }).await.map_err(|e| anyhow::anyhow!("Send failed: {}", e))?;
    println!("  Response: {}\n", response);

    println!("=== Test Complete ===");
    Ok(())
}
