# Architecture

**Analysis Date:** 2026-01-21

## Pattern Overview

**Overall:** Headless-first plugin architecture with capability-based trait composition

**Key Characteristics:**
- Layered hardware abstraction with driver factory pattern for extensibility
- Async-first design using Tokio throughout
- Capability traits (not monolithic device traits) enable fine-grained composition
- Multi-consumer data streaming via Tokio broadcast channels
- Zero-allocation frame handling for high-FPS acquisition via object pools
- Modular CLI, gRPC server, and GUI (egui) can run independently

## Layers

**Foundation (daq-core):**
- Purpose: Defines all core abstractions, error types, and fundamental protocols
- Location: `crates/daq-core/src/`
- Contains: Capability traits, Parameter<T>, Observable types, driver factory interface, error types, limits
- Depends on: Tokio, async-trait, anyhow
- Used by: All other crates, acts as the contract layer

**Hardware Abstraction Layer (daq-hardware):**
- Purpose: Registry, device composition, configuration, and driver management
- Location: `crates/daq-hardware/src/`
- Contains: DeviceRegistry, DeviceComponents, DriverFactory implementations, capability introspection, port resolver
- Depends on: daq-core, individual driver crates
- Used by: daq-bin (daemon), daq-server (gRPC), daq-egui (GUI), daq-experiment (plans)

**Driver Layer (daq-driver-*):**
- Purpose: Individual hardware driver implementations with DriverFactory trait
- Location: `crates/daq-driver-{mock,thorlabs,newport,spectra-physics,pvcam,comedi}/src/`
- Contains: Device-specific drivers, serial communication, hardware initialization
- Depends on: daq-core, possibly daq-hardware traits
- Used by: Registered in DeviceRegistry at startup; accessed via capability traits

**Storage & Processing (daq-storage, daq-pool):**
- Purpose: Data persistence and high-performance frame buffering
- Location: `crates/daq-storage/src/`, `crates/daq-pool/src/`
- Contains: Ring buffers, writers (CSV, HDF5, Arrow, TIFF), async buffer pools, document writers
- Depends on: daq-core (Frame types)
- Used by: daq-server, daq-egui, experiments

**Experiment Orchestration (daq-experiment):**
- Purpose: Bluesky-style plan execution, RunEngine state management
- Location: `crates/daq-experiment/src/`
- Contains: Plans, PlanRegistry, RunEngine, document types (Start, Descriptor, Event, Stop)
- Depends on: daq-core, daq-hardware
- Used by: daq-server (RunEngineService), daq-scripting (plan runners)

**Scripting Engine (daq-scripting):**
- Purpose: Rhai script execution with hardware bindings, optional Python support
- Location: `crates/daq-scripting/src/`
- Contains: RhaiEngine, script bindings for cameras/stages, ComediBindings, yield channel infrastructure
- Depends on: daq-core, daq-hardware, daq-experiment (for plan bindings)
- Used by: daq-bin (run command), daq-server (script upload/execute)

**Communication (daq-proto):**
- Purpose: Protobuf definitions and type conversions between domain and transport layers
- Location: `crates/daq-proto/src/`
- Contains: Generated proto types (daq.proto, health.proto, ni_daq.proto), conversion traits, compression/downsampling
- Depends on: daq-core (for converting to/from domain types)
- Used by: daq-server (gRPC), GUI clients, remote clients

**Backend (daq-server):**
- Purpose: gRPC server with multi-service architecture
- Location: `crates/daq-server/src/grpc/`
- Contains: HardwareService, RunEngineService, ScriptingService (optional), ModuleService (optional), ControlService
- Depends on: daq-core, daq-hardware, daq-proto, daq-experiment, daq-scripting (optional)
- Used by: daq-bin (daemon mode), remote clients via gRPC

**Frontend (daq-egui):**
- Purpose: GUI application with panel-based UI
- Location: `crates/daq-egui/src/`
- Contains: App state machine, panels (devices, image viewer, signal plotter, scans, scripts), client connection logic, auto-reconnect
- Depends on: daq-proto (to call gRPC), tokio, egui
- Used by: Desktop client connected to daemon

**Integration Layer (rust-daq):**
- Purpose: Top-level workspace coordination and prelude exports
- Location: `crates/rust-daq/src/`
- Contains: Prelude module (organized re-exports), config, validation, optional feature gates
- Depends on: All other crates, conditionally based on features
- Used by: Applications like daq-bin that need organized imports

**Entry Points (daq-bin):**
- Purpose: CLI commands and daemon startup
- Location: `crates/daq-bin/src/main.rs`
- Contains: Clap CLI parser, `run` command (script execution), `daemon` command (gRPC server), `client` commands (remote control)
- Depends on: rust-daq, daq-server, daq-scripting
- Used by: Command-line users

## Data Flow

**Script Execution (Single Shot):**

1. User: `rust-daq run script.rhai --config hardware.toml`
2. daq-bin parses config, loads hardware into DeviceRegistry
3. RhaiEngine::new() initialized with registered device bindings
4. Script executes, calls hardware via Rhai bindings (e.g., `stage.move_to(x)`)
5. Results streamed to stdout or captured by script

**Daemon Mode (Continuous):**

1. User: `rust-daq daemon --port 50051 --hardware-config config.toml`
2. daq-bin starts gRPC server on port 50051
3. Hardware registry loaded; factories register drivers
4. Client connects (GUI or remote tool) via gRPC
5. Client sends requests → gRPC service → DeviceRegistry → Driver → Hardware
6. Responses streamed back; frame data streamed via `StreamFramesRequest`

**Experiment with RunEngine:**

1. Client uploads plan (e.g., GridScan) to RunEngineService
2. RunEngine queues plan and transitions to RUNNING
3. Plan yields commands: `Move(stage, x)`, `Trigger(camera)`, `Read(sensor)`
4. RunEngine deserializes commands, looks up device from registry, calls capability trait
5. RunEngine emits documents (Start, Descriptor, Event, Stop) to client
6. Storage layer writes documents to file (CSV, HDF5, etc.)

**Real-Time Frame Streaming:**

1. Client requests `StreamFrames(device_id=camera0, quality=Preview)`
2. HardwareService acquires device, calls `start_stream()`
3. Camera driver reads frames into pool (zero-allocation)
4. gRPC service downsamples if needed (4x4 binning for Preview)
5. Frames compressed (e.g., JPEG if bandwidth-limited)
6. Backpressure handling: if channel buffer >75% full, newest frames dropped
7. Client receives stream until `StopFrames()` called or channel closes

**State Management:**

- Device state: `Parameter<T>` (reactive, with hardware callbacks)
- Hardware-to-app feedback: `Observable<T>` (allows async watchers)
- RunEngine state: `EngineState` enum (Ready, Running, Paused, Stopped)
- GUI state: Egui local state per panel + connection state machine
- Shared mutable hardware access: Single registry instance (Arc<RwLock>), individual drivers wrapped in Arc

## Key Abstractions

**Capability Traits:**
- Purpose: Define what hardware can do without monolithic device traits
- Examples: `Movable`, `Readable`, `FrameProducer`, `Triggerable`, `ExposureControl`, `WavelengthTunable`, `ShutterControl`, `EmissionControl`, `Parameterized`
- Pattern: Each trait is small, async, and composable; devices implement multiple traits
- Files: `crates/daq-core/src/capabilities.rs` (trait definitions)

**Parameter<T>:**
- Purpose: Reactive state with hardware callbacks; replaces raw Mutex/RwLock
- Pattern: Holds value T, wraps in Arc<RwLock<T>>, optionally connected to async hardware write function
- Usage: `let wavelength = Parameter::new("wavelength_nm", 800.0).connect_to_hardware_write(...)`
- Files: `crates/daq-core/src/parameter.rs`

**DeviceRegistry:**
- Purpose: Central runtime registry of discovered devices with capability introspection
- Pattern: Stores factories and instantiated devices; trait objects wrap drivers; query by capability
- Usage: `registry.register_factory(Box::new(Ell14Factory))`, `registry.get_movable("rotator_2")?`
- Files: `crates/daq-hardware/src/registry.rs`

**DriverFactory:**
- Purpose: Plugin interface for dynamic driver registration
- Pattern: Factories implement trait, build() returns DeviceComponents with capabilities
- Usage: `impl DriverFactory for Ell14Factory { fn build() -> DeviceComponents { ... } }`
- Files: `crates/daq-core/src/driver.rs`, individual driver crate implementations

**DeviceComponents:**
- Purpose: Lightweight wrapper carrying device + all its capabilities
- Pattern: Builder-style type with optional Arc<dyn Capability1>, Arc<dyn Capability2>, etc.
- Usage: `DeviceComponents::new().with_movable(driver).with_parameterized(driver)`
- Files: `crates/daq-core/src/driver.rs`

**RunEngine:**
- Purpose: State machine executing Plans (Bluesky-inspired) with pause/resume
- Pattern: Queues plans, emits lifecycle documents, coordinates hardware through capability traits
- States: Ready → Running → Paused → Running → Stopped
- Files: `crates/daq-experiment/src/run_engine.rs`

**Plan / PlanRegistry:**
- Purpose: Declarative experiment definitions that yield commands
- Pattern: Plans implement Plan trait (yields PlanCommand enum: Move, Trigger, Read, etc.)
- Examples: GridScan, TimeSeries, VoltageScan
- Files: `crates/daq-experiment/src/plans.rs`, `plans_daq.rs`, `plans_imperative.rs`

**Ring Buffer (Sync & Async):**
- Purpose: Lock-free circular buffer for streaming frame/scalar data without per-frame allocation
- Pattern: Fixed-size, wrap-around when full; async variant yields when full
- Usage: `let buf = RingBuffer::create(1024*1024)?; let data = buf.read_snapshot()?`
- Files: `crates/daq-storage/src/ring_buffer.rs`, `ring_buffer_reader.rs`

**Pool<T> / BufferPool:**
- Purpose: Zero-allocation object pool for high-FPS frame handling
- Pattern: Semaphore + lock-free queue (SegQueue); Loaned caches pointer for lock-free access
- Usage: `let frame = pool.acquire().await; frame[0] = pixel; drop(frame); // returns to pool`
- Files: `crates/daq-pool/src/buffer_pool.rs`, `lib.rs`

**Observable<T>:**
- Purpose: Reactive property with async watchers
- Pattern: Watches can subscribe to value changes, invoked whenever set() called
- Usage: `let obs = Observable::new(42); obs.watch(|val| async move { println!("{}", val); }).await`
- Files: `crates/daq-core/src/observable.rs`

**Document (Bluesky Model):**
- Purpose: Structured data flow from experiments
- Pattern: StartDoc (metadata), Descriptor (schema), Event (data), StopDoc (summary)
- Usage: Emitted by RunEngine, written to storage, consumed by GUI/analysis
- Files: `crates/daq-core/src/experiment/document.rs`

## Entry Points

**CLI (rust-daq run):**
- Location: `crates/daq-bin/src/main.rs`
- Triggers: User runs `rust-daq run script.rhai`
- Responsibilities: Parse args, load config, initialize hardware registry, create RhaiEngine, execute script, report results

**Daemon (rust-daq daemon):**
- Location: `crates/daq-bin/src/main.rs`
- Triggers: User runs `rust-daq daemon --port 50051`
- Responsibilities: Load config, start hardware registry, initialize gRPC server, listen for client connections, graceful shutdown on SIGTERM

**GUI (daq-egui):**
- Location: `crates/daq-egui/src/main.rs` (or `app.rs` for library mode)
- Triggers: User runs GUI binary or imports as library
- Responsibilities: Initialize egui, connect to daemon, render panels, handle user input, update device state

**gRPC Services (daq-server):**
- Location: `crates/daq-server/src/grpc/`
- Services: HardwareService, RunEngineService, ScriptingService, ModuleService, HealthService
- Triggered: Implicitly when client sends RPC request
- Responsibilities: Deserialize proto message, validate, call domain logic (registry/engine/scripting), serialize response

## Error Handling

**Strategy:** Layered error mapping with context preservation

**Patterns:**

1. **Domain Layer (daq-core):** Uses `anyhow::Result<T>` for internal errors
   - File: `crates/daq-core/src/error.rs`
   - Example: `driver.move_abs(x).await.context("Failed to move stage")?`

2. **gRPC Layer (daq-server):** Converts domain errors to gRPC Status codes
   - File: `crates/daq-server/src/grpc/error_mapping.rs`
   - Pattern: `anyhow::Error` → `tonic::Status` with error code + message

3. **Recovery Layer (daq-core):** Implements fallback strategies
   - File: `crates/daq-core/src/error_recovery.rs`
   - Example: Retry with exponential backoff, degrade to mock driver

4. **Driver Layer:** Serial communication errors get retry logic
   - File: Individual driver crates
   - Pattern: Validate device identity on connect to fail-fast on misconfiguration

## Cross-Cutting Concerns

**Logging:**
- Framework: `tracing` crate with `tracing-subscriber`
- GUI capture: `GuiLogLayer` intercepts span events and buffers for display panel
- Files: `crates/daq-egui/src/gui_log_layer.rs`, individual driver crate logs

**Validation:**
- Size limits: `crates/daq-core/src/limits.rs` enforces MAX_SCRIPT_SIZE, MAX_FRAME_BYTES, etc.
- Device validation: Serial drivers query device identity (`*IDN?`) on connect
- Configuration: Figment-based validation in config loading

**Authentication:**
- Optional feature in gRPC: JWT token validation in interceptor
- Default: Disabled (open to localhost)
- Files: `crates/daq-server/src/grpc/server.rs` (interceptor setup)

**Testing:**
- Levels: Unit tests in crates, integration tests in `tests/`, hardware tests with `#[cfg(feature = "hardware_tests")]`
- Mock drivers: `crates/daq-driver-mock/` provides MockStage, MockCamera, MockPowerMeter
- Pattern: All drivers accessible via DriverFactory for substitution in tests

---

*Architecture analysis: 2026-01-21*
