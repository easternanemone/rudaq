# LIBS Drivers Integration Smoke Tests

This document summarizes the smoke test implementation for LIBS (Laser-Induced Breakdown Spectroscopy) hardware drivers.

## Completed Work

### 1. Compilation Error Fixes

#### driver-dover-motion (bd-3yb8.1)
**Issue:** `*mut c_void` handle is not `Send`/`Sync` safe for async contexts

**Fix:** Created `AxisHandle` newtype wrapper with explicit `unsafe impl Send + Sync`

```rust
#[repr(transparent)]
struct AxisHandle(*mut c_void);

unsafe impl Send for AxisHandle {}
unsafe impl Sync for AxisHandle {}
```

**Rationale:** The Dover Motion SDK supports concurrent access to axis handles from multiple threads. All accesses are protected by `Arc<Mutex<>>` and executed in `spawn_blocking` contexts.

**Files Modified:**
- `crates/driver-dover-motion/src/driver.rs`

#### driver-spirit-laser (bd-3yb8.4)
**Issues:**
1. `Readable` trait: Added `units()` method not in trait definition
2. `ShutterControl` trait: Used `shutter_state()` instead of `is_shutter_open()`
3. `EmissionControl` trait: Used `emission_state()` instead of `is_emission_enabled()`
4. Return types: Used `Result<T, DaqError>` instead of `Result<T>` (from anyhow)
5. `Parameterized` trait: Used async methods instead of sync

**Fixes:**
- Removed `units()` method from `Readable` impl
- Renamed `shutter_state()` → `is_shutter_open()`
- Renamed `emission_state()` → `is_emission_enabled()`
- Changed all return types to use `anyhow::Result<T>`
- Changed `Parameterized` from async to sync (with temporary `OnceLock` workaround)

**Files Modified:**
- `crates/driver-spirit-laser/src/spirit.rs`

#### driver-andor-sdk3 (bd-3yb8.2/3)
**Status:** No changes required. The `AT_H` type is an integer handle that is already `Send`/`Sync` compatible.

### 2. Smoke Test Implementation

Created comprehensive smoke tests following the pattern established in `driver-comedi/tests/hardware_smoke.rs`:

#### driver-dover-motion
**File:** `crates/driver-dover-motion/tests/libs_smoke.rs`

**Mock Tests (Always Run):**
- `mock_driver_initialization` - Driver creation
- `mock_basic_motion` - Move operations
- `mock_trigger_on_position` - TOP functionality
- `test_invalid_top_parameters` - Error handling

**Hardware Tests (Gated):**
- `hardware_device_connection` - Device connectivity
- `hardware_small_move` - Motion accuracy verification

**Environment Variables:**
- `DOVER_MOTION_SMOKE_TEST=1` - Enable hardware tests
- `DOVER_CONFIG_PATH` - Device config XML path
- `DOVER_AXIS_NAME` - Axis to test (default: "X")

#### driver-andor-sdk3
**File:** `crates/driver-andor-sdk3/tests/libs_smoke.rs`

**Mock Tests (Always Run):**
- `mock_camera_initialization` - Camera setup
- `mock_camera_exposure_control` - Exposure settings
- `mock_camera_triggering` - Trigger/arm operations
- `mock_camera_frame_producer` - Streaming control
- `mock_spectrograph_initialization` - Spectrograph setup

**Hardware Tests (Gated):**
- `hardware_camera_connection` - Camera connectivity
- `hardware_camera_exposure` - Exposure timing verification
- `hardware_camera_trigger_config` - External trigger setup
- `hardware_spectrograph_connection` - Spectrograph connectivity
- `hardware_spectrograph_wavelength` - Wavelength control
- `hardware_camera_and_trigger_sync` - Trigger configuration test

**Environment Variables:**
- `ANDOR_SMOKE_TEST=1` - Enable hardware tests
- `ANDOR_CAMERA_INDEX` - Camera index (default: 0)
- `ANDOR_SPECTROGRAPH_INDEX` - Spectrograph index (default: 0)

#### driver-spirit-laser
**File:** `crates/driver-spirit-laser/tests/libs_smoke.rs`

**Mock Tests (Always Run):**
- `mock_laser_initialization` - Laser connection
- `mock_laser_shutter_control` - Shutter open/close
- `mock_laser_emission_control` - Emission enable/disable
- `mock_laser_readable` - State readout
- `test_canopen_sdo_protocol` - CANopen communication
- `test_shutter_safety_sequence` - Safety protocol
- `test_timeout_handling` - Error handling

**Hardware Tests (Gated):**
- `hardware_laser_connection` - CAN bus connectivity
- `hardware_laser_shutter` - Shutter operation
- `hardware_laser_emission` - Emission control (safety notes)

**Environment Variables:**
- `SPIRIT_SMOKE_TEST=1` - Enable hardware tests
- `SPIRIT_CAN_ADAPTER` - CAN adapter path (default: "can0" on Linux, "COM3" on Windows)

#### Multi-Device Integration
**File:** `tests/libs_integration_smoke.rs`

**Mock Tests (Always Run):**
- `mock_stage_camera_sync` - Stage-camera coordination concept
- `mock_full_libs_sequence` - Complete LIBS workflow simulation
- `test_timing_requirements` - LIBS timing documentation

**Hardware Tests (Gated):**
- `hardware_stage_camera_top_sync` - Stage triggers camera via TOP
- `hardware_libs_safety_sequence` - Laser safety protocol verification

**Environment Variables:**
- `LIBS_INTEGRATION_TEST=1` - Enable integration tests
- (Plus all individual driver environment variables)

### 3. Test Infrastructure

#### Test Categories
1. **Mock Mode Tests** - Always run, no hardware required, verify driver logic
2. **Hardware Tests** - Gated by environment variables, require real devices
3. **Integration Tests** - Multi-device coordination tests

#### Environment Variable Pattern
All tests follow the pattern:
```rust
fn smoke_test_enabled() -> bool {
    env::var("DRIVER_SMOKE_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

macro_rules! skip_if_disabled {
    () => {
        if !smoke_test_enabled() {
            println!("Test skipped (set DRIVER_SMOKE_TEST=1 to enable)");
            return;
        }
    };
}
```

#### Nextest Profile
The tests use the `--profile hardware` nextest profile (or `--profile libs-hardware` for LIBS-specific tests):

```bash
# Mock tests (no hardware)
cargo nextest run -p driver-dover-motion

# Hardware tests
export DOVER_MOTION_SMOKE_TEST=1
cargo nextest run --profile hardware --features hardware -p driver-dover-motion

# Integration tests
export LIBS_INTEGRATION_TEST=1
cargo nextest run --profile libs-hardware --features hardware --test libs_integration_smoke
```

## Test Coverage Matrix

| Driver | Mock Tests | Hardware Tests | Integration Tests |
|--------|------------|----------------|-------------------|
| driver-dover-motion | ✅ 4 tests | ✅ 2 tests | ✅ Included |
| driver-andor-sdk3 | ✅ 5 tests | ✅ 6 tests | ✅ Included |
| driver-spirit-laser | ✅ 7 tests | ✅ 3 tests | ✅ Included |

Total: **16 mock tests**, **11 hardware tests**, **5 integration tests**

## Running the Tests

### All Mock Tests (CI-Safe)
```bash
cargo nextest run -p driver-dover-motion -p driver-andor-sdk3 -p driver-spirit-laser
cargo nextest run --test libs_integration_smoke
```

### All Hardware Tests (Requires Hardware)
```bash
# Set all environment variables
export DOVER_MOTION_SMOKE_TEST=1
export ANDOR_SMOKE_TEST=1
export SPIRIT_SMOKE_TEST=1
export LIBS_INTEGRATION_TEST=1

# Run with hardware feature
cargo nextest run --profile hardware --features hardware \
  -p driver-dover-motion \
  -p driver-andor-sdk3 \
  -p driver-spirit-laser \
  --test libs_integration_smoke
```

### Individual Driver Tests
```bash
# Dover Motion only
export DOVER_MOTION_SMOKE_TEST=1
cargo nextest run --profile hardware --features hardware -p driver-dover-motion

# Andor SDK3 only
export ANDOR_SMOKE_TEST=1
cargo nextest run --profile hardware --features hardware,camera -p driver-andor-sdk3

# Spirit Laser only
export SPIRIT_SMOKE_TEST=1
cargo nextest run --profile hardware --features hardware -p driver-spirit-laser
```

## Multi-Device Coordination Tests

The integration tests verify the following LIBS workflows:

### Triggered Acquisition (TOP Synchronization)
1. Dover Motion stage configured with Trigger-On-Position (TOP)
2. Andor camera configured for external trigger
3. Stage scans and triggers camera at each position
4. Camera acquires spectrum on each trigger

### Safety Sequence
1. Camera armed first (ready to acquire)
2. Laser emission enabled with shutter closed
3. Shutter opened only when ready
4. During shutdown:
   - Shutter closed FIRST (beam blocked)
   - Then emission disabled

### Timing Requirements
- Camera DDG delay: ~1.3µs (avoid plasma continuum)
- Camera DDG width: ~10µs (capture atomic emission)
- Stage TOP pulse: 1µs minimum
- Camera exposure: 1-2ms per trigger

## Files Created

- `crates/driver-dover-motion/tests/libs_smoke.rs` (192 lines)
- `crates/driver-dover-motion/tests/README.md` (46 lines)
- `crates/driver-andor-sdk3/tests/libs_smoke.rs` (287 lines)
- `crates/driver-spirit-laser/tests/libs_smoke.rs` (317 lines)
- `tests/libs_integration_smoke.rs` (412 lines)
- `LIBS_SMOKE_TESTS.md` (this file)

## Files Modified

- `crates/driver-dover-motion/src/driver.rs` - Added AxisHandle newtype (7 lines added, safety docs)
- `crates/driver-spirit-laser/src/spirit.rs` - Fixed trait implementations (~50 lines modified)

## Dependencies

All tests use existing dependencies:
- `tokio::test` - Async test runtime
- `std::env` - Environment variable access
- Driver-specific types - From the drivers being tested
- `common::capabilities` - Capability traits

No new dependencies added.

## Next Steps

1. **CI Integration**: Add LIBS smoke tests to GitHub Actions workflow
2. **Hardware CI**: Set up dedicated LIBS hardware test environment
3. **Coverage Metrics**: Track test coverage for LIBS drivers
4. **Documentation**: Add test documentation to driver README files
5. **Performance**: Add benchmarks for critical paths (TOP timing, camera acquisition latency)
6. **Negative Tests**: Add more error condition tests (timeouts, hardware failures)

## Notes

- All mock tests are CI-safe and run without hardware
- Hardware tests are strictly gated by environment variables
- Integration tests demonstrate real LIBS workflows
- Safety protocols are emphasized in laser control tests
- Timing requirements are documented for proper LIBS operation
