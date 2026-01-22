# Technology Stack

**Analysis Date:** 2026-01-21

## Languages

**Primary:**
- Rust (Edition 2021) - All core business logic, server, drivers, and device acquisition

**Secondary:**
- Protobuf 3 - gRPC API definitions (`crates/daq-proto/proto/daq.proto`)
- TOML - Configuration files, manifests
- Python (optional) - Scripting engine via PyO3 (`daq-scripting/python` feature)
- Rhai - Script-based plugin language for config-driven drivers

## Runtime

**Environment:**
- Tokio 1.36 - Async runtime for all I/O operations (multi-threaded)
- Linux/macOS target (x86_64 primarily, WebAssembly conditionals present)

**Package Manager:**
- Cargo - Rust package management
- Lockfile: `Cargo.lock` (present, tracked in git)

## Frameworks

**Core:**
- `tokio` 1.36 - Async runtime foundation
- `tonic` 0.10 - gRPC server/client library with TLS support
- `prost` 0.12 - Protocol buffer code generation
- `async-trait` 0.1 - Async trait definitions for cross-crate abstractions

**Storage:**
- `hdf5` (hdf5-metno 0.11) - Optional HDF5 file format support
- `arrow` 57 - Apache Arrow for columnar data (optional)
- `parquet` 57 - Parquet file format (optional)
- `csv` 1.3 - CSV file writing (default)
- `image` 0.25 - TIFF format export (optional)
- `memmap2` 0.9 - Memory-mapped file access

**Serialization:**
- `serde` 1.0 + `serde_json` 1.0 - Serialization/deserialization framework
- `toml` 0.9.8 - TOML config parsing
- `bincode` 1.3 - Binary serialization

**GUI (Optional):**
- `egui` 0.33 - Immediate-mode GUI framework
- `eframe` 0.33 - egui window/rendering backend (OpenGL via glow)
- `egui_dock` 0.18 - Dockable panels
- `egui_plot` 0.34 - Plotting/charting
- `egui_extras` 0.33 - Image loaders (TIFF, PNG, etc.)
- `egui-phosphor` 0.11 - Icon set for UI
- `egui-notify` 0.19 - Notification toasts
- `rerun` 0.27.3 - 3D/2D visualization with embedded viewer

**Configuration:**
- `figment` 0.10 - Structured config management (TOML + env var overrides)
- `config` 0.14 - Legacy config layer (being phased out)

**Build & Development:**
- `tonic-build` 0.11 - Protocol buffer code generation at build time
- `flatbuffers` 24.3.25 - FlatBuffers schema compiler (legacy, minimal use)
- `criterion` 0.5 - Benchmarking framework

## Key Dependencies

**Critical:**
- `bytes` 1.11 - Efficient byte buffer handling for frame streaming
- `parking_lot` 0.12 - Faster synchronization primitives (replaces std::sync)
- `anyhow` 1.0 - Error context and chaining
- `thiserror` 1.0 - Error type derive macros
- `tracing` 0.1 - Structured logging/observability
- `log` 0.4 - Legacy logging interface (backing for tracing)
- `tokio-serial` 5.4 - Async serial port I/O for hardware drivers
- `futures` 0.3 - Future utilities and combinators

**Protocol & Compression:**
- `lz4_flex` 0.11 - LZ4 frame compression for gRPC streaming bandwidth reduction
- `sha2` 0.10 - Hashing (JWT, auth tokens)
- `jsonwebtoken` 9.3 - JWT encode/decode for API authentication (server only)

**Hardware Integration:**
- `pvcam-sys` (internal binding in `crates/daq-driver-pvcam/pvcam-sys`) - Teledyne PVCAM SDK C bindings (optional, requires SDK installation)
- `serialport` 4.3 - Synchronous serial I/O (fallback, tokio-serial preferred)

**Scripting & Plugins:**
- `rhai` 1.19 - Embedded script language for config-driven devices
- `pyo3` 0.24 - Python integration (optional, `daq-scripting/python` feature)
- `hot-lib-reloader` 0.8 - Plugin hot-reload (optional development feature)

**Observability & Monitoring:**
- `prometheus` 0.14 - Metrics export (optional, `server/metrics` feature)
- `sysinfo` 0.37.2 - System resource monitoring

**Platform & Utilities:**
- `uuid` 1.10 - UUID generation with serde support
- `chrono` 0.4 - Timestamp/duration handling
- `url` 2 - URL parsing for daemon addresses
- `clap` 4.5 - CLI argument parsing (daq-bin only)
- `once_cell` 1.19 - Lazy statics and one-time initialization
- `mimalloc` 0.1 - Microsoft mimalloc allocator (performance-critical crates)
- `tokio-stream` 0.1 - Async stream combinators
- `http` 0.2 - HTTP types for gRPC/web
- `tower-http` 0.4 - HTTP middleware (CORS, tracing)
- `tonic-web` 0.10 - gRPC-web protocol support
- `dirs` 4.0 - Platform-specific directory locations
- `regex` 1.0 - Regular expressions
- `rand` 0.8 - Random number generation
- `evalexpr` 11 - Expression evaluation for device parameters
- `enum_dispatch` 0.3 - Fast enum dispatch without vtables

**Validation & Schemas:**
- `schemars` 0.8 - JSON schema generation
- `serde_valid` 0.24 - Serde validation framework

**WebAssembly Support (Conditional):**
- `wasm-bindgen` 0.2 - JS interop for WASM
- `web-sys` 0.3 - Web API bindings
- `getrandom` 0.2/0.3 - Random generation in WASM
- `gloo-timers` 0.3 - Timer polyfills
- `web-time` 1.1 - Timing APIs for WASM

## Configuration

**Environment Variables:**
- `PVCAM_SDK_DIR` - Path to PVCAM SDK installation (e.g., `/opt/pvcam/sdk`)
- `PVCAM_VERSION` - PVCAM SDK version string (e.g., `7.1.1.118`) - critical for Error 151 prevention
- `LIBRARY_PATH` - Build-time library search path
- `LD_LIBRARY_PATH` - Runtime library search path
- `DAQ_DAEMON_URL` - Default daemon connection URL (GUI uses this)
- `RERUN_URL` - Visualization server URL for remote streaming
- `ESP300_PORT`, `ELLIPTEC_PORT`, `NEWPORT_1830C_PORT` - Serial port overrides for hardware tests
- `POSTGRES_PASSWORD` - Database credentials (docker-compose)
- `NEO4J_PASSWORD` - Graph database credentials (docker-compose)
- `PGDATA_PATH`, `NEO4J_DATA_PATH` - Volume mount paths for containers

**Build-Time:**
- `Cargo.toml` features control compilation:
  - Storage backends: `storage_csv`, `storage_hdf5`, `storage_arrow`, `storage_parquet`, `storage_tiff`
  - Hardware drivers: `thorlabs`, `newport`, `spectra_physics`, `pvcam_sdk`, `comedi_hardware`
  - UI: `standalone` (egui), `rerun_viewer`
  - Scripting: `scripting`, `scripting_python`
  - High-level profiles: `backend`, `frontend`, `cli`, `full`, `maitai`
- `.envrc` / `.envrc.template` - direnv automatic environment setup

**Runtime Config Files:**
- `config/config.v4.toml` - gRPC server security (bind address, CORS, auth, TLS)
- `config/maitai_hardware.toml` - Hardware device registry (maitai lab configuration)
- `config/demo.toml` - Mock device configuration for demos
- `config/hosts/maitai.env` - Host-specific environment setup script
- `config/devices/*.toml` - Declarative serial device configurations (ELL14, Newport ESP300, etc.)

## Platform Requirements

**Development:**
- Rust 1.70+ (no specified MSRV, Edition 2021)
- macOS or Linux (x86_64)
- libssl-dev (OpenSSL for TLS, tokio-native-tls)

**PVCAM Hardware (Optional):**
- PVCAM SDK installation at `/opt/pvcam/sdk`
- Teledyne PVCAM version 7.1.x or 7.0.x (tested with 7.1.1.118)
- USB or PCI camera connection
- Environment variables: `PVCAM_SDK_DIR`, `PVCAM_VERSION`, `LD_LIBRARY_PATH`

**Comedi DAQ (Optional, Linux only):**
- libcomedi development files (`libcomedi-dev` on Ubuntu)
- Comedi kernel module loaded
- Requires `comedi_hardware` feature

**Database Containers (Optional):**
- Docker/Docker Compose for PostgreSQL 16 with pgvector (vector embeddings)
- Neo4j 5.14 (graph database for hardware knowledge graph)
- See `docker-compose.yml` for service definitions

**Serial Hardware (Optional):**
- libserialport development files (for native serial port support)
- tokio-serial wraps this for async I/O

**Production (gRPC Server):**
- Standalone binary `rust-daq-daemon` compiled with features: `server`, `all_hardware`, `storage_csv`
- Listens on configurable port (default 50051)
- Supports gRPC (Tonic), gRPC-web (browser clients), optional HTTP/2 TLS

---

*Stack analysis: 2026-01-21*
