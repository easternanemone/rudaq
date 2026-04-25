# Rust DAQ System Architecture

## Overview

`rust-daq` is a modular, high-performance data acquisition system built in Rust. It is designed for scientific experiments requiring low-latency hardware control, high-throughput data streaming, and crash-resilient operation.

The architecture follows a **Headless-First** design: the core daemon runs as a robust, autonomous process that owns the hardware, while the user interface runs as a separate, lightweight client. This ensures that a GUI crash never interrupts a running experiment.

## Core Design Principles

1.  **Crash Resilience:** Strict separation between the daemon and the client UIs (`egui` native/WASM, plus experimental `ui-slint`).
2.  **Capability-Based Hardware:** Drivers are composed of atomic traits (`Movable`, `Triggerable`) rather than monolithic inheritance.
3.  **Hot-Swappable Logic:** Experiments are defined in **Rhai** scripts, allowing logic changes without recompiling the daemon.
4.  **Zero-Copy Data Path:** High-speed data flows through a memory-mapped ring buffer (Arrow IPC) for visualization and storage.

---

## System Components

The project is structured as a Cargo workspace with 30 crates organized by layer:

### 1. Application Layer
*   **`bin`**: The entry point for the daemon (`rust-daq-daemon`). Wires together the system based on compile-time features.
*   **`ui`**: The main `egui` client crate. Produces the native desktop GUI (`rust-daq-gui`) and the browser/WASM GUI (`rust-daq-web`). Connects to the daemon via gRPC or gRPC-web depending on platform.
*   **`ui-slint`**: Experimental Slint-based UI workspace member used for evaluation and benchmarks.
*   **`client`**: gRPC client library for connecting to the daemon. Provides a typed API for remote hardware control, streaming, and device management.

### 2. Domain Logic
*   **`experiment`**: The orchestration engine ("RunEngine"). Executes declarative plans and manages the experiment state machine. Includes `AcquisitionCoordinator` for multi-device workflows, `FeedbackEvent` channel for adaptive scans, and `execute_adaptive()` for feedback-driven plan execution.
*   **`scripting`**: Embeds the **Rhai** scripting engine. Provides a safe sandbox for user scripts to control hardware (10k operation limit, timeout protection). Optional Python bindings via PyO3.
*   **`server`**: The network interface. Implements a gRPC server (`tonic`) exposing hardware control, script execution, and data streaming. Includes token-based authentication and CORS configuration.
*   **`daq-modules`**: Experiment modules and plugin system. Provides a modular framework for composing experiment workflows with runtime module assignment.

### 3. Hardware Abstraction
*   **`hardware`**: The Hardware Abstraction Layer (HAL). Defines capability traits, `DeviceRegistry`, and config/schema loading. Concrete driver feature selection now lives in `driver-registry`.

### 4. Driver Crates (Standalone)

Each driver lives in its own crate for independent compilation, testing, and optional inclusion:

#### Registry & Feature Gating
*   **`driver-registry`**: Concrete factory registration and hardware feature gating. Always registers `driver-mock` and `driver-universal`; optionally includes `driver-pvcam`, `driver-andor-sdk3`, `driver-comedi` behind feature flags (pvcam, andor, comedi, all_hardware, full).

#### Camera Drivers (Native SDK)
*   **`driver-pvcam`**: Photometrics PVCAM cameras (Prime 95B, Prime BSI). Requires PVCAM SDK.
*   **`driver-andor-sdk3`**: Andor iStar camera and Shamrock spectrograph via SDK3.

#### Motion Control (Native SDK)
*   **`driver-dover-motion`**: Dover Motion SmartStage driver via MotionSynergyAPI FFI.

#### DAQ & Signal (Native SDK)
*   **`driver-comedi`**: Linux Comedi DAQ boards (NI PCI-MIO-16XE-10). Analog/digital I/O.

#### Manifest-Driven & Testing
*   **`driver-universal`**: Universal config-driven driver (schema **v4**, see [ADR-018](../adr/018-v4-manifest-first-universal-driver-ui.md)). Define new instruments via TOML files without writing Rust code. Supports serial, TCP, and SCPI transports with MiniJinja templates, format-string response variants, declarative transforms (`map`/`match_one_of`/`format`/`regex_extract`), and a curated evalexpr palette. Regex parsing is demoted to `[responses.X.advanced]` with a deprecation warning. Inline `[commands.X.ui]` / `[parameters.X.ui]` tables let authors declare widget and layout hints next to the command they control; a synthesizer turns these into the `ControlPanelConfig` consumed by `ConfigDrivenPanel` so no hand-written Rust panel is needed for universal-driver-eligible devices. Tooling: `manifest-check`, `export-manifest-schema`, `migrate-v3`, `manifest-wizard`. Always compiled. Devices in `config/devices/`: ELL14 rotators, ESP300/ESP301 stages, Newport 1830-C power meter, MaiTai laser, Red Pitaya PID, IPG laser, Thorlabs PM400, a Siglent SDG1025 function generator, a Modbus example, and a walkthrough `tutorial_device_example.toml`. See [`docs/how-to/write-a-device-manifest.md`](../how-to/write-a-device-manifest.md).
*   **`driver-mock`**: Mock hardware drivers for testing, simulation, and demo mode. Always compiled. `MockCameraProfile`/`MockStageProfile` select fidelity levels (Fast, Realistic, Noisy, Faulty). `ScenarioConfig` groups devices with a shared RNG seed for deterministic multi-device tests.

### 5. Infrastructure
*   **`pool`**: Zero-allocation object pool for high-performance frame handling. Provides `Pool<T>` for generic objects and `BufferPool` for byte buffers with `bytes::Bytes` integration. Critical for high-FPS camera streaming where per-frame allocations cause latency. `ForeignView` trait enables zero-copy access from Python/C++/GPU consumers. `BorrowGuard`/`BorrowCount` prevent slot reclamation while foreign code holds references. `DlPackDescriptor` (feature `dlpack`) provides tensor metadata for NumPy/PyTorch interop.
*   **`storage`**: Handles data persistence. Implements the "Mullet Strategy": fast **Arrow IPC** ring buffer in the front, reliable **HDF5** writer in the back. Also supports **Parquet**, **Tiff**, and **Zarr** formats. The `DocumentSink` trait decouples document production (RunEngine) from consumption -- implementations include HDF5, Arrow, and `ZarrSink` (feature `storage_zarr`) which maps `scan_indices` to Zarr chunk coordinates.
*   **`protocol`**: Defines the wire protocol (Protobuf) for all network communication. Includes domain↔proto conversion utilities.
*   **`db`**: Embedded SQLite persistence layer for the control plane — primary and only backend (bd-2a2ne). Uses `rusqlite` (bundled) with `tokio-rusqlite`. Manages device/experiment metadata with a two-plane model: SQLite as control plane (desired state), DashMap `DeviceRegistry` as data plane (observed state). Earlier revisions documented an optional SurrealDB variant; that backend has been removed and no `db-surreal` feature exists in the current workspace.

### 6. Core
*   **`common`**: The foundation. Defines shared types (`Parameter<T>`, `Observable<T>`), error handling, size limits (`limits.rs`), and module domain types.

### 7. FFI Bindings
*   **`pvcam-sys`**: Raw FFI bindings to the PVCAM C library (nested under `driver-pvcam`).
*   **`comedi-sys`**: Raw FFI bindings to the Linux Comedi library.
*   **`andor-sdk3-sys`**: Raw FFI bindings to the Andor SDK3 C library.
*   **`dover-motion-sys`**: Raw FFI bindings to the Dover MotionSynergyAPI C library.

### 8. Testing
*   **`integration-tests`**: Workspace-level integration test suite covering cross-crate workflows.

---

## Architectural Diagrams

### High-Level Topology

```mermaid
graph TD
    subgraph "Host Machine"
        subgraph "Daemon Process (rust-daq-daemon)"
            Server[gRPC Server]
            Script[Rhai Engine]
            Modules[DAQ Modules]
            HW[Hardware Manager]
            Ring[Ring Buffer / Arrow]
            Writer[HDF5 Writer]
        end

        subgraph "Client Process (rust-daq-gui / rust-daq-web)"
            GUI[egui Interface]
            Dock[Docking System]
            Plot[Real-time Plots]
        end

        subgraph "Driver Layer"
            DrvRegistry[driver-registry]
            DrvPvcam[driver-pvcam]
            DrvAndor[driver-andor-sdk3]
            DrvDover[driver-dover-motion]
            DrvComedi[driver-comedi]
            DrvUniversal[driver-universal]
            DrvMock[driver-mock]
        end
    end

    GUI <-->|gRPC / HTTP2| Server

    Server --> Script
    Server --> Modules
    Script --> HW
    HW --> DrvRegistry
    DrvRegistry --> DrvPvcam
    DrvRegistry --> DrvAndor
    DrvRegistry --> DrvComedi
    DrvDover -.->|experimental, not registry-wired| DrvRegistry
    DrvRegistry --> DrvUniversal
    DrvRegistry --> DrvMock
    HW -->|Frame Data| Ring
    Ring -->|Zero-Copy| Writer
    Ring -.->|Stream| Server
```

### Data Pipeline (The "Mullet Strategy")

To resolve the conflict between high-throughput reliable storage and low-latency live visualization, the system implements a **Tee-based Pipeline**:

1.  **Source:** Hardware drivers produce data (e.g., `Arc<Frame>`).
2.  **Ring Buffer:** Data is written to a lock-free, memory-mapped Ring Buffer using Apache Arrow IPC format.
3.  **Storage Path:** A dedicated background thread reads from the Ring Buffer and writes to HDF5 files.
4.  **Live Stream:** The `DaqServer` subscribes to the stream and broadcasts it via gRPC to the GUI.

---

## Key Features

### Hardware Abstraction
Hardware is modeled by **Capabilities**, not identities. A device is defined by what it can *do*:
*   `Movable`: Can move to a position (e.g., Motors, Piezo stages, rotators).
*   `Triggerable`: Can accept a start signal (e.g., Cameras).
*   `Readable`: Can return a scalar value (e.g., Sensors, power meters).
*   `FrameProducer`: Can stream 2D image data (e.g., Detectors, cameras).
*   `FrameObserver`: Can consume frame data from producers.
*   `ExposureControl`: Can set integration time.
*   `WavelengthTunable`: Can tune wavelength (e.g., Lasers, monochromators).
*   `ShutterControl`: Can open/close a beam shutter.
*   `EmissionControl`: Can enable/disable laser emission.
*   `Stageable`: Multi-axis stage with position reporting.
*   `Settable`: Generic settable parameter (scalar or complex).
*   `Switchable`: Binary on/off control.
*   `Actionable`: Single-shot action trigger.
*   `Loggable`: Periodic value logging capability.
*   `Parameterized`: Exposes reactive `Parameter<T>` state for observation and persistence.
*   `Camera`: Combined camera capabilities (FrameProducer + ExposureControl + Triggerable).
*   `GatedCamera`: Camera with external gating control.
*   `Commandable`: Direct command-response protocol.
*   `SpectrometerControl`: Wavelength control for spectrometers.
*   `TriggerOnPosition`: Positional triggering for synchronized motion.
*   `SafetyInterlock`: Safety monitoring and interlock control.
*   `Reconfigurable`: Runtime reconfiguration of device settings.
*   `StateRefreshable`: Re-read all parameters from hardware after a reconnect. Implemented by all `driver-universal` devices (bd-47p2).
*   `CounterConfigurable`: Configure pulse / edge counters (Comedi counter channels).
*   `RangeIntrospectable`: Report valid-range metadata for GUI slider bounds.
*   `DeviceIntrospection`: Report device metadata (serial, firmware, model).
*   `ReadableWithMetadata`: `Readable` extension that returns value + timestamp + units.
*   `SpectrumReadable`: Return a 1D spectrum array (spectrometers, wavemeters).
*   `CompositeCapability`: Orchestrates multi-device operations (e.g., move+trigger+read). Paired with `CapabilityProvider` for typed device lookups.

Authoritative trait list lives in `crates/common-traits/src/capabilities.rs` (30 traits as of 2026-04). Re-exports in `crates/common`.

This allows generic experiment scripts to work with any compatible hardware (e.g., `scan(movable, triggerable)`).

### Safety Architecture

The system uses a 3-layer safety stack to protect against hardware being left in dangerous states:

*   **Layer 1: Safety heartbeat task** (proactive) — A Tokio task (`crates/bin/src/safety_heartbeat_task.rs`, entry `spawn_heartbeat`) toggles a Comedi DIO channel to drive an external hardware interlock, driven by a `HeartbeatConfig` (`crates/hardware/src/registry/types.rs`) loaded from the `[safety_heartbeat]` stanza of the hardware config TOML (e.g. `config/maitai_universal.toml`). If the daemon process dies for any reason (crash, SIGKILL, power loss), the pulse stops and the external circuitry cuts laser power. Feature-gated on `comedi_hardware`.
*   **Layer 2: HardwareWatchdog** (reactive) — A dedicated OS thread monitors daemon liveness. If the Tokio runtime hangs or deadlocks (no kick received within 30s), it fires a 5-step emergency shutdown: close shutters, disable emission, stop motors, zero DAQ outputs.
*   **Layer 3: Panic hook** — On Rust panics, the same 5-step emergency shutdown sequence runs from the panic handler, using bridge threads and a pre-allocated emergency runtime to execute async hardware calls.

See [ADR-004](../adr/004-panic-safety.md) for the full defense-in-depth design.

### Reactive Parameters
All hardware state is managed via `Parameter<T>`. This provides:
*   **Observability:** Changes are broadcast to all subscribers (GUI, Scripts).
*   **Validation:** Setters can reject invalid values.
*   **Persistence:** Parameter values are snapshotted to HDF5.

### Scripting (Rhai)
Experiments are written in [Rhai](https://rhai.rs), a scripting language designed for Rust.
*   **Safety:** Scripts run in a sandbox with operation limits to prevent infinite loops.
*   **Integration:** Rust async functions are exposed as synchronous Rhai functions (e.g., `stage.move_abs(10.0)`).
*   **Hot-Swap:** Scripts are uploaded via gRPC and executed immediately.

---

## Directory Structure

```
.
├── crates/
│   ├── bin/                  # Application entry points (daemon, CLI)
│   ├── client/               # gRPC client library
│   ├── common/               # Foundation types, errors, parameters, limits
│   ├── daq-modules/          # Experiment modules and plugin system
│   ├── driver-andor-sdk3/    # Andor iStar camera / Shamrock spectrograph
│   ├── driver-comedi/        # Comedi DAQ driver for Linux boards
│   ├── driver-dover-motion/  # Dover Motion SmartStage driver
│   ├── driver-universal/     # Universal config-driven driver (schema v3)
│   ├── driver-mock/          # Mock hardware for testing/demo
│   ├── driver-pvcam/         # PVCAM camera driver
│   │   └── pvcam-sys/        # Raw FFI bindings to PVCAM
│   ├── driver-registry/      # Factory registration and hardware feature gating
│   ├── andor-sdk3-sys/       # Raw FFI bindings to Andor SDK3
│   ├── comedi-sys/           # Raw FFI bindings to Comedi
│   ├── dover-motion-sys/     # Raw FFI bindings to Dover MotionSynergyAPI
│   ├── db/                   # SQLite control-plane database
│   ├── experiment/           # RunEngine and Plan definitions
│   ├── hardware/             # HAL with capability traits and DeviceRegistry
│   ├── integration-tests/    # Cross-crate integration tests
│   ├── pool/                 # Zero-allocation object pool for frame handling
│   ├── protocol/             # Protobuf definitions and conversions
│   ├── scripting/            # Rhai scripting engine integration
│   ├── server/               # gRPC server implementation
│   ├── storage/              # Ring buffers, HDF5, Arrow, Parquet, TIFF, Zarr storage
│   ├── ui/                   # egui GUI crate (native + WASM)
│   └── ui-slint/             # experimental Slint UI crate
├── config/                   # Runtime configuration (TOML)
│   ├── devices/              # Declarative driver configs (TOML manifest files)
│   │   ├── ell14.toml        # Thorlabs ELL14 rotation mounts
│   │   ├── esp300.toml       # Newport ESP300 motion controller
│   │   ├── esp301_example.toml
│   │   ├── ipg_laser.toml    # IPG YLPP-200 laser
│   │   ├── maitai.toml       # Spectra-Physics MaiTai laser
│   │   ├── newport_1830c.toml # Newport 1830-C power meter
│   │   ├── red_pitaya_pid.toml
│   │   ├── thorlabs_pm400.toml
│   │   └── ...
├── docs/                     # Documentation
│   ├── adr/                  # Architecture Decision Records
│   ├── architecture/         # Architecture policies
│   ├── explanation/          # System explanations
│   ├── how-to/               # Guides and procedures
│   ├── reference/            # API and SDK references
│   └── tutorials/            # Learning tutorials
├── examples/                 # Rhai script examples
```

---

## Module Decomposition (2026-02 Tech Debt Remediation)

Several monolithic files have been decomposed into bounded submodules to improve
maintainability and review surface. Public API paths are preserved via re-exports.

### driver-pvcam

`crates/driver-pvcam/src/` — directory module:

| File | Lines | Responsibility |
|------|-------|----------------|
| `lib.rs` | ~2230 | PvcamDriver struct, trait impls, entry point |
| `macros.rs` | ~14k | Macro-generated parameter bindings |
| `components/acquisition/mod.rs` | ~3350 | Frame acquisition loop, callback handling |
| `components/acquisition/buffer.rs` | — | Frame buffer management |
| `components/acquisition/callback_context.rs` | — | FFI callback context |
| `components/acquisition/ffi_safe.rs` | — | FFI-safe type wrappers |
| `components/features/mod.rs` | ~3340 | PVCAM feature enumeration and parameter mapping |
| `components/features/enums.rs` | — | Feature enum definitions |
| `components/features/types.rs` | — | Feature type mappings |
| `components/connection.rs` | — | Camera connection lifecycle |
| `components/frame_pool.rs` | — | Frame pool integration |
| `components/speed_table.rs` | — | Readout speed table |
| `components/taps.rs` | — | Camera tap configuration |

### ui::panels::image_viewer

`crates/ui/src/panels/image_viewer/` — directory module:

| File | Lines | Responsibility |
|------|-------|----------------|
| `mod.rs` | ~6220 | ImageViewerPanel struct, impl, tests |
| `processing.rs` | ~480 | RGBA conversion pipeline, histogram computation |
| `colormap.rs` | ~260 | Colormap LUTs, ContrastMode, ScaleMode enums |
| `types.rs` | ~220 | `FrameUpdate` (`Arc<Vec<u8>>` data), StreamMetrics, state enums, channels |
| `echelle_extraction.rs` | ~56k | Echelle spectrograph order extraction |
| `echelle_profile_cache.rs` | ~5.5k | Cached echelle spatial profiles |
| `echelle_sidecar.rs` | ~7.7k | Echelle sidecar panel UI |

### server::grpc::hardware_service

`crates/server/src/grpc/hardware_service/` — directory module:

| File | Lines | Responsibility |
|------|-------|----------------|
| `mod.rs` | ~3510 | HardwareServiceImpl struct, gRPC trait impl, dedicated LZ4 compression thread, tests |
| `helpers.rs` | ~370 | Validation, error mapping, proto conversions |
| `streaming.rs` | ~290 | GrpcStreamObserver, ObserverFramePacket, StreamLimiter |

### Frame Streaming Pipeline

The camera-to-GUI frame streaming path is latency-critical and processes multi-MB
frames at 30+ fps. The pipeline is designed to minimize per-frame heap allocations.

```
Camera Driver ─→ FrameView<'a> (zero-copy borrow)
       │
       ▼
GrpcStreamObserver::on_frame()
  • Full:    frame.pixels().to_vec()        ← ALLOC #1 (copy from driver)
  • Preview: downsample_2x2() → Vec<u8>
  • Fast:    downsample_4x4() → Vec<u8>
       │
       ▼  (tokio::mpsc channel)
Dedicated LZ4 compression thread (std::thread)
  • compress_frame_into(&mut frame, &mut reusable_buf)  ← buffer reused
  • Owns persistent compression buffer across frames
       │
       ▼  (tokio::mpsc channel)
Async forwarding task
  • Rate limiting, backpressure, FPS/latency metrics
  • Sends FrameData proto to gRPC stream
       │
       ▼  ~~~ network (gRPC) ~~~
Client streaming loop (ui/panels/image_viewer/mod.rs)
  • decompress_frame_into(&mut frame, &mut reusable_buf)  ← buffer reused
       │
       ▼
FrameUpdate { data: Arc<Vec<u8>>, ... }
  • Arc<Vec<u8>> avoids layout-converting memcpy of Arc<[u8]>
       │
       ▼  (std::sync::mpsc channel)
RGBA converter thread (std::thread)
  • convert_frame_to_rgba_into(req, &mut reusable_rgba_buf)  ← buffer reused
       │
       ▼
egui TextureHandle::set()
```

**Buffer reuse APIs** (`protocol::compression`):
- `compress_frame_into(frame, buf)` / `decompress_frame_into(frame, buf)` — write
  into pre-allocated `Vec<u8>` buffers via `std::mem::swap`, eliminating per-frame
  allocation. Wire-compatible with the allocating `compress_frame`/`decompress_frame`.
- The compression thread owns its buffer; the client streaming loop owns its own.

**Threading model**: The compression thread is a long-lived `std::thread` (not
`spawn_blocking`) to avoid Tokio blocking-pool scheduling overhead (~50-200μs per
frame). This mirrors the RGBA converter thread already used on the client side.

See also: [ADR-014: Frame Streaming Buffer Reuse](../adr/014-frame-streaming-buffer-reuse.md)

---

## Persistence & Data Architecture

The system uses a **three-tier hybrid persistence model**, each tier optimized for its data category. See [ADR-015](../adr/015-hybrid-persistence-architecture.md) for the full rationale.

| Tier | Technology | Data Category | Example |
|------|-----------|---------------|---------|
| 1 — Design-time | TOML files (git-tracked) | Hardware configs, calibration profiles, device manifests | `config/devices/ell14.toml` |
| 2 — Runtime control plane | SQLite (embedded, via `rusqlite`/`tokio-rusqlite`, bd-2a2ne) | Parameter state, run records, device features, config reconciliation | `device_runtime_state` table |
| 3 — Science data | Specialized writers | Camera frames, scan datasets, spectral profiles | HDF5, Arrow IPC, Zarr V3 |

TOML is always the authoritative source of truth for device configuration. The SQLite control plane is the primary (and only) embedded DB backend — earlier revisions supported an optional SurrealDB backend; it has been removed (bd-2a2ne). Science data writers implement the `DocumentSink` trait, decoupling RunEngine document production from storage format.

The bridge between tiers is a **Kubernetes-style reconciliation loop**: TOML configs are shadow-written to SQLite on startup, and a watcher reconciles DB changes back into the `DeviceRegistry` (~300 ms convergence). `docs/how-to/surrealdb-integration.md` in this tree predates the SQLite migration and is retained for history; do not treat it as current.

---

## RunEngine Composition

The `RunEngine` (`crates/experiment/src/run_engine/mod.rs`) delegates to composed sub-components rather than owning all state directly:

*   **`TaskQueue`** (`task_queue.rs`) — Plan queue management (enqueue, dequeue, inspect, clear). Wraps a `Mutex<Vec<QueuedPlan>>` with a focused API, allowing queue logic (e.g., future priority ordering) to evolve independently.
*   **`WatchdogManager`** (`watchdog.rs`) — Orphan-plan detection. Tracks the timestamp of the last meaningful activity (MoveTo, Read, Trigger, etc.) and a configurable timeout (default: 5 minutes). A background task spawned via `RunEngine::spawn_watchdog()` periodically checks elapsed time and auto-aborts stale plans.

This composition pattern keeps the RunEngine struct focused on state machine transitions while delegating queue and watchdog concerns to specialized types.

---

## Production Monitoring

Two features support unattended overnight experiments:

*   **Webhook Alerting** (`crates/server/src/alerting.rs`, feature: `alerting` section in `config/config.v4.toml`) — Sends Slack/Discord-compatible webhook notifications when a device faults, exhausts restart attempts, or the RunEngine aborts a plan. Rate-limited per device key to prevent alert storms during cascading failures. All sends are fire-and-forget via `tokio::spawn`.

*   **Heartbeat JSONL Log** (`crates/server/src/health/heartbeat_log.rs`) — Writes one JSON object per minute to `/tmp/rust_daq_heartbeat.jsonl` containing system vitals: CPU%, RSS, disk free, device health summary, RunEngine state, and queue depth. Designed for post-mortem analysis of failed overnight runs without parsing full daemon logs.

Together, these provide real-time alerting (webhooks push to your phone) and forensic breadcrumbs (JSONL provides a timeline for post-mortems).

---

## Legacy Migration Status

This table has historically drifted from the code. Verified state as of
2026-04-19 (grep against `#[deprecated]` annotations on current HEAD):

**Still in the tree and carrying `#[deprecated]`:**

| Item | Crate | Annotation | Replacement |
|------|-------|-----------|-------------|
| `DeviceConfig` (schema v2) | hardware | `since = "0.3.0"` | `UniversalDriverConfig` (schema v3) |
| `TiffWriter::write_frame` | storage | `since = "0.3.0"` | `TiffWriter::write_frame_data` |
| `hardware::registry::create_mock_registry` | hardware | yes (see `hardware/src/registry/loading.rs:153`) | `driver_registry::create_canonical_mock_registry(workspace_root)` |

**Already removed from the tree** (earlier "deprecated" status resolved):
- `ScanServiceImpl` (server) — replaced by `RunEngineService` and deleted. Grep returns 0 matches.
- `take_frame_receiver` / `subscribe_frames` (common) — replaced by the `FrameObserver` trait and deleted. Grep returns 0 matches in `common/src/`.
- `DataPoint` (common) — replaced by `Observable<T>` / `Parameter<T>`.
- `ScriptHost` (scripting) — replaced by the `ScriptEngine` trait; `RhaiEngine` is the default backend.
- `Ell14Driver` legacy constructors (hardware) — serial drivers moved to `driver-universal`.
- `InstrumentConfigV3` type alias (common).
- `CodePreviewPanel::ui()` method (ui).
- `execute_script` free function (hardware).

**Not carrying `#[deprecated]` and not present** in the forms earlier docs
described — treat earlier documentation as historical:
- `GenericDriver::new` — a bare `new` constructor is not present on
  `GenericDriver`; the available constructors are `new_serial`,
  `new_tcp`, and `new_mock` (`crates/hardware/src/manifest_driver/driver.rs`).
- `PvcamDriver::new` / `PvcamDriver::from_config` — the current API is
  `PvcamDriver::new_async(camera_name)` (`crates/driver-pvcam/src/lib.rs:353`).
  There is no `from_config` method; the factory (`PvcamFactory`) is what
  consumes `InstrumentConfig`.
