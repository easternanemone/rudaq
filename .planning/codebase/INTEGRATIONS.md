# External Integrations

**Analysis Date:** 2026-01-21

## APIs & External Services

**gRPC Remote Procedure Calls:**
- Proto definition: `crates/daq-proto/proto/daq.proto`
- Service: `ControlService` (script management, measurements, status streaming)
- Supported message types: `UploadRequest`, `StartRequest`, `StreamMeasurements`, `StreamStatus`
- Framework: Tonic 0.10 (native gRPC) + tonic-web 0.10 (browser clients)
- Compression: LZ4 frame format for frame streaming (bandwidth reduction)
- TLS: Optional (config: `config/config.v4.toml`)

**Rerun 3D/2D Visualization (Optional):**
- Integration: `crates/daq-server/src/rerun_sink.rs` + `crates/daq-egui/rerun_viewer` feature
- API: Rerun RecordingStream over gRPC
- Modes:
  - Server mode: Streams to Rerun server for remote visualization
  - File mode: Records to disk (`.rrd` format)
  - Embedded: Direct integration with egui GUI
- Usage: `RerunSink::new_server()`, `RerunSink::new_file()` constructors
- Version: 0.27.3 with native viewer support
- File format: Rerun Recording Data (.rrd)

## Data Storage

**Databases:**

- **PostgreSQL 16 (Optional, Docker):**
  - Service: `postgres` container in `docker-compose.yml`
  - Extension: pgvector for vector embeddings
  - Database name: `cocoindex`
  - Connection: `postgresql://cocoindex:password@localhost:5432/cocoindex`
  - Environment: `POSTGRES_PASSWORD` (default: `cocoindex`)
  - Volume mount: `pgdata:/var/lib/postgresql/data`
  - Use case: Metadata storage, vector embeddings for hardware state
  - Health check: `pg_isready -U cocoindex`

- **Neo4j 5.14 (Optional, Docker):**
  - Service: `neo4j` container in `docker-compose.yml`
  - APOC plugin enabled for graph algorithms
  - Protocols:
    - Bolt: `bolt://localhost:7687` (native client)
    - HTTP/REST: `http://localhost:7474` (Neo4j Browser)
  - Authentication: Neo4j credentials via `NEO4J_PASSWORD` (default: `cocoindex`)
  - Volumes:
    - `neo4jdata:/data`
    - `neo4jlogs:/logs`
    - `neo4jimport:/var/lib/neo4j/import`
  - Use case: Hardware knowledge graph (relationships between devices, capabilities, serial ports)
  - Health check: Cypher shell query `RETURN 1`

**File Storage:**
- **CSV** (default): Per-acquisition CSV files written to disk
  - Location: Configurable, typically `./data/` or `/tmp/`
  - Format: RFC 4180 with timestamps
  - Implementation: `daq-storage` crate using `csv` crate

- **HDF5** (optional feature `storage_hdf5`):
  - Library: hdf5-metno 0.11.0
  - Requires: libhdf5-dev system library
  - Use: High-performance, hierarchical scientific data format
  - Implementation: `crates/daq-storage/src/hdf5.rs`

- **Apache Arrow** (optional feature `storage_arrow`):
  - Library: arrow 57
  - Format: Arrow IPC (Inter-Process Communication) for zero-copy streaming
  - Use: Columnar format, efficient for analytics
  - Implementation: `crates/daq-storage/src/arrow.rs`

- **Parquet** (optional feature `storage_parquet`):
  - Library: parquet 57 + arrow 57
  - Format: Parquet with Snappy compression
  - Use: Cloud-native columnar format
  - Implementation: `crates/daq-storage/src/parquet.rs`

- **TIFF Images** (optional feature `storage_tiff`):
  - Library: image 0.25 with TIFF feature
  - Format: Tagged Image File Format (16-bit/32-bit)
  - Use: Direct camera frame export
  - Implementation: `crates/daq-storage/src/tiff.rs`

- **Local Filesystem Only:**
  - No remote object storage (S3, GCS, etc.) integration
  - All data persisted locally to disk or Docker volumes

**Caching:**
- Memory ring buffers (in-memory only, no external cache)
  - `RingBuffer` and `AsyncRingBuffer` in `daq-pool`/`daq-storage`
  - Zero-allocation object pool for frame buffers
  - No Redis, Memcached, or external cache service

## Authentication & Identity

**Auth Provider:**
- Custom JWT HMAC (optional, disabled by default)
  - Implementation: `crates/daq-server/src/auth.rs`
  - Library: `jsonwebtoken` 9.3
  - Token format: JWT with HMAC-SHA256
  - Configuration: `config/config.v4.toml` field `auth_enabled`, `auth_token`
  - Claim fields: Default expiry, user identifier

**Security:**
- CORS (Cross-Origin Resource Sharing):
  - Configuration: `config/config.v4.toml` field `allowed_origins`
  - Default allowed: `["http://localhost:3000", "http://127.0.0.1:3000"]`
  - Implementation: `tower-http` 0.4 CORS middleware
  - Scope: gRPC-web requests only

- TLS/HTTPS (optional):
  - Configuration: `config/config.v4.toml` fields `tls_cert_path`, `tls_key_path`
  - Format: PEM-encoded certificates and private keys
  - Implementation: Native TLS via Tonic
  - Default: Disabled (uses plaintext gRPC)

- No OAuth2, SAML, or external identity provider integration

## Monitoring & Observability

**Error Tracking:**
- None (no integration with Sentry, Rollbar, DataDog, etc.)
- Errors logged via `tracing` crate

**Logs:**
- Structured logging via `tracing` 0.1 + `tracing-subscriber` 0.3
- Filtering: Environment variable `RUST_LOG` with `env-filter` directive
- Output: stderr (formatted via `tracing-subscriber`)
- No centralized log aggregation (ELK, Splunk, etc.)

**Metrics (Optional):**
- Framework: `prometheus` 0.14 (feature: `server/metrics`)
- Endpoint: HTTP `/metrics` endpoint (when metrics feature enabled)
- Transport: `hyper` 0.14 HTTP server (standalone metrics port)
- Metrics: Prometheus text format
- No integration with Grafana (but compatible)

**System Resource Monitoring:**
- Library: `sysinfo` 0.37.2
- Tracked: Memory usage, CPU, process info
- Integration: Published to `SystemStatus` gRPC message for streaming

**No external observability:**
- No OpenTelemetry integration
- No distributed tracing (Jaeger, etc.)
- No APM agent (New Relic, Datadog APM, etc.)

## CI/CD & Deployment

**Hosting:**
- Docker Compose (development/testing)
  - Defined in: `docker-compose.yml`
  - Services: PostgreSQL 16 + Neo4j 5.14
  - Network: `rust-daq-network` (bridge driver)
  - Volumes: Bind mounts to `.docker-data/`

- Standalone Binary:
  - Entrypoint: `crates/daq-bin/src/main.rs` → `rust-daq-daemon`
  - Deployment: scp/rsync to remote machine (e.g., maitai@100.117.5.12)
  - Execution: Headless daemon listening on TCP port 50051 (configurable)
  - Process manager: None (systemd/supervisor can wrap)

**CI Pipeline:**
- GitHub Actions (defined in `.github/workflows/`)
- Triggers: Push to main, PRs
- Tasks:
  - `cargo fmt --check` (formatting)
  - `cargo clippy --all-targets` (linting)
  - `cargo test` (unit tests)
  - `cargo build --release` (optimization)
- No deployment automation (manual or external scripts)

## Environment Configuration

**Required env vars:**
- PVCAM ecosystem (hardware machines only):
  - `PVCAM_SDK_DIR` - Path to SDK installation
  - `PVCAM_VERSION` - SDK version string (critical for preventing Error 151)
  - `LD_LIBRARY_PATH` - Runtime library search
  - `LIBRARY_PATH` - Build-time library search

- Development:
  - `DAQ_DAEMON_URL` - Default daemon URL for clients
  - `RERUN_URL` - Visualization server endpoint
  - `RUST_LOG` - Tracing filter level

- Docker database:
  - `POSTGRES_PASSWORD` - Database password
  - `NEO4J_PASSWORD` - Graph DB password

**Secrets location:**
- Environment variables (`.env` file, not committed)
- Configuration files (e.g., `config/config.v4.toml` with auth_token)
- No secrets management service (HashiCorp Vault, AWS Secrets Manager, etc.)
- direnv (`.envrc`) for automatic loading on cd

## Webhooks & Callbacks

**Incoming:**
- None. No webhook endpoints defined.
- Only bidirectional gRPC streaming from client to daemon.

**Outgoing:**
- None. Daemon does not call external webhooks.
- Rerun visualization: One-way streaming to Rerun server (push model, no callbacks).

## Hardware Integrations (Serial/USB)

**Teledyne PVCAM Camera:**
- SDK: pvcam-sys (internal C bindings in `crates/daq-driver-pvcam/pvcam-sys`)
- Version: PVCAM 7.0.x, 7.1.x
- Connection: USB or PCI camera interface
- Driver implementation: `crates/daq-driver-pvcam/src/`
- Capabilities: Frame streaming, exposure control, emission control
- Features: `pvcam_hardware` (requires SDK installation)
- Environment: `PVCAM_SDK_DIR`, `PVCAM_VERSION`

**Newport ESP300 Motion Controller:**
- Protocol: Serial ASCII, 19200 baud
- Driver: `crates/daq-driver-newport/src/`
- Multi-axis support: 1-3 axes
- Serial port: `/dev/ttyUSB0` (typical maitai)
- Capabilities: Absolute positioning, velocity control
- Feature: `newport`

**Newport 1830-C Power Meter:**
- Protocol: Serial ASCII (NOT SCPI), 9600 baud
- Driver: `crates/daq-driver-newport/src/power_meter/`
- Serial port: `/dev/ttyS0` (typical maitai)
- Capabilities: Power reading, wavelength configuration
- Feature: `newport_power_meter`

**Spectra-Physics MaiTai Laser:**
- Protocol: Serial ASCII, 115200 baud, no flow control
- Driver: `crates/daq-driver-spectra-physics/src/`
- Serial port: `/dev/ttyUSB5` (typical maitai)
- Capabilities: Wavelength tuning, emission on/off, shutter control
- Features: `spectra_physics`, `WavelengthTunable`, `ShutterControl`, `EmissionControl`

**Thorlabs ELL14 Rotation Mount (RS-485 Bus):**
- Protocol: Serial hex-encoded commands, 9600 baud, RS-485 multidrop
- Driver: `crates/daq-driver-thorlabs/src/ell14/`
- Serial port: `/dev/ttyUSB1` (typical maitai)
- Addressing: Device addresses 0-15 on shared bus
- Capabilities: Absolute positioning, homing
- Feature: `thorlabs`
- Bus management: `Ell14Bus` provides calibrated device handles

**Comedi Data Acquisition (Linux only):**
- System: Linux kernel Comedi subsystem
- Library: libcomedi
- Driver: `crates/daq-driver-comedi/src/`
- Features: `comedi`, `comedi_hardware`
- Analog input/output, digital I/O support
- Platform: Linux x86_64 only

**VISA (Virtual Instrument Software Architecture):**
- Optional library: `visa-rs` 0.5 (feature: `visa`)
- Not currently used (available for future integration with VISA-compatible instruments)

## Third-Party Scripting & Plugins

**Rhai Script Engine:**
- Version: 1.19 with `sync` feature
- Use: Config-driven device definitions, automation scripts
- Execution: Sandboxed, no filesystem access by default
- Implementation: `crates/daq-scripting/src/`
- No external script repositories or package management

**Python Integration (Optional):**
- Library: PyO3 0.24 with `auto-initialize` feature
- Feature: `scripting_python` (off by default)
- Use: Python scripts as alternative to Rhai
- Implementation: `crates/daq-scripting/src/python.rs`
- CPython: No version pinning (uses system Python)

**Plugin Hot-Reload (Development Only):**
- Library: `hot-lib-reloader` 0.8, `notify` 7
- Feature: `plugins_hot_reload`
- Use: Reload native plugins without daemon restart
- File watcher: `notify` crate monitors `.so`/`.dylib` changes

**Native Plugin Loading (ABI-stable):**
- Library: `abi_stable` 0.11.3
- Plugin API: `crates/daq-plugin-api/src/`
- Format: Compiled `.so`/`.dylib` with stable C ABI
- No plugin registry or marketplace

## Data Format Exports

**Frame/Image Streaming:**
- Format: Protobuf 3 (binary) over gRPC
- Compression: LZ4 frame encoding (optional `StreamQuality` levels)
- Quality modes:
  - `Full`: No downsampling (100% bandwidth)
  - `Preview`: 2x2 binning (~75% reduction)
  - `Fast`: 4x4 binning (~94% reduction)
- Backpressure handling: Frame skipping when gRPC buffer >75% full

**Measurement Data Streaming:**
- Format: Protobuf 3 (binary) over gRPC
- Schema: `daq.proto` messages `DataPoint`, `SystemStatus`
- Real-time vs. recorded: Both supported via `StreamMeasurements` RPC

---

*Integration audit: 2026-01-21*
