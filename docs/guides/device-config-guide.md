# Creating Device Configurations

A step-by-step guide to creating TOML configuration files for the GenericSerialDriver.

## Table of Contents

- [Overview](#overview)
- [Quick Start (5 Minutes)](#quick-start-5-minutes)
- [Step-by-Step Guide](#step-by-step-guide)
- [Template Reference](#template-reference)
- [Common Patterns](#common-patterns)
- [Testing Your Config](#testing-your-config)
- [Troubleshooting](#troubleshooting)

---

## Overview

The GenericSerialDriver allows you to add support for new hardware devices **without writing any Rust code**. Instead, you define the device protocol in a TOML configuration file.

### When to Use Config-Driven Drivers

| Use Config-Driven | Use Native Rust |
|-------------------|-----------------|
| ASCII command/response protocols | Complex binary protocols |
| Standard serial communication | High-performance requirements |
| Simple state machines | Proprietary SDKs |
| Quick prototyping | Multi-threaded I/O |

### Available Templates

| Template | Lines | Use Case |
|----------|-------|----------|
| `minimal_device_template.toml` | ~85 | Starting point - only required fields |
| `sample_temperature_controller.toml` | ~690 | Comprehensive reference with all features |
| `ell14.toml` | ~400 | Real-world example (Thorlabs rotator) |

---

## Quick Start (5 Minutes)

### 1. Copy the Minimal Template

```bash
cp config/devices/minimal_device_template.toml config/devices/my_device.toml
```

### 2. Edit Required Fields

```toml
[device]
name = "Acme Power Meter PM-100"      # Your device name
protocol = "acme_pm100"               # Unique identifier

[connection]
type = "serial"
baud_rate = 19200                     # Match your device
terminator_tx = "\r\n"                # Command terminator
terminator_rx = "\r\n"                # Response terminator
```

### 3. Add Your First Command

```toml
[commands.read_power]
template = "MEAS:POW?"
description = "Query power reading"
response = "power_reading"

[responses.power_reading]
pattern = "^(?P<value>[+-]?\\d+\\.?\\d*(?:[eE][+-]?\\d+)?)$"

[responses.power_reading.fields.value]
type = "float"
unit = "W"
```

### 4. Test It

```bash
# Load and validate your config
cargo run --example validate_config -- config/devices/my_device.toml
```

---

## Step-by-Step Guide

### Step 1: Gather Device Information

Before writing the config, collect:

- [ ] **Communication settings**: Baud rate, data bits, parity, stop bits
- [ ] **Command format**: ASCII? Binary? What terminators?
- [ ] **Command list**: All commands you need to support
- [ ] **Response format**: How does the device respond?
- [ ] **Error codes**: How does the device report errors?

**Tip**: Check the device manual or use a serial terminal to experiment.

### Step 2: Create the Config File

Start with the minimal template:

```bash
cp config/devices/minimal_device_template.toml config/devices/my_device.toml
```

### Step 3: Configure Device Identity

```toml
[device]
name = "Acme Power Meter PM-100"      # Display name
description = "Optical power meter with wavelength correction"
manufacturer = "Acme Instruments"
model = "PM-100"
protocol = "acme_pm100"               # MUST be unique across all configs
category = "sensor"                   # See categories below
capabilities = ["Readable"]           # See capabilities below
```

**Categories**: `stage`, `sensor`, `source`, `detector`, `modulator`, `analyzer`, `data_acquisition`, `other`

**Capabilities**: `Movable`, `Readable`, `Settable`, `ShutterControl`, `WavelengthTunable`, `FrameProducer`, `Triggerable`, `ExposureControl`, `EmissionControl`, `Stageable`, `Commandable`, `Parameterized`

### Step 4: Configure Connection Settings

```toml
[connection]
type = "serial"                       # serial, rs485, tcp, udp
baud_rate = 19200                     # Device-specific
data_bits = 8                         # Usually 8
parity = "none"                       # none, odd, even
stop_bits = 1                         # 1 or 2
flow_control = "none"                 # none, software, hardware
timeout_ms = 2000                     # Response timeout

# Command/response terminators (IMPORTANT - check your device manual!)
terminator_tx = "\r\n"                # Sent after each command
terminator_rx = "\r\n"                # Expected in responses
```

**Common terminator patterns**:
- `"\r\n"` - Carriage return + line feed (most common)
- `"\n"` - Line feed only
- `"\r"` - Carriage return only
- `""` - No terminator (fixed-length responses)

### Step 5: Define Commands

Commands use **template syntax** with parameter interpolation:

```toml
# Simple command (no parameters)
[commands.get_id]
template = "*IDN?"
description = "Query device identity"
response = "identity"

# Command with one parameter
[commands.set_wavelength]
template = "WAVE ${wavelength}"
description = "Set wavelength for power correction"
parameters = { wavelength = "float" }
expects_response = false

# Command with formatted parameter
[commands.move_to]
template = "MA${position:08X}"        # 8-char uppercase hex
description = "Move to absolute position"
parameters = { position = "int32" }
timeout_ms = 5000                     # Override default timeout
```

**Parameter types**: `string`, `int32`, `int64`, `uint32`, `uint64`, `float`, `bool`

**Format specifiers** (after `:`):
- `08X` - 8-character uppercase hex
- `08x` - 8-character lowercase hex
- `04d` - 4-digit decimal with leading zeros
- `.2f` - Float with 2 decimal places

### Step 6: Define Response Parsing

Use **regex with named capture groups** to parse responses:

```toml
# Simple numeric response: "1.234e-3"
[responses.power_reading]
pattern = "^(?P<value>[+-]?\\d+\\.?\\d*(?:[eE][+-]?\\d+)?)$"

[responses.power_reading.fields.value]
type = "float"
unit = "W"

# Response with multiple fields: "WAVE 800 RANGE 2"
[responses.settings]
pattern = "^WAVE\\s+(?P<wavelength>\\d+)\\s+RANGE\\s+(?P<range>\\d+)$"

[responses.settings.fields.wavelength]
type = "int"
unit = "nm"

[responses.settings.fields.range]
type = "int"

# Hex response: "0PO00004650"
[responses.position]
pattern = "^(?P<addr>[0-9A-Fa-f])PO(?P<pulses>[0-9A-Fa-f]{8})$"

[responses.position.fields.addr]
type = "string"

[responses.position.fields.pulses]
type = "hex_i32"                      # Parse as signed 32-bit hex
signed = true
```

**Field types**: `string`, `int`, `uint`, `float`, `bool`, `hex_u8`, `hex_u16`, `hex_u32`, `hex_u64`, `hex_i32`, `hex_i64`

### Step 7: Add Unit Conversions (Optional)

For devices that use internal units different from user-facing units:

```toml
[conversions.degrees_to_pulses]
formula = "round(degrees * 398.2222)"
description = "Convert degrees to motor pulses"

[conversions.pulses_to_degrees]
formula = "pulses / 398.2222"
description = "Convert motor pulses to degrees"
```

**Available functions**: `round()`, `floor()`, `ceil()`, `abs()`, `min()`, `max()`, `sqrt()`, `sin()`, `cos()`, `tan()`

### Step 8: Map Trait Methods (Optional)

Connect your commands to capability traits for integration with scripts:

```toml
# For Readable trait
[trait_mapping.Readable.read]
command = "read_power"
output_field = "value"

# For Movable trait
[trait_mapping.Movable.move_abs]
command = "move_to"
input_conversion = "degrees_to_pulses"
input_param = "position"
from_param = "position"

[trait_mapping.Movable.position]
command = "get_position"
output_conversion = "pulses_to_degrees"
output_field = "pulses"
```

---

## Template Reference

### Minimal Template Structure

```toml
# REQUIRED
[device]
name = "..."
protocol = "..."

[connection]
type = "serial"

# RECOMMENDED
[commands.your_command]
template = "..."

[responses.your_response]
pattern = "..."
```

### Full Template Structure

```toml
[device]                    # Device identity (REQUIRED)
[connection]                # Communication settings (REQUIRED)
[connection.bus]            # RS-485/multidrop settings
[default_retry]             # Default retry behavior
[parameters.*]              # Runtime parameters
[commands.*]                # Command definitions
[responses.*]               # Response parsing
[conversions.*]             # Unit conversions
[error_codes.*]             # Error code mapping
[validation.*]              # Parameter validation
[trait_mapping.*]           # Capability trait mapping
[[init_sequence]]           # Initialization commands
[scripts.*]                 # Rhai scripts
[ui]                        # Control panel configuration
[binary_commands.*]         # Binary protocol commands
[binary_responses.*]        # Binary response parsing
```

---

## Common Patterns

### Pattern 1: Query/Response Device

Most instruments follow a query/response pattern:

```toml
[commands.get_reading]
template = "READ?"
response = "reading"

[responses.reading]
pattern = "^(?P<value>[+-]?\\d+\\.?\\d*)$"

[responses.reading.fields.value]
type = "float"
```

### Pattern 2: Set/Get Parameter

```toml
[commands.get_setpoint]
template = "SETP?"
response = "setpoint"

[commands.set_setpoint]
template = "SETP ${value}"
parameters = { value = "float" }
expects_response = false

[responses.setpoint]
pattern = "^SETP\\s+(?P<value>[+-]?\\d+\\.?\\d*)$"

[responses.setpoint.fields.value]
type = "float"
```

### Pattern 3: RS-485 Multidrop Bus

Multiple devices sharing one serial port:

```toml
[connection]
type = "serial"
baud_rate = 9600

[connection.bus]
type = "rs485"
address_format = "hex_char"           # 0-9, A-F
default_address = "0"

[commands.get_position]
template = "${address}gp"             # Address prefix in command
response = "position"
```

### Pattern 4: Error Handling

```toml
[error_codes."ERR01"]
name = "CommunicationError"
description = "Communication timeout"
recoverable = true
severity = "warning"

[error_codes."ERR02"]
name = "OverTemperature"
description = "Device overheated"
recoverable = false
severity = "critical"

[error_codes."ERR02".recovery_action]
auto_recover = false
manual_instructions = "Allow device to cool for 5 minutes"
```

### Pattern 5: Initialization Sequence

Verify device identity and state on connect:

```toml
[[init_sequence]]
command = "get_id"
description = "Verify device identity"
required = true
expect = "ACME PM-100"               # Response must contain this

[[init_sequence]]
command = "get_status"
description = "Check device is ready"
required = true
delay_ms = 100                        # Wait before next command
```

### Pattern 6: Retry Configuration

For unreliable communication:

```toml
[default_retry]
max_retries = 3
initial_delay_ms = 100
max_delay_ms = 2000
backoff_multiplier = 2.0

# Per-command override
[commands.slow_operation]
template = "SLOW"
timeout_ms = 10000

[commands.slow_operation.retry]
max_retries = 5
initial_delay_ms = 500
```

---

## Testing Your Config

### 1. Validate TOML Syntax

```bash
# Quick syntax check with Python
python3 -c "import tomllib; tomllib.load(open('config/devices/my_device.toml', 'rb')); print('✅ Valid TOML')"
```

### 2. Test with Real Hardware

Create a simple test script:

```rust
use daq_driver_generic::{DriverFactory, load_device_config};
use std::path::Path;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load config
    let config = load_device_config(Path::new("config/devices/my_device.toml"))?;
    println!("Loaded config for: {}", config.device.name);
    
    // Create driver (requires actual hardware)
    // let driver = DriverFactory::create_from_config(config, port, address).await?;
    
    Ok(())
}
```

### 3. Test Commands Manually

Use a serial terminal (like `minicom`, `screen`, or `picocom`) to test commands before adding them to your config:

```bash
# Connect to device
picocom -b 19200 /dev/ttyUSB0

# Send commands manually and observe responses
*IDN?
MEAS:POW?
```

---

## Troubleshooting

### "Response pattern didn't match"

**Problem**: Your regex pattern doesn't match the actual device response.

**Solutions**:
1. Test your regex at [regex101.com](https://regex101.com/) with actual responses
2. Add debug logging to see raw responses
3. Check for hidden characters (CR, LF, null bytes)
4. Escape special regex characters: `.` → `\\.`, `?` → `\\?`

### "Communication timeout"

**Problem**: Device isn't responding within timeout.

**Solutions**:
1. Increase `timeout_ms` in connection settings
2. Verify baud rate and other serial settings
3. Check cable connections
4. Try sending commands manually with a serial terminal

### "Wrong device at address"

**Problem**: RS-485 address mismatch.

**Solutions**:
1. Verify device address in hardware (DIP switches, configuration)
2. Check `address_format` matches device protocol
3. Use `init_sequence` to validate device identity on connect

### "Invalid command template"

**Problem**: Template syntax error.

**Solutions**:
1. Check parameter names match between template and `parameters` table
2. Verify format specifiers are valid (`08X`, not `8X`)
3. Escape `$` if needed in literal text

### "Conversion formula error"

**Problem**: evalexpr formula fails.

**Solutions**:
1. Use parentheses to clarify order of operations
2. Ensure variable names match parameter names exactly
3. Test formula with known values

---

## Reference Examples

| Device Type | Example Config | Key Features |
|-------------|----------------|--------------|
| Rotation mount | `ell14.toml` | RS-485 multidrop, hex commands |
| Motion controller | `esp300.toml` | Multi-axis, SCPI-like |
| Power meter | `newport_1830c.toml` | Simple ASCII, Readable trait |
| Temperature controller | `sample_temperature_controller.toml` | All features demonstrated |

---

## See Also

- [Hardware Driver Development Guide](./hardware-drivers.md) - For native Rust drivers
- [Plugin System](../plugins/README.md) - Distributing your configs as plugins
- [Scripting Guide](./scripting.md) - Using your device from Rhai scripts
