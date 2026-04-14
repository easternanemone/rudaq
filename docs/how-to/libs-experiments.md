# LIBS Experiments Guide

This guide covers running Laser-Induced Breakdown Spectroscopy (LIBS) experiments with rust-daq. These experiments were ported from the Python [CAAAMLIBS/LIBS](https://github.com/CAAAMLIBS/LIBS) codebase.

## Hardware Requirements

| Device | Driver | Purpose |
|--------|--------|---------|
| Andor iStar sCMOS | `driver-andor-sdk3` | Gated ICCD camera (DDG + MCP) |
| Andor Shamrock | `driver-andor-sdk3` | Spectrograph (grating, wavelength, slit) |
| Dover SmartStage XYZ | `driver-dover-motion` | Sample positioning + Trigger-On-Position |
| NI-DAQmx | *planned* | Digital trigger routing (Dev1/ctr0) |
| Spirit Laser | *planned* | Femtosecond ablation laser (CANopen) |

## Quick Start

```bash
# Mock mode (no hardware needed)
cargo run -- run examples/libs/singleshot.rhai

# Real hardware
cargo run --features leabs_hardware -- run examples/libs/step_scan.rhai \
  --hardware-config config/libs_hardware.toml
```

## Available Experiment Scripts

All scripts are in `examples/libs/`. Each accepts the `--hardware-config` flag and works in both mock and hardware modes.

### Basic Measurements

| Script | Python Origin | Description |
|--------|--------------|-------------|
| `singleshot.rhai` | `singleshot()` | Single laser shot, one spectrum |
| `multishot_scan.rhai` | `multishot_step_scan_x()` | N shots accumulated per position |
| `live_custom.rhai` | `live_acquisition_custom()` | Interactive monitoring with custom params |

### 1D Scans

| Script | Python Origin | Description |
|--------|--------------|-------------|
| `step_scan.rhai` | `step_scan_x/y/z()` | Step-and-shoot line scan (any axis) |
| `height_scan.rhai` | `height_scan()` | Z-axis focus optimization |
| `wavelength_scan.rhai` | — | Sweep spectrograph center wavelength |

### 2D/3D Scans

| Script | Python Origin | Description |
|--------|--------------|-------------|
| `step_scan_2d.rhai` | `step_scan_2d()` | Nested X/Y grid with serpentine rows |
| `varied_pulses_2d.rhai` | `varied_pulses_2d()` | 2D grid varying pulse count per column |
| `depth_profile.rhai` | `custom_depth_scan()` | Repeated ablation for layer analysis |

### Timing & Trigger Studies

| Script | Python Origin | Description |
|--------|--------------|-------------|
| `delay_scan.rhai` | `delay_scan()` | Sweep DDG gate delay (plasma evolution) |
| `top_acquisition.rhai` | `TOP_acquisition()` | Trigger-On-Position continuous scan |
| `rapidfire_scan.rhai` | `rapidfire_moving_constantLIBS()` | High-speed constant-velocity scan |

### Multi-Grating

| Script | Python Origin | Description |
|--------|--------------|-------------|
| `multi_grating.rhai` | `multi_grating_pos()` | Stitch spectra across grating positions |

## Key Concepts

### DDG (Digital Delay Generator)

The iStar camera uses DDG timing to gate spectral collection relative to the laser pulse:

- **Delay** (ps): Time after laser pulse before opening the gate. Typically 1–2 µs to skip continuum radiation.
- **Width** (ps): Duration the gate stays open. Typically 10 µs for atomic emission.
- **MCP Gain** (0–4095): Microchannel plate intensifier gain. 3600 is standard for LIBS.

```rhai
camera.set_gate_mode("DDG");
camera.set_ddg_timing(1300000, 10000000);  // 1.3 µs delay, 10 µs width
camera.set_mcp_gain(3600);
```

### Trigger-On-Position (TOP)

Dover stages can output trigger pulses at fixed spatial intervals during motion. This synchronizes the camera to stage position rather than time:

```rhai
let stage = create_dover_axis("Y");
stage.enable_top(0.0, 10.0, 0.5, false, 1000);  // 0→10mm, 0.5mm steps, 1µs pulse
stage.set_velocity(10.0);                          // 10 mm/s
stage.move_abs(10.0);                              // triggers fire during motion
stage.wait_settled();
stage.disable_top();
```

### Radiance Calibration

Correct for system spectral response using a calibrated lamp:

```rhai
let cal = create_radiance_calibrator("DH-3P", "g1_cal.toml", "g2_cal.toml");
// Apply to acquired data via the RunEngine calibration pipeline
```

## Configuration

### Hardware Config (`config/libs_hardware.toml`)

Defines all LIBS devices. Active devices: Dover XYZ stages, iStar camera, Shamrock spectrograph. Placeholder sections for Spirit laser and NI-DAQmx trigger routing.

### Calibration Profile (`config/profiles/libs_calibration.toml`)

Conservative settings for calibration lamp measurements (low MCP gain, CW gate mode).

## Experiments Not Ported

The following Python experiments are intentionally **not** ported as standalone scripts because they are either too hardware-specific or better served by composing the building blocks above:

- `YLS3000_Power_Scan`, `YLS3000_Hatch_Scan` — YLS-laser-specific with hardcoded paths
- `O2_scan`, `velocity_scan` — Interactive `input()` workflows with hardcoded paths
- `binning_optimization_*` — One-off ROI optimization diagnostics
- `laser_bkg_test` — Background characterization (use `singleshot.rhai` with shutter closed)

These patterns can be recreated by modifying the existing scripts or composing Rhai building blocks.

## Testing

```bash
# Mock integration tests (always run)
cargo nextest run --test libs_integration_smoke

# Hardware tests (requires LIBS hardware + env vars)
export LIBS_INTEGRATION_TEST=1
cargo nextest run --profile libs-hardware --features hardware_tests --test libs_integration_smoke
```
