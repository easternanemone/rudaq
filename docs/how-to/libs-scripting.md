# LIBS Scripting Reference

This guide covers the Rhai scripting API for LIBS (Laser-Induced Breakdown Spectroscopy)
experiments in rust-daq.  Enable the bindings by building with `--features libs_scripting`.

## Quick Start

```rhai
// Minimal single-point LIBS acquisition
let cam   = create_andor_camera();
let spec  = create_andor_spectrograph();
let stage = create_dover_axis("X");

cam.set_gate_mode("DDG");
cam.set_ddg_timing(1300000, 10000000);  // 1.3 µs delay, 10 µs gate width (ps)
cam.set_mcp_gain(3600);
cam.arm();

spec.set_grating(1);
spec.set_wavelength(310.0);  // Al I line region

stage.move_abs(5.0);
stage.wait_settled();
// laser fires, camera acquires on trigger...
cam.stop_stream();
```

---

## Handle Types

### `GatedCamera` — Andor iStar ICCD

Created with `create_andor_camera()`.

| Method | Signature | Description |
|--------|-----------|-------------|
| `set_gate_mode` | `(mode: String)` | `"DDG"` (gated), `"CW"` (continuous) |
| `set_trigger_mode` | `(mode: String)` | `"External"`, `"Internal"`, `"Software"` |
| `set_ddg_timing` | `(delay_ps: int, width_ps: int)` | DDG delay and gate width in picoseconds |
| `set_mcp_gain` | `(gain: int)` | MCP intensifier gain, 0–4095 |
| `arm` | `()` | Arm for external trigger acquisition |
| `stop_stream` | `()` | Stop / disarm acquisition |
| `temperature` | `() -> float` | Sensor temperature in °C |
| `supports_ddg` | `() -> bool` | Whether DDG control is available |
| `supports_mcp_gain` | `() -> bool` | Whether MCP gain control is available |

**Typical LIBS camera settings:**
```rhai
cam.set_trigger_mode("External");  // triggered by laser or Dover TOP
cam.set_gate_mode("DDG");
cam.set_ddg_timing(1300000, 10000000);  // skip first 1.3 µs continuum; 10 µs window
cam.set_mcp_gain(3600);  // ~88% of maximum — good SNR for trace elements
cam.arm();
```

---

### `Spectrograph` — Andor Shamrock

Created with `create_andor_spectrograph()`.

| Method | Signature | Description |
|--------|-----------|-------------|
| `set_grating` | `(index: int)` | Select grating by index (1-based) |
| `get_grating` | `() -> int` | Current grating index |
| `set_wavelength` | `(nm: float)` | Center wavelength in nm |
| `get_wavelength` | `() -> float` | Current center wavelength in nm |
| `set_slit_width` | `(port: int, um: float)` | Slit width in µm (port 0 = entrance) |
| `get_calibration` | `(pixels: int) -> Array` | Wavelength axis for N pixels (nm per pixel) |

**Grating index conventions:** grating 1 is typically 1200 l/mm (UV/VIS), grating 2 is
300 l/mm (survey); confirm with your spectrograph configuration.

---

### `DoverAxis` — Dover SmartStage

Created with `create_dover_axis("X")`, `create_dover_axis("Y")`, or `create_dover_axis("Z")`.

| Method | Signature | Description |
|--------|-----------|-------------|
| `move_abs` | `(mm: float)` | Move to absolute position (mm) |
| `move_rel` | `(mm: float)` | Move relative distance (mm) |
| `position` | `() -> float` | Current position (mm) |
| `wait_settled` | `()` | Block until move completes |
| `set_velocity` | `(mm_s: float)` | Set motion velocity (mm/s) |
| `enable_top` | `(start, end, increment, bidir, pulse_ns)` | Arm Trigger-On-Position |
| `disable_top` | `()` | Disarm TOP |
| `top_enabled` | `() -> bool` | Query TOP state |

**TOP (Trigger-On-Position)** generates hardware trigger pulses at precise spatial intervals
during continuous motion, enabling camera acquisition without stop-and-go overhead:

```rhai
// Configure camera for external triggering first
cam.arm();

// Enable TOP: trigger every 0.1 mm from 0 to 20 mm, 1 µs pulse width
stage_x.enable_top(0.0, 20.0, 0.1, false, 1000);
stage_x.set_velocity(1.0);   // 1 mm/s → 10 Hz trigger rate
stage_x.move_abs(20.0);      // continuous scan: 200 hardware triggers emitted
stage_x.wait_settled();
stage_x.disable_top();
```

---

### `ScanController` — Synchronized Spectrograph + Camera

Created with `create_scan_controller(camera, spectrograph)`.

Coordinates the stop→tune→re-arm sequence required when changing the grating or
wavelength mid-experiment.  Use `set_config` instead of manually calling
`stop_stream` / `set_grating` / `set_wavelength` / `arm`.

| Method | Signature | Description |
|--------|-----------|-------------|
| `set_config` | `(grating: int, nm: float)` | Stop camera, tune spectrograph, re-arm |
| `set_wavelength` | `(nm: float)` | Tune wavelength without grating change |
| `camera` | `() -> GatedCamera` | Access the managed camera |
| `spectrograph` | `() -> Spectrograph` | Access the managed spectrograph |

```rhai
let sc = create_scan_controller(
    create_andor_camera(),
    create_andor_spectrograph(),
);

// Initial setup
sc.camera().set_ddg_timing(1300000, 10000000);
sc.camera().set_mcp_gain(3600);
sc.camera().arm();

// Change spectral region — safely stops and re-arms
sc.set_config(1, 310.0);   // grating 1, Al I 310 nm region
// ... acquire ...
sc.set_config(2, 450.0);   // grating 2, Fe I 450 nm region
// ... acquire ...
```

---

### `RadianceCalibrator` — Spectral Correction

Created with `create_radiance_calibrator(lamp_file, g1_file, g2_file)`.

Applies radiometric correction using a white-light lamp reference and per-grating
system-response calibration files (`.lmp`, `.asc`, `.csv`).

| Method | Signature | Description |
|--------|-----------|-------------|
| `calibrate` | `(wl: Array, vals: Array, grating: int, normalize: bool) -> Array` | Apply correction |
| `lamp_wavelengths` | `() -> Array` | Wavelength axis of the loaded lamp spectrum |
| `has_grating` | `(grating: int) -> bool` | Whether grating calibration is loaded |

```rhai
let cal = create_radiance_calibrator(
    "calibration/lamp_spec.lmp",
    "calibration/g1_cal.asc",
    "calibration/g2_cal.asc",
);

let wavelengths = spec.get_calibration(2560);
let raw_values  = [/* ... acquired intensities ... */];

// Normalize to correct for wavelength-dependent detector + grating efficiency
let corrected = cal.calibrate(wavelengths, raw_values, 1, false);
let normalized = cal.calibrate(wavelengths, raw_values, 1, true);  // max=1
```

**Calibration file format:** two-column plain text (wavelength\tintensity).
Lines starting with `#`, `%`, or `;` are skipped (comments/headers).

---

## Example Scripts

| Script | Description |
|--------|-------------|
| `examples/libs/step_scan.rhai` | 1-D line scan, stop-and-go |
| `examples/libs/step_scan_2d.rhai` | 2-D raster scan (serpentine) |
| `examples/libs/top_acquisition.rhai` | Continuous-motion TOP scan |
| `examples/libs/delay_scan.rhai` | DDG timing study |
| `examples/libs/multishot_scan.rhai` | Multi-shot accumulation per position |
| `examples/libs/multi_grating.rhai` | Multi-grating full-spectrum survey |
| `examples/libs/height_scan.rhai` | Z-axis focus optimization |
| `examples/libs/wavelength_scan.rhai` | Coordinated wavelength sweep (ScanController) |

---

## Feature Flags

```toml
# Cargo.toml
[dependencies]
scripting = { path = "crates/scripting", features = ["libs_scripting"] }
```

`libs_scripting` enables:
- `driver-andor-sdk3` (mock mode on non-Windows; real SDK on Windows)
- `driver-dover-motion` (mock mode on all platforms; real C API on any OS)
- `common::processing::radiance_calibration` (pure Rust, always available)

`scripting_full_libs` = `scripting_full` + `libs_scripting` (recommended for development).

---

## Python → Rhai Migration

| Python | Rhai equivalent |
|--------|-----------------|
| `LIBS.spc.SetGrating(g)` | `spec.set_grating(g)` |
| `LIBS.spc.SetWavelength(nm)` | `spec.set_wavelength(nm)` |
| `LIBS.cam.SetMCPGain(v)` | `cam.set_mcp_gain(v)` |
| `LIBS.stage_x.move_abs(mm)` | `stage_x.move_abs(mm)` |
| `LIBS.stage_x.wait_move_done()` | `stage_x.wait_settled()` |
| `LIBS.stage_x.enable_top(...)` | `stage_x.enable_top(...)` |
| `radiance_calibration(da, ...)` | `cal.calibrate(wl, vals, g, norm)` |

---

## See Also

- `docs/how-to/driver-andor-sdk3.md` — Andor SDK3 driver internals
- `docs/how-to/driver-dover-motion.md` — Dover Motion driver internals
- `docs/how-to/scripting.md` — General Rhai scripting guide
- `config/libs_hardware.toml` — Reference hardware configuration for Windows LIBS machine
