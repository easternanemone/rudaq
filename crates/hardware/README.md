# hardware

Hardware abstraction layer for rust-daq with device registry and driver management.

## Overview

`hardware` provides the central hardware driver system:

- **DeviceRegistry** - Thread-safe device registration and discovery
- **DriverFactory** - Plugin architecture for dynamic driver loading
- **Capability Traits** - Movable, Readable, FrameProducer, etc.
- **Config-Driven Drivers** - TOML-based generic serial drivers
- **Serial Port Management** - Stable by-id paths and multidrop bus support

## DeviceRegistry

Central hub for device management:

```rust
use hardware::registry::DeviceRegistry;

let registry = DeviceRegistry::new();

// Register a driver factory
registry.register_factory(Box::new(MyDriverFactory));

// Load devices from TOML config at startup
// (handled by crates/bin/src/main.rs)

// Access by capability
if let Some(device) = registry.get_movable("rotator") {
    device.move_abs(45.0).await?;
}

// List all devices
for info in registry.list_devices() {
    println!("{}: {:?}", info.id, info.capabilities);
}
```

## Capability Traits

Fine-grained traits for device behavior (re-exported from `common`):

| Trait | Purpose | Example Devices |
|-------|---------|-----------------|
| `Movable` | Position control | Stages, rotators |
| `Readable` | Scalar measurements | Power meters, sensors |
| `FrameProducer` | Image acquisition | Cameras |
| `Triggerable` | External triggers | Cameras |
| `ShutterControl` | Shutter open/close | Lasers |
| `WavelengthTunable` | Wavelength control | Tunable lasers |
| `EmissionControl` | Emission on/off | Lasers |
| `Parameterized` | Device settings | All configurable devices |

## Implementing a Driver

Use the `DriverFactory` trait:

```rust
use common::driver::{DriverFactory, DeviceComponents, Capability};

pub struct MyDriverFactory;

impl DriverFactory for MyDriverFactory {
    fn driver_type(&self) -> &'static str { "my_device" }
    fn name(&self) -> &'static str { "My Custom Device" }

    fn capabilities(&self) -> &'static [Capability] {
        &[Capability::Movable, Capability::Readable]
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let driver = Arc::new(MyDriver::new(&config).await?);
            Ok(DeviceComponents::new()
                .with_movable(driver.clone())
                .with_readable(driver))
        })
    }
}

// Register the factory
registry.register_factory(Box::new(MyDriverFactory));
```

## Config-Driven Drivers

Define devices in TOML without writing Rust code:

```toml
# config/devices/my_device.toml
[device]
name = "My Serial Device"
capabilities = ["Movable"]

[connection]
type = "serial"
baud_rate = 9600

[commands.move_absolute]
template = "MA${position}"
response_timeout_ms = 1000

[responses.position]
pattern = "^POS:(?P<value>[0-9.]+)$"
```

## Multidrop Bus Pattern

Share a serial port across multiple devices (RS-485). This is now handled automatically by `driver-universal`'s transport registry. Multiple device configs with the same port path will share a single transport:

```toml
# config/devices/ell14_rotator_a.toml
[connection]
port = "/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0"
address = "1"

# config/devices/ell14_rotator_b.toml
[connection]
port = "/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0"
address = "2"
```

The registry automatically shares the transport to prevent command interleaving.

## Serial Port Resolution

Use stable `/dev/serial/by-id/` paths:

```rust
use hardware::port_resolver::resolve_port;

// Stable across reboots (NOT /dev/ttyUSB0)
let port = resolve_port(
    "/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0"
)?;
```

## Feature Flags

**Note:** Hardware feature flags (pvcam, comedi, andor, etc.) have been moved to [`driver-registry`](../driver-registry). The `hardware` crate only maintains:

```toml
[features]
# Serial communication for tokio-serial support
serial = ["common/serial"]
```

For hardware driver selection, see `driver-registry/Cargo.toml`.

## Available Drivers

**TOML manifest-based** (via `driver-universal`, always compiled):
- ELL14 (Thorlabs rotation mount) - Movable, Parameterized
- MaiTai (Spectra-Physics laser) - Readable, ShutterControl, WavelengthTunable
- ESP300 (Newport motion controller) - Movable, Parameterized
- Newport 1830-C (power meter) - Readable, WavelengthTunable
- IPG YLPP-200 laser, Red Pitaya PID, Thorlabs PM400

**Native SDK drivers** (feature-gated in `driver-registry`):
- PVCAM (Photometrics cameras) - FrameProducer, Triggerable
- Comedi (Linux DAQ cards) - Readable, Settable
- Andor SDK3 (Andor cameras) - FrameProducer, Triggerable
- Dover Motion (Cellino stages) - Movable

## Related Crates

- [`common`](../common) - Capability traits and error types
- [`driver-registry`](../driver-registry) - Hardware feature gating and factory registration
- [`driver-universal`](../driver-universal) - TOML manifest-based driver system
- [`driver-pvcam`](../driver-pvcam) - PVCAM camera driver
- [`driver-mock`](../driver-mock) - Mock devices for testing

## License

See the repository root for license information.
