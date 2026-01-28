# Driver Plugins Implementation Plan

<metadata>
<date>2026-01-08</date>
<planner>Claude Opus 4.5</planner>
<project>rust-daq</project>
<based_on>.prompts/001-driver-plugins-research/driver-plugins-research.md</based_on>
<confidence>high</confidence>
</metadata>

<summary>
This plan establishes a phased implementation of a declarative, config-driven hardware driver plugin system for rust-daq. The architecture combines TOML-based protocol definitions with compile-time code generation using enum_dispatch for zero-overhead trait dispatch. Starting with a GenericSerialDriver that interprets TOML configs at runtime, the system evolves to support state machines via smlang and optional scripted extensions via Rhai. The ELL14 driver serves as the migration proof-of-concept in Phase 2, with ESP300 following to validate the pattern works across different protocol styles. By Phase 4, users can add new serial instruments by creating a TOML file without writing any Rust code.
</summary>

<architecture>

## Target Architecture

```
                        ┌─────────────────────────────────────┐
                        │          User Config Files          │
                        │    config/devices/*.toml            │
                        └──────────────┬──────────────────────┘
                                       │
                        ┌──────────────▼──────────────────────┐
                        │       Config Loader (Figment)       │
                        │  - Schema validation (serde_valid)  │
                        │  - JSON Schema export (schemars)    │
                        └──────────────┬──────────────────────┘
                                       │
            ┌──────────────────────────┼──────────────────────────┐
            │                          │                          │
            ▼                          ▼                          ▼
   ┌────────────────┐       ┌────────────────┐        ┌────────────────┐
   │  Built-in      │       │  Generic       │        │  Specialized   │
   │  Drivers       │       │  Serial        │        │  Drivers       │
   │  (enum_dispatch)│       │  Driver        │        │  (hand-coded)  │
   │                │       │                │        │                │
   │  - Ell14Config │       │  Interprets    │        │  - PVCAM       │
   │  - Esp300Config│       │  TOML at       │        │  - Comedi      │
   │  - MaiTaiConfig│       │  runtime       │        │                │
   └───────┬────────┘       └───────┬────────┘        └───────┬────────┘
           │                        │                         │
           └────────────────────────┼─────────────────────────┘
                                    │
                        ┌───────────▼────────────┐
                        │   Capability Traits    │
                        │   (existing HAL)       │
                        │                        │
                        │   - Movable            │
                        │   - Readable           │
                        │   - WavelengthTunable  │
                        │   - ShutterControl     │
                        │   - Parameterized      │
                        └────────────────────────┘
```

## Key Components

### 1. Device Config Schema (`DeviceConfig`)
TOML structure defining device identity, connection settings, commands, responses, and conversions.

### 2. GenericSerialDriver
Runtime interpreter for TOML configs. Implements capability traits by:
- Formatting commands from templates
- Parsing responses via regex/delimiter patterns
- Applying unit conversions

### 3. ConfiguredDriver Enum (enum_dispatch)
Static dispatch enum wrapping all config-driven drivers for zero-overhead polymorphism.

### 4. DriverFactory
Creates drivers from config files, returning the appropriate variant.

### 5. State Machine Support (Phase 3+)
Config-defined initialization sequences compiled via smlang.

</architecture>

<phases>

## Phase 1: Core Infrastructure (Foundation)

**Objective:** Establish config schema, validation, and basic config loading without changing any existing drivers.

**Duration:** 1 sprint (1-2 weeks)

### Tasks

1. **Define DeviceConfig schema structs** (`crates/daq-hardware/src/config/mod.rs`)
   - `DeviceIdentity`: name, type, protocol, capabilities list
   - `ConnectionConfig`: serial settings (baud, parity, stop bits, flow control, timeout)
   - `CommandConfig`: template, parameters, description
   - `ResponseConfig`: pattern (regex), fields, delimiter mode, fixed-position mode
   - `ConversionConfig`: formula expressions for unit conversion
   - `ValidationConfig`: parameter ranges, units

2. **Implement config validation with serde_valid**
   - Add `#[validate]` attributes to all config structs
   - Range validation for baud rates, timeouts
   - Pattern validation for addresses, command templates
   - Custom validator for response patterns (valid regex check)

3. **Generate JSON Schema via schemars**
   - Derive `JsonSchema` on all config structs
   - Export schema to `config/schemas/device.schema.json`
   - Document schema for IDE completion support

4. **Create config loader module** (`crates/daq-hardware/src/config/loader.rs`)
   - Load single device config: `load_device_config(path: &Path) -> Result<DeviceConfig>`
   - Load all device configs from directory: `load_all_devices(dir: &Path) -> Result<Vec<DeviceConfig>>`
   - Validate after loading

5. **Add Cargo dependencies**
   - `serde_valid` for validation
   - `schemars` for JSON schema generation
   - `regex` (already used)
   - Keep `figment` (already used)

### Deliverables
- `crates/daq-hardware/src/config/mod.rs` - Config structs
- `crates/daq-hardware/src/config/loader.rs` - Config loading
- `crates/daq-hardware/src/config/validation.rs` - Custom validators
- `config/schemas/device.schema.json` - Generated schema
- Unit tests for config parsing and validation

### Dependencies
- None (greenfield module)

### Verification
- [ ] Config structs deserialize example TOML files correctly
- [ ] Invalid configs produce clear error messages with field paths
- [ ] JSON schema validates against example configs
- [ ] `cargo test -p daq-hardware` passes

---

## Phase 2: GenericSerialDriver + ELL14 Migration (MVP)

**Objective:** Create working GenericSerialDriver that can replace ELL14 for basic movement commands. Prove the pattern works end-to-end.

**Duration:** 1-2 sprints (2-4 weeks)

### Tasks

1. **Create ELL14 TOML config file** (`config/devices/ell14.toml`)
   - Full protocol definition (see TOML Schema section)
   - All commands: ma, mr, gp, gs, ho, fw, bw, sj, gj, in
   - Response patterns for position, status, device info
   - Calibration conversion formulas

2. **Implement GenericSerialDriver** (`crates/daq-hardware/src/drivers/generic_serial.rs`)
   - Constructor from `DeviceConfig`
   - Serial port management (shared port support for RS-485)
   - Command formatting with template interpolation
   - Response parsing (regex, delimiter, fixed modes)
   - Unit conversion engine
   - Error handling with device-specific error codes

3. **Implement capability traits for GenericSerialDriver**
   - `Movable` - maps to move_absolute, move_relative, get_position commands
   - `Parameterized` - expose all device parameters
   - Capability trait selection based on config's `capabilities` list

4. **Create ConfiguredDriver enum with enum_dispatch**
   ```rust
   #[enum_dispatch(Movable, Readable, Parameterized)]
   pub enum ConfiguredDriver {
       Ell14(GenericSerialDriver),
       Generic(GenericSerialDriver),
   }
   ```

5. **Implement DriverFactory** (`crates/daq-hardware/src/factory.rs`)
   - `create_driver(config: &DeviceConfig) -> Result<ConfiguredDriver>`
   - `create_driver_from_file(path: &Path) -> Result<ConfiguredDriver>`

6. **Create migration test for ELL14**
   - Compare behavior: existing Ell14Driver vs GenericSerialDriver with ell14.toml
   - Mock serial port tests for command/response sequences
   - Integration test on real hardware (maitai remote)

### Deliverables
- `config/devices/ell14.toml` - Complete ELL14 protocol definition
- `crates/daq-hardware/src/drivers/generic_serial.rs` - Generic driver
- `crates/daq-hardware/src/factory.rs` - Driver factory
- Integration tests comparing ELL14 implementations
- Documentation: migration guide section

### Dependencies
- Phase 1 complete (config infrastructure)

### Verification
- [ ] GenericSerialDriver with ell14.toml passes all existing ELL14 tests
- [ ] `move_abs(45.0)` produces identical serial output to current driver
- [ ] Response parsing extracts correct position values
- [ ] Unit conversion (degrees <-> pulses) is accurate
- [ ] RS-485 multidrop works (shared port with multiple addresses)
- [ ] Remote hardware test passes on maitai machine

---

## Phase 3: ESP300 + Additional Capabilities (Pattern Validation)

**Objective:** Validate the pattern generalizes by adding ESP300, which has different protocol characteristics. Add more capability traits.

**Duration:** 1 sprint (1-2 weeks)

### Tasks

1. **Create ESP300 TOML config file** (`config/devices/esp300.toml`)
   - Different protocol style: `{axis}PA{position}` format
   - Query responses with different parsing
   - Multi-axis support via config

2. **Extend GenericSerialDriver for ESP300 patterns**
   - Axis parameter in command templates
   - Different response terminator handling
   - Velocity/acceleration commands

3. **Add more capability trait implementations**
   - `Readable` - for devices with scalar readout
   - `WavelengthTunable` - for tunable sources
   - `ShutterControl` - for shutter-equipped devices

4. **Create Newport1830C and MaiTai config files**
   - Prove pattern works for Readable devices
   - Prove pattern works for wavelength-tunable devices

5. **Update enum_dispatch configuration**
   ```rust
   #[enum_dispatch(Movable, Readable, WavelengthTunable, ShutterControl, Parameterized)]
   pub enum ConfiguredDriver {
       Ell14(GenericSerialDriver),
       Esp300(GenericSerialDriver),
       Newport1830C(GenericSerialDriver),
       MaiTai(GenericSerialDriver),
       Generic(GenericSerialDriver),
   }
   ```

6. **Add state machine placeholders**
   - Document which commands require sequencing
   - Identify initialization sequences in existing drivers

### Deliverables
- `config/devices/esp300.toml` - ESP300 protocol definition
- `config/devices/newport_1830c.toml` - Power meter definition
- `config/devices/maitai.toml` - Laser definition
- Extended capability trait implementations
- Comparison tests for all four drivers

### Dependencies
- Phase 2 complete (GenericSerialDriver working)

### Verification
- [ ] ESP300 commands match existing driver output
- [ ] Newport1830C reads power correctly via config
- [ ] MaiTai wavelength control works via config
- [ ] All existing hardware tests pass with config-based drivers
- [ ] No performance regression (benchmark enum_dispatch vs direct)

---

## Phase 4: State Machines + Initialization Sequences

**Objective:** Add config-driven state machine support for devices with complex initialization or multi-step operations.

**Duration:** 1-2 sprints (2-4 weeks)

### Tasks

1. **Design state machine config schema**
   ```toml
   [state_machine]
   name = "DeviceState"
   initial = "Disconnected"

   [state_machine.states.Disconnected]
   transitions = [
       { event = "Connect", target = "Initializing", action = "open_port" }
   ]

   [state_machine.states.Initializing]
   on_enter = "run_init_sequence"
   transitions = [
       { event = "InitComplete", target = "Ready" },
       { event = "InitFailed", target = "Error" }
   ]
   ```

2. **Implement state machine code generation**
   - Build script to generate smlang macro invocations from config
   - Or: runtime state machine interpreter (simpler but less performant)

3. **Add initialization sequence support**
   ```toml
   [init_sequence]
   steps = [
       { command = "get_info", validate = "response_ok" },
       { command = "home", wait_ms = 5000 },
       { command = "get_status", validate = "status_ready" }
   ]
   ```

4. **Implement retry/recovery patterns**
   - Config-driven retry policies per command
   - Error recovery sequences

5. **Add async sequence execution**
   - `run_init_sequence() -> Result<()>`
   - Step-by-step execution with validation

### Deliverables
- State machine config schema extension
- Initialization sequence support
- Updated ELL14/ESP300 configs with init sequences
- Tests for complex device initialization

### Dependencies
- Phase 3 complete (multiple devices working)

### Verification
- [ ] ELL14 motor optimization sequence works via config
- [ ] Devices recover from error states
- [ ] Init sequences run on first connection
- [ ] Timeout/retry behavior is config-driven

---

## Phase 5: Scripted Extensions (Rhai Fallback)

**Objective:** Add Rhai scripting support for edge cases where declarative config isn't sufficient.

**Duration:** 1 sprint (1-2 weeks)

### Tasks

1. **Define Rhai extension points**
   ```toml
   [scripts]
   custom_response_parser = "scripts/ell14_parse.rhai"
   custom_conversion = "scripts/ell14_convert.rhai"
   ```

2. **Create Rhai API for scripts**
   - `serial_write(cmd: String)`
   - `serial_read_until(terminator: String) -> String`
   - `parse_hex(s: String) -> i64`
   - `parse_float(s: String) -> f64`

3. **Implement script sandbox**
   - Resource limits (memory, execution time)
   - No filesystem access
   - No network access

4. **Add script loading and execution**
   - Cache compiled scripts
   - Hot-reload support (optional, via feature flag)

### Deliverables
- Rhai integration module
- Example scripts for complex parsing
- Script API documentation
- Security documentation

### Dependencies
- Phase 4 complete (state machines working)
- `daq-scripting` crate (already exists)

### Verification
- [ ] Complex response parsing works via Rhai
- [ ] Scripts cannot access filesystem
- [ ] Script errors produce clear messages
- [ ] Performance acceptable for typical use cases

---

## Phase 6: Binary Protocols (Future)

**Objective:** Extend system to support binary protocols (Modbus, custom framing).

**Duration:** 2 sprints (3-4 weeks)

### Tasks

1. **Design binary command config schema**
   ```toml
   [binary_commands.read_register]
   frame = [
       { type = "u8", value = "${address}" },
       { type = "u8", value = "0x03" },  # Function code
       { type = "u16be", value = "${register}" },
       { type = "u16be", value = "${count}" },
       { type = "crc16_modbus" }
   ]
   ```

2. **Implement binary frame builder**
3. **Add Modbus protocol support**
4. **Create example Modbus device config**

### Deliverables
- Binary protocol config schema
- Frame builder implementation
- Modbus device example
- Documentation for binary protocols

### Dependencies
- Phase 3+ complete

### Verification
- [ ] Modbus commands match reference implementation
- [ ] CRC calculation correct
- [ ] Big/little endian handling correct

</phases>

<toml_schema>

## Complete TOML Schema Specification

### Example: Full ELL14 Configuration

```toml
# config/devices/ell14.toml
# Thorlabs ELL14 Rotation Mount - TOML Protocol Definition

[device]
name = "Thorlabs ELL14"
description = "Elliptec rotation mount with RS-485 multidrop support"
manufacturer = "Thorlabs"
model = "ELL14"
protocol = "elliptec"
category = "Stage"  # Maps to DeviceCategory::Stage

# Capabilities this device implements (used for trait dispatch)
capabilities = ["Movable", "Parameterized"]

[connection]
type = "serial"
baud_rate = 9600
data_bits = 8
parity = "none"       # "none", "odd", "even"
stop_bits = 1         # 1 or 2
flow_control = "none" # "none", "software", "hardware"
timeout_ms = 1000
terminator_tx = ""    # No terminator for ELL14
terminator_rx = "\r\n"

# RS-485 multidrop configuration
[connection.bus]
type = "rs485"
address_format = "hex_char"  # "0"-"9", "A"-"F"
default_address = "0"

# Device-specific parameters
[parameters]
address = { type = "string", default = "0", description = "Device address on RS-485 bus (0-F)" }
pulses_per_degree = { type = "float", default = 398.2222, description = "Calibration factor from device" }
position_deg = { type = "float", default = 0.0, range = [0.0, 360.0], unit = "degrees", description = "Current position" }
jog_step_deg = { type = "float", default = 5.0, range = [0.001, 360.0], unit = "degrees", description = "Jog step size" }

# Command definitions
[commands]
# Movement commands
move_absolute = {
    template = "${address}ma${position_pulses:08X}",
    description = "Move to absolute position",
    parameters = { position_pulses = "int32" }
}

move_relative = {
    template = "${address}mr${distance_pulses:08X}",
    description = "Move relative distance",
    parameters = { distance_pulses = "int32" }
}

home = {
    template = "${address}ho0",
    description = "Home to mechanical zero (direction 0)"
}

stop = {
    template = "${address}st",
    description = "Stop motion immediately"
}

# Query commands
get_position = {
    template = "${address}gp",
    description = "Query current position",
    response = "position"
}

get_status = {
    template = "${address}gs",
    description = "Query device status",
    response = "status"
}

get_info = {
    template = "${address}in",
    description = "Query device information",
    response = "device_info"
}

# Jog commands
jog_forward = { template = "${address}fw", description = "Jog forward" }
jog_backward = { template = "${address}bw", description = "Jog backward" }
set_jog_step = {
    template = "${address}sj${step_pulses:08X}",
    parameters = { step_pulses = "int32" }
}
get_jog_step = {
    template = "${address}gj",
    response = "jog_step"
}

# Response definitions
[responses]
# Position response: 0PO00001234 (address + "PO" + 8 hex digits)
position = {
    pattern = "^(?P<addr>[0-9A-Fa-f])PO(?P<pulses>[0-9A-Fa-f]{8})$",
    fields = {
        addr = { type = "string" },
        pulses = { type = "hex_i32", signed = true }
    }
}

# Status response: 0GS00 (address + "GS" + 2 hex digits)
status = {
    pattern = "^(?P<addr>[0-9A-Fa-f])GS(?P<code>[0-9A-Fa-f]{2})$",
    fields = {
        addr = { type = "string" },
        code = { type = "hex_u8" }
    }
}

# Device info response (30 or 33 chars depending on firmware)
# Format: 0IN{type:2}{serial:8}{year:4}{fw:2}{hw?:3}{travel:4}{pulses:4}
device_info = {
    pattern = "^(?P<addr>[0-9A-Fa-f])IN(?P<type>[0-9]{2})(?P<serial>[0-9A-Za-z]{8})(?P<year>[0-9]{4})(?P<fw>[0-9]{2})(?P<rest>.+)$",
    fields = {
        addr = { type = "string" },
        type = { type = "string" },
        serial = { type = "string" },
        year = { type = "int" },
        fw = { type = "string" },
        rest = { type = "string" }  # Variable length suffix
    }
}

# Jog step response
jog_step = {
    pattern = "^(?P<addr>[0-9A-Fa-f])GJ(?P<pulses>[0-9A-Fa-f]{8})$",
    fields = {
        pulses = { type = "hex_i32", signed = true }
    }
}

# Error response
error = {
    pattern = "^(?P<addr>[0-9A-Fa-f])GS(?P<code>[0-9A-Fa-f]{2})$",
    fields = {
        code = { type = "hex_u8" }
    }
}

# Unit conversions
[conversions]
# Position: degrees <-> pulses
degrees_to_pulses = "round(degrees * pulses_per_degree)"
pulses_to_degrees = "pulses / pulses_per_degree"

# Error code mapping
[error_codes]
0x00 = { name = "OK", description = "No error" }
0x01 = { name = "CommunicationTimeout", description = "Communication timeout" }
0x02 = { name = "MechanicalTimeout", description = "Motor didn't reach target" }
0x03 = { name = "CommandError", description = "Invalid command" }
0x04 = { name = "ValueOutOfRange", description = "Value out of range" }
0x05 = { name = "ModuleIsolated", description = "Module isolated" }
0x06 = { name = "ModuleOutOfIsolation", description = "Module out of isolation" }
0x07 = { name = "InitializationError", description = "Initialization error" }
0x08 = { name = "ThermalError", description = "Overtemperature" }
0x09 = { name = "Busy", description = "Device busy" }
0x0A = { name = "SensorError", description = "Sensor error" }
0x0B = { name = "MotorError", description = "Motor error" }
0x0C = { name = "OutOfRange", description = "Position out of range" }
0x0D = { name = "OverCurrentError", description = "Over current" }

# Validation rules
[validation]
position_deg = { range = [0.0, 360.0], unit = "degrees" }
jog_step_deg = { range = [0.001, 360.0], unit = "degrees" }
address = { pattern = "^[0-9A-Fa-f]$" }

# Trait mapping: how commands map to capability trait methods
[trait_mapping.Movable]
move_abs = {
    command = "move_absolute",
    input_conversion = "degrees_to_pulses",
    input_param = "position_pulses",
    from_param = "position"
}
move_rel = {
    command = "move_relative",
    input_conversion = "degrees_to_pulses",
    input_param = "distance_pulses",
    from_param = "distance"
}
position = {
    command = "get_position",
    output_conversion = "pulses_to_degrees",
    output_field = "pulses"
}
wait_settled = {
    poll_command = "get_status",
    success_condition = "code == 0",
    poll_interval_ms = 50,
    timeout_ms = 30000
}
stop = { command = "stop" }
```

### Schema Structure Reference

```toml
# Top-level sections
[device]           # Device identity and metadata
[connection]       # Communication settings
[parameters]       # Device-specific parameters
[commands]         # Command definitions
[responses]        # Response parsing definitions
[conversions]      # Unit conversion formulas
[error_codes]      # Error code mapping (optional)
[validation]       # Validation rules
[trait_mapping]    # Maps trait methods to commands
[state_machine]    # State machine definition (Phase 4+)
[init_sequence]    # Initialization sequence (Phase 4+)
[scripts]          # Rhai script references (Phase 5+)
```

### Field Type Reference

**Parameter Types:**
- `string` - Text value
- `int` / `int32` / `int64` - Signed integers
- `uint` / `uint32` / `uint64` - Unsigned integers
- `float` / `float64` - Floating point
- `bool` - Boolean
- `hex_i32` - 32-bit signed integer from hex string
- `hex_u8` / `hex_u16` / `hex_u32` - Unsigned integers from hex

**Response Pattern Types:**
- `pattern` - Regex with named capture groups
- `delimiter` - Delimiter-separated fields
- `fixed` - Fixed-position fields

**Conversion Formulas:**
Simple expression language supporting:
- Arithmetic: `+`, `-`, `*`, `/`, `%`
- Functions: `round()`, `floor()`, `ceil()`, `abs()`
- Variables: Any parameter name from `[parameters]`

</toml_schema>

<migration_guide>

## ELL14 Migration Guide

### Step 1: Create Config File

Create `config/devices/ell14.toml` with the full protocol definition (see TOML Schema section above).

### Step 2: Update Feature Flags

In `crates/daq-hardware/Cargo.toml`:

```toml
[features]
driver-thorlabs = ["dep:regex"]
driver-thorlabs-config = ["driver-thorlabs"]  # Config-based ELL14

# New default includes config-based driver
default = ["serial", "driver-thorlabs-config"]
```

### Step 3: Create Compatibility Shim

In `crates/daq-hardware/src/drivers/ell14.rs`, add at the bottom:

```rust
// Compatibility: Config-based driver
#[cfg(feature = "driver-thorlabs-config")]
pub mod config_based {
    use crate::config::loader::load_device_config;
    use crate::drivers::generic_serial::GenericSerialDriver;
    use std::path::Path;

    /// Create ELL14 driver from TOML config
    pub async fn from_config(
        config_path: &Path,
        port_path: &str,
        address: &str,
    ) -> anyhow::Result<GenericSerialDriver> {
        let mut config = load_device_config(config_path)?;
        config.connection.port_path = Some(port_path.to_string());
        config.parameters.get_mut("address")
            .map(|p| p.default = serde_json::Value::String(address.to_string()));
        GenericSerialDriver::new(config).await
    }
}
```

### Step 4: Run Comparison Tests

Create `crates/daq-hardware/tests/ell14_migration.rs`:

```rust
//! Migration tests: Compare existing ELL14 driver with config-based version

use daq_hardware::drivers::ell14::{Ell14Bus, Ell14Driver};
use daq_hardware::drivers::generic_serial::GenericSerialDriver;
use daq_hardware::capabilities::Movable;
use std::path::Path;

/// Mock serial port for testing
struct MockSerial { /* ... */ }

#[tokio::test]
async fn test_move_abs_command_identical() {
    let mock = MockSerial::new();

    // Existing driver
    let existing = Ell14Driver::with_shared_port(mock.clone(), "2");

    // Config-based driver
    let config_based = GenericSerialDriver::from_config(
        Path::new("config/devices/ell14.toml"),
        mock.clone(),
        "2"
    ).await.unwrap();

    // Both should produce identical commands
    existing.move_abs(45.0).await.unwrap();
    let existing_cmd = mock.last_write();

    mock.reset();

    config_based.move_abs(45.0).await.unwrap();
    let config_cmd = mock.last_write();

    assert_eq!(existing_cmd, config_cmd, "Commands should be identical");
}

#[tokio::test]
async fn test_response_parsing_identical() {
    // Test that both parse "2PO00004650" to the same position
    let response = "2PO00004650";
    let expected_position = 45.0; // 0x4650 = 17488 pulses / 398.22 ≈ 43.9°

    // ... compare parsing results
}
```

### Step 5: Hardware Validation

On maitai remote machine:

```bash
# Test existing driver
cargo test --features hardware_tests test_ell14 -- --nocapture

# Test config-based driver
cargo test --features hardware_tests,driver-thorlabs-config test_ell14_config -- --nocapture
```

### Step 6: Gradual Rollout

1. **Week 1:** Config-based driver available via feature flag, existing driver default
2. **Week 2:** Both drivers available, logs show comparison
3. **Week 3:** Config-based driver becomes default, existing driver deprecated
4. **Week 4+:** Remove existing hand-coded driver

### Deprecation Timeline

```rust
// Phase 2: Add deprecation warning
#[deprecated(
    since = "0.3.0",
    note = "Use GenericSerialDriver with ell14.toml config instead"
)]
pub struct Ell14Driver { /* ... */ }

// Phase 3+: Remove (major version bump)
// Delete crates/daq-hardware/src/drivers/ell14.rs
```

</migration_guide>

<metadata>

## Confidence Assessment

**Overall Confidence:** High (85%)

**Per-Phase Confidence:**
- Phase 1: 95% - Standard config/validation work
- Phase 2: 85% - GenericSerialDriver is novel but well-scoped
- Phase 3: 80% - Pattern validation may reveal design issues
- Phase 4: 70% - State machines add complexity
- Phase 5: 75% - Rhai integration exists in codebase
- Phase 6: 60% - Binary protocols not deeply researched

## Dependencies

### External Crates (to add)

| Crate | Version | Purpose | Phase |
|-------|---------|---------|-------|
| `serde_valid` | ^0.24 | Config validation | 1 |
| `schemars` | ^0.8 | JSON schema generation | 1 |
| `enum_dispatch` | ^0.3 | Zero-cost polymorphism | 2 |
| `evalexpr` | ^11 | Expression evaluation for conversions | 2 |

### Internal Dependencies

- `common::capabilities` - Trait definitions (no changes needed)
- `common::parameter` - Parameter system (no changes needed)
- `daq-scripting` - Rhai integration (Phase 5)

## Open Questions

1. **Expression Language:** Use `evalexpr` crate or implement minimal custom parser for conversions?
   - **Decision:** Use `evalexpr` - mature, well-tested, adequate performance

2. **Hot Reload:** Should config changes be picked up without restart?
   - **Decision:** Defer to Phase 5+, implement via feature flag `plugins_hot_reload`

3. **Nested Device Configs:** Should configs support `include` directives?
   - **Decision:** Defer. Use TOML's native features if needed.

4. **Binary Protocol Priority:** When should Modbus support be added?
   - **Decision:** Phase 6 (future). No current hardware requires it.

## Assumptions

1. **Serial Protocol Dominance:** Most rust-daq devices use serial ASCII protocols
   - *Validation:* Survey existing drivers - all use serial

2. **Regex Performance:** Regex parsing is fast enough for device responses
   - *Validation:* Typical response parsing < 1ms, acceptable for serial timeouts

3. **Expression Evaluation:** Unit conversion expressions are simple arithmetic
   - *Validation:* Review existing drivers - all use simple formulas

4. **Config File Location:** Users expect configs in `config/devices/`
   - *Validation:* Consistent with existing `config/config.v4.toml` location

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| GenericSerialDriver doesn't cover all ELL14 edge cases | Medium | High | Extensive mock + hardware testing in Phase 2 |
| enum_dispatch doesn't work with async_trait | Low | High | Research shows compatibility; verify in Phase 2 |
| Config schema is too complex for users | Medium | Medium | Provide schema validation + IDE support |
| Performance regression with config-based drivers | Low | Medium | Benchmark in Phase 3; enum_dispatch mitigates |
| State machine config is too limiting | Medium | Low | Rhai fallback in Phase 5 |

## Test Strategy

### Unit Tests (per phase)
- Config parsing and validation
- Command template formatting
- Response regex parsing
- Unit conversion accuracy

### Integration Tests
- Mock serial port tests for command sequences
- Comparison tests: existing driver vs config-based

### Hardware Tests (on maitai)
- ELL14: position accuracy, homing, jog
- ESP300: multi-axis movement, velocity
- Newport1830C: power reading accuracy
- MaiTai: wavelength tuning, shutter

### Performance Tests
- enum_dispatch vs trait object benchmark
- Config loading time
- Command formatting overhead

</metadata>
