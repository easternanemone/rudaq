# Phase 4: Production Hardening - Summary

## Objective
Add production-ready features to the declarative driver plugin system including retry logic, error handling, per-command timeouts, and initialization sequences.

## Completed Work

### 1. Schema Enhancements

#### Per-Command Timeout (`CommandConfig.timeout_ms`)
Commands can now specify their own timeout, overriding the connection-level default:
```toml
[commands.move_absolute]
template = "${address}ma${position_pulses:08X}"
timeout_ms = 5000  # Override connection timeout for slow operations
```

#### Retry Configuration (`RetryConfig`)
Full exponential backoff retry logic with error-specific control:
```toml
[default_retry]
max_retries = 3
initial_delay_ms = 100
max_delay_ms = 2000
backoff_multiplier = 2.0
retry_on_errors = ["0x01", "0x09"]      # Only retry these errors
no_retry_on_errors = ["0x07", "0x0D"]   # Never retry these errors

[commands.move_absolute.retry]
max_retries = 5                          # Command-specific override
initial_delay_ms = 500
```

#### Enhanced Error Codes (`ErrorCodeConfig`)
Error codes now include severity levels and recovery actions:
```toml
[error_codes."0x02"]
name = "MechanicalTimeout"
description = "Motor didn't reach target position"
recoverable = true
severity = "error"                       # info, warning, error, critical, fatal

[error_codes."0x02".recovery_action]
command = "home"
auto_recover = false
manual_instructions = "Home the device to recalibrate position"
```

#### Initialization Sequence (`init_sequence`)
Devices can define an ordered sequence of commands to run on connect:
```toml
[[init_sequence]]
command = "get_info"
description = "Query device information"
required = true

[[init_sequence]]
command = "get_status"
description = "Verify device is ready"
required = true
expect = "GS00"

[[init_sequence]]
command = "get_position"
description = "Initialize position state"
required = false
delay_ms = 100
```

### 2. GenericSerialDriver Enhancements

#### Error Detection (`check_for_error`)
Automatically checks responses for configured error codes:
```rust
if let Some(device_error) = driver.check_for_error(&response) {
    // device_error contains: code, name, description, severity, recoverable
}
```

#### Retry Logic (`execute_with_retry`)
New method that wraps command execution with retry:
```rust
let result = driver.execute_with_retry("move_absolute", &params).await?;
// result.response - the successful response
// result.retries - number of retry attempts made
// result.duration - total time taken
```

#### Init Sequence Execution (`run_init_sequence`)
Run the configured initialization sequence:
```rust
driver.run_init_sequence().await?;
```

### 3. New Types

| Type | Purpose |
|------|---------|
| `RetryConfig` | Retry behavior configuration |
| `ErrorSeverity` | Enum: `Info`, `Warning`, `Error`, `Critical`, `Fatal` |
| `RecoveryAction` | Recovery command/instructions for errors |
| `InitStep` | Single step in initialization sequence |
| `DeviceError` | Runtime error with severity/recovery info |
| `CommandResult` | Response with retry tracking |

## Test Coverage

### Production Hardening Tests (`production_hardening.rs`)
- `test_per_command_timeout_parsing` - Per-command timeout in schema
- `test_default_retry_config_parsing` - Default retry configuration
- `test_command_specific_retry_config` - Command-level retry override
- `test_retry_config_defaults` - RetryConfig::default() values
- `test_error_code_parsing` - Error codes with severity/recovery
- `test_error_severity_serialization` - All severity levels
- `test_init_sequence_parsing` - Init sequence steps
- `test_error_code_detection` - Runtime error detection
- `test_full_production_config_parsing` - Complete config integration
- `test_driver_creation_with_production_config` - Driver instantiation

## Test Results

```
171 tests total:
- 96 unit tests (lib.rs)
- 16 ell14_migration tests
- 12 esp300_migration tests
- 22 maitai_migration tests
- 15 newport1830c_migration tests
- 10 production_hardening tests
All passing, 1 ignored (schema regeneration)
```

## Files Modified/Created

### Schema (`crates/daq-hardware/src/config/schema.rs`)
- Added `RetryConfig` struct with exponential backoff fields
- Added `timeout_ms` and `retry` fields to `CommandConfig`
- Added `ErrorSeverity` enum
- Added `RecoveryAction` struct to `ErrorCodeConfig`
- Added `InitStep` struct
- Added `init_sequence` and `default_retry` to `DeviceConfig`

### GenericSerialDriver (`crates/daq-hardware/src/drivers/generic_serial.rs`)
- Added `DeviceError` and `CommandResult` types
- Added `check_for_error()` method
- Added `should_retry()` helper
- Added `execute_with_retry()` method
- Added `transaction_with_timeout()` method
- Added `run_init_sequence()` method

### ELL14 Config (`config/devices/ell14.toml`)
- Added `[default_retry]` section
- Added per-command `timeout_ms` for movement commands
- Added `severity` to all error codes
- Added `recovery_action` for recoverable errors
- Added `[[init_sequence]]` steps

### Tests
- Created `crates/daq-hardware/tests/production_hardening.rs`

## Usage Examples

### Basic Retry Configuration
```toml
# Global defaults
[default_retry]
max_retries = 3
initial_delay_ms = 100
backoff_multiplier = 2.0

# Command-specific (inherits + overrides)
[commands.slow_operation.retry]
max_retries = 5
initial_delay_ms = 500
```

### Error Recovery Flow
```toml
[error_codes."BUSY"]
name = "Device Busy"
recoverable = true
severity = "info"

[error_codes."BUSY".recovery_action]
auto_recover = true
delay_ms = 200  # Wait 200ms then retry
```

### Device Initialization
```toml
[[init_sequence]]
command = "identify"
required = true

[[init_sequence]]
command = "self_test"
required = false  # Optional - continue on failure
expect = "PASS"
delay_ms = 1000
```

## Remaining Work (Phase 5+)

1. **Connection Management** - Reconnection handling, connection pooling
2. **State Machines** - smlang integration for complex device states
3. **Hot Reload** - Watch config files for changes
4. **Documentation** - User guide for creating new device configs

## Architecture Notes

### Retry Logic Flow
```
execute_with_retry(command, params)
    └── Loop (max_retries):
        ├── Format command
        ├── Execute with timeout (per-command or default)
        ├── Check for device error in response
        │   └── If recoverable + should_retry → delay → continue
        │   └── If non-recoverable → return error
        ├── Parse response
        └── Return CommandResult
```

### Init Sequence Flow
```
run_init_sequence()
    └── For each step:
        ├── Execute command via execute_with_retry
        ├── Validate expected response (if configured)
        │   └── If required=true and mismatch → fail
        │   └── If required=false → warn and continue
        └── Apply delay_ms
```
