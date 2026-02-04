# driver-dover-motion

Safe Rust driver for Dover Motion's SmartStage product range (SmartStage XY, SmartStage Linear, DOF-5) via the MotionSynergyAPI C++ library.

## Features

- **Movable**: Absolute/relative motion, position queries, homing
- **Parameterized**: Observable parameters (position, velocity, acceleration)
- **TriggerOnPosition (TOP)**: Generate GPIO pulses at position intervals
  - Critical for LIBS experiments (synchronized laser triggering)
  - Bidirectional triggering support
  - Configurable pulse width (50ns - 204,800ns, in 50ns increments)

## Feature Flags

- `dover-hardware`: Enable real Dover Motion SDK (requires hardware)
- Default (no features): Use mock driver for testing/development

## Usage

### With Hardware

```toml
[dependencies]
driver-dover-motion = { version = "0.1", features = ["dover-hardware"] }
```

### Mock Mode (Development/Testing)

```toml
[dependencies]
driver-dover-motion = "0.1"
```

## Configuration

```toml
[[devices]]
id = "smartstage_x"
type = "dover_axis"
enabled = true

[devices.config]
device_path = "C:\\ProgramData\\Dover Motion\\SmartStage.xml"
axis_name = "X"
communication_type = "USB"
```

## LIBS Experiment Example

```rust
use driver_dover_motion::DoverAxisDriver;
use common::capabilities::{Movable, TriggerOnPosition};

// Initialize driver
let stage = DoverAxisDriver::new_async(
    "C:\\ProgramData\\Dover Motion\\SmartStage.xml",
    "X",
    "USB"
).await?;

// Configure TOP for 100 µm spacing
stage.enable_trigger_on_position(
    0.0,          // start at 0 mm
    10.0,         // end at 10 mm
    0.1,          // trigger every 100 µm (0.1 mm)
    true,         // bidirectional
    1000,         // 1 µs pulse width
).await?;

// Scan - triggers fire automatically at position intervals
stage.move_abs(10.0).await?;
stage.wait_settled().await?;

// Cleanup
stage.disable_trigger_on_position().await?;
```

## Architecture

This driver uses a multi-layer architecture:

1. `dover-motion-sys`: Low-level FFI bindings (unsafe)
2. `DoverAxisDriver`: Safe wrapper with async interface
3. `DoverMockDriver`: Mock implementation for testing

All blocking FFI calls are wrapped in `tokio::task::spawn_blocking` to avoid blocking the async runtime.

## Platform Support

- **Windows (Primary)**: Full SDK support with MotionSynergyCore.dll
- **Linux (Secondary)**: Supported via libMotionSynergyCore.so

## Documentation

- Dover Motion - Motion Synergy API User Manual (Document 102925)
- Section 6: C++ Software Integration (pp. 121-163)
- Section 6.2.2: IAxisDevice Interface (pp. 140-156)

## License

MIT OR Apache-2.0
