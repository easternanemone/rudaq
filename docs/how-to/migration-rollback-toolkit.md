# Migration and Rollback Toolkit

Operator playbooks for backup, export, import, and recovery of rust-daq configuration data during the universal+SurrealDB rollout.

## Overview

The daemon stores instrument configuration in two forms:

1. **TOML files** — static hardware configs (`config/*.toml`), the source of truth for initial setup.
2. **SurrealDB** — runtime control-plane database (in-memory or RocksDB-persisted), populated by shadow-writing TOML configs at startup.

These playbooks cover moving data between these forms, backing up state, and recovering from incidents.

## 1. Backup Procedures

### 1a. Online Backup (gRPC — daemon running)

Export the current database state as TOML via the running daemon:

```bash
# Export to stdout
rust-daq-daemon client config-export --addr http://127.0.0.1:50051

# Export to file
rust-daq-daemon client config-export --addr http://127.0.0.1:50051 > backup_$(date +%Y%m%d_%H%M%S).toml
```

Verify export integrity:

```bash
rust-daq-daemon client config-info --addr http://127.0.0.1:50051
# Check instrument count matches expected
```

### 1b. Offline Backup (direct DB — daemon stopped)

Export from a RocksDB database file without a running daemon:

```bash
rust-daq-daemon config export --db-path data/surrealdb-maitai > backup.toml
```

### 1c. Filesystem Backup (RocksDB tarball)

For full database preservation including internal state:

```bash
# Stop daemon first to ensure consistent snapshot
pkill -INT -f 'rust-daq-daemon daemon'

# Create tarball of RocksDB directory
tar czf surrealdb-backup_$(date +%Y%m%d_%H%M%S).tar.gz data/surrealdb-maitai/

# Restart daemon
./target/release/rust-daq-daemon daemon --runtime-mode hybrid-db --db-path data/surrealdb-maitai
```

## 2. Restore Procedures

### 2a. Import from TOML (online — daemon running)

```bash
# Import TOML config into running daemon's database
rust-daq-daemon client config-import backup.toml --addr http://127.0.0.1:50051
```

Verify:

```bash
rust-daq-daemon client config-list --addr http://127.0.0.1:50051
# Confirm expected instruments are present
```

### 2b. Import from TOML (offline — daemon stopped)

```bash
# Import into a RocksDB database directly
rust-daq-daemon config import backup.toml --db-path data/surrealdb-maitai
```

### 2c. Full DB Rebuild from Hardware Config

If the database is corrupted or needs a clean start, rebuild from the original TOML hardware config:

```bash
# Delete existing database
rm -rf data/surrealdb-maitai/

# Restart daemon — it will shadow-write from TOML config at startup
./target/release/rust-daq-daemon daemon \
  --runtime-mode hybrid-db \
  --db-path data/surrealdb-maitai
```

The daemon automatically shadow-writes the hardware TOML config into the database on startup.

## 3. Engine Migration

### 3a. In-Memory to RocksDB

Export from a running in-memory daemon and import into a new RocksDB instance:

```bash
# Export from running in-memory daemon
rust-daq-daemon client config-export --addr http://127.0.0.1:50051 > migration.toml

# Stop daemon
pkill -INT -f 'rust-daq-daemon daemon'

# Import into RocksDB (offline)
rust-daq-daemon config import migration.toml --db-path data/surrealdb-persistent

# Restart with RocksDB
./target/release/rust-daq-daemon daemon \
  --runtime-mode hybrid-db \
  --db-path data/surrealdb-persistent
```

### 3b. RocksDB to In-Memory

```bash
# Export from RocksDB (offline only — stop daemon first)
rust-daq-daemon config export --db-path data/surrealdb-persistent > migration.toml

# Restart without --db-path (uses in-memory engine)
# TOML hardware config will be shadow-written at startup
./target/release/rust-daq-daemon daemon --runtime-mode hybrid-db
```

Note: in-memory engine loses state on daemon restart. The TOML hardware config is re-imported each startup.

## 4. Incident Rollback Runbook

### Decision Tree

```
Incident detected
  │
  ├─ Daemon won't start with hybrid-db?
  │   └─ Immediate: restart with --runtime-mode native
  │      Then: investigate DB state (see 4a)
  │
  ├─ DB corruption suspected?
  │   └─ Export what you can (see 4b)
  │      Then: delete DB and restart (auto-rebuilds from TOML)
  │
  ├─ Wrong config in DB (device misconfigured)?
  │   └─ Export current state for forensics
  │      Then: reimport correct TOML (see 2a/2b)
  │
  └─ Universal driver regression (device not responding)?
      └─ Immediate: restart with --runtime-mode native
         Then: file regression issue (see 4c)
```

### 4a. Immediate Rollback to Native Mode

```bash
# Stop current daemon
pkill -INT -f 'rust-daq-daemon daemon'

# Restart in native mode (bypasses universal drivers and DB entirely)
./target/release/rust-daq-daemon daemon --runtime-mode native
```

This uses `config/maitai_hardware.toml` with legacy native drivers. No database interaction.

### 4b. Data Preservation from Corrupted DB

```bash
# Try online export first (if daemon is still running)
rust-daq-daemon client config-export --addr http://127.0.0.1:50051 > rescue.toml 2>/dev/null

# If daemon is down, try offline export
rust-daq-daemon config export --db-path data/surrealdb-maitai > rescue.toml 2>/dev/null

# If both fail, preserve raw DB files for forensics
tar czf surrealdb-forensic_$(date +%Y%m%d_%H%M%S).tar.gz data/surrealdb-maitai/

# Delete corrupted DB and restart (rebuilds from TOML)
rm -rf data/surrealdb-maitai/
./target/release/rust-daq-daemon daemon \
  --runtime-mode hybrid-db \
  --db-path data/surrealdb-maitai
```

### 4c. Filing a Regression Issue

When rolling back, capture:

- Runtime mode that was in use
- Startup policy log lines (`Runtime policy [...]`)
- Affected devices and commands
- DB export (if available)
- Daemon log output

## 5. Verification

### Config Parity Check

After any migration or restore, verify the database matches expectations:

```bash
# Count check
rust-daq-daemon client config-list --addr http://127.0.0.1:50051
# Verify instrument count matches hardware config

# Health check
rust-daq-daemon client config-info --addr http://127.0.0.1:50051
# Verify: Healthy=true, correct engine, expected counts

# Diff check (compare DB export to source TOML)
rust-daq-daemon client config-export --addr http://127.0.0.1:50051 > db_export.toml
diff <(sort db_export.toml) <(sort config/maitai_universal.toml)
# Note: field ordering may differ; focus on device IDs and driver types
```

### Post-Migration Smoke Test

After any engine migration or rollback:

1. Verify all devices appear in `config-list`.
2. Move one stage axis and confirm motion completes.
3. Read one sensor and confirm value is reasonable.
4. Check health endpoint reports `Healthy: true`.
