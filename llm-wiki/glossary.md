# Glossary

<!--
last-ingested: 2026-04-19
sources:
  - CLAUDE.md
  - docs/explanation/architecture.md
  - docs/explanation/newcomer-guide.md
  - docs/reference/driver-capability-matrix.md
see-also:
  - ./invariants.md
  - ./concepts/
-->

Alphabetical. Terms used in code, docs, and beads. Prefer the definition
here to loose prose.

| Term | Definition |
|------|------------|
| **AGENTS.md** | Canonical agent policy. **Gitignored** (see `.gitignore`) — auto-injected into sessions by tooling, not committed. Referenced by `CLAUDE.md` and `GEMINI.md`. |
| **bd / beads** | Issue tracker for this repo. All task tracking goes through `bd` (not `TodoWrite`, not markdown TODOs). Data in `.beads/` (Dolt-backed). |
| **Bluesky** | NSLS-II's Python-based experiment orchestration framework. Inspiration for `Plan` + `RunEngine` + document stream. |
| **BoxFuture** | `futures::future::BoxFuture<'static, Result<()>>` — the boxed future type used for `Parameter<T>` hardware callbacks. |
| **Capability trait** | Small focused trait describing *what* a device does: `Movable`, `Readable`, `FrameProducer`, etc. See [`concepts/capability-traits.md`](./concepts/capability-traits.md). **30 in total** (authoritative: `crates/common-traits/src/capabilities.rs`). |
| **`Commandable`** | Capability: execute arbitrary vendor-specific commands with JSON args. |
| **CompositeCapability** | Trait that orchestrates multi-device operations (move+trigger+read). Paired with `CapabilityProvider`. |
| **`create_canonical_mock_registry()`** | `driver-registry` function that returns a fully populated mock registry for tests. No feature flag needed. |
| **DAQ** | Data Acquisition. The thing this system is. |
| **Daemon** | The headless gRPC server binary `rust-daq-daemon` produced by the `bin` crate. |
| **`DeviceId`** | `Arc<str>`-backed identity type for a device. Cheap to clone, stable across the run. |
| **`DeviceRegistry`** | Runtime map of `DeviceId → Arc<dyn Capability…>`. Central "phone book" for hardware. Data plane (observed state). See `concepts/driver-registry.md`. |
| **Diataxis** | The four-quadrant doc system (reference / how-to / explanation / tutorials) used under `docs/`. |
| **`DocumentSink`** | Trait in `storage` for consumers of the RunEngine document stream. Impls: HDF5, Arrow, Zarr. |
| **Dolt** | Git-backed SQL database used by `bd` (beads). Pushed via `bd dolt push`. |
| **`DriverFactory`** | Trait in `common-traits` (and re-exported). Each driver crate implements it so `driver-registry` can instantiate drivers from TOML config. |
| **driver-universal** | Crate that loads TOML device manifests (schema v3) and produces drivers for serial/TCP/SCPI instruments without custom Rust code. **Forward path** for new non-SDK devices. |
| **edition 2024** | Rust edition pinned by this workspace. |
| **`FrameProducer`** | Capability for 2D image streaming. Implementations include PVCAM, Andor iStar, MockCamera. |
| **`GatedCamera`** | Capability: camera with DDG + MCP gain control (ICCDs — Andor iStar). |
| **Handoff** | Markdown file under `.claude/handoffs/` summarizing session context for the next agent. |
| **`hardware_tests`** | Cargo feature that enables integration tests requiring real hardware. Paired with `#[ignore]`. Run via nextest `hardware` profile. |
| **HDF5** | Hierarchical Data Format. Primary long-term storage format for experiment data (`hdf5-metno` crate). |
| **leabs-dev** | Hardware machine at `ssh leabs-dev`, daemon at `http://100.109.21.118:50051`. Hosts Andor iStar + IPG YLPP-200 + Thorlabs PM400. See [`hardware/leabs-dev.md`](./hardware/leabs-dev.md). |
| **maitai** | Hardware machine at `maitai@maitai-eos`, daemon at `http://100.117.5.12:50051`. Hosts PVCAM + Comedi + ELL14×3 + MaiTai + ESP300×3 + Newport PM (15 devices). See [`hardware/maitai.md`](./hardware/maitai.md). |
| **Mock registry** | `create_canonical_mock_registry()` — always-on mock drivers for deterministic testing. |
| **Movable** | Capability: `move_abs(position)`, `home()`. Implemented by stages, rotators, piezos. |
| **Mullet Strategy** | Tee pipeline: Arrow ring buffer in the front (low-latency viz), HDF5 in the back (reliable storage). |
| **nextest** | `cargo nextest` — parallel test runner used instead of `cargo test` (which is only kept for doctests). |
| **`Parameter<T>`** | Reactive hardware-state primitive in `common`. Wraps `Observable<T>` over `tokio::sync::watch` + async setter with validation. See [`concepts/parameter.md`](./concepts/parameter.md). |
| **`Plan`** | Generator yielding `PlanCommand`s (`MoveTo`, `Trigger`, `Checkpoint`, `EmitEvent`). Bluesky-style. |
| **`Pool<T>`** | Lock-free object pool in `pool` crate. Zero-allocation frame handling. |
| **Protobuf** | Wire format for gRPC. Defs in `protocol` crate. |
| **PVCAM** | Photometrics Camera API C SDK. Driver crate: `driver-pvcam`. Nested sys crate: `driver-pvcam/pvcam-sys`. |
| **`Readable`** | Capability: `read()` returns a scalar value. Power meters, thermocouples, Comedi AI. |
| **Rhai** | Embedded scripting language for experiment logic. Crate: `scripting`. Sandboxed (10k op limit, timeout). |
| **Ring buffer** | mmap + seqlock Arrow IPC buffer for frame streaming. Path: `/dev/shm/ring.buf`. See [`concepts/ring-buffer.md`](./concepts/ring-buffer.md). |
| **RunEngine** | Executes `Plan`s. State machine (Idle / Running / Paused). Composed of `TaskQueue` + `WatchdogManager`. See [`concepts/plan-run-engine.md`](./concepts/plan-run-engine.md). |
| **Rust 1.92.0** | Toolchain pinned in `rust-toolchain.toml`. Do not bump without bead. |
| **Safety heartbeat** | Layer-1 safety: Tokio task in `crates/bin/src/safety_heartbeat_task.rs` (entry `spawn_heartbeat`) toggles a Comedi DIO channel to drive an external interlock, configured by `HeartbeatConfig` in `[safety_heartbeat]` of the hardware config TOML. Feature: `comedi_hardware`. (Not a Rust type — historical docs that call it `SafetyHeartbeat` are misleading.) |
| **schema v3** | Current TOML schema for `driver-universal` manifests. See [`concepts/driver-universal.md`](./concepts/driver-universal.md). |
| **SCPI** | Standard Commands for Programmable Instruments. Common ASCII command protocol for benchtop instruments, supported by `driver-universal`. |
| **SDK driver** | A `driver-*` crate that binds to a vendor C SDK (PVCAM, Andor SDK3, Comedi, Dover Motion). Each has a paired `*-sys` FFI crate. |
| **Seqlock** | Lock-free concurrency primitive used in the ring buffer. |
| **StartDoc / DescriptorDoc / EventDoc / StopDoc** | Bluesky document types emitted by `RunEngine`. |
| **`Triggerable`** | Capability: arm + trigger. Cameras, pulsed lasers. |
| **Universal driver** | Alias for driver-universal. |
| **universal feature** | Cargo feature gating universal-driver-specific integration tests. |
| **`WavelengthTunable`** | Capability: set emission wavelength. MaiTai, Newport 1830-C, Thorlabs PM400, MockLaser. |
| **Zarr** | Chunked array storage format. Optional storage backend (feature `storage_zarr`). |
