# driver-generic

Generic config-driven serial driver for rust-daq.

This crate provides a `GenericSerialDriver` that interprets TOML device
configurations at runtime, enabling new serial instruments to be added
without writing Rust code.

## Overview

The driver-generic system supports two schema versions:

| Schema | Template Syntax | Location | Use Case |
|--------|-----------------|----------|----------|
| v1 | `${variable}` | `driver-generic` crate | Complex devices (regex parsing, conversions, Rhai scripts) |
| v2 | `{{ variable }}` (minijinja) | `hardware` crate | Simple SCPI-like devices |

**Recommendation:** Start with v2 for new devices. Use v1 only if you need
advanced features like custom response parsing, unit conversions, or Rhai scripting.

## Schema Version 2 (Recommended)

Schema v2 uses [minijinja](https://docs.rs/minijinja/) templates and is designed
for straightforward SCPI-style instruments. Configuration files use the
`.scpi.toml` or `.declarative_scpi.toml` suffix.

### TOML Structure

```toml
schema_version = 2
name = "Device Name"
version = "1.0.0"

[connection]
type = "serial"           # or "tcp"
baud_rate = 9600
terminator = "\r\n"
timeout_ms = 1000

[commands.command_name]
template = "COMMAND {{ value }}"
query = true              # true if expecting response
response_type = "float"   # none, string, float, integer, boolean, array_float

[capabilities.movable]    # Maps trait methods to commands
move_abs = "move_abs"
move_rel = "move_rel"
position = "get_position"
stop = "stop"

[[instances]]             # Device instances to register
id = "device_id"
name = "Display Name"
port = "/dev/ttyUSB0"
address = "1"
disable = false
```

### Minijinja Templating

Schema v2 uses minijinja syntax (`{{ variable }}`) instead of strfmt (`${variable}`).

**Available variables in command templates:**
- `value` - The input value passed to capability methods (e.g., position for `move_abs`)

**Example templates:**

```toml
# Simple command with parameter
[commands.move_abs]
template = "1PA {{ value }}"    # Sends "1PA 45.0" for move_abs(45.0)
query = false

# Query command (no parameters)
[commands.get_position]
template = "1TP?"
query = true
response_type = "float"

# Wavelength control
[commands.set_wavelength]
template = "WAVE {{ value }}"   # Sends "WAVE 800.0"
query = false
```

### Response Types

| Type | Description | Example Response |
|------|-------------|------------------|
| `none` | No response expected | - |
| `string` | Raw string | `"OK"` |
| `float` | Floating point number | `"45.123"` |
| `integer` | Integer | `"42"` |
| `boolean` | Boolean (0/1, true/false) | `"1"` |
| `array_float` | Comma-separated floats | `"1.0,2.0,3.0"` |

### Capability Mappings

Map hardware capability traits to your commands:

```toml
# Movable (motion stages)
[capabilities.movable]
move_abs = "move_abs_cmd"      # Required
move_rel = "move_rel_cmd"      # Required
position = "get_position_cmd"  # Required (must be query with float/integer)
stop = "stop_cmd"              # Required
wait_settled = "motion_done"   # Optional

# Readable (sensors, power meters)
[capabilities.readable]
read = "read_cmd"              # Required (must be query)

# WavelengthTunable (lasers, monochromators)
[capabilities.wavelength_tunable]
set_wavelength = "set_wl_cmd"  # Required
get_wavelength = "get_wl_cmd"  # Required (must be query with float)

# ShutterControl
[capabilities.shutter_control]
open_shutter = "open_cmd"      # Required
close_shutter = "close_cmd"    # Required
is_shutter_open = "status_cmd" # Required (must be query with boolean/integer)
```

### Complete Example (Newport ESP300)

```toml
schema_version = 2
name = "Newport ESP300"
version = "1.0.0"

[connection]
type = "serial"
baud_rate = 19200
terminator = "\r\n"
timeout_ms = 5000

[commands.move_abs]
template = "1PA {{ value }}"
query = false
response_type = "none"

[commands.move_rel]
template = "1PR {{ value }}"
query = false
response_type = "none"

[commands.position]
template = "1TP?"
query = true
response_type = "float"

[commands.stop]
template = "1ST"
query = false
response_type = "none"

[commands.motion_done]
template = "1MD?"
query = true
response_type = "boolean"

[capabilities.movable]
move_abs = "move_abs"
move_rel = "move_rel"
position = "position"
stop = "stop"
wait_settled = "motion_done"

[[instances]]
id = "esp300"
name = "Newport ESP300"
port = "/dev/ttyUSB0"
address = "1"
disable = true
```

## Schema Version 1 (Advanced)

Schema v1 is used by the `driver-generic` crate for devices requiring:

- Complex response parsing with regex
- Unit conversions (degrees to pulses, etc.)
- Rhai scripting for protocol logic
- Custom error code handling
- Retry policies

### Key Differences from v2

| Feature | v1 | v2 |
|---------|----|----|
| Template syntax | `${variable}` | `{{ variable }}` |
| Format specifiers | `${value:08X}` | Not supported |
| Response parsing | Regex with named groups | Simple type conversion |
| Unit conversions | Built-in formulas | Not supported |
| Error codes | Full mapping | Not supported |
| Rhai scripting | Optional feature | Not supported |

### v1 Template Syntax

```toml
# Basic interpolation
template = "${address}ma${position}"

# With format specifier (8-char uppercase hex)
template = "${address}ma${position_pulses:08X}"

# Available format specifiers:
# - :X  - uppercase hex
# - :x  - lowercase hex
# - :d  - decimal integer
# - :08X - zero-padded 8-char hex
```

### v1 Response Parsing

```toml
[responses.position]
pattern = "^(?P<addr>[0-9A-Fa-f])PO(?P<pulses>[0-9A-Fa-f]{1,8})$"

[responses.position.fields.pulses]
type = "hex_i32"
signed = true
```

### v1 Unit Conversions

```toml
[conversions.degrees_to_pulses]
formula = "round(degrees * pulses_per_degree)"

[trait_mapping.Movable.move_abs]
command = "move_absolute"
input_conversion = "degrees_to_pulses"
input_param = "position_pulses"
from_param = "position"
```

## Configuration File Locations

Device configurations are stored in `config/devices/`:

```
config/devices/
├── ell14.toml                      # v1 schema (complex Elliptec protocol)
├── ell14.declarative_scpi.toml     # v2 schema (simplified, disabled)
├── esp300.scpi.toml                # v2 schema
├── maitai.scpi.toml                # v2 schema
└── minimal_scpi_template.scpi.toml # v2 template
```

## API Usage

### v2 Driver (GenericScpiDriver)

```rust
use daq_hardware::drivers::generic_scpi::GenericScpiDriver;
use daq_hardware::config::InstrumentManifest;

// Load manifest from TOML
let manifest: InstrumentManifest = toml::from_str(config_str)?;

// Create driver with I/O handle
let driver = GenericScpiDriver::new(manifest, shared_io)?;

// Use via capability traits
use common::capabilities::Movable;
driver.move_abs(45.0).await?;
let pos = driver.position().await?;
```

### v1 Driver (GenericSerialDriver)

```rust
use driver_generic::{GenericSerialDriver, SharedPort};
use plugin_api::config::InstrumentConfig;

// Load config from TOML
let config: InstrumentConfig = toml::from_str(config_str)?;

// Create driver with shared serial port
let driver = GenericSerialDriver::new(config, shared_port, "2")?;

// Use via capability traits
use common::capabilities::Movable;
driver.move_abs(45.0).await?;
```

## Validation

Schema v2 manifests are validated at load time using [garde](https://docs.rs/garde/):

- `schema_version` must equal 2
- All required fields present
- Capability command references exist
- Response types match capability requirements
- Instance configurations valid

Use `InstrumentManifest::validate_strict()` for comprehensive validation:

```rust
let manifest: InstrumentManifest = toml::from_str(config_str)?;
manifest.validate_strict()?;  // Returns garde::Report on validation errors
```

## Feature Flags

| Feature | Description |
|---------|-------------|
| `scripting` | Enable Rhai scripting support (v1 only) |

## See Also

- `crates/hardware/src/drivers/generic_scpi.rs` - v2 driver implementation
- `crates/hardware/src/config/schema_v2.rs` - v2 schema definitions
- `config/devices/*.scpi.toml` - v2 configuration examples
- `config/devices/*.toml` - v1 configuration examples
