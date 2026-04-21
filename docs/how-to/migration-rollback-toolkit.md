# Migration and Rollback Toolkit

Operator playbooks for backing up and recovering rust-daq configuration and control-plane state.

## Current Storage Model

The current codebase has two relevant control-plane sources:

1. **TOML files** — static hardware configs (`config/*.toml`) and universal-driver manifests (`config/devices/*.toml`). These remain the source of truth for deployment configuration.
2. **SQLite** — runtime control-plane database provided by the `db` crate and enabled by the daemon `db` feature.

The old SurrealDB/RocksDB feature family (`db-surreal-*`, `kv-*`) has been removed. Older commands that mention `data/surrealdb-*` or `db-surreal-rocksdb` are historical and should not be used.

## Backup Procedures

### Online Export

When the daemon is running with ConfigService enabled:

```bash
rust-daq-daemon client config-export --addr http://127.0.0.1:50051 > backup_$(date +%Y%m%d_%H%M%S).toml
rust-daq-daemon client config-info --addr http://127.0.0.1:50051
```

### TOML Backup

For deployment configuration, back up the git-tracked files directly:

```bash
tar czf rust-daq-config_$(date +%Y%m%d_%H%M%S).tar.gz \
  config/*.toml config/devices/*.toml config/hosts/*.env
```

### SQLite File Backup

Stop the daemon first to get a consistent file snapshot:

```bash
pkill -INT -f 'rust-daq-daemon daemon'
cp data/daq.db data/daq_$(date +%Y%m%d_%H%M%S).db
```

Use the actual `--db-path` configured for the deployment; `data/daq.db` is only an example.

## Restore Procedures

### Restore From TOML

Start the daemon with the desired hardware config. The daemon will rebuild runtime state from TOML and the configured SQLite database path.

```bash
./target/release/rust-daq-daemon daemon \
  --hardware-config config/maitai_universal.toml \
  --db-path data/daq.db
```

### Restore A SQLite Snapshot

Stop the daemon, replace the DB file, then restart:

```bash
pkill -INT -f 'rust-daq-daemon daemon'
cp backups/daq_good.db data/daq.db
./target/release/rust-daq-daemon daemon \
  --hardware-config config/maitai_universal.toml \
  --db-path data/daq.db
```

### Rebuild A Corrupt DB

If the SQLite file is corrupt or suspected stale, move it aside and let the daemon recreate state from TOML:

```bash
pkill -INT -f 'rust-daq-daemon daemon'
mv data/daq.db data/daq_corrupt_$(date +%Y%m%d_%H%M%S).db
./target/release/rust-daq-daemon daemon \
  --hardware-config config/maitai_universal.toml \
  --db-path data/daq.db
```

## Verification

After any restore:

```bash
rust-daq-daemon client config-info --addr http://127.0.0.1:50051
rust-daq-daemon client config-list --addr http://127.0.0.1:50051
```

Confirm device count, driver types, and critical parameters before enabling lasers or unattended acquisition.
