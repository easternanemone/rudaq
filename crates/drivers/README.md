# drivers

A re-export crate for hardware drivers (compatibility shim).

## Overview

The `drivers` crate aggregates all hardware driver crates and provides unified feature flags for easy dependency management.

**Note**: This is primarily a convenience crate for re-exporting driver implementations. It handles feature coordination but does not implement drivers itself.

## Purpose

Instead of depending on individual driver crates like:

```toml
[dependencies]
driver-pvcam = { path = "../driver-pvcam", features = ["pvcam"] }
driver-comedi = { path = "../driver-comedi", features = ["comedi"] }
driver-thorlabs = { path = "../driver-thorlabs" }
# ... many more
```

You can use:

```toml
[dependencies]
drivers = { path = "../drivers", features = ["maitai"] }
```

## Feature Flags

### Individual Drivers

| Feature | Description | Crate |
|---------|-------------|-------|
| `pvcam` | PVCAM camera (mock mode) | `driver-pvcam` |
| `pvcam_sdk` | PVCAM camera (real SDK) | `driver-pvcam` |
| `comedi` | Comedi DAQ (mock mode) | `driver-comedi` |
| `comedi_hardware` | Comedi DAQ (real hardware) | `driver-comedi` |
| `thorlabs` | ELL14 rotators | `driver-thorlabs` |
| `newport` | ESP300 motion controller, 1830-C power meter | `driver-newport` |
| `spectra_physics` | MaiTai Ti:Sapphire laser | `driver-spectra-physics` |
| `generic` | Generic config-driven serial driver | `driver-generic` |
| `mock` | Mock drivers for testing | `driver-mock` |

### Convenience Sets

| Feature | Includes |
|---------|----------|
| `all` | All drivers: `thorlabs`, `newport`, `spectra_physics`, `pvcam`, `comedi`, `mock`, `generic` |
| `maitai` | Real hardware drivers for the maitai lab: `thorlabs`, `newport`, `spectra_physics`, `pvcam_sdk` |
| `hardware` | All real hardware drivers: `thorlabs`, `newport`, `spectra_physics`, `pvcam_sdk`, `comedi_hardware` |

**Note:** The default feature set is empty (`default = []`). To use drivers, explicitly enable one or more features.

## Usage

### Basic Setup (All Mock Drivers)

```toml
[dependencies]
drivers = { path = "../drivers" }
```

### For Maitai Hardware

```toml
[dependencies]
drivers = { path = "../drivers", features = ["maitai"] }
```

### Custom Feature Set

```toml
[dependencies]
drivers = { path = "../drivers", features = ["pvcam", "thorlabs", "mock"] }
```

## Ensuring Drivers Are Linked

In your main binary, call `link_drivers()` early to ensure driver factories are linked:

```rust
use drivers::link_drivers;

fn main() -> anyhow::Result<()> {
    // Ensure all driver factories are linked into the binary
    link_drivers();

    // Load config, start server, etc.
    Ok(())
}
```

### Why Link Drivers?

In Rust, code that is not directly referenced may be optimized away by the linker. Driver crates register their factories at module initialization time, but if nothing in the binary directly uses the crate, the linker may strip it.

`link_drivers()` provides an explicit reference to each enabled driver crate, ensuring their factory registrations are included in the final binary.

## Available Functions

### `link_drivers()`
Force the linker to include all enabled driver crates. Call this in `main()` before loading config.

```rust
drivers::link_drivers();
```

### `available_drivers()`
Get a list of driver types linked into the binary.

```rust
let drivers = drivers::available_drivers();
for driver_type in drivers {
    println!("Available: {}", driver_type);
}
```

## Re-exported Types

The crate re-exports core driver types for convenience:

```rust
use drivers::{Capability, DeviceComponents, DeviceMetadata, DriverFactory};
```

## Related Documentation

- [CLAUDE.md](../../CLAUDE.md) - Hardware configuration and setup
- [Demo Guide](../../DEMO.md) - Quick start with example devices
- [Contributing](../../CLAUDE.md#supervisors-implementers) - Driver architecture decisions and patterns

## See Also

- `hardware` crate - Device registry and hardware initialization
- `common` crate - Capability traits and abstractions
- Individual driver crates:
  - `driver-pvcam` - Photometrics Prime camera
  - `driver-comedi` - Linux DAQ boards
  - `driver-thorlabs` - ELL14 rotation mounts
  - `driver-newport` - Newport ESP300 and power meter
  - `driver-spectra-physics` - MaiTai laser
  - `driver-mock` - Mock devices for testing
