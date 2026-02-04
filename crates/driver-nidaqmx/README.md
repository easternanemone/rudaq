# driver-nidaqmx

NI-DAQmx driver for the rust-daq ecosystem using PyO3 bridge to Python's `nidaqmx` package.

## Overview

This driver provides Rust integration with National Instruments DAQ hardware by bridging to the mature Python `nidaqmx` library via PyO3. This approach avoids the complexity of directly interfacing with the massive NI-DAQmx C API while leveraging a battle-tested Python wrapper.

## Architecture

```
┌─────────────────────────────────────┐
│   Rust Application                  │
│   (uses Triggerable trait)          │
└─────────────────┬───────────────────┘
                  │
┌─────────────────▼───────────────────┐
│   NiDaqTrigger (Rust)               │
│   - Implements Triggerable          │
│   - Holds Python GIL references     │
└─────────────────┬───────────────────┘
                  │ PyO3
┌─────────────────▼───────────────────┐
│   nidaqmx (Python package)          │
│   - Task management                 │
│   - Channel configuration           │
└─────────────────┬───────────────────┘
                  │ ctypes/CFFI
┌─────────────────▼───────────────────┐
│   NI-DAQmx C API                    │
│   - Hardware drivers                │
└─────────────────────────────────────┘
```

## Features

- **Mock Mode (default)**: Works without hardware for development/testing
- **Hardware Mode**: Requires Python 3.x with `nidaqmx` package installed

## Prerequisites

### For Hardware Mode

1. **Python 3.x** with `nidaqmx` package:
   ```bash
   pip install nidaqmx
   ```

2. **NI-DAQmx Runtime** (system drivers):
   - Download from [NI website](https://www.ni.com/en-us/support/downloads/drivers/download.ni-daqmx.html)
   - Typical path: `/usr/local/natinst` (Linux) or `C:\Program Files\National Instruments` (Windows)

### For Mock Mode

No additional dependencies required. The driver will compile but hardware operations will fail gracefully.

## Usage

### Basic Digital Trigger

```rust
use driver_nidaqmx::{NiDaqTrigger, TriggerMode, Triggerable};

// Create trigger
let trigger = NiDaqTrigger::new(
    TriggerMode::Digital,
    0.1,   // high_time (seconds)
    0.001, // low_time (seconds)
    1,     // samps_per_chan
).await?;

// Use with Triggerable trait
trigger.arm().await?;
trigger.trigger().await?;
trigger.disarm().await?;
```

### Trigger On Position (External Edge Trigger)

```rust
use driver_nidaqmx::{NiDaqTrigger, TriggerMode};

let trigger = NiDaqTrigger::new(
    TriggerMode::TriggerOnPosition {
        trigger_source: "/Dev1/PFI0".to_string(),
        rising_edge: true,
        retriggerable: false,
    },
    0.001, // high_time
    0.001, // low_time
    1,     // samps_per_chan
).await?;
```

### TOML Configuration

```toml
[[devices]]
id = "camera_trigger"
type = "nidaqmx_trigger"
enabled = true

[devices.config]
high_time = 0.1
low_time = 0.001
samps_per_chan = 1
trigger_mode = "digital"  # or "trigger_on_position"
device_name = "Dev1"      # optional, defaults to "Dev1"
counter = "ctr0"          # optional, defaults to "ctr0"

# For trigger_on_position mode:
# trigger_source = "/Dev1/PFI0"
# rising_edge = true
# retriggerable = false
```

## Ported from LIBS

This driver ports the `digital` and `TOP` (Trigger On Position) classes from the legacy LIBS Python codebase (`LIBS/trigger.py`):

| Python Class | Rust Equivalent | Description |
|--------------|-----------------|-------------|
| `digital` | `TriggerMode::Digital` | Simple software-triggered pulse generation |
| `TOP` | `TriggerMode::TriggerOnPosition` | External digital edge-triggered pulse |

## Testing

```bash
# Run tests (mock mode)
cargo test -p driver-nidaqmx

# Run with hardware (requires NI-DAQmx and Python nidaqmx)
cargo test -p driver-nidaqmx --features hardware
```

## Migration to Linux

When migrating to Linux, this driver should be replaced with a native Comedi-based implementation:

- Use the existing `driver-comedi` crate
- Comedi supports counter/timer operations for pulse generation
- Avoids Python dependency for production deployments

This PyO3 bridge is a pragmatic interim solution for accelerating LIBS integration while hardware is Windows-based.

## Safety Notes

- **Thread Safety**: The driver uses `Arc<Mutex<>>` for thread-safe access to Python objects
- **GIL Management**: All Python calls are wrapped in `Python::with_gil()` and executed in `spawn_blocking` tasks
- **Async-Safe**: No blocking operations in async context - all Python interactions are offloaded to blocking threads

## License

MIT OR Apache-2.0
