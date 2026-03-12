# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`AGENTS.md` is the canonical agent policy (local/gitignored, auto-injected by Claude Code hooks). `bdh prime` provides the authoritative beads workflow.

## Build / Test / Lint

```bash
cargo build                              # Default feature set (see Cargo.toml)
cargo nextest run                        # Parallel test runner (install: cargo install cargo-nextest --locked)
cargo nextest run test_name              # Single test by name
cargo nextest run -p common              # Single crate
cargo nextest run --profile ci           # CI profile (3 retries, no fail-fast)
cargo nextest run --workspace --exclude ui --exclude comedi-sys --exclude driver-comedi --profile ci  # Full CI parity test slice
cargo check -p ui --lib --target wasm32-unknown-unknown --no-default-features --features web  # UI WASM compile smoke (CI parity)
cargo test --doc                         # Doctests (nextest doesn't support these)
cargo fmt --all                          # Format
cargo fmt --all -- --check               # Format check (CI/pre-push parity)
cargo clippy --all-targets               # Lint
cargo clippy --workspace --all-targets --exclude ui --exclude comedi-sys --exclude driver-comedi -- -D warnings  # Clippy gate (CI/pre-push parity)
```

### Maitai Hardware Build (Critical)

Always use `bash scripts/build-maitai.sh` for real hardware. Building without `--features maitai` silently selects mock PVCAM paths. Verify: daemon log shows `pvcam_sdk feature enabled: true` and registers expected physical devices.

### Hardware Tests (maitai only)

```bash
source scripts/env-check.sh && cargo nextest run --profile hardware --features hardware_tests
```

## Architecture

### Crate Dependency Layers (25 workspace crates)

```
Foundation
  common           ← Capability traits, Parameter<T>, DaqError, Frame, DriverFactory
  pool             ← Lock-free object pool for zero-allocation frame handling
  protocol         ← Protobuf definitions (daq.proto, experiment.proto, hardware.proto, health.proto, ni_daq.proto, storage.proto)

Hardware Core
  hardware         ← HAL: DeviceRegistry (DashMap-backed), factory orchestration, plugin system
  driver-registry  ← Concrete factory registration, hardware feature gating
  driver-mock      ← Always compiled; used for testing and demos
  driver-universal ← TOML-manifest driver for text-protocol devices (the forward path)

Native SDK Drivers (FFI-bound, irreplaceable by manifests)
  driver-pvcam          ← Photometrics cameras (+ pvcam-sys bindgen)
  driver-andor-sdk3     ← Andor cameras (+ andor-sdk3-sys bindgen)
  driver-comedi         ← Linux DAQ cards (+ comedi-sys bindgen)
  driver-dover-motion   ← Dover/Cellino stages (+ dover-motion-sys bindgen)

Engine
  experiment       ← Bluesky-style RunEngine + Plan trait (PlanCommand yields)
  scripting        ← Rhai engine with hardware bindings, optional PyO3
  daq-modules      ← PyMoDAQ/DynExp-style module system with Bluesky lifecycle

Services & Storage
  server           ← gRPC services: Hardware, Scan, RunEngine, Storage, Plugin, etc.
  client           ← gRPC client library
  db               ← SurrealDB control-plane (kv-mem for tests, kv-rocksdb for prod)
  storage          ← RingBuffer (mmap, seqlock), HDF5, Arrow IPC, Parquet, Tiff, Zarr writers

Applications
  bin              ← CLI daemon (mimalloc allocator), reconciler, safety sentinel, safety heartbeat
  ui               ← Web-based user interface

Testing
  integration-tests ← Cross-crate integration test suite
```

> **`driver-universal` is the forward path.** New serial/TCP/SCPI devices should be defined as schema v3 TOML manifests in `config/devices/`, not as new driver crates. See `docs/how-to/legacy-scpi-deprecation.md`.

### Key Abstractions

**Capability traits** (`common/src/capabilities.rs`): `Movable`, `Readable`, `FrameProducer`, `Triggerable`, `ExposureControl`, `ShutterControl`, `WavelengthTunable`, `EmissionControl`, `Stageable`, `Settable`, `Switchable`, `Actionable`, `Loggable`, `Parameterized`, `Camera`, `Commandable`, `GatedCamera`, `SpectrometerControl`, `TriggerOnPosition`, `PulseGenerator`, `SafetyInterlock`, `Reconfigurable`, etc. All are `async_trait + Send + Sync`. Devices are defined by what they *do*, not what they *are*.

**`DeviceComponents`** (`common/src/driver.rs`): Capability bag returned by `DriverFactory::build()` — one `Option<Arc<dyn Trait>>` per capability. The `DeviceRegistry` stores these and provides typed accessors (`get_movable("stage_1")`).

**`Parameter<T>`** (`common/src/parameter.rs`): Reactive state inspired by QCodes/ScopeFoundry. Wraps `Observable<T>` + hardware callbacks. Flow: `set(value)` → validate constraints → call `hardware_writer` (async BoxFuture) → update internal value (notifies subscribers) → call change listeners. Use `Parameter<T>` for device state, never raw `Arc<Mutex<T>>`.

**`Plan` + `RunEngine`** (`experiment/src/`): Bluesky-inspired. Plans yield `PlanCommand` variants (`MoveTo`, `Read`, `Trigger`, `Wait`, `Checkpoint`, `EmitEvent`, `Set`). RunEngine executes them as a state machine (`Idle → Running → Paused → Aborting`) and emits Bluesky-style documents (`Start`, `Descriptor`, `Event`, `Stop`, `Manifest`). `Set` variant: `Set { device_id, parameter, value }` — set a device parameter.

**`RingBuffer`** (`storage/src/ring_buffer.rs`): mmap-backed circular buffer with seqlock for lock-free reads. Uses Apache Arrow IPC format. "Tap" consumers receive every Nth frame via async channel for live visualization without blocking writers.

**SafetyHeartbeat** (`bin/src/safety_heartbeat_task.rs`): Toggles a Comedi DIO channel at 100ms to drive an external hardware interlock. Feature-gated on `hardware`. **HardwareWatchdog** (`common/src/health/watchdog.rs`): Dedicated OS thread fires a 5-step emergency shutdown (close shutters, disable emission, stop motors, zero DAQ outputs) if the Tokio runtime hangs. See [ADR-004](docs/adr/004-panic-safety.md).

**Frame streaming compression** (`protocol/src/compression.rs`): LZ4 compression for camera frame data. Use the buffer-reuse variants (`compress_frame_into`, `decompress_frame_into`) on hot paths — they write into pre-allocated `Vec<u8>` buffers via `std::mem::swap`, eliminating per-frame heap allocations. The server runs a dedicated `std::thread` per stream for compression; the client reuses a decompression buffer in its streaming loop. See [ADR-014](docs/adr/014-frame-streaming-buffer-reuse.md).

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

**Compile-time** (Cargo features in `driver-registry/Cargo.toml`): `serial` (default), `pvcam`/`pvcam_sdk`/`pvcam_hardware`, `comedi`/`hardware`, `andor`/`andor_hardware`, `all_hardware`, `full`. Mock drivers (`driver-mock`) and `driver-universal` are always compiled. Serial/SCPI devices use `driver-universal` TOML manifests (always compiled, no feature flag needed).

**Runtime** (`config/feature_flags.toml`): Loaded via `FeatureFlags::load()`. Toggles: `frame_pool_preallocation`, `async_ring_buffer`, `experimental_streaming`, `debug_frame_timing`, etc.

## Code Style

- Current stable Rust toolchain (edition 2024 workspace members require newer than 1.75), async/await everywhere (Tokio runtime). Never `std::thread::sleep` in async code.
- Error handling: propagate with `?`, add context via `anyhow::Context`. No `.unwrap()` in library code (CI enforces `clippy::unwrap_used`); use `.expect("reason")` for invariants.
- Hardware state: always use `Parameter<T>` with `BoxFuture<'static, Result<()>>` callbacks.
- Workspace clippy: pedantic lints enabled with project-specific allows (see `Cargo.toml` `[workspace.lints.clippy]`).

## Testing Patterns

- **Nextest profiles**: `default` (local, 2 retries), `ci` (3 retries, no fail-fast), `hardware` (single-threaded, 6min timeout), `libs-hardware` (inherits hardware), `coverage` (no retries).
- **Test groups**: `serial-hardware`, `pvcam-hardware`, `andor-hardware`, `elliptec-hardware`, `daemon-e2e` — each max-threads=1 for shared resource serialization.
- **Mock devices**: Always available without feature flags. Use `register_mock_factories(&registry)` for integration tests.
- **Timing tests**: Use `#[tokio::test(start_paused = true)]` with `tokio::time::Instant` for deterministic timing. Wall-clock tests use `TimingTolerance` helpers from `integration-tests/tests/common/`.
- **Hardware gating**: `#[cfg(feature = "hardware_tests")]` + `#[ignore]` for real-device tests.

## Tools & Workflow

**Issue tracking**: This project uses `bdh` (beads). Run `bdh prime` for the authoritative workflow — it is auto-injected at session start by hooks. Run `bdh onboard` to generate agent policy guidance including multi-agent coordination and recording guidelines.

**Code search**: Primary tool is `grepai search "query" --json --compact`. Trace calls with `grepai trace callers/callees "Symbol" --json`. Fall back to `rg`/`grep` if grepai is unavailable.

**Structural search**: `sg` (ast-grep) for AST-aware code patterns. E.g., `sg -p '$EXPR.unwrap()' --lang rust`.

**Quality gates**: `bdh close` runs lightweight check (fmt + ast-grep). `git push` runs full gate (fmt + clippy + tests). `bdh preflight` checks PR readiness.

**LSP**: `rust-analyzer` enabled via `.claude/settings.json`.

## Hardware-in-the-Loop via WASM GUI

Claude Code can directly interact with real DAQ hardware through the WASM GUI in Chrome (via claude-in-chrome MCP). Deploy daemons, then navigate Chrome to the WASM GUI to verify device panels.

| Machine | SSH | Daemon URL | Devices |
|---------|-----|-----------|---------|
| **maitai** | `maitai@100.117.5.12` | `http://100.117.5.12:50051` | 12 (PVCAM, Comedi, ELL14 x3, MaiTai, ESP300, Newport PM) |
| **leabs-dev** | `ssh leabs-dev` | `http://10.0.0.40:50051` | 3 (Andor iStar, IPG YLPP-200, Thorlabs PM400) |

WASM GUI: `http://100.117.5.12:8080`. Known reconnect bug (beefcake-48ad): must reload page to change daemon URL.

## Key Scripts

| Script | Purpose |
|--------|---------|
| `scripts/deploy-maitai.sh` | Full deploy to maitai (pull, clean, build, daemon, GUI) |
| `scripts/deploy-leabs.sh` | Full deploy to leabs-dev (pull, build, daemon, GUI) |
| `scripts/build-maitai.sh` | Full hardware build for maitai machine |
| `scripts/build-lab.sh [--release]` | Build daemon with pvcam_sdk for lab |
| `scripts/demo.sh` | Mock-hardware demo (daemon + GUI/script) |
| `scripts/env-check.sh` | Source before hardware tests |
| `scripts/install-hooks.sh [quick]` | Pre-commit hooks (full or format-only) |
| `scripts/pre-push-gate.sh` | Pre-commit/push quality gate |
| `scripts/install-service.sh` | Install daemon as systemd service |
| `scripts/calibrate-comedi.sh` | Comedi DAQ calibration |
| `scripts/leabs-daemon-watchdog.sh` | Leabs daemon health monitor |
| `scripts/install-target-maintenance.sh` | Install target cleanup cron job |
| `scripts/run-ast-grep.sh` | AST-grep structural search helper |
| `scripts/target-maintenance.sh` | Clean bloated target/ directory |
| `scripts/bd-safe.sh` | Worktree-safe beads commands (auto-discovers Dolt/SQLite backend) |

## References

- Agent policy: `AGENTS.md` (local/gitignored, auto-injected by hooks; generate with `bdh onboard`)
- Testing details: `docs/how-to/testing.md`
- Feature flags: `config/feature_flags.toml`
- Architecture deep-dive: `docs/explanation/architecture.md`
- Hardware setup: `docs/how-to/hardware-setup.md`
- Driver guide: `docs/how-to/hardware-drivers.md`

<!-- BEADHUB:START -->
## BeadHub Coordination Rules

This project uses `bdh` for multi-agent coordination and issue tracking, `bdh` is a wrapper on top of `bd` (beads). Commands starting with : like `bdh :status` are managed by `bdh`. Other commands are sent to `bd`.

You are expected to work and coordinate with a team of agents. ALWAYS prioritize the team vs your particular task.

You will see notifications telling you that other agents have written mails or chat messages, or are waiting for you. NEVER ignore notifications. It is rude towards your fellow agents. Do not be rude.

Your goal is for the team to succeed in the shared project.

The active project policy as well as the expected behaviour associated to your role is shown via `bdh :policy`.

## Start Here (Every Session)

```bash
bdh :policy    # READ CAREFULLY and follow diligently
bdh :status    # who am I? (alias/workspace/role) + team status
bdh ready      # find unblocked work
```

Use `bdh :help` for bdh-specific help.

## Rules

- Always use `bdh` (not `bd`) so work is coordinated
- Default to mail (`bdh :aweb mail list|open|send`) for coordination; use chat (`bdh :aweb chat pending|open|send-and-wait|send-and-leave|history|extend-wait`) when you need a conversation with another agent.
- Respond immediately to WAITING notifications — someone is blocked.
- Notifications are for YOU, the agent, not for the human.
- Don't overwrite the work of other agents without coordinating first.
- ALWAYS check what other agents are working on with bdh :status which will tell you which beads they have claimed and what files they are working on (reservations).
- `bdh` derives your identity from the `.beadhub` file in the current worktree. If you run it from another directory you will be impersonating another agent, do not do that.
- Prioritize good communication — your goal is for the team to succeed

## Using mail

Mail is fire-and-forget — use it for status updates, handoffs, and non-blocking questions.

```bash
bdh :aweb mail send <alias> "message"                         # Send a message
bdh :aweb mail send <alias> "message" --subject "API design"  # With subject
bdh :aweb mail list                                           # Check your inbox
bdh :aweb mail open <alias>                                   # Read & acknowledge
```

## Using chat

Chat sessions are persistent per participant pair. Use `--start-conversation` when initiating a new exchange (longer wait timeout).

**Starting a conversation:**
```bash
bdh :aweb chat send-and-wait <alias> "question" --start-conversation
```

**Replying (when someone is waiting for you):**
```bash
bdh :aweb chat send-and-wait <alias> "response"
```

**Final reply (you don't need their answer):**
```bash
bdh :aweb chat send-and-leave <alias> "thanks, got it"
```

**Other commands:**
```bash
bdh :aweb chat pending          # List conversations with unread messages
bdh :aweb chat open <alias>     # Read unread messages
bdh :aweb chat history <alias>  # Full conversation history
bdh :aweb chat extend-wait <alias> "need more time"  # Ask for patience
```
<!-- BEADHUB:END -->
