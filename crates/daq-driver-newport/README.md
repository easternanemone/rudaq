# daq-driver-newport

Rust drivers for Newport precision optical instruments: the ESP300 motion controller and 1830-C optical power meter.

## Hardware Supported

- **Newport ESP300 Universal Motion Controller** - 3-axis programmable motion controller with RS-232 communication
- **Newport 1830-C Optical Power Meter** - High-precision optical power measurement instrument

## Quick Start

### ESP300 Motion Controller

#### Hardware Setup

- **Baud Rate:** 19200
- **Protocol:** ASCII command/response, 8N1, no flow control
- **Serial Port Path:** `/dev/ttyUSB0` (see Hardware Inventory)
- **Axes:** 1-3 (each axis controlled independently)

#### Configuration Example

```toml
[[devices]]
id = "esp300_axis1"
type = "esp300"
enabled = true

[devices.config]
port = "/dev/ttyUSB0"
axis = 1  # Axis 1, 2, or 3
```

#### Usage in Rust

```rust
use daq_driver_newport::Esp300Factory;
use common::driver::DriverFactory;

// Register the factory
registry.register_factory(Box::new(Esp300Factory));

// Create via config
let config = toml::toml! {
    port = "/dev/ttyUSB0"
    axis = 1
};
let components = factory.build(config.into()).await?;

// Move to absolute position (mm)
let movable = components.movable.unwrap();
movable.move_abs(10.5).await?;  // Move to 10.5mm

// Query position
let pos = movable.get_position().await?;
println!("Current position: {} mm", pos);
```

#### Usage in Rhai Scripts

```rhai
let stage = create_esp300("/dev/ttyUSB0", 1);

// Move to absolute position (mm)
stage.move_to(5.0);

// Move relative (mm)
stage.move_by(2.0);

// Get current position
let pos = stage.position();
print(`Stage at ${pos} mm`);

// Axis calibration (consult ESP300 manual)
stage.set_parameter("home_position", 0.0);
```

### Newport 1830-C Power Meter

#### Hardware Setup

- **Baud Rate:** 9600
- **Protocol:** ASCII command/response (not SCPI), 8N1, no flow control
- **Serial Port Path:** `/dev/ttyS0` (built-in RS-232, always stable)
- **Measurement Units:** Watts (W)

**Important:** The 1830-C uses a non-standard ASCII protocol, NOT SCPI. Command format differs from ESP300.

#### Configuration Example

```toml
[[devices]]
id = "power_meter"
type = "newport_1830c"
enabled = true

[devices.config]
port = "/dev/ttyS0"
```

#### Usage in Rust

```rust
use daq_driver_newport::Newport1830CFactory;
use common::driver::DriverFactory;

// Register the factory
registry.register_factory(Box::new(Newport1830CFactory));

// Create via config
let config = toml::toml! {
    port = "/dev/ttyS0"
};
let components = factory.build(config.into()).await?;

// Read optical power
let readable = components.readable.unwrap();
let power_w = readable.read_value().await?;
println!("Optical power: {:.3} W", power_w);
```

#### Usage in Rhai Scripts

```rhai
let power_meter = create_power_meter("/dev/ttyS0");

// Read power (returns Watts)
let power = power_meter.read_power();
print(`Power: ${power * 1000} mW`);  // Convert to mW

// Zero/calibrate
power_meter.zero();
sleep(2.0);  // Wait for calibration

let baseline = power_meter.read_power();
```

## Features

### ESP300: Multi-Axis Motion Control

```rust
// Axis validation (must be 1-3)
let driver = Esp300Driver::new_async("/dev/ttyUSB0", 2).await?;

// Movement with automatic status checking
driver.move_abs(25.0).await?;   // Move to 25mm
let pos = driver.get_position().await?;

// Command timeout
let driver = Esp300Driver::new_async_with_timeout(
    "/dev/ttyUSB0",
    1,
    Duration::from_secs(10)
).await?;
```

### Newport 1830-C: Precision Power Measurement

```rust
// Read power (returns Watts)
let power = driver.read_value().await?;

// Wavelength tuning capability
driver.set_wavelength(808.0).await?;

// Optional: Auto-zeroing
driver.zero().await?;
```

### Shared Device Registry

Both devices use the standard `DriverFactory` pattern for integration with the device registry:

```rust
// Register both factories
registry.register_factory(Box::new(Esp300Factory));
registry.register_factory(Box::new(Newport1830CFactory));

// Create from TOML config
let esp300 = registry.create_device("esp300", config).await?;
let meter = registry.create_device("newport_1830c", config).await?;
```

## Protocol Reference

### ESP300 Command Format

ASCII commands for motion control:

```
Format: {Axis}{Command}{Value}
Example: "1PA5.0"  → Axis 1, Position Absolute, 5.0mm

Common Commands:
- PA{value}  - Position Absolute (mm)
- PR{value}  - Position Relative (mm)
- DH         - Define Home (current position = 0)
- VE{value}  - Velocity (encoder units/sec)
- VA{value}  - Acceleration (encoder units/sec²)
- ?          - Query position
```

### Newport 1830-C Command Format

ASCII protocol (NOT SCPI):

```
Format: {Command} {Parameters}
Example: "W?"  → Query wavelength

Common Commands:
- W?         - Query wavelength
- W {value}  - Set wavelength (nm)
- P?         - Query power (Watts)
- Z          - Zero/calibrate
- *IDN?      - Query device identity
```

## Capabilities Implemented

### ESP300

- **Movable:** `move_abs(mm)`, `move_rel(mm)`, `get_position()`
- **Parameterized:** Velocity, acceleration, home position

### Newport 1830-C

- **Readable:** `read_value()` → Watts
- **WavelengthTunable:** `set_wavelength(nm)`, `get_wavelength()`
- **Parameterized:** Measurement range, units

## Hardware Inventory

### maitai Machine (Verified Working)

| Device | Axis/Port | Serial Port | Baud Rate | Notes |
|--------|-----------|-------------|-----------|-------|
| esp300_axis1 | Axis 1 | `/dev/ttyUSB0` | 19200 | X-axis motion |
| power_meter | N/A | `/dev/ttyS0` | 9600 | Built-in RS-232 (stable) |

**Note:** The power meter uses `/dev/ttyS0` (built-in serial) rather than USB. This port is stable across reboots.

## Configuration Options

### ESP300

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `port` | string | Required | Serial port path |
| `axis` | integer | Required | Axis number (1-3) |
| `timeout_secs` | integer | 5 | Command timeout in seconds |

### Newport 1830-C

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `port` | string | Required | Serial port path (typically `/dev/ttyS0`) |
| `timeout_secs` | integer | 5 | Command timeout in seconds |

## Troubleshooting

### ESP300 Axis Out of Range

```
Error: "ESP300 axis must be 1-3, got 4"
```

**Solution:** ESP300 supports only 3 axes. Create separate driver instances for each axis:

```rust
let axis_1 = Esp300Driver::new_async("/dev/ttyUSB0", 1).await?;
let axis_2 = Esp300Driver::new_async("/dev/ttyUSB0", 2).await?;
let axis_3 = Esp300Driver::new_async("/dev/ttyUSB0", 3).await?;
```

### Power Meter Reads All Zeros

```
Power: 0.000 W (unchanging)
```

**Solution:** Check:

1. Input connector is connected
2. Device is switched on (LED should be lit)
3. Correct wavelength is set (if applicable)
4. Optical input is not blocked

### Communication Timeout

```
Error: "Command timeout after 5s"
```

**Solution:** Increase timeout or check device is responding:

```rust
let driver = Esp300Driver::new_async_with_timeout(
    "/dev/ttyUSB0",
    1,
    Duration::from_secs(10)  // Increase to 10 seconds
).await?;
```

### Wrong Serial Port

```
Error: "Failed to open port /dev/ttyUSB0"
```

**Solution:** Find the correct port:

```bash
# List all serial devices
ls -la /dev/serial/by-id/

# Or use dmesg to find USB device
dmesg | grep ttyUSB
```

## Dependencies

- `tokio` - Async runtime
- `tokio-serial` - Async serial I/O
- `anyhow` - Error handling
- `serde` - TOML configuration

## See Also

- [CLAUDE.md - Serial Driver Conventions](../../CLAUDE.md#serial-driver-conventions) - General serial driver patterns
- [CLAUDE.md - Hardware Inventory](../../CLAUDE.md#hardware-inventory-maitai) - Device port mappings
- [Hardware Drivers Guide](../../docs/guides/hardware-drivers.md) - General driver development
