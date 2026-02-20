# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`AGENTS.md` is the canonical policy document. If there is any conflict, follow `AGENTS.md`.

## Build / Test / Lint

```bash
cargo build                              # Default features: storage_csv, instrument_serial
cargo nextest run                        # Parallel test runner (install: cargo install cargo-nextest --locked)
cargo nextest run test_name              # Single test by name
cargo nextest run -p common              # Single crate
cargo nextest run --profile ci           # CI profile (3 retries, no fail-fast)
cargo test --doc                         # Doctests (nextest doesn't support these)
cargo fmt --all                          # Format
cargo clippy --all-targets               # Lint
```

### Maitai Hardware Build (Critical)

Always use `bash scripts/build-maitai.sh` for real hardware. Building without `--features maitai` silently selects mock PVCAM paths. Verify: daemon log shows `pvcam_sdk feature enabled: true` and registers expected physical devices.

### Hardware Tests (maitai only)

```bash
source scripts/env-check.sh && cargo nextest run --profile hardware --features hardware_tests
```

## Architecture

### Crate Dependency Layers (~17 core crates)

```
Foundation
  common           ← Capability traits, Parameter<T>, DaqError, Frame, DriverFactory
  pool             ← Lock-free object pool for zero-allocation frame handling
  protocol         ← Protobuf definitions (daq.proto, health.proto, ni_daq.proto)

Hardware Core
  hardware         ← HAL: DeviceRegistry (DashMap-backed), factory orchestration, plugin system
  driver-mock      ← Always compiled; used for testing and demos
  driver-universal ← TOML-manifest driver for text-protocol devices (the forward path)

Native SDK Drivers (FFI-bound, irreplaceable by manifests)
  driver-pvcam          ← Photometrics cameras (+ pvcam-sys bindgen)
  driver-andor-sdk3     ← Andor cameras (+ andor-sdk3-sys bindgen)
  driver-comedi         ← Linux DAQ cards (+ comedi-sys bindgen)
  driver-dover-motion   ← Dover/Cellino stages (+ dover-motion-sys bindgen)

Legacy Serial Drivers (soft-deprecated → driver-universal manifests)
  driver-thorlabs, driver-newport, driver-spectra-physics, driver-generic,
  driver-red-pitaya
  ↳ Still functional but new text-protocol devices should use config/devices/*.toml

Engine
  experiment       ← Bluesky-style RunEngine + Plan trait (PlanCommand yields)
  scripting        ← Rhai engine with hardware bindings, optional PyO3
  daq-modules      ← PyMoDAQ/DynExp-style module system with Bluesky lifecycle

Services & Storage
  server           ← gRPC services: Hardware, Scan, RunEngine, Storage, Plugin, etc.
  client           ← gRPC client library
  db               ← SurrealDB control-plane (kv-mem for tests, kv-rocksdb for prod)
  storage          ← RingBuffer (mmap, seqlock), HDF5, Arrow, Tiff, Zarr writers

Applications
  bin              ← CLI daemon (mimalloc allocator), reconciler, safety sentinel
  ui               ← Web-based user interface

Testing & Plugins
  integration-tests ← Cross-crate integration test suite
  plugin-api        ← Plugin system API definitions
  plugin-example    ← Example plugin implementation
```

> **`driver-universal` is the forward path.** New serial/TCP/SCPI devices should be defined as schema v3 TOML manifests in `config/devices/`, not as new driver crates. See `docs/how-to/legacy-scpi-deprecation.md`.

### Key Abstractions

**Capability traits** (`common/src/capabilities.rs`): `Movable`, `Readable`, `FrameProducer`, `Triggerable`, `ExposureControl`, `ShutterControl`, `WavelengthTunable`, etc. All are `async_trait + Send + Sync`. Devices are defined by what they *do*, not what they *are*.

**`DeviceComponents`** (`common/src/driver.rs`): Capability bag returned by `DriverFactory::build()` — one `Option<Arc<dyn Trait>>` per capability. The `DeviceRegistry` stores these and provides typed accessors (`get_movable("stage_1")`).

**`Parameter<T>`** (`common/src/parameter.rs`): Reactive state inspired by QCodes/ScopeFoundry. Wraps `Observable<T>` + hardware callbacks. Flow: `set(value)` → validate constraints → call `hardware_writer` (async BoxFuture) → update internal value (notifies subscribers) → call change listeners. Use `Parameter<T>` for device state, never raw `Arc<Mutex<T>>`.

**`Plan` + `RunEngine`** (`experiment/src/`): Bluesky-inspired. Plans yield `PlanCommand` variants (`MoveTo`, `Read`, `Trigger`, `Wait`, `Checkpoint`, `EmitEvent`). RunEngine executes them as a state machine (`Idle → Running → Paused → Aborting`) and emits Bluesky-style documents (`Start`, `Descriptor`, `Event`, `Stop`, `Manifest`).

**`RingBuffer`** (`storage/src/ring_buffer.rs`): mmap-backed circular buffer with seqlock for lock-free reads. Uses Apache Arrow IPC format. "Tap" consumers receive every Nth frame via async channel for live visualization without blocking writers.

### DriverFactory Pattern

> **For serial/TCP/SCPI devices**, prefer writing a `config/devices/*.toml` manifest for `driver-universal` over implementing `DriverFactory` directly. The pattern below is for native SDK drivers that need custom FFI bindings.

```rust
// 1. Implement the trait
impl DriverFactory for MyDeviceFactory {
    fn driver_type(&self) -> &'static str { "my_device" }
    fn capabilities(&self) -> &'static [Capability] { &[Capability::Movable] }
    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> { ... }
}

// 2. Register with DeviceRegistry
registry.register_factory(Box::new(MyDeviceFactory));

// 3. Instantiate from TOML config
registry.register_from_config(DeviceConfig { id, name, driver: DriverConfig { type: "my_device", ... } }).await?;
```

### Feature Flags

**Compile-time** (Cargo features in `hardware/Cargo.toml`): `serial`, `thorlabs`, `newport`, `spectra_physics`, `pvcam`/`pvcam_sdk`/`pvcam_hardware`, `comedi`/`comedi_hardware`, `andor`/`andor_hardware`, `all_hardware`, `full`. Mock drivers (`driver-mock`) are always compiled.

**Runtime** (`config/feature_flags.toml`): Loaded via `FeatureFlags::load()`. Toggles: `frame_pool_preallocation`, `async_ring_buffer`, `experimental_streaming`, `debug_frame_timing`, etc.

## Code Style

- Rust 1.75+, async/await everywhere (Tokio runtime). Never `std::thread::sleep` in async code.
- Error handling: propagate with `?`, add context via `anyhow::Context`. No `.unwrap()` in library code (CI enforces `clippy::unwrap_used`); use `.expect("reason")` for invariants.
- Hardware state: always use `Parameter<T>` with `BoxFuture<'static, Result<()>>` callbacks.
- Workspace clippy: pedantic lints enabled with project-specific allows (see `Cargo.toml` `[workspace.lints.clippy]`).

## Testing Patterns

- **Nextest profiles**: `default` (local, 2 retries), `ci` (3 retries, no fail-fast), `hardware` (single-threaded, 6min timeout), `libs-hardware` (inherits hardware), `coverage` (no retries).
- **Test groups**: `serial-hardware`, `pvcam-hardware`, `elliptec-hardware` — each max-threads=1 for shared resource serialization.
- **Mock devices**: Always available without feature flags. Use `register_mock_factories(&registry)` for integration tests.
- **Timing tests**: Use `#[tokio::test(start_paused = true)]` with `tokio::time::Instant` for deterministic timing. Wall-clock tests use `TimingTolerance` helpers from `integration-tests/tests/common/`.
- **Hardware gating**: `#[cfg(feature = "hardware_tests")]` + `#[ignore]` for real-device tests.

## Tools & Workflow

**Issue tracking**: Use `bd` (beads) for ALL task tracking. Never use markdown TODOs. Statuses: `open`, `in_progress`, `blocked`, `closed`. Priorities: 0=critical through 4=backlog.

**Code search**: Primary tool is `grepai search "query" --json --compact`. Trace calls with `grepai trace callers/callees "Symbol" --json`. Fall back to `rg`/`grep` if grepai is unavailable.

**Structural search**: `sg` (ast-grep) for AST-aware code patterns. E.g., `sg -p '$EXPR.unwrap()' --lang rust`.

**Quality gates**: `bd close` runs lightweight check (fmt + ast-grep). `git push` runs full gate (fmt + clippy + tests).

**LSP**: `rust-analyzer` enabled via `.claude/settings.json`.

## Key Scripts

| Script | Purpose |
|--------|---------|
| `scripts/build-maitai.sh` | Full hardware build for maitai machine |
| `scripts/build-lab.sh [--release]` | Build daemon with pvcam_sdk for lab |
| `scripts/demo.sh` | Mock-hardware demo (daemon + GUI/script) |
| `scripts/env-check.sh` | Source before hardware tests |
| `scripts/install-hooks.sh [quick]` | Pre-commit hooks (full or format-only) |
| `scripts/run-ast-grep.sh` | AST-grep structural search helper |
| `scripts/target-maintenance.sh` | Clean bloated target/ directory |
| `scripts/bd-safe.sh` | Worktree-safe beads commands |

## References

- Canonical agent policy: `AGENTS.md`
- Testing details: `docs/how-to/testing.md`
- Feature flags: `config/feature_flags.toml`
- Architecture deep-dive: `docs/explanation/architecture.md`
- Hardware setup: `docs/how-to/hardware-setup.md`
- Driver guide: `docs/how-to/hardware-drivers.md`
