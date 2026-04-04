# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`AGENTS.md` is the canonical agent policy (local/gitignored, auto-injected by Claude Code hooks). `bd prime` provides the authoritative beads workflow.

## Worktree Isolation (MANDATORY for Agents)

**Agents MUST NOT work directly on `main` in the primary checkout.** Always use a worktree or feature branch to avoid destroying concurrent work. Use `isolation: "worktree"` when spawning sub-agents, or `bd worktree create <name>` for manual work. Never squash-merge large changes directly onto main — use feature branches. See AGENTS.md "Worktree Isolation" section for full rules.

## PR Policy

**NEVER push directly to main.** All changes go through feature branches and PRs:

1. **Before writing code**: Create a feature branch (`git checkout -b feat/<issue-id>-description`)
2. **Commit to the branch**, not main
3. **Push branch and create PR**: `git push -u origin feat/... && gh pr create`
4. **Only merge after review** (automated reviewers count: CodeRabbit, Qodo, Copilot)

**The ONLY exception** for direct-to-main: single-file fixes under 20 lines (typos, config tweaks).

**If you accidentally pushed to main**: Create a GitHub issue documenting the commits for post-merge review (see TheFermiSea/rust-daq#505 as an example of what NOT to do).

## Build / Test / Lint

```bash
cargo build                              # Default feature set (see Cargo.toml)
cargo check --workspace --exclude ui     # Fast compile smoke (developer loop)
cargo nextest run                        # Parallel test runner (install: cargo install cargo-nextest --locked)
cargo nextest run test_name              # Single test by name
cargo nextest run -p common              # Single crate
cargo nextest run -p integration-tests --features universal --profile ci  # Runtime smoke test (CI parity)
cargo nextest run -p integration-tests --no-default-features --features networking,server,scripting,storage_hdf5,storage_arrow,serial,modules,pvcam,universal,db-surreal-rocksdb --profile ci  # Runtime RocksDB smoke (CI parity)
cargo nextest run --profile ci           # CI profile (3 retries, no fail-fast)
cargo nextest run --workspace --exclude ui --exclude comedi-sys --exclude driver-comedi --profile ci  # Full CI parity test slice
cargo check -p ui --lib --target wasm32-unknown-unknown --no-default-features --features web  # UI WASM compile smoke (CI parity)
cargo nextest run -p daq-modules test_start_module_auto_stages  # Module lifecycle: start() auto-stages
cargo nextest run -p daq-modules test_stop_module_auto_unstages # Module lifecycle: stop() auto-unstages
cargo nextest run -p driver-pvcam edge_trigger_mock             # PVCAM trigger/timing parameter coverage
cargo run -p driver-pvcam --features pvcam_sdk --example list_pvcam_params  # PVCAM parameter discovery (trigger/timing/scan)
cargo test --doc                         # Doctests (nextest doesn't support these)
cargo fmt --all                          # Format
cargo fmt --all -- --check               # Format check (CI/pre-push parity)
cargo clippy --all-targets               # Lint
cargo clippy --workspace --all-targets --exclude ui --exclude comedi-sys --exclude driver-comedi -- -D warnings  # Clippy gate (CI/pre-push parity)
cargo hack check -p common --feature-powerset --no-dev-deps  # Feature-flag powerset check (single crate)
bash scripts/ci/feature-check.sh                # Feature powerset check (all key crates)
bash scripts/ci/feature-check.sh common --quick # Single crate, each-feature mode (fast)
cargo watch -x 'check -p common'             # File-watching auto-check (dev loop)
cargo expand --package common parameter      # Macro expansion debugging
cargo flamegraph --bin rust-daq-daemon       # CPU flamegraph (requires dtrace on macOS)
cargo bloat --release -p ui --target wasm32-unknown-unknown  # WASM binary size breakdown
cargo deny check                             # License/advisory/ban audit
cargo machete                                # Find unused dependencies
bash scripts/ops/fast-check.sh               # Quick smoke (check + nextest + doctests, excludes UI)
bash scripts/generate-feature-matrix.sh --check  # Detect feature-doc drift from cargo metadata
```

### Maitai Hardware Build (Critical)

Always use `bash scripts/ops/build-maitai.sh` for real hardware. Building without `--features maitai` silently selects mock PVCAM paths. Verify: daemon log shows `pvcam_sdk feature enabled: true` and registers expected physical devices.

### Hardware Tests (maitai only)

```bash
source scripts/ops/env-check.sh && cargo nextest run --profile hardware --features hardware_tests
```

## Architecture

### Crate Dependency Layers (27 workspace crates)

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
  daq-modules      ← PyMoDAQ/DynExp-style module system with Bluesky lifecycle, capability validation, and auto-stage/auto-unstage lifecycle enforcement

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
  ui               ← Web-based user interface (egui/eframe + WASM)
  ui-slint         ← [EXPERIMENTAL] Slint evaluation UI (native + WASM)

Testing
  integration-tests ← Cross-crate integration test suite
```

> **`driver-universal` is the forward path.** New serial/TCP/SCPI devices should be defined as schema v3 TOML manifests in `config/devices/`, not as new driver crates. See `docs/how-to/legacy-scpi-deprecation.md`.

### Key Abstractions

**Capability traits** (`common/src/capabilities.rs`): `Movable`, `Readable`, `FrameProducer`, `Triggerable`, `ExposureControl`, `ShutterControl`, `WavelengthTunable`, `EmissionControl`, `Stageable`, `Settable`, `Switchable`, `Actionable`, `Loggable`, `Parameterized`, `Camera`, `Commandable`, `GatedCamera`, `SpectrometerControl`, `SpectrumReadable`, `TriggerOnPosition`, `PulseGenerator`, `SafetyInterlock`, `Reconfigurable`, etc. All are `async_trait + Send + Sync`. Devices are defined by what they *do*, not what they *are*. **`SpectrumReadable`** + **`SpectrumData`** (bd-lncj) provide the 1D detector abstraction for spectrometers and line detectors — `read_spectrum()` returns wavelength/intensity arrays with units. **`CompositeCapability`** orchestrates multi-device operations (e.g., move+trigger+read); **`CapabilityProvider`** is the trait that supplies typed device lookups for composites (implemented by `DeviceRegistry`).

**`DeviceComponents`** (`common/src/driver.rs`): Capability bag returned by `DriverFactory::build()` — one `Option<Arc<dyn Trait>>` per capability. The `DeviceRegistry` stores these and provides typed accessors (`get_movable("stage_1")`).

**`Parameter<T>`** (`common/src/parameter.rs`): Reactive state inspired by QCodes/ScopeFoundry. Wraps `Observable<T>` + hardware callbacks. Flow: `set(value)` → validate constraints → call `hardware_writer` (async BoxFuture) → update internal value (notifies subscribers) → call change listeners. Use `Parameter<T>` for device state, never raw `Arc<Mutex<T>>`.

**`Plan` + `RunEngine`** (`experiment/src/`): Bluesky-inspired. Plans yield `PlanCommand` variants (`MoveTo`, `Read`, `Trigger`, `Wait`, `Checkpoint`, `EmitEvent`, `Set`, `ConditionalBranch`, `WaitSettled`, `RepeatWhile`). RunEngine executes them as a state machine (`Idle → Running → Paused → Aborting`) and emits Bluesky-style documents (`Start`, `Descriptor`, `Event`, `Stop`, `Manifest`). State is push-based: `subscribe_state()` returns a `broadcast::Receiver<EngineState>` for reactive UIs, and the server exposes a `StreamEngineStatus` streaming RPC (client: `stream_engine_status()`). `ConditionalBranch` evaluates an `EvalCondition` (threshold, comparison, expression) and dispatches to then/else command lists. `WaitSettled` blocks until a device reports stable. `RepeatWhile` loops a command body with a safety cap on iterations. `EmitEvent` carries optional `scan_indices: Vec<(String, usize)>` for dimensional scan coordinates (used by `ZarrSink` for chunk placement). Command dispatch now fails fast when devices are missing required capabilities (Move/Read/Trigger/Set), instead of silently skipping device actions. **Frame metadata pipeline**: `Frame.metadata` (hardware timestamps, bit_depth, roi_count, etc.) flows through RunEngine `Event` documents into HDF5 storage via `ExperimentFrameObserver`. **`AcquisitionCoordinator`** (`experiment/src/coordinator.rs`) composes move+trigger+read workflows via `CompositeCapability`. **Feedback system** (`experiment/src/feedback.rs`): `FeedbackEvent` (ThresholdCrossed, StabilityReached, ValueUpdate) feeds adaptive scans; `execute_adaptive()` on RunEngine runs plans with a feedback channel. `FeedbackRouter` (`server/src/grpc/feedback_router.rs`) bridges gRPC streams to the feedback channel.

**PVCAM trigger/timing parameters** (`driver-pvcam/src/lib.rs`, `driver-pvcam/src/components/features/mod.rs`): Exposes trigger controls for external synchronization through typed parameters: `trigger.expose_out_mode`, `trigger.edge_trigger`, `trigger.pre_delay_us`, and `trigger.post_delay_us`. The SDK-backed implementation maps these through `PARAM_EXPOSE_OUT_MODE`, `PARAM_EDGE_TRIGGER`, `PARAM_PRE_TRIGGER_DELAY`, and `PARAM_POST_TRIGGER_DELAY`; mock state mirrors the same behavior for tests.

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
- Non-trivial TODO/FIXME comments should include a beads reference (e.g., `TODO(bd-xxxx)`), matching repository policy enforcement.
- **Fix pre-existing issues**: Do NOT dismiss test failures, warnings, or broken code as "pre-existing" and move on. If you encounter a failing test or warning in a file you're working in (or that blocks your verification), fix it immediately. If the fix is non-trivial, create a bead for it — but never leave the codebase in a worse or equally broken state. "It was already broken" is not an acceptable reason to ship broken code.

## Testing Patterns

- **Nextest profiles**: `default` (local, 2 retries), `ci` (3 retries, no fail-fast), `hardware` (single-threaded, 6min timeout), `libs-hardware` (inherits hardware), `coverage` (no retries).
- **Test groups**: `serial-hardware`, `pvcam-hardware`, `andor-hardware`, `elliptec-hardware`, `daemon-e2e` — each max-threads=1 for shared resource serialization.
- **Mock devices**: Always available without feature flags. Use `driver_registry::create_canonical_mock_registry()` for new tests — it registers all mock + universal-manifest factories with correct config paths. (`hardware::registry::create_mock_registry()` is deprecated.) `MockCameraProfile`/`MockStageProfile` select fidelity (Fast, Realistic, Noisy, Faulty). `ScenarioConfig` groups multiple mock devices with a shared RNG seed for deterministic multi-device tests.
- **Timing tests**: Use `#[tokio::test(start_paused = true)]` with `tokio::time::Instant` for deterministic timing. Wall-clock tests use `TimingTolerance` helpers from `integration-tests/tests/common/`.
- **Hardware gating**: `#[cfg(feature = "hardware_tests")]` + `#[ignore]` for real-device tests.

## Tools & Workflow

**Issue tracking**: `bd` (beads) — Run `bd prime` for workflow context (auto-injected at session start). If a worktree cannot resolve the canonical beads DB, use `bash scripts/bd-safe.sh ...` (including `ready`, `where`, and `memories` lookups). If `bd dolt push` reports missing remote `origin`, run `bash scripts/ops/setup-beads-dolt-remote.sh`.

**Advanced features:**
- `bd query "status=open AND priority<=1"` — compound query language
- `bd graph <epic> --html > deps.html` — interactive dependency visualization
- `bd swarm create/validate/status <epic>` — parallel agent work coordination
- `bd mol pour <formula> --var key=value` — instantiate workflow templates
- `bd gate list/check/resolve` — async coordination (human, timer, CI, PR gates)
- `bd merge-slot acquire/release` — exclusive access for merge queue
- `bd agent state <id> working` — agent lifecycle reporting
- `bd slot set <agent> hook <bead>` — attach work to agent
- `bd worktree create/remove <name>` — managed worktrees with beads redirect
- `bd defer <id> --until="next monday"` — temporal issue management
- `bd todo add "quick task"` — lightweight task capture
- `bd github sync` — two-way GitHub issue sync
- `bd find-duplicates --method ai` — AI-powered duplicate detection
- `bd gc --older-than 90` — lifecycle garbage collection
- `bd preflight --check` — pre-PR readiness checklist
- `bd sql "SELECT ..."` — raw SQL queries on issue database

**Worktree safety**: Use `bd worktree create/remove` for managed worktrees. Fallback: `bash scripts/bd-safe.sh ...`.

**Code search**: Primary tool is `colgrep`. Use semantic search first (`colgrep "<query>" -k 25`) and narrow with include/exclude patterns as needed. Fall back to `rg` for exact text matches or when `colgrep` is unavailable.

**Structural search**: `sg` (ast-grep) for AST-aware code patterns. E.g., `sg -p '$EXPR.unwrap()' --lang rust`.

**Quality gates**: `bd close` triggers hook checks (`validate-epic-close` + `quality-gate-on-close`: fmt check + ast-grep error scan). `git push` triggers `.claude/hooks/pre-push-checks.sh` (fmt + clippy + tests, excluding `ui` and `integration-tests`, nextest `--profile ci` when available). `bd preflight --check` for PR readiness.

**Hook dispatch**: `.claude/hooks/pretool-dispatch.sh` routes `bd close` and `git push` to the relevant checks, and blocks `git worktree remove` unless the command starts with an explicit `cd` to a safe directory.

**LSP**: `rust-analyzer` enabled via `.claude/settings.json`.

## Hardware-in-the-Loop via WASM GUI

Claude Code can directly interact with real DAQ hardware through the WASM GUI in Chrome (via claude-in-chrome MCP). Deploy daemons, then navigate Chrome to the WASM GUI to verify device panels.

| Machine | SSH | Daemon URL | Devices |
|---------|-----|-----------|---------|
| **maitai** | `maitai@100.117.5.12` | `http://100.117.5.12:50051` | 12 (PVCAM, Comedi, ELL14 x3, MaiTai, ESP300, Newport PM) |
| **leabs-dev** | `ssh leabs-dev` | `http://100.109.21.118:50051` | 3 (Andor iStar, IPG YLPP-200, Thorlabs PM400) |

WASM GUI: `http://100.117.5.12:8080` (maitai) or `http://100.109.21.118:8080` (leabs-dev, requires `--wasm-gui` deploy flag). Known reconnect bug (bd-0zu5): must reload page to change daemon URL.

**WASM GUI build**: `trunk` (external CLI tool, not a Cargo dependency) is required. `deploy-leabs.sh --wasm-gui` installs a pre-built `trunk` binary to `/usr/local/bin` when missing. To build manually: `cd crates/ui && trunk build --release`, then serve `dist/` with `python3 -m http.server 8080`.

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
- **URL-based daemon selection**: `?daemon=http://100.117.5.12:50051` — fixes reconnect bug (bd-0zu5) by allowing bookmarkable daemon URLs
- **Settings persistence**: Save last daemon URL, panel layout, calibration display preferences to `localStorage`
- **Tab title**: Show "DAQ Panel — Connected (maitai)" or "DAQ Panel — DISCONNECTED" in browser tab

## Key Scripts

| Script | Purpose |
|--------|---------|
| **deploy/** | |
| `scripts/deploy/deploy.sh` | Preferred unified deploy entry point (`--target maitai|leabs-dev`); wrapper scripts delegate here |
| `scripts/deploy/deploy-maitai.sh` | Full deploy to maitai (pull, clean, build, daemon, GUI) |
| `scripts/deploy/deploy-leabs.sh` | Full deploy to leabs-dev (remote checkout+pull+build, daemon restart, optional GUI). Use `--wasm-gui` to build+serve WASM GUI on leabs-dev:8080 |
| `scripts/deploy/install-service.sh` | Install daemon as systemd service |
| **ops/** | |
| `scripts/ops/build-maitai.sh` | Full hardware build for maitai machine |
| `scripts/ops/build-lab.sh [--release]` | Build daemon with pvcam_sdk for lab |
| `scripts/ops/demo.sh` | Mock-hardware demo (daemon + GUI/script) |
| `scripts/ops/env-check.sh` | Source before hardware tests |
| `scripts/ops/install-hooks.sh [quick]` | Pre-commit hooks (full or format-only) |
| `scripts/ops/calibrate-comedi.sh` | Comedi DAQ calibration |
| `scripts/ops/fast-check.sh` | Fast local smoke loop (cargo check + nextest + doctests), not a replacement for pre-push gates |
| `scripts/ops/setup-beads-dolt-remote.sh` | Configure beads Dolt `origin` remote when sync/push fails |
| `scripts/ops/post-crash-forensics.sh` | Post-crash system forensics (dmesg, coredumps, journal, network) |
| **ci/** | |
| `scripts/ci/pre-push-gate.sh` | Pre-push quality gate (fmt, optional mdBook build, clippy, tests) |
| `scripts/ci/feature-check.sh` | cargo-hack feature powerset check on key crates (local CI parity) |
| `scripts/ci/run-ast-grep.sh` | AST-grep structural search helper |
| **hygiene/** | |
| `scripts/hygiene/target-maintenance.sh` | Clean bloated target/ directory |
| `scripts/hygiene/install-target-maintenance.sh` | Install target cleanup cron job |
| `scripts/hygiene/beads-worktree-hygiene.sh` | Detect/clean stale worktree-local beads runtime artifacts |
| `scripts/hygiene/check-doc-drift.sh` | Detect documentation drift from code |
| `scripts/hygiene/check-inventory-drift.sh` | Detect inventory drift |
| `scripts/hygiene/check-dependency-hygiene.sh` | Dependency audit (cargo-audit, cargo-deny, cargo-machete) |
| `scripts/hygiene/cleanup-report.sh` | Recurring cleanup report (oversized files, doc drift, TODO/FIXME/HACK counts, deprecated items) |
| **repro/** | |
| `scripts/repro/leabs-daemon-watchdog.sh` | Leabs daemon health monitor |
| `scripts/repro/repro-istar-stream-crash.sh` | iSTAR stream crash repro harness (grpcurl soak + artifact capture) |
| `scripts/repro/istar-stream-overnight-matrix.sh` | Long-run iSTAR repro matrix over quality/FPS/exposure grids |
| `scripts/repro/leabs-daemon-crash-wrapper.sh` | Remote daemon crash-capture wrapper used by repro/watchdog flows |
| **echelle/** | |
| `scripts/echelle/overnight-soak.sh` | 12h echelle extraction stability soak (memory, frame drops, latency) |
| `scripts/echelle/analyze-soak-results.py` | Plot and analyze soak test CSV output (memory, latency, PASS/FAIL) |
| `scripts/echelle/validate_vs_pypeit.py` | E2E validation: compare rust-daq extraction vs PypeIt reference |
| **root** | |
| `scripts/bd-safe.sh` | Worktree-safe beads commands (auto-discovers Dolt/SQLite backend) |
| `scripts/generate-feature-matrix.sh` | Generate/check feature matrix from Cargo metadata (`--check`, `--output`) |

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
bash scripts/repro/leabs-daemon-watchdog.sh --build-remote-on-start   # Health monitor + auto-restart for leabs daemon
bash scripts/repro/repro-istar-stream-crash.sh --build-remote --soak-seconds 1800  # iSTAR crash repro soak + artifact capture
bash scripts/repro/istar-stream-overnight-matrix.sh --hours 10 --batch-size 6       # Overnight iSTAR stream matrix run
```

## Quick Commands

- `/rust-check` — CI-style Rust gate (`cargo fmt --all -- --check`, workspace clippy gate, nextest `--profile ci`)
- `/security-audit` — Structural audit via `bash scripts/ci/run-ast-grep.sh`
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

- Agent policy: `AGENTS.md` (local/gitignored, auto-injected by hooks; generate with `bd onboard`)
- Testing details: `docs/how-to/testing.md`
- Feature flags: `config/feature_flags.toml`
- Architecture deep-dive: `docs/explanation/architecture.md`
- Hardware setup: `docs/how-to/hardware-setup.md`
- Driver guide: `docs/how-to/hardware-drivers.md`
- Echelle calibration config: `config/calibration/mechelle_5000.toml`
- Build config: `.cargo/config.toml`
- Prompt bundles: `.prompts/`

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:b9766037 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

## Landing the Plane (Session Completion)

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   bd dolt push   # if remote 'origin' missing: bash scripts/ops/setup-beads-dolt-remote.sh
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds
<!-- END BEADS INTEGRATION -->
