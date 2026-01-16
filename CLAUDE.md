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

## Architecture Overview

**Design Philosophy:** Headless-first + scriptable control + remote GUI

### Crate Structure

| Crate | Purpose |
|-------|---------|
| `daq-core` | Foundation types, errors, parameters, observables, size limits |
| `daq-hardware` | HAL, capability traits (`Movable`, `Readable`, `FrameProducer`), serial drivers |
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
                           daq-core (foundation)
                               ↑
                ┌──────────────┼──────────────┬─────────────┐
                │              │              │             │
         daq-driver-pvcam  daq-proto    daq-storage   daq-experiment
         daq-driver-comedi                   ↑             ↑
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

- **Capability Traits**: `Movable`, `Readable`, `FrameProducer`, `Triggerable`, `ExposureControl` - define what hardware can do
- **Parameter<T>**: Reactive state with hardware callbacks - use instead of raw Mutex/RwLock
- **Plan + RunEngine**: Bluesky-inspired experiment orchestration
- **RingBuffer**: Sync and async variants for data streaming

## Feature Flags

**High-Level Profiles:**
- `backend` - Server, modules, all hardware, CSV storage
- `frontend` - GUI (egui) + networking
- `cli` - All hardware, CSV storage, scripting
- `full` - Most features (excludes HDF5)

**Storage:** `storage_csv` (default), `storage_hdf5`, `storage_arrow`, `storage_matlab`

**Hardware (daq-hardware):** `serial`, `driver-thorlabs`, `driver-newport`, `driver-spectra-physics`, `driver_pvcam`, `pvcam_hardware`

**System:** `scripting`, `server`, `networking`, `hardware_tests`

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

### Serial Port Inventory (maitai)

| Device | Port | Driver |
|--------|------|--------|
| Newport 1830-C Power Meter | `/dev/ttyS0` | `Newport1830CDriver` |
| MaiTai Laser | `/dev/ttyUSB5` | `MaiTaiDriver` |
| ELL14 Rotators (addr 2,3,8) | `/dev/ttyUSB1` | `Ell14Driver` |
| ESP300 Motion Controller | `/dev/ttyUSB0` | `Esp300Driver` |

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
