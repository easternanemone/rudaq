# ESP300 Hardware Validation Test Suite

## Overview

Comprehensive hardware validation test suite for the Newport ESP300 Multi-Axis Motion Controller driver. The test suite validates all critical functionality required for production use, including position control, velocity profiles, error handling, and communication robustness.

**Status**: Complete (18 tests)
**Location**: `tests/hardware_esp300_validation.rs`
**Required Features**: `hardware_tests`, `instrument_newport`

## Test Coverage

### Test Group 1: Position Control Accuracy (4 tests)

Tests basic absolute and relative position movement accuracy across different ranges.

| Test | Purpose | Range | Tolerance |
|------|---------|-------|-----------|
| `test_esp300_position_accuracy_small_movement` | Small incremental movement | 0.1mm | ±0.1mm |
| `test_esp300_position_accuracy_medium_movement` | Medium range movement | 3.0mm | ±0.1mm |
| `test_esp300_position_accuracy_large_movement` | Large range movement | 23mm | ±0.1mm |
| `test_esp300_relative_position_movement` | Relative incremental moves | ±2.0mm | ±0.1mm |

**Expected Result**: Position commands achieve target within specified tolerance.

### Test Group 2: Velocity and Acceleration Profiles (4 tests)

Validates velocity/acceleration setting, readback, and motion timing.

| Test | Purpose | Parameter | Tolerance |
|------|---------|-----------|-----------|
| `test_esp300_velocity_setting` | Set and read velocity | 5.0 mm/s | ±5% |
| `test_esp300_acceleration_setting` | Set and read acceleration | 10.0 mm/s² | ±5% |
| `test_esp300_velocity_profile_timing` | Verify motion timing matches velocity | 5mm @ 2mm/s | ±20% |
| `test_esp300_velocity_changes_during_motion` | Multiple velocity changes | 1.5-5.0 mm/s | ±10% |

**Expected Result**: Velocity/acceleration parameters set correctly and motion timing is consistent.

### Test Group 3: Error Handling and Recovery (4 tests)

Tests fault conditions, recovery, and command reliability.

| Test | Purpose | Scenario |
|------|---------|----------|
| `test_esp300_stop_halts_motion` | Stop command halts ongoing motion | Slow movement interrupted mid-command |
| `test_esp300_recovery_after_stop` | Recovery after manual stop | Move → Stop → Move to different position |
| `test_esp300_home_returns_to_origin` | Home successfully returns to zero | Move away → Home → Verify at origin |
| `test_esp300_position_query_consistency` | Position readings remain consistent | Multiple rapid queries at settled position |

**Expected Result**: All error conditions handled gracefully with full recovery.

### Test Group 4: Serial Communication Robustness (2 tests)

Validates serial protocol implementation under stress conditions.

| Test | Purpose | Scenario |
|------|---------|----------|
| `test_esp300_rapid_commands` | Handle rapid sequential commands | 10 commands without deadlock |
| `test_esp300_command_timeout_handling` | Timeout recovery | Verify normal operation resumes |

**Expected Result**: Serial communication remains robust under rapid operation.

### Test Group 5: Multi-Axis Coordination (2 tests)

Tests independent and coordinated control of multiple axes (if available).

| Test | Purpose | Scenario |
|------|---------|----------|
| `test_esp300_multi_axis_independence` | Axes move independently | Axis 1 to 5mm, Axis 2 to 8mm |
| `test_esp300_multi_axis_coordinated_motion` | Simultaneous axis motion | Both axes move at same velocity |

**Expected Result**: Multiple axes operate independently and in coordination without interference.

### Integration Tests (2 tests)

Full workflow validation simulating real-world usage patterns.

| Test | Purpose | Workflow |
|------|---------|----------|
| `test_esp300_complete_workflow` | Full experiment sequence | Setup → Scan → Return to home |
| `test_esp300_stress_many_movements` | Stress testing | 20 sequential movements |

**Expected Result**: Stable operation through complete workflows and sustained operation.

## Hardware Setup Requirements

### Physical Setup
- **ESP300 Controller**: Connected via RS-232 serial port
- **Default Port**: `/dev/ttyUSB0` (Linux/macOS) or `COM3` (Windows)
- **Baud Rate**: 19200, 8N1, No Flow Control
- **Mechanical Load**: Linear stage or similar with ≥25mm travel
- **Homing Sensor**: Home switch for origin reference

### Environment Variable Override
```bash
export ESP300_PORT=/dev/ttyUSB1  # Override default port
cargo test --test hardware_esp300_validation --features hardware_tests,instrument_newport
```

### Safe Travel Range
- **Minimum**: 0.5mm (avoid collision at home)
- **Maximum**: 24.0mm (keep margin from hard stop)
- **All tests confined to 0-23mm range**

## Running Tests

### Run All Hardware Tests
```bash
cargo test --test hardware_esp300_validation \
  --features hardware_tests,instrument_newport \
  -- --nocapture
```

### Run Specific Test Group
```bash
# Position accuracy tests only
cargo test --test hardware_esp300_validation \
  --features hardware_tests,instrument_newport \
  test_esp300_position_accuracy

# Velocity profile tests
cargo test --test hardware_esp300_validation \
  --features hardware_tests,instrument_newport \
  test_esp300_velocity

# Error handling tests
cargo test --test hardware_esp300_validation \
  --features hardware_tests,instrument_newport \
  test_esp300_recovery
```

### Run with Detailed Output
```bash
RUST_LOG=debug cargo test --test hardware_esp300_validation \
  --features hardware_tests,instrument_newport \
  -- --nocapture --test-threads=1
```

## Safety Features

All tests implement the following safety measures:

1. **Pre-test Setup**: Homing to establish known state
2. **Soft Limits**: Position validation against safe range
3. **Timeout Protection**: 60-second timeout on settling operations
4. **Graceful Cleanup**: Automatic stop and home after test completion
5. **Position Tolerance**: ±0.1mm typical tolerance for accuracy validation

## Test Execution Timeline

**Estimated Execution Time**: 45 minutes for complete suite

- Small movement accuracy: 30 seconds
- Medium movement accuracy: 40 seconds
- Large movement accuracy: 60 seconds
- Relative movement: 40 seconds
- Velocity setting: 10 seconds
- Acceleration setting: 10 seconds
- Velocity timing: 30 seconds
- Velocity changes: 20 seconds
- Stop halt motion: 40 seconds
- Recovery after stop: 50 seconds
- Home returns: 50 seconds
- Position consistency: 30 seconds
- Rapid commands: 50 seconds
- Timeout handling: 20 seconds
- Multi-axis independence: 120 seconds (if available)
- Multi-axis coordination: 180 seconds (if available)
- Complete workflow: 120 seconds
- Stress many movements: 180 seconds

## Expected Results

### Success Criteria
- All 18 tests pass consistently
- No timeouts or communication errors
- Position accuracy within ±0.1mm
- Velocity/acceleration within ±5%
- Motion timing within ±20% of calculated values

### Known Issues
- Multi-axis tests skip gracefully if axis 2 not available
- Position tolerance may need adjustment for worn hardware (specify in test comments)

## Troubleshooting

### Port Not Found
```bash
# List available serial ports (Linux)
ls /dev/ttyUSB*

# List available serial ports (macOS)
ls /dev/tty.usb*

# List available serial ports (Windows)
mode  # or use Device Manager
```

### Timeout Errors
- Check serial port connection
- Verify baud rate is correct (19200)
- Ensure no other process is accessing the port

### Position Out of Range
- Verify mechanical limits are set correctly on hardware
- Check that homing sensor is properly configured
- Ensure stage is not mechanically blocked

### Velocity/Acceleration Not Updating
- Some ESP300 firmware versions require parameter save
- Verify axis acceleration range: typically 1-1000 mm/s²
- Verify axis velocity range: typically 0.1-40 mm/s

## Integration with CI/CD

Tests can be integrated into continuous integration pipelines:

```yaml
# GitHub Actions example
- name: Run ESP300 Hardware Validation
  if: env.HARDWARE_AVAILABLE == 'true'
  run: |
    cargo test --test hardware_esp300_validation \
      --features hardware_tests,instrument_newport \
      -- --nocapture
```

## Related Documentation

- [ESP300 Driver Implementation](../hardware/esp300.rs)
- [Capability Traits](../hardware/capabilities.rs)
- [Hardware Testing Guide](./HARDWARE_TESTING_GUIDE.md)

## Issue Tracking

- **Issue**: bd-38fa - ESP300 hardware validation (16 tests, 45min)
- **Status**: Complete - 18 tests implemented and passing
- **Completion Date**: 2025-11-22
