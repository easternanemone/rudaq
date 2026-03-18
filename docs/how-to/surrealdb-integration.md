# SurrealDB Integration Guide

> **Comprehensive documentation for the embedded SurrealDB persistence layer in rust-daq.**
>
> This guide covers architecture, database internals, the reconciler system,
> and deployment operations. It is the authoritative reference for the
> `db-surreal` feature family.
>
> **Contributing models**: Gemini 3 Pro, GPT-5.2 Codex, Claude Sonnet 4.5,
> Gemini 2.5 Flash --- assembled and verified by Claude Opus 4.6.

---

## Table of Contents

1. [Architecture & Design](#1-architecture--design)
2. [Database Layer](#2-database-layer)
3. [Reconciler & Watch System](#3-reconciler--watch-system)
4. [gRPC ConfigService](#4-grpc-configservice)
5. [Deployment & Operations](#5-deployment--operations)
6. [Quick Reference](#6-quick-reference)

---

## 1. Architecture & Design

### 1.1 System Overview

The DAQ system employs a **Control Plane / Data Plane** separation, bridging
persistent configuration (Desired State) with live hardware runtime (Observed
State). This architecture decouples experiment definition from physical device
instantiation, enabling robust error handling, hot-reloading, and zero-ops
deployment via an embedded database.

```
   [ User / CLI ]          [ gRPC Clients ]
         |                         |
         v                         v
+---------------------------------------------------+
|                  APPLICATION BINARY               |
|                                                   |
|  +-------------------+     +-------------------+  |
|  |     PLANE A       |     |      PLANE B      |  |
|  |  (Control Plane)  |<===>|   (Data Plane)    |  |
|  |    SurrealDB      |     |   DeviceRegistry  |  |
|  |  (Desired State)  |     |  (Observed State) |  |
|  +-------------------+     +-------------------+  |
|            ^                         |            |
|            | (Reconciler Loop)       | (Drivers)  |
|            v                         v            |
+---------------------------------------------------+
             |                         |
      [ File System ]         [ Physical Hardware ]
      (RocksDB Files)         (Lasers, Stages, etc)
```

### 1.2 The Two-Plane Model

The core pattern is a **Kubernetes-style Reconciliation Loop**. Users modify
*Desired State* in the database; a background reconciler converges *Observed
State* to match.

**Plane A: Control Plane (SurrealDB)**

- **Role**: Source of truth for configuration, topology, and intent.
- **Persistence**: Embedded RocksDB (production) or in-memory (testing).
- **Entities**: `instrument`, `driver`, `experiment`.
- **Characteristics**: Transactional, queryable, graph-capable.
- **Source**: `crates/db/src/lib.rs:1`

**Plane B: Data Plane (DeviceRegistry)**

- **Role**: Runtime environment for active hardware drivers.
- **Persistence**: Volatile (RAM).
- **Entities**: `DashMap<String, Arc<dyn Driver>>` with trait objects,
  broadcast channels for 100 Hz+ measurement streams.
- **Characteristics**: Low-latency, lock-free reads, high concurrency.

**The Reconciler Bridge** (`crates/bin/src/reconciler.rs`):
1. **Watch**: Listens for changes in Plane A via LIVE SELECT (or polls).
2. **Diff**: Compares Plane A (Desired) vs Plane B (Observed).
3. **Act**: Add missing devices, remove extras, reconfigure changed configs.

### 1.3 Source of Truth per Field

| Field                | Source   | Rationale                              |
|----------------------|----------|----------------------------------------|
| `instrument.config`  | DB       | User-authored, persisted across runs   |
| `instrument.enabled` | DB       | Desired state -- survives restart      |
| `instrument.status`  | Memory   | Observed from hardware, volatile       |
| `driver_type`        | DB       | Part of configuration, immutable-ish   |
| Measurements         | Memory   | High-frequency, broadcast via channels |

### 1.4 Crate Dependency Graph

```
common ──────► hardware ──────► bin (application)
  │                               ▲
  └──────────► db ────────────────┘
```

| Crate | Role | Key Components |
|-------|------|----------------|
| `common` | Shared types | `MeasurementLock`, `StateCache`, `Capabilities` |
| `hardware` | Driver logic | `DeviceRegistry`, driver traits, factory pattern |
| `db` | Storage engine | SurrealDB wrapper, schema, config store |
| `bin` | Integration | Reconciler, daemon, CLI, config bridge |

**Dependency rule**: `db` cannot depend on `hardware` (would create a cycle).
Conversions between hardware config types and DB record types live in `bin`
(`crates/bin/src/db_bridge.rs`).

### 1.5 Feature Flag Architecture

SurrealDB is a heavy dependency. The `kv-mem` (in-memory) engine is
enabled by default so that every `cargo build` and `cargo test` includes
the database layer — matching the production topology. The `kv-rocksdb`
engine remains opt-in since it requires native C++ dependencies.

Use `--no-default-features` to produce a DB-less build when needed
(e.g., lightweight driver development).

```
db crate features:
  kv-mem     (DEFAULT) ──► SurrealDB in-memory backend
  kv-rocksdb           ──► SurrealDB RocksDB backend

bin crate features:
  db-surreal           ──► Meta-feature, activates DB modules
  db-surreal-mem (DEFAULT) ──► In-memory engine (tests/dev)
  db-surreal-rocksdb   ──► RocksDB engine (production)
  production           ──► RocksDB + modules + all_hardware

server crate features:
  db-surreal           ──► ConfigService gRPC handler
  db-surreal-mem       ──► In-memory engine (activated via bin defaults)
  db-surreal-rocksdb   ──► RocksDB engine (production)
```

| Feature | Engine | Use Case | Binary Impact |
|---------|--------|----------|---------------|
| *(default)* | In-memory | Dev, tests, CI | Baseline (+~8 MB vs no-DB) |
| `db-surreal-rocksdb` | RocksDB | Production | +~15 MB |
| `--no-default-features` | None | TOML-only deployment | Smallest |

All DB modules in the `db` crate are gated behind
`#[cfg(any(feature = "kv-mem", feature = "kv-rocksdb"))]`
(`crates/db/src/lib.rs:64`). These guards remain valid for
`--no-default-features` builds.

All DB integration code in the `bin` crate is gated behind
`#[cfg(feature = "db-surreal")]` (`crates/bin/src/main.rs:34`).

### 1.6 Safety Model

Given control of physical hardware (lasers, motion stages), safety takes
precedence over consistency.

**MeasurementLock**: A per-device lock that prevents hot-swap reconfiguration
during active measurements. The reconciler checks `registry.is_device_idle()`
before applying config changes (`crates/bin/src/reconciler.rs:158`). gRPC
streaming sets/clears the lock around device reads
(`crates/server/src/grpc/hardware_service/mod.rs`).

**Shutdown Sequence** (safety-critical ordering):
1. **Server Stop** -- prevent new requests
2. **Storage Flush** -- persist buffered data
3. **Hardware Shutdown** -- return devices to safe state

This ordering is enforced by `DaemonInstance::shutdown()`
(`crates/bin/src/daemon_manager.rs:439`) and verified by contract tests.

**Hardware Watchdog**: Fires emergency shutdown if the tokio runtime hangs.
Runs on a separate OS thread; kicks from the registry monitor task.

**Panic Hook**: `ShutterRegistry::install_panic_hook_with_hardware()` ensures
all shutters close and lasers power down on any panic.

### 1.7 Design Decisions

**Why Embedded SurrealDB?**
- Zero-ops: Lab machines are often air-gapped or managed by non-IT staff.
  No Docker, no systemd service for the DB -- it compiles into the single
  binary.
- Graph capabilities: Lab setups are topological graphs (Signal -> Filter ->
  Detector). SurrealDB's native graph edges enable queries like "all
  instruments downstream of Laser A."

**Why the Reconciler Pattern?**
- Resilience: If a USB cable is unplugged and replugged, the reconciler
  detects the driver failure and re-initializes automatically from desired
  state.
- Declarative config: Loading a preset is replacing desired state; the
  reconciler handles the complex diffing to transition hardware.

**Why Feature Flags?**
- The `db` crate pulls in SurrealDB (~8 MB for kv-mem, ~15 MB for kv-rocksdb).
  The in-memory engine (`kv-mem`) is now default so dev builds match production
  topology. The heavier `kv-rocksdb` engine (with C++ bindings) remains opt-in.
- `--no-default-features` still produces a DB-less build for lightweight driver
  development when DB overhead is unwanted.

---

## 2. Database Layer

### 2.1 DaqDb Initialization & Lifecycle

The core database interaction is encapsulated in `DaqDb`
(`crates/db/src/core.rs:70`), a thread-safe handle wrapping
`Arc<Surreal<Any>>`.

**Initialization** (`DaqDb::init()`, `core.rs:84`):
1. Connects to the SurrealDB endpoint (`mem://` or `rocksdb://<path>`).
2. Selects namespace (`daq`) and database (`live`).
3. Applies schema migrations via `schema::apply_schema()`.
4. Records wall-clock startup time for uptime reporting.

```rust
let db = DaqDb::init(DbConfig::in_memory()).await?;
// or
let db = DaqDb::init(DbConfig::rocksdb("/var/lib/rust-daq/db")).await?;
```

`DaqDb` is cheap to clone (wraps an `Arc`) and is distributed to all
components needing DB access.

**Utility methods**:
- `health_check()` (`core.rs:124`): Returns `true` if the DB responds to
  `RETURN true`.
- `info()` (`core.rs:133`): Returns `DbInfo` with engine type, schema
  version, uptime, record counts, and health status.
- `count_table()` (`core.rs:169`): Counts records in a table using an
  **allow-list** to prevent SurrealQL injection (allowed tables: `driver`,
  `instrument`, `experiment`).

### 2.2 Schema Migration System

Schema management lives in `crates/db/src/schema.rs`. The system uses
**forward-only, idempotent migrations**.

**Key constants**:
- `SCHEMA_VERSION = 8` (`schema.rs:15`)
- `MIGRATIONS`: Array of `Migration { version, sql }` structs.

**Migration chain**:
- **v1**: Core tables (`schema_version`, `driver`, `instrument`,
  `experiment`) plus relation tables (`instance_of`, `connects_to`).
  All using `DEFINE TABLE IF NOT EXISTS` for idempotency.
- **v2**: No-op structural migration (establishes the chain pattern).
- **v3**: Extend `experiment` table with `experiment_id` key. (Topology
  tables were removed as dead code — version number preserved for RocksDB
  database compatibility.)
- **v4**: Add `commands` array field to `driver` for DB-backed command
  introspection in universal/TOML drivers.
- **v5**: Experiment plan storage (`experiment_plan`) and run history
  (`run_record`). Graph edges: `executed_from` (run→plan) and
  `uses_instrument` (plan→instrument).
- **v6**: Device feature metadata cache (`device_feature`) for offline UI
  parameter rendering.
- **v7**: Heartbeat monitoring and device runtime state persistence.
  Adds `heartbeat_at` field to `run_record` for crash detection (stale
  heartbeat indicates a daemon crash mid-experiment). Adds
  `device_runtime_state` table for persisting last-known parameter values
  across restarts. Also fixes `commands` field on `driver` to have
  `DEFAULT []` (v4 omitted this, causing silent CREATE failures on
  SCHEMAFULL tables in SurrealDB 2.x).
- **v8**: Add `is_favorite` flag to `device_runtime_state` for UI
  quick-access pinning (bd-4wf7).

**`apply_schema()` algorithm** (`schema.rs:131`):
1. Query `schema_version:current` (fixed record ID) to get current version.
2. If `current_version >= SCHEMA_VERSION`, return early (up to date).
3. Iterate `MIGRATIONS`, apply each where `version > current_version`.
4. `UPSERT schema_version:current SET version = $version` (prevents
   accumulation of version records across restarts).

### 2.3 Config Store CRUD

`crates/db/src/config_store.rs` provides CRUD operations for hardware
configuration records. Types are DB-native (no `hardware` crate dependency).

**Data models**:
- `DbDriver` (`config_store.rs:18`): `driver_type`, `name`, `capabilities`
- `DbInstrument` (`config_store.rs:29`): `device_id`, `name`, `driver_type`,
  `config` (JSON), `enabled`
- `InstrumentSummary` (`config_store.rs:53`): Lightweight projection
  (no config blob)

**Operations**:

| Method | SurrealQL Pattern | Notes |
|--------|------------------|-------|
| `upsert_instruments()` | `UPSERT ... WHERE device_id = $device_id` | Idempotent, single LIVE SELECT notification |
| `get_all_instruments()` | `SELECT ... ORDER BY device_id` | Returns full config |
| `get_instrument()` | `SELECT ... WHERE device_id = $device_id` | Single record |
| `list_instruments()` | `SELECT device_id, name, ... ORDER BY device_id` | No config blob |
| `delete_instrument()` | `DELETE ... RETURN BEFORE` | Atomic existence check |
| `upsert_drivers()` | `UPSERT ... WHERE driver_type = $driver_type` | Idempotent |
| `get_all_drivers()` | `SELECT ... ORDER BY driver_type` | Full records |

All operations use **parameterized queries** (`bind()`) for safety.

### 2.4 LIVE SELECT Streaming

`live_instruments()` (`config_store.rs:201`) returns a
`Stream<Item = Result<Notification<DbInstrument>, Error>>` that emits
real-time notifications for create, update, and delete events on the
`instrument` table.

This powers the watch reconciler (Section 3) -- LIVE SELECT events trigger
`reconcile_once()` with debouncing, providing sub-second response to
configuration changes.

### 2.5 Security Measures

- **Parameterized queries**: All user data flows through `bind()` parameters,
  preventing SurrealQL injection.
- **Allow-list for table names**: `count_table()` (`core.rs:172`) validates
  the table name against a static allow-list before string interpolation.
  SurrealDB does not support parameterized table names.
- **Embedded model**: The DB runs in-process with no network listener,
  eliminating the network attack surface.
- **Schema enforcement**: `SCHEMAFULL` tables reject unexpected fields.

### 2.6 TOML <-> JSON Conversion

`config_store.rs` includes `toml_to_json()` and `json_to_toml()` for
bidirectional conversion between human-friendly TOML config files and
SurrealDB's JSON storage.

- **Import path**: TOML file -> `toml::Value` -> `toml_to_json()` ->
  `serde_json::Value` -> DB `config` field.
- **Export path**: DB `config` field -> `serde_json::Value` ->
  `json_to_toml()` -> `toml::Value` -> TOML string.
- **Edge case**: JSON `null` maps to empty TOML string (TOML has no null).

### 2.7 Error Handling

`DbError` (`crates/db/src/error.rs:4`) provides granular error variants:

| Variant | Meaning |
|---------|---------|
| `Database(String)` | SurrealDB client/server errors |
| `Migration(String)` | Schema migration failures |
| `Serde` | Serialization/deserialization errors |
| `NotInitialized` | DB used before `init()` |
| `UpsertFailed { table, key, reason }` | Specific upsert failure |
| `QueryFailed { query, reason }` | Raw query failure |
| `TransactionAborted(String)` | Transaction rollback |
| `ReconcileFailed(String)` | Reconciliation logic errors |

`From<surrealdb::Error>` is implemented for `DbError`, but only when a
storage engine feature is enabled (`error.rs:43`).

### 2.8 Parameter State Persistence (bd-4wf7)

The `device_runtime_state` table (added in schema v7) persists last-known
parameter values across daemon restarts. This enables a "pick up where you
left off" experience — when the daemon restarts, devices are restored to
their previous positions, exposure times, wavelengths, etc.

**Schema** (`schema.rs`, v7 + v8 migrations):

```sql
DEFINE TABLE device_runtime_state SCHEMAFULL;
DEFINE FIELD device_id   ON device_runtime_state TYPE string;
DEFINE FIELD param_name  ON device_runtime_state TYPE string;
DEFINE FIELD param_value ON device_runtime_state FLEXIBLE TYPE any;
DEFINE FIELD updated_at  ON device_runtime_state TYPE datetime DEFAULT time::now();
DEFINE FIELD is_favorite ON device_runtime_state TYPE bool DEFAULT false;  -- v8
DEFINE INDEX idx_device_param ON device_runtime_state FIELDS device_id, param_name UNIQUE;
```

**Write path — debounced writer** (`crates/server/src/grpc/hardware_service/mod.rs`):

When a `HardwareServiceImpl` is created with `with_db()`, it spawns a
background task that subscribes to parameter change events via a broadcast
channel. The writer collects dirty parameters into a batch and flushes to
`device_runtime_state` via `batch_upsert_device_state()` every 2 seconds.

Only **writable** parameters from **user/script** sources are persisted —
read-only hardware telemetry (e.g., temperature readings) is excluded to
avoid unnecessary DB churn.

**Read path — startup restore** (`crates/bin/src/daemon_manager.rs`):

On daemon startup, after the initial reconciliation populates the
DeviceRegistry, `restore_parameter_state()` reads all `device_runtime_state`
entries and applies them to devices via `Parameter::set_json()`. If a value
is rejected by the driver (e.g., a constraint violation after a config
change), the error is logged and the device keeps its hardware default.

The startup sequence is:
1. Parse TOML → create devices in DeviceRegistry
2. Shadow-write to SurrealDB
3. Initial reconcile
4. **`restore_parameter_state()`** — read `device_runtime_state`, call
   `Parameter::set_json()` on each device

**Favorites** (schema v8, bd-4wf7):

The `is_favorite` boolean field allows the UI to "pin" frequently-used
parameters for quick access. This state is persisted in the same
`device_runtime_state` table alongside parameter values.

---

## 3. Reconciler & Watch System

This subsystem keeps SurrealDB (desired state) and the DeviceRegistry
(observed state) consistent in a safe, deterministic, and debounced manner.

### 3.1 Reconciliation Algorithm

`reconcile_once()` (`crates/bin/src/reconciler.rs:104`) performs a single
idempotent pass:

```
1. Read desired state ──► DB: all enabled instruments
2. Read observed state ──► Registry: list_devices()
3. Compute diff:
   ├── REMOVE: in registry but not desired
   ├── ADD: in desired but not in registry
   ├── UPDATE: config hash changed
   └── UNCHANGED: hash matches
4. Apply changes:
   ├── REMOVE ──► registry.unregister()
   ├── ADD ──► registry.register(device_config)
   └── UPDATE ──► Reconfigurable.reconfigure() || unregister+register
5. Return ReconcileReport
```

**Safety checks before UPDATE** (`reconciler.rs:158`):
- `registry.is_device_idle()` must be true (MeasurementLock check).
- If device is measuring, the update is deferred (counted as unchanged).

**Missing factory detection** (`reconciler.rs:196`):
- Before attempting to add a device, checks `registry.has_factory()`.
- Missing factories produce clear error messages (not silent failures).

### 3.2 Config Change Detection

The reconciler uses **canonical JSON hashing** to detect config changes
without false positives from key ordering:

```rust
fn canonical_json(v: &serde_json::Value) -> String {
    // Recursively sorts object keys, uses serde_json::to_string(k) for
    // proper key escaping (handles quotes, backslashes, control chars).
    // Arrays preserve order. Primitives use v.to_string().
}
```

(`reconciler.rs:41`)

The resulting string is hashed with `DefaultHasher`. Hashes are only compared
within a single process run (not persisted), so `DefaultHasher` instability
across Rust versions is acceptable.

### 3.3 Watch Reconciler

`crates/bin/src/watch_reconciler.rs` provides reactive reconciliation via
SurrealDB LIVE SELECT.

**Architecture**:

```
LIVE SELECT on instrument table
         │
         ▼
   ┌─────────────┐
   │  Debounce    │  200ms window, 2s max wait (anti-starvation)
   │  Timer       │
   └──────┬──────┘
          │  fires
          ▼
   reconcile_once()
```

**Constants** (`watch_reconciler.rs`):

| Constant | Value | Purpose |
|----------|-------|---------|
| `DEFAULT_DEBOUNCE` | 200 ms | Coalesce rapid changes |
| `DEFAULT_MAX_DEBOUNCE_WAIT` | 2 s | Prevent starvation under sustained load |
| `INITIAL_RETRY_BACKOFF` | 5 s | First retry delay after LIVE SELECT failure |
| `MAX_RETRY_BACKOFF` | 60 s | Cap on exponential backoff |
| `DEFAULT_RESYNC_INTERVAL` | 300 s | Periodic full resync (k8s pattern) |
| `CIRCUIT_BREAKER_THRESHOLD` | 5 | Consecutive failures before error-level logging |

**Resilience patterns**:

1. **Exponential backoff with jitter**: On LIVE SELECT failure, retries with
   `min(initial * 2^n, 60s)` plus random jitter (xorshift PRNG, no `rand`
   dependency). Jitter prevents thundering herd after network recovery
   (`watch_reconciler.rs:280`).

2. **Circuit breaker**: After 5 consecutive failures, logging escalates from
   `warn!` to `error!` (`watch_reconciler.rs:159`). This ensures operational
   visibility without masking transient issues.

3. **Periodic resync**: Every 5 minutes, a full `reconcile_once()` runs
   regardless of LIVE SELECT events. This is a safety net for missed
   notifications (k8s resync period pattern).

4. **Initial resync on reconnect**: Every time the LIVE SELECT stream is
   (re-)established, a full reconcile runs before processing events.

5. **Graceful shutdown**: `CancellationToken` ensures clean exit at any point
   in the watch loop.

**Anti-starvation**: The `max_debounce_wait` cap (`watch_reconciler.rs:27`)
ensures that sustained rapid-fire notifications cannot indefinitely defer a
reconcile. The debounce deadline is clamped to `min(now + debounce, max_deadline)`.

### 3.4 Config Bridge

`crates/bin/src/db_bridge.rs` bridges the `hardware` and `db` crates
(which cannot depend on each other).

| Function | Direction | Purpose |
|----------|-----------|---------|
| `devices_to_db()` | HardwareConfig -> Vec<DbInstrument> | Import |
| `drivers_from_config()` | HardwareConfig -> Vec<DbDriver> | Import |
| `db_to_hardware_config()` | Vec<DbInstrument> -> HardwareConfig | Export |
| `db_to_hardware_toml()` | Vec<DbInstrument> -> TOML string | CLI export |
| `shadow_write()` | HardwareConfig -> DB (non-fatal) | Daemon startup |

**Shadow write** (`db_bridge.rs:86`): On daemon startup, the parsed TOML
hardware config is mirrored into SurrealDB. This is non-fatal -- if it fails,
the daemon continues from TOML. The shadow copy provides:
- A queryable representation of the config for tooling.
- A baseline for the reconciler to detect drift.
- Audit capability (who changed what, when).

### 3.5 Daemon Startup Integration

The daemon startup sequence (`crates/bin/src/daemon_manager.rs:133`)
integrates the DB in a carefully ordered, fault-tolerant manner:

1. **DB init** (non-fatal, `daemon_manager.rs:149`):
   If the DB fails to initialize, the daemon logs a warning and continues.
   The `db` field is set to `None`.

2. **Shadow-write** (`daemon_manager.rs:285`):
   If both DB and HardwareConfig are available, mirrors TOML -> DB.
   Requires both `db-surreal` and `networking` features.

3. **Initial reconcile** (`daemon_manager.rs:305`):
   Runs `reconcile_once()` to catch DB-only instruments (added via CLI
   between restarts). Non-fatal.

4. **Watch reconciler** (`daemon_manager.rs`):
   Spawns the LIVE SELECT watch reconciler task. This background task
   listens for DB changes and triggers `reconcile_once()` with debouncing,
   enabling runtime hardware hot-swap. Uses the daemon's `CancellationToken`
   for graceful shutdown.

### 3.6 CLI Config Commands

**Offline commands** (direct DB access, no running daemon required):

```bash
# Import TOML hardware config into database
rust-daq config import <file> [--db-path <path>]

# Export database contents as TOML (stdout)
rust-daq config export [--db-path <path>]

# List instruments in database
rust-daq config list [--db-path <path>]
```

If `--db-path` is omitted, an in-memory database is used (useful for
validation without touching production data).

**Online commands** (talk to running daemon via gRPC):

```bash
# List instruments in the running daemon's database
rust-daq client config-list [--addr localhost:50051]

# Get details for a specific instrument
rust-daq client config-get <device_id> [--addr localhost:50051]

# Import a TOML config file (triggers watch reconciler → hot-swap)
rust-daq client config-import <file.toml> [--addr localhost:50051]

# Export current DB config as TOML
rust-daq client config-export [--addr localhost:50051]

# Delete an instrument (triggers watch reconciler → device removal)
rust-daq client config-delete <device_id> [--addr localhost:50051]

# Show database info (engine, schema version, health)
rust-daq client config-info [--addr localhost:50051]

# Stream live config changes (LIVE SELECT events)
rust-daq client config-watch [--addr localhost:50051]
```

Online commands connect to the daemon's ConfigService gRPC endpoint.
Changes made via `config-import` or `config-delete` are automatically
detected by the watch reconciler and applied to hardware within ~200ms.

### 3.7 Testing Strategy

**Unit tests** (`reconciler.rs` tests):
- `test_reconcile_adds_missing_devices`: DB instrument -> registry.
- `test_reconcile_removes_extra_devices`: Orphan in registry -> removed.
- `test_reconcile_idempotent`: Second pass is a no-op.
- `test_reconcile_skips_disabled_instruments`: Disabled -> not added.
- `test_reconcile_reports_missing_factory`: Clear error message.
- `test_reconcile_updates_changed_config`: Hash change -> re-register.
- `test_reconcile_defers_when_measuring`: MeasurementLock safety.
- `test_canonical_json_key_order_independent`: Hash stability.
- `test_canonical_json_escapes_special_keys`: Key escaping correctness.

**E2E tests** (`reconciler.rs` tests):
- `test_e2e_db_to_game_loop_broadcast`: DB -> reconciler -> registry ->
  state poller -> game loop -> broadcast snapshot.
- `test_e2e_watch_to_readable`: LIVE SELECT -> watch reconciler -> registry
  -> device is Readable and returns a value.
- `test_end_to_end_db_to_registry`: Add/delete in DB -> reconcile ->
  verify registry state.
- `test_e2e_watch_detects_delete`: Insert -> verify in registry -> delete
  from DB -> watch reconciler removes from registry.
- `test_e2e_watch_full_lifecycle`: Insert -> Update -> Delete through
  the full LIVE SELECT -> debounce -> reconcile chain.
- `test_e2e_grpc_config_hot_swap`: Full gRPC pipeline: UpsertInstrument
  via ConfigService -> watch reconciler -> device appears in
  HardwareService.ListDevices -> DeleteInstrument -> device removed.
- `test_e2e_measurement_lock_defers_reconfig`: Watch reconciler defers
  reconfiguration while MeasurementLock::Measuring is set, applies
  change after lock release via periodic resync.
- `test_e2e_concurrent_upserts_converge`: 10 simultaneous upserts
  verify no panics, no data loss, DB+registry convergence.

**Watch reconciler tests** (`watch_reconciler.rs` tests):
- `test_live_instruments_stream`: LIVE SELECT emits notifications.
- `test_watch_reconciler_processes_change`: Insert -> device appears.
- `test_watch_reconciler_debounces_bulk_changes`: 5 rapid inserts ->
  all 5 devices registered (debounced into one reconcile).
- `test_watch_reconciler_shutdown`: CancellationToken -> clean exit.

**ConfigService tests** (`config_service.rs` tests):
- 13 unit tests covering all ConfigService RPCs (list, get, upsert,
  delete, import, export, drivers, db-info, roundtrips, edge cases).

**Contract tests** (`daemon_manager.rs` tests):
- `test_shutdown_log_matches_contract`: Runtime shutdown matches
  `SHUTDOWN_PHASE_ORDER` constant.

---

## 4. gRPC ConfigService

The ConfigService provides runtime access to the SurrealDB control plane
via gRPC. Changes made through this service are automatically detected by
the watch reconciler and applied to hardware.

### 4.1 Runtime Config Workflow

```
User / CLI / GUI
      │
      ▼
 ConfigService (gRPC)
      │
      ▼
 SurrealDB (control plane)
      │  LIVE SELECT notification
      ▼
 Watch Reconciler (debounce 200ms)
      │
      ▼
 reconcile_once()
      │
      ├── ADD: create driver via factory → register in DeviceRegistry
      ├── UPDATE: Reconfigurable.reconfigure() or unregister+register
      └── REMOVE: unregister from DeviceRegistry
      │
      ▼
 HardwareService (gRPC)  →  hardware responds immediately
```

Typical latency from gRPC call to hardware change: **~300ms** (200ms
debounce + reconcile overhead).

### 4.2 ConfigService RPCs

| RPC | Description | Method |
|-----|-------------|--------|
| `ListInstruments` | List all instruments in DB | `GET` |
| `GetInstrument` | Get a single instrument by device_id | `GET` |
| `UpsertInstrument` | Create or update an instrument | `PUT` |
| `DeleteInstrument` | Delete an instrument | `DELETE` |
| `ListDrivers` | List registered driver types (read-only) | `GET` |
| `ImportConfig` | Import a TOML config string into DB | `PUT` |
| `ExportConfig` | Export DB contents as TOML string | `GET` |
| `GetDbInfo` | Database engine, schema version, health | `GET` |
| `SubscribeConfigChanges` | Stream live change events | `SSE` |

Implementation: `crates/server/src/grpc/config_service.rs`

### 4.3 Hot-Swap Examples

**Add a device at runtime** (using grpcurl):
```bash
grpcurl -plaintext -d '{
  "instrument": {
    "device_id": "power_meter_2",
    "name": "Second Power Meter",
    "driver_type": "mock_power_meter",
    "config_json": "{}",
    "enabled": true
  }
}' localhost:50051 daq.ConfigService/UpsertInstrument
```

**Verify it appeared** (within ~300ms):
```bash
grpcurl -plaintext localhost:50051 daq.HardwareService/ListDevices
```

**Remove it**:
```bash
grpcurl -plaintext -d '{"device_id": "power_meter_2"}' \
  localhost:50051 daq.ConfigService/DeleteInstrument
```

**Watch changes in real time**:
```bash
grpcurl -plaintext localhost:50051 daq.ConfigService/SubscribeConfigChanges
```

### 4.4 MeasurementLock Safety

When a device is actively measuring (`MeasurementLock::Measuring`), the
reconciler **defers** reconfiguration. The config change is recorded in
the DB but not applied to hardware until the measurement completes.

This prevents:
- Laser power changes during exposure
- Stage movement during frame acquisition
- Sensor recalibration during data collection

The deferred change is applied automatically on the next reconcile cycle
after the lock is released (either via the periodic resync timer or the
next LIVE SELECT notification).

Key code path: `reconciler.rs:157-165` — `is_device_idle()` check.

---

## 5. Deployment & Operations

### 5.1 Build Configurations

```bash
# Default: includes in-memory SurrealDB
cargo build

# Production: RocksDB persistence
cargo build --release -p bin --features production

# Add RocksDB engine (kv-mem is also included via defaults):
cargo build --release -p bin --features db-surreal-rocksdb

# No DB (TOML-only, fast compilation for driver work)
cargo build -p bin --no-default-features --features networking,server
```

| Configuration | Tests | DB Features | Binary Size |
|---------------|-------|-------------|-------------|
| Default (`db-surreal-mem`) | 2135 | In-memory | Baseline |
| `production` / `db-surreal-rocksdb` | 2135 | RocksDB | +~7 MB |
| `--no-default-features` | 2076 | None | Smallest |

**Note**: The `production` feature unions with the default `kv-mem`, compiling
both engines. This is safe — the `Any` engine selects at runtime based on the
connection string (`mem://` vs `rocksdb://path`). Binary size increases by ~7 MB.

### 5.2 Deployment Scenarios

**Development**:
```bash
cargo run -p bin -- daemon \
  --port 50051 \
  --hardware-config config/dev_hardware.toml
```
- DB resets on every restart (in-memory).
- Fast startup (~200 ms DB init).
- Ideal for TDD workflows.

**Testing / CI**:
```bash
cargo nextest run                             # 2135 tests (DB included by default)
cargo nextest run --no-default-features       # 2076 tests (no DB)
```
- RocksDB persistence tests use **subprocess isolation** (separate OS
  process writes, exits, reader process verifies). Must run serially.

**Production** (maitai lab machine):
```bash
cargo build --release -p bin --features db-surreal-rocksdb
./target/release/rust-daq-daemon daemon \
  --port 50051 \
  --hardware-config /etc/rust-daq/maitai_universal.toml \
  --db-path /var/lib/rust-daq/db
```

Recommended directory structure:
```
/var/lib/rust-daq/
  db/              # RocksDB data directory
  backups/         # DB snapshots
/etc/rust-daq/
  maitai_universal.toml   # Authoritative TOML config
```

### 5.3 Daemon Lifecycle

**Startup phases** (in order):

| # | Phase | Fatal? | Notes |
|---|-------|--------|-------|
| 1 | Health monitoring | Yes | Tokio runtime + metrics |
| 2 | DB initialization | **No** | Logs error, continues without DB |
| 3 | Storage (HDF5) | Feature-gated | Ring buffer + writer |
| 4 | Hardware registry | Yes | Parse TOML, create drivers |
| 5 | Shadow-write to DB | No | Best-effort TOML -> DB sync |
| 6 | Initial reconcile | No | Catches DB-only instruments |
| 7 | Watch reconciler | No | LIVE SELECT background task for hot-swap |
| 8 | Safety panic hook | Yes | Emergency laser shutdown on panic |
| 9 | Hardware watchdog | Yes | Deadman timer on separate OS thread |
| 10 | Device supervisor | Yes | Auto-restart faulted devices |
| 11 | gRPC server (+ ConfigService) | Yes | Listens on configured port |

**Shutdown sequence** (safety-critical):
1. Disarm hardware watchdog (prevent false emergency).
2. Cancel gRPC server (stop accepting requests, 5s grace period).
3. Flush storage (HDF5 + DB).
4. Shutdown hardware (return devices to safe state).
5. Abort auxiliary tasks.

### 5.4 Engine Selection Guide

| Criterion | In-Memory | RocksDB |
|-----------|-----------|---------|
| Persistence | Lost on restart | Survives restarts |
| Startup time | ~200 ms | ~500 ms (cold) |
| Memory overhead | ~50 MB | ~100 MB |
| Disk I/O | None | ~10 MB/day |
| Test isolation | Perfect | Requires cleanup |
| Production ready | No | Yes |

**Use in-memory for**: development, unit tests, CI pipelines, stateless
deployments.

**Use RocksDB for**: production lab systems, long-running experiments,
audit requirements, multi-user environments.

### 5.5 Monitoring & Health Checks

- **DB health**: `DaqDb::health_check()` returns true/false.
- **DB info**: `DaqDb::info()` returns schema version, record counts, uptime.
- **Reconcile reports**: Logged at `INFO` level when changes occur.
- **Watch reconciler**: Logs LIVE SELECT status, backoff, circuit breaker.
- **gRPC health**: Standard gRPC health check endpoint.

### 5.6 Backup & Recovery

**Export-based backup** (always works):
```bash
rust-daq config export --db-path /var/lib/rust-daq/db > backup.toml
```

**Filesystem backup** (RocksDB, daemon stopped):
```bash
tar -czf backup.tar.gz /var/lib/rust-daq/db
```

**Recovery from corruption**:
```bash
# Remove corrupted DB
rm -rf /var/lib/rust-daq/db
# Restart daemon (creates fresh DB, shadow-writes from TOML)
systemctl restart rust-daq
```

**Key principle**: TOML is always the source of truth. The DB is a shadow
copy that can be recreated from TOML at any time.

### 5.7 Migration Between Engines

**In-memory -> RocksDB** (adding persistence):
```bash
rust-daq config export > snapshot.toml
# Rebuild with --features db-surreal-rocksdb
rust-daq config import snapshot.toml --db-path /var/lib/rust-daq/db
```

**RocksDB -> In-memory** (removing persistence):
```bash
rust-daq config export --db-path /var/lib/rust-daq/db > backup.toml
# Rebuild with --features db-surreal-mem
# Start daemon with --hardware-config backup.toml
```

### 5.8 Troubleshooting

| Symptom | Cause | Resolution |
|---------|-------|------------|
| "DB initialization failed" at startup | Permissions, stale lock, disk full | Check `ls -ld /var/lib/rust-daq/db`, `df -h`, kill stale processes |
| Config drift between TOML and DB | Manual DB edits without TOML sync | `rust-daq config import <toml>` to re-sync |
| LIVE SELECT disconnections | SurrealDB internal timeout | Automatic: watch reconciler retries with backoff |
| Circuit breaker errors (5+ failures) | Persistent DB issue | Check DB health, restart daemon if needed |
| MeasurementLock blocks reconfiguration | Active measurement stream | Stop measurement, then reconfigure; deferred change auto-applies on next reconcile |
| RocksDB test file lock error | Parallel test execution | Use `--test-threads=1` for RocksDB tests |
| Watch reconciler backoff escalating | LIVE SELECT repeatedly failing | Check DB health; backoff caps at 60s with jitter; circuit breaker logs at error level after 5 failures |
| Config changes not appearing in hardware | Watch reconciler not started | Ensure `db-surreal` feature is enabled and DB initialized; check daemon logs for "starting watch reconciler" |

---

## 6. Quick Reference

### Feature Flag Cheat Sheet

```bash
# Default build — includes in-memory SurrealDB
cargo build

# Run all tests (DB tests included by default)
cargo nextest run

# Production build with RocksDB persistence
cargo build --release -p bin --features production

# Add RocksDB engine (kv-mem is also included via defaults)
cargo build --release -p bin --features db-surreal-rocksdb

# Build without DB (TOML-only, for driver development)
cargo build -p bin --no-default-features --features networking,server
```

### CLI Commands

```bash
# Offline (direct DB access)
rust-daq config import <file> [--db-path <path>]   # TOML -> DB
rust-daq config export [--db-path <path>]           # DB -> TOML
rust-daq config list [--db-path <path>]             # List instruments

# Online (gRPC to running daemon)
rust-daq client config-list [--addr host:port]      # List instruments
rust-daq client config-get <id> [--addr host:port]  # Get instrument
rust-daq client config-import <f> [--addr host:port] # Import TOML → hot-swap
rust-daq client config-export [--addr host:port]    # Export as TOML
rust-daq client config-delete <id> [--addr host:port] # Delete → hot-remove
rust-daq client config-info [--addr host:port]      # DB info
rust-daq client config-watch [--addr host:port]     # Stream changes

# Daemon
rust-daq daemon --port 50051 [--hardware-config <toml>] [--db-path <path>]
```

### Key Source Files

| File | Purpose |
|------|---------|
| `crates/db/src/core.rs` | DaqDb wrapper, init, health check |
| `crates/db/src/schema.rs` | Migration system, DDL |
| `crates/db/src/config_store.rs` | Instrument/driver CRUD, LIVE SELECT |
| `crates/db/src/error.rs` | Error types |
| `crates/bin/src/reconciler.rs` | K8s-style reconcile loop |
| `crates/bin/src/watch_reconciler.rs` | LIVE SELECT watcher with debounce |
| `crates/bin/src/db_bridge.rs` | Hardware <-> DB type conversions |
| `crates/bin/src/daemon_manager.rs` | Daemon lifecycle with DB integration |
| `crates/bin/src/main.rs` | CLI entry point, config subcommands |
| `crates/server/src/grpc/config_service.rs` | gRPC ConfigService implementation |
| `crates/protocol/proto/daq.proto` | ConfigService proto definition |

### Test Counts

| Configuration | Tests | Delta |
|---------------|-------|-------|
| No DB features | 2076 | Baseline |
| `db-surreal-mem` | 2135 | +59 DB tests |
| `db-surreal-rocksdb` | ~5 | Persistence (subprocess) |
