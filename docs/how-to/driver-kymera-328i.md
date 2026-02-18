# Kymera 328i Spectrograph Driver Development Guide

## Overview

Andor Kymera 328i imaging spectrograph with motorized grating turret, slits, and wavelength drive.

**Reference:** `docs/reference/markdown/kymera-328i.md`

---

## Specifications

| Parameter | Value |
|-----------|-------|
| Focal Length | 328 mm |
| Focal Ratio | f/3.8 |
| Grating Size | 43 mm |
| Interface | USB 2.0 (primary), I2C (alternative) |
| Slit Range | 10 - 2500 µm |
| Slit Step Size | 2.5 µm |
| Operating Temperature | 0°C to 30°C |
| Power Consumption | 21W max |
| Power Input | 100-240V AC, 47-63 Hz |

---

## Communication

### USB 2.0 (Primary)

| Parameter | Value |
|-----------|-------|
| Cable | Type A to Type B (3m supplied) |
| Driver | Shamrock USB Drivers (Solis installation) |
| Protocol | Proprietary via Andor SDK |

### I2C (Alternative)

| Parameter | Value |
|-----------|-------|
| Purpose | Direct camera integration |
| Note | Mutually exclusive with USB |

**WARNING:** Do not connect both USB and I2C simultaneously.

---

## Configuration Variants

| Model | Side Input | Direct Input | Direct Output | Side Output | Motorized Port Selection |
|-------|------------|--------------|---------------|-------------|-------------------------|
| KYMERA-328i-A | Manual slit | - | Camera | - | No |
| KYMERA-328i-B1 | Manual slit | - | Camera | Manual slit | Yes |
| KYMERA-328i-B2 | Manual slit | - | Camera | Camera | Yes |
| KYMERA-328i-C | Manual slit | Manual slit | Camera | - | Yes |
| KYMERA-328i-D1 | Manual slit | Manual slit | Camera | Manual slit | Yes |
| KYMERA-328i-D2 | Manual slit | Manual slit | Camera | Camera | Yes |

**Silver-coated variants:** Add `-SIL` suffix (e.g., `KYMERA-328i-B1-SIL`)

---

## Complete Feature/Command List

### Grating Turret Control

| Command | Parameters | Returns | Description |
|---------|------------|---------|-------------|
| `SelectGrating` | `position: 1-3` | Status | Select grating position on turret |
| `GetGrating` | - | `int (1-3)` | Query current grating position |
| `GetGratingInfo` | `position: 1-3` | `GratingInfo` | Get grating specifications |
| `ResetTurret` | - | Status | Reset turret to home position |
| `GetNumberOfGratings` | - | `int` | Number of gratings installed |

**GratingInfo Structure:**
```rust
struct GratingInfo {
    lines_per_mm: u32,      // Groove density
    blaze_wavelength_nm: f64, // Blaze wavelength
    min_wavelength_nm: f64,   // Minimum usable wavelength
    max_wavelength_nm: f64,   // Maximum usable wavelength
}
```

### xPressID RFID System

| Command | Parameters | Returns | Description |
|---------|------------|---------|-------------|
| `ReadRFIDCalibration` | `turret_id: 1-3` | `Calibration` | Read calibration from RFID chip |
| `WriteRFIDCalibration` | `turret_id, data` | Status | Write calibration to RFID chip |
| `GetRFIDStatus` | - | `RFIDStatus` | Check RFID chip presence/health |

**Notes:**
- Up to 3 turret configurations stored in memory
- 4th turret overwrites oldest stored configuration
- No re-calibration needed for previously used turrets

### Wavelength Control

| Command | Parameters | Returns | Description |
|---------|------------|---------|-------------|
| `SetWavelength` | `wavelength_nm: f64` | Status | Set center wavelength |
| `GetWavelength` | - | `f64` | Query current wavelength |
| `GetWavelengthRange` | - | `(min, max)` | Get valid range for current grating |
| `ResetWavelength` | - | Status | Reset to reference position |
| `GotoZeroOrder` | - | Status | Move to zero-order position |
| `GetDetectorOffset` | - | `f64` | Query detector offset |
| `SetDetectorOffset` | `offset: f64` | Status | Set detector offset |
| `GetGratingOffset` | `grating: 1-3` | `f64` | Query grating-specific offset |
| `SetGratingOffset` | `grating, offset` | Status | Set grating-specific offset |

**Wavelength Drive Specifications:**
- Software-controlled stepper motor
- Wavelength range depends on selected grating
- Dispersion depends on grating groove density

### Slit Control

| Command | Parameters | Returns | Description |
|---------|------------|---------|-------------|
| `SetSlitWidth` | `width_um: f64` | Status | Set slit width (10-2500 µm) |
| `GetSlitWidth` | - | `f64` | Query slit width |
| `ResetSlit` | - | Status | Reset to default (10 µm) |
| `GetSlitRange` | - | `(min, max)` | Get valid slit range |

**Slit Specifications:**
| Parameter | Value |
|-----------|-------|
| Minimum Width | 10 µm |
| Maximum Width | 2500 µm |
| Step Size | 2.5 µm |
| Default | 10 µm |

**Resolution vs Throughput Trade-off:**
| Slit Width | Resolution | Throughput |
|------------|------------|------------|
| 10 µm | Maximum | Minimum |
| 50 µm | High | Low |
| 100 µm | Good | Medium |
| 500 µm | Moderate | High |
| 2500 µm | Low | Maximum |

### Filter Wheel (Optional)

| Command | Parameters | Returns | Description |
|---------|------------|---------|-------------|
| `InitializeFilterWheel` | - | Status | Initialize filter wheel |
| `SetFilterPosition` | `position: int` | Status | Select filter position |
| `GetFilterPosition` | - | `int` | Query current position |
| `GetNumberOfFilters` | - | `int` | Number of filter positions |

### Shutter Control (Optional)

| Command | Parameters | Returns | Description |
|---------|------------|---------|-------------|
| `OpenShutter` | - | Status | Open internal shutter |
| `CloseShutter` | - | Status | Close internal shutter |
| `GetShutterState` | - | `bool` | Query shutter state |

**External Shutter:** TTL control via BNC connector

### Flipper Mirror (Optional)

| Command | Parameters | Returns | Description |
|---------|------------|---------|-------------|
| `SetFlipperPosition` | `position: 0-1` | Status | Select input/output path |
| `GetFlipperPosition` | - | `int` | Query flipper position |
| `ResetFlipper` | - | Status | Reset flipper to default |

### Active Focus (Optional)

| Command | Parameters | Returns | Description |
|---------|------------|---------|-------------|
| `EnableAutoFocus` | - | Status | Enable automatic focus |
| `DisableAutoFocus` | - | Status | Disable automatic focus |
| `SetFocusPosition` | `position: f64` | Status | Manual focus position |
| `GetFocusPosition` | - | `f64` | Query focus position |
| `RunAutoFocus` | - | Status | Execute auto-focus routine |

### Port Selection (B/C/D Models)

| Command | Parameters | Returns | Description |
|---------|------------|---------|-------------|
| `SelectInputPort` | `port: InputPort` | Status | Select input port |
| `GetInputPort` | - | `InputPort` | Query selected input |
| `SelectOutputPort` | `port: OutputPort` | Status | Select output port |
| `GetOutputPort` | - | `OutputPort` | Query selected output |

**InputPort Enum:**
- `Side` - Side entrance slit
- `Direct` - Direct entrance slit

**OutputPort Enum:**
- `Direct` - Primary output (camera)
- `Side` - Side output (B1/D1: slit, B2/D2: camera)

### Device Information

| Command | Parameters | Returns | Description |
|---------|------------|---------|-------------|
| `GetSerialNumber` | - | `String` | Device serial number |
| `GetFirmwareVersion` | - | `String` | Firmware version |
| `GetModelName` | - | `String` | Model identifier |
| `GetCalibrationDate` | - | `DateTime` | Last calibration date |

---

## Calibration Procedure

### Equipment Required

| Item | Purpose |
|------|---------|
| Mercury pen-ray lamp | Primary calibration source |
| Neon pen-ray lamp | Alternative/verification source |
| Argon pen-ray lamp | Alternative/verification source |

### Reference Spectral Lines

**Mercury (Hg):**
| Wavelength (nm) | Relative Intensity |
|-----------------|-------------------|
| 253.65 | Strong |
| 296.73 | Medium |
| 302.15 | Medium |
| 312.57 | Medium |
| 365.02 | Very Strong |
| 404.66 | Strong |
| 435.83 | Very Strong |
| 546.07 | Very Strong |
| 576.96 | Strong |
| 579.07 | Strong |

**Neon (Ne):**
| Wavelength (nm) | Relative Intensity |
|-----------------|-------------------|
| 585.25 | Strong |
| 640.22 | Strong |
| 703.24 | Very Strong |
| 724.52 | Strong |
| 837.76 | Strong |

### Calibration Steps

1. **Setup:**
   - Set entrance slit to 10 µm (minimum)
   - Position spectral lamp at input

2. **For each grating:**
   ```
   a. Select grating position
   b. Set wavelength to known spectral line (e.g., Hg 546.07 nm)
   c. Observe spectrum on CCD
   d. Adjust Detector Offset until line centers on CCD
   e. Confirm alignment after wavelength reset
   f. Record calibration values
   ```

3. **Store calibration:**
   - Calibration stored to xPressID RFID chip
   - Per-grating offsets saved

4. **Verify:**
   - Test with multiple known lines
   - Check dispersion accuracy

---

## Grating Selection Guide

| Grating (lines/mm) | Typical Range (nm) | Resolution | Throughput | Application |
|-------------------|--------------------|------------|------------|-------------|
| 150 | 200-1100 | Low | High | Broadband survey |
| 300 | 200-1100 | Medium-Low | Medium-High | General spectroscopy |
| 600 | 200-800 | Medium | Medium | Balanced performance |
| 1200 | 200-600 | High | Medium-Low | High resolution |
| 1800 | 200-500 | Very High | Low | Detailed analysis |
| 2400 | 200-400 | Maximum | Minimum | Ultra-high resolution |

---

## Error Handling

| Error | Cause | Recovery |
|-------|-------|----------|
| Turret stuck | Mechanical obstruction | Power cycle, check grating |
| Wavelength out of range | Invalid for current grating | Check grating limits |
| Slit out of range | Width < 10 or > 2500 µm | Clamp to valid range |
| RFID read failure | Dirty contacts | Clean, retry |
| USB disconnected | Cable unplugged | Reconnect, re-init |
| Calibration invalid | RFID corruption | Re-calibrate |

---

## Rust Implementation

```rust
pub struct Kymera328i {
    handle: ShamrockHandle,
    grating_calibration: HashMap<u8, GratingCalibration>,
    current_grating: u8,
}

impl Kymera328i {
    pub async fn open(device_index: u32) -> Result<Self> {
        let handle = shamrock_open(device_index)?;
        let calibration = load_rfid_calibration(&handle)?;
        let current = get_grating(&handle)?;
        
        Ok(Self {
            handle,
            grating_calibration: calibration,
            current_grating: current,
        })
    }
    
    pub async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()> {
        // Validate against current grating range
        let cal = self.grating_calibration.get(&self.current_grating)
            .ok_or(anyhow!("No calibration for grating {}", self.current_grating))?;
        
        if wavelength_nm < cal.min_nm || wavelength_nm > cal.max_nm {
            return Err(anyhow!(
                "Wavelength {} nm out of range [{}, {}] for grating {}",
                wavelength_nm, cal.min_nm, cal.max_nm, self.current_grating
            ));
        }
        
        shamrock_set_wavelength(&self.handle, wavelength_nm)
    }
    
    pub async fn set_slit_width(&self, width_um: f64) -> Result<()> {
        // Validate slit range
        if width_um < 10.0 || width_um > 2500.0 {
            return Err(anyhow!("Slit width {} µm out of range [10, 2500]", width_um));
        }
        
        // Round to 2.5 µm steps
        let rounded = (width_um / 2.5).round() * 2.5;
        shamrock_set_slit_width(&self.handle, rounded)
    }
}

impl WavelengthTunable for Kymera328i {
    async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()> {
        self.set_wavelength(wavelength_nm).await
    }
    
    async fn get_wavelength(&self) -> Result<f64> {
        shamrock_get_wavelength(&self.handle)
    }
}
```

---

## Configuration Example

```toml
[[devices]]
id = "spectrograph"
type = "kymera_328i"
enabled = true

[devices.config]
device_index = 0

[devices.config.gratings.1]
lines_per_mm = 150
blaze_nm = 500
min_nm = 200
max_nm = 1100

[devices.config.gratings.2]
lines_per_mm = 600
blaze_nm = 500
min_nm = 200
max_nm = 800

[devices.config.gratings.3]
lines_per_mm = 1200
blaze_nm = 300
min_nm = 200
max_nm = 600

[devices.config.calibration]
detector_offset = 0.0
grating_1_offset = 0.0
grating_2_offset = 0.0
grating_3_offset = 0.0

[devices.config.defaults]
slit_width_um = 100
grating = 2
```
