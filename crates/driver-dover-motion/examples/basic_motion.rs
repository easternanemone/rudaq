//! Basic motion control example using Dover Motion driver
//!
//! This example demonstrates:
//! - Creating a mock Dover axis driver
//! - Performing absolute and relative moves
//! - Querying position
//! - Using the Movable trait

use anyhow::Result;
use common::capabilities::Movable;
use driver_dover_motion::DoverMockDriver;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing for debug output
    tracing_subscriber::fmt::init();

    println!("Dover Motion Basic Motion Example");
    println!("==================================\n");

    // Create mock driver (no hardware required)
    let axis = DoverMockDriver::new("X");
    println!("Created mock Dover axis 'X'\n");

    // Query initial position
    let pos = axis.position().await?;
    println!("Initial position: {} mm", pos);

    // Perform absolute move
    println!("\nMoving to absolute position 10.0 mm...");
    axis.move_abs(10.0).await?;
    axis.wait_settled().await?;
    let pos = axis.position().await?;
    println!("Current position: {} mm", pos);

    // Perform relative move
    println!("\nMoving relative +5.0 mm...");
    axis.move_rel(5.0).await?;
    axis.wait_settled().await?;
    let pos = axis.position().await?;
    println!("Current position: {} mm", pos);

    // Perform relative move in opposite direction
    println!("\nMoving relative -3.0 mm...");
    axis.move_rel(-3.0).await?;
    axis.wait_settled().await?;
    let pos = axis.position().await?;
    println!("Current position: {} mm", pos);

    println!("\nDone!");
    Ok(())
}
