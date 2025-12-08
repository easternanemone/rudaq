//! Demonstration of configuration validation
//!
//! This example shows how the hardware registry validates device configurations
//! before attempting to spawn drivers, providing clear error messages.
//!
//! Run with:
//!   cargo run --example config_validation_demo --features instrument_serial

use rust_daq::hardware::registry::{DeviceConfig, DeviceRegistry, DriverType};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== Hardware Configuration Validation Demo ===\n");

    let mut registry = DeviceRegistry::new();

    // Example 1: Valid configuration (mock device)
    println!("1. Registering valid mock device...");
    match registry
        .register(DeviceConfig {
            id: "mock_stage".into(),
            name: "Mock Stage".into(),
            driver: DriverType::MockStage {
                initial_position: 0.0,
            },
        })
        .await
    {
        Ok(_) => println!("   ✓ Successfully registered mock_stage\n"),
        Err(e) => println!("   ✗ Failed: {}\n", e),
    }

    // Example 2: Invalid serial port
    println!("2. Attempting to register device with non-existent serial port...");
    match registry
        .register(DeviceConfig {
            id: "power_meter".into(),
            name: "Newport 1830-C Power Meter".into(),
            driver: DriverType::Newport1830C {
                port: "/dev/nonexistent_serial".into(),
            },
        })
        .await
    {
        Ok(_) => println!("   ✓ Successfully registered (unexpected)\n"),
        Err(e) => {
            println!("   ✗ Failed with validation error (expected):");
            println!("   {}\n", e);
        }
    }

    // Example 3: Invalid ELL14 address
    println!("3. Attempting to register ELL14 with invalid address...");
    match registry
        .register(DeviceConfig {
            id: "rotator".into(),
            name: "ELL14 Rotation Mount".into(),
            driver: DriverType::Ell14 {
                port: "/dev/ttyUSB0".into(),
                address: "ZZZ".into(), // Invalid - must be single hex digit
            },
        })
        .await
    {
        Ok(_) => println!("   ✓ Successfully registered (unexpected)\n"),
        Err(e) => {
            println!("   ✗ Failed with validation error (expected):");
            println!("   {}\n", e);
        }
    }

    // Example 4: Invalid ESP300 axis
    println!("4. Attempting to register ESP300 with invalid axis...");
    match registry
        .register(DeviceConfig {
            id: "stage_5".into(),
            name: "Newport ESP300 Axis 5".into(),
            driver: DriverType::Esp300 {
                port: "/dev/ttyUSB1".into(),
                axis: 5, // Invalid - must be 1-3
            },
        })
        .await
    {
        Ok(_) => println!("   ✓ Successfully registered (unexpected)\n"),
        Err(e) => {
            println!("   ✗ Failed with validation error (expected):");
            println!("   {}\n", e);
        }
    }

    // Example 5: Empty PVCAM camera name
    println!("5. Attempting to register PVCAM camera with empty name...");
    match registry
        .register(DeviceConfig {
            id: "camera".into(),
            name: "PVCAM Camera".into(),
            driver: DriverType::Pvcam {
                camera_name: "".into(), // Invalid - cannot be empty
            },
        })
        .await
    {
        Ok(_) => println!("   ✓ Successfully registered (unexpected)\n"),
        Err(e) => {
            println!("   ✗ Failed with validation error (expected):");
            println!("   {}\n", e);
        }
    }

    println!("=== Summary ===");
    println!("Successfully registered devices: {}", registry.len());
    println!("\nValidation prevents driver spawn failures by checking configurations early!");

    Ok(())
}
