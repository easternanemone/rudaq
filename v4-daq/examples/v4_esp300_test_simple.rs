//! V4 ESP300 Motion Controller Test (Simplified)

use kameo::Actor;
use v4_daq::actors::esp300::{ESP300, Home, MoveAbsolute, ReadPosition, Stop};
use v4_daq::traits::motion_controller::MotionConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("=== V4 ESP300 Motion Controller Test ===\n");

    // Create mock ESP300 with 3 axes
    let actor = ESP300::spawn(ESP300::mock("test_esp300".to_string(), 3));

    // Test 1: Home axis 0
    println!("Test 1: Home axis 0");
    actor
        .ask(Home { axis: 0 })
        .await
        .map_err(|e| anyhow::anyhow!("Send failed: {}", e))?;
    println!("  ✓ Axis 0 homed\n");

    // Test 2: Move to position 10.0
    println!("Test 2: Move axis 0 to position 10.0");
    actor
        .ask(MoveAbsolute {
            axis: 0,
            position: 10.0,
        })
        .await
        .map_err(|e| anyhow::anyhow!("Send failed: {}", e))?;
    println!("  ✓ Move command sent\n");

    // Test 3: Read position
    println!("Test 3: Read current position");
    let position = actor
        .ask(ReadPosition { axis: 0 })
        .await
        .map_err(|e| anyhow::anyhow!("Send failed: {}", e))?;
    println!("  Position: {} units\n", position);

    // Test 4: Stop motion
    println!("Test 4: Stop all axes");
    actor
        .ask(Stop { axis: None })
        .await
        .map_err(|e| anyhow::anyhow!("Send failed: {}", e))?;
    println!("  ✓ All axes stopped\n");

    println!("=== Test Complete ===");

    actor.kill();
    actor.wait_for_shutdown().await;

    Ok(())
}
