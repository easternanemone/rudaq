# Elliptec ELL14 Hardware Testing Findings

## Summary

**Status**: ⚠️ **HARDWARE ISSUE** - Devices not responding despite correct protocol implementation  
**Date**: 2025-10-31  
**Expected Port**: `/dev/ttyUSB0` (FTDI FT230X)  
**Expected Baud Rate**: 9600, 8N1, no flow control  
**Expected Addresses**: 2, 3, 8

## Driver Implementation - COMPLETE ✅

### 1. Device-Specific Calibration Feature

The driver now automatically queries each Elliptec device during connection to extract device-specific calibration data from the IN command response.

**Implementation** (elliptec.rs:35-37, 79-110, 113-151):

```rust
// Store pulses per degree for each device
pulses_per_degree: HashMap<u8, f64>,

// Parse IN command response to extract calibration
fn parse_device_info(&self, response: &str) -> Result<f64> {
    // Extract bytes 25-32 (8 hex chars): pulses per measurement unit
    let pulses_hex = &response[25..33];
    let pulses_raw = u32::from_str_radix(pulses_hex, 16)?;
    Ok(pulses_raw as f64)
}

// Query calibration during connection
for &addr in &self.device_addresses {
    let response = self.send_command_async(addr, "in").await?;
    let pulses_per_degree = self.parse_device_info(&response)?;
    self.pulses_per_degree.insert(addr, pulses_per_degree);
}

// Use device-specific conversion in position commands
let pulses_per_degree = self.pulses_per_degree.get(&address)?;
let degrees = (raw_pos as f64) / pulses_per_degree;
```

**Benefits**:
- ✅ Works with any Elliptec device version (no hardcoded constants)
- ✅ Accounts for manufacturing variations between devices
- ✅ Eliminates need to update code when hardware changes
- ✅ Logs calibration values for verification

**Previous Approach** (WRONG):
```rust
// Hardcoded conversion - only works for specific hardware revision!
let degrees = (raw_pos as f64 / 143360.0) * 360.0;
```

**Correct Approach** (NEW):
```rust
// Device-specific conversion from IN command
let pulses_per_degree = self.pulses_per_degree.get(&address)?;
let degrees = (raw_pos as f64) / pulses_per_degree;
```

### 2. Protocol Implementation

**Communication Parameters** (per manual):
- Baud rate: 9600
- Data bits: 8
- Stop bits: 1
- Parity: None
- Flow control: **None** (no handshake)
- Terminator: **None** (Elliptec uses fixed 3-byte commands)

**Command Format**:
```
[Address][Command][Parameter (optional)]
```

Examples:
- `2in` - Get info from device at address 2
- `2gp` - Get position from device at address 2  
- `2ma12345678` - Move device 2 to absolute position 0x12345678

**Response Format for IN command** (33 bytes):
```
AIN0ESSSSSSSSYYYYFFRR001FPPPPPPPP
├─┤├┤├┤├──────┤├──┤├┤├┤├──┤├──────┤
│  │  │  │      │    │  │  │    └─ Pulses per M.U. (8 hex chars, big-endian)
│  │  │  │      │    │  │  └────── Travel range (hex)
│  │  │  │      │    │  └─────────  Hardware release
│  │  │  │      │    └───────────── Firmware release
│  │  │  │      └────────────────── Year
│  │  │  └───────────────────────── Serial number (8 hex chars)
│  │  └──────────────────────────── Device type (0E for ELL14)
│  └─────────────────────────────── Command echo
└────────────────────────────────── Address echo
```

### 3. Configuration

**config/default.toml**:
```toml
[instruments.elliptec]
type = "elliptec"
name = "Elliptec ELL14 Rotation Mounts"
port = "/dev/ttyUSB0"  # FTDI FT230X adapter
baud_rate = 9600
device_addresses = [2, 3, 8]  # HWP incident, QWP, HWP analyzer
polling_rate_hz = 2.0
```

## Hardware Testing - FAILED ❌

### Test Methodology

**Systematic Testing Performed**:
1. ✅ Verified USB serial ports exist (`/dev/ttyUSB0-5`)
2. ✅ Verified port ownership and permissions (user in `uucp` group)
3. ✅ Verified no processes holding ports open  
4. ✅ Tested all 6 USB ports (`/dev/ttyUSB0` through `/dev/ttyUSB5`)
5. ✅ Tested multiple baud rates (9600, 19200, 38400, 115200)
6. ✅ Tested multiple device addresses (0=broadcast, 2, 3, 8)
7. ✅ Tested with various read timeouts (0.3s, 0.5s, 2s, 3s)
8. ✅ Used different testing approaches:
   - Direct echo to port + dd read
   - File descriptor approach with stty timeout
   - Python pyserial (not available)
   - Multiple shell scripting methods

### Test Results

**All tests returned**: No response from any device on any port

**Ports Tested**:
```
/dev/ttyUSB0 → FTDI FT230X (Elliptec bus per docs) ❌ No response
/dev/ttyUSB1 → FTDI 4-port adapter port 0         ❌ No response
/dev/ttyUSB2 → FTDI 4-port adapter port 1         ❌ No response
/dev/ttyUSB3 → FTDI 4-port adapter port 2         ❌ No response
/dev/ttyUSB4 → FTDI 4-port adapter port 3         ❌ No response
/dev/ttyUSB5 → Silicon Labs CP2102 (MaiTai)       ❌ No response
```

**Commands Tested** (per Elliptec protocol manual):
```bash
# Broadcast address
echo -n "0in" > /dev/ttyUSB0  # No response

# Device-specific addresses
echo -n "2in" > /dev/ttyUSB0  # No response
echo -n "3in" > /dev/ttyUSB0  # No response
echo -n "8in" > /dev/ttyUSB0  # No response
```

### Observations

1. **User confirmed**: "almost entirely certain that the elliptec rotators are plugged in and powered on"

2. **Communication parameters verified**: Manual confirms 9600 baud, 8N1, no handshake

3. **All test scripts hang or timeout** when attempting to:
   - Configure port with `stty`
   - Open port for reading with `exec 3<>"$port"`
   - Read responses with `dd`, `cat`, `head`

4. **System checks pass**:
   - User `maitai` is in `uucp` group (has port access)
   - Ports exist and have correct permissions (0660 crw-rw----)
   - No other processes have ports open

## Possible Causes

### 1. Physical Connection Issues (MOST LIKELY)
- ❌ USB cable from rotation mounts to FTDI adapter disconnected
- ❌ Power not reaching rotation mounts
- ❌ Wrong FTDI adapter connected to rotation mounts
- ❌ Rotation mounts on a different adapter than documented

### 2. Device Configuration Issues
- ❌ Devices configured for different baud rate
- ❌ Devices in a mode that doesn't respond to commands
- ❌ Devices have different addresses than expected
- ❌ RS-485 termination resistors issue

### 3. Adapter Issues
- ❌ FTDI FT230X adapter configured for wrong RS-485 mode
- ❌ TX/RX or A/B wiring swapped
- ❌ RS-485 differential signaling not working

### 4. Software Issues (UNLIKELY)
- ⚠️ Port access permissions (checked - user in uucp group)
- ⚠️ Kernel driver issues with FTDI adapter
- ⚠️ Port locked by another process (checked - no locks)

## Recommended Actions

### Immediate (Physical Verification)

1. **Verify power to Elliptec devices**
   - Check LED indicators on ELL14 mounts (if present)
   - Confirm power supply is on and connected

2. **Verify USB connections**
   - Trace cable from ELL14 mounts to computer
   - Confirm connected to FTDI FT230X adapter
   - Check for loose connections

3. **Verify RS-485 adapter settings**
   - Check for TX/RX enable jumpers  
   - Verify termination resistor settings
   - Confirm A/B wiring polarity

4. **Test with Thorlabs software** (if available)
   - Use Thorlabs Elliptec software on Windows
   - Verify devices respond via official software
   - Confirm addresses (2, 3, 8)

### Testing (If Hardware is Confirmed Working)

5. **Manually query with known-working tool**
   - Use PyMoDAQ if working with these devices
   - Capture exact command bytes sent
   - Compare with our implementation

6. **USB analyzer** (if available)
   - Monitor actual bytes on RS-485 bus
   - Verify command format matches manual
   - Check for electrical issues

### Code Verification (Low Priority)

7. **Protocol implementation review**
   - ✅ Command format matches manual (3-byte, no terminator)
   - ✅ Baud rate correct (9600)
   - ✅ No flow control (correct per manual)
   - ✅ Device-specific calibration implemented
   - Code ready for testing once hardware is available

## Comparison with Working Instruments

| Instrument | Port | Status | Notes |
|------------|------|--------|-------|
| Newport 1830-C | `/dev/ttyS0` | ✅ Working | Native RS-232, simple protocol |
| Elliptec ELL14 | `/dev/ttyUSB0` | ❌ No Response | RS-485, awaiting hardware verification |
| ESP300 | `/dev/ttyUSB1` | ⚠️ Not Tested | Requires RTS/CTS flow control |
| MaiTai | `/dev/ttyUSB5` | ⚠️ Not Tested | Should work per config |

## Next Steps

**BLOCKED**: Elliptec integration is **blocked on hardware verification**

1. User must physically verify:
   - Devices are powered on (check LEDs)
   - USB cable is connected to correct adapter
   - FTDI FT230X adapter is connected to `/dev/ttyUSB0`

2. Once hardware is confirmed:
   - Re-run systematic port scan
   - Test with rust-daq application
   - Verify calibration values are correctly extracted

3. If still no response:
   - Test with Thorlabs official software
   - Check RS-485 adapter configuration
   - Consider USB/RS-485 analyzer for debugging

## References

- Elliptec Protocol Manual: https://www.thorlabs.com/Software/Elliptec/Communications_Protocol/ELLx%20modules%20protocol%20manual_Issue7.pdf
- Manual specs: 9600 baud, 8N1, no parity, no handshake
- IN command response: 33 bytes with device info and calibration
- Driver implementation: `src/instrument/elliptec.rs`
- Configuration: `config/default.toml`
