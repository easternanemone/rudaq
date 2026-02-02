# driver-generic: Declarative SCPI Device Driver

A configuration-driven serial hardware driver that enables adding new SCPI/serial devices without Rust code changes. Devices are defined entirely in TOML configuration files using minijinja template syntax.

## Overview

The generic driver interprets TOML device configurations at runtime, translating capability traits (Movable, Readable, WavelengthTunable, ShutterControl) into serial commands. New devices can be added by dropping a `.scpi.toml` config file in `config/devices/` without touching Rust code.

### Key Features

- **Schema Versioning** - V2 uses minijinja templating (`{{ var }}`), V1 uses strfmt (`${var}`)
- **Capability Mapping** - Declaratively map trait methods to serial commands
- **Response Parsing** - Parse device responses with configurable type conversions
- **Port Sharing** - Multiple device instances can share a single serial port (e.g., RS-485 multidrop)
- **Parameter Management** - Store and reuse device parameters across commands

## V2 Schema Structure

The V2 schema uses minijinja templating for maximum flexibility. All new device configs should use V2.

### Minimal Example

```toml
schema_version = 2
name = "Example SCPI Device"
version = "1.0.0"

[connection]
type = "serial"
baud_rate = 9600
terminator = "\r\n"
timeout_ms = 1000

[commands.read_value]
template = "READ?"
query = true
response_type = "float"

[capabilities.readable]
read = "read_value"

[[instances]]
id = "example_device"
name = "Example SCPI Device"
port = "/dev/ttyUSB0"
```

### Complete Configuration Structure

#### Device Metadata

```toml
schema_version = 2          # Must be 2 (V2 schema with minijinja)
name = "Device Name"        # Human-readable device name
version = "1.0.0"           # Device firmware version (informational)
```

#### Connection Configuration

```toml
[connection]
type = "serial"              # Connection type: "serial" or "tcp"
baud_rate = 9600             # Serial baud rate (serial only)
terminator = "\r\n"          # Command line terminator (CR-LF, LF, etc.)
timeout_ms = 1000            # Response timeout in milliseconds

# For TCP connections instead:
# type = "tcp"
# host = "192.168.1.100"
# port = 5025
# terminator = "\n"
```

#### Commands Section

Commands are the atomic building blocks. Each command has a template (using minijinja syntax) and response configuration.

```toml
[commands.read_value]
template = "READ?"           # Minijinja template: {{ var }} syntax
query = true                 # Whether command expects a response
response_type = "float"      # Response type: none, string, float, integer, boolean, array_float
```

**Command Template Syntax (Minijinja)**

Minijinja templates support variable substitution:

```toml
# Simple variable substitution
template = "MOVE {{ position }}"

# Device address (multi-address buses)
template = "ADDR{{ address }} SET {{ value }}"

# Multiple parameters
template = "{{ command_prefix }}:CONF {{ channel }},{{ range }}"

# Conditional and loops (advanced)
template = "{% if value > 100 %}FAST{% else %}SLOW{% endif %}"
```

**Response Types**

| Type | Description | Example |
|------|-------------|---------|
| `none` | No response expected | Command succeeds or fails |
| `string` | Raw string response | `"DC29V25A"` |
| `float` | Floating-point number | `"3.141592"` |
| `integer` | Integer number | `"42"` |
| `boolean` | Boolean (true/false or 1/0) | `"1"`, `"ON"` |
| `array_float` | Comma-separated floats | `"1.1, 2.2, 3.3"` |

#### Capabilities Section

Maps trait methods to commands. Each capability requires specific commands with matching response types.

**Movable Trait**

```toml
[capabilities.movable]
move_abs = "move_abs_cmd"         # Move to absolute position
move_rel = "move_rel_cmd"         # Move relative to current
position = "get_position_cmd"     # Query current position (float/integer)
stop = "stop_cmd"                 # Emergency stop
wait_settled = "is_settled_cmd"   # Optional: check if motion complete
```

**Readable Trait**

```toml
[capabilities.readable]
read = "read_value_cmd"           # Read value (float/integer/string/boolean)
```

**WavelengthTunable Trait**

```toml
[capabilities.wavelength_tunable]
set_wavelength = "set_wl_cmd"     # Set wavelength command
get_wavelength = "get_wl_cmd"     # Get wavelength (must return float)
```

**ShutterControl Trait**

```toml
[capabilities.shutter_control]
open_shutter = "open_cmd"         # Open shutter
close_shutter = "close_cmd"       # Close shutter
is_shutter_open = "query_cmd"     # Check if open (returns boolean/integer)
```

#### Instances Section

Device instances map logical device IDs to physical ports and addresses.

```toml
[[instances]]
id = "device_id"                  # Unique device ID for registry
name = "Display Name"             # Human-readable name
port = "/dev/ttyUSB0"             # Serial port path (or TCP host:port)
address = "0"                     # Device address (for RS-485, GPIB, etc.)
disable = false                   # Set to true to skip this device
driver_type = "generic_scpi"      # Optional: explicit driver type
baud_rate = 9600                  # Optional: override connection baud_rate
```

## Real-World Examples

### Example 1: MaiTai Laser (WavelengthTunable + Readable)

```toml
schema_version = 2
name = "Spectra-Physics MaiTai"
version = "2.0.0"

[connection]
type = "serial"
baud_rate = 115200
terminator = "\n"
timeout_ms = 1000

[commands.measure_power]
template = "READ:POW?"
query = true
response_type = "float"

[commands.set_wavelength]
template = "WAVE {{ value }}"
query = false
response_type = "none"

[commands.get_wavelength]
template = "WAVE?"
query = true
response_type = "float"

[commands.open_shutter]
template = "SHUTTER:OPEN"
query = false
response_type = "none"

[commands.close_shutter]
template = "SHUTTER:CLOSE"
query = false
response_type = "none"

[commands.is_shutter_open]
template = "SHUTTER?"
query = true
response_type = "boolean"

[capabilities.readable]
read = "measure_power"

[capabilities.wavelength_tunable]
set_wavelength = "set_wavelength"
get_wavelength = "get_wavelength"

[capabilities.shutter_control]
open_shutter = "open_shutter"
close_shutter = "close_shutter"
is_shutter_open = "is_shutter_open"
```

### Example 2: Newport ESP300 Motion Controller (Movable)

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
id = "esp300_axis1"
name = "Newport ESP300 Axis 1"
port = "/dev/ttyUSB0"
address = "1"
```

### Example 3: RS-485 Multi-Address Bus (ELL14 Rotators)

For shared-port scenarios where multiple devices talk on the same RS-485 bus:

```toml
schema_version = 2
name = "Thorlabs ELL14"
version = "1.0.0"

[connection]
type = "serial"
baud_rate = 9600
terminator = ""
timeout_ms = 500

[commands.move_abs]
template = "{{ address }}ma{{ value }}"
query = false
response_type = "none"

[commands.position]
template = "{{ address }}gp?"
query = true
response_type = "integer"

[commands.stop]
template = "{{ address }}gs"
query = false
response_type = "none"

[capabilities.movable]
move_abs = "move_abs"
move_rel = "move_abs"        # Use move_abs for relative via parameter
position = "position"
stop = "stop"

[[instances]]
id = "rotator_2"
name = "ELL14 Rotator (Address 2)"
port = "/dev/serial/by-id/usb-FTDI_FT230X_..."
address = "2"

[[instances]]
id = "rotator_3"
name = "ELL14 Rotator (Address 3)"
port = "/dev/serial/by-id/usb-FTDI_FT230X_..."
address = "3"

[[instances]]
id = "rotator_8"
name = "ELL14 Rotator (Address 8)"
port = "/dev/serial/by-id/usb-FTDI_FT230X_..."
address = "8"
```

## Template Variable Reference

### Available Variables in Templates

Variables are passed into templates when executing commands. Common sources:

| Source | Variables | Notes |
|--------|-----------|-------|
| Method parameters | `value`, `position`, `distance` | From trait method calls |
| Device instance | `address` | From `[[instances]]` section |
| Stored parameters | Custom names | Set via `set_parameter()` |

### Minijinja Syntax

Basic syntax for V2 templates:

```toml
# Simple substitution
template = "SET {{ value }}"

# Filters for formatting
template = "SET {{ value | round }}"

# Conditionals
template = "{% if value > 100 %}FAST{% else %}SLOW{% endif %}"

# Loops
template = "{% for ch in channels %}CH{{ ch }} {% endfor %}"

# Arithmetic
template = "OFFSET {{ value * 10 }}"
```

See [minijinja documentation](https://docs.rs/minijinja/latest/minijinja/) for complete syntax.

### Comparison: V1 vs V2 Syntax

The crate initially supported V1 (strfmt-based), but V2 minijinja is now the standard.

| Feature | V1 (`${var}`) | V2 (`{{ var }}`) |
|---------|--------------|-----------------|
| Variables | `${position}` | `{{ position }}` |
| Format specifiers | `${value:2d}` (hex) | `{{ value \| int }}` + custom filters |
| Conditionals | No | Yes: `{% if %} ... {% endif %}` |
| Loops | No | Yes: `{% for %} ... {% endfor %}` |
| Filters | No | Yes: `{{ value \| round }}` |
| Recommended | No - deprecated | **Yes - use for new configs** |

## Response Parsing

Device responses are parsed based on the `response_type` specified in commands.

### Simple Type Parsing

For `float`, `integer`, `string`, `boolean` responses, the driver extracts the numeric or boolean value from the response string:

```toml
[commands.temperature]
template = "TEMP?"
query = true
response_type = "float"
# Device response: "25.3C\r\n" → parsed as 25.3
```

### Array Response Parsing

For comma-separated arrays:

```toml
[commands.spectrum]
template = "SPEC?"
query = true
response_type = "array_float"
# Device response: "1.1,2.2,3.3" → parsed as [1.1, 2.2, 3.3]
```

## Capability Validation

The driver validates that:

1. **Commands referenced in capabilities exist** - No dangling references
2. **Response types match capability requirements**:
   - `position` (Movable) → must return `float` or `integer`
   - `read` (Readable) → must return `float`, `integer`, `string`, or `boolean`
   - `get_wavelength` (WavelengthTunable) → must return `float`
   - `is_shutter_open` (ShutterControl) → must return `boolean` or `integer`

Example - this config will fail validation:

```toml
[commands.bad_position]
template = "POS?"
query = true
response_type = "string"  # ERROR: position must return float/integer

[capabilities.movable]
position = "bad_position"  # ← Validation error here
```

## Using the Driver

### Loading Configurations

Configs are loaded at daemon startup:

```rust
use driver_generic::GenericSerialDriverFactory;
use std::path::Path;

// Load single config
let factory = GenericSerialDriverFactory::from_file(
    Path::new("config/devices/maitai.scpi.toml")
)?;

// Load all configs from directory
let factories = driver_generic::load_all_factories(
    Path::new("config/devices")
)?;
```

### Instantiating Drivers

The factory creates instances from TOML configuration:

```rust
let components = factory.build(instance_config)?;

// Use through trait objects
use common::capabilities::Movable;
components.movable.unwrap().move_abs(45.0).await?;
```

### Via Hardware Registry

The daemon automatically discovers and registers `.scpi.toml` files:

```bash
# Place config in config/devices/
cp my_device.scpi.toml config/devices/

# Restart daemon
./rust-daq-daemon daemon --port 50051 --hardware-config config/config.toml
```

Device appears in the hardware registry and is accessible via gRPC:

```bash
# GUI shows device in Instruments panel
# Scripts can access: device("device_id")
# API: ReadValue, SetParameter, ExecuteCommand
```

## Parameter Management

The driver maintains a parameter cache for use in templates:

```rust
// Set a parameter
driver.set_parameter("offset", 10.0).await;

// Retrieve parameter
let val = driver.get_parameter("offset").await;  // Some(10.0)

// Use in template
let cmd = driver.format_command("my_cmd", &[("offset", 10.0)])?;
// Template: "OFFSET {{ offset }}" → "OFFSET 10"
```

## Port Sharing and RS-485

For shared ports (multiple devices on one RS-485 bus), instances use the device `address` field:

```toml
[[instances]]
id = "rotator_2"
port = "/dev/serial/by-id/usb-FTDI_..."
address = "2"

[[instances]]
id = "rotator_3"
port = "/dev/serial/by-id/usb-FTDI_..."
address = "3"
```

The `{{ address }}` variable is automatically substituted into templates:

```toml
[commands.move_abs]
template = "{{ address }}ma{{ value }}"
# rotator_2: "2ma45" (address=2, value=45)
# rotator_3: "3ma45" (address=3, value=45)
```

The factory's `port_cache` ensures the same port instance is reused across all device instances, preventing port conflicts.

## Crate Features

```toml
[features]
default = []
scripting = ["dep:rhai"]    # Enable Rhai script engine support
```

## Testing

Run integration tests:

```bash
cargo test -p driver-generic

# With more detailed output
cargo test -p driver-generic -- --nocapture --test-threads=1
```

## Configuration File Locations

By convention, device configs are stored in `config/devices/`:

```
config/
  devices/
    maitai.scpi.toml          # Spectra-Physics MaiTai laser
    esp300.scpi.toml          # Newport ESP300 motion
    ell14.scpi.toml           # Thorlabs ELL14 rotators
    custom_device.scpi.toml   # Your custom device
```

## Debugging

### Enable Tracing

Set `RUST_LOG=debug` to see command execution and response parsing:

```bash
RUST_LOG=debug ./rust-daq-daemon daemon --port 50051
```

Output includes:
- Commands sent to device
- Raw device responses
- Parsed values
- Parameter lookups
- Template interpolation

### Validate Configuration

Use the strict validation API:

```rust
use std::path::Path;
let factory = GenericSerialDriverFactory::from_file(
    Path::new("config/devices/my_device.scpi.toml")
)?;
// If this succeeds, config is valid
```

If validation fails, error message indicates missing commands or type mismatches.

### Test Commands Manually

```bash
# Using a serial terminal to test device communication
# before configuring the TOML

# Example: Test MaiTai
minicom -D /dev/ttyUSB0 -b 115200
# Type: WAVE?
# Should respond with wavelength value
```

## Troubleshooting

### Device not appearing in registry

1. Check config file is in `config/devices/` with `.scpi.toml` extension
2. Validate config loads without errors: `RUST_LOG=debug` daemon start
3. Check `disable = false` in all `[[instances]]` sections
4. Verify port path exists: `ls -la /dev/ttyUSB0` or `/dev/serial/by-id/...`

### Commands timing out

1. Check `timeout_ms` in `[connection]` section (increase if device is slow)
2. Verify device terminator matches actual device (e.g., `\n` vs `\r\n`)
3. Check baud rate matches device (common: 9600, 19200, 115200)
4. Test manually with serial terminal to confirm device responds

### Response parsing errors

1. Check `response_type` matches what device actually returns
2. Test response manually with serial terminal
3. Verify device response doesn't have unexpected characters/formatting

### Multi-device port conflicts

1. Use `/dev/serial/by-id/` paths instead of `/dev/ttyUSBX` (prevents conflicts on reboot)
2. Ensure only ONE config file manages each port
3. Check all instances using same port have unique addresses

## References

- **Minijinja**: https://docs.rs/minijinja/latest/minijinja/
- **SCPI Standard**: https://en.wikipedia.org/wiki/Standard_Commands_for_Programmable_Instruments
- **Example Configs**: `config/devices/*.scpi.toml`
- **Schema Validation**: `crates/hardware/src/config/schema_v2.rs`
