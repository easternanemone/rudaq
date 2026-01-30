# ESP300 Native Plugin

A native Rust plugin demonstrating how to create dynamically loadable modules for rust-daq using `abi_stable`.

## Overview

This example plugin provides a driver for the Newport ESP300 Universal Motion Controller. It demonstrates:

- **FFI-safe plugin architecture** using `abi_stable`
- **Full module lifecycle**: configure, stage, start, pause, resume, stop, unstage
- **Movable capability**: move_abs, move_rel, position, wait_settled, stop
- **State serialization** for hot-reload support during development
- **Event and data emission** for host integration

## Building

### Prerequisites

- Rust 1.70+ (for `abi_stable` compatibility)
- Target platform: Linux, macOS, or Windows

### Build Commands

```bash
# From the plugin directory
cd examples/plugins/esp300-native

# Debug build
cargo build

# Release build (recommended for deployment)
cargo build --release

# Build with mock mode for testing without hardware
cargo build --features mock
```

### Output Locations

| Platform | Library Path |
|----------|--------------|
| Linux    | `target/release/libesp300_native.so` |
| macOS    | `target/release/libesp300_native.dylib` |
| Windows  | `target/release/esp300_native.dll` |

## Installation

Copy the built library and `plugin.toml` to your plugin directory:

```bash
# Create plugin directory
mkdir -p ~/.rust-daq/plugins/esp300-native

# Copy files
cp target/release/libesp300_native.* ~/.rust-daq/plugins/esp300-native/
cp plugin.toml ~/.rust-daq/plugins/esp300-native/
```

## Configuration

### Plugin Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `port_path` | string | `/dev/ttyUSB0` | Serial port path |
| `axis` | int | `1` | ESP300 axis number (1-3) |
| `velocity` | float | `10.0` | Motion velocity (mm/s) |
| `acceleration` | float | `50.0` | Motion acceleration (mm/s²) |

### Example Configuration

```toml
# In your experiment config
[[modules]]
type = "esp300_stage"
instance_id = "x_stage"

[modules.params]
port_path = "/dev/ttyUSB0"
axis = 1
velocity = 20.0
acceleration = 100.0
```

## Usage

### Module Lifecycle

```
Created → Configured → Staged → Running ↔ Paused
                          ↓
                       Stopped
```

1. **configure()**: Set parameters (port, axis, velocity)
2. **stage()**: Open serial port, initialize communication
3. **start()**: Enable motor amplifier
4. **pause()/resume()**: Suspend/resume operation
5. **stop()**: Halt motion, disable motor
6. **unstage()**: Close serial port, clean up

### Motion Commands

Once the module is running, use the Movable capability:

```rust
// Move to absolute position
module.move_abs(10.5)?;  // Move to 10.5 mm
module.wait_settled()?;   // Wait for motion to complete

// Move relative
module.move_rel(-2.0)?;   // Move -2.0 mm from current position
module.wait_settled()?;

// Query position
let pos = module.position()?;
println!("Current position: {:.3} mm", pos);

// Emergency stop
module.stop()?;

// Home the axis
module.home()?;
```

## ESP300 Protocol Reference

The ESP300 uses ASCII commands over RS-232:

| Command | Format | Description |
|---------|--------|-------------|
| Move Absolute | `{axis}PA{position}` | Move to absolute position |
| Move Relative | `{axis}PR{distance}` | Move relative to current |
| Tell Position | `{axis}TP?` | Query current position |
| Motion Done? | `{axis}MD?` | Check if motion complete |
| Stop | `{axis}ST` | Stop motion |
| Home | `{axis}OR` | Find mechanical zero |
| Set Velocity | `{axis}VA{value}` | Set motion velocity |
| Set Acceleration | `{axis}AC{value}` | Set acceleration |

Serial settings: 19200 baud, 8N1, no flow control.

## Hot-Reload Support

The plugin supports hot-reload during development. State is preserved across reloads:

```rust
// State that survives hot-reload
pub struct Esp300State {
    pub position: f64,
    pub target_position: Option<f64>,
    pub velocity: f64,
    pub acceleration: f64,
    pub is_homed: bool,
    pub is_moving: bool,
}
```

Enable hot-reload in your development build:

```bash
cargo build --features hot-reload
```

## Testing

### Unit Tests

```bash
cargo test
```

### Mock Mode

For testing without hardware:

```bash
cargo test --features mock
```

Or configure mock mode at runtime:

```toml
[modules.params]
mock = true
```

## Troubleshooting

### Common Issues

1. **Serial port permission denied**
   ```bash
   sudo usermod -a -G dialout $USER
   # Log out and back in
   ```

2. **ABI version mismatch**
   - Rebuild plugin against the same `plugin-api` version as the host

3. **Library not found**
   - Check library extension matches platform (.so, .dylib, .dll)
   - Verify `plugin.toml` library name matches

### Debug Logging

Enable tracing for debug output:

```bash
RUST_LOG=esp300_native=debug cargo run
```

## License

MIT OR Apache-2.0
