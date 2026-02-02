# examples

Example code demonstrating rust-daq APIs and use cases.

## Overview

This crate contains executable examples and scripts that demonstrate how to use the rust-daq system. Examples range from simple single-device operations to complex multi-device experiments.

Run examples with:

```bash
# Rust examples
cargo run --example <name>

# Rhai script examples (requires scripting support)
./target/release/rhai-runner examples/<name>.rhai
```

## Rust Examples

### Basic Hardware Testing

| Example | Description |
|---------|-------------|
| `scan_hardware.rs` | Enumerate all devices from TOML config |
| `list_params.rs` | List device parameters and capabilities |
| `parameter_generic_access.rs` | Read/write device parameters generically |

### Camera Operations

| Example | Description |
|---------|-------------|
| `exposure_test.rs` | Set exposure and trigger modes |
| `roi_test.rs` | Configure region-of-interest (ROI) |
| `trigger_test.rs` | Test trigger signal behavior |
| `remote_stream_test.rs` | Stream frames over gRPC to remote client |

### Motion Control (ELL14 Rotators)

| Example | Description |
|---------|-------------|
| `elliptec_scanner.rs` | Scan angular positions on ELL14 rotators |
| `test_elliptec.rs` | Basic ELL14 communication test |
| `test_elliptec_robust.rs` | Robust ELL14 test with error handling |

### Motion Control (ESP300)

| Example | Description |
|---------|-------------|
| `test_esp300.rs` | Basic ESP300 motion controller test |
| `test_esp300_robust.rs` | Robust ESP300 test with axis configuration |

### Laser and Power Meter

| Example | Description |
|---------|-------------|
| `test_maitai.rs` | Basic MaiTai laser communication |
| `test_maitai_serial.rs` | MaiTai serial port testing |
| `test_maitai_shutter.rs` | Shutter control and safety testing |
| `eom_power_sweep.rs` | Sweep EOM voltage while reading power |

### Data Acquisition

| Example | Description |
|---------|-------------|
| `ring_buffer_demo.rs` | Basic ring buffer usage |
| `ring_buffer_reader_demo.rs` | Reading from ring buffer |
| `ring_buffer_tap_demo.rs` | Tapping into buffer for monitoring |
| `hdf5_storage_example.rs` | Store measurements in HDF5 files |

### Integration and Server

| Example | Description |
|---------|-------------|
| `grpc_server_demo.rs` | Start daemon and connect gRPC clients |
| `config_validation_demo.rs` | Validate TOML hardware configurations |
| `health_monitor_demo.rs` | Monitor hardware health status |

### Scripting (Rhai)

| Example | Description |
|---------|-------------|
| `scripting_demo.rs` | Embed Rhai scripts in Rust code |
| `scripting_hardware_demo.rs` | Use hardware from Rhai scripts |

## Rhai Script Examples

Rhai is a simple embedded scripting language. Scripts are runnable with `rhai-runner`:

```bash
cargo build --release -p scripting --features scripting_full
./target/release/rhai-runner examples/simple_scan.rhai
```

### Simple Scripts

| Script | Description |
|--------|-------------|
| `simple_scan.rhai` | Basic angle scanning example |
| `focus_scan.rhai` | Autofocus using rotator angles |
| `error_demo.rhai` | Demonstrate error handling in scripts |
| `scripting_demo.rhai` | Basic scripting patterns |

### Experiment Automation

| Script | Description |
|--------|-------------|
| `angular_power_scan.rhai` | Scan angle while measuring power |
| `multi_angle_acquisition.rhai` | Acquire frames at multiple angles |
| `orchestrated_scan.rhai` | Complex multi-axis experiment |
| `triggered_acquisition.rhai` | Synchronize acquisition with triggers |

### Polarization Characterization

| Script | Description |
|--------|-------------|
| `polarization_test.rhai` | Basic polarization measurement |
| `polarization_characterization.rhai` | Full 4D polarization sweep |
| `waveplate_calibration_4d.rhai` | Calibrate waveplates (~3 hours) |
| `waveplate_calibration_4d_test.rhai` | Quick test version (24 points) |

### Polarization Hardware Test

| Example | Description |
|--------|-------------|
| `hw_polarization_test.rs` | Rust version of polarization test |
| `polarization_test.rhai` | Rhai script version |

### Supporting Files

Configuration examples in `examples/configs/`:

| File | Description |
|------|-------------|
| `example_elliptec_scan.yaml` | ELL14 rotator configuration |
| `example_esp300.toml` | ESP300 motion controller setup |
| `example_newport_1830c.toml` | Newport power meter configuration |

Basic Rhai scripts in `examples/scripts/`:

| Script | Description |
|--------|-------------|
| `simple_math.rhai` | Basic arithmetic |
| `loops.rhai` | Loop and iteration patterns |
| `globals_demo.rhai` | Global variable usage |
| `validation_test.rhai` | Parameter validation |

## Running Examples

### All Mock Devices (Local Development)

```bash
# Build with mock hardware
cargo build --examples

# Run Rust example
cargo run --example scan_hardware

# Run Rhai script (requires scripting_full feature)
cargo build --release -p scripting --features scripting_full
./target/release/rhai-runner examples/simple_scan.rhai
```

### Real Hardware (maitai Lab)

First, set up environment:

```bash
source scripts/env-check.sh
bash scripts/build-maitai.sh
```

Then run hardware tests:

```bash
# Rust hardware example
cargo run --release --example test_elliptec

# Rhai with real hardware
./target/release/rhai-runner examples/angular_power_scan.rhai
```

## Common Patterns

### Device Configuration

Most examples load hardware from TOML:

```rust
let config: toml::Value = std::fs::read_to_string("config/demo.toml")?
    .parse()?;

let registry = hardware::initialize_registry(&config)?;
```

For real hardware on maitai, use `config/maitai_hardware.toml` instead. Available config files:
- `config/demo.toml` - Mock devices (local development)
- `config/demo_mock_all.toml` - All mock devices
- `config/maitai_hardware.toml` - Real hardware (maitai lab)
- `config/maitai_no_camera.toml` - Real hardware without camera

### Accessing Devices

```rust
use common::prelude::*;

let movable = registry.get_movable("rotator_2")?;
movable.move_abs(45.0).await?;

let readable = registry.get_readable("power_meter")?;
let value = readable.read_value().await?;
```

### Rhai Device Access

```rhai
let rotator = create_elliptec("/dev/ttyUSB0", "2");
rotator.move_abs(45.0);

let power_meter = create_newport_1830c("/dev/ttyS0");
let power = power_meter.read();
```

## Scripting Features

### With Serial Hardware

```bash
cargo build --release -p scripting --features scripting_full
```

Enables: MaiTai laser, Newport power meter, ELL14 rotators, ESP300

### With Comedi DAQ

```bash
cargo build --release -p scripting --features scripting_full_comedi
```

Adds: Analog I/O, digital I/O, counters

## Testing and CI

Examples are not run in CI by default (they require hardware or lengthy execution). To test examples:

```bash
# Build all examples
cargo build --examples

# Build examples with real hardware
cargo build --release --examples --features maitai
```

## Related Documentation

- [Quick Start Guide](../../DEMO.md) - Start here for new users
- [Rhai Scripting Guide](../../docs/guides/rhai-scripting.md) - Script syntax and API
- [Hardware Configuration](../../docs/guides/hardware-config.md) - TOML device setup
- [Testing Guide](../../docs/guides/testing.md) - Test patterns
