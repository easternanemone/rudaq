# daq-hardware

Hardware abstraction layer for rust-daq with device registry and driver management.

## Overview

`daq-hardware` provides the central hardware driver system:

- **DeviceRegistry** - Thread-safe device registration and discovery
- **DriverFactory** - Plugin architecture for dynamic driver loading
- **Capability Traits** - Movable, Readable, FrameProducer, etc.
- **Config-Driven Drivers** - TOML-based generic serial drivers
- **Serial Port Management** - Stable by-id paths and multidrop bus support

## DeviceRegistry

Central hub for device management:

```rust
use daq_hardware::{DeviceRegistry, DeviceConfig, DriverType};

let registry = DeviceRegistry::new();

// Register a device
registry.register(DeviceConfig {
    id: "rotator".into(),
    name: "ELL14 Rotator".into(),
    driver: DriverType::Ell14 {
        port: "/dev/serial/by-id/usb-FTDI_FT230X...".into(),
        address: "2".into(),
    },
}).await?;

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

Share a serial port across multiple devices (RS-485):

```rust
use daq_hardware::drivers::ell14::Ell14Bus;

let bus = Ell14Bus::open("/dev/ttyUSB0").await?;
let rotator_2 = bus.device("2").await?;  // Address 2
let rotator_3 = bus.device("3").await?;  // Address 3

// Both share the same serial port
rotator_2.move_abs(45.0).await?;
rotator_3.move_abs(90.0).await?;
```

## Serial Port Resolution

Use stable `/dev/serial/by-id/` paths:

```rust
use daq_hardware::port_resolver::resolve_port;

// Stable across reboots (NOT /dev/ttyUSB0)
let port = resolve_port(
    "/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0"
)?;
```

## Feature Flags

```toml
[features]
# Serial communication
serial = ["tokio-serial"]

# Hardware drivers
thorlabs = ["daq-driver-thorlabs"]
newport = ["daq-driver-newport"]
spectra_physics = ["daq-driver-spectra-physics"]
pvcam = ["daq-driver-pvcam"]
comedi = ["daq-driver-comedi"]

# All hardware (for maitai builds)
all_hardware = ["thorlabs", "newport", "spectra_physics", "pvcam", "comedi"]
```

## Built-in Drivers

| Driver | Traits | Hardware |
|--------|--------|----------|
| ELL14 | Movable, Parameterized | Thorlabs rotation mount |
| MaiTai | Readable, ShutterControl, WavelengthTunable | Spectra-Physics laser |
| ESP300 | Movable, Parameterized | Newport motion controller |
| Newport 1830-C | Readable, WavelengthTunable | Power meter |
| PVCAM | FrameProducer, Triggerable | Photometrics cameras |
| Comedi | Readable, Settable | NI DAQ cards |

## Related Crates

- [`common`](../common) - Capability traits and error types
- [`daq-driver-pvcam`](../daq-driver-pvcam) - PVCAM camera driver
- [`daq-driver-thorlabs`](../daq-driver-thorlabs) - Thorlabs ELL14 driver
- [`daq-driver-newport`](../daq-driver-newport) - Newport drivers

## License

See the repository root for license information.
