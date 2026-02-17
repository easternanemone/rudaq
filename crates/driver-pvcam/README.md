# driver-pvcam

PVCAM camera driver for rust-daq. Defaults to mock mode; hardware mode uses the Photometrics PVCAM SDK.

## Features

- `mock` (default): build without PVCAM SDK, uses synthetic frames.
- `pvcam_sdk`: enable PVCAM SDK bindings (requires env vars + libraries).

## Dynamic Discovery

The driver provides runtime discovery of camera capabilities rather than hardcoding assumptions.

### Camera Enumeration

```rust
use driver_pvcam::components::connection::PvcamConnection;

// Initialize SDK and list available cameras
let mut conn = PvcamConnection::new();
conn.initialize()?;
let cameras = PvcamConnection::list_available_cameras()?;
for name in &cameras {
    println!("Found camera: {}", name);
}
```

### Capability Discovery

```rust
use driver_pvcam::{PvcamDriver, PvcamFeatures};

let driver = PvcamDriver::new_async("PrimeBSI").await?;
let conn = driver.connection.lock().await;

// Discover exposure modes supported by this camera
let modes = PvcamFeatures::list_exposure_modes(&conn)?;
for (value, name) in &modes {
    println!("  {} ({})", name, value);
}

// Discover readout configuration options
let speeds = PvcamFeatures::list_speed_modes(&conn)?;
let gains = PvcamFeatures::list_gain_modes(&conn)?;
let ports = PvcamFeatures::list_readout_ports(&conn)?;

// Discover timing modes
let clear_modes = PvcamFeatures::list_clear_modes(&conn)?;
let expose_out_modes = PvcamFeatures::list_expose_out_modes(&conn)?;
```

### Available Discovery Functions

| Function | Returns | Description |
|----------|---------|-------------|
| `list_available_cameras()` | `Vec<String>` | Enumerate connected cameras |
| `list_exposure_modes()` | `Vec<(i32, String)>` | Trigger/timing modes |
| `list_clear_modes()` | `Vec<(i32, String)>` | Sensor clearing strategies |
| `list_expose_out_modes()` | `Vec<(i32, String)>` | Expose output signal modes |
| `list_speed_modes()` | `Vec<SpeedMode>` | Readout speed options |
| `list_readout_ports()` | `Vec<ReadoutPort>` | Port selection (if multiple) |
| `list_gain_modes()` | `Vec<GainMode>` | Gain/bit-depth options |
| `list_pp_features()` | `Vec<PpFeature>` | Post-processing features |
| `list_serial_binning()` | `Vec<i32>` | Serial binning factors |
| `list_parallel_binning()` | `Vec<i32>` | Parallel binning factors |

## SDK Compatibility

### Minimum Supported Version

- **PVCAM SDK:** 3.x or higher

### PVCAM 3.x Compatibility Fixes (PR #357)

The driver includes compatibility fixes for PVCAM 3.x API changes:

**1. `pl_io_script_control` signature change**

```rust
// PVCAM 2.x (old)
pl_io_script_control(hcam, addr, state)  // 3 args

// PVCAM 3.x (new, fixed in PR #357)
pl_io_script_control(hcam, addr, state, location)  // 4 args
// - state: flt64 (was uns32)
// - location: uns32 (new parameter)
```

**2. `pl_cam_get_diags` removed**

```rust
// PVCAM 2.x (removed in 3.x)
pl_cam_get_diags(hcam)  // No longer available

// PVCAM 3.x replacement (implemented in PR #357)
// Query PARAM_DD_INFO via pl_get_param()
let diag_info = query_param(hcam, PARAM_DD_INFO)?;
```

These fixes ensure compatibility with the latest PVCAM SDK while maintaining backward compatibility where possible.

## Environment (hardware)

Set before building or running with `--features pvcam_sdk`:

```bash
# Required at runtime (Error 151 if missing)
export PVCAM_VERSION=7.1.1.118

# SDK and library roots
export PVCAM_SDK_DIR=/opt/pvcam/sdk
export PVCAM_LIB_DIR=/opt/pvcam/library/x86_64

# Linker and runtime paths
export LIBRARY_PATH=$PVCAM_LIB_DIR:$LIBRARY_PATH
export LD_LIBRARY_PATH=/opt/pvcam/drivers/user-mode:$PVCAM_LIB_DIR:$LD_LIBRARY_PATH
```

**Quick setup (recommended on `maitai`):**

```bash
source /etc/profile.d/pvcam.sh
source /etc/profile.d/pvcam-sdk.sh
export PVCAM_SDK_DIR=/opt/pvcam/sdk
export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH
export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:/opt/pvcam/drivers/user-mode:$LD_LIBRARY_PATH
```

For deeper setup and debugging, see [PVCAM Setup & Troubleshooting](../../docs/troubleshooting/PVCAM_SETUP.md).

## Running PVCAM SDK examples (remote helper)

Use the helper to run upstream SDK binaries on the hardware host (defaults to `maitai@100.117.5.12`):

```bash
scripts/pvcam_sdk_examples.sh LiveImage
scripts/pvcam_sdk_examples.sh LiveImage_SmartStreaming
TIMEOUT_SECONDS=20 scripts/pvcam_sdk_examples.sh FastStreamingToDisk
```

The helper applies the required env vars and runs binaries from `/opt/pvcam/sdk/examples/code_samples/bin/linux-x86_64/release`.

## Testing

### Mock

```bash
cargo test -p driver-pvcam --no-default-features
```

### Hardware (Prime BSI)

Smoke and streaming (requires env above):

```bash
source /etc/profile.d/pvcam.sh
export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH

# Quick smoke
cargo test -p driver-pvcam --test hardware_smoke --features pvcam_sdk -- --nocapture --test-threads=1

# Continuous streaming suite (includes sustained run)
cargo test -p driver-pvcam --features pvcam_sdk --test continuous_acquisition_tier1 -- --nocapture --test-threads=1
```

Notes:
- Set `PVCAM_SMOKE_TEST=1` to enable the full smoke battery.
- Continuous tests exercise FIFO drain, auto-restart recovery from hardware stalls, and sustained 20s streaming.

## Examples

- SDK reference binaries: run via `scripts/pvcam_sdk_examples.sh` (see above) when comparing driver behavior to the vendor samples.
