# V4 Hardware Validation Framework

Comprehensive automated test framework for executing 94 hardware test scenarios across 5 V4 actors on real hardware via SSH to `maitai@maitai-eos`.

## Framework Overview

**Total Test Scenarios: 94**

- 17 SCPI generic instrument tests
- 14 Newport 1830-C optical power meter tests
- 16 ESP300 motion controller tests
- 28 PVCAM camera tests
- 19 MaiTai laser tests (with critical safety checks)

## Architecture

### Module Structure

```
tests/
  hardware_validation_test.rs          # Main test harness and discovery
  hardware_validation/
    mod.rs                             # Framework core
    scpi_hardware_tests.rs             # 17 SCPI tests
    newport_hardware_tests.rs          # 14 Newport tests
    esp300_hardware_tests.rs           # 16 ESP300 tests
    pvcam_hardware_tests.rs            # 28 PVCAM tests
    maitai_hardware_tests.rs           # 19 MaiTai tests
```

### Core Components

#### `HardwareTestHarness`

Collects and reports test results:

```rust
pub struct HardwareTestHarness {
    results: Vec<TestResult>,
    setup_errors: Vec<String>,
    teardown_errors: Vec<String>,
}
```

Methods:
- `new()` - Create new harness
- `add_result(result: TestResult)` - Add test result
- `add_setup_error(error: &str)` - Record setup error
- `add_teardown_error(error: &str)` - Record teardown error
- `results()` - Get all results
- `summary()` - Get statistics
- `print_report()` - Print formatted report

#### `TestResult`

Individual test result with timing:

```rust
pub struct TestResult {
    pub test_name: String,
    pub passed: bool,
    pub duration_ms: u64,
    pub error_message: Option<String>,
    pub timestamp_ns: i64,
}
```

Methods:
- `passed(test_name: &str, duration_ms: u64)` - Create passing result
- `failed(test_name: &str, duration_ms: u64, error: &str)` - Create failing result

#### Utility Functions

Located in `hardware_validation::utils`:

- `measure_test_execution<F, T>(test_fn: F) -> (T, u64)` - Measure execution time
- `verify_hardware_response_timeout<F, T>(operation: F, timeout: Duration) -> Result<T, String>` - Timeout wrapper
- `safe_operation<F, T>(pre_check, operation, post_check) -> Result<T, String>` - Safety wrapper for critical operations

### Timeout Constants

```rust
pub const HARDWARE_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);
pub const COMMUNICATION_TIMEOUT: Duration = Duration::from_secs(2);
pub const MEASUREMENT_TIMEOUT: Duration = Duration::from_secs(10);
```

## Test Suites

### SCPI Tests (17 scenarios)

Generic SCPI instrument tests for any VISA-compliant instrument.

**Tests:**
1. VISA resource detection
2. Instrument identification (*IDN?)
3. Clear instrument state (*CLS)
4. Reset instrument (*RST)
5. Operation complete query (*OPC?)
6. Device error query (SYST:ERR?)
7. Set measurement function
8. Configure measurement range
9. Read single measurement
10. Continuous measurement mode
11. Measurement accuracy (10V, 1%)
12. Measurement accuracy (100mV, 2%)
13. Handle command errors
14. Measurement with timeout
15. Bulk read from buffer
16. Transaction retry on timeout
17. Graceful disconnection

**File:** `/Users/briansquires/code/rust-daq/v4-daq/tests/hardware_validation/scpi_hardware_tests.rs`

### Newport 1830-C Tests (14 scenarios)

Optical power meter tests (700-1100nm wavelength range).

**Tests:**
1. Instrument identification
2. Wavelength calibration (633nm, HeNe standard)
3. Wavelength calibration (532nm, green laser)
4. Wavelength IR (800nm, Ti:Sapphire)
5. Wavelength IR (1064nm, Nd:YAG)
6. Read power (watts)
7. Read power (milliwatts)
8. Read power (microwatts)
9. Zero/reference calibration
10. Power measurement accuracy (2% tolerance)
11. Multi-unit measurement (all 5 units on maitai-eos)
12. Sensor temperature reading
13. Wavelength sweep measurement
14. Graceful shutdown

**File:** `/Users/briansquires/code/rust-daq/v4-daq/tests/hardware_validation/newport_hardware_tests.rs`

### ESP300 Motion Controller Tests (16 scenarios)

Newport ESP300 multi-axis motion controller tests (3 axes).

**Tests:**
1. Controller identification
2. Axis configuration query
3. Home axis 0
4. Home axis 1
5. Home axis 2
6. Move axis 0 absolute (10mm)
7. Move axis 1 absolute (5mm)
8. Read position accuracy
9. Velocity configuration
10. Soft limit minimum
11. Soft limit maximum
12. Limit switch detection
13. Emergency stop (CRITICAL)
14. Multi-axis synchronized move
15. Relative move
16. Graceful shutdown (return to home)

**Safety Features:**
- All tests return axes to home position after completion
- Safe return helper function: `safe_return_home()`
- Emergency stop verification (immediate stop all axes)
- Soft limit validation

**File:** `/Users/briansquires/code/rust-daq/v4-daq/tests/hardware_validation/esp300_hardware_tests.rs`

### PVCAM Camera Tests (28 scenarios)

PVCAM camera acquisition and configuration tests.

**Tests:**
1. Camera detection
2. Camera initialization
3. Query sensor dimensions (2048x2048)
4. Query pixel format (MONO16)
5. Set exposure time
6. Set binning (1x1)
7. Set binning (2x2)
8. Set binning (4x4)
9. Set ROI (full sensor)
10. Set ROI (center crop, 256x256)
11. Set ROI (custom)
12. Single frame acquisition
13. Frame rate measurement (~9 fps with 100ms exposure)
14. Trigger mode (internal/free-running)
15. Trigger mode (external/TTL)
16. Query sensor temperature
17. Enable cooler (if available)
18. Streaming start/stop
19. Frame timestamp accuracy
20. Memory buffer allocation
21. Multi-ROI support
22. Gain configuration
23. Throughput test (2048x2048, ~72 MB/s)
24. Frame timing jitter (<2ms)
25. Error recovery
26. Graceful shutdown
27. Dark frame acquisition (shutter closed)
28. Full sensor linearity test

**Performance Benchmarks:**
- Frame rate: ~9 fps (100ms exposure = 110ms period)
- Throughput: ~72 MB/s at full resolution
- Memory per frame: 8 MB (2048x2048 x 2 bytes)
- Frame period with 2x2 binning: ~27.5ms (faster 4x speed)

**File:** `/Users/briansquires/code/rust-daq/v4-daq/tests/hardware_validation/pvcam_hardware_tests.rs`

### MaiTai Laser Tests (19 scenarios)

Spectra Physics MaiTai Ti:Sapphire laser tests with critical safety checks.

**Tests:**
1. Laser identification
2. CRITICAL: Verify shutter closed (safety check)
3. Shutter open/close
4. Wavelength set (690nm, minimum)
5. Wavelength set (800nm, typical)
6. Wavelength set (1000nm, high end)
7. Wavelength set (1040nm, maximum)
8. Wavelength accuracy (±0.5nm)
9. Read power output (0.1-2W)
10. Power stability over time
11. Wavelength sweep (690-1040nm)
12. Wavelength tuning speed (~100nm/sec)
13. Query crystal temperature
14. UART communication timeout handling
15. Error recovery
16. Repeated measurement cycle
17. Safe shutdown sequence
18. Emergency shutdown (force shutter closed)
19. FINAL SAFETY CHECK (shutter closed)

**Safety-Critical Features:**

All MaiTai tests implement forced safety checks:

1. **Pre-operation checks:**
   - Verify shutter is closed before operation
   - Check no previous errors in queue

2. **Safety wrappers:**
   ```rust
   safe_operation(
       || { /* pre-check: verify shutter closed */ },
       async { /* operation */ },
       || { /* post-check: verify shutter closed */ }
   ).await
   ```

3. **Post-operation enforcement:**
   - Force close shutter after every operation
   - Verify closed state via query

4. **Emergency shutdown:**
   - Immediate force-close: `SHUTTER:0`
   - No timeout on emergency commands
   - Verify closed state mandatory

5. **Final verification:**
   - Test 19 (Final Safety Check) MUST pass
   - Confirms laser is safe at test completion

**Wavelength Range:** 690-1040nm (Ti:Sapphire tuning)

**Serial Configuration:**
- Port: `/dev/ttyUSB5` (configurable)
- Baud: 9600
- Line terminator: `\r`
- Response delimiter: `\r`
- Timeout: 2 seconds

**File:** `/Users/briansquires/code/rust-daq/v4-daq/tests/hardware_validation/maitai_hardware_tests.rs`

## Running Tests

### All Hardware Tests (with mock hardware)

```bash
cargo test --test hardware_validation_test -- --ignored
```

Total: 8 integration tests + 94 hardware tests = 102 tests

### By Test Suite

```bash
# SCPI tests (17)
cargo test --test hardware_validation_test -- --ignored scpi

# Newport tests (14)
cargo test --test hardware_validation_test -- --ignored newport

# ESP300 tests (16)
cargo test --test hardware_validation_test -- --ignored esp300

# PVCAM tests (28)
cargo test --test hardware_validation_test -- --ignored pvcam

# MaiTai tests (19)
cargo test --test hardware_validation_test -- --ignored maitai
```

### Integration Tests Only (no hardware)

```bash
cargo test --test hardware_validation_test -- --nocapture
```

Runs:
- Framework initialization test
- Result creation test
- Harness collection test
- Utility function tests
- Documentation tests

## Hardware Requirements

Tests are designed to run on:

**Target:** `maitai@maitai-eos` via SSH

**Hardware Configuration:**

1. **SCPI Instruments:** Any VISA-compliant instrument
   - Example: Keysight 34401A multimeter
   - Resource: `TCPIP0::192.168.1.100::INSTR`

2. **Newport 1830-C Power Meters (x5)**
   - All 5 units responsive
   - Serial/GPIB connection
   - Wavelength range: 200-1100nm

3. **ESP300 Motion Controller**
   - Serial: `/dev/ttyUSB0`
   - Baud: 19200
   - 3 axes (X, Y, Z)
   - Soft limits configurable

4. **PVCAM Camera**
   - Sensor: 2048x2048 16-bit monochrome
   - Interface: PCI or USB
   - Temperature monitoring available

5. **MaiTai Laser**
   - Serial: `/dev/ttyUSB5`
   - Baud: 9600
   - Wavelength: 690-1040nm
   - Power: 100mW-2W typical

## Test Execution Model

### Mock Hardware Testing

All tests can run on mock hardware (no physical hardware required):

```rust
// Tests use simulated responses
let idn_response = "Keysight Technologies,34401A,US12345678,A.01.15";
let power_reading = 0.123; // Simulated 123mW
```

### Real Hardware Testing

For real hardware validation via SSH:

1. **Connect to maitai-eos:**
   ```bash
   ssh maitai@maitai-eos
   ```

2. **Navigate to project:**
   ```bash
   cd ~/rust-daq/v4-daq
   ```

3. **Run tests:**
   ```bash
   cargo test --test hardware_validation_test -- --ignored --test-threads=1
   ```
   (Single-threaded to avoid hardware contention)

4. **Review results:**
   Tests print detailed status for each operation

## Safety Considerations

### MaiTai Laser Safety

**CRITICAL:** Shutter must always be closed when not actively measuring.

1. **Startup:** Shutter verified closed (Test 2)
2. **Operations:** Shutter closed between commands
3. **Measurements:** Shutter open only during measurement
4. **Shutdown:** Shutter forced closed (Tests 17-19)

### ESP300 Motion Safety

1. **Homing:** All axes homed before operation
2. **Limits:** Soft limits enforced
3. **Emergency:** Emergency stop tested (Test 13)
4. **Shutdown:** Axes returned to home position

### PVCAM Camera Safety

1. **Streaming:** Always stopped before shutdown
2. **Buffers:** Memory cleaned up on error
3. **Trigger:** Trigger mode appropriate for use case

## Test Result Reporting

### Console Output

Example output:
```
PASS: SCPI_01_VISA_Resource_Detection completed in 5ms
PASS: MaiTai_02_CRITICAL_Verify_Shutter_Closed completed in 50ms - SHUTTER CONFIRMED CLOSED
FAIL: Newport_11_Multi_Unit_Measurement failed: Timeout after 2000ms
...
=== SUMMARY ===
Total:   94
Passed:  92
Failed:  2
Duration: 45,000ms
```

### Summary Statistics

```rust
pub struct TestSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub total_duration_ms: u64,
    pub setup_errors: usize,
    pub teardown_errors: usize,
}
```

## Extension and Customization

### Adding New Tests

1. **Create test function:**
   ```rust
   #[tokio::test]
   #[ignore]
   async fn test_device_XX_description() {
       let test_name = "Device_XX_Description";
       let (result, duration_ms) = measure_test_execution(|| {
           // Test logic
           Ok::<(), String>(())
       });

       if result.is_ok() {
           println!("PASS: {} completed in {}ms", test_name, duration_ms);
       } else {
           println!("FAIL: {} failed: {:?}", test_name, result.err());
       }
   }
   ```

2. **Use safety wrapper for critical operations:**
   ```rust
   let result = safe_operation(
       || { /* pre-check */ },
       async { /* operation */ },
       || { /* post-check */ }
   ).await;
   ```

3. **Add to appropriate test file:**
   - SCPI operations → `scpi_hardware_tests.rs`
   - Newport → `newport_hardware_tests.rs`
   - etc.

### Custom Timeout Values

```rust
let result = verify_hardware_response_timeout(
    async { /* operation */ },
    Duration::from_millis(5000) // Custom 5 second timeout
).await;
```

## Troubleshooting

### Test Timeouts

If tests timeout:
1. Verify hardware is responsive
2. Check serial port connections
3. Increase timeout if hardware is slow
4. Check for blocking operations

### Connection Failures

1. Verify SSH access to `maitai@maitai-eos`
2. Check hardware serial ports with `ls /dev/ttyUSB*`
3. Verify VISA resources: `visainfo` (on target system)
4. Check serial port permissions

### Safety Check Failures

For MaiTai shutter safety failures:
1. Manually close shutter: Serial command `SHUTTER:0`
2. Power cycle laser if unresponsive
3. Check serial connection

## Performance Benchmarks

### Expected Execution Times

- **SCPI tests:** 5-100ms each (~2 seconds total)
- **Newport tests:** 50-500ms each (~5 seconds total)
- **ESP300 tests:** 800-2000ms each (~20 seconds total, includes motion)
- **PVCAM tests:** 100-2000ms each (~30 seconds total, includes frame acquisition)
- **MaiTai tests:** 50-700ms each (~10 seconds total, safety checks add overhead)

**Total expected runtime:** ~67 seconds (with real hardware)

## File Locations

```
/Users/briansquires/code/rust-daq/v4-daq/
  tests/
    hardware_validation_test.rs              (94 tests + 8 integration)
    hardware_validation/
      mod.rs                                 (Framework core)
      scpi_hardware_tests.rs                 (17 tests)
      newport_hardware_tests.rs              (14 tests)
      esp300_hardware_tests.rs               (16 tests)
      pvcam_hardware_tests.rs                (28 tests)
      maitai_hardware_tests.rs               (19 tests)
  HARDWARE_VALIDATION_FRAMEWORK.md           (This file)
```

## References

### Actor Implementations

- `src/actors/scpi.rs` - Generic SCPI actor
- `src/actors/newport_1830c.rs` - Newport power meter actor
- `src/actors/esp300.rs` - ESP300 motion controller
- `src/actors/pvcam.rs` - PVCAM camera
- `src/actors/maitai.rs` - MaiTai laser

### Hardware Traits

- `src/traits/scpi_endpoint.rs` - SCPI interface
- `src/traits/power_meter.rs` - Power meter interface
- `src/traits/motion_controller.rs` - Motion interface
- `src/traits/camera_sensor.rs` - Camera interface
- `src/traits/tunable_laser.rs` - Laser interface

### Configuration

- Figment-based configuration system for hardware parameters
- See `src/config/v4_config.rs`

## Future Enhancements

Potential improvements:

1. **Remote Execution:** Execute tests directly on maitai-eos via SSH
2. **Continuous Integration:** GitHub Actions workflow for regular hardware validation
3. **Performance Trending:** Track performance metrics over time
4. **Automated Recovery:** Self-healing for common failure modes
5. **Extended Diagnostics:** Detailed error logs and telemetry
6. **Load Testing:** Stress tests for sustained operation
7. **Multi-Hardware:** Parallel testing of multiple instruments

## License

Part of rust-daq V4 architecture project.

## Contact

For questions about the hardware validation framework, refer to:
- Project documentation: `ARCHITECTURE.md`
- Actor implementations: `src/actors/`
- Test framework: `tests/hardware_validation_test.rs`
