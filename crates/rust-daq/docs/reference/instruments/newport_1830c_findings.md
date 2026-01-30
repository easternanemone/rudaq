# Newport 1830-C Hardware Testing Findings

## Summary

**Status**: ✅ **SUCCESSFUL** - Newport 1830-C fully integrated and operational
**Date**: 2025-11-26 (Updated)
**Port**: `/dev/ttyS0` (Native RS-232)
**Baud Rate**: 9600, 8N1
**Flow Control**: **NONE** (no RTS/CTS required)

## Recent Updates (2025-11-26)

**Discovered additional working commands:**
- `W?` - Query wavelength (returns 4-digit nm, e.g., `0800` for 800nm)
- `Wxxxx` - Set wavelength (e.g., `W0800` for 800nm)
- `R?` - Query range setting (returns 1-8)
- `U?` - Query units setting (returns 0=W, 1=dBm, 2=dB)

**Driver updates:**
- Added `WavelengthTunable` trait implementation
- Added `query_wavelength()`, `set_wavelength_nm()` methods
- Added `query_range()`, `query_units()` methods
- Fixed `send_config_command()` to clear buffer after commands

## ✅ Successful Integration

### Working Configuration
```toml
[instruments.newport_1830c]
type = "newport_1830c"
name = "Newport 1830-C Power Meter"
port = "/dev/ttyS0"  # Native RS-232, NOT USB!
baud_rate = 9600
attenuator = 0  # 0=off, 1=on
filter = 2      # 1=Slow, 2=Medium, 3=Fast
polling_rate_hz = 2.0
```

### Command Protocol (from Newport 1830-C Manual)

**The Newport 1830-C uses SIMPLE single-letter commands, NOT SCPI!**

#### Working Commands:
- `D?` - Data Query (power reading) - **Primary command**
- `A0` / `A1` / `A?` - Attenuator off/on/query
- `F1` / `F2` / `F3` / `F?` - Filter Slow/Medium/Fast/query
- `W?` - Wavelength query (returns 4-digit nm, e.g., `0800`)
- `Wxxxx` - Wavelength set (e.g., `W0800` for 800nm)
- `R?` - Range query (returns 1-8)
- `U?` - Units query (returns 0=W, 1=dBm, 2=dB)
- `B0` / `B1` / `B?` - Beep off/on/query
- `G0` / `G1` / `G?` - Hold/Go/query
- `CS` - Clear Status Byte Register
- `E0` / `E1` - Echo off/on (default: off for programmatic control)

#### Terminator
- **LF only** (`\n`), NOT CR+LF

#### Response Format
- **Watts mode (U1)**: Scientific notation (e.g., `5E-9`, `+.75E-9`)
- **dBm mode (U2)**: Decimal notation (e.g., `-15.24`)
- **dB mode (U3)**: Decimal notation (e.g., `3.02`)
- Configuration commands (A0, A1, F1, F2, F3): **NO RESPONSE**
- Query commands (D?, A?, F?): Return values

**CRITICAL**: The driver sets the device to Watts mode (U1) on initialization
to ensure consistent scientific notation parsing. If the device is in dBm mode,
`D?` returns decimal format which would cause incorrect value interpretation.

### Test Results
```bash
Port: /dev/ttyS0
Baud: 9600
Flow Control: NONE

# Power readings (successful):
D? → "5E-9"      (5 nanowatts)
D? → "+.75E-9"   (0.75 nanowatts)
D? → "+.75E-9"   (stable reading)
```

### rust-daq Integration Log
```
[INFO] Connecting to Newport 1830-C: newport_1830c
[INFO] Set attenuator to 0
[INFO] Set filter to 2
[INFO] Newport 1830-C 'newport_1830c' connected successfully
```

## Critical Discovery: Hardware Flow Control

**Different instruments have different flow control requirements!**

| Instrument | Port | Baud | Flow Control Required? |
|------------|------|------|----------------------|
| Newport 1830-C | `/dev/ttyS0` | 9600 | ❌ NO |
| ESP300 | `/dev/ttyUSB1` | 19200 | ✅ YES (RTS/CTS) |
| MaiTai | `/dev/ttyUSB5` | 9600 | ⚠️ Added, needs verification |
| Elliptec | `/dev/ttyUSB0` | 9600 | ⚠️ Added, needs verification |

**ESP300 Test Results:**
```bash
# Without RTS/CTS - NO RESPONSE
echo -ne "*IDN?\r\n" > /dev/ttyUSB1
# (no response)

# With RTS/CTS - SUCCESS!
stty -F /dev/ttyUSB1 19200 crtscts
echo -ne "*IDN?\r\n" > /dev/ttyUSB1
# Response: "ESP300 Version 3.04 07/27/01"
```

## Why Previous Tests Failed

### 1. Wrong Commands (FIXED)
- ❌ Used SCPI: `PM:Power?`, `PM:Lambda?`, `PM:Units?`
- ✅ Correct: `D?`, `A?`, `F?` (simple single-letter commands)

### 2. Wrong Assumptions (CORRECTED 2025-11-26)
- ❌ Previously assumed wavelength/units weren't configurable via commands
- ✅ **Newport 1830-C DOES support wavelength commands** (W? and Wxxxx)
- `W?` returns 4-digit wavelength (e.g., "0800" for 800nm)
- `Wxxxx` sets wavelength (e.g., "W0800" for 800nm)
- Units (U?) is query-only - must be set on physical meter

### 3. Wrong Response Handling (FIXED)
- ❌ Expected responses from `A0`, `A1`, `F1`, `F2`, `F3`
- ✅ Configuration commands are **write-only** (no response)
- Added `send_config_command()` method for write-only commands

### 4. Initially Wrong Port (FIXED)
- ❌ Tested USB ports (`/dev/ttyUSB0-5`)
- ✅ Newport is on native RS-232: `/dev/ttyS0`

## Code Implementation

### Driver Structure
```rust
// Query power (expects response)
async fn send_command_async(&self, command: &str) -> Result<String>

// Configure settings (no response expected)
async fn send_config_command(&self, command: &str) -> Result<()>
```

### Power Reading Loop
```rust
// Poll at 2 Hz
match instrument.send_command_async("D?").await {
    Ok(response) => {
        // Parse scientific notation: "5E-9" → 5e-9 watts
        let value = parse_power_response(&response)?;
        broadcast(DataPoint { value, unit: "W" });
    }
}
```

### Parameter Control
```rust
// Attenuator: write-only command
self.send_config_command("A0").await?;  // Off
self.send_config_command("A1").await?;  // On

// Filter: write-only command
self.send_config_command("F1").await?;  // Slow
self.send_config_command("F2").await?;  // Medium
self.send_config_command("F3").await?;  // Fast
```

## Port Mapping (Verified)

Based on `/dev/serial/by-id/` and hardware testing:
- `/dev/ttyUSB0` → FTDI FT230X - **Elliptec bus**
- `/dev/ttyUSB1` → FTDI 4-port adapter - **ESP300**
- `/dev/ttyUSB2-4` → FTDI 4-port adapter - (available)
- `/dev/ttyUSB5` → Silicon Labs CP2102 - **MaiTai laser**
- `/dev/ttyS0` → Native RS-232 - **Newport 1830-C** ✅

## Next Steps

### Completed ✅
- [x] Newport 1830-C driver rewritten with correct protocol
- [x] Configuration updated to use /dev/ttyS0
- [x] Hardware flow control added to MaiTai and Elliptec
- [x] Newport successfully connecting and streaming data
- [x] ESP300 flow control requirements verified

### Remaining Tasks
- [ ] Test MaiTai laser with hardware flow control (blocked on keyswitch - needs REMOTE mode)
- [ ] Test Elliptec rotators with hardware flow control
- [ ] Verify all instruments connect simultaneously
- [ ] Document power reading stability over time
- [ ] Test attenuator and filter parameter changes during operation
- [x] Create operator guide with Newport command reference (done 2025-11-26)
- [x] Add wavelength get/set commands (done 2025-11-26)
- [x] Add range/units query commands (done 2025-11-26)
- [x] Fix GUI unit normalization bug (done 2026-01-22)

## GUI Unit Handling (2026-01-22)

### Problem Solved

The GUI power meter panel was showing readings that reset to zero or displayed
incorrect values. This was caused by a units mismatch:

- **Newport 1830-C returns**: Watts (e.g., `5E-9` = 5 nanowatts)
- **gRPC ReadValueResponse**: Includes `value` and `units` fields
- **GUI (before fix)**: Stored raw value as milliwatts without conversion

For a 5 nW signal (5×10⁻⁹ W), the GUI displayed `0.0000 mW` because it treated
the raw Watts value as if it were already in milliwatts.

### Solution

The GUI now normalizes all readings to milliwatts using the `units` field from
the gRPC response. See `power_meter_panel.rs::normalize_power_to_mw()`.

**Data flow:**
```
Newport → "5E-9" (Watts)
        ↓
Driver  → f64: 5e-9
        ↓
gRPC    → ReadValueResponse { value: 5e-9, units: "W" }
        ↓
GUI     → normalize_power_to_mw(5e-9, "W") = 5e-6 mW
        ↓
Display → "5.0000 µW" (auto-scaled)
```

### Unit Conversion Table

| Input Units | Conversion to mW |
|-------------|------------------|
| W           | × 1000           |
| mW          | × 1 (no change)  |
| µW, uW      | ÷ 1000           |
| nW          | ÷ 1,000,000      |
| "" (empty)  | × 1000 (assume W)|

### Display Auto-Scaling

The gauge display automatically scales based on signal magnitude:

| Power Level | Display Unit |
|-------------|--------------|
| ≥ 1000 mW   | W            |
| ≥ 1 mW      | mW           |
| < 1 mW      | µW           |

### Device Metadata Registration

The Newport 1830-C is registered with `measurement_units: "W"` in the device
registry (`hardware/src/registry.rs`). This metadata is returned by the
gRPC server in `ReadValueResponse.units`.

## Operator Notes

### Newport 1830-C Quick Reference

**Power Monitoring:**
```bash
# Current power reading
D?
# Returns: "1.23E-6" (1.23 microwatts)
```

**Wavelength Control (NEW 2025-11-26):**
```bash
W?      # Query current wavelength
        # Returns: "0800" (800nm)
W0800   # Set wavelength to 800nm
W1064   # Set wavelength to 1064nm
W0532   # Set wavelength to 532nm
```

**Range and Units Query:**
```bash
R?      # Query range setting (returns 0=auto, 1-8)
U?      # Query units setting (returns 1=W, 2=dBm, 3=dB, 4=REL) - 1-indexed!
```

**Set Units (1-indexed):**
```bash
U1      # Set to Watts (scientific notation response)
U2      # Set to dBm (decimal response)
U3      # Set to dB (decimal response)
U4      # Set to REL (decimal response)
```

**Attenuator Control:**
```bash
A0  # Disable attenuator
A1  # Enable attenuator
A?  # Query attenuator state (returns 0 or 1)
```

**Filter Control:**
```bash
F1  # Slow filter (most stable, slowest response)
F2  # Medium filter (balanced)
F3  # Fast filter (fastest response, more noise)
F?  # Query filter setting (returns 1, 2, or 3)
```

**System Control:**
```bash
G0  # Hold (pause measurements)
G1  # Go (resume measurements)
CS  # Clear status byte register
```

### Troubleshooting

**If Newport doesn't respond:**
1. Check physical connection to RS-232 port (not USB!)
2. Verify meter is powered on
3. Check port: `ls -l /dev/ttyS0`
4. **Do NOT enable hardware flow control** for Newport
5. Verify baud rate: 9600 (check meter settings)

**Expected Response Times:**
- Power query (D?): < 500ms
- Configuration commands (A0, F2): No response (write-only)

## References

- Newport 1830-C User Manual (commands documented in Section 5.4)
- Working PyMoDAQ plugin: `pymodaq_plugins_urashg/daq_0Dviewer_Newport1830C.py`
- Hardware test scripts: `/tmp/test_newport.sh` on remote machine
- Integration test: `tests/newport_1830c_hardware_test.rs`