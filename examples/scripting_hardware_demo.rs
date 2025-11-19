//! Example: Hardware Control from Rhai Scripts
//!
//! This example demonstrates how to use Rhai scripts to control hardware devices.
//! It creates mock hardware (stage and camera) and executes a scientific workflow
//! defined in a Rhai script.
//!
//! Run with: cargo run --example scripting_hardware_demo

use rust_daq::hardware::capabilities::Movable;
use rust_daq::hardware::mock::{MockCamera, MockStage};
use rust_daq::scripting::{CameraHandle, ScriptHost, StageHandle};
use std::sync::Arc;
use tokio::runtime::Handle;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!("=== Rhai Hardware Scripting Demo ===\n");

    // Create mock hardware devices
    let stage = Arc::new(MockStage::new());
    let camera = Arc::new(MockCamera::new(1920, 1080));

    // Create script host with hardware bindings
    let mut host = ScriptHost::with_hardware(Handle::current());

    // Create handles for script access
    let mut scope = rhai::Scope::new();
    scope.push("stage", StageHandle { driver: stage.clone() });
    scope.push(
        "camera",
        CameraHandle {
            driver: camera.clone(),
        },
    );

    // Load and execute the demo script
    let script = include_str!("scripting_demo.rhai");

    println!("Executing Rhai script...\n");
    println!("--- Script Output ---");

    match host.engine_mut().eval_with_scope::<f64>(&mut scope, script) {
        Ok(result) => {
            println!("--- End Script Output ---\n");
            println!("Script returned: {}", result);
        }
        Err(e) => {
            eprintln!("Script error: {}", e);
        }
    }

    // Verify hardware state
    println!("\n=== Hardware State Verification ===");
    println!("Stage position: {:.2}mm", stage.position().await.unwrap());
    println!("Camera frames captured: {}", camera.frame_count().await);
    println!("Camera is armed: {}", camera.is_armed().await);
}
