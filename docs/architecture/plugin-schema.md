# Instrument Configuration Schema

**Declarative driver specification for rust-daq**

## Overview

This schema defines the structure of `InstrumentConfig` TOML files used by the `GenericSerialDriver`. It allows adding support for new serial (RS-232, RS-485, USB-Serial) and network (TCP/UDP) instruments purely through configuration, without writing Rust code.

The schema maps high-level **Capability Traits** (like `Movable`, `Readable`, `ShutterControl`) to low-level **Commands** and **Responses**.

## File Structure

A complete instrument configuration consists of the following sections:

```toml
[device]           # Identity, protocol type, capabilities
[connection]       # Baud rate, timeout, bus settings
[parameters]       # Device variables (position, speed)
[commands]         # Command templates
[responses]        # Response parsing (regex/binary)
[conversions]      # Unit conversion formulas
[trait_mapping]    # Maps traits (Movable) to commands
[error_codes]      # Error code definitions
```

---

## 1. Device Identity `[device]`

Defines what the device is and what it can do.

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | String | **Yes** | Human-readable name (e.g., "Thorlabs ELL14") |
| `protocol` | String | **Yes** | Protocol identifier (e.g., "elliptec", "scpi") |
| `capabilities` | List<String> | No | Traits implemented: `Movable`, `Readable`, `WavelengthTunable`, `ShutterControl` |
| `description` | String | No | Detailed description |
| `manufacturer` | String | No | Manufacturer name |
| `model` | String | No | Model number |

## 2. Connection Settings `[connection]`

Defines how to talk to the device.

| Field | Type | Default | Description |
|---|---|---|---|
| `type` | String | "serial" | `serial`, `tcp`, `udp` |
| `baud_rate` | Integer | 9600 | Serial baud rate (300-921600) |
| `data_bits` | Integer | 8 | 5, 6, 7, 8 |
| `stop_bits` | Integer | 1 | 1, 2 |
| `parity` | String | "none" | `none`, `odd`, `even` |
| `flow_control` | String | "none" | `none`, `software`, `hardware` |
| `terminator_tx` | String | "" | String appended to every sent command |
| `terminator_rx` | String | "\r\n" | String that marks end of response |
| `timeout_ms` | Integer | 1000 | Read/Write timeout |

## 3. Parameters `[parameters]`

Defines state variables. These can be inputs for commands or outputs from responses.

```toml
[parameters.position_deg]
type = "float"      # string, int, float, bool
default = 0.0
range = [0.0, 360.0]
unit = "degrees"
description = "Rotation position"
```

## 4. Commands `[commands]`

Defines templates for sending data. Supports parameter interpolation.

```toml
[commands.move_absolute]
template = "${address}ma${position_pulses:08X}"
description = "Move to absolute position"
expects_response = true # Default true
timeout_ms = 5000       # Override connection timeout
```

**Interpolation Syntax:**
- `${param}`: Insert value as string.
- `${param:08X}`: Format as 8-digit uppercase Hex.
- `${param:d}`: Format as decimal.

## 5. Responses `[responses]`

Defines how to parse incoming data using Regex.

```toml
[responses.position]
pattern = "^((?P<addr>[0-9A-Fa-f])PO(?P<pulses>[0-9A-Fa-f]{8}))$"

[responses.position.fields.pulses]
type = "hex_i32" # hex_u32, int, float, string
signed = true
```

## 6. Conversions `[conversions]`

Formulas for unit conversion using `evalexpr`.

```toml
[conversions.degrees_to_pulses]
formula = "round(degrees * pulses_per_degree)"
```

## 7. Trait Mapping `[trait_mapping]`

The glue that binds capabilities to commands.

```toml
[trait_mapping.Movable.move_abs]
command = "move_absolute"
input_conversion = "degrees_to_pulses"
input_param = "position_pulses" # Parameter name in command template
from_param = "degrees"          # Variable name in conversion formula
```

## 8. Error Codes `[error_codes]`

Map device error strings/codes to semantic errors.

```toml
[error_codes.BUSY]
name = "Device Busy"
description = "The device is currently moving"
severity = "warning"
recoverable = true
```

```