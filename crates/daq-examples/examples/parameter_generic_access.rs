//! Example: Generic Parameter Access via ParameterBase trait
//!
//! Demonstrates how gRPC services can access parameters generically
//! without knowing their concrete types at compile time.

use anyhow::Result;
use common::observable::{Observable, ParameterSet};

fn main() -> Result<()> {
    println!("=== Generic Parameter Access Example ===\n");

    // Create a parameter set with heterogeneous types
    let mut params = ParameterSet::new();

    params.register(
        Observable::new("wavelength_nm", 800.0)
            .with_units("nm")
            .with_description("Laser wavelength")
            .with_range(700.0, 1000.0),
    );

    params.register(
        Observable::new("power_mw", 100.0)
            .with_units("mW")
            .with_description("Laser power"),
    );

    params.register(Observable::new("shutter_open", false).with_description("Shutter state"));

    params.register(Observable::new("mode", "auto".to_string()).with_description("Operating mode"));

    println!("Registered {} parameters\n", params.names().len());

    // Scenario 1: List all parameters (like gRPC list_parameters)
    println!("--- All Parameters ---");
    for (name, param) in params.iter() {
        let metadata = param.metadata();
        let units = metadata.units.as_deref().unwrap_or("none");
        let value = param.get_json()?;
        println!("  {}: {} [{}]", name, value, units);
    }
    println!();

    // Scenario 2: Get and set parameter by name (like gRPC set_parameter)
    println!("--- Generic Set/Get ---");

    if let Some(param) = params.get("wavelength_nm") {
        println!("Current wavelength: {}", param.get_json()?);

        // Set new value via JSON (type-safe deserialization)
        param.set_json(serde_json::json!(850.0))?;
        println!("Updated wavelength: {}", param.get_json()?);
    }

    if let Some(param) = params.get("shutter_open") {
        println!("Current shutter: {}", param.get_json()?);
        param.set_json(serde_json::json!(true))?;
        println!("Updated shutter: {}", param.get_json()?);
    }
    println!();

    // Scenario 3: Type mismatch handling
    println!("--- Type Mismatch Error Handling ---");

    if let Some(param) = params.get("wavelength_nm") {
        // Try to set a string to a numeric parameter
        match param.set_json(serde_json::json!("not a number")) {
            Ok(_) => println!("  ERROR: Should have failed!"),
            Err(e) => println!("  Expected error: {}", e),
        }
    }
    println!();

    // Scenario 4: Validation errors
    println!("--- Validation Error Handling ---");

    if let Some(param) = params.get("wavelength_nm") {
        // Try to set value out of range
        match param.set_json(serde_json::json!(1200.0)) {
            Ok(_) => println!("  ERROR: Should have failed validation!"),
            Err(e) => println!("  Expected error: {}", e),
        }
    }
    println!();

    // Scenario 5: Still support typed access when type is known
    println!("--- Typed Access (backwards compatible) ---");

    if let Some(wavelength) = params.get_typed::<Observable<f64>>("wavelength_nm") {
        println!("  Typed access to wavelength: {} nm", wavelength.get());
    }

    if let Some(mode) = params.get_typed::<Observable<String>>("mode") {
        println!("  Typed access to mode: {}", mode.get());
    }

    Ok(())
}
