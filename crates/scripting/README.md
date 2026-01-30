# scripting

Embedded scripting engine for rust-daq experiments and automation using Rhai (pure Rust).

## Overview

`scripting` provides a safe, sandboxed scripting environment for controlling hardware devices and orchestrating complex experimental sequences. Scripts are written in Rhai—a dynamically-typed embedded language with zero external dependencies.

### Why Rhai?

- **Pure Rust** - No external Python/Lua interpreter required
- **Fast startup** - Entire engine is ~200KB compiled
- **Type-safe** - Strong integration with Rust type system
- **Zero dependencies** - Self-contained, no external runtime
- **Sandboxed** - Cannot access filesystem or network by default
- **Async-aware** - Supports hardware that's inherently async

## Quick Start

### Basic Rhai Script

```rhai
// Variables and expressions
let wavelength = 800;
let count = 10;

// Looping and conditionals
for i in 0..count {
    print(`Iteration: ${i}`);
    if i % 2 == 0 {
        print("Even!");
    }
}

// Functions
fn double(x) { x * 2 }
print("Doubled: " + double(21));

// Return value (last expression)
wavelength * 1.25
```

### Hardware Control Script

```rhai
// Move a stage to different positions and measure power
stage.move_abs(0.0);
stage.wait_settled();

let power = power_meter.read();
print(`Power at 0mm: ${power} W`);

stage.move_abs(10.0);
stage.wait_settled();

let power = power_meter.read();
print(`Power at 10mm: ${power} W`);
```

## Hardware Bindings

Scripts control hardware through type-safe handles. All operations are async-aware even though scripts use synchronous syntax.

### CameraHandle - Image Acquisition

Wraps devices implementing the `Camera` trait (cameras, detectors, imaging sensors).

```rhai
// Arm for trigger
camera.arm();

// Send trigger pulse
camera.trigger();

// Query resolution
let res = camera.resolution();  // Returns [width, height]
print(`Resolution: ${res[0]}x${res[1]}`);
```

**Methods:**
- `camera.arm()` - Prepare device for trigger
- `camera.trigger()` - Send software trigger
- `camera.resolution()` - Get [width, height] array

**Deprecation Note (v0.7.0):** Direct camera methods are deprecated. Use yield-based plans instead:
```rhai
let result = yield_plan(count(device_id, num_frames));
```

### StageHandle - Motion Control

Wraps devices implementing the `Movable` trait (stages, rotators, actuators, linear actuators).

```rhai
// Move to absolute position
stage.move_abs(10.0);

// Wait for motion to complete
stage.wait_settled();

// Query current position
let pos = stage.position();
print(`Position: ${pos} mm`);

// Move relative to current
stage.move_rel(5.0);

// Home the stage to mechanical zero
stage.home();
```

**Methods:**
- `stage.move_abs(position)` - Move to absolute position
- `stage.move_rel(distance)` - Move relative distance
- `stage.position()` - Get current position
- `stage.wait_settled()` - Wait for motion to complete (15s timeout)
- `stage.home()` - Move to position 0.0

**Safety (Soft Limits):**
Soft limits prevent scripts from commanding hardware to unsafe positions:

```rhai
// Check limits
let limits = stage.get_soft_limits();  // [min, max] or []
print(`Limits: ${limits[0]}–${limits[1]}`);

// Try to move outside limits → Error
stage.move_abs(150.0);  // ERROR if max < 150.0
```

**Deprecation Note (v0.7.0):** Direct stage methods are deprecated. Use yield-based imperative plans:
```rhai
yield_move("stage_id", 10.0);  // Moves stage to 10.0
```

### ReadableHandle - Scalar Measurements

Wraps devices implementing the `Readable` trait (power meters, temperature sensors, voltmeters, detectors).

```rhai
// Single reading
let power = power_meter.read();
print(`Power: ${power} W`);

// Average multiple readings
let avg = power_meter.read_averaged(10);
print(`Averaged: ${avg} W`);
```

**Methods:**
- `readable.read()` - Single measurement
- `readable.read_averaged(samples)` - Average N readings with 50ms between samples

### Newport1830CHandle - Power Meter (Device-Specific)

Extended handle for Newport 1830-C power meter with zeroing capability.

```rhai
// Create power meter on specific port
let pm = create_newport_1830c("/dev/ttyS0");

// Read power
let power = pm.read();

// Average readings
let avg = pm.read_averaged(20);

// Zeroing operations
pm.zero();                        // Zero without attenuator
pm.zero_with_attenuator();        // Zero with attenuator
pm.set_attenuator(true);          // Enable attenuator
pm.set_attenuator(false);         // Disable attenuator
```

**Methods:**
- `pm.read()` - Read power in watts
- `pm.read_averaged(samples)` - Average N readings
- `pm.zero(with_attenuator)` - Zero the measurement
- `pm.zero_with_attenuator()` - Zero with attenuator enabled
- `pm.set_attenuator(enabled)` - Control attenuator

### ShutterHandle - Laser Shutter Control

Wraps devices implementing the `ShutterControl` trait (laser shutters).

```rhai
// Open shutter (beam on)
shutter.open();

// Query state
if shutter.is_open() {
    print("Beam is available");
}

// Close shutter (beam off)
shutter.close();

// Safe execution with automatic closure
with_shutter_open(shutter, || {
    // Beam is open here
    // Shutter automatically closes on exit, even if error occurs
});
```

**Methods:**
- `shutter.open()` - Open the shutter
- `shutter.close()` - Close the shutter
- `shutter.is_open()` - Query shutter state
- `with_shutter_open(shutter, callback)` - Execute callback with shutter open (auto-closes)

**Safety Warning:**
`with_shutter_open()` cannot protect against SIGKILL, power failure, or hardware crashes. For production laser labs, ALWAYS use hardware interlocks in addition to software safety.

### Ell14Handle - ELL14 Rotator (Thorlabs)

Specialized handle for ELL14 rotators with velocity control.

```rhai
// Create rotator on specific port and address
let rotator = create_elliptec("/dev/serial/by-id/usb-FTDI_...", "2");

// Motion control
rotator.move_abs(45.0);     // Move to 45 degrees
rotator.wait_settled();     // Wait for motion

let angle = rotator.position();
print(`Angle: ${angle}°`);

// Velocity control (0-100%)
let vel = rotator.velocity();           // Get cached velocity
rotator.set_velocity(100);              // Set to max speed
rotator.refresh_settings();             // Update cache from hardware
let hw_vel = rotator.get_velocity();    // Query hardware
```

**Methods:**
- `rotator.move_abs(degrees)` - Move to absolute angle (0-360°)
- `rotator.position()` - Get current angle
- `rotator.wait_settled()` - Wait for motion to complete
- `rotator.home()` - Home to 0 degrees
- `rotator.velocity()` - Get cached velocity (%)
- `rotator.set_velocity(percent)` - Set velocity (0-100%)
- `rotator.get_velocity()` - Query velocity from hardware
- `rotator.refresh_settings()` - Update cache from hardware

**Performance Note:**
During initialization via `create_elliptec()`, velocity is automatically set to maximum (100%) for fastest scans. Use `set_velocity()` to reduce speed if precise positioning is needed.

### ComediHandle - NI DAQ Analog I/O

Handles for NI PCI-MIO-16XE-10 DAQ card via Comedi framework.

**Analog Input:**
```rhai
let ai = create_comedi_input("/dev/comedi0");

let voltage = ai.read(0);           // Read channel 0
let all = ai.read_all();            // Read all channels [array]
let range = ai.range(0);            // Get voltage range [min, max]
```

**Analog Output:**
```rhai
let ao = create_comedi_output("/dev/comedi0");

ao.write(0, 2.5);      // Set DAC0 to 2.5V
ao.zero_all();         // Zero all outputs
```

**Digital I/O:**
```rhai
let dio = create_comedi_dio("/dev/comedi0");

let state = dio.read(0);           // Read pin 0
dio.write(1, true);                // Set pin 1 high
dio.set_direction(0, false);       // Configure pin 0 as input
```

**Counter:**
```rhai
let counter = create_comedi_counter("/dev/comedi0");

let count = counter.read(0);       // Read counter 0
counter.reset(0);                  // Reset counter 0
counter.arm(0);                    // Arm counter for trigger
```

## Factory Functions - Creating Drivers

### Mock Devices (Testing)

```rhai
// Create mock stage for testing
let stage = create_mock_stage();

// Create mock stage with soft limits
let stage_limited = create_mock_stage_limited(0.0, 100.0);

// Create mock camera
let camera = create_mock_camera(1920, 1080);

// Create mock power meter
let meter = create_mock_power_meter(1.0);  // Base power in watts
```

### Real Hardware (Feature-Gated)

These functions are only available when compiled with the `hardware_factories` feature.

**ELL14 Rotator:**
```rhai
// Create rotator with auto-calibration
let rotator = create_elliptec("/dev/serial/by-id/...", "2");

// Features:
// - Validates device is responding (3s timeout)
// - Sets velocity to maximum (100%) automatically
// - Falls back to default calibration on timeout
```

**Newport 1830-C Power Meter:**
```rhai
// Create power meter
let pm = create_newport_1830c("/dev/ttyS0");

// Validates device identity before returning
```

**MaiTai Laser:**
```rhai
// Create with default baud (115200)
let laser = create_maitai("/dev/serial/by-id/usb-Silicon_Labs_CP2102...");

// Or with custom baud rate
let laser = create_maitai_with_baud("/dev/ttyUSB0", 9600);
```

## Data Storage - HDF5 (Feature-Gated)

Write experimental data to HDF5 files with this feature.

```rhai
// Create HDF5 file
let hdf5 = create_hdf5("experiment_data.h5");

// Write attributes
hdf5.write_attr("experiment", "polarization_scan");
hdf5.write_attr_f64("wavelength", 800.0);
hdf5.write_attr_i64("num_points", 100);

// Write datasets
let power_data = [1.0, 1.5, 2.0, 1.8, 1.2];
hdf5.write_array_1d("power_trace", power_data);

// Write 2D arrays
let scan_data = [
    [0.0, 1.0],
    [45.0, 1.5],
    [90.0, 2.0]
];
hdf5.write_array_2d("angle_vs_power", scan_data);

// Create groups for nested structure
hdf5.create_group("metadata");

// Close and flush to disk
hdf5.close();
```

**Methods:**
- `create_hdf5(path)` - Create new file
- `hdf5.write_attr(name, value)` - Write string attribute
- `hdf5.write_attr_f64(name, value)` - Write float attribute
- `hdf5.write_attr_i64(name, value)` - Write int attribute
- `hdf5.write_array_1d(name, data)` - Write 1D array
- `hdf5.write_array_2d(name, data)` - Write 2D array
- `hdf5.create_group(name)` - Create HDF5 group
- `hdf5.path()` - Get file path
- `hdf5.close()` - Close file and flush

## Utility Functions

### Sleep

```rhai
// Sleep for 0.5 seconds
sleep(0.5);

// Loop with delay
for i in 0..10 {
    print(`Iteration: ${i}`);
    sleep(0.1);
}
```

Automatically uses `tokio::time::sleep` when running in async context, falls back to blocking sleep if needed.

### Timestamps

```rhai
// YYYYMMDD_HHMMSS format (for filenames)
let ts = timestamp();              // "20250125_143027"
print("File: data_" + ts + ".h5");

// ISO8601 format with timezone
let iso = timestamp_iso();         // "2025-01-25T14:30:27.123456Z"
```

## Script Execution Models

### Basic Synchronous Scripts

For simple experiments with direct hardware commands:

```rust
use daq_scripting::{RhaiEngine, ScriptEngine};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = RhaiEngine::with_hardware()?;

    let script = r#"
        stage.move_abs(10.0);
        stage.wait_settled();
        let power = power_meter.read();
        print("Power: " + power + " W");
    "#;

    engine.execute_script(script).await?;
    Ok(())
}
```

### Yield-Based Plans (Recommended for v0.7+)

For complex experiments with proper Document emission and data collection:

```rust
use daq_scripting::{RhaiEngine, YieldChannelBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = RhaiEngine::with_yield_support()?;

    // Set up yield channels for plan execution
    let (handle, rx, tx) = YieldChannelBuilder::new().build();
    engine.set_yield_handle(handle)?;

    let script = r#"
        // Yield plans for proper data collection
        let result = yield_plan(line_scan("x", 0, 10, 11, "det"));

        // Access result data
        if result.data["det"] > threshold {
            yield_plan(high_res_scan(...));
        }

        result.data["det"]
    "#;

    let result = engine.execute_script(script).await?;
    Ok(())
}
```

### With RunEngine Integration

For plans that need to queue operations on a shared device registry:

```rust
use daq_scripting::{RhaiEngine, plan_bindings::RunEngineHandle};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = RhaiEngine::with_hardware()?;

    // Create RunEngineHandle from device registry
    let registry = Arc::new(/* ... */);
    let run_engine = RunEngineHandle::new(registry);
    engine.set_run_engine(run_engine)?;

    let script = r#"
        // Queue plans
        let plan = line_scan("stage_x", 0, 10, 11, "detector");
        run_engine.queue(plan);
        run_engine.start();
    "#;

    engine.execute_script(script).await?;
    Ok(())
}
```

## Safety & Limits

### Operation Limit

Scripts are limited to prevent infinite loops from hanging the application:

- **Default limit:** 10,000 operations
- **With hardware:** 10,000 operations
- **Yield support:** 100,000 operations
- **Custom limit:** Use `with_hardware_and_limit()`

```rust
use daq_scripting::RhaiEngine;

// For long experiments, increase the limit
let mut engine = RhaiEngine::with_hardware_and_limit(1_000_000)?;
```

Exceeding the limit raises a `RuntimeError`:
```
Safety limit exceeded: maximum 10,000 operations
```

### Soft Limits (Positional Safety)

Stage soft limits prevent scripts from commanding hardware to unsafe positions:

```rhai
let stage = create_mock_stage_limited(0.0, 100.0);

// Valid move - succeeds
stage.move_abs(50.0);

// Invalid move - raises error
stage.move_abs(150.0);
// ERROR: Position 150 exceeds soft limit maximum 100
```

### Timeout Protection

Long operations have internal timeouts to prevent hanging:

- `stage.position()` - 3s timeout
- `stage.wait_settled()` - 15s timeout
- `create_elliptec()` calibration - 3s timeout

### Error Handling

Scripts can handle errors gracefully:

```rhai
// Try-catch equivalent (via closures and callbacks)
let result = try {
    stage.move_abs(10.0);
    "success"
} catch(e) {
    print("Error: " + e);
    "failed"
};
```

## Script Examples

### Polarization Scan

```rhai
// Sweep rotation mount and measure power at each angle
let hdf5 = create_hdf5("polarization_scan.h5");
hdf5.write_attr("experiment", "polarization_characterization");

let angles = [];
let powers = [];

for angle in range(0, 360, 15) {
    rotator.move_abs(angle);
    rotator.wait_settled();

    let power = power_meter.read();
    angles.push(angle);
    powers.push(power);

    print(`Angle ${angle}°: ${power}W`);
}

hdf5.write_array_1d("angles", angles);
hdf5.write_array_1d("powers", powers);
hdf5.close();
```

### Position-Dependent Measurement

```rhai
// Move stage and take measurements at each position
let positions = [];
let measurements = [];

for pos in range(0.0, 100.0, 5.0) {
    stage.move_abs(pos);
    stage.wait_settled();

    // Take multiple readings and average
    let reading = power_meter.read_averaged(5);
    positions.push(pos);
    measurements.push(reading);

    sleep(0.1);
}

// Save data
let hdf5 = create_hdf5("scan_results.h5");
hdf5.write_array_2d("data", [positions, measurements]);
hdf5.close();
```

### Wavelength Sweep with Laser Safety

```rhai
// Sweep wavelength with shutter control
let laser = create_maitai("/dev/serial/by-id/...");

with_shutter_open(laser, || {
    for wl in range(700, 1000, 10) {
        laser.set_wavelength(wl);

        // Wait for laser to stabilize
        sleep(0.5);

        let power = power_meter.read();
        print(`${wl}nm: ${power}W`);
    }
});

// Shutter automatically closed here
```

### Multi-Device Coordination

```rhai
// Coordinate stage, camera, and laser
stage.move_abs(0.0);
stage.wait_settled();

camera.arm();
with_shutter_open(laser, || {
    for i in 0..10 {
        camera.trigger();
        sleep(0.1);

        stage.move_rel(1.0);
        stage.wait_settled();
    }
});
```

## Advanced Topics

### Setting Global Variables

Pass data from Rust into scripts:

```rust
use daq_scripting::{RhaiEngine, ScriptValue, ScriptEngine};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = RhaiEngine::with_hardware()?;

    // Set variables before executing
    engine.set_global("wavelength", ScriptValue::new(800_i64))?;
    engine.set_global("num_scans", ScriptValue::new(10_i64))?;

    let script = r#"
        print(`Wavelength: ${wavelength}nm`);
        for i in 0..num_scans {
            // Use variables in script
        }
    "#;

    engine.execute_script(script).await?;
    Ok(())
}
```

### Retrieving Results

Return values from scripts:

```rust
use daq_scripting::{RhaiEngine, ScriptEngine};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = RhaiEngine::with_hardware()?;

    let script = r#"
        let power_data = [];
        for i in 0..10 {
            let p = power_meter.read();
            power_data.push(p);
        }
        power_data  // Return the array
    "#;

    let result = engine.execute_script(script).await?;
    // Result is a Rhai Array
    Ok(())
}
```

### Custom Operations Limits

For complex experiments that need more operations:

```rust
use daq_scripting::RhaiEngine;

// Polarization scan: ~1M operations for 360 angles
let mut engine = RhaiEngine::with_hardware_and_limit(1_000_000)?;

// Very long experiment
let mut engine = RhaiEngine::with_hardware_and_limit(10_000_000)?;
```

## Feature Flags

| Flag | Contents |
|------|----------|
| `scripting_full` | ELL14 rotator support |
| `hardware_factories` | Factory functions for real hardware (ELL14, Newport, MaiTai) |
| `hdf5_scripting` | HDF5 file writing from scripts |
| `python` | Python/PyO3 interop (separate engine) |

## Error Messages

Common errors and how to fix them:

### "Soft limit violation: Position X below soft limit minimum Y"
- **Cause:** Script tried to move stage below minimum safe position
- **Fix:** Check stage.get_soft_limits() and respect the bounds

### "Safety limit exceeded: maximum N operations"
- **Cause:** Script exceeded operation count limit (infinite loop?)
- **Fix:** Increase limit with `with_hardware_and_limit()` or optimize script

### "Tokio current-thread runtime cannot run blocking hardware calls"
- **Cause:** Engine running on wrong Tokio runtime flavor
- **Fix:** Use `#[tokio::main(flavor = "multi_thread")]` not current-thread

### "Device not responding" or "position query timed out"
- **Cause:** Hardware is not connected or not responding
- **Fix:** Check device connection, verify port path with `ls /dev/serial/by-id/`

## Async Bridge

Hardware drivers are inherently async (serial I/O, network, etc.). The scripting engine bridges this to synchronous Rhai scripts using `tokio::task::block_in_place()`.

```
Rhai Script (sync)
       ↓
run_blocking() helper
       ↓
tokio::task::block_in_place()  ← Yields to runtime
       ↓
Hardware Driver (async)
```

This ensures:
- Scripts have simple synchronous syntax
- Hardware I/O doesn't block the async runtime
- Runtime can process other tasks while hardware I/O is in flight

## Module Structure

- `bindings.rs` - Core hardware bindings (Stage, Camera, Readable, Shutter, ELL14)
- `comedi_bindings.rs` - NI DAQ/Comedi hardware bindings
- `yield_bindings.rs` - Yield-based plan scripting (v0.7+)
- `plan_bindings.rs` - Declarative plan definitions
- `rhai_engine.rs` - RhaiEngine implementation
- `engine.rs` - ScriptEngine trait definition
- `shutter_safety.rs` - Shutter emergency shutdown registry
- `script_runner.rs` - Plan-based script execution

## Related Documentation

- [hardware README](../hardware/README.md) - Hardware abstraction and driver patterns
- [experiment README](../experiment/README.md) - Plan definitions and experiment execution
- [docs/guides/scripting.md](../../docs/guides/scripting.md) - Scripting guide with examples
- [Architecture: Script Execution Model](../../docs/architecture/adr-script-execution.md)

## Contributing

To add new hardware bindings:

1. Create a new handle type (e.g., `MyDeviceHandle`)
2. Implement methods using `engine.register_fn()`
3. Use `run_blocking()` for async operations
4. Document in module comments and this README

See `bindings.rs` for complete patterns.
