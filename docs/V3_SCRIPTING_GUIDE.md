# V3 Instrument Scripting with Rhai

This guide describes how to use the Rhai scripting backend with V3 instruments in rust-daq.

## Overview

The Rhai scripting backend (implemented in bd-ya3l/P4.5) provides a lightweight, embedded scripting language for controlling V3 instruments. It bridges async Rust instrument operations with synchronous Rhai scripts.

## Features

- **V3 Instrument Support**: Direct access to V3 meta traits (Camera, PowerMeter, Stage, Laser)
- **Async Bridge**: Transparent handling of async operations from synchronous scripts
- **Type Safe**: Strong typing with runtime validation
- **Fast**: Compiled to bytecode for efficient execution
- **Safety Limits**: Built-in protection against infinite loops

## Supported Instrument Types

### Camera (V3CameraHandle)
```rhai
// Configure camera
camera.set_exposure(100.0);  // milliseconds
camera.set_binning(2, 2);    // 2x2 binning
camera.set_roi(0, 0, 512, 512);  // x, y, width, height

// Acquisition
camera.start_acquisition();
sleep(1.0);
camera.stop_acquisition();

// Triggered mode
camera.arm_trigger();
camera.trigger();

// Query state
let id = camera.id();
let state = camera.state();
let roi = camera.roi();  // returns [x, y, width, height]
```

### PowerMeter (V3PowerMeterHandle)
```rhai
// Configure power meter
power_meter.set_wavelength(800.0);  // nm
power_meter.set_range(0.001);        // watts
power_meter.zero();

// Query state
let id = power_meter.id();
let state = power_meter.state();
```

### Stage (V3StageHandle)
```rhai
// Motion control
stage.move_absolute(10.5);  // mm
stage.move_relative(2.0);   // mm
stage.wait_settled(5.0);    // timeout in seconds

// Query state
let pos = stage.position();
let moving = stage.is_moving();

// Homing and velocity
stage.home();
stage.set_velocity(10.0);  // mm/s
stage.stop_motion();
```

### Laser (V3LaserHandle)
```rhai
// Wavelength control
laser.set_wavelength(800.0);  // nm
let wl = laser.wavelength();

// Power control
laser.set_power(2.5);  // watts
let pwr = laser.power();

// Shutter control
laser.shutter_open();
sleep(1.0);
laser.shutter_close();
```

## Common Methods

All V3 instrument handles support these methods:

```rhai
let id = instrument.id();          // Get instrument ID
let state = instrument.state();    // Get current state
instrument.initialize();           // Initialize hardware
```

## Utility Functions

```rhai
sleep(0.5);  // Sleep for 0.5 seconds
```

## Example: Camera Acquisition Workflow

```rhai
// Configure camera
camera.set_exposure(100.0);
camera.set_binning(1, 1);
camera.set_roi(0, 0, 1024, 1024);

// Acquire 10 frames
camera.start_acquisition();

for i in 0..10 {
    print("Frame " + (i + 1) + "/10");
    sleep(0.15);  // exposure + overhead
}

camera.stop_acquisition();
print("Acquisition complete");
```

## Example: Stage Scan

```rhai
// Configure stage
stage.set_velocity(5.0);
stage.home();
stage.wait_settled(10.0);

// Scan from 0 to 10 mm in 1 mm steps
let pos = 0.0;
while pos <= 10.0 {
    stage.move_absolute(pos);
    stage.wait_settled(5.0);

    print("Position: " + stage.position() + " mm");

    // Acquire data here
    sleep(0.2);

    pos += 1.0;
}
```

## Using from Rust

```rust
use rhai::{Engine, Scope};
use rust_daq::scripting::{register_v3_hardware, V3CameraHandle};
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create Rhai engine
    let mut engine = Engine::new();
    register_v3_hardware(&mut engine);

    // Create V3 instrument (example)
    let camera = MockCameraV3::new("camera1");
    camera.initialize().await?;

    // Wrap in V3 handle
    let camera_handle = V3CameraHandle {
        instrument: Arc::new(Mutex::new(camera)),
    };

    // Create scope and add instrument
    let mut scope = Scope::new();
    scope.push("camera", camera_handle);

    // Execute script
    let script = r#"
        camera.set_exposure(50.0);
        camera.start_acquisition();
        sleep(1.0);
        camera.stop_acquisition();
    "#;

    engine.run_with_scope(&mut scope, script)?;
    Ok(())
}
```

## Safety and Error Handling

- **Operation Limits**: Scripts are limited to 10,000 operations by default
- **Error Propagation**: Rust errors are converted to Rhai runtime errors
- **Thread Safety**: All instrument access is protected by Arc<Mutex<>>

## Implementation Details

- **Async→Sync Bridge**: Uses `tokio::task::block_in_place()` for safe async execution
- **Thread Safe**: Arc<Mutex<>> ensures concurrent access safety
- **Zero Copy**: Data broadcast via V3 data channels (not through scripts)

## Related Documentation

- `/Users/briansquires/code/rust-daq/src/scripting/script_engine.rs` - ScriptEngine trait
- `/Users/briansquires/code/rust-daq/src/scripting/rhai_engine.rs` - Rhai implementation
- `/Users/briansquires/code/rust-daq/src/scripting/bindings_v3.rs` - V3 bindings
- `/Users/briansquires/code/rust-daq/src/core_v3.rs` - V3 core traits

## Issue References

- bd-ya3l (P4.5): Implement Alternative Scripting Backend (Rhai/Lua)
- Jules-10: ScriptEngine trait foundation
- Jules-14: Rhai/Lua backend implementation
