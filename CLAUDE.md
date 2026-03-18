# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`AGENTS.md` is the canonical agent policy (local/gitignored, auto-injected by Claude Code hooks). `bdh prime` provides the authoritative beads workflow.

## Build / Test / Lint

```bash
cargo build                              # Default feature set (see Cargo.toml)
cargo nextest run                        # Parallel test runner (install: cargo install cargo-nextest --locked)
cargo nextest run test_name              # Single test by name
cargo nextest run -p common              # Single crate
cargo nextest run -p integration-tests --features universal --profile ci  # Runtime smoke test (CI parity)
cargo nextest run -p integration-tests --no-default-features --features networking,server,scripting,storage_hdf5,storage_arrow,serial,modules,pvcam,universal,db-surreal-rocksdb --profile ci  # Runtime RocksDB smoke (CI parity)
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
  pool             ← Lock-free object pool for zero-allocation frame handling, ForeignView trait, BorrowGuard/BorrowCount, DlPackDescriptor (feature "dlpack")
  protocol         ← Protobuf definitions (daq.proto, experiment.proto, hardware.proto, health.proto, ni_daq.proto, storage.proto)

Hardware Core
  hardware         ← HAL: DeviceRegistry (DashMap-backed), factory orchestration, plugin system
  driver-registry  ← Concrete factory registration, hardware feature gating
  driver-mock      ← Always compiled; used for testing and demos. MockCameraProfile/MockStageProfile (Fast, Realistic, Noisy, Faulty), ScenarioConfig for multi-device test setups
  driver-universal ← TOML-manifest driver for text-protocol devices (the forward path)

Native SDK Drivers (FFI-bound, irreplaceable by manifests)
  driver-pvcam          ← Photometrics cameras (+ pvcam-sys bindgen)
  driver-andor-sdk3     ← Andor cameras (+ andor-sdk3-sys bindgen)
  driver-comedi         ← Linux DAQ cards (+ comedi-sys bindgen)
  driver-dover-motion   ← Dover/Cellino stages (+ dover-motion-sys bindgen)

Engine
  experiment       ← Bluesky-style RunEngine + Plan trait (PlanCommand yields), AcquisitionCoordinator, FeedbackEvent, adaptive scans
  scripting        ← Rhai engine with hardware bindings, optional PyO3
  daq-modules      ← PyMoDAQ/DynExp-style module system with Bluesky lifecycle

Echelle Spectroscopy (in common crate)
  echelle                       ← Profile types, BadPixelMask, calibration schema
  echelle_calibration_pipeline  ← End-to-end: arc frame → wavelength-calibrated profile
  echelle_wavelength_fitting    ← Arc line detection, HgAr atlas, Chebyshev wavelength fits
  echelle_trace_fitting         ← Order trace detection from flat/arc frames
  echelle_rectification         ← Per-order rectification into contiguous buffers
  echelle_optimal_extraction    ← Horne 1986 optimal extraction kernel
  echelle_scattered_light       ← 2D Chebyshev scattered light subtraction
  fits_io [feature = "fits"]    ← FITS file I/O for calibration frame import

Services & Storage
  server           ← gRPC services: Hardware, Scan, RunEngine, Storage, Plugin, etc.
  client           ← gRPC client library
  db               ← SurrealDB control-plane (kv-mem for tests, kv-rocksdb for prod)
  storage          ← RingBuffer (mmap, seqlock), HDF5, Arrow IPC, Parquet, Tiff, Zarr writers, DocumentSink trait, ZarrSink (feature "storage_zarr")

Applications
  bin              ← CLI daemon (mimalloc allocator), reconciler, safety sentinel, safety heartbeat, snapshot + calibrate subcommands
  ui               ← Web-based user interface

Testing
  integration-tests ← Cross-crate integration test suite
```

> **`driver-universal` is the forward path.** New serial/TCP/SCPI devices should be defined as schema v3 TOML manifests in `config/devices/`, not as new driver crates. See `docs/how-to/legacy-scpi-deprecation.md`.

### Key Abstractions

**Capability traits** (`common/src/capabilities.rs`): `Movable`, `Readable`, `FrameProducer`, `Triggerable`, `ExposureControl`, `ShutterControl`, `WavelengthTunable`, `EmissionControl`, `Stageable`, `Settable`, `Switchable`, `Actionable`, `Loggable`, `Parameterized`, `Camera`, `Commandable`, `GatedCamera`, `SpectrometerControl`, `TriggerOnPosition`, `PulseGenerator`, `SafetyInterlock`, `Reconfigurable`, etc. All are `async_trait + Send + Sync`. Devices are defined by what they *do*, not what they *are*. **`CompositeCapability`** orchestrates multi-device operations (e.g., move+trigger+read); **`CapabilityProvider`** is the trait that supplies typed device lookups for composites (implemented by `DeviceRegistry`).

**`DeviceComponents`** (`common/src/driver.rs`): Capability bag returned by `DriverFactory::build()` — one `Option<Arc<dyn Trait>>` per capability. The `DeviceRegistry` stores these and provides typed accessors (`get_movable("stage_1")`).

**`Parameter<T>`** (`common/src/parameter.rs`): Reactive state inspired by QCodes/ScopeFoundry. Wraps `Observable<T>` + hardware callbacks. Flow: `set(value)` → validate constraints → call `hardware_writer` (async BoxFuture) → update internal value (notifies subscribers) → call change listeners. Use `Parameter<T>` for device state, never raw `Arc<Mutex<T>>`.

**`Plan` + `RunEngine`** (`experiment/src/`): Bluesky-inspired. Plans yield `PlanCommand` variants (`MoveTo`, `Read`, `Trigger`, `Wait`, `Checkpoint`, `EmitEvent`, `Set`, `ConditionalBranch`, `WaitSettled`, `RepeatWhile`). RunEngine executes them as a state machine (`Idle → Running → Paused → Aborting`) and emits Bluesky-style documents (`Start`, `Descriptor`, `Event`, `Stop`, `Manifest`). `ConditionalBranch` evaluates an `EvalCondition` (threshold, comparison, expression) and dispatches to then/else command lists. `WaitSettled` blocks until a device reports stable. `RepeatWhile` loops a command body with a safety cap on iterations. `EmitEvent` carries optional `scan_indices: Vec<(String, usize)>` for dimensional scan coordinates (used by `ZarrSink` for chunk placement). **`AcquisitionCoordinator`** (`experiment/src/coordinator.rs`) composes move+trigger+read workflows via `CompositeCapability`. **Feedback system** (`experiment/src/feedback.rs`): `FeedbackEvent` (ThresholdCrossed, StabilityReached, ValueUpdate) feeds adaptive scans; `execute_adaptive()` on RunEngine runs plans with a feedback channel. `FeedbackRouter` (`server/src/grpc/feedback_router.rs`) bridges gRPC streams to the feedback channel.

**`RingBuffer`** (`storage/src/ring_buffer.rs`): mmap-backed circular buffer with seqlock for lock-free reads. Uses Apache Arrow IPC format. "Tap" consumers receive every Nth frame via async channel for live visualization without blocking writers.

**SafetyHeartbeat** (`bin/src/safety_heartbeat_task.rs`): Toggles a Comedi DIO channel at 100ms to drive an external hardware interlock. Feature-gated on `comedi_hardware`. **HardwareWatchdog** (`common/src/health/watchdog.rs`): Dedicated OS thread fires a 5-step emergency shutdown (close shutters, disable emission, stop motors, zero DAQ outputs) if the Tokio runtime hangs. See [ADR-004](docs/adr/004-panic-safety.md).

**Frame streaming compression** (`protocol/src/compression.rs`): LZ4 compression for camera frame data. Use the buffer-reuse variants (`compress_frame_into`, `decompress_frame_into`) on hot paths — they write into pre-allocated `Vec<u8>` buffers via `std::mem::swap`, eliminating per-frame heap allocations. The server runs a dedicated `std::thread` per stream for compression; the client reuses a decompression buffer in its streaming loop. See [ADR-014](docs/adr/014-frame-streaming-buffer-reuse.md).

**Webhook Alerting** (`server/src/alerting.rs`): Sends Slack/Discord-compatible webhook notifications when a device faults, exhausts restart attempts, or the RunEngine aborts a plan. Configured via `[alerting]` in `config/config.v4.toml` or `RUSTDAQ_ALERTING__WEBHOOK_URL` env var. Rate-limited per device key; fire-and-forget via `tokio::spawn`.

**Heartbeat JSONL Log** (`server/src/health/heartbeat_log.rs`): Writes one JSON object per minute to `/tmp/rust_daq_heartbeat.jsonl` with system vitals (CPU%, RSS, disk free, device health, RunEngine state). Designed for post-mortem analysis of overnight run failures.

**Hybrid Persistence** — Three-tier model: TOML (design-time, git-tracked), SurrealDB (runtime control plane, optional), specialized writers (science data: HDF5, Arrow, Zarr). See [ADR-015](docs/adr/015-hybrid-persistence-architecture.md).

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

**Compile-time** (Cargo features in `driver-registry/Cargo.toml`): `serial` (default), `pvcam`/`pvcam_sdk`/`pvcam_hardware`, `comedi`/`comedi_hardware`, `andor`/`andor_hardware`, `all_hardware`, `full`. Mock drivers (`driver-mock`) and `driver-universal` are always compiled. Serial/SCPI devices use `driver-universal` TOML manifests (always compiled, no feature flag needed). Additional per-crate features: `dlpack` (pool — DLPack tensor descriptor for zero-copy NumPy/PyTorch interop), `storage_zarr` (storage — Zarr V3 sink via `DocumentSink`), `metrics` (pool/server — Prometheus counters for stream and document lifecycle). Alerting requires the `alerting` feature on the `server` crate (enabled by default, pulls in `reqwest`). Configured via `[alerting]` in `config/config.v4.toml`; disabled at runtime when `webhook_url` is unset.

**Runtime** (`config/feature_flags.toml`): Loaded via `FeatureFlags::load()`. Toggles: `frame_pool_preallocation`, `async_ring_buffer`, `experimental_streaming`, `debug_frame_timing`, etc.

## Code Style

- Current stable Rust toolchain (edition 2024 workspace members require newer than 1.75), async/await everywhere (Tokio runtime). Never `std::thread::sleep` in async code.
- Error handling: propagate with `?`, add context via `anyhow::Context`. No `.unwrap()` in library code (CI enforces `clippy::unwrap_used`); use `.expect("reason")` for invariants.
- Hardware state: always use `Parameter<T>` with `BoxFuture<'static, Result<()>>` callbacks.
- Workspace clippy: pedantic lints enabled with project-specific allows (see `Cargo.toml` `[workspace.lints.clippy]`).

## Testing Patterns

- **Nextest profiles**: `default` (local, 2 retries), `ci` (3 retries, no fail-fast), `hardware` (single-threaded, 6min timeout), `libs-hardware` (inherits hardware), `coverage` (no retries).
- **Test groups**: `serial-hardware`, `pvcam-hardware`, `andor-hardware`, `elliptec-hardware`, `daemon-e2e` — each max-threads=1 for shared resource serialization.
- **Mock devices**: Always available without feature flags. Use `register_mock_factories(&registry)` for integration tests. `MockCameraProfile`/`MockStageProfile` select fidelity (Fast, Realistic, Noisy, Faulty). `ScenarioConfig` groups multiple mock devices with a shared RNG seed for deterministic multi-device tests.
- **Timing tests**: Use `#[tokio::test(start_paused = true)]` with `tokio::time::Instant` for deterministic timing. Wall-clock tests use `TimingTolerance` helpers from `integration-tests/tests/common/`.
- **Hardware gating**: `#[cfg(feature = "hardware_tests")]` + `#[ignore]` for real-device tests.

## Tools & Workflow

**Issue tracking**: This project uses `bdh` (beads). Run `bdh prime` for the authoritative workflow — it is auto-injected at session start by hooks. Run `bdh onboard` to generate agent policy guidance including multi-agent coordination and recording guidelines.

**Worktree safety**: Always use `bdh ...` as the primary beads entrypoint, including in git worktrees (it handles BeadHub coordination). Only if you must run low-level `bdh` directly (rare) should you use `bash scripts/bd-safe.sh ...` to avoid worktree-local `.beads` drift. Verify local/runtime artifact drift with `bash scripts/beads-worktree-hygiene.sh status`, and use `bash scripts/beads-worktree-hygiene.sh cleanup --apply` to move stale worktree-local `.beads` artifacts.

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

### WASM DOM Interop

The WASM build uses `web-sys` + `wasm-bindgen` for browser API access. Currently enabled features: `Window`, `Document`, `HtmlCanvasElement`. Additional features are verified to compile (tested March 2026):

**Available with additional `web-sys` features** (add to `crates/ui/Cargo.toml` `[target.'cfg(target_arch = "wasm32")'.dependencies]`):

| Feature | API | Use Case |
|---------|-----|----------|
| `Location` | `window().location().search()`, `.set_href()` | Read URL params (`?daemon=http://...`), redirects |
| `Storage` | `window().local_storage()`, `.get_item()`, `.set_item()` | Persist settings across page loads (daemon URL, layout) |
| `UrlSearchParams` | `UrlSearchParams::new_with_str(&search).get(name)` | Parse URL query parameters |
| `HtmlElement` | `element.dyn_into::<HtmlElement>().set_inner_text()` | Modify DOM elements outside canvas |

**Already works with current features** (no Cargo.toml changes needed):
- `document.set_title("DAQ Panel - Connected")` — update browser tab title based on connection status
- `window().set_timeout_with_callback_and_timeout_and_arguments_0()` — already used in `runtime.rs` for async sleep

**Patterns** (all verified to compile for `wasm32-unknown-unknown`):

```rust
// Read URL query param (requires: Location, UrlSearchParams features)
#[cfg(target_arch = "wasm32")]
pub fn get_url_param(name: &str) -> Option<String> {
    let window = web_sys::window()?;
    let search = window.location().search().ok()?;
    let params = web_sys::UrlSearchParams::new_with_str(&search).ok()?;
    params.get(name)
}

// localStorage get/set (requires: Storage feature)
#[cfg(target_arch = "wasm32")]
pub fn local_storage_get(key: &str) -> Option<String> {
    let storage = web_sys::window()?.local_storage().ok()??;
    storage.get_item(key).ok()?
}

// Update browser tab title (works with existing features)
#[cfg(target_arch = "wasm32")]
pub fn set_page_title(title: &str) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        doc.set_title(title);
    }
}
```

**Constraints:**
- Gemini/Trusted Types: not relevant here (applies to third-party sites, not your own WASM app)
- All `web-sys` calls are main-thread only (WASM is single-threaded in browser)
- Use `#[cfg(target_arch = "wasm32")]` guards — these APIs don't exist on native builds
- Use `wasm_bindgen::JsCast` for `.dyn_into::<T>()` downcasts (e.g., `Element` → `HtmlElement`)
- Keep `web-sys` feature list minimal — each feature increases WASM binary size

**Practical applications for rust-daq:**
- **URL-based daemon selection**: `?daemon=http://100.117.5.12:50051` — fixes reconnect bug (beefcake-48ad) by allowing bookmarkable daemon URLs
- **Settings persistence**: Save last daemon URL, panel layout, calibration display preferences to `localStorage`
- **Tab title**: Show "DAQ Panel — Connected (maitai)" or "DAQ Panel — DISCONNECTED" in browser tab

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
| `scripts/repro-istar-stream-crash.sh` | iSTAR stream crash repro harness (grpcurl soak + artifact capture) |
| `scripts/istar-stream-overnight-matrix.sh` | Long-run iSTAR repro matrix over quality/FPS/exposure grids |
| `scripts/leabs-daemon-crash-wrapper.sh` | Remote daemon crash-capture wrapper used by repro/watchdog flows |
| `scripts/install-target-maintenance.sh` | Install target cleanup cron job |
| `scripts/run-ast-grep.sh` | AST-grep structural search helper |
| `scripts/target-maintenance.sh` | Clean bloated target/ directory |
| `scripts/bd-safe.sh` | Worktree-safe beads commands (auto-discovers Dolt/SQLite backend) |
| `scripts/beads-worktree-hygiene.sh` | Detect/clean stale worktree-local beads runtime artifacts |
| `scripts/echelle/overnight-soak.sh` | 12h echelle extraction stability soak (memory, frame drops, latency) |
| `scripts/echelle/analyze-soak-results.py` | Plot and analyze soak test CSV output (memory, latency, PASS/FAIL) |
| `scripts/echelle/validate_vs_pypeit.py` | E2E validation: compare rust-daq extraction vs PypeIt reference |
| `scripts/post-crash-forensics.sh` | Post-crash system forensics (dmesg, coredumps, journal, network) |

### Echelle Calibration CLI

```bash
# Capture a single frame from a running daemon
rust-daq-daemon snapshot <device_id> -o frame.tiff [--exposure-ms 100] [--format tiff|png|raw] [--addr http://host:50051]

# Run offline echelle 3-pass calibration pipeline on arc + flat frames (recommended)
rust-daq-daemon calibrate \
  --frame arc_hgar.tiff \
  --flat flat_dh3p.tiff \
  --config config/calibration/mechelle_5000.toml \
  --output calibrated_profile.toml

# Single-frame calibration (arc only, fewer traces detected)
rust-daq-daemon calibrate --frame arc.tiff --config config/calibration/mechelle_5000.toml --output profile.toml
```

Calibration configs live in `config/calibration/`. The `calibrate` subcommand loads frame(s), builds a `CalibrationPipelineConfig` from the TOML, and executes the 3-pass calibration pipeline:

1. **Pass 1: Echelle equation seed** — atlas matching within 5nm tolerance using physical order estimates
2. **Pass 2: Quadratic regression re-seed** — fit m(i) from successful matches, re-seed failed orders with predicted m
3. **Pass 3: Physics bootstrap** — for uncalibrated orders, use 2D Chebyshev residual surface to predict wavelengths

Output: `EchelleCalibrationProfile` with all 115 orders calibrated (42 arc-matched + 73 bootstrapped), covering 230–844nm.

### Leabs/iSTAR Repro Commands

```bash
bash scripts/leabs-daemon-watchdog.sh --build-remote-on-start   # Health monitor + auto-restart for leabs daemon
bash scripts/repro-istar-stream-crash.sh --build-remote --soak-seconds 1800  # iSTAR crash repro soak + artifact capture
bash scripts/istar-stream-overnight-matrix.sh --hours 10 --batch-size 6       # Overnight iSTAR stream matrix run
```

## Quick Commands

- `/test [crate] [--ci|--hardware|--coverage]` — Run nextest with smart defaults
- `/clippy [crate] [--fix]` — Clippy with CI-parity flags
- `/check [crate] [--wasm|--all]` — Fast cargo check
- `/grind [--max N] [--issue ID]` — Autonomous beads issue loop

## Build Optimization

`.cargo/config.toml` enables:
- `split-debuginfo = "packed"` — faster macOS linking
- `opt-level = 2` for all dependencies in dev mode — faster tests
- Build script optimization (`opt-level = 2` for build-override)

## References

- Agent policy: `AGENTS.md` (local/gitignored, auto-injected by hooks; generate with `bdh onboard`)
- Testing details: `docs/how-to/testing.md`
- Feature flags: `config/feature_flags.toml`
- Architecture deep-dive: `docs/explanation/architecture.md`
- Hardware setup: `docs/how-to/hardware-setup.md`
- Driver guide: `docs/how-to/hardware-drivers.md`
- Echelle calibration config: `config/calibration/mechelle_5000.toml`
- Build config: `.cargo/config.toml`
- Custom commands: `.claude/commands/`

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