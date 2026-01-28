# Codebase Structure

**Analysis Date:** 2026-01-21

## Directory Layout

```
rust-daq/
├── crates/                          # Workspace crates (monorepo pattern)
│   ├── common/                    # Foundation: traits, errors, types
│   │   └── src/
│   │       ├── lib.rs               # Module organization
│   │       ├── capabilities.rs      # Trait definitions (Movable, Readable, etc.)
│   │       ├── core.rs              # Legacy Measurement, DataPoint types
│   │       ├── driver.rs            # DriverFactory, DeviceComponents traits
│   │       ├── error.rs             # DaqError, error types
│   │       ├── error_recovery.rs    # Retry/fallback strategies
│   │       ├── parameter.rs         # Parameter<T> reactive state
│   │       ├── observable.rs        # Observable<T> with watchers
│   │       ├── data.rs              # Frame, PixelBuffer types
│   │       ├── pipeline.rs          # MeasurementSource trait
│   │       ├── limits.rs            # Size validation (DoS prevention)
│   │       ├── health/mod.rs        # Health check infrastructure
│   │       ├── experiment/document.rs # Document types (Start, Descriptor, Event, Stop)
│   │       ├── modules.rs           # Module management contracts
│   │       ├── platform.rs          # Platform-specific stubs
│   │       └── serial.rs            # Serial port trait (requires "serial" feature)
│   │
│   ├── daq-hardware/                 # HAL: Registry, configuration, driver management
│   │   └── src/
│   │       ├── lib.rs               # Public API re-exports
│   │       ├── registry.rs          # DeviceRegistry (device lookup by capability)
│   │       ├── factory.rs           # DriverFactory implementations, ConfiguredDriver
│   │       ├── drivers/mod.rs       # Driver crate re-exports (ell14, esp300, pvcam, etc.)
│   │       ├── config.rs            # DeclarativeDeviceConfig (TOML-driven devices)
│   │       ├── port_resolver.rs     # Serial port discovery utilities
│   │       ├── plugin.rs            # Plugin loading infrastructure
│   │       ├── resource_pool.rs     # Device resource allocation
│   │       └── drivers/
│   │           ├── ell14.rs         # Legacy ELL14 driver (compatibility)
│   │           ├── esp300.rs        # Legacy ESP300 driver (compatibility)
│   │           ├── maitai.rs        # Legacy MaiTai laser driver (compatibility)
│   │           ├── mock.rs          # Mock driver re-exports
│   │           ├── mock_serial.rs   # Test mock serial port
│   │           ├── generic_serial.rs # Config-driven serial driver
│   │           ├── script_engine.rs # Sandboxed Rhai execution
│   │           └── binary_protocol.rs # Modbus RTU, CRC support
│   │
│   ├── daq-driver-mock/             # Mock drivers for testing (no hardware)
│   │   └── src/
│   │       ├── lib.rs               # Factory registration, link() function
│   │       ├── mock_stage.rs        # MockStage, MockStageFactory
│   │       ├── mock_camera.rs       # MockCamera, MockCameraFactory
│   │       ├── mock_power_meter.rs  # MockPowerMeter, MockPowerMeterFactory
│   │       └── pattern.rs           # Gradient/checkerboard patterns
│   │
│   ├── daq-driver-thorlabs/         # Thorlabs ELL14 rotator (DriverFactory-based)
│   │   └── src/
│   │       ├── lib.rs               # Ell14Factory, module exports
│   │       └── ...                  # ELL14 RS-485 bus implementation
│   │
│   ├── daq-driver-newport/          # Newport ESP300 motion, 1830-C power meter
│   │   └── src/
│   │       ├── lib.rs               # Esp300Factory, Newport1830CFactory
│   │       └── ...                  # Serial drivers
│   │
│   ├── daq-driver-spectra-physics/  # MaiTai Ti:Sapphire laser
│   │   └── src/
│   │       ├── lib.rs               # MaiTaiFactory
│   │       └── ...                  # Laser control implementation
│   │
│   ├── daq-driver-pvcam/            # PVCAM camera (requires SDK)
│   │   ├── src/
│   │   │   ├── lib.rs               # PvcamFactory, feature-gating
│   │   │   └── ...                  # PVCAM bindings
│   │   └── pvcam-sys/src/           # Low-level PVCAM FFI bindings
│   │
│   ├── daq-driver-comedi/           # Comedi DAQ boards (Linux)
│   │   └── src/
│   │       ├── lib.rs               # ComediFactory
│   │       └── ...                  # Comedi device driver
│   │
│   ├── daq-storage/                 # Data persistence and buffering
│   │   └── src/
│   │       ├── lib.rs               # Public API re-exports
│   │       ├── ring_buffer.rs       # RingBuffer (sync), AsyncRingBuffer
│   │       ├── ring_buffer_reader.rs # Reader with statistics
│   │       ├── document_writer.rs   # Base DocumentWriter trait
│   │       ├── comedi_writer.rs     # Continuous acquisition writer
│   │       ├── hdf5_writer.rs       # HDF5 format (feature-gated)
│   │       ├── arrow_writer.rs      # Arrow/Parquet formats (feature-gated)
│   │       ├── tiff_writer.rs       # TIFF format (feature-gated)
│   │       ├── tap_registry.rs      # Tap/sink registration
│   │       └── compression.rs       # Compression codec wrappers
│   │
│   ├── daq-pool/                    # Zero-allocation frame pool
│   │   └── src/
│   │       ├── lib.rs               # Pool<T> generic pool
│   │       ├── buffer_pool.rs       # BufferPool specialized for bytes
│   │       └── frame_data.rs        # Frame data structures
│   │
│   ├── daq-experiment/              # Bluesky-style experiment orchestration
│   │   └── src/
│   │       ├── lib.rs               # RunEngine, Plan exports
│   │       ├── run_engine.rs        # RunEngine state machine
│   │       ├── plans.rs             # Plan trait, PlanRegistry
│   │       ├── plans_daq.rs         # Concrete plans (GridScan, TimeSeries, VoltageScan)
│   │       └── plans_imperative.rs  # Imperative plan executor
│   │
│   ├── daq-scripting/               # Rhai and optional Python scripting
│   │   └── src/
│   │       ├── lib.rs               # RhaiEngine, ScriptEngine trait
│   │       ├── rhai_engine.rs       # Rhai interpreter
│   │       ├── traits.rs            # ScriptEngine, ScriptValue, ScriptError
│   │       ├── bindings.rs          # Hardware bindings (stage, camera handles)
│   │       ├── plan_bindings.rs     # Plan API for scripts
│   │       ├── yield_bindings.rs    # Yield/pause infrastructure
│   │       ├── yield_handle.rs      # YieldHandle, YieldChannelBuilder
│   │       ├── comedi_bindings.rs   # Comedi-specific bindings
│   │       ├── script_runner.rs     # ScriptPlanRunner, ScriptRunReport
│   │       └── pyo3_engine.rs       # Python support (feature-gated)
│   │
│   ├── daq-proto/                   # Protobuf definitions and conversions
│   │   ├── src/
│   │   │   ├── lib.rs               # Generated proto module includes
│   │   │   ├── convert.rs           # Domain ↔ Proto conversions
│   │   │   ├── compression.rs       # Frame compression (JPEG, PNG)
│   │   │   └── downsample.rs        # Binning for bandwidth reduction
│   │   └── proto/
│   │       ├── daq.proto            # Core DAQ service (devices, streams, plans)
│   │       ├── health.proto         # gRPC health checking
│   │       └── ni_daq.proto         # NI DAQ extensions
│   │
│   ├── daq-server/                  # gRPC backend server
│   │   └── src/
│   │       ├── lib.rs               # Feature-gated module exports
│   │       ├── grpc/
│   │       │   ├── mod.rs           # Server setup, interceptors
│   │       │   ├── server.rs        # DaqServer struct, main listen loop
│   │       │   ├── hardware_service.rs  # Device list, capability query, frame streaming
│   │       │   ├── run_engine_service.rs # Plan queueing, pause/resume, document streaming
│   │       │   ├── scan_service.rs  # High-level scan definitions (GridScan, etc.)
│   │       │   ├── storage_service.rs # Data persistence endpoints
│   │       │   ├── plugin_service.rs # Plugin loading/unloading (serial feature)
│   │       │   ├── module_service.rs # Module registration (modules feature)
│   │       │   ├── metrics_service.rs # Performance metrics, health
│   │       │   ├── health_service.rs # gRPC health protocol
│   │       │   ├── custom_health_service.rs # Extended health checks
│   │       │   ├── preset_service.rs # Preset save/load
│   │       │   ├── error_mapping.rs # Domain error → gRPC Status
│   │       │   └── error_mapping_tests.rs # Error mapping tests
│   │       ├── health.rs            # Health monitor infrastructure
│   │       ├── modules/             # Module management (feature-gated)
│   │       └── rerun_sink.rs        # Rerun visualization integration
│   │
│   ├── daq-egui/                    # GUI application
│   │   ├── src/
│   │   │   ├── lib.rs               # Feature-gated API
│   │   │   ├── main.rs              # CLI entry (standalone feature)
│   │   │   ├── main_rerun.rs        # Rerun visualization entry
│   │   │   ├── app.rs               # App state machine, frame/panel rendering
│   │   │   ├── client.rs            # gRPC client wrapper with error handling
│   │   │   ├── connection.rs        # Connection state management
│   │   │   ├── reconnect.rs         # Auto-reconnect with backoff
│   │   │   ├── daemon_launcher.rs   # Spawn daemon if not running
│   │   │   ├── theme.rs             # Dark/light theme, color palette
│   │   │   ├── layout.rs            # Panel layout engine
│   │   │   ├── icons.rs             # Icon set
│   │   │   ├── gui_log_layer.rs     # Logging panel backend
│   │   │   ├── panels/
│   │   │   │   ├── mod.rs           # Panel registry
│   │   │   │   ├── devices.rs       # Device list, state control
│   │   │   │   ├── devices_tiled.rs # Tiled device view
│   │   │   │   ├── image_viewer.rs  # 2D frame display with zoom/pan
│   │   │   │   ├── signal_plotter.rs # 1D/2D plot with history
│   │   │   │   ├── signal_plotter_stream.rs # Stream-optimized plotter
│   │   │   │   ├── scans.rs         # Plan builder UI
│   │   │   │   ├── plan_runner.rs   # Execute plans, show progress
│   │   │   │   ├── scripts.rs       # Script editor, upload, run
│   │   │   │   ├── storage.rs       # Storage configuration, format selection
│   │   │   │   ├── document_viewer.rs # Experiment documents browser
│   │   │   │   ├── logging.rs       # Log stream display
│   │   │   │   ├── modules.rs       # Module browser (feature-gated)
│   │   │   │   ├── getting_started.rs # Onboarding panel
│   │   │   │   └── instrument_manager/  # Multi-panel instrument control
│   │   │   │       ├── mod.rs
│   │   │   │       ├── dispatch.rs  # Route to device-specific panels
│   │   │   │       └── comedi/      # Comedi-specific UI
│   │   │   └── widgets/             # Reusable UI components
│   │   │       └── ...              # Custom widgets
│   │   └── gui-tauri/               # Desktop wrapper (optional Tauri integration)
│   │
│   ├── daq-bin/                     # CLI and daemon entry point
│   │   └── src/
│   │       └── main.rs              # Clap CLI, run/daemon/client commands
│   │
│   ├── rust-daq/                    # Integration layer (top-level crate)
│   │   ├── src/
│   │   │   ├── lib.rs               # Feature-gated module re-exports
│   │   │   ├── prelude.rs           # Organized re-exports (core, hardware, storage, etc.)
│   │   │   ├── config.rs            # AppConfig, Figment-based loading
│   │   │   └── validation.rs        # Config parameter validation
│   │   ├── fuzz/                    # Fuzzing harnesses (excluded from workspace)
│   │   └── python/                  # Python bindings (excluded)
│   │
│   ├── daq-plugin-api/              # FFI plugin interface (abi_stable)
│   │   └── src/
│   │
│   ├── daq-plugin-example/          # Example plugin using daq-plugin-api
│   │   └── src/
│   │
│   ├── daq-drivers/                 # Driver aggregation and linking
│   │   └── src/
│   │       ├── lib.rs               # Re-exports all driver crates
│   │       └── link_drivers.rs      # Linker integration for factory registration
│   │
│   ├── comedi-sys/                  # Low-level Comedi FFI bindings
│   │   └── src/
│   │
│   └── daq-examples/                # Example code (excluded from workspace build)
│       └── ...
│
├── config/                          # Configuration files and presets
│   ├── maitai_hardware.toml         # Lab hardware (laser, rotators, stages, camera)
│   ├── mock_hardware.toml           # Mock-only configuration
│   └── hosts/                       # Host-specific environment configs
│       └── maitai.env               # PVCAM and serial port setup for maitai@100.117.5.12
│
├── examples/                        # Example scripts and configurations
│   ├── simple_scan.rhai             # Rhai script example
│   └── ...
│
├── scripts/                         # Utility shell scripts
│   ├── env-check.sh                 # Validate and set PVCAM environment
│   ├── build-maitai.sh              # Clean build for PVCAM hardware
│   └── ...
│
├── docs/                            # Documentation
│   ├── architecture/                # Architecture Decision Records (ADRs)
│   │   ├── adr-pvcam-continuous-acquisition.md
│   │   ├── adr-pvcam-driver-architecture.md
│   │   └── ...
│   ├── guides/                      # How-to guides
│   │   ├── testing.md
│   │   └── ...
│   └── ...
│
├── tools/                           # Development tools
│   └── ...
│
├── .planning/                       # GSD planning documents (this directory)
│   └── codebase/
│       ├── ARCHITECTURE.md          # (you are here)
│       ├── STRUCTURE.md
│       ├── STACK.md
│       ├── INTEGRATIONS.md
│       ├── CONVENTIONS.md
│       └── TESTING.md
│
├── .beads/                          # Issue tracking database
│
├── .github/                         # GitHub workflows
│   └── workflows/
│
├── Cargo.toml                       # Workspace root
└── Cargo.lock
```

## Directory Purposes

**crates/common/:**
- Purpose: Foundation abstractions - traits, types, error handling
- Contains: Capability traits (Movable, Readable, FrameProducer, etc.), Parameter<T>, Observable<T>, error types, limits enforcement
- Key files: `capabilities.rs` (trait definitions), `parameter.rs` (reactive state), `driver.rs` (plugin interface)

**crates/daq-hardware/:**
- Purpose: Hardware abstraction layer and runtime device management
- Contains: DeviceRegistry (device lookup by capability), DriverFactory implementations, configuration loading, port discovery
- Key files: `registry.rs` (device registry), `factory.rs` (factory patterns), `drivers/mod.rs` (driver re-exports)

**crates/daq-driver-*/ (mock, thorlabs, newport, spectra-physics, pvcam, comedi):**
- Purpose: Individual hardware driver implementations
- Contains: Device-specific initialization, serial communication, capability trait implementations
- Key pattern: Each implements DriverFactory trait for registration; drivers are Arc-wrapped and accessed via trait objects

**crates/daq-storage/:**
- Purpose: Data persistence, buffering, and format writing
- Contains: Ring buffers (sync and async), writers (CSV, HDF5, Arrow, TIFF), document types
- Key files: `ring_buffer.rs` (circular buffer), `document_writer.rs` (base writer trait), format-specific writers

**crates/daq-pool/:**
- Purpose: Zero-allocation frame buffering for high-FPS acquisition
- Contains: Generic Pool<T>, specialized BufferPool, Loaned pointer caching
- Key pattern: Semaphore + lock-free queue avoids per-access locking; Loaned caches raw pointer

**crates/daq-experiment/:**
- Purpose: Bluesky-style plan execution and RunEngine orchestration
- Contains: Plans (GridScan, TimeSeries, VoltageScan), RunEngine state machine, PlanRegistry
- Key files: `run_engine.rs` (orchestrator), `plans.rs` (plan traits), `plans_daq.rs` (concrete plans)

**crates/daq-scripting/:**
- Purpose: Script-based automation with hardware bindings
- Contains: RhaiEngine (default), optional PyO3Engine, bindings for stages/cameras/Comedi, yield channel infrastructure
- Key files: `rhai_engine.rs` (Rhai interpreter), `bindings.rs` (hardware API), `script_runner.rs` (plan runner)

**crates/daq-proto/:**
- Purpose: gRPC transport layer and type conversions
- Contains: Generated Protobuf types, domain ↔ proto conversions, compression/downsampling
- Key files: `proto/daq.proto` (main service definition), `convert.rs` (type mapping)

**crates/daq-server/:**
- Purpose: gRPC backend services
- Contains: HardwareService (device listing, streaming), RunEngineService (plan execution), ScriptingService, health checks
- Key files: `server.rs` (listener and interceptors), `hardware_service.rs`, `run_engine_service.rs`

**crates/daq-egui/:**
- Purpose: Desktop GUI application
- Contains: App state machine, panels (devices, images, signals, plans, scripts), gRPC client, auto-reconnect logic
- Key files: `app.rs` (main UI state), `client.rs` (gRPC client wrapper), `panels/` (individual UI panels)

**crates/daq-bin/:**
- Purpose: CLI entry point and daemon launcher
- Contains: Clap parser, subcommands (run, daemon, client)
- Key files: `main.rs` (all CLI logic)

**crates/rust-daq/:**
- Purpose: Integration layer with organized re-exports
- Contains: Prelude module (grouped by functionality), configuration, validation
- Key files: `prelude.rs` (convenient imports), `config.rs` (AppConfig loading)

**config/:**
- Purpose: Application configuration templates and hardware definitions
- Contains: TOML files for hardware setup (maitai, mock), host-specific env variables
- Key files: `maitai_hardware.toml` (lab setup), `mock_hardware.toml` (testing)

**examples/:**
- Purpose: Sample code for developers
- Contains: Rhai script examples, configuration templates
- Key files: `simple_scan.rhai` (script example)

**scripts/:**
- Purpose: Build and environment utilities
- Contains: PVCAM environment setup, helper functions
- Key files: `env-check.sh` (validate/set PVCAM env), `build-maitai.sh` (clean PVCAM build)

**docs/:**
- Purpose: Architecture documentation and guides
- Contains: ADRs (Architectural Decision Records), how-to guides, testing documentation
- Key files: `architecture/adr-pvcam-*.md` (design decisions)

## Key File Locations

**Entry Points:**
- `crates/daq-bin/src/main.rs` - CLI commands (run script, start daemon, remote control)
- `crates/daq-egui/src/main.rs` - GUI application (standalone feature)
- `crates/daq-egui/src/main_rerun.rs` - Rerun visualization entry

**Configuration:**
- `crates/rust-daq/src/config.rs` - AppConfig structure and Figment loading
- `config/maitai_hardware.toml` - Lab hardware configuration
- `config/mock_hardware.toml` - Mock device configuration

**Core Logic:**
- `crates/common/src/driver.rs` - DriverFactory plugin interface
- `crates/daq-hardware/src/registry.rs` - DeviceRegistry (runtime device lookup)
- `crates/daq-experiment/src/run_engine.rs` - Plan execution and orchestration
- `crates/daq-server/src/grpc/hardware_service.rs` - gRPC device listing and streaming

**Hardware Drivers:**
- `crates/daq-driver-mock/src/` - Mock drivers (testing)
- `crates/daq-driver-thorlabs/src/` - Thorlabs ELL14 (DriverFactory-based)
- `crates/daq-driver-newport/src/` - Newport ESP300 and 1830-C
- `crates/daq-driver-spectra-physics/src/` - MaiTai laser
- `crates/daq-driver-pvcam/src/` - PVCAM camera (requires SDK)

**Testing:**
- `crates/daq-driver-mock/src/` - Mock implementations for all devices
- `crates/common/examples/` - Example code and review checks
- Individual crates: `tests/` directories with integration tests

**GUI:**
- `crates/daq-egui/src/app.rs` - Main app state machine and rendering loop
- `crates/daq-egui/src/panels/` - Individual panel implementations
- `crates/daq-egui/src/client.rs` - gRPC client wrapper

**gRPC:**
- `crates/daq-proto/proto/daq.proto` - Service definition (HardwareService, RunEngineService, ScriptingService, etc.)
- `crates/daq-server/src/grpc/` - Service implementations

## Naming Conventions

**Files:**
- Rust source files: `snake_case.rs` (e.g., `mock_stage.rs`, `error_mapping.rs`)
- Driver crates: `daq-driver-{lowercase}` (e.g., `daq-driver-thorlabs`, `daq-driver-pvcam`)
- Feature: `snake_case` in Cargo.toml (e.g., `pvcam_hardware`, `storage_hdf5`)

**Directories:**
- Crate names: `daq-{component}` or `{vendor}-sys` (e.g., `common`, `daq-storage`, `comedi-sys`)
- Module subdirs: Flat structure preferred (one file per module); complex modules get subdirs (e.g., `panels/`, `drivers/`, `grpc/`)
- Config: Lowercase with dashes for machine/host names (e.g., `maitai_hardware.toml`, `hosts/maitai.env`)

**Modules:**
- Public re-exports: `prelude.rs` for organized imports
- Feature-gated: `#[cfg(feature = "...")]` wrapping entire module blocks
- Legacy compatibility: Kept alongside new implementations (e.g., `drivers/ell14.rs` legacy alongside `daq-driver-thorlabs/`)

**Types:**
- Traits: PascalCase, action-verb names (e.g., `Movable`, `FrameProducer`, `Triggerable`)
- Structs: PascalCase (e.g., `RunEngine`, `DeviceRegistry`, `Parameter`)
- Enums: PascalCase (e.g., `EngineState`, `PlanCommand`, `DeviceCategory`)
- Constants: UPPER_SNAKE_CASE (e.g., `MAX_FRAME_BYTES`, `DEFAULT_POOL_SIZE`)

## Where to Add New Code

**New Hardware Driver:**
1. Create `crates/daq-driver-{device-name}/Cargo.toml`
2. Implement `crates/daq-driver-{device-name}/src/lib.rs` with struct implementing `DriverFactory`
3. Driver struct implements capability traits (e.g., `Movable`, `Readable`)
4. Register in composition root: `daq-bin/src/main.rs` calls `registry.register_factory(Box::new(MyFactory))`

**New Capability Trait:**
1. Define in `crates/common/src/capabilities.rs` (async, Send + Sync)
2. Add variant to `Capability` enum in `common/src/driver.rs`
3. Add field to `DeviceComponents` struct (e.g., `.with_my_capability(driver)`)
4. Update gRPC service if remote access needed: `crates/daq-server/src/grpc/hardware_service.rs`

**New Plan Type:**
1. Define struct in `crates/daq-experiment/src/plans_daq.rs` (for DAQ-specific) or `plans_imperative.rs`
2. Implement `Plan` trait (async fn next() -> Option<PlanCommand>)
3. Register with `PlanRegistry`: `registry.register("plan_name", Box::new(my_plan))`
4. Optional: Add Rhai bindings in `crates/daq-scripting/src/plan_bindings.rs`

**New GUI Panel:**
1. Create file in `crates/daq-egui/src/panels/{panel_name}.rs`
2. Struct must implement panel trait (typically `fn ui(&mut self, ui: &mut egui::Ui)`)
3. Register in `panels/mod.rs`: add to panel enum and match in rendering loop
4. Add toggle in app state to show/hide: `app.show_{panel_name}`

**New gRPC Service:**
1. Define `service` in `crates/daq-proto/proto/daq.proto`
2. Run code generation (build script auto-generates Rust types in `crates/daq-proto/src/daq.rs`)
3. Implement service struct in `crates/daq-server/src/grpc/{service_name}.rs`
4. Register server in `crates/daq-server/src/grpc/server.rs`: add `ServiceServer` to Server builder

**New Storage Format:**
1. Create writer in `crates/daq-storage/src/{format}_writer.rs`
2. Implement `DocumentWriter` trait
3. Add feature flag in `crates/daq-storage/Cargo.toml` (e.g., `storage_myformat`)
4. Register in `StorageFormat` enum in `crates/daq-storage/src/comedi_writer.rs`

**Utility Functions (Shared Code):**
1. If crate-independent: Add to `crates/common/src/` new module
2. If hardware-specific: Add to driver crate's utilities module
3. If UI: Add to `crates/daq-egui/src/widgets/` for reusable components
4. Cross-crate imports via `use rust_daq::prelude::*` or direct crate imports

## Special Directories

**crates/rust-daq/fuzz/:**
- Purpose: Fuzzing harnesses for property testing
- Generated: No (hand-written)
- Committed: Yes
- Excluded from workspace builds (`exclude` in root Cargo.toml)

**crates/rust-daq/python/:**
- Purpose: Python FFI bindings (PyO3)
- Generated: Partially (Cargo generates extension module)
- Committed: Yes (source), No (compiled .so)
- Excluded from workspace builds

**crates/daq-examples/:**
- Purpose: Example projects and demonstrations
- Generated: No
- Committed: Yes
- Excluded from workspace builds (excluded in root Cargo.toml)

**target/:**
- Purpose: Build artifacts, compiled binaries
- Generated: Yes (by Cargo)
- Committed: No (.gitignore)

**config/:**
- Purpose: Configuration templates and presets
- Generated: No (user-edited)
- Committed: Yes (checked in for reproducibility)
- Location: Project root for easy discovery

**.beads/:**
- Purpose: Issue tracking database (bytecode format)
- Generated: Yes (by `bd` tool)
- Committed: Yes (persistent across sessions)

---

*Structure analysis: 2026-01-21*
