//! Example: Hardware Control from Rhai Scripts
//!
//! This example demonstrates how to use Rhai scripts to control hardware devices.
//! It creates mock hardware (stage and camera) and executes a scientific workflow
//! defined in a Rhai script.
//!
//! Run with: cargo run --example scripting_hardware_demo

use rust_daq::hardware::capabilities::{FrameProducer, Movable};
use rust_daq::hardware::mock::{MockCamera, MockStage};
use rust_daq::scripting::{CameraHandle, RhaiEngine, ScriptEngine, ScriptValue, StageHandle};
use std::sync::Arc;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!("=== Rhai Hardware Scripting Demo ===\n");

    // Create mock hardware devices
    let stage = Arc::new(MockStage::new());
    let camera = Arc::new(MockCamera::new(1920, 1080));

    // Create engine with hardware bindings
    let mut engine = RhaiEngine::with_hardware().expect("Failed to create engine");

    // Create handles for script access
    engine
        .set_global(
            "stage",
            ScriptValue::new(StageHandle {
                driver: stage.clone(),
                data_tx: None,
            }),
        )
        .expect("Failed to set stage");

    engine
        .set_global(
            "camera",
            ScriptValue::new(CameraHandle {
                driver: camera.clone(),
                data_tx: None,
            }),
        )
        .expect("Failed to set camera");

    // Load and execute the demo script
    let script = include_str!("scripting_demo.rhai");

    println!("Executing Rhai script...\n");
    println!("--- Script Output ---");

    match engine.execute_script(script).await {
        Ok(result) => {
            println!("--- End Script Output ---\n");
            // Try to downcast to f64 if possible, or just print debug
            if let Some(val) = result.downcast_ref::<f64>() {
                println!("Script returned: {}", val);
            } else {
                println!("Script returned: {:?}", result);
            }
        }
        Err(e) => {
            eprintln!("Script error: {}", e);
        }
    }

    // Verify hardware state
    println!("\n=== Hardware State Verification ===");
    println!("Stage position: {:.2}mm", stage.position().await.unwrap());
    println!("Camera frames captured: {}", camera.frame_count());
    println!("Camera is armed: {}", camera.is_armed().await);
}
