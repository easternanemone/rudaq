# Hardware Inventory - Laboratory Instruments

**System:** maitai@100.117.5.12 (laboratory hardware system)
**Last Verified:** 2026-01-06
**Detection Tool:** `cargo run --bin quick_test --features instrument_serial`

---

## Device Discovery Mechanisms

This system uses two different discovery mechanisms depending on device type:

### Serial Devices (ELL14, MaiTai, ESP300, Newport 1830-C)

Serial devices use OS-level paths (`/dev/ttyUSBN`) that can change between reboots
based on USB enumeration order. **Use stable `/dev/serial/by-id/` paths** for
reliability:

```rust
use daq_hardware::port_resolver::resolve_port;
use daq_hardware::drivers::ell14::Ell14Bus;

// Stable path (recommended) - survives reboots
let bus = Ell14Bus::open("/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0").await?;

// Direct path (fragile) - may change after reboot
let bus = Ell14Bus::open("/dev/ttyUSB1").await?;
```

### PVCAM Cameras (Prime BSI)

PVCAM cameras are discovered via the SDK's internal enumeration. The camera name
(e.g., `"PrimeBSI"`) is stored in camera firmware and is **inherently stable** -
no special path handling required:

```rust
use driver_pvcam::PvcamDriver;

// Camera name from firmware - always stable regardless of USB port
let camera = PvcamDriver::new_async("PrimeBSI".to_string()).await?;
```

---

## Active Instruments (4/5 Device Types)

### 1. Photometrics Prime BSI sCMOS Camera ✅

**Interface:** USB 3.0 (via PVCAM SDK)
**Camera Name:** `PrimeBSI` (firmware-reported, inherently stable)
**Status:** OPERATIONAL
**Sensor:** GS2020 (2048×2048 pixels, 16-bit)

**PVCAM Configuration:**
- SDK Version: 3.10.2.5
- Camera opened via: `pl_cam_open("PrimeBSI", &hcam, 0)`
- Environment: `PVCAM_VERSION=7.1.1.118`, `PVCAM_SDK_DIR=/opt/pvcam/sdk`

**Connection:**
```rust
use driver_pvcam::PvcamDriver;
let camera = PvcamDriver::new_async("PrimeBSI".to_string()).await?;
```

**Discovery Note:** Unlike serial devices, PVCAM cameras don't need stable path
handling. The camera name is read from firmware during SDK enumeration and remains
constant regardless of which USB port the camera is connected to.

---

### 2. Newport 1830-C Optical Power Meter ✅

**Port:** `/dev/ttyS0` (Native RS-232)
**Status:** OPERATIONAL
**Current Reading:** +.11E-9 W (11 nanowatts)

**Serial Configuration:**
- Baud rate: 9600
- Flow control: None
- Terminator: LF (`\n`)
- Command format: Simple ASCII (non-SCPI)
- Example command: `D?\n` (query power reading)

**Response Format:** Scientific notation (e.g., `+.11E-9\n`)

**Notes:**
- Simplest protocol of all devices
- Fast response time (~500ms)
- Uses native motherboard RS-232 port (not USB adapter)

---

### 3. Spectra-Physics MaiTai Ti:Sapphire Laser ✅

**Port:** `/dev/ttyUSB5` (USB-Serial via FTDI 4-port cable, interface 3)
**Stable Path:** `/dev/serial/by-id/usb-FTDI_USB__-__Serial_Cable_FT1RALWL-if03-port0`
**Status:** OPERATIONAL
**Wavelength:** 820 nm (configured default)

**Hardware Identification:**
```
Spectra Physics,MaiTai,3227/51054/40856,0245-2.00.34 / CD00000019 / 214-00.004.057
```

**Parsed Fields:**
- Manufacturer: Spectra Physics
- Model: MaiTai
- Serial Number: `3227/51054/40856`
- Firmware Version: `0245-2.00.34`
- Control Firmware: `CD00000019`
- Build Version: `214-00.004.057`

**Serial Configuration:**
- Baud rate: 9600
- Flow control: Software (XON/XOFF)
- Terminator: CR (`\r`)
- Command format: SCPI
- Identity query: `*IDN?\r`

**Timing Requirements:**
- Port timeout: 3000ms (laser takes 2+ seconds to respond)
- Post-write delay: 2000ms
- **CRITICAL:** Must call `flush()` after `write_all()`

**Notes:**
- Slowest responding device in lab
- Requires extended timeouts for reliable detection
- Uses Software flow control (XON/XOFF) to prevent buffer overflows

---

### 4. Thorlabs Elliptec ELL14 Rotation Mounts (3 Units) ✅

**Port:** `/dev/ttyUSB1` (USB-Serial via FTDI FT230X Basic UART)
**Stable Path:** `/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0`
**Status:** OPERATIONAL (All 3 devices responding)
**Bus Type:** Multidrop addressable bus (0-F)

#### Device 1 - Address 2
**Hardware ID:** `2IN0E1140051720231701016800023000`
- Model: `0E11` (ELL14)
- Serial Number: `005172023` (extracted from bytes 10-18)
- Full Response: `2IN0E1140051720231701016800023000\r\n`

#### Device 2 - Address 3
**Hardware ID:** `3IN0E1140028420211501016800023000`
- Model: `0E11` (ELL14)
- Serial Number: `002842021`
- Full Response: `3IN0E1140028420211501016800023000\r\n`

#### Device 3 - Address 8
**Hardware ID:** `8IN0E1140060920231701016800023000`
- Model: `0E11` (ELL14)
- Serial Number: `006092023`
- Full Response: `8IN0E1140060920231701016800023000\r\n`

**Serial Configuration:**
- Baud rate: 9600 (fixed, cannot be changed)
- Flow control: None
- Terminator: CR+LF (`\r\n`)
- Bus addresses: 0-F (hexadecimal)
- Active addresses: 2, 3, 8

**Protocol Details:**
- Message format: 3-byte header (address + 2-char command) + optional data
- Commands: Lowercase for host→device (e.g., `2in` = get info from address 2)
- Responses: Uppercase from device→host (e.g., `2IN...` = info response)
- Timeout: 2 seconds between bytes (packet discarded if gap > 2s)
- Bus topology: Open-drain with 10kΩ pull-up to 3.3V

**Discovery Requirements:**
- Must scan addresses 0-F individually
- Address 0 is NOT guaranteed to have a device
- Each device responds only to its assigned address

**Notes:**
- Multidrop bus allows up to 16 devices on one port
- Hardware addresses are factory-set or user-configured
- Discovery tool must scan all 16 addresses to find devices

---

## Inactive Instruments

### 5. Newport ESP300 Motion Controller ❌

**Port:** `/dev/ttyUSB2` (USB-Serial via FTDI 4-port cable, interface 0)
**Stable Path:** `/dev/serial/by-id/usb-FTDI_USB__-__Serial_Cable_FT1RALWL-if00-port0`
**Status:** NOT RESPONDING (likely powered off)
**Last Verified:** 2025-11-20

**Serial Configuration:**
- Baud rate: 19200 (fixed, cannot be changed)
- Flow control: Hardware (RTS/CTS) **REQUIRED**
- Terminator: CR (`\r`)
- Command format: ESP300 protocol
- Identity query: `ID?\r` or `VE?\r`

**Notes:**
- Device has not responded in multiple test sessions
- Most likely cause: Powered off
- Alternative causes: Cable issue, USB adapter doesn't support RTS/CTS
- FTDI chip should support hardware flow control
- Requires physical verification of power status

---

## Device Mapping

| Device | Port | Stable Path (by-id) | Status |
|--------|------|---------------------|--------|
| Prime BSI Camera | USB 3.0 | `PrimeBSI` (PVCAM name) | ✅ Working |
| Newport 1830-C | `/dev/ttyS0` | Native RS-232 (no by-id) | ✅ Working |
| ELL14 Bus (3 units) | `/dev/ttyUSB1` | `usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0` | ✅ Working |
| MaiTai Laser | `/dev/ttyUSB5` | `usb-FTDI_USB__-__Serial_Cable_FT1RALWL-if03-port0` | ✅ Working |
| ESP300 | `/dev/ttyUSB2` | `usb-FTDI_USB__-__Serial_Cable_FT1RALWL-if00-port0` | ❌ Powered off |

### Using Stable Paths (Recommended)

The stable paths in `/dev/serial/by-id/` do not change between reboots, unlike `/dev/ttyUSBN` which depends on enumeration order.

```rust
use daq_hardware::drivers::ell14::Ell14Bus;

// Using stable by-id path (recommended)
let bus = Ell14Bus::open("/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0").await?;

// Or just the by-id name (without full path)
let bus = Ell14Bus::open("usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0").await?;
```

---

## Hardware Discovery Tools

### Quick Test (Recommended)
```bash
cargo run --bin quick_test --features instrument_serial
```
- **Duration:** ~20 seconds
- **Coverage:** 4 known device types
- **Detection Rate:** 75% (3/4 devices, ESP300 powered off)

### Full Scan (Comprehensive)
```bash
cargo run --bin discovery --features instrument_serial
```
- **Duration:** ~12-14 minutes
- **Coverage:** All 38 serial ports on system
- **Use Case:** First-time setup or when devices move to different ports

---

## Configuration File Template

```toml
# Hardware configuration for maitai@100.117.5.12

[instruments.newport_1830c]
type = "newport_1830c"
port = "/dev/ttyS0"
baud_rate = 9600
flow_control = "none"

[instruments.maitai]
type = "maitai"
port = "/dev/ttyUSB5"
baud_rate = 9600
flow_control = "xonxoff"
wavelength = 820.0
# Serial: 3227/51054/40856
# Firmware: 0245-2.00.34 / CD00000019 / 214-00.004.057

[instruments.ell14_bus]
type = "elliptec_bus"
port = "/dev/ttyUSB0"
baud_rate = 9600
flow_control = "none"

# Three ELL14 devices on multidrop bus
[[instruments.ell14_bus.devices]]
address = 2
model = "ELL14"
serial = "005172023"
# Full ID: 2IN0E1140051720231701016800023000

[[instruments.ell14_bus.devices]]
address = 3
model = "ELL14"
serial = "002842021"
# Full ID: 3IN0E1140028420211501016800023000

[[instruments.ell14_bus.devices]]
address = 8
model = "ELL14"
serial = "006092023"
# Full ID: 8IN0E1140060920231701016800023000

# ESP300 - NOT RESPONDING (likely powered off)
# [instruments.esp300]
# type = "esp300"
# port = "/dev/ttyUSB1"
# baud_rate = 19200
# flow_control = "hardware"  # RTS/CTS required
```

---

## References

- **Discovery Success Report:** `docs/HARDWARE_DISCOVERY_SUCCESS_2025-11-19.md`
- **Discovery Tool Fix:** `docs/DISCOVERY_TOOL_FIX_2025-11-19.md`
- **Hardware Drivers:** `docs/HARDWARE_DRIVERS_EXAMPLE.md`
- **Quick Test Source:** `tools/discovery/quick_test.rs`
- **Full Scan Source:** `tools/discovery/main.rs`

---

**Document Version:** 1.0
**Author:** Claude Code
**Date:** 2025-11-20
