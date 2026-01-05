# Hardware Communication Reference - Laboratory Instruments

**System**: maitai@100.117.5.12 (laboratory hardware system)
**Last Updated**: 2025-11-19 08:25 CST
**Status**: 5/6 devices operational (83% success rate)

---

## Overview

This document provides definitive communication parameters for all laboratory instruments connected to the rust-daq system. Use this as the authoritative reference for hardware configuration, driver development, and troubleshooting.

### Quick Reference Table

| Device | Port | Status | Baud | Flow Control | Response Time | HWID Match |
|--------|------|--------|------|--------------|---------------|------------|
| MaiTai Laser | /dev/ttyUSB5 | ✅ Working | 9600 | XON/XOFF | 2+ sec | CP2102:20230228-906 |
| Newport 1830C | /dev/ttyS0 | ✅ Working | 9600 | None | ~500ms | Native RS-232 |
| Elliptec ELL14 (×3) | /dev/ttyUSB0 | ✅ Working | 9600 | None | ~200ms | FT230X:DK0AHAJZ |
| Newport ESP300 | /dev/ttyUSB1 | ❌ Powered Off | 19200 | RTS/CTS | N/A | FT4232H:FT1RALWL |

---

## Device 1: Spectra-Physics MaiTai Ti:Sapphire Laser

### Hardware Identification

**Port**: `/dev/ttyS0`
**Status**: ✅ **OPERATIONAL** (verified 2025-11-19)

**USB Hardware ID (HWID)**:
```
Vendor:        Silicon Labs (VID: 10c4)
Model:         CP2102 USB to UART Bridge Controller (PID: ea60)
Serial Number: 20230228-906
USB Device:    Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_20230228-906
Chip:          CP210x UART Bridge
```

**HWID Match String** (for udev rules):
```bash
ID_VENDOR_ID="10c4"
ID_MODEL_ID="ea60"
ID_SERIAL_SHORT="20230228-906"
# Or combined:
ID_SERIAL="Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_20230228-906"
```

### Serial Communication Parameters

| Parameter | Value | Notes |
|-----------|-------|-------|
| **Baud Rate** | 9600 | Fixed, configurable on device but default is 9600 |
| **Data Bits** | 8 | Standard |
| **Stop Bits** | 1 | Standard |
| **Parity** | None | No parity checking |
| **Flow Control** | **Software (XON/XOFF)** | **REQUIRED** - device will not respond without it |
| **Terminator (TX)** | CR (`\r`, 0x0D) | Carriage return only, NOT CRLF |
| **Terminator (RX)** | LF (`\n`, 0x0A) | Line feed in responses |

### Protocol Syntax

**Command Format**: SCPI-like ASCII commands
```
*IDN?<CR>        Query device identification
WAVELENGTH?<CR>  Query current wavelength
POWER?<CR>       Query output power
SHUTTER?<CR>     Query shutter status (0=closed, 1=open)
```

**Response Format**: ASCII text terminated with LF
```
Example: *IDN? → "Spectra Physics,MaiTai,3227/51054/40856,0245-2.00.34 / CD00000019 / 214-00.004.057\n"
```

### Critical Timing Requirements

⚠️ **CRITICAL**: MaiTai has **exceptionally slow response time**

| Timing Parameter | Value | Reason |
|------------------|-------|--------|
| **Response Time** | **2000-3000ms** | Device takes 2+ seconds to process commands |
| **Port Timeout** | **3000ms minimum** | Must wait for full response |
| **Post-Write Delay** | **2000ms minimum** | Allow device processing time before read |
| **Inter-Command Delay** | 500ms recommended | Prevent buffer overflow |

**Discovery Tool Configuration**:
```rust
serialport::new("/dev/ttyUSB5", 9600)
    .timeout(Duration::from_millis(3000))  // CRITICAL: 3 second timeout
    .flow_control(serialport::FlowControl::Software)  // XON/XOFF required
    .open()?;

port.write_all(b"*IDN?\r")?;
port.flush()?;  // CRITICAL: Must flush before waiting
thread::sleep(Duration::from_millis(2000));  // Wait for device processing
```

### Validated Commands

| Command | Response Example | Purpose | Response Time |
|---------|-----------------|---------|---------------|
| `*IDN?\r` | `Spectra Physics,MaiTai,3227/51054/40856,...` | Device identification | 2000-3000ms |
| `WAVELENGTH?\r` | `820nm\n` | Current wavelength setting | 2000-3000ms |
| `POWER?\r` | `W\n` | Output power (response may be truncated) | 2000-3000ms |
| `SHUTTER?\r` | `0\n` or `1\n` | Shutter status (0=closed, 1=open) | 2000-3000ms |

### Device Information

**Manufacturer**: Spectra-Physics (Newport/MKS Instruments)
**Model**: MaiTai Ti:Sapphire Laser
**Serial Number**: 3227/51054/40856
**Firmware Version**: 0245-2.00.34 / CD00000019 / 214-00.004.057
**Wavelength Range**: 690-1040 nm (tunable)
**Current Wavelength**: 820 nm (as of 2025-11-19)

### Safety Notes

⚠️ **LASER SAFETY**:
- Class 4 laser - extreme eye and fire hazard
- Always verify `SHUTTER?` returns `0` (closed) before any testing
- NEVER send shutter open commands without proper safety procedures
- All discovery/testing commands are non-destructive queries only

---

## Device 2: Newport 1830-C Optical Power Meter

### Hardware Identification

**Port**: `/dev/ttyS0`
**Status**: ✅ **OPERATIONAL** (verified 2025-11-19)

**Hardware Type**: **Native RS-232 Port** (not USB adapter)
```
Device Node:   /dev/ttyS0
Type:          Built-in RS-232 serial port
Chipset:       16550A UART (or compatible)
Permissions:   crw-rw---- (root:uucp)
```

**No USB HWID** (native serial port - use port name `/dev/ttyS0` for identification)

### Serial Communication Parameters

| Parameter | Value | Notes |
|-----------|-------|-------|
| **Baud Rate** | 9600 | Configurable on device, default is 9600 |
| **Data Bits** | 8 | Standard |
| **Stop Bits** | 1 | Standard |
| **Parity** | None | No parity checking |
| **Flow Control** | **None** | No handshaking |
| **Terminator (TX)** | LF (`\n`, 0x0A) | Line feed only, **NOT** SCPI standard |
| **Terminator (RX)** | LF (`\n`, 0x0A) | Line feed in responses |

### Protocol Syntax

**Command Format**: Simple single-letter ASCII commands (**NOT SCPI**)
```
D?<LF>   Query current power reading
W?<LF>   Query wavelength setting
U?<LF>   Query units
```

**Response Format**: Scientific notation or simple ASCII
```
Example: D? → "+.11E-9\n"  (11 nanowatts in scientific notation)
Example: D? → "9E-9\n"     (9 nanowatts)
```

### Critical Timing Requirements

| Timing Parameter | Value | Reason |
|------------------|-------|--------|
| **Response Time** | **~500ms** | Fast response compared to MaiTai |
| **Port Timeout** | **1000ms** | Conservative timeout |
| **Post-Write Delay** | **100ms** | Minimal delay needed |

**Discovery Tool Configuration**:
```rust
serialport::new("/dev/ttyS0", 9600)
    .timeout(Duration::from_millis(1000))
    .flow_control(serialport::FlowControl::None)  // No flow control
    .open()?;

port.write_all(b"D?\n")?;  // Note: LF terminator, not CR
port.flush()?;
thread::sleep(Duration::from_millis(100));
```

### Validated Commands

| Command | Response Example | Purpose | Response Time |
|---------|-----------------|---------|---------------|
| `D?\n` | `+.11E-9\n` or `9E-9\n` | Power reading in watts (scientific notation) | ~500ms |
| `W?\n` | (not tested) | Wavelength setting | ~500ms |
| `U?\n` | (not tested) | Units setting | ~500ms |

### Response Parsing

**Scientific Notation Format**: `[sign][digit]E[sign][exponent]`
- Example: `+.11E-9` = 11 × 10^-9 W = 11 nanowatts
- Example: `9E-9` = 9 × 10^-9 W = 9 nanowatts
- Always contains letter 'E' (used for discovery matching)

**Rust Parsing**:
```rust
let response = "+.11E-9\n";
let value: f64 = response.trim().parse().unwrap();  // 0.00000000011
```

### Device Information

**Manufacturer**: Newport Corporation (MKS Instruments)
**Model**: 1830-C Optical Power Meter
**Interface**: Native RS-232 (9-pin Sub-D connector)
**Current Reading**: 11 nanowatts (as of 2025-11-19 08:20)
**Protocol Type**: Proprietary (NOT SCPI compliant)

### Important Notes

- ⚠️ **NOT SCPI**: Uses simple letter commands, not standard SCPI syntax
- ⚠️ **LF Terminator**: Unlike most devices, uses LF only (not CR or CRLF)
- ✅ **No Flow Control**: Simplest protocol of all devices
- ✅ **Fast Response**: ~500ms response time (much faster than MaiTai)
- 📍 **Native RS-232**: Requires proper RS-232 cable, not USB adapter

---

## Device 3: Thorlabs Elliptec ELL14 Rotation Mounts (Multidrop Bus)

### Hardware Identification

**Port**: `/dev/ttyUSB0`
**Status**: ✅ **OPERATIONAL** - **3 devices detected** on multidrop bus (verified 2025-11-19)

**USB Hardware ID (HWID)**:
```
Vendor:        FTDI (VID: 0403)
Model:         FT230X Basic UART (PID: 6015)
Serial Number: DK0AHAJZ
USB Device:    FTDI_FT230X_Basic_UART_DK0AHAJZ
Chip:          FT230X (Bridge I2C/SPI/UART/FIFO)
```

**HWID Match String** (for udev rules):
```bash
ID_VENDOR_ID="0403"
ID_MODEL_ID="6015"
ID_SERIAL_SHORT="DK0AHAJZ"
# Or combined:
ID_SERIAL="FTDI_FT230X_Basic_UART_DK0AHAJZ"
```

### Serial Communication Parameters

| Parameter | Value | Notes |
|-----------|-------|-------|
| **Baud Rate** | **9600 ONLY** | **NOT configurable** - strictly 9600 |
| **Data Bits** | 8 | Standard |
| **Stop Bits** | 1 | Standard |
| **Parity** | None | No parity checking |
| **Flow Control** | **None** | No handshaking |
| **Terminator (TX)** | **NONE** | Commands have no terminator |
| **Terminator (RX)** | CR+LF (`\r\n`, 0x0D 0x0A) | Responses end with CR then LF |

### Protocol Syntax - Multidrop Addressing

**⚠️ CRITICAL**: This is a **multidrop bus** with up to 16 addressable devices (0-F)

**Command Format**: `{ADDRESS}{2-char command}{optional hex data}`
```
0in       Get info from address 0
2in       Get info from address 2
3ma       Move absolute (address 3) - requires data bytes
```

**Address Range**: Single hex digit `0-9, A-F` (16 possible addresses)

**Response Format**: `{ADDRESS}{2-CHAR RESPONSE}{hex data}<CR><LF>`
```
Example: 2in → "2IN0E1140051720231701016800023000\r\n"
         │││└─ Hex data (model, serial, firmware, calibration)
         ││└── Response command (uppercase)
         │└─── Original address echoed back
         └──── Carriage Return + Line Feed
```

### Discovered Devices on Bus

**3 ELL14 devices found at addresses 2, 3, and 8**:

#### Device 1 (Address 2):
```
Address:  2
Response: 2IN0E1140051720231701016800023000
Model:    0E11 (ELL14)
Serial:   005172023 (manufacturing date: 2023)
```

#### Device 2 (Address 3):
```
Address:  3
Response: 3IN0E1140028420211501016800023000
Model:    0E11 (ELL14)
Serial:   002842021 (manufacturing date: 2021)
```

#### Device 3 (Address 8):
```
Address:  8
Response: 8IN0E1140060920231701016800023000
Model:    0E11 (ELL14)
Serial:   006092023 (manufacturing date: 2023)
```

### Critical Timing Requirements

| Timing Parameter | Value | Reason |
|------------------|-------|--------|
| **Response Time** | **~200ms per device** | Fast response time |
| **Port Timeout** | **500ms** | Conservative timeout |
| **Post-Write Delay** | **50-100ms** | Allow response to arrive |
| **Inter-Byte Timeout** | **2000ms** | **CRITICAL**: Packet discarded if gap >2 seconds |
| **Address Scan Delay** | **200ms per address** | When scanning bus |

⚠️ **CRITICAL PROTOCOL RULE**:
- If time between received bytes exceeds **2 seconds**, the packet is **discarded**
- Carriage return (CR, 0x0D) can be used to clear receive state machine and exit timeout error

### Bus Scanning Procedure

To discover all devices on the bus, scan addresses 0-F:

```rust
fn scan_elliptec_bus(port: &mut SerialPort) -> Vec<ElliptecDevice> {
    let mut devices = Vec::new();

    for addr in "0123456789ABCDEF".chars() {
        // Send info query to this address
        let cmd = format!("{}in", addr);
        port.write_all(cmd.as_bytes())?;
        port.flush()?;

        thread::sleep(Duration::from_millis(200));

        // Read response
        let mut buf = [0u8; 64];
        if let Ok(n) = port.read(&mut buf) {
            let response = String::from_utf8_lossy(&buf[..n]);

            // Valid response: starts with address and contains "IN"
            if response.starts_with(addr) && response.contains("IN") {
                devices.push(ElliptecDevice {
                    address: addr,
                    model: parse_model(&response),
                    serial: parse_serial(&response),
                    raw_response: response.trim_end().to_string(),
                });
            }
        }
    }

    devices
}
```

### Protocol Message Structure

**Message Header** (3 bytes):
```
Byte 1:    ADDRESS (0-F)
Byte 2-3:  COMMAND ID (2 ASCII characters)
           Examples: "in" (info), "ma" (move absolute), "ho" (home)
```

**Optional Data Packet** (variable length):
```
Byte 4-N:  HEX ASCII DATA
           Format depends on command
           Example: For "0A" (decimal 10) send ASCII bytes "30 41" (0x30 0x41)
```

**Response Structure**:
```
{ADDRESS}{COMMAND_UPPERCASE}{HEX_DATA}<CR><LF>
```

### Command Examples

| Command | Format | Purpose | Response |
|---------|--------|---------|----------|
| Info | `{ADDR}in` | Get device information | `{ADDR}IN{model}{serial}{firmware}...` |
| Home | `{ADDR}ho` | Home the device | `{ADDR}HO` + status |
| Get Position | `{ADDR}gp` | Get current position | `{ADDR}GP{position}` |
| Move Absolute | `{ADDR}ma{pos}` | Move to absolute position | `{ADDR}MA` + status |

**Note**: Commands are **lowercase** (host → device), responses are **UPPERCASE** (device → host)

### Device Information

**Manufacturer**: Thorlabs Inc.
**Model**: ELL14 (Elliptec rotation mount)
**Bus Type**: RS-232 multidrop (open drain signals)
**Pull-up Requirement**: 10kΩ to 3.3V (signal must be 0-3.3V range)
**Protocol Type**: Binary/ASCII hybrid (Elliptec proprietary)

### Important Notes

- ⚠️ **Strictly 9600 Baud**: Cannot be changed, device will not respond at other rates
- ⚠️ **No Terminators on Commands**: Send raw command bytes without CR or LF
- ⚠️ **Multidrop Bus**: Multiple devices share one port, use addressing
- ⚠️ **2-Second Timeout**: Inter-byte gap >2 seconds causes packet discard
- ✅ **Open Drain Bus**: Multiple devices can coexist on one serial line
- 📍 **Address Range**: Only use 0-9 and A-F (uppercase hex digits)

---

## Device 4: Newport ESP300 Motion Controller (PLACEHOLDER)

### Hardware Identification

**Port**: `/dev/ttyUSB1`
**Status**: ❌ **NOT RESPONDING** (likely powered off - verified 2025-11-19)

**USB Hardware ID (HWID)**:
```
Vendor:        FTDI (VID: 0403)
Model:         FT4232H Quad HS USB-UART/FIFO IC (PID: 6011)
Serial Number: FT1RALWL
USB Device:    FTDI_USB__-__Serial_Cable_FT1RALWL
Chip:          FT4232H (Quad high-speed USB-UART)
```

**HWID Match String** (for udev rules):
```bash
ID_VENDOR_ID="0403"
ID_MODEL_ID="6011"
ID_SERIAL_SHORT="FT1RALWL"
# Or combined:
ID_SERIAL="FTDI_USB__-__Serial_Cable_FT1RALWL"
```

### Serial Communication Parameters

| Parameter | Value | Notes |
|-----------|-------|-------|
| **Baud Rate** | **19200** | **FIXED** - cannot be changed by user |
| **Data Bits** | 8 | Fixed |
| **Stop Bits** | 1 | Fixed |
| **Parity** | None | Fixed - no parity |
| **Flow Control** | **Hardware (RTS/CTS)** | **REQUIRED** per manual |
| **Terminator (TX)** | CR (`\r`, 0x0D) | Carriage return |
| **Terminator (RX)** | CR (`\r`, 0x0D) | Carriage return in responses |

### Protocol Syntax

**Command Format**: ASCII commands (Newport ESP300 protocol)
```
ID?<CR>      Query device identification
1TP?<CR>     Query axis 1 position
VE?<CR>      Query firmware version
```

**Expected Response Format**: ASCII text terminated with CR
```
Example: ID? → "ESP300 Version 3.04 25AUG10\r"
```

### Hardware Flow Control Requirements

⚠️ **CRITICAL**: ESP300 **requires** hardware flow control (RTS/CTS handshake)

**From ESP300 Manual**:
> "To prevent buffer overflow when data is transferred to the ESP controller input buffer, a CTS/RTS hardware handshake protocol is implemented. The host terminal can control transmission of characters from the ESP by enabling the Request To Send (RTS) signal once the controller's Clear To Send (CTS) signal is ready. Before sending any further characters, the ESP will wait for a CTS from the host."

**Handshake Behavior**:
1. ESP asserts CTS when buffer has space available
2. ESP de-asserts CTS when buffer is full
3. Host must enable RTS signal before ESP will transmit
4. Host waits for CTS before sending data

**Physical Requirements**:
- DB-9 pin 7: RTS (Request To Send)
- DB-9 pin 8: CTS (Clear To Send)
- USB adapter must support hardware flow control
- Cable must have RTS/CTS pins connected

### Critical Timing Requirements

| Timing Parameter | Value | Reason |
|------------------|-------|--------|
| **Response Time** | Unknown (not responding) | Not tested - device powered off |
| **Port Timeout** | **3000ms** (estimated) | Conservative timeout |
| **Post-Write Delay** | Unknown | Not tested |

**Discovery Tool Configuration** (untested):
```rust
serialport::new("/dev/ttyUSB1", 19200)
    .timeout(Duration::from_millis(3000))
    .flow_control(serialport::FlowControl::Hardware)  // RTS/CTS required
    .open()?;

port.write_all(b"ID?\r")?;
port.flush()?;
thread::sleep(Duration::from_millis(2000));  // Estimated delay
```

### Expected Commands (Untested)

| Command | Expected Response | Purpose | Response Time |
|---------|------------------|---------|---------------|
| `ID?\r` | `ESP300 Version ...` | Device identification | Unknown |
| `VE?\r` | `ESP300...` | Firmware version | Unknown |
| `1TP?\r` | Position value | Query axis 1 position | Unknown |

### Device Information (Expected)

**Manufacturer**: Newport Corporation (MKS Instruments)
**Model**: ESP300 Universal Motion Controller
**Interface**: RS-232 with hardware flow control
**Default Baud**: 19200 (fixed, non-configurable)
**Axes**: 3 (configurable per axis)
**Protocol Type**: Proprietary Newport ASCII protocol

### Status and Next Steps

**Current Status**: ❌ Device not responding to any commands

**Tests Performed**:
- ✅ Tested with hardware flow control (RTS/CTS) - TIMEOUT
- ✅ Tested without flow control - TIMEOUT
- ✅ Tested `ID?\r` command - TIMEOUT
- ✅ Tested `VE?\r` command - TIMEOUT
- ✅ Verified port is not locked by other processes

**Most Likely Cause**: Device is powered off

**Next Steps**:
1. **Physical verification**: Check if ESP300 power indicator is illuminated
2. **Cable verification**: Confirm RTS/CTS pins are connected (DB-9 pins 7 & 8)
3. **Power-on test**: Power on device and re-test with discovery tool
4. **Initialization sequence**: Check manual for required startup commands

### Important Notes

- ⚠️ **Fixed Baud Rate**: 19200 only, cannot be changed per manual
- ⚠️ **Hardware Flow Control Required**: Will not work without RTS/CTS
- ⚠️ **Physical Pins Required**: USB adapter must support hardware flow control
- ⚠️ **Currently Powered Off**: Device not responding, likely not powered on
- 📍 **FT4232H Chip**: High-quality FTDI quad UART, supports hardware flow control
- 📍 **DB-9 Connector**: Requires proper RS-232 cable with all pins connected

---

## Discovery Configuration Summary

### Robust HWID-Based Discovery

For production systems, use hardware IDs to create persistent device mappings:

#### udev Rules for Persistent Device Names

```bash
# /etc/udev/rules.d/99-lab-instruments.rules

# MaiTai Laser - CP2102 with serial 20230228-906
SUBSYSTEM=="tty", ATTRS{idVendor}=="10c4", ATTRS{idProduct}=="ea60", \
  ATTRS{serial}=="20230228-906", SYMLINK+="tty-maitai", GROUP="uucp", MODE="0660"

# Elliptec Bus - FT230X with serial DK0AHAJZ
SUBSYSTEM=="tty", ATTRS{idVendor}=="0403", ATTRS{idProduct}=="6015", \
  ATTRS{serial}=="DK0AHAJZ", SYMLINK+="tty-elliptec", GROUP="uucp", MODE="0660"

# ESP300 - FT4232H with serial FT1RALWL
SUBSYSTEM=="tty", ATTRS{idVendor}=="0403", ATTRS{idProduct}=="6011", \
  ATTRS{serial}=="FT1RALWL", SYMLINK+="tty-esp300", GROUP="uucp", MODE="0660"

# Newport 1830C - Native RS-232 (always /dev/ttyS0)
# No udev rule needed - use port name directly
```

After creating rules:
```bash
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Devices will appear as:
- `/dev/tty-maitai` → `/dev/ttyUSB5`
- `/dev/tty-elliptec` → `/dev/ttyUSB0`
- `/dev/tty-esp300` → `/dev/ttyUSB1`
- `/dev/ttyS0` (Newport 1830C - native port)

### Discovery Tool Implementation

**Recommended Discovery Order**:

1. **Quick verification pass** (check known HWID mappings):
   ```rust
   // Check if devices are at expected locations
   verify_device_hwid("/dev/ttyUSB5", "Silicon_Labs", "20230228-906") // MaiTai
   verify_device_hwid("/dev/ttyUSB0", "FTDI", "DK0AHAJZ")            // Elliptec
   verify_device_hwid("/dev/ttyUSB1", "FTDI", "FT1RALWL")            // ESP300
   ```

2. **Full port scan** (if quick verification fails):
   ```rust
   // Enumerate all serial ports
   for port in serialport::available_ports()? {
       // Get HWID
       let hwid = get_port_hwid(&port.port_name)?;

       // Match against known devices
       match hwid.serial {
           "20230228-906" => probe_maitai(&port.port_name),
           "DK0AHAJZ" => probe_elliptec(&port.port_name),
           "FT1RALWL" => probe_esp300(&port.port_name),
           _ => continue,
       }
   }
   ```

3. **Protocol validation** (after HWID match):
   ```rust
   // For each matched device, validate protocol
   // This confirms correct wiring and device identity
   send_identification_command();
   parse_and_validate_response();
   extract_device_serial_number();  // Fingerprint for verification
   ```

### Device Fingerprinting

Each device provides unique identifiers in responses:

| Device | Fingerprint Data | Source |
|--------|-----------------|---------|
| MaiTai | Serial: `3227/51054/40856` | `*IDN?` response |
| Newport 1830C | (No serial in protocol) | N/A |
| Elliptec Addr 2 | Serial: `005172023` | `2in` response |
| Elliptec Addr 3 | Serial: `002842021` | `3in` response |
| Elliptec Addr 8 | Serial: `006092023` | `8in` response |
| ESP300 | (Not responding) | `ID?` response (when working) |

Store these fingerprints in config file to detect:
- Device swapped to different port
- Wrong device connected to expected port
- Multiple identical device models

---

## Configuration File Template

### config.v4.toml

```toml
# Laboratory Hardware Configuration
# Generated: 2025-11-19 08:25 CST
# Last Verified: 2025-11-19 08:25 CST

# ============================================================================
# Spectra-Physics MaiTai Ti:Sapphire Laser
# ============================================================================
[instruments.maitai]
enabled = true
type = "maitai"

# Port Configuration
port = "/dev/ttyUSB5"              # Or use: "/dev/tty-maitai" with udev rule
hwid_vendor = "Silicon_Labs"
hwid_model = "CP2102"
hwid_serial = "20230228-906"
hwid_vid = "10c4"
hwid_pid = "ea60"

# Serial Parameters
baud_rate = 9600
data_bits = 8
stop_bits = 1
parity = "none"
flow_control = "xonxoff"           # Software flow control (XON/XOFF) REQUIRED
timeout_ms = 3000                  # CRITICAL: 3 second timeout for slow response

# Protocol Parameters
terminator_tx = "\r"               # CR only
terminator_rx = "\n"               # LF in responses
command_delay_ms = 2000            # CRITICAL: Wait 2+ seconds for response

# Device Fingerprint
device_serial = "3227/51054/40856"
firmware_version = "0245-2.00.34 / CD00000019 / 214-00.004.057"

# Operational Parameters
wavelength_nm = 820.0              # Current wavelength setting
shutter_closed = true              # Safety: verify shutter is closed

# ============================================================================
# Newport 1830-C Optical Power Meter
# ============================================================================
[instruments.newport_1830c]
enabled = true
type = "newport_1830c"

# Port Configuration
port = "/dev/ttyS0"                # Native RS-232 port (no HWID)
port_type = "native_rs232"

# Serial Parameters
baud_rate = 9600
data_bits = 8
stop_bits = 1
parity = "none"
flow_control = "none"              # No flow control
timeout_ms = 1000

# Protocol Parameters
terminator_tx = "\n"               # LF only (NOT SCPI standard!)
terminator_rx = "\n"               # LF in responses
command_delay_ms = 100             # Fast response

# Operational Parameters
attenuator = 0
filter = 2                         # Medium filter
polling_rate_hz = 10.0

# ============================================================================
# Thorlabs Elliptec ELL14 Rotation Mounts (Multidrop Bus)
# ============================================================================
[instruments.elliptec_bus]
enabled = true
type = "elliptec_bus"

# Port Configuration
port = "/dev/ttyUSB0"              # Or use: "/dev/tty-elliptec" with udev rule
hwid_vendor = "FTDI"
hwid_model = "FT230X"
hwid_serial = "DK0AHAJZ"
hwid_vid = "0403"
hwid_pid = "6015"

# Serial Parameters
baud_rate = 9600                   # FIXED - cannot be changed
data_bits = 8
stop_bits = 1
parity = "none"
flow_control = "none"
timeout_ms = 500

# Protocol Parameters
terminator_tx = ""                 # No terminator on commands
terminator_rx = "\r\n"             # CR+LF in responses
command_delay_ms = 100
inter_byte_timeout_ms = 2000       # CRITICAL: Packet discarded if >2 seconds

# Bus Configuration
bus_type = "multidrop"
address_range = "0-F"              # 16 possible addresses
scan_all_addresses = true

# Device 1: Address 2
[[instruments.elliptec_bus.devices]]
address = "2"
model = "ELL14"
device_serial = "005172023"
raw_response = "2IN0E1140051720231701016800023000"
manufacturing_date = "2023"

# Device 2: Address 3
[[instruments.elliptec_bus.devices]]
address = "3"
model = "ELL14"
device_serial = "002842021"
raw_response = "3IN0E1140028420211501016800023000"
manufacturing_date = "2021"

# Device 3: Address 8
[[instruments.elliptec_bus.devices]]
address = "8"
model = "ELL14"
device_serial = "006092023"
raw_response = "8IN0E1140060920231701016800023000"
manufacturing_date = "2023"

# ============================================================================
# Newport ESP300 Motion Controller (PLACEHOLDER - DEVICE POWERED OFF)
# ============================================================================
[instruments.esp300]
enabled = false                    # DISABLED - device not responding
type = "esp300"
status = "powered_off"             # Last known status: 2025-11-19

# Port Configuration
port = "/dev/ttyUSB1"              # Or use: "/dev/tty-esp300" with udev rule
hwid_vendor = "FTDI"
hwid_model = "FT4232H"
hwid_serial = "FT1RALWL"
hwid_vid = "0403"
hwid_pid = "6011"

# Serial Parameters
baud_rate = 19200                  # FIXED - cannot be changed per manual
data_bits = 8
stop_bits = 1
parity = "none"
flow_control = "hardware"          # RTS/CTS REQUIRED per manual
timeout_ms = 3000                  # Estimated (not tested)

# Protocol Parameters
terminator_tx = "\r"               # CR terminator
terminator_rx = "\r"               # CR in responses
command_delay_ms = 2000            # Estimated (not tested)

# Configuration Notes
# - Requires hardware flow control (RTS/CTS) - pins 7 & 8 on DB-9
# - Device not responding - likely powered off
# - Requires physical verification before enabling
```

---

## Troubleshooting Guide

### Common Issues and Solutions

#### Issue: Device not detected despite being powered on

**Diagnosis**:
1. Verify HWID matches expected values:
   ```bash
   udevadm info --name=/dev/ttyUSB0 | grep ID_SERIAL
   ```

2. Check permissions:
   ```bash
   ls -la /dev/ttyUSB0
   # Should show: crw-rw---- root uucp
   # Your user must be in 'uucp' group
   ```

3. Test with minimal parameters:
   ```bash
   timeout 5 bash -c "exec 3<>/dev/ttyUSB0; stty -F /dev/ttyUSB0 9600; echo -ne 'command' >&3; cat <&3"
   ```

#### Issue: MaiTai timeout / no response

**Solution**: Increase timeout to 3+ seconds and add 2 second post-write delay

```rust
.timeout(Duration::from_millis(3000))  // Not 1000!
thread::sleep(Duration::from_millis(2000));  // Not 500!
```

#### Issue: Elliptec not found at address 0

**Solution**: Scan all addresses 0-F, devices may be configured at non-zero addresses

```bash
for addr in 0 1 2 3 4 5 6 7 8 9 A B C D E F; do
    echo -ne "${addr}in" >/dev/ttyUSB0
    sleep 0.2
    cat </dev/ttyUSB0
done
```

#### Issue: ESP300 requires hardware flow control

**Solution**: Verify USB adapter supports RTS/CTS and cable has pins connected

```bash
# Enable hardware flow control in stty
stty -F /dev/ttyUSB1 19200 crtscts
```

---

## References

### Device Manuals

- **MaiTai**: Spectra-Physics MaiTai User's Manual
- **Newport 1830-C**: Newport 1830-C User's Manual
- **Elliptec ELL14**: Thorlabs Elliptec Communication Protocol Manual
- **ESP300**: Newport ESP300 User's Manual (Section 3.2.1 - RS-232C Interface)

### Related Documentation

- `HARDWARE_TEST_REPORT_2025-11-18.md` - Initial hardware validation
- `DISCOVERY_TOOL_RESULTS_2025-11-19.md` - First discovery scan (before fixes)
- `DISCOVERY_TOOL_FIX_2025-11-19.md` - MaiTai timeout fix details
- `HARDWARE_DISCOVERY_SUCCESS_2025-11-19.md` - Complete discovery success report

---

**Document Maintained By**: rust-daq development team
**Last Updated**: 2025-11-19 08:25 CST
**Version**: 1.0
**Status**: Production reference document
