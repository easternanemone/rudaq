# Elliptec ELL14 Hardware Testing Findings

## Summary

**Status**: ✅ **WORKING** - All three rotators operational with bus-centric API
**Last Updated**: 2025-01-06
**Port**: `/dev/ttyUSB1` (FTDI FT230X)
**Baud Rate**: 9600, 8N1, no flow control
**Device Addresses**: 2, 3, 8
**Known Issue**: Rotator 2 occasionally returns GS02 (mechanical timeout) - suspected hardware issue

## Driver Implementation - COMPLETE ✅

### Bus-Centric API (Ell14Bus)

The ELL14 uses RS-485 multidrop architecture where all devices share one serial connection. The `Ell14Bus` struct enforces this model:

```rust
use daq_hardware::drivers::ell14::Ell14Bus;
use daq_hardware::capabilities::Movable;

// Open the RS-485 bus (one connection for all devices)
let bus = Ell14Bus::open("/dev/ttyUSB1").await?;

// Get calibrated device handles (queries device for pulses/degree)
let rotator_2 = bus.device("2").await?;
let rotator_3 = bus.device("3").await?;
let rotator_8 = bus.device("8").await?;

// All devices share the connection - no contention
rotator_2.move_abs(45.0).await?;
rotator_3.move_abs(90.0).await?;

// Discover all devices on the bus
let devices = bus.discover().await?;
for dev in devices {
    println!("Found {} at address {}", dev.info.device_type, dev.address);
}
```

**Key Methods:**
- `Ell14Bus::open(port)` - Opens the RS-485 bus
- `bus.device(addr)` - Gets a calibrated device handle (queries firmware for pulses/degree)
- `bus.device_uncalibrated(addr)` - Gets device with default calibration (faster)
- `bus.discover()` - Scans all 16 addresses to enumerate devices

### Device-Specific Calibration

Each ELL14 unit stores calibration data in firmware. The driver queries this via the `IN` command:

```rust
// bus.device() automatically queries calibration
let rotator = bus.device("2").await?;

// Or manually query
let info = rotator.get_device_info().await?;
println!("Pulses/degree: {:.4}", rotator.get_pulses_per_degree());
```

**IN Command Response Format** (varies by firmware):
- Older firmware (v15-v17): 30 data chars
- Newer firmware: 33 data chars

The driver auto-detects the format and parses accordingly.

### Protocol Implementation

**Communication Parameters** (per manual):
- Baud rate: 9600
- Data bits: 8
- Stop bits: 1
- Parity: None
- Flow control: **None** (no handshake)

**Command Format**:
```
[Address][Command][Parameter (optional)]
```

Examples:
- `2in` - Get info from device at address 2
- `2gp` - Get position from device at address 2
- `2ma12345678` - Move device 2 to absolute position 0x12345678

## Hardware Testing Results

### Test Results (2025-01-06)

All hardware tests pass on the remote machine (maitai@100.117.5.12):

| Test | Result | Notes |
|------|--------|-------|
| `test_all_rotators_respond_to_position_query` | ✅ PASS | All 3 rotators respond |
| `test_stability_short` (60s) | ✅ PASS | 186 queries, 100% success rate |
| `test_simultaneous_movement_all_three` | ✅ PASS | All 3 moved concurrently |
| `test_simultaneous_movement_two_devices` | ✅ PASS | Rot3 + Rot8 successful |

**Stability Test Results:**
- Duration: 60 seconds
- Total queries: 186
- Success rate: 100%
- Position stability: std dev < 0.002°

**Running Tests:**
```bash
# On remote machine (maitai)
cargo test --features "hardware_tests,instrument_thorlabs" \
  --test hardware_elliptec_validation -- --nocapture
```

### Known Issues

**Rotator 2 (Address 2):** Occasionally returns GS02 status (mechanical timeout) during move operations. This appears to be a hardware issue (possibly physical obstruction or motor problem), not a software issue. Rotators 3 and 8 work consistently.

## Port Mapping

| Device | Port | Adapter | Status |
|--------|------|---------|--------|
| ELL14 Rotators (2, 3, 8) | `/dev/ttyUSB1` | FTDI FT230X | ✅ Working |
| Newport 1830-C | `/dev/ttyS0` | Native RS-232 | ✅ Working |
| ESP300 | `/dev/ttyUSB1` | FTDI 4-port | ✅ Working |
| MaiTai | `/dev/ttyUSB5` | CP2102 | ✅ Working |

## Deprecated API

The following constructors are deprecated (open dedicated ports instead of sharing):

```rust
// DEPRECATED - Opens dedicated port (fails on multidrop)
#[deprecated(since = "0.2.0")]
Ell14Driver::new(port, addr)
Ell14Driver::new_async(port, addr)
Ell14Driver::new_async_with_device_calibration(port, addr)
```

Use `Ell14Bus` instead for all new code.

## References

- Elliptec Protocol Manual: https://www.thorlabs.com/Software/Elliptec/Communications_Protocol/ELLx%20modules%20protocol%20manual_Issue7.pdf
- Manual specs: 9600 baud, 8N1, no parity, no handshake
- IN command response: 30-33 bytes depending on firmware version
- Driver implementation: `crates/daq-hardware/src/drivers/ell14.rs`
- Hardware tests: `crates/rust-daq/tests/hardware_elliptec_validation.rs`
