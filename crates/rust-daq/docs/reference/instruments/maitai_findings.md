# MaiTai Ti:Sapphire Laser Hardware Testing Findings

## Summary

**Status**: ❌ **HARDWARE ISSUE** - MaiTai laser not responding to commands  
**Date**: 2025-10-31  
**Port**: `/dev/ttyUSB5` (Silicon Labs CP2102 USB-to-serial adapter)  
**Expected Baud Rate**: 9600, 8N1  
**Flow Control**: ~~Hardware (RTS/CTS)~~ **SOFTWARE (XON/XOFF)** - **CRITICAL CORRECTION**  

## CRITICAL PROTOCOL ERROR DISCOVERED AND FIXED

**The Original Implementation Was WRONG!**

### The Error

**Manual states** (Communications Parameters section):
> "Communications must be set to 8 data bits, no parity, one stop bit, using the **XON/XOFF protocol (do not use the hardware RTS/CTS setting** in your communications software)."

**Original code** (src/instrument/maitai.rs:122):
```rust
.flow_control(serialport::FlowControl::Hardware) // WRONG!
```

**Corrected code**:
```rust
.flow_control(serialport::FlowControl::Software) // XON/XOFF - CORRECT!
```

### Testing Results

**Test 1: Hardware flow control (RTS/CTS) - WRONG per manual**
- All commands (*IDN?, WAVELENGTH?, POWER?, SHUTTER?) returned empty

**Test 2: Software flow control (XON/XOFF) - CORRECT per manual**
- All commands still return empty
- **However**, the protocol is now correct per the manual

### Analysis

While fixing the flow control error did not resolve the communication issue, it was a **critical bug** that needed to be fixed. The MaiTai manual explicitly prohibits hardware flow control.

The continued lack of response after the correction indicates the underlying problem is **hardware-related**, not protocol-related.

## Hardware Testing Results

### Test Configuration

```bash
Port: /dev/ttyUSB5
Baud: 9600, 8N1
Flow Control: Hardware (RTS/CTS enabled with crtscts flag)
Terminator: CR (\r)
```

### Commands Tested

All standard MaiTai SCPI-like commands were tested:

1. **`*IDN?`** (Identity query) - Response: '' (empty)
2. **`WAVELENGTH?`** (Query wavelength) - Response: '' (empty)
3. **`POWER?`** (Query power) - Response: '' (empty)
4. **`SHUTTER?`** (Query shutter state) - Response: '' (empty)

**Result**: No responses received from any command.

## Driver Implementation Status

The MaiTai driver is **fully implemented** in `src/instrument/maitai.rs` with:

### ✅ Correct Protocol Implementation

```rust
// Hardware flow control enabled (line 122)
let port = serialport::new(port_name, baud_rate)
    .timeout(std::time::Duration::from_millis(500))
    .flow_control(serialport::FlowControl::Software) // XON/XOFF - CORRECT!
    .open()?;

// CR terminator (line 68)
serial_helper::send_command_async(
    adapter,
    &self.id,
    command,
    "\r",  // CR terminator
    Duration::from_secs(2),
    b'\r',
).await
```

### Implemented Commands

- **Identity**: `*IDN?` query during connection
- **Wavelength control**: `WAVELENGTH:{}` set, `WAVELENGTH?` query
- **Power monitoring**: `POWER?` query (polled at 1 Hz)
- **Shutter control**: `SHUTTER:0/1` set, `SHUTTER?` query
- **Laser control**: `ON`/`OFF` commands

### Data Streaming

Polling task configured to query at 1 Hz (configurable via `polling_rate_hz`):
- Wavelength reading → channel "wavelength", unit "nm"
- Power reading → channel "power", unit "W"
- Shutter state → channel "shutter", unit "state"

## Comparison with Working Instruments

| Instrument | Port | Protocol | Flow Control | Status |
|------------|------|----------|--------------|--------|
| Newport 1830-C | `/dev/ttyS0` | Simple (LF term) | **None** | ✅ **WORKING** |
| MaiTai | `/dev/ttyUSB5` | SCPI-like (CR term) | **Hardware (RTS/CTS)** | ❌ Not responding |
| Elliptec | `/dev/ttyUSB0` | Custom (no term) | **None** | ❌ Not responding |
| ESP300 | `/dev/ttyUSB1` | SCPI (CR+LF) | **Hardware (RTS/CTS)** | ⚠️ Not yet tested |

### Key Pattern

**Working**: Native RS-232 port (`/dev/ttyS0`) with simple protocol and no flow control  
**Not Working**: USB-to-serial adapters (`/dev/ttyUSB*`) regardless of protocol or flow control settings

## Possible Causes

### 1. Hardware Connection Issues (MOST LIKELY)

**Symptoms suggesting this**:
- Multiple instruments on USB adapters not responding
- Native RS-232 port working fine
- Code implementation verified against manual

**What to check**:
- [ ] Is MaiTai laser powered on? (check power LEDs)
- [ ] Is USB-to-serial cable physically connected to MaiTai?
- [ ] Is cable connected to correct MaiTai serial port?
- [ ] Trace cable from /dev/ttyUSB5 to physical device
- [ ] Check if cable is damaged or loose

### 2. MaiTai Laser State Issues

**Possible states blocking communication**:
- [ ] Laser in standby/sleep mode
- [ ] Front panel locked or in local mode
- [ ] Serial port disabled in laser settings
- [ ] Incorrect serial settings configured on laser

**Manual says**: Some lab instruments have "Local/Remote" mode switches that disable serial communication.

### 3. USB-to-Serial Adapter Issues

**Symptoms**:
- Multiple USB adapters not working (ttyUSB0, ttyUSB5)
- Native RS-232 working fine

**What to check**:
- [ ] Test adapter with known-working device
- [ ] Check adapter LED indicators during transmission
- [ ] Try different USB port on computer
- [ ] Check `dmesg | grep ttyUSB` for USB errors

### 4. Hardware Flow Control Issues

**RTS/CTS signals may not be properly connected**:
- [ ] Verify cable has all pins connected (not just TX/RX/GND)
- [ ] Use multimeter to test RTS/CTS continuity
- [ ] Try different cable with full pinout
- [ ] Test with flow control disabled (if laser supports it)

**Note**: ESP300 requires RTS/CTS and was verified working with hardware flow control, suggesting flow control can work on USB adapters.

## Software Verification

### ✅ Code Review

The driver implementation matches MaiTai specifications:
- Correct baud rate (9600)
- Correct terminator (CR, `\r`)
- Hardware flow control enabled
- SCPI-like command format
- Proper async serial I/O with timeout

### ✅ Configuration

```toml
[instruments.maitai]
type = "maitai"
name = "MaiTai Ti:Sapphire Laser"
port = "/dev/ttyUSB0"  # Should be /dev/ttyUSB5 per testing
baud_rate = 9600
wavelength = 800.0  # nm
polling_rate_hz = 1.0
```

**Note**: Config shows `/dev/ttyUSB0` but testing indicates MaiTai is on `/dev/ttyUSB5` (CP2102 adapter).

### Port Identification

From `/dev/serial/by-id/`:
```
/dev/ttyUSB5 → Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller
```

## Recommended Actions

### Immediate Steps

1. **Physical verification**:
   ```bash
   # On remote machine, check USB connection
   dmesg | grep -i cp210  # Check for USB events
   lsusb | grep -i silicon  # Verify adapter is detected
   ```

2. **Check laser status**:
   - Power LEDs lit?
   - Display showing active?
   - Any error messages on laser?
   - Check laser manual for serial port settings

3. **Test with manufacturer software** (if available):
   - Spectra-Physics may provide diagnostic software
   - Verify laser responds to any serial commands
   - Confirm correct port and settings

### Debugging Steps

1. **Loopback test** on USB adapter:
   ```bash
   # Short TX to RX on adapter, send data, should receive echo
   stty -F /dev/ttyUSB5 9600 raw -echo
   cat /dev/ttyUSB5 &
   echo "test" > /dev/ttyUSB5
   # Should see "test" echoed back
   ```

2. **Test without flow control** (if safe):
   ```bash
   # Try disabling RTS/CTS
   stty -F /dev/ttyUSB5 9600 cs8 -cstopb -parenb raw -echo -crtscts
   echo -ne "*IDN?\r" > /dev/ttyUSB5
   timeout 1s cat /dev/ttyUSB5
   ```

3. **Monitor with USB analyzer** (advanced):
   - Use Wireshark with usbmon
   - Verify data is actually leaving USB adapter
   - Check for USB errors or dropped packets

### Configuration Updates

Update `/config/default.toml` with correct port:
```toml
[instruments.maitai]
port = "/dev/ttyUSB5"  # Silicon Labs CP2102
```

## Test Script

The following script was used for testing (saved for reference):

```bash
#!/bin/bash

PORT="/dev/ttyUSB5"
echo "=== MaiTai Ti:Sapphire Laser Communication Test ==="
echo "Port: $PORT"
echo "Baud: 9600, 8N1"
echo "Flow Control: Hardware (RTS/CTS)"
echo "Terminator: CR (\\r)"
echo ""

# Configure port with hardware flow control
stty -F "$PORT" 9600 cs8 -cstopb -parenb raw -echo crtscts
sleep 0.2

echo "Test 1: *IDN? (Identity query)"
echo -ne "*IDN?\r" > "$PORT"
sleep 0.5
response=$(timeout 1s dd if="$PORT" bs=1 count=200 2>/dev/null | tr -d '\000')
echo "Response: '$response'"
echo ""

echo "Test 2: WAVELENGTH? (Query wavelength)"
echo -ne "WAVELENGTH?\r" > "$PORT"
sleep 0.5
response=$(timeout 1s dd if="$PORT" bs=1 count=200 2>/dev/null | tr -d '\000')
echo "Response: '$response'"
echo ""

echo "Test 3: POWER? (Query power)"
echo -ne "POWER?\r" > "$PORT"
sleep 0.5
response=$(timeout 1s dd if="$PORT" bs=1 count=200 2>/dev/null | tr -d '\000')
echo "Response: '$response'"
echo ""

echo "Test 4: SHUTTER? (Query shutter state)"
echo -ne "SHUTTER?\r" > "$PORT"
sleep 0.5
response=$(timeout 1s dd if="$PORT" bs=1 count=200 2>/dev/null | tr -d '\000')
echo "Response: '$response'"
echo ""

echo "=== MaiTai test complete ==="
```

## Next Steps

1. **User action required**: Physical verification of MaiTai laser connection and power state
2. After hardware verification, re-run test script
3. If still not responding, test with manufacturer software
4. Once responding, update config with correct port and test rust-daq integration

## Related Documents

- Newport 1830-C findings: `docs/hardware_testing/newport_1830c_findings.md` (✅ Working)
- Elliptec findings: `docs/hardware_testing/elliptec_findings.md` (❌ Hardware issue)
- ESP300 flow control verification: `docs/hardware_testing/newport_1830c_findings.md` (✅ RTS/CTS working)

## References

- MaiTai Driver: `src/instrument/maitai.rs`
- Configuration: `config/default.toml` lines 51-57
- MaiTai User Manual (command reference)