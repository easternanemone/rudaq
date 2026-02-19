# Maitai Universal+DB Hardware Signoff Runbook

Purpose: operational signoff checklist for `hybrid-db` runtime mode on maitai hardware.

## Preconditions

1. Access to `maitai@100.117.5.12`.
2. PVCAM environment available (`source /etc/profile.d/pvcam.sh` or `source config/hosts/maitai.env`).
3. Daemon built with DB support (`db-surreal-mem` or `db-surreal-rocksdb`) and maitai hardware features.

## Build and Launch

```bash
# On maitai
cd ~/code/rust-daq
source config/hosts/maitai.env

# Clean rebuild with full hardware + surrealdb
cargo clean
cargo build --release -p bin --features "maitai,db-surreal-rocksdb,metrics"

# Start daemon in explicit hybrid mode
./target/release/rust-daq-daemon daemon \
  --port 50051 \
  --runtime-mode hybrid-db
```

Expected startup indicators:

- `Runtime mode: hybrid-db`
- `Runtime policy [...]` summary line with non-zero universal count
- `Database ready` (or explicit non-fatal DB warning if unavailable)
- device registration list including camera + universal instruments

## Smoke Validation

1. Device inventory:

```bash
rust-daq client config-list --addr http://127.0.0.1:50051
```

Expected:

- instruments present for camera, rotators, ESP300 axes, maitai laser, and power meter.

2. Metadata integrity:

```bash
rust-daq client config-export --addr http://127.0.0.1:50051 | rg "type|id"
```

Expected:

- exported config includes expected `universal_*` driver types and camera native driver.

3. UI metadata parity:

- Launch GUI and confirm:
  - advanced widget picker shows command/status candidates for universal devices.
  - camera settings changes from image viewer propagate to camera.

4. Command path checks:

- Rotator forward/back relative moves.
- Maitai shutter open/close and telemetry refresh.
- Power meter read updates.

5. DB reconciliation sanity:

- Edit one instrument config via config service and verify watch/reconcile applies change without losing command metadata.

## Signoff Record Template

- Date:
- Operator:
- Build SHA:
- Daemon feature set:
- Runtime mode:
- Result:
  - [ ] PASS
  - [ ] FAIL
- Notes:
- Follow-up issues:

## Known Limitations / Follow-ups

- Hardware-specific timing/transport edge cases still require soak testing.
- `hybrid-db` assumes daemon was built with db-surreal features.
- Legacy SCPI/TCP native driver paths still function but are on deprecation path; see `docs/how-to/legacy-scpi-deprecation.md`.
