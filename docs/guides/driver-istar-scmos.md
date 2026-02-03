# iStar-sCMOS Intensified Camera Driver Guide

This guide provides comprehensive information for implementing a rust-daq driver for the Andor iStar-sCMOS intensified camera, which combines a sCMOS sensor with a gated image intensifier.

## Hardware Overview

The iStar-sCMOS is an intensified scientific camera combining:
- **sCMOS Sensor**: 2560 × 2160 pixels, 6.5 µm pixel size
- **Image Intensifier**: Gen 2 or Gen 3 with selectable photocathodes
- **Digital Delay Generator (DDG)**: 10 ps timing resolution
- **Microchannel Plate (MCP)**: 12-bit gain control (0-4095)

### Key Specifications

| Parameter | Value |
|-----------|-------|
| Sensor Resolution | 2560 × 2160 pixels |
| Pixel Size | 6.5 µm |
| Intensifier Diameters | Ø18 mm (standard), Ø25 mm (option) |
| MCP Gain Range | 0 - 4095 (12-bit) |
| DDG Timing Resolution | 10 ps |
| DDG Delay/Width Range | 0 ns - 10 s |
| Min Cooling Temperature | 0°C |
| Dark Current @ 0°C | 0.18 e⁻/pixel/s |

## SDK3 Interface

The iStar-sCMOS uses Andor SDK3, inheriting all base SDK3 features plus intensifier-specific features.

### Feature Types

| Type | C Function | Description |
|------|------------|-------------|
| Integer | `AT_GetInt`, `AT_SetInt` | Whole numbers (gain, dimensions) |
| Float | `AT_GetFloat`, `AT_SetFloat` | Decimal values (exposure, timing) |
| Boolean | `AT_GetBool`, `AT_SetBool` | True/false flags |
| Enum | `AT_GetEnumIndex`, `AT_SetEnumIndex` | Named options |
| Command | `AT_Command` | Trigger actions |
| String | `AT_GetString`, `AT_SetString` | Text values (wide char) |

## MCP (Microchannel Plate) Control

### Gain Control

The MCP provides signal amplification through electron multiplication.

| Parameter | Value |
|-----------|-------|
| Gain Range | 0 - 4095 (12-bit digital) |
| Voltage Range | 500V - 1kV across plate |
| Single Stage Gain | Up to 10⁴ |
| Channel Diameter | ~10 µm |
| Plate Thickness | ~1 mm |

```rust
// SDK3 feature for MCP gain
let gain: i64 = 2048; // Mid-range gain
AT_SetInt(handle, "MCPGain", gain)?;

// Read current gain
let current_gain = AT_GetInt(handle, "MCPGain")?;
```

### MCP with Intelligate

Intelligate provides enhanced gating with MCP pre-switching:

| Mode | Max Repetition Rate | Photoelectron Rejection |
|------|---------------------|------------------------|
| Intelligate OFF | 500 kHz | Standard |
| Intelligate ON | 5 kHz | >10⁷ : 1 (below 200 nm) |

**Intelligate Switching Sequence:**
1. MCP receives fast rising edge from gating electronics
2. 100 ns delay for voltage settling
3. Photocathode opens
4. MCP remains at user-set gain for gate width duration
5. MCP decays rapidly after gate pulse ends

```rust
// Enable Intelligate mode
AT_SetBool(handle, "Intelligate", true)?;
```

## Digital Delay Generator (DDG)

The DDG provides precise timing control for the intensifier gate.

### Timing Parameters

| Parameter | Range | Resolution |
|-----------|-------|------------|
| Gate Delay | 0 ns - 10 s | 10 ps |
| Gate Width | 0 ns - 10 s | 10 ps |
| Output A/B/C Delay | 0 ns - 10 s | 10 ps |
| IOC Period | 20 ns minimum | 20 ns steps |

```rust
// Set gate timing (values in seconds)
AT_SetFloat(handle, "GateDelay", 100e-9)?;     // 100 ns delay
AT_SetFloat(handle, "GateWidth", 50e-9)?;      // 50 ns width

// Configure DDG outputs for external sync
AT_SetFloat(handle, "OutputADelay", 0.0)?;     // Output A at trigger
AT_SetFloat(handle, "OutputBDelay", 1e-6)?;    // Output B at 1 µs
AT_SetFloat(handle, "OutputCDelay", 2e-6)?;    // Output C at 2 µs
```

### DDG Output Specifications

| Parameter | Value |
|-----------|-------|
| Voltage Level | +5V CMOS |
| Source Impedance | 50 Ω |
| Load (non-terminating) | 5V |
| Load (50 Ω terminated) | 2.5V |
| Polarity | Configurable |

### Insertion Delay Settings

| Mode | Delay | Use Case |
|------|-------|----------|
| Ultra Fast | ~35 ns | Minimum latency |
| Normal | ~135 ns | Standard operation |

**Note:** sCMOS sensor takes ~300 ns to fully open after trigger.

## Gating Modes

### Gate Mode Enumeration

| Mode | Description | Behavior |
|------|-------------|----------|
| `CW_On` | Continuous Wave On | Photocathode continuously ON |
| `CW_Off` | Continuous Wave Off | Photocathode continuously OFF |
| `DDG` | Digital Delay Generator | Software-controlled pulsed gating |

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GateMode {
    CwOn,     // Photocathode continuously ON
    CwOff,    // Photocathode continuously OFF
    Ddg,      // DDG-controlled gating
}

impl GateMode {
    pub fn to_sdk3_index(&self) -> i32 {
        match self {
            GateMode::CwOn => 0,
            GateMode::CwOff => 1,
            GateMode::Ddg => 2,
        }
    }
}
```

### Direct Gate Input

Alternative to DDG for direct photocathode control:

| Parameter | Value |
|-----------|-------|
| Input Type | TTL compatible |
| Safe Voltage Range | -0.5 to +5.75 V |
| Input Impedance | 50 Ω to ground |
| Logic High (minimum) | 1.7 V |
| Logic Low (maximum) | 0.8 V |

## Trigger Modes

### Available Modes

| Mode | Role | Description |
|------|------|-------------|
| Internal | Master | Camera controls timing |
| External | Slave | External device triggers |
| External Start | Slave | External starts acquisition |
| External Exposure | Slave | External controls exposure |

### External Trigger Input

| Parameter | Value |
|-----------|-------|
| Type | TTL compatible |
| Safe Voltage Range | -0.3 to +5.0 V |
| Max Frequency | 500 kHz |
| Configurable | Polarity, termination, threshold |
| Threshold Range | +0.25 to +3.3 V |

### Timing Coordination (Critical)

When using external trigger:
- **Intensifier opens:** 35-135 ns after trigger (depending on insertion delay)
- **sCMOS fully open:** ~300 ns after trigger

**Solution:** Add 100-300 ns to DDG gate delay to ensure intensifier opens AFTER sCMOS sensor.

```rust
// Compensate for sensor/intensifier timing mismatch
const SCMOS_LATENCY_NS: f64 = 300e-9;
const INTENSIFIER_DELAY_NS: f64 = 35e-9; // Ultra Fast mode

let gate_delay = SCMOS_LATENCY_NS - INTENSIFIER_DELAY_NS + user_delay;
AT_SetFloat(handle, "GateDelay", gate_delay)?;
```

## Integrate-On-Chip (IOC)

IOC allows multiple gate pulses within a single sensor exposure.

### Availability
- **Required:** DDG gate mode
- **Purpose:** Accumulate multiple gated events on sensor

### IOC Options (Internal Trigger)

| Option | Description |
|--------|-------------|
| Fit to Exposure | Max pulses calculated from exposure time |
| Number of Pulses | User specifies exact pulse count |

### IOC Options (External Trigger)

| Option | Description |
|--------|-------------|
| Number of Pulses = 1 | One gate pulse per trigger |

### IOC Timing

| Parameter | Value |
|-----------|-------|
| Minimum Delay | 20 ns (when DDG generates pulses internally) |
| Period Resolution | 20 ns intervals |

```rust
// Enable IOC with 10 pulses at 1 MHz
AT_SetBool(handle, "IntegrateOnChip", true)?;
AT_SetInt(handle, "IOCPulseCount", 10)?;
AT_SetFloat(handle, "IOCPeriod", 1e-6)?;  // 1 µs period
```

## DDG Step Mode (Kinetic Series)

Automatically adjusts delay/width across frames for time-resolved measurements.

### Step Mode Enumeration

| Mode | Formula | Coefficients |
|------|---------|--------------|
| Constant | Step = a × (Frame - 1) | 1 |
| Exponential | Step = a × exp(b × (Frame - 1)) | 2 |
| Logarithmic | Step = a × ln(b × (Frame - 1) + 1) | 2 |

### Constant Step Example

| Frame | Delay (Coefficient = 500 ps) |
|-------|------------------------------|
| 1 | 0 ps |
| 2 | 500 ps |
| 3 | 1000 ps |
| 4 | 1500 ps |
| 5 | 2000 ps |

### Exponential Step Example

| Frame | Delay (a=100 ps, b=0.694) |
|-------|---------------------------|
| 1 | 0 ps |
| 2 | 200 ps |
| 3 | 400 ps |
| 4 | 800 ps |
| 5 | 1600 ps |

```rust
#[derive(Debug, Clone)]
pub enum DdgStepMode {
    Constant { coefficient: f64 },
    Exponential { amplitude: f64, exponent: f64 },
    Logarithmic { amplitude: f64, coefficient: f64 },
}

impl DdgStepMode {
    pub fn calculate_step(&self, frame: u32) -> f64 {
        let n = (frame - 1) as f64;
        match self {
            DdgStepMode::Constant { coefficient } => coefficient * n,
            DdgStepMode::Exponential { amplitude, exponent } => {
                amplitude * (exponent * n).exp() - amplitude
            }
            DdgStepMode::Logarithmic { amplitude, coefficient } => {
                amplitude * (coefficient * n + 1.0).ln()
            }
        }
    }
}
```

## Phosphor Types

The phosphor screen converts electrons to visible light for the sCMOS sensor.

| Type | Color | Peak (nm) | Decay to 10% | Efficiency | Use Case |
|------|-------|-----------|--------------|------------|----------|
| P43 | Yellow/Green | 545 | 2 ms | 100 (ref) | Standard, best linearity |
| P46 | Yellow/Green | 530 | 200 ns | 10 | Fast scan (>100 Hz) |

### Selection Guide

- **P43 (Default):** Superior linearity, higher efficiency, slower decay
- **P46 (Fast):** Required for scan rates >100 Hz, lower efficiency

## Photocathode Types

### Gen 2 (Multi-alkali) Photocathodes

| Model | QE (%) | Wavelength (nm) | Resolution (µm) | Gating | Max Gain |
|-------|--------|-----------------|-----------------|--------|----------|
| 18*-03 | 18 | 180-850 | 25 | U (<2ns), F (<5ns) | >1000 |
| 18*-04 | 18 | 180-850 | 30 | U (<2ns), F (<5ns) | >500 |
| 18*-05 | 15 | 120-850 | 25 | U (<5ns), F (<10ns) | >1000 |
| 18H-13 | 13.5 | 180-920 | 25 | H (<50ns) | >850 |
| 18H-83 | 25 | 180-850 | 25 | H (<100ns) | >500 |
| 18*-E3 | 22 | 180-850 | 25 | U (<2ns), F (<5ns) | >300 |

### Gen 3 (GaAs) Photocathodes

| Model | QE (%) | Wavelength (nm) | Resolution (µm) | Max Rep (kHz) |
|-------|--------|-----------------|-----------------|---------------|
| 18*-63 | >47.5 | UV-NIR | 30 | 500 |
| 18*-73 | >25.5 | UV-NIR | 30 | 500 |
| 18*-93 | >5 | 380-1090 | 30 | - |
| 18*-A3 | >40 | UV-NIR | 30 | 500 |
| 18*-C3 | >17 | <200-910 | 40 | - |

### Gating Speed Classifications

| Class | Gate Width | QE Trade-off |
|-------|------------|--------------|
| U (Ultrafast) | <2 ns | Slightly reduced QE |
| F (Fast) | <5 ns | Moderate |
| H (High QE) | >50 ns | Highest QE |

## sCMOS Sensor Details

### Readout Modes

| Mode | Bit Depth | Gain (e⁻/ADU) | Full Well (e⁻) | File Size |
|------|-----------|---------------|----------------|-----------|
| 12-bit High Well | 12 | 7.5 | 30,000 | 8 MB |
| 12-bit Low Noise (GS) | 12 | 0.42 | 1,700 | 8 MB |
| 12-bit Low Noise (RS) | 12 | 0.28 | 1,100 | 8 MB |
| 16-bit Dual Gain (GS) | 16 | 0.45 | 30,000 | 10.5 MB |
| 16-bit Dual Gain (RS) | 16 | 0.45 | 30,000 | 10.5 MB |

### Sensor Performance

| Parameter | Value |
|-----------|-------|
| Linearity | >99.8% |
| Read Noise | Ultra-low (several orders better than CCD) |
| Dark Current @ 0°C | 0.18 e⁻/pixel/s |

## Safety Considerations

### Photocathode Damage Risks

| Risk | Cause | Prevention |
|------|-------|------------|
| Bleaching | Over-illumination | Use mechanical shutter during non-use |
| Ion Damage | Excessive photoelectrons | Monitor gain with light levels |
| Saturation Damage | Electrons beyond saturation | Reduce gain or light input |

### Electrical Safety

| Parameter | Limit |
|-----------|-------|
| Direct Gate Safe Voltage | -0.5 to +5.75 V |
| External Trigger Safe Voltage | -0.3 to +5.0 V |
| Overvoltage Category | CAT II AC/DC |

### Thermal Management

| Parameter | Value |
|-----------|-------|
| Ambient Operating Range | 0°C to 40°C |
| Fan Trigger Temperature | Heat-sink > 50°C |
| Coolant Flow Rate | 2 L/min recommended |
| Maximum Pressure | 2 bar (30 PSI) |
| Ventilation Clearance | ≥100 mm |

## Gate Monitor Output

The Gate Monitor provides real-time verification of photocathode switching:

| Signal | Meaning |
|--------|---------|
| Negative Spike | Photocathode ON |
| Positive Spike | Photocathode OFF |
| Additional Spike (Intelligate) | MCP switching ON |

**Connection:** BNC connector, AC coupled

## Complete iStar SDK3 Features

### Intensifier-Specific Features

| Feature | Type | Range/Values | Description |
|---------|------|--------------|-------------|
| MCPGain | Integer | 0-4095 | MCP voltage control |
| Intelligate | Boolean | true/false | Enhanced gating mode |
| GateMode | Enum | CwOn/CwOff/Ddg | Gating strategy |
| GateDelay | Float | 0-10 s | Gate pulse delay |
| GateWidth | Float | 0-10 s | Gate pulse width |
| InsertionDelay | Enum | UltraFast/Normal | Trigger-to-gate delay |
| IntegrateOnChip | Boolean | true/false | Multiple pulses per exposure |
| IOCPulseCount | Integer | 1-N | Number of IOC pulses |
| IOCPeriod | Float | ≥20 ns | IOC pulse period |
| DDGStepMode | Enum | Constant/Exp/Log | Kinetic stepping mode |
| DDGStepCoeffA | Float | Varies | Step coefficient A |
| DDGStepCoeffB | Float | Varies | Step coefficient B |
| OutputADelay | Float | 0-10 s | Sync output A delay |
| OutputBDelay | Float | 0-10 s | Sync output B delay |
| OutputCDelay | Float | 0-10 s | Sync output C delay |
| OutputAPolarity | Enum | Positive/Negative | Output A polarity |
| OutputBPolarity | Enum | Positive/Negative | Output B polarity |
| OutputCPolarity | Enum | Positive/Negative | Output C polarity |

### Inherited SDK3 Features

| Feature | Type | Description |
|---------|------|-------------|
| AOIWidth | Integer | Region of interest width |
| AOIHeight | Integer | Region of interest height |
| AOILeft | Integer | ROI left offset |
| AOITop | Integer | ROI top offset |
| ExposureTime | Float | Sensor exposure duration |
| FrameRate | Float | Acquisition frame rate |
| PixelEncoding | Enum | Mono12/Mono16/etc |
| TriggerMode | Enum | Internal/External/etc |
| SensorCooling | Boolean | Enable cooling |
| TargetSensorTemperature | Float | Cooling setpoint |
| SensorTemperature | Float | Current temperature (read-only) |
| CycleMode | Enum | Fixed/Continuous |
| FrameCount | Integer | Frames in fixed mode |

## DriverFactory Integration

```rust
use common::driver::{DriverFactory, DeviceComponents, Capability};
use futures::future::BoxFuture;

pub struct IStarScmosFactory;

impl DriverFactory for IStarScmosFactory {
    fn driver_type(&self) -> &'static str { "istar_scmos" }
    
    fn name(&self) -> &'static str { "Andor iStar-sCMOS Intensified Camera" }
    
    fn capabilities(&self) -> &'static [Capability] {
        &[
            Capability::Triggerable,
            Capability::FrameProducer,
            Capability::Parameterized,
            Capability::GatedImaging,    // Custom capability for intensifier
        ]
    }
    
    fn validate(&self, config: &toml::Value) -> Result<()> {
        // Validate MCP gain range
        if let Some(gain) = config.get("mcp_gain") {
            let g = gain.as_integer().unwrap_or(0);
            if g < 0 || g > 4095 {
                return Err(anyhow!("MCP gain must be 0-4095"));
            }
        }
        Ok(())
    }
    
    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let driver = Arc::new(IStarScmosDriver::new(&config).await?);
            Ok(DeviceComponents::new()
                .with_triggerable(driver.clone())
                .with_frame_producer(driver.clone())
                .with_parameterized(driver))
        })
    }
}
```

## Configuration Example

```toml
[[devices]]
id = "istar"
type = "istar_scmos"
enabled = true

[devices.config]
# Camera identification
serial_number = "VSC-12345"

# MCP settings
mcp_gain = 2048           # Mid-range gain
intelligate = true        # Enhanced gating

# DDG timing
gate_mode = "ddg"
gate_delay_ns = 100       # 100 ns delay
gate_width_ns = 50        # 50 ns width
insertion_delay = "ultra_fast"

# IOC settings
integrate_on_chip = false
ioc_pulse_count = 1

# Sensor settings
exposure_time_ms = 10
pixel_encoding = "mono16"
target_temperature_c = 0

# DDG outputs
output_a_delay_ns = 0
output_b_delay_ns = 1000
output_c_delay_ns = 2000
```

## Typical Workflow

### Time-Resolved Fluorescence Lifetime

```rust
// 1. Configure for lifetime measurement
driver.set_gate_mode(GateMode::Ddg).await?;
driver.set_intelligate(true).await?;

// 2. Set initial timing
driver.set_gate_delay(0.0).await?;    // Start at t=0
driver.set_gate_width(5e-9).await?;   // 5 ns gate

// 3. Configure DDG step for exponential sampling
driver.set_ddg_step_mode(DdgStepMode::Exponential {
    amplitude: 100e-12,  // 100 ps
    exponent: 0.694,     // Doubles each frame
}).await?;

// 4. Run kinetic series
driver.set_cycle_mode(CycleMode::Fixed).await?;
driver.set_frame_count(100).await?;
driver.start_acquisition().await?;

// 5. Process decay curve from acquired frames
```

### Fast Gating with External Trigger

```rust
// 1. Configure trigger
driver.set_trigger_mode(TriggerMode::External).await?;
driver.set_trigger_threshold(1.5).await?;  // 1.5V threshold

// 2. Configure fast gating
driver.set_gate_mode(GateMode::Ddg).await?;
driver.set_insertion_delay(InsertionDelay::UltraFast).await?;

// 3. Compensate for sensor latency
const SCMOS_LATENCY: f64 = 300e-9;
const ULTRA_FAST_DELAY: f64 = 35e-9;
driver.set_gate_delay(SCMOS_LATENCY - ULTRA_FAST_DELAY).await?;

// 4. Set gate width
driver.set_gate_width(10e-9).await?;  // 10 ns gate

// 5. Start and wait for triggers
driver.start_acquisition().await?;
```

## References

- Andor iStar sCMOS Hardware Guide
- Andor SDK3 Programming Manual
- `docs/guides/driver-andor-sdk3.md` - Base SDK3 API reference
