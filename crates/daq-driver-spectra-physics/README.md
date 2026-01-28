# daq-driver-spectra-physics

Rust driver for Spectra-Physics laser instruments, with comprehensive support for the MaiTai Ti:Sapphire tunable laser.

## Hardware Supported

- **Spectra-Physics MaiTai HP/MaiTai XF** - Tunable Ti:Sapphire laser with wavelength selection, shutter control, and emission management

## Quick Start

### Hardware Setup

MaiTai communicates via serial using either USB-to-USB or RS-232:

**USB-to-USB Connection (Recommended):**
- **Baud Rate:** 115200
- **Port:** `/dev/serial/by-id/usb-Silicon_Labs_CP2102_*` (see Hardware Inventory)
- **Protocol:** ASCII commands, LF-only terminator (NOT CR+LF)

**RS-232 Connection:**
- **Baud Rate:** 9600
- **Port:** Standard RS-232 serial port
- **Protocol:** ASCII commands, LF-only terminator

**Important:** The MaiTai protocol uses **LF-only terminator**, not CR+LF. This is unusual and critical to correct operation.

### Configuration Example

```toml
[[devices]]
id = "maitai"
type = "maitai"
enabled = true

[devices.config]
port = "/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0"
baud_rate = 115200  # USB connection: 115200, RS-232: 9600
wavelength_nm = 800  # Optional: initial wavelength
```

### Usage in Rust

```rust
use daq_driver_spectra_physics::MaiTaiFactory;
use common::driver::DriverFactory;

// Register the factory
registry.register_factory(Box::new(MaiTaiFactory));

// Create via config
let config = toml::toml! {
    port = "/dev/serial/by-id/usb-Silicon_Labs_CP2102_..."
    baud_rate = 115200
    wavelength_nm = 800
};
let components = factory.build(config.into()).await?;

// Tune laser wavelength
let tunable = components.wavelength_tunable.unwrap();
tunable.set_wavelength(820.0).await?;

// Get current wavelength
let wl = tunable.get_wavelength().await?;
println!("MaiTai wavelength: {} nm", wl);

// Control shutter
let shutter = components.shutter_control.unwrap();
shutter.open_shutter().await?;
shutter.close_shutter().await?;

// Control emission
let emission = components.emission_control.unwrap();
emission.enable_emission().await?;
emission.disable_emission().await?;

// Read power output
let readable = components.readable.unwrap();
let power = readable.read_value().await?;
println!("Output power: {:.2} W", power);
```

### Usage in Rhai Scripts

```rhai
let laser = create_maitai("/dev/serial/by-id/usb-Silicon_Labs_CP2102_...");

// Set wavelength (690-1040 nm range)
laser.set_wavelength(800.0);

// Get current wavelength
let wl = laser.wavelength();
print(`Current wavelength: ${wl} nm`);

// Shutter control
laser.open_shutter();
sleep(1.0);  // Warm-up time
laser.close_shutter();

// Emission control
laser.enable_emission();
let power = laser.read_power();
print(`Output power: ${power} W`);

laser.disable_emission();
```

## Features

### Wavelength Tuning

The MaiTai supports continuous wavelength tuning across a wide range:

```rust
// Valid range: 690-1040 nm
let tunable = components.wavelength_tunable.unwrap();

// Set wavelength
tunable.set_wavelength(850.0).await?;

// Query wavelength
let current = tunable.get_wavelength().await?;
println!("Current wavelength: {} nm", current);
```

**Tuning Range:** 690-1040 nm
**Precision:** ~0.1 nm step size
**Settling Time:** ~500ms typical (depends on wavelength change magnitude)

### Shutter and Emission Control

Independent control of optical shutter and emission:

```rust
// Shutter control (safety interlock)
let shutter = components.shutter_control.unwrap();
shutter.open_shutter().await?;    // Open beam path
shutter.close_shutter().await?;   // Block beam

// Emission control (laser on/off)
let emission = components.emission_control.unwrap();
emission.enable_emission().await?;   // Turn laser on
emission.disable_emission().await?;  // Turn laser off

// Safe sequence: Check shutter before enabling emission
if !shutter.is_open().await? {
    shutter.open_shutter().await?;
}
emission.enable_emission().await?;
```

### Power Monitoring

Read real-time output power from the laser:

```rust
let readable = components.readable.unwrap();
let power_w = readable.read_value().await?;
println!("Output power: {:.2} W", power_w);

// Unit handling: Returns Watts (W)
// Convert to mW if needed: power_w * 1000
```

### Device Query Commands

Query device identity and status:

```rust
let driver = components.readable.unwrap();

// Query device identity (returns "MaiTai ...")
// This is done during initialization for validation
```

## Protocol Reference

### Command Format

MaiTai uses ASCII commands with **LF-only terminator** (0x0A, NOT CR+LF):

```
Format: {command} {parameter}\n
Example: "wav 820\n"

Response Format: {value}\n or {value}{unit}\n
Example: "820nm\n" or "3.00W\n"
```

### Common Commands

| Command | Format | Response | Description |
|---------|--------|----------|-------------|
| Set Wavelength | `wav {nm}` | None (OK assumed) | Set wavelength (690-1040nm) |
| Query Wavelength | `wav?` | `{nm}nm` | Get commanded wavelength |
| Get Actual Wavelength | `read:wav?` | `{nm}nm` | Get operating wavelength |
| Open Shutter | `shut 1` | None | Open beam path |
| Close Shutter | `shut 0` | None | Close beam path |
| Query Shutter | `shut?` | `0` or `1` | 0=closed, 1=open |
| Enable Emission | `on` | None | Turn laser on |
| Disable Emission | `off` | None | Turn laser off |
| Query Emission | `read:pow?` | `{W}W` | Get output power |
| Query Status Byte | `*stb?` | `{byte}` | Bit 0: emission status |

### Capabilities Implemented

- **WavelengthTunable:** `set_wavelength(nm)`, `get_wavelength()`
- **ShutterControl:** `open_shutter()`, `close_shutter()`, `is_open()`
- **EmissionControl:** `enable_emission()`, `disable_emission()`
- **Readable:** `read_value()` → Watts
- **Parameterized:** Wavelength, shutter, emission parameters

## Hardware Inventory

### maitai Machine (Verified Working)

| Device | Serial Port | Baud Rate | Notes |
|--------|-------------|-----------|-------|
| maitai | `/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0` | 115200 | USB-to-UART bridge, LF-only protocol |

**Important:** Always use `/dev/serial/by-id/` path. USB device numbers change on reboot.

## Configuration Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `port` | string | Required | Serial port path |
| `baud_rate` | integer | 115200 | Connection baud rate (115200 USB, 9600 RS-232) |
| `wavelength_nm` | float | None | Optional initial wavelength (690-1040) |

## Troubleshooting

### Wrong Serial Port

```
Error: "Failed to open port /dev/ttyUSB5"
```

**Solution:** Use `/dev/serial/by-id/` path instead:

```bash
ls -la /dev/serial/by-id/ | grep -i silicon
# Should show: usb-Silicon_Labs_CP2102_...
```

### Device Identity Check Fails

```
Error: "Wrong device connected"
```

**Solution:** The device at the given port didn't respond with MaiTai identifier. Check:

1. Correct serial port is selected
2. Device is powered on
3. No other process has the port open
4. Try unplugging and replugging the USB cable

### Wavelength Out of Range

```
Error: "Wavelength 1100 nm out of MaiTai tuning range (690-1040 nm)"
```

**Solution:** MaiTai range is 690-1040 nm. Choose a wavelength within this range.

### Communication Timeout

```
Error: "Command timeout"
```

**Solution:** Device is not responding. Check:

1. Device is powered on (indicator light on)
2. No other application using the serial port
3. Serial cable is securely connected
4. Try restarting the device

### Baud Rate Mismatch

```
Error: "Garbled response" or "Command not recognized"
```

**Solution:** Verify baud rate matches connection type:

```toml
# USB connection (most common)
baud_rate = 115200

# RS-232 connection
baud_rate = 9600
```

### Shutter Won't Open

```
Shutter remains closed
```

**Solution:** Check:

1. Laser is powered on
2. No mechanical obstruction
3. Check laser's front panel for error LED
4. Try power-cycling the device

### Emission Control Not Working

```
Error: "Cannot enable emission"
```

**Solution:** Ensure shutter is open before enabling emission:

```rust
// Safe sequence
if !shutter.is_open().await? {
    shutter.open_shutter().await?;
}
emission.enable_emission().await?;
```

## Protocol Quirks and Notes

1. **LF-Only Terminator:** Unlike SCPI, MaiTai uses LF (0x0A) only, not CR+LF. This is critical for correct operation.

2. **No Flow Control:** The MaiTai doesn't use RTS/CTS or Xon/Xoff. All timing is controlled via timeouts.

3. **Response Delays:** Some commands have 100-500ms response latency. Timeouts are set accordingly.

4. **Query vs. Set:** Query commands (ending with `?`) return values; set commands return nothing (assumed OK).

5. **Power Readings:** Power is returned with "W" suffix (e.g., "3.00W\n"). The driver parses this automatically.

## Dependencies

- `tokio` - Async runtime
- `tokio-serial` - Async serial I/O
- `anyhow` - Error handling
- `serde` - TOML configuration

## See Also

- [CLAUDE.md - MaiTai Hardware Specifics](../../CLAUDE.md#spectra-physics-setup) - Serial configuration and protocol details
- [CLAUDE.md - Serial Driver Conventions](../../CLAUDE.md#serial-driver-conventions) - General serial driver patterns
- [Hardware Drivers Guide](../../docs/guides/hardware-drivers.md) - General driver development
