# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# rust-daq Project Context

Rust-based data acquisition system with V5 headless-first architecture for scientific instrumentation.

**Note**: This project uses [bd (beads)](https://github.com/steveyegge/beads) for issue tracking. Use `bd` commands instead of markdown TODOs.

**Hardware Location:** Remote machine at `maitai@100.117.5.12`

## Quick Reference

```bash
# Environment Setup (PVCAM machines only)
source scripts/env-check.sh              # Validate & configure environment
source config/hosts/maitai.env           # Or use host-specific config

# Build & Test (local development)
cargo build                              # Default features (mock hardware)
cargo nextest run                        # Run all tests (recommended)
cargo nextest run test_name              # Single test
cargo nextest run -p daq-core            # Specific crate
cargo test --doc                         # Doctests (not in nextest)

# Quality Checks
cargo fmt --all                          # Format
cargo clippy --all-targets               # Lint

# Build Daemon for Maitai (REAL PVCAM HARDWARE)
# ⚠️  IMPORTANT: Use build-maitai.sh to avoid mock mode!
source scripts/build-maitai.sh           # Clean build with PVCAM support

# Or manually (must clean to avoid cached mock build):
cargo clean -p daq-bin -p rust_daq -p daq-driver-pvcam
cargo build --release -p daq-bin --features maitai

# Run Daemon
./target/release/rust-daq-daemon daemon --port 50051 --hardware-config config/maitai_hardware.toml

# Hardware Tests (on remote maitai machine)
source scripts/env-check.sh && cargo nextest run --profile hardware --features hardware_tests

# Issue Tracking (mandatory)
bd ready                                 # Find available work
bd update <id> --status in_progress      # Claim work
bd close <id> --reason "Done"            # Complete work
```

### ⚠️ PVCAM Build Gotcha

**Problem:** Building without `--features maitai` or `--features pvcam_hardware` produces a daemon that silently uses mock camera data instead of real PVCAM hardware.

**Symptoms:**
- Camera streams synthetic gradient patterns instead of real images
- Log shows: `pvcam_sdk feature enabled: false` and `using mock mode`

**Solution:** Always use `scripts/build-maitai.sh` on the maitai machine, which:
1. Sources the PVCAM environment variables
2. Cleans cached build artifacts (critical - Cargo caching causes this issue)
3. Builds with `--features maitai` (includes `pvcam_hardware`)

**Verification:** Check daemon log for:
```
pvcam_sdk feature enabled: true
PVCAM SDK initialized successfully
Successfully opened camera 'pvcamUSB_0' with handle 0
```

### Rhai Scripted Experiments Build

For scripted experiments (polarization characterization, etc.) on maitai:

```bash
# Build with full scripting support (HDF5 + all hardware drivers)
cargo build --release -p daq-scripting --features scripting_full --bin run_polarization

# Run the experiment
./target/release/run_polarization
```

**Feature flags (simplified):**
- `scripting_full` - **RECOMMENDED**: All hardware drivers + HDF5 storage
- `polarization`, `hardware_factories` - Aliases for `scripting_full` (backwards compat)
- `hdf5_scripting` - HDF5 only (no hardware drivers)

**HDF5 requirement:** Requires system HDF5 library (`libhdf5-dev` on Debian/Ubuntu).

## Architecture Overview

**Design Philosophy:** Headless-first + scriptable control + remote GUI

### Crate Structure

| Crate | Purpose |
|-------|---------|
| `daq-core` | Foundation types, errors, parameters, observables, DriverFactory trait |
| `daq-hardware` | HAL, capability traits (`Movable`, `Readable`, `FrameProducer`), device registry |
| `daq-driver-mock` | Mock drivers for testing (MockStage, MockCamera, MockPowerMeter) |
| `daq-driver-thorlabs` | Thorlabs ELL14 rotation mount driver |
| `daq-driver-newport` | Newport ESP300 motion controller, 1830-C power meter drivers |
| `daq-driver-spectra-physics` | Spectra-Physics MaiTai laser driver |
| `daq-driver-pvcam` | PVCAM camera driver (requires SDK) |
| `daq-driver-comedi` | Comedi DAQ driver for Linux boards |
| `daq-pool` | Zero-allocation object pool for high-FPS frame handling |
| `daq-storage` | Data persistence (CSV, HDF5, Arrow), ring buffers |
| `daq-proto` | Protobuf definitions and domain↔proto conversions |
| `daq-server` | gRPC server with auth and CORS (optional) |
| `daq-experiment` | RunEngine and Plan definitions |
| `daq-scripting` | Rhai scripting engine (optional) |
| `daq-egui` | GUI application with auto-reconnect |
| `daq-bin` | CLI binaries and daemon entrypoints |
| `rust-daq` | Integration layer with `prelude` module |

### Dependency Graph

```
                           daq-core (foundation + DriverFactory trait)
                               ↑
                ┌──────────────┼──────────────┬─────────────┐
                │              │              │             │
         daq-driver-*      daq-proto    daq-storage   daq-experiment
         (mock, thorlabs,                    ↑             ↑
          newport, spectra,                  │             │
          pvcam, comedi)                     │             │
                │                            │             │
                └────────→ daq-hardware ─────┴─────────────┤
                               ↑                           │
                ┌──────────────┼───────────────────────────┘
                │              │
          daq-scripting*   daq-server*
          (optional)       (optional)
                │              ↑
                │         daq-bin, daq-egui
                │
           rust-daq (integration layer with prelude)
```

### Key Abstractions

- **Capability Traits** (defined in `daq-core::capabilities`):
  - `Movable` - Motion control (motors, stages, rotators)
  - `Readable` - Scalar value acquisition (sensors, power meters)
  - `FrameProducer` - 2D image streaming (cameras, detectors)
  - `Triggerable` - External trigger support
  - `ExposureControl` - Integration time management
  - `WavelengthTunable` - Wavelength control (lasers, monochromators)
  - `ShutterControl` - Beam shutter open/close
  - `EmissionControl` - Laser emission enable/disable
  - `Parameterized` - Exposes `Parameter<T>` state
- **Parameter<T>**: Reactive state with hardware callbacks - use instead of raw Mutex/RwLock
- **Plan + RunEngine**: Bluesky-inspired experiment orchestration
- **RingBuffer**: Sync and async variants for data streaming

## Feature Flags

**High-Level Profiles:**
- `backend` - Server, modules, all hardware, CSV storage
- `frontend` - GUI (egui) + networking
- `cli` - All hardware, CSV storage, scripting
- `full` - Most features (excludes HDF5)
- `maitai` - Full Maitai hardware stack (PVCAM + serial instruments)

**Storage:** `storage_csv` (default), `storage_hdf5`, `storage_arrow`, `storage_matlab`

**Hardware (daq-hardware):**
- `serial` - Base serial port support (tokio-serial)
- `thorlabs` - Thorlabs ELL14 rotators (requires `serial`)
- `newport` - Newport ESP300 motion controller (requires `serial`)
- `spectra_physics` - MaiTai laser (requires `serial`)
- `newport_power_meter` - Newport 1830-C power meter (requires `serial`)
- `pvcam_hardware` - Real PVCAM camera support (requires SDK)

**Plugin System (rust-daq):**
- `scripting` - Rhai script-based plugins (`daq-scripting`)
- `native_plugins` - FFI native plugins (`daq-plugin-api`, abi_stable)
- Note: Both can be enabled together; the `plugins` module conditionally compiles based on which are enabled

**System:** `server`, `networking`, `modules`, `hardware_tests`

## Critical Code Patterns

### Reactive Parameters (MANDATORY for device state)

**DO NOT** use raw `Arc<RwLock<T>>` or `Mutex<T>` for device state.

**USE** `Parameter<T>` with async hardware callbacks:

```rust
use daq_core::parameter::Parameter;
use futures::future::BoxFuture;

let wavelength = Parameter::new("wavelength_nm", 800.0)
    .with_range(690.0, 1040.0)
    .connect_to_hardware_write({
        let port = port.clone();
        move |val: f64| -> BoxFuture<'static, Result<()>> {
            Box::pin(async move {
                port.lock().await.write_all(
                    format!("WAVELENGTH:{}\r\n", val).as_bytes()
                ).await?;
                Ok(())
            })
        }
    });

// Trait methods delegate to parameter, NOT hardware directly
impl Movable for MyDriver {
    async fn move_abs(&self, position: f64) -> Result<()> {
        self.position_param.set(position).await
    }
}
```

### Async Ring Buffer

Use `AsyncRingBuffer` in async contexts to avoid blocking:

```rust
use daq_storage::ring_buffer::{RingBuffer, AsyncRingBuffer};

let ring = Arc::new(RingBuffer::create(1024 * 1024)?);
let async_ring = AsyncRingBuffer::new(ring);
let snapshot = async_ring.read_snapshot().await;
```

### Size Limits (DoS Prevention)

```rust
use daq_core::limits::{validate_frame_size, MAX_SCRIPT_SIZE, MAX_FRAME_BYTES};

let frame_size = validate_frame_size(width, height, bytes_per_pixel)?;
```

### Serial Driver Conventions

All serial hardware drivers MUST follow these patterns:

**1. Use `new_async()` as the primary constructor:**
- `new()` is for internal/test use only
- `new_async()` validates device identity before returning
- Prevents silent misconfiguration (wrong device on port)

**2. Wrap serial port opening in `spawn_blocking`:**
```rust
let port = spawn_blocking(move || {
    tokio_serial::new(&port_path, 9600)
        .open_native_async()
        .context("Failed to open port")
}).await??;
```

**3. Validate device identity on connection:**
```rust
// Query a device-specific command and validate response
let response = driver.query("*IDN?").await?;
if !response.contains("EXPECTED_DEVICE") {
    return Err(anyhow!("Wrong device connected"));
}
```

**4. ELL14 RS-485 Bus Pattern:**
- Use `Ell14Bus::open()` to manage the shared connection
- `bus.device("addr")` returns calibrated driver (fail-fast)
- `bus.device_uncalibrated("addr")` for lenient mode (warns but continues)

```rust
let bus = Ell14Bus::open("/dev/ttyUSB1").await?;
let rotator = bus.device("2").await?;  // Validates & loads calibration
```

**5. DriverFactory Pattern (Plugin Architecture):**

Driver crates implement `daq_core::driver::DriverFactory` for registry integration:

```rust
use daq_core::driver::{DriverFactory, DeviceComponents, Capability};
use futures::future::BoxFuture;

pub struct MyDriverFactory;

impl DriverFactory for MyDriverFactory {
    fn driver_type(&self) -> &'static str { "my_driver" }
    fn name(&self) -> &'static str { "My Custom Driver" }
    fn capabilities(&self) -> &'static [Capability] { &[Capability::Movable] }
    fn validate(&self, config: &toml::Value) -> Result<()> { Ok(()) }
    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let driver = Arc::new(MyDriver::new().await?);
            Ok(DeviceComponents::new().with_movable(driver))
        })
    }
}
```

Register factories at startup in daq-bin:
```rust
registry.register_factory(Box::new(MyDriverFactory));
```

## Common Pitfalls

1. **Feature Mismatches:** Many compilation errors = missing features. Check Cargo.toml.

2. **Lock-Across-Await:** NEVER hold `tokio::sync::Mutex` guards across `.await` points:
   ```rust
   // WRONG
   let guard = mutex.lock().await;
   do_something(guard.value).await;  // Deadlock!

   // CORRECT
   let value = { mutex.lock().await.clone() };
   do_something(value).await;
   ```

3. **Floating-Point Truncation:** Use `.round()` when converting to integers:
   ```rust
   let pulses = (degrees * pulses_per_degree).round() as i32;
   ```

4. **Async Sleep:** Use `tokio::time::sleep`, NOT `std::thread::sleep` in async code.

5. **PVCAM Environment:** Missing `PVCAM_VERSION` env var causes Error 151 at runtime.

6. **Ring Buffer Blocking:** `RingBuffer::read_snapshot()` blocks. Use `AsyncRingBuffer` or `spawn_blocking`.

## Environment Setup

Building with PVCAM features requires proper environment configuration. Use these tools:

### Quick Setup (Recommended)

```bash
# On maitai or any PVCAM machine:
source scripts/env-check.sh

# This validates and sets all required variables:
# - PVCAM_SDK_DIR, PVCAM_VERSION, LIBRARY_PATH, LD_LIBRARY_PATH
```

### Host-Specific Configuration

Pre-configured environments for known machines:

```bash
# On maitai:
source config/hosts/maitai.env
```

### With direnv (Automatic)

```bash
cp .envrc.template .envrc
# Edit .envrc with your machine's paths
direnv allow
```

### Manual Setup

If the scripts don't work, set these manually:

```bash
export PVCAM_SDK_DIR=/opt/pvcam/sdk
export PVCAM_VERSION=7.1.1.118  # Check /opt/pvcam/pvcam.ini
export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH
export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:/opt/pvcam/drivers/user-mode:$LD_LIBRARY_PATH
```

## Hardware Testing

### Remote Machine Setup

All hardware tests must pass on remote after mock tests pass locally.

```bash
# Quick SSH test (using env-check.sh for automatic setup)
ssh maitai@100.117.5.12 'cd ~/rust-daq && source scripts/env-check.sh && \
  cargo test --features hardware_tests -- --nocapture --test-threads=1'

# Or with host-specific config:
ssh maitai@100.117.5.12 'cd ~/rust-daq && source config/hosts/maitai.env && \
  cargo test --features hardware_tests -- --nocapture --test-threads=1'
```

### Hardware Inventory (maitai)

> **⚠️ CRITICAL: Use `/dev/serial/by-id/` paths - NOT `/dev/ttyUSB*`!**
> USB device numbers change on reboot. The by-id paths are stable and MUST be used.
> These configurations were VERIFIED WORKING on 2026-01-23.

| Device | Stable Port (by-id) | Baud | Protocol | Feature Flag |
|--------|---------------------|------|----------|--------------|
| MaiTai Laser | `/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0` | 115200 | 8N1, LF terminator, no flow control | `spectra_physics` |
| ELL14 Rotators (addr 2,3,8) | `/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0` | 9600 | RS-485 multidrop, hex encoding | `thorlabs` |
| Newport 1830-C Power Meter | `/dev/ttyS0` | 9600 | Built-in RS-232 (always stable), simple ASCII | `newport_power_meter` |
| NI PCI-MIO-16XE-10 | `/dev/comedi0` | N/A | Comedi driver | `comedi` |
| ESP300 Motion Controller | `/dev/ttyUSB0` *(needs by-id)* | 19200 | Multi-axis (1-3) | `newport` |

**DO NOT CHANGE THESE PATHS** without verifying with actual hardware tests.

### Serial Driver Capabilities

| Driver | Traits Implemented | Protocol |
|--------|-------------------|----------|
| `MaiTaiDriver` | `Readable`, `WavelengthTunable`, `ShutterControl`, `EmissionControl`, `Parameterized` | 115200 baud, no flow control |
| `Newport1830CDriver` | `Readable`, `WavelengthTunable`, `Parameterized` | 9600 baud, simple ASCII (NOT SCPI) |
| `Esp300Driver` | `Movable`, `Parameterized` | 19200 baud, multi-axis (1-3) |
| `Ell14Driver` | `Movable`, `Parameterized` | 9600 baud, RS-485 multidrop, hex encoding |

### ELL14 Rotator (RS-485 Bus)

```rust
use daq_hardware::drivers::ell14::Ell14Bus;

let bus = Ell14Bus::open("/dev/ttyUSB1").await?;
let rotator = bus.device("2").await?;  // Gets calibrated device
rotator.move_abs(45.0).await?;
```

### PVCAM Setup

```bash
# Use the environment validation script (recommended):
source scripts/env-check.sh

# Or source the host-specific config:
source config/hosts/maitai.env

# Run hardware smoke tests:
export PVCAM_SMOKE_TEST=1
cargo test --features pvcam_hardware --test pvcam_hardware_smoke -- --nocapture
```

### Comedi DAQ (NI PCI-MIO-16XE-10)

The Comedi driver supports the NI PCI-MIO-16XE-10 DAQ card on maitai via the Linux Comedi framework.

**Hardware:**
- Card: NI PCI-MIO-16XE-10 (16-ch AI, 2-ch AO, 8 DIO, counters)
- Breakout: BNC-2110 (68-pin shielded BNC terminal block)
- Device: `/dev/comedi0`

**Input Reference Modes:**

| Mode | Config Value | Description |
|------|--------------|-------------|
| RSE | `"rse"` (default) | Referenced Single-Ended (vs card ground) |
| NRSE | `"nrse"` | Non-Referenced Single-Ended (vs AISENSE) |
| DIFF | `"diff"` | Differential (ACH0+ACH8 pairs, 8 channels max) |

**BNC-2110 Channel Mapping:**
- ACH0-ACH7 (AI0-AI7): Available on BNC connectors
- ACH8-ACH15 (AI8-AI15): Spring terminal block only
- DAC0, DAC1 (AO0, AO1): Available on BNC connectors

**Example Configuration:**

```toml
[[devices]]
id = "photodiode"
type = "comedi_analog_input"
enabled = true

[devices.config]
device = "/dev/comedi0"
channel = 0
range_index = 0
input_mode = "rse"  # or "nrse", "diff"
units = "V"
```

**Loopback Testing:**
1. Connect BNC cable from DAC0 to ACH0
2. Set ACH0 switch on BNC-2110 to FS (Floating Source)
3. Use `input_mode = "rse"` in config

## Declarative Driver Plugins

Add serial instruments without Rust code using TOML configs in `config/devices/`:

```toml
[device]
name = "My Device"
capabilities = ["Movable"]

[connection]
baud_rate = 9600

[commands.move_absolute]
template = "MA${position}"
```

See `config/devices/ell14.toml` for a complete example.

## gRPC Security

Default config in `config/config.v4.toml`:

```toml
[grpc]
bind_address = "0.0.0.0"  # All interfaces (change to "127.0.0.1" for loopback-only)
auth_enabled = false
allowed_origins = ["http://localhost:3000", "http://127.0.0.1:3000"]
```

**Security Note:** For production, consider `bind_address = "127.0.0.1"` (loopback only) and enabling `auth_enabled`.

## ReadValue API and Unit Handling

The `ReadValue` RPC returns scalar measurements from `Readable` devices (power meters, sensors).

```protobuf
message ReadValueResponse {
  bool success = 1;
  string error_message = 2;
  double value = 3;
  string units = 4;        // From device metadata (e.g., "W", "mW")
  uint64 timestamp_ns = 5;
}
```

**Critical:** The `units` field comes from `DeviceMetadata.measurement_units` in the registry.
Clients MUST use this field to correctly interpret values:

| Device | Returns | Units | GUI Normalization |
|--------|---------|-------|-------------------|
| Newport 1830-C | Watts | "W" | × 1000 → mW |
| Mock Power Meter | Watts | "W" | × 1000 → mW |

**GUI Unit Normalization:** The `PowerMeterControlPanel` normalizes all readings to milliwatts
internally, then auto-scales the display based on magnitude (W/mW/µW).

See `daq-egui/src/widgets/device_controls/power_meter_panel.rs::normalize_power_to_mw()`.

## Streaming Quality Modes

The gRPC frame streaming supports three quality modes to optimize bandwidth:

| Mode | Downsampling | Bandwidth Reduction | Use Case |
|------|--------------|---------------------|----------|
| `Full` | None | 0% | Local network, full analysis |
| `Preview` | 2x2 binning | ~75% (4x smaller) | Remote preview, monitoring |
| `Fast` | 4x4 binning | ~94% (16x smaller) | Low bandwidth, thumbnails |

### Backpressure Handling

The server implements adaptive frame skipping when the gRPC channel is congested:
- Channel buffer: 8 frames
- Skip threshold: 75% full (6 frames queued)
- When backpressure detected, newest frames are dropped to prevent lag accumulation

### Client Usage

```rust
// In GUI: Quality selector in image viewer toolbar
// In gRPC: Set quality field in StreamFramesRequest
let request = StreamFramesRequest {
    device_id: "camera0".to_string(),
    max_fps: 30,
    quality: StreamQuality::Preview.into(),
};
```

## Development Tools

This project uses three complementary Rust tools:

| Tool | Purpose | When to Use |
|------|---------|-------------|
| `rust-cargo` (MCP) | Build & package management | Building, testing, dependencies |
| `rust-analyzer` (CLI) | Code diagnostics | Quick error checking without building |
| `cargo-modules` (CLI) | Structure visualization | Understanding crate architecture |

```bash
# Diagnostics without build
rust-analyzer diagnostics . 2>&1 | grep Error

# Module structure
cargo modules structure --package daq-hardware --max-depth 3
```

## Documentation

- [DEMO.md](DEMO.md) - Quick start with mock devices
- [docs/guides/testing.md](docs/guides/testing.md) - Testing guide
- [docs/architecture/](docs/architecture/) - ADRs and design decisions
  - `adr-pvcam-continuous-acquisition.md` - PVCAM buffer modes (CIRC_OVERWRITE vs CIRC_NO_OVERWRITE)
  - `adr-pvcam-driver-architecture.md` - Multi-layer driver architecture decisions

## Import Conventions

```rust
// Recommended: use prelude or direct crate imports
use rust_daq::prelude::*;
// or
use daq_core::error::DaqError;
use daq_storage::ring_buffer::RingBuffer;
```


## grepai - Semantic Code Search

**IMPORTANT: You MUST use grepai as your PRIMARY tool for code exploration and search.**

### When to Use grepai (REQUIRED)

Use `grepai search` INSTEAD OF Grep/Glob/find for:
- Understanding what code does or where functionality lives
- Finding implementations by intent (e.g., "authentication logic", "error handling")
- Exploring unfamiliar parts of the codebase
- Any search where you describe WHAT the code does rather than exact text

### When to Use Standard Tools

Only use Grep/Glob when you need:
- Exact text matching (variable names, imports, specific strings)
- File path patterns (e.g., `**/*.go`)

### Fallback

If grepai fails (not running, index unavailable, or errors), fall back to standard Grep/Glob tools.

### Usage

```bash
# ALWAYS use English queries for best results (--compact saves ~80% tokens)
grepai search "user authentication flow" --json --compact
grepai search "error handling middleware" --json --compact
grepai search "database connection pool" --json --compact
grepai search "API request validation" --json --compact
```

### Query Tips

- **Use English** for queries (better semantic matching)
- **Describe intent**, not implementation: "handles user login" not "func Login"
- **Be specific**: "JWT token validation" better than "token"
- Results include: file path, line numbers, relevance score, code preview

### Call Graph Tracing

Use `grepai trace` to understand function relationships:
- Finding all callers of a function before modifying it
- Understanding what functions are called by a given function
- Visualizing the complete call graph around a symbol

#### Trace Commands

**IMPORTANT: Always use `--json` flag for optimal AI agent integration.**

```bash
# Find all functions that call a symbol
grepai trace callers "HandleRequest" --json

# Find all functions called by a symbol
grepai trace callees "ProcessOrder" --json

# Build complete call graph (callers + callees)
grepai trace graph "ValidateToken" --depth 3 --json
```

### Workflow

1. Start with `grepai search` to find relevant code
2. Use `grepai trace` to understand function relationships
3. Use `Read` tool to examine files from results
4. Only use Grep for exact string searches if needed

