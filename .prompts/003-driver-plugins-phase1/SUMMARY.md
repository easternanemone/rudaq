# Phase 1: Declarative Driver Plugin System - Core Infrastructure

## Overview

This phase implements the foundational TOML-based device configuration schema and loader for the declarative driver plugin system. The goal is to enable config-driven hardware drivers that don't require code changes for new device protocols.

## Completed Work

### 1. Config Module Structure

Created `crates/daq-hardware/src/config/` with:

- `mod.rs` - Module root with re-exports and JSON schema generation
- `schema.rs` - Rust types for device protocol definitions (~700 lines)
- `validation.rs` - Custom validators for regex, evalexpr formulas, cross-field validation
- `loader.rs` - TOML config loading with Figment, validation, and error handling

### 2. Schema Structs

All schema types derive `Debug, Clone, Serialize, Deserialize, JsonSchema, Validate`:

| Struct | Purpose |
|--------|---------|
| `DeviceConfig` | Top-level config with all sections |
| `DeviceIdentity` | Device name, manufacturer, protocol, capabilities |
| `ConnectionConfig` | Serial/network settings (baud, parity, timeout) |
| `CommandConfig` | Command templates with parameter placeholders |
| `ResponseConfig` | Response parsing (regex, delimiter, fixed-width) |
| `ConversionConfig` | Unit conversion formulas (evalexpr syntax) |
| `ParameterConfig` | Device-specific parameters with types/ranges |
| `TraitMappingConfig` | Maps capability traits to commands |
| `ErrorCodeConfig` | Error code definitions |
| `ValidationRuleConfig` | Parameter validation rules |

Supporting enums: `DeviceCategory`, `CapabilityType`, `ConnectionType`, `FieldType`, `ParitySetting`, `FlowControlSetting`, `BusType`, `AddressFormat`, etc.

### 3. Validation

**Declarative validation (serde_valid):**
- Range constraints: `baud_rate` (300-921600), `timeout_ms` (1-60000)
- String length constraints: `name`, `description`, `protocol`
- Nested struct validation via `#[validate]` attribute

**Custom validators:**
- `validate_regex_pattern()` - Validates regex syntax
- `validate_evalexpr_formula()` - Validates formula syntax via `evalexpr::build_operator_tree()`
- `validate_device_config()` - Cross-field validation:
  - Command references to responses exist
  - Trait mapping references to commands/conversions exist
  - Parameter ranges are valid (min <= max)

### 4. Config Loader

```rust
// Load single file
let config = load_device_config(Path::new("config/devices/ell14.toml"))?;

// Load from string (for testing)
let config = load_device_config_from_str(toml_content)?;

// Load all configs from directory
let configs = load_all_devices(Path::new("config/devices/"))?;
```

Error types: `ConfigLoadError::NotFound`, `ReadError`, `ParseError`, `ValidationError`, `SchemaValidationError`

### 5. JSON Schema Generation

```rust
// Generate schema string
let schema_json = daq_hardware::config::generate_json_schema()?;

// Write to file (via ignored test)
// cargo test -p daq-hardware write_json_schema_file -- --ignored
```

Output: `config/schemas/device.schema.json` (JSON Schema draft-07)

### 6. Dependencies Added

```toml
# crates/daq-hardware/Cargo.toml
serde_valid = "0.24"     # Declarative validation
schemars = "0.8"         # JSON Schema generation
evalexpr = "11"          # Formula validation
figment = { version = "0.10", features = ["toml"] }  # Config loading
```

## Test Coverage

27 config-related tests covering:
- Minimal config parsing
- Full ELL14 config with all sections
- Invalid regex rejection
- Invalid formula rejection
- Out-of-range baud rate rejection (2000000 > 921600)
- Out-of-range timeout rejection (100000 > 60000)
- Missing required fields rejection
- Command referencing non-existent response
- Default values
- JSON schema generation

## Files Created/Modified

```
crates/daq-hardware/
├── Cargo.toml                    # +4 dependencies
└── src/
    ├── lib.rs                    # +2 lines (module + re-export)
    └── config/
        ├── mod.rs                # 170 lines
        ├── schema.rs             # 728 lines
        ├── validation.rs         # 341 lines
        └── loader.rs             # 495 lines

config/
├── schemas/
│   └── device.schema.json        # Generated JSON schema
└── devices/
    └── .gitkeep                  # Placeholder for device configs
```

## Usage Example

```toml
# config/devices/ell14.toml
[device]
name = "Thorlabs ELL14"
manufacturer = "Thorlabs"
model = "ELL14"
protocol = "elliptec"
category = "stage"
capabilities = ["Movable", "Parameterized"]

[connection]
type = "serial"
baud_rate = 9600
timeout_ms = 1000
terminator_rx = "\r\n"

[connection.bus]
type = "rs485"
address_format = "hex_char"
default_address = "0"

[parameters.pulses_per_degree]
type = "float"
default = 398.2222
description = "Calibration factor"

[commands.move_absolute]
template = "${address}ma${position_pulses:08X}"
parameters = { position_pulses = "int32" }

[commands.get_position]
template = "${address}gp"
response = "position"

[responses.position]
pattern = "^(?P<addr>[0-9A-Fa-f])PO(?P<pulses>[0-9A-Fa-f]{8})$"

[responses.position.fields.pulses]
type = "hex_i32"
signed = true

[conversions.degrees_to_pulses]
formula = "round(degrees * pulses_per_degree)"

[trait_mapping.Movable.move_abs]
command = "move_absolute"
input_conversion = "degrees_to_pulses"
input_param = "position_pulses"
from_param = "position"
```

## Verification

```bash
# Build
cargo build -p daq-hardware  # ✓ Passes

# Lint
cargo clippy -p daq-hardware --all-targets  # ✓ No warnings

# Tests
cargo test -p daq-hardware --lib  # ✓ 87 passed

# Config-specific tests
cargo test -p daq-hardware config  # ✓ 27 passed

# Generate schema
cargo test -p daq-hardware write_json_schema_file -- --ignored
```

## Next Steps (Phase 2)

1. **Config Interpreter**: Execute commands from config templates
2. **Response Parser**: Parse responses using patterns from config
3. **Trait Implementation**: Generate trait impls from trait_mapping
4. **Hot Reload**: Watch config files for changes

## Design Decisions

1. **serde_valid over validator**: Better integration with serde, JSON Schema alignment
2. **Figment for loading**: Already used in project, supports TOML + env overlays
3. **evalexpr for formulas**: Pure Rust, no external dependencies, sufficient for unit conversions
4. **Separate validation module**: Custom validators reusable outside schema context
5. **Nested #[validate]**: Required for serde_valid to recursively validate nested structs
