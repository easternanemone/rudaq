# Operations and Deployment Guide

Practical procedures for running, deploying, and maintaining the rust-daq daemon
in development and production (maitai lab) environments.

## Daemon Startup

### Local development (mock hardware)

```bash
# Build and run with mock devices only
cargo build -p bin
./target/debug/rust-daq-daemon daemon --port 50051
```

The daemon listens on `localhost:50051` by default. Without a hardware config
file, all devices are mocks.

### With a hardware configuration file

```bash
./target/release/rust-daq-daemon daemon \
  --port 50051 \
  --hardware-config config/maitai_universal.toml
```

### Lab-hardware shorthand

The `--lab-hardware` flag selects the built-in maitai lab profile
(mutually exclusive with `--hardware-config`):

```bash
./target/release/rust-daq-daemon daemon --port 50051 --lab-hardware
```

### Explicit runtime modes

Use `--runtime-mode` for deterministic launcher/profile selection:

```bash
# Mock-only local runtime
./target/release/rust-daq-daemon daemon --runtime-mode mock

# Native maitai profile
./target/release/rust-daq-daemon daemon --runtime-mode native

# Universal TOML profile
./target/release/rust-daq-daemon daemon --runtime-mode universal

# Universal + SurrealDB control-plane expectations
./target/release/rust-daq-daemon daemon --runtime-mode hybrid-db --db-path ./data/surrealdb
```

### CLI reference

| Flag | Default | Description |
|------|---------|-------------|
| `--port` | `50051` | gRPC listen port |
| `--runtime-mode <mock\|native\|universal\|hybrid-db>` | unset | Explicit runtime profile selector |
| `--db-path <path>` | none | SurrealDB RocksDB path in daemon mode (`db-surreal` builds) |
| `--hardware-config <path>` | none | Path to a TOML hardware config |
| `--lab-hardware` | off | Backward-compatible alias for native maitai profile |

## Daemon Shutdown

The daemon handles **Ctrl-C / SIGINT** and performs an ordered graceful
shutdown that is critical for laser safety:

1. **Stop gRPC server** -- reject new requests, drain in-flight RPCs (5 s grace).
2. **Flush storage** -- persist any buffered data to disk.
3. **Shutdown hardware** -- return all devices to a safe physical state.
4. **Cleanup** -- abort monitoring/supervisor tasks, disarm hardware watchdog.

Never kill the daemon with `SIGKILL` (`kill -9`) unless absolutely necessary --
this skips the safety shutdown and may leave laser shutters open.

## Maitai Deployment Workflow

The maitai machine (`maitai@100.117.5.12`, via Tailscale) runs the real
hardware stack: PVCAM camera, serial instruments, and Comedi DAQ.

### Step 1 -- Validate the environment

```bash
# Source the maitai env (sets PVCAM paths, serial ports, etc.)
source config/hosts/maitai.env

# Or run a standalone validation check
./scripts/env-check.sh --check
```

`env-check.sh` verifies:
- Rust toolchain presence
- PVCAM SDK directory (`/opt/pvcam/sdk`)
- PVCAM libraries (`/opt/pvcam/library/x86_64`)
- `PVCAM_VERSION` environment variable
- `LD_LIBRARY_PATH` and `LIBRARY_PATH` configuration
- `pvcam.ini` existence

When sourced (`source scripts/env-check.sh`), it auto-fixes missing variables.
When invoked as a script (`./scripts/env-check.sh --check`), it reports only.

### Step 2 -- Build with the maitai feature

```bash
bash scripts/build-maitai.sh
```

This script:
1. Sources `config/hosts/maitai.env` (PVCAM SDK paths).
2. Verifies `PVCAM_SDK_DIR` is set.
3. Runs `cargo clean` (full clean is required because feature flags are baked
   into transitive dependencies; partial cleaning does not reliably invalidate
   them).
4. Builds in release mode: `cargo build --release -p bin --features maitai`.

The `maitai` feature flag enables **all** real hardware drivers:
- PVCAM (real SDK, not mock)
- Serial/TCP devices via `driver-universal` TOML manifests (ELL14 rotators, ESP300, Newport power meter, MaiTai laser)

### Step 3 -- Start the daemon

```bash
./target/release/rust-daq-daemon daemon \
  --port 50051 \
  --hardware-config config/maitai_universal.toml
```

### Step 4 -- Verify

Check the daemon log output for:

| Expected log message | Meaning |
|----------------------|---------|
| `pvcam_sdk feature enabled: true` | Real PVCAM SDK linked |
| `Successfully opened camera` with real handle | Camera initialized (not mock) |
| `Registered 7 device(s)` | All instruments detected |

The registered devices should include: `prime_bsi`, `maitai`, `power_meter`,
`rotator_2`, `rotator_3`, `rotator_8`, `esp300_axis1`.

If the log shows `using mock mode`, the build is incorrect -- re-run
`build-maitai.sh`.

If startup logs show legacy SCPI/TCP deprecation warnings, migrate to universal
driver types per `docs/how-to/legacy-scpi-deprecation.md`.

## Feature Flags (Runtime)

Runtime feature flags are defined in `config/feature_flags.toml`. They are
loaded at startup and queried via `FeatureFlags::is_enabled("flag_name")`.

Key flags:

| Flag | Default | Purpose |
|------|---------|---------|
| `frame_pool_preallocation` | `true` | Pre-allocate frame memory pool |
| `async_ring_buffer` | `true` | Use async ring buffer for data plane |
| `strict_serial_validation` | `true` | Validate serial command checksums |
| `experimental_streaming` | `false` | Experimental streaming mode |
| `experimental_frame_compression` | `false` | Compressed frame transport |
| `debug_frame_timing` | `false` | Log per-frame timing |
| `debug_serial_commands` | `false` | Log raw serial traffic |
| `verbose_grpc_logging` | `false` | Log all gRPC requests/responses |
| `legacy_pvcam_mode` | `false` | Compatibility with older PVCAM SDKs |

Edit the file directly and restart the daemon to apply changes.

## Monitoring and Health Checks

### gRPC Health Service

The daemon exposes a `HealthService` via gRPC with these RPCs:

| RPC | Description |
|-----|-------------|
| `GetSystemHealth` | Overall system status (`Healthy`, `Degraded`, `Critical`), module-level breakdown, and counts of healthy/unhealthy modules |
| `GetModuleHealth` | Per-module health status |
| `GetDeviceHealth` | Per-device health state (when a `DeviceRegistry` is attached) |
| `GetErrorHistory` | Recent error records with severity levels |
| `StreamHealthUpdates` | Server-streaming RPC for live health change notifications |

### Hardware watchdog

A `HardwareWatchdog` runs on a dedicated OS thread (not a Tokio task) inside
the daemon and monitors daemon liveness. The daemon's registry monitor task
kicks the watchdog every 10 seconds; if the Tokio runtime hangs or deadlocks,
the watchdog fires a 5-step emergency shutdown sequence:

1. Close scripting-registered shutters (`ShutterRegistry`)
2. Close ALL `ShutterControl` devices from the `DeviceRegistry`
3. Disable ALL `EmissionControl` devices (laser sources)
4. Stop all motors (`Movable` devices)
5. Zero all DAQ analog outputs (`Settable` devices)

The ordering prioritizes beam blocking (shutters, then emission disable)
before addressing mechanical motion and analog outputs. Each device
operation has a 2-second timeout to prevent a single unresponsive device
from blocking the rest of the sequence.

The watchdog is automatically disarmed during graceful shutdown to prevent
false emergency triggers. Default timeout: 30 seconds.

See [ADR-004](../adr/004-panic-safety.md) for the full safety architecture.

### Safety heartbeat

When running on hardware with Comedi DAQ support (`comedi_hardware` feature),
the daemon can drive an external hardware interlock via a DIO pulse train.
A Tokio task toggles a digital output channel at a fixed interval (default
100ms). If the daemon crashes, hangs, or is killed, the pulse stops and the
external interlock circuit cuts laser power.

This is a proactive safety measure that protects against failure modes
software cannot catch (SIGKILL, power loss, total process death).

**Configuration** in the hardware TOML file:

```toml
[safety_heartbeat]
enabled = true          # Whether the heartbeat is active (default: true)
device = "/dev/comedi0" # Comedi device path
# subdevice = 2         # DIO subdevice index (omit to auto-detect)
channel = 0             # DIO channel number to toggle
interval_ms = 100       # Toggle interval in milliseconds (default: 100)
```

The heartbeat task is only compiled when the `comedi_hardware` feature is
enabled. On non-Comedi builds, the configuration is silently ignored.

On graceful shutdown, the channel is driven LOW before the task exits. If
the toggle encounters 10 consecutive failures, the task stops (the external
interlock detects the missing pulse and triggers its own shutdown).

### Database readiness (SurrealDB)

When running with the `db-surreal` feature, the daemon reports database state
through multiple channels:

- **Startup banner**: Shows `ConfigService [DB available]` or
  `⚠ ConfigService UNAVAILABLE` in the feature list.
- **Health endpoint**: `GetSystemHealth` includes `db_available`,
  `config_service_available`, `db_engine`, and `db_state_message` fields.
- **Standard health check**: `grpc.health.v1.Health/Check` reports
  `daq.ConfigService` as `SERVING` or `NOT_SERVING`.
- **Module health**: The `database` module appears in `GetModuleHealth`
  with heartbeat status (healthy) or error record (degraded).

If DB initialization fails, the daemon continues running from TOML config
but `ConfigService` is unavailable and system health reports `Degraded`.

To query database info directly:

```bash
rust-daq-daemon client config-info --addr http://localhost:50051
```

Returns engine type, namespace, schema version, uptime, and instrument counts.

### Client reconnection

The Rust client library (`crates/client`) includes a `ReconnectManager` with
configurable health-check probes. For local/Tailscale connections, the default
probes use a faster interval.

## Webhook Alerting

The daemon can send Slack/Discord-compatible webhook notifications for
critical events during unattended operation. Configure via the `[alerting]`
section in `config/config.v4.toml`:

```toml
[alerting]
webhook_url = "https://hooks.slack.com/services/T.../B.../..."
rate_limit_secs = 60
alert_on_fault = true
alert_on_restart_limit = true
alert_on_plan_abort = true
max_restart_attempts = 5
```

**Env var override:** Set `RUSTDAQ_ALERTING__WEBHOOK_URL` to configure the
webhook URL without editing the config file (useful for CI or Docker
deployments).

**Alert types:**

| Event | Message | Trigger |
|-------|---------|---------|
| Device fault | `Device Fault \| \`cam1\` faulted after 3 consecutive failures: ...` | Device transitions to `Faulted` state |
| Restart limit | `Restart Limit Reached \| \`cam1\` failed after 5 restart attempts` | Device exhausts `max_restart_attempts` |
| Plan abort | `Plan Aborted \| run \`abc123\` aborted after 47 events: ...` | RunEngine emits a `Stop` document with `exit_status == "abort"` |

**Rate limiting:** Alerts are rate-limited per device key (e.g.,
`fault:cam1`). The default is 60 seconds — a device that repeatedly faults
generates at most one alert per minute. Different devices and different
alert types are tracked independently.

**Discord compatibility:** Discord incoming webhooks accept the same
`{"text": "..."}` JSON payload format as Slack. No configuration changes
are needed — just use your Discord webhook URL.

Source: `crates/server/src/alerting.rs`

---

## Heartbeat JSONL Log

The daemon writes a structured JSONL log every 60 seconds for overnight
run forensics. Each line is a self-contained JSON object with system vitals:

```json
{"ts":"2026-03-17T03:14:00Z","cpu_pct":12.3,"rss_mb":847,"disk_free_gb":42.1,
 "devices":12,"healthy":10,"faulted":1,"degraded":1,
 "engine":"running","run_uid":"abc123","seq_num":47,"queued":0}
```

**Default location:** `/tmp/rust_daq_heartbeat.jsonl`

**Field reference:**

| Field | Type | Description |
|-------|------|-------------|
| `ts` | string | ISO 8601 UTC timestamp |
| `cpu_pct` | float | Global CPU usage percentage |
| `rss_mb` | int | Daemon process RSS in megabytes |
| `disk_free_gb` | float | Free disk space on storage volume (1 decimal) |
| `devices` | int | Total registered devices |
| `healthy` | int | Devices in Healthy state |
| `faulted` | int | Devices in Faulted state |
| `degraded` | int | Devices in Degraded state |
| `engine` | string | RunEngine state: `idle`, `running`, `paused`, `aborting` |
| `run_uid` | string? | Current run UID (omitted when idle) |
| `seq_num` | int? | Current event sequence number (omitted when idle) |
| `queued` | int | Number of plans queued |

### Post-mortem analysis with jq

```bash
# Show timeline of engine state changes
jq -r '[.ts, .engine, .run_uid // "–"] | @tsv' /tmp/rust_daq_heartbeat.jsonl

# Find when a device faulted
jq 'select(.faulted > 0)' /tmp/rust_daq_heartbeat.jsonl

# Show memory growth over time
jq -r '[.ts, .rss_mb] | @tsv' /tmp/rust_daq_heartbeat.jsonl

# Find disk space drops
jq 'select(.disk_free_gb < 10)' /tmp/rust_daq_heartbeat.jsonl

# Last entry before a crash (tail the file)
tail -1 /tmp/rust_daq_heartbeat.jsonl | jq .
```

Source: `crates/server/src/health/heartbeat_log.rs`

---

## Overnight Run Monitoring

For unattended experiments (e.g., overnight spectral scans), combine
webhook alerting and the heartbeat log for complete coverage:

1. **Real-time alerts** — Configure a Slack/Discord webhook to get push
   notifications on your phone if a device faults or a plan aborts.
2. **Forensic breadcrumbs** — The heartbeat JSONL provides a minute-by-minute
   timeline of system health. After an overnight failure, use `jq` queries
   to reconstruct what happened.
3. **Crash detection** — The RunEngine updates `heartbeat_at` on the
   `run_record` in SurrealDB every ~10 seconds during plan execution. If
   the daemon crashes, the reconciler on restart detects stale heartbeats
   and marks the run as `crashed`.

**Recommended checklist before overnight runs:**

- [ ] Verify webhook URL is configured (`grep webhook_url config/config.v4.toml`)
- [ ] Confirm test alert arrives (`curl -X POST -H 'Content-Type: application/json' -d '{"text":"test"}' <webhook_url>`)
- [ ] Check available disk space (`df -h /tmp`)
- [ ] Start the daemon and verify all devices are healthy
- [ ] Queue the plan and confirm `engine: running` in the heartbeat log

---

## Log Management

Logging uses the `tracing` / `tracing-subscriber` stack with the standard
`RUST_LOG` environment variable.

### Setting log levels

```bash
# Default (info)
./target/release/rust-daq-daemon daemon --port 50051

# Debug logging for the entire workspace
RUST_LOG=debug ./target/release/rust-daq-daemon daemon --port 50051

# Fine-grained per-crate control
RUST_LOG="rust_daq=debug,server=trace,hardware=info" \
  ./target/release/rust-daq-daemon daemon --port 50051

# Silence everything except warnings
RUST_LOG=warn ./target/release/rust-daq-daemon daemon --port 50051
```

### Common filter patterns

| Pattern | Use case |
|---------|----------|
| `RUST_LOG=debug` | General debugging |
| `RUST_LOG=server=trace` | Trace gRPC request handling |
| `RUST_LOG=hardware=debug` | Debug device communication |
| `RUST_LOG=driver_pvcam=trace` | Trace PVCAM SDK calls |
| `RUST_LOG=scripting=debug` | Debug Rhai script execution |

### Redirecting to a file

```bash
RUST_LOG=info ./target/release/rust-daq-daemon daemon --port 50051 2>&1 | tee daemon.log
```

## Error Reporting

rust-daq currently relies on structured logs, gRPC health checks, daemon startup banners, and module health reporting for operational visibility. There is no `error_tracking` / Sentry feature in the current build matrix.

Use the health endpoints, daemon logs, and `rust-daq-daemon client config-info` for operational diagnostics.

## Target Directory Maintenance

Over time, `target/` can grow to tens of gigabytes. Two scripts manage this.

### Manual cleanup

```bash
# Check size and clean if above 30 GB (default threshold)
bash scripts/target-maintenance.sh

# Force a full cargo clean regardless of size
bash scripts/target-maintenance.sh --force --mode full

# Partial clean (remove incremental artifacts + heavy crates only)
bash scripts/target-maintenance.sh --mode partial

# Dry run -- see what would happen
bash scripts/target-maintenance.sh --dry-run
```

| Flag | Default | Description |
|------|---------|-------------|
| `--mode full` | `full` | `cargo clean` (removes everything) |
| `--mode partial` | -- | Remove `target/debug/incremental` + `cargo clean -p ui -p server` |
| `--threshold-gb N` | `30` | Only run if `target/` is >= N GiB |
| `--force` | off | Ignore threshold, always run |
| `--dry-run` | off | Print actions without executing |

### Scheduled cleanup

Install a periodic cleanup job (launchd on macOS, systemd timer on Linux):

```bash
# Install with defaults (full mode, 30 GB threshold)
bash scripts/install-target-maintenance.sh

# Install with custom settings
bash scripts/install-target-maintenance.sh --mode partial --threshold-gb 20

# Remove the scheduled job
bash scripts/install-target-maintenance.sh --uninstall
```
