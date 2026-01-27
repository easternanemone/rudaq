# Rhai Scripting Guide

This guide covers the Rhai scripting system for experiment automation in rust-daq.

## Overview

The `daq-scripting` crate provides a Rhai-based scripting engine for writing
experiment scripts that control hardware devices. Rhai is an embedded scripting
language for Rust that provides:

- Safe execution with operation limits
- Synchronous script syntax with async hardware underneath
- Type-safe hardware handles
- Automatic shutter safety via `with_shutter_open()`

## Quick Start

### Building Script Runners

Scripts are executed via binary runners. Build with the `hardware_factories` feature:

```bash
# Build all script runners
cargo build --release -p daq-scripting --features hardware_factories

# Available binaries:
# - run_polarization      - Polarization element characterization
# - run_waveplate_cal     - Full 4D waveplate calibration (2-3 hours)
# - run_waveplate_cal_test - Quick test version (24 points)
```

**IMPORTANT**: You must use `--features hardware_factories` to enable the
hardware factory functions (`create_maitai`, `create_newport_1830c`, etc.).
The `scripting_full` feature alone is NOT sufficient due to feature aliasing.

### Running a Script

```bash
cd ~/rust-daq
./target/release/run_waveplate_cal_test
```

## Available Functions

### Device Factory Functions

| Function | Returns | Description |
|----------|---------|-------------|
| `create_maitai(port)` | `Shutter` | MaiTai laser (shutter only) |
| `create_maitai_tunable(port)` | `MaiTaiLaser` | MaiTai with wavelength control |
| `create_newport_1830c(port)` | `Newport1830C` | Power meter |
| `create_elliptec(port, addr)` | `Ell14` | ELL14 rotator |
| `create_comedi(device)` | `ComediDAQ` | Comedi DAQ (requires `comedi_scripting` feature) |
| `create_hdf5(path)` | `Hdf5File` | HDF5 file for data storage |

### MaiTai Laser Methods (MaiTaiLaser type)

```rhai
let laser = create_maitai_tunable("/dev/serial/by-id/...");

// Shutter control
laser.open();              // Open shutter
laser.close();             // Close shutter  
laser.is_open();           // Query shutter state

// Wavelength control
laser.set_wavelength(800.0);  // Set to 800nm
let wl = laser.get_wavelength();  // Query wavelength

// For use with with_shutter_open()
let shutter = laser.as_shutter();
```

### Newport 1830-C Power Meter Methods

```rhai
let pm = create_newport_1830c("/dev/ttyS0");

// Power reading
let power = pm.read();              // Read power (Watts)
let avg = pm.read_averaged(10);     // Average 10 readings

// Calibration
pm.zero();                          // Zero without attenuator
pm.zero_with_attenuator();          // Zero with attenuator
pm.set_attenuator(true);            // Enable/disable attenuator

// Wavelength calibration (NEW)
pm.set_wavelength(800.0);           // Set calibration wavelength
let wl = pm.get_wavelength();       // Query calibration wavelength
```

### ELL14 Rotator Methods

```rhai
let rotator = create_elliptec("/dev/serial/by-id/...", "2");

// Motion
rotator.move_abs(45.0);     // Move to 45 degrees
rotator.move_rel(10.0);     // Move 10 degrees relative
rotator.home();             // Home the rotator
rotator.wait_settled();     // Wait for motion complete

// Query
let pos = rotator.position();   // Current position
let vel = rotator.velocity();   // Cached velocity (0-100%)
```

### Comedi DAQ Methods (requires `comedi_scripting` feature)

```rhai
let daq = create_comedi("/dev/comedi0");

// Device info
print("Board: " + daq.board_name());
print("Driver: " + daq.driver_name());
print("AI channels: " + daq.n_ai_channels());
print("AO channels: " + daq.n_ao_channels());
print("DIO channels: " + daq.n_dio_channels());

// Analog input
let voltage = daq.read_voltage(0);              // Read AI channel 0
let v = daq.read_voltage_range(0, 1);           // Read with specific range

// Analog output (EOM control)
daq.write_voltage(1, 2.5);                      // Write 2.5V to AO channel 1
// WARNING: AO channel 0 controls EOM - be careful!

// Digital I/O
daq.set_dio(0, true);                           // Set DIO channel 0 high
let state = daq.get_dio(1);                     // Read DIO channel 1
```

**Building with Comedi support:**
```bash
cargo build --release -p daq-scripting --features scripting_full_comedi
```

### HDF5 Data Storage

```rhai
let hdf5 = create_hdf5("output.h5");

// Attributes
hdf5.write_attr("name", "value");           // String attribute
hdf5.write_attr_f64("wavelength", 800.0);   // Float attribute
hdf5.write_attr_i64("samples", 100);        // Integer attribute

// Arrays
hdf5.write_array_1d("angles", [0.0, 10.0, 20.0]);
hdf5.write_array_2d("data", [[0.0, 1.0], [2.0, 3.0]]);

// Groups
hdf5.create_group("wavelength_800");

// Close (required!)
hdf5.close();
```

### Utility Functions

```rhai
sleep(1.0);                 // Sleep for 1 second
let ts = timestamp();       // "20260127_115319" format
let iso = timestamp_iso();  // Full ISO8601 timestamp
```

## Shutter Safety

**CRITICAL**: Always use `with_shutter_open()` for laser experiments. This
guarantees shutter closure even if the script errors or is interrupted.

```rhai
let laser = create_maitai_tunable("/dev/serial/by-id/...");
let shutter = laser.as_shutter();

let result = with_shutter_open(shutter, || {
    // Shutter is open here
    let power = power_meter.read();
    
    // Even if this errors, shutter will close
    do_measurement();
    
    // Return value from closure
    power
});

// Shutter is guaranteed closed here
print("Result: " + result);
```

## Example: Multi-Dimensional Sweep

```rhai
// 4D Waveplate Calibration Example
let ELLIPTEC_PORT = "/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0";
let NEWPORT_PORT = "/dev/ttyS0";
let MAITAI_PORT = "/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0";

// Initialize hardware
let power_meter = create_newport_1830c(NEWPORT_PORT);
let laser = create_maitai_tunable(MAITAI_PORT);
let rotator_lp = create_elliptec(ELLIPTEC_PORT, "3");
let rotator_hwp = create_elliptec(ELLIPTEC_PORT, "2");

// Create output file
let hdf5 = create_hdf5("calibration_" + timestamp() + ".h5");

// Run with shutter safety
let shutter = laser.as_shutter();
let data = with_shutter_open(shutter, || {
    let results = [];
    
    for wavelength in [780.0, 800.0, 820.0] {
        // Set wavelengths
        laser.set_wavelength(wavelength);
        power_meter.set_wavelength(wavelength);
        sleep(60.0);  // Stabilization
        
        for lp_angle in [0.0, 45.0, 90.0] {
            rotator_lp.move_abs(lp_angle);
            rotator_lp.wait_settled();
            
            let power = power_meter.read_averaged(3);
            results.push([wavelength, lp_angle, power]);
        }
    }
    
    results
});

// Save data
hdf5.write_array_2d("measurements", data);
hdf5.close();
```

## Serial Port Paths

Always use stable `/dev/serial/by-id/` paths that don't change on reboot:

| Device | Port Path |
|--------|-----------|
| MaiTai Laser | `/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0` |
| ELL14 Rotators | `/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0` |
| Newport 1830-C | `/dev/ttyS0` (built-in RS-232, always stable) |

## Creating New Script Runners

To create a runner for a new script:

1. Create the Rhai script in `crates/daq-examples/examples/`:

```rhai
// my_experiment.rhai
print("Starting experiment...");
// ... script content
```

2. Create a binary in `crates/daq-scripting/src/bin/`:

```rust
// run_my_experiment.rs
use daq_scripting::traits::ScriptEngine;
use daq_scripting::RhaiEngine;
use tracing_subscriber::EnvFilter;

const SCRIPT: &str = include_str!("../../../daq-examples/examples/my_experiment.rhai");
const MAX_OPERATIONS: u64 = 1_000_000;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let mut engine = RhaiEngine::with_hardware_and_limit(MAX_OPERATIONS)
        .expect("Failed to create RhaiEngine");

    match engine.execute_script(SCRIPT).await {
        Ok(result) => println!("Success: {:?}", result),
        Err(e) => {
            eprintln!("Error: {:?}", e);
            std::process::exit(1);
        }
    }
}
```

3. Build with hardware features:

```bash
cargo build --release -p daq-scripting --features hardware_factories --bin run_my_experiment
```

## Troubleshooting

### "Function not found: create_newport_1830c"

Build with the correct feature flag:
```bash
cargo build --features hardware_factories  # Correct
cargo build --features scripting_full      # WRONG - doesn't enable factories
```

### Script hits operation limit

Increase `MAX_OPERATIONS` in the binary runner:
```rust
const MAX_OPERATIONS: u64 = 10_000_000;  // For long experiments
```

### Shutter not closing on error

Ensure you're using `with_shutter_open()`:
```rhai
// WRONG - shutter may stay open on error
laser.open();
do_risky_operation();
laser.close();

// CORRECT - shutter always closes
with_shutter_open(shutter, || {
    do_risky_operation();
});
```

## Feature Flags

| Feature | Description |
|---------|-------------|
| `hardware_factories` | Marker feature for cfg guards (enabled by scripting_full) |
| `hdf5_scripting` | HDF5 data storage |
| `comedi_scripting` | Comedi DAQ support (requires comedilib on Linux) |
| `scripting_full` | **Recommended** - All serial hardware + HDF5 |
| `scripting_full_comedi` | scripting_full + Comedi DAQ support |

## Related Documentation

- [Comedi Setup](comedi-setup.md) - NI DAQ card configuration
- [Testing Guide](testing.md) - Running hardware tests
- [EOM Power Sweep](eom-power-sweep.md) - Power sweep experiments
