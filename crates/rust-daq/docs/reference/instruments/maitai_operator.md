# MaiTai Ti:Sapphire Laser Operator Guide

## Overview

The Spectra-Physics MaiTai is a tunable Ti:Sapphire laser with a wavelength range of 690-1040 nm. This document covers software control, safety procedures, and troubleshooting.

**Hardware Location**: `maitai@100.117.5.12`
**Serial Port**: `/dev/ttyUSB5`
**Communication**: 9600 baud, 8N1, XON/XOFF flow control

## Safety Requirements

### ⚠️ CLASS 4 LASER - SERIOUS HAZARD ⚠️

**Before operating:**
1. Obtain authorization from Laser Safety Officer (LSO)
2. Complete laser safety training for Class 4 lasers
3. Wear OD6+ safety glasses rated for 690-1040 nm
4. Verify beam path is enclosed and properly terminated
5. Activate all hardware interlocks
6. Post warning signs and activate warning lights
7. Confirm no reflective surfaces in beam path

**Emergency Procedures:**
1. Know location of emergency stop button
2. Know location of fire extinguisher (CO2 for electrical fires)
3. **Shutdown order**: Close shutter FIRST, then disable emission

## Startup Procedure

### 1. Pre-Flight Checks
```bash
# SSH to laser control machine
ssh maitai@100.117.5.12

# Verify serial port exists
ls -la /dev/ttyUSB5

# Check no other process is using the port
fuser /dev/ttyUSB5
```

### 2. Enable Remote Control Mode

**CRITICAL**: The laser front panel must be in **REMOTE** mode for software control.

1. On the laser front panel, locate the LOCAL/REMOTE switch
2. Set to **REMOTE** position
3. Verify "REMOTE" indicator is lit on front panel

> **Note**: If the panel is in LOCAL mode, software commands are acknowledged but NOT executed. This is a common issue if shutter control appears non-functional.

### 3. Software Connection

```rust
use rust_daq::hardware::maitai::MaiTaiDriver;

// Create driver instance
let laser = MaiTaiDriver::new("/dev/ttyUSB5")?;

// Verify communication
let identity = laser.identify().await?;
println!("Connected to: {}", identity);
```

## Operational Commands

### Wavelength Control

```rust
use rust_daq::hardware::capabilities::WavelengthTunable;

// Set wavelength (690-1040 nm range)
laser.set_wavelength(800.0).await?;

// Query current wavelength
let wavelength = laser.wavelength().await?;
println!("Wavelength: {} nm", wavelength);
```

**Notes:**
- Wavelength tuning takes several seconds
- Large wavelength changes may require mode-lock re-acquisition
- Verify mode-lock indicator on front panel after tuning

### Shutter Control

```rust
use rust_daq::hardware::capabilities::ShutterControl;

// Close shutter (ALWAYS close first when starting)
laser.close_shutter().await?;

// Open shutter (only when beam path is safe)
laser.open_shutter().await?;

// Query shutter state
let is_open = laser.is_shutter_open().await?;
```

**Safety Rules:**
- Always close shutter before enabling/disabling emission
- Verify shutter state before opening enclosure
- Never open shutter without verified beam termination

### Emission Control

```rust
use rust_daq::hardware::capabilities::EmissionControl;

// Enable emission (shutter MUST be closed)
laser.enable_emission().await?;

// Disable emission
laser.disable_emission().await?;

// Check emission state
let is_emitting = laser.is_emission_enabled().await?;
```

**Safety Interlock:**
The driver refuses to enable emission if shutter is open or state is unknown. This is a software safety feature.

### Power Monitoring

```rust
use rust_daq::hardware::capabilities::Readable;

// Read output power (in Watts)
let power_w = laser.read().await?;
println!("Power: {:.3} W", power_w);
```

## Shutdown Procedure

1. **Close shutter**: `laser.close_shutter().await?`
2. **Verify shutter closed**: Check front panel indicator
3. **Disable emission**: `laser.disable_emission().await?`
4. **Wait for cooldown**: Allow pump to cool (follow MaiTai manual)
5. **Set LOCAL mode** (optional): Return front panel to LOCAL

## Troubleshooting

### Commands Timeout or No Response

**Symptoms**: All commands return timeout errors

**Check:**
1. Serial port exists: `ls -la /dev/ttyUSB5`
2. Correct baud rate: 9600, 8N1, XON/XOFF
3. No conflicting processes: `fuser /dev/ttyUSB5`
4. USB-serial adapter connected properly

### Shutter Commands Don't Work

**Symptoms**: Commands succeed but shutter doesn't move

**Cause**: Front panel is in LOCAL mode

**Solution**: Switch front panel to REMOTE mode

### Wavelength Readback Differs from Setpoint

**Possible causes:**
1. Wavelength still tuning (allow 2-3 seconds)
2. Requested wavelength out of mode-lock range
3. Crystal needs optimization

### Power Reading is Zero

**Check:**
1. Is emission enabled?
2. Is shutter open?
3. Is laser properly warmed up (typically 20-30 minutes)?

## Configuration Example

```toml
# config/hardware.toml

[[instruments]]
id = "maitai"
driver = "maitai"
port = "/dev/ttyUSB5"
description = "MaiTai Ti:Sapphire Laser"

[instruments.settings]
wavelength_nm = 800.0
auto_shutter = false
```

## API Reference

### Capability Traits Implemented

| Trait | Methods |
|-------|---------|
| `Readable` | `read()` - Returns power in Watts |
| `WavelengthTunable` | `set_wavelength()`, `wavelength()` |
| `ShutterControl` | `open_shutter()`, `close_shutter()`, `is_shutter_open()` |
| `EmissionControl` | `enable_emission()`, `disable_emission()`, `is_emission_enabled()` |

### Direct Methods

| Method | Description |
|--------|-------------|
| `new(port)` | Create driver instance |
| `identify()` | Query laser identity string |
| `query_power()` | Query power (internal) |

## Related Documentation

- [MaiTai Hardware Findings](../instruments/maitai_findings.md)
- [Hardware Validation Tests](../../tests/hardware_maitai_validation.rs)
- [Serial Communication Guide](../guides/serial_setup.md)
