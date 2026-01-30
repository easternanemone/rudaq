# ELL14 Config-Only Plugin Example

This example demonstrates the **simplest plugin path** in rust-daq: adding hardware support with **just configuration files and ZERO code**.

## The Zero-Code Approach

Traditional hardware drivers require:
- Writing Rust code
- Implementing trait methods
- Compiling the driver
- Rebuilding the application

Config-only plugins eliminate all of that. Users can add new instruments by:
1. Creating a `plugin.toml` manifest
2. Writing a `device.toml` protocol definition
3. Dropping the plugin folder into the plugins directory

The `GenericSerialDriver` interprets `device.toml` at runtime, providing full hardware support without any compilation.

## Directory Structure

```
ell14-config/
├── plugin.toml    # Plugin metadata and discovery info
├── device.toml    # Complete protocol definition
└── README.md      # This file
```

## How It Works

### 1. Plugin Discovery

The plugin registry scans the plugins directory and finds `plugin.toml`:

```toml
[plugin]
name = "ell14-config"
version = "1.0.0"
description = "Thorlabs ELL14 rotation mount via config-driven driver"
categories = ["stage", "motion"]
```

### 2. Device Configuration

The `device.toml` defines everything the `GenericSerialDriver` needs:

- **Connection settings**: Baud rate, parity, timeouts
- **Commands**: Templates with parameter interpolation
- **Responses**: Regex patterns for parsing
- **Conversions**: Unit transformations (degrees ↔ pulses)
- **Error codes**: Severity levels and recovery actions
- **Trait mapping**: How standard traits map to device commands

### 3. Runtime Interpretation

When you request an ELL14 driver:

```rust
use daq_hardware::factory::DriverFactory;

// Load from plugin's device.toml
let driver = DriverFactory::create_from_file(
    "plugins/ell14-config/device.toml",
    serial_port,
    "0"  // RS-485 address
).await?;

// Use standard Movable trait - works just like a coded driver!
driver.move_abs(45.0).await?;
let position = driver.position().await?;
```

## Key Sections in device.toml

### Commands with Templates

```toml
[commands.move_absolute]
template = "${address}ma${position_pulses:08X}"
description = "Move to absolute position"
parameters = { position_pulses = "int32" }
timeout_ms = 5000
```

The template `${address}ma${position_pulses:08X}` becomes `0ma00004650` for address "0" and 17744 pulses.

### Response Parsing

```toml
[responses.position]
pattern = "^(?P<addr>[0-9A-Fa-f])PO(?P<pulses>[0-9A-Fa-f]{1,8})$"

[responses.position.fields.pulses]
type = "hex_i32"
signed = true
```

Response `0PO00004650` is parsed to extract `pulses = 17744`.

### Unit Conversions

```toml
[conversions.degrees_to_pulses]
formula = "round(degrees * pulses_per_degree)"

[conversions.pulses_to_degrees]
formula = "pulses / pulses_per_degree"
```

Users work in degrees; the driver handles pulse conversion automatically.

### Trait Mapping

```toml
[trait_mapping.Movable.move_abs]
command = "move_absolute"
input_conversion = "degrees_to_pulses"
input_param = "position_pulses"
from_param = "position"
```

This maps the `Movable::move_abs(degrees)` trait method to the device's `move_absolute` command with automatic unit conversion.

## Adding Your Own Device

1. **Copy this example** as a starting point
2. **Edit plugin.toml** with your device metadata
3. **Create device.toml** defining:
   - Connection settings for your device
   - Command templates from the device manual
   - Response patterns to parse replies
   - Unit conversions if needed
   - Trait mappings for standard capabilities

4. **Test** by loading your plugin and exercising the device

## Benefits

| Traditional Driver | Config-Only Plugin |
|--------------------|-------------------|
| Requires Rust knowledge | Just TOML editing |
| Must compile | No compilation |
| Rebuild application | Hot-reload capable |
| Code changes for fixes | Config updates only |
| Developer-only | User-extensible |

## Reference

- [ELL14 Protocol Manual](https://www.thorlabs.com/Software/Elliptec/Communications_Protocol/) - Thorlabs Elliptec protocol documentation
- [GenericSerialDriver](../../crates/hardware/src/drivers/generic_serial.rs) - The runtime interpreter
- [Device Config Schema](../../crates/hardware/src/config/schema.rs) - Full TOML schema reference
