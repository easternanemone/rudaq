# daq-driver-thorlabs

Rust driver for Thorlabs optical devices, with comprehensive support for the ELL14 rotation mount.

## Hardware Supported

- **Thorlabs ELL14 Elliptec Rotation Mount** - Compact motorized rotation stage for precision wavelength selection, polarization control, and optical element rotation

## Quick Start

### Hardware Setup

ELL14 devices communicate via **RS-485 multidrop bus** on a single serial port:

- **Baud Rate:** 9600
- **Bus Type:** RS-485 (multidrop, half-duplex)
- **Protocol:** ASCII-encoded hex commands
- **Addressing:** Each device has a unique address 0-F (0-15)
- **Serial Port Path:** `/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_*` (see Hardware Inventory)

### Configuration Example

```toml
[[devices]]
id = "rotator_2"
type = "ell14"
enabled = true

[devices.config]
port = "/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0"
address = "2"  # ELL14 device address (0-F)
```

### Usage in Rust

```rust
use daq_driver_thorlabs::Ell14Factory;
use common::driver::DriverFactory;

// Register the factory
registry.register_factory(Box::new(Ell14Factory));

// Create via config
let config = toml::toml! {
    port = "/dev/ttyUSB1"
    address = "2"
};
let components = factory.build(config.into()).await?;

// Use the driver
let movable = components.movable.unwrap();
movable.move_abs(45.0).await?;  // Rotate to 45 degrees
```

### Usage in Rhai Scripts

```rhai
let rotator = create_ell14("/dev/serial/by-id/...", "2");

// Move to absolute angle (degrees)
rotator.move_to(45.0);

// Get current velocity (0-100%)
let vel = rotator.velocity();

// Set velocity for speed/precision tradeoff
rotator.set_velocity(100);  // Maximum speed

// Refresh cached settings from hardware
rotator.refresh_settings();
```

## Features

### Velocity Control

The ELL14 supports velocity control from 0-100% for balancing speed vs positioning precision:

- **100% (Maximum):** Fastest motion, lower precision (~1 degree)
- **50% (Moderate):** Balanced speed and accuracy
- **10% (Minimum):** Slowest, highest precision

When using the calibrated initialization mode (`with_shared_port_calibrated`), velocity is automatically set to 100% for fastest operation during scanning.

```rust
// Get current velocity setting
let vel = driver.get_velocity().await?;  // Query from hardware

// Set velocity for next movements
driver.set_velocity(50).await?;  // 50% speed

// Fast cached read (non-blocking)
let cached = driver.cached_velocity().await;
```

### Cached Settings

The driver maintains a cache of device settings that can be read without blocking on serial I/O:

```rust
// Non-blocking reads from cache
let ppd = driver.cached_pulses_per_degree().await;

// Blocking reads from hardware
let ppd = driver.get_pulses_per_degree().await?;

// Refresh cache from hardware
driver.refresh_cached_settings().await?;
```

### RS-485 Bus Management

The ELL14 Elliptec protocol uses RS-485 multidrop addressing. Multiple rotators can share a single serial port:

```rust
// Each device specified by its hex address (0-F)
let rotator_2 = Ell14Driver::with_shared_port_calibrated(port, "2").await?;
let rotator_3 = Ell14Driver::with_shared_port_calibrated(port, "3").await?;
let rotator_8 = Ell14Driver::with_shared_port_calibrated(port, "8").await?;
```

### Auto-Calibration

On initialization, the driver queries the device firmware for calibration data:

```rust
// Automatic calibration (queries device)
let driver = Ell14Driver::with_shared_port_calibrated(port, "2").await?;

// Or use custom calibration (e.g., 45.1 pulses per degree)
let driver = Ell14Driver::with_calibration(port, "2", 45.1);
```

## Protocol Reference

### Command Format

Thorlabs ELL14 uses hex-encoded command format:

```
Address: Single hex digit (0-F)
Command: 2-letter ASCII
Data: Hex digits (variable length)
```

Example: `2MV0200` = Address 2, Move Velocity to 0x200 (512)

### Common Commands

| Command | Format | Description |
|---------|--------|-------------|
| Move Absolute | `{addr}MV{data}` | Rotate to absolute angle |
| Home | `{addr}HO` | Home the device |
| Get Status | `{addr}GS` | Query device status |
| Get Info | `{addr}IN` | Query device information |
| Get Pulses/Degree | `{addr}GL` | Query calibration data |
| Set Velocity | `{addr}SV{data}` | Set rotation velocity |

### Capabilities Implemented

- **Movable:** `move_abs(degrees)`, `move_rel(degrees)`
- **Parameterized:** Velocity and calibration parameter control

## Hardware Inventory

### maitai Machine (Verified Working)

| Rotator | Address | Serial Port | Notes |
|---------|---------|-------------|-------|
| rotator_2 | 2 | `/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0` | ELL14 on shared RS-485 bus |
| rotator_3 | 3 | `/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0` | ELL14 on shared RS-485 bus |
| rotator_8 | 8 | `/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0` | ELL14 on shared RS-485 bus |

**Important:** Always use `/dev/serial/by-id/` paths, not `/dev/ttyUSB*`. Device numbers change on reboot; by-id paths are stable.

## Troubleshooting

### Device Not Found

```
Error: "Failed to open port /dev/ttyUSB1"
```

**Solution:** Use `/dev/serial/by-id/` path instead. Find your device:

```bash
ls -la /dev/serial/by-id/ | grep FTDI
```

### "Wrong device connected" Error

```
Error: "Wrong device connected"
```

**Solution:** The device at the given address didn't respond correctly. Check:

1. Serial port is correct
2. Device address matches (check ELL14 DIP switches or firmware address)
3. No other devices on the bus using the same address

### Timeout During Movement

```
Error: "Mechanical timeout"
```

**Solution:** Device encountered a hard stop or is jammed. Check:

1. Mechanical stops aren't engaged
2. Motor isn't obstructed
3. Device is powered correctly

### Velocity Parameter Not Changing

**Issue:** Velocity setting is ignored

**Solution:** Velocity is cached for performance. To verify hardware has the new setting:

```rust
driver.set_velocity(50).await?;
driver.refresh_cached_settings().await?;  // Force refresh from hardware
let actual = driver.get_velocity().await?;
assert_eq!(actual, 50);
```

## Configuration Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `port` | string | Required | Serial port path (use `/dev/serial/by-id/`) |
| `address` | string | Required | ELL14 address (0-F hex) |
| `pulses_per_degree` | float | None (auto) | Custom calibration (pulses/degree) |
| `timeout_ms` | integer | 500 | Command timeout in milliseconds |

## Dependencies

- `tokio` - Async runtime
- `tokio-serial` - Async serial I/O
- `anyhow` - Error handling
- `serde` - TOML configuration

## See Also

- [CLAUDE.md - Hardware Drivers](../../CLAUDE.md#serial-driver-conventions) - Serial driver patterns and conventions
- [CLAUDE.md - ELL14 Specifics](../../CLAUDE.md#ell14-rotator-rs-485-bus) - Velocity control and calibration
- [Hardware Drivers Guide](../../docs/guides/hardware-drivers.md) - General driver development
