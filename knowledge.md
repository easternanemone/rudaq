# Project knowledge

This file gives Codebuff context about your project: goals, commands, conventions, and gotchas.

## What is this?

**rust-daq** — A modular, high-performance, headless-first Data Acquisition (DAQ) system in Rust for scientific experiments. Designed for precise hardware control, high-throughput data streaming, and scripted automation.

**Hardware Location:** Remote machine at `maitai@100.117.5.12`

**Issue Tracking:** Uses [bd (beads)](https://github.com/steveyegge/beads) — run `bd ready` to find work.

## Quickstart

```bash
# Setup: Install Rust 1.75+ and cargo-nextest
cargo install cargo-nextest --locked

# Build daemon (mock hardware by default)
cargo build -p daq-bin

# Run daemon with demo hardware
cargo run --bin rust-daq-daemon -- daemon --hardware-config config/demo.toml

# Run a demo script
cargo run --bin rust-daq-daemon -- run examples/demo_scan.rhai

# Test
cargo nextest run           # All tests (recommended)
cargo test --doc            # Doctests (not in nextest)

# Quality checks
cargo fmt --all             # Format
cargo clippy --all-targets  # Lint
```

### PVCAM Build (Real Hardware)

**⚠️ CRITICAL:** Building without `--features maitai` silently uses mock camera!

```bash
# Always use the build script on maitai
source scripts/build-maitai.sh

# Verify in logs:
# pvcam_sdk feature enabled: true
# PVCAM SDK initialized successfully
```

## Architecture

### Key directories
- `crates/` — Workspace with all crates
  - `daq-core` — Foundation types, errors, `Parameter<T>`, capability traits
  - `daq-hardware` — HAL with capability traits (`Movable`, `Readable`, `FrameProducer`)
  - `daq-driver-*` — Hardware drivers (mock, pvcam, thorlabs, newport, spectra-physics)
  - `daq-server` — gRPC server implementation
  - `daq-egui` — Desktop GUI application
  - `daq-scripting` — Rhai scripting engine
  - `daq-bin` — CLI binaries and daemon
- `config/` — Configuration files (TOML)
- `examples/` — Demo scripts (Rhai)

### Data flow
1. **Rhai scripts** or **gRPC clients** → **daemon**
2. **RunEngine** executes **Plans** (Bluesky-inspired)
3. **HAL** abstracts hardware via capability traits
4. **Storage** persists data (CSV, HDF5, Arrow)

## Critical Code Patterns

### Reactive Parameters (MANDATORY)

**DO NOT** use raw `Arc<RwLock<T>>` or `Mutex<T>` for device state.

**USE** `Parameter<T>` with async hardware callbacks:

```rust
use daq_core::parameter::Parameter;
use futures::future::BoxFuture;

let wavelength = Parameter::new("wavelength_nm", 800.0)
    .with_range(690.0, 1040.0)
    .connect_to_hardware_write(move |val| -> BoxFuture<'static, Result<()>> {
        Box::pin(async move {
            // Write to hardware here
            Ok(())
        })
    });

// Trait methods delegate to parameter
impl Movable for MyDriver {
    async fn move_abs(&self, position: f64) -> Result<()> {
        self.position_param.set(position).await
    }
}
```

### Serial Driver Conventions

1. **Use `new_async()`** as primary constructor (validates device identity)
2. **Wrap serial port opening** in `spawn_blocking`
3. **Validate device identity** on connection before returning
4. **Use `/dev/serial/by-id/` paths** — NOT `/dev/ttyUSB*` (changes on reboot)

### DriverFactory Pattern

```rust
use daq_core::driver::{DriverFactory, DeviceComponents, Capability};

impl DriverFactory for MyDriverFactory {
    fn driver_type(&self) -> &'static str { "my_driver" }
    fn capabilities(&self) -> &'static [Capability] { &[Capability::Movable] }
    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let driver = Arc::new(MyDriver::new().await?);
            Ok(DeviceComponents::new().with_movable(driver))
        })
    }
}
```

## Conventions

### Import Conventions

```rust
// Recommended: use prelude or direct crate imports
use rust_daq::prelude::*;
// or
use daq_core::error::DaqError;
use daq_storage::ring_buffer::RingBuffer;
```

### Capability Traits
- `Movable` — Motion control (motors, stages, rotators)
- `Readable` — Scalar value acquisition (sensors, power meters)
- `FrameProducer` — 2D image streaming (cameras)
- `Triggerable` — External trigger support
- `ExposureControl` — Integration time
- `WavelengthTunable` — Wavelength control (lasers)
- `ShutterControl` — Beam shutter
- `EmissionControl` — Laser emission

### Formatting/linting
- `cargo fmt --all` before commits
- `cargo clippy --all-targets` must pass

## Things to Avoid (Common Pitfalls)

1. **Lock-Across-Await:** NEVER hold `tokio::sync::Mutex` guards across `.await`:
   ```rust
   // WRONG - Deadlock!
   let guard = mutex.lock().await;
   do_something(guard.value).await;

   // CORRECT
   let value = { mutex.lock().await.clone() };
   do_something(value).await;
   ```

2. **`std::thread::sleep`** in async code — use `tokio::time::sleep`

3. **Floating-Point Truncation:** Use `.round()` when converting to integers:
   ```rust
   let pulses = (degrees * pulses_per_degree).round() as i32;
   ```

4. **Assuming features:** Check Cargo.toml for available features before using

5. **PVCAM Environment:** Missing `PVCAM_VERSION` env var causes Error 151 at runtime

6. **Ring Buffer Blocking:** `RingBuffer::read_snapshot()` blocks — use `AsyncRingBuffer`

## Feature Flags

| Profile | Description |
|---------|-------------|
| `maitai` | Full hardware stack (PVCAM + serial instruments) |
| `backend` | Server, modules, all hardware, CSV storage |
| `frontend` | GUI (egui) + networking |
| `cli` | All hardware, CSV storage, scripting |
| `scripting_full` | All hardware drivers + HDF5 storage |

**Storage:** `storage_csv` (default), `storage_hdf5`, `storage_arrow`

**Hardware (daq-hardware):**
- `serial` — Base serial port support
- `thorlabs` — Thorlabs ELL14 rotators
- `newport` — Newport ESP300 motion controller
- `spectra_physics` — MaiTai laser
- `newport_power_meter` — Newport 1830-C
- `pvcam_hardware` — Real PVCAM camera (requires SDK)

## Hardware Inventory (maitai)

| Device | Stable Port (by-id) | Baud | Feature Flag |
|--------|---------------------|------|--------------|
| MaiTai Laser | `usb-Silicon_Labs_CP2102_...-port0` | 115200 | `spectra_physics` |
| ELL14 Rotators | `usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0` | 9600 | `thorlabs` |
| Newport 1830-C | `/dev/ttyS0` (native RS-232) | 9600 | `newport_power_meter` |
| NI PCI-MIO-16XE-10 | `/dev/comedi0` | N/A | `comedi` |

## Remote Hardware Testing

```bash
# Quick SSH test on maitai (using env-check.sh for automatic setup)
ssh maitai@100.117.5.12 'cd ~/rust-daq && source scripts/env-check.sh && \
  cargo test --features hardware_tests -- --nocapture --test-threads=1'
```

## Environment Setup (PVCAM)

```bash
# On maitai - use the validation script
source scripts/env-check.sh

# Or source host-specific config
source config/hosts/maitai.env

# Required env vars:
# PVCAM_SDK_DIR=/opt/pvcam/sdk
# PVCAM_VERSION=7.1.1.118
# LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH
# LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:/opt/pvcam/drivers/user-mode:$LD_LIBRARY_PATH
```

## AI Agent Tools

### grepai (Optional)

If installed, AI coding agents can use [grepai](https://yoanbernabeu.github.io/grepai/) for semantic code search:

```bash
# Semantic search (instead of grep for intent-based queries)
grepai search "error handling middleware" --json --compact

# Call graph tracing
grepai trace callers "HandleRequest" --json
grepai trace callees "ProcessOrder" --json
```

Fall back to standard grep/ripgrep if grepai is not available.

## More Info

- [CLAUDE.md](CLAUDE.md) — Detailed development guide
- [DEMO.md](DEMO.md) — Try without hardware
- [docs/guides/testing.md](docs/guides/testing.md) — Testing guide
- [docs/architecture/ARCHITECTURE.md](docs/architecture/ARCHITECTURE.md) — System design
