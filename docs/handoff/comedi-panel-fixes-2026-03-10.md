> [!WARNING] **ARCHIVAL / HISTORICAL**
> This document is a historical snapshot and is preserved for context. It does not represent current operational guidance or source-of-truth architecture.

# Handoff: Comedi Panel Fixes & Deployment

**Date:** 2026-03-10
**Branch:** `main`
**Beads issue:** `bd-g3g9` (deploy + verify)

## What happened

The Comedi DAQ panel in the WASM GUI had a 4-layer bug causing all operations to fail with "Failed to open device '/dev/photodiode'" instead of using `/dev/comedi0`. All 4 layers have been fixed and pushed.

During testing via the Chrome WASM GUI, clicking "Read All" on the Analog Input tab fired 16 concurrent `ReadAnalogInput` gRPC calls. Each one called `ComediDevice::open("/dev/comedi0")` inside `spawn_blocking`, deadlocking the Comedi kernel driver. **Maitai froze completely and needs a physical reboot.**

## Commits (all pushed to `main`)

| Commit | Description |
|--------|-------------|
| `27dabcec7` | Server: resolve device path from driver TOML config instead of device ID |
| `fa0a89c57` | Proto: add `device_path` field to `DAQStatus`; UI reads it from gRPC response |
| `23f6ee5b9` | Registry: preserve raw driver config (was discarding it, breaking `get_driver_config_str`) |
| `5baec5e46` | **Server: add `Semaphore(1)` to serialize all 12 Comedi RPC handlers** |
| `997e8e49c` | UI: downgrade `try_grpc_ui_config` logs from debug→trace (console spam fix) |

## What needs to happen after maitai reboot

### Step 1: Deploy

```bash
# From your local machine (macOS)
bash scripts/deploy-maitai.sh --daemon-only
```

If deploy script fails (git pull issues on maitai), use the git bundle method:

```bash
# Find common ancestor
MAITAI_HEAD=$(ssh maitai@100.117.5.12 'cd ~/code/rust-daq && git rev-parse HEAD')
ANCESTOR=$(git merge-base $MAITAI_HEAD HEAD)

# Create and send bundle
git bundle create /tmp/comedi-fix.bundle ${ANCESTOR}..HEAD --all
scp /tmp/comedi-fix.bundle maitai@100.117.5.12:/tmp/

# On maitai: merge
ssh maitai@100.117.5.12 'cd ~/code/rust-daq && git stash && git fetch /tmp/comedi-fix.bundle HEAD:refs/remotes/bundle/main && git merge bundle/main --no-edit'

# Build on maitai
ssh maitai@100.117.5.12 'cd ~/code/rust-daq && bash scripts/build-maitai.sh'
```

### Step 2: Restart daemon

```bash
ssh maitai@100.117.5.12
# Kill old daemon if running
pkill -f rust-daq-daemon || true
# Start new daemon
cd ~/code/rust-daq
./target/release/rust-daq-daemon daemon --port 50051 --hardware-config config/maitai_universal.toml &
```

### Step 3: Verify via WASM GUI

Open Chrome to `http://100.117.5.12:8080`, connect to daemon at `100.117.5.12:50051`.

Test each Comedi panel tab for **all 3 Comedi devices** (`photodiode`, `ni_daq_ao0`, `ni_daq_ao1`):

- [ ] **Overview tab** — Shows "Connected", board name `pci-mio-16xe-10`, driver `ni_pcimio`, path `/dev/comedi0`
- [ ] **Analog Input** — Read single channel works. **"Read All" button** works without hanging (this was the crash trigger)
- [ ] **Analog Output** — Set voltage on channel 0 and 1
- [ ] **Digital I/O** — Configure pins, read/write individual pins and ports
- [ ] **Counter** — Read counter value, reset counter

### Step 4: Close the issue

```bash
bd close bd-g3g9 --reason="Deployed and verified all Comedi panel tabs"
```

## Key files modified

| File | Change |
|------|--------|
| `crates/server/src/grpc/ni_daq_service.rs` | Semaphore(1) on struct, acquired in all 12 `#[cfg(feature = "comedi")]` blocks |
| `crates/hardware/src/registry.rs` | `components_to_registered()` preserves raw TOML config |
| `crates/protocol/proto/ni_daq.proto` | `device_path` field 5 in `DAQStatus` |
| `crates/ui/src/panels/comedi/unified.rs` | Reads `device_path` from gRPC response |
| `crates/ui/src/panels/instrument_manager/dispatch.rs` | Log level debug→trace |

## Technical context for the semaphore

The NI PCI-MIO-16XE-10 is accessed through `/dev/comedi0`. All 3 logical Comedi devices (photodiode AI, ao0, ao1) share this single kernel device file. The `ni_pcimio` kernel driver cannot handle 16+ simultaneous `open()` calls — it deadlocks and hangs the entire machine.

The `tokio::sync::Semaphore(1)` lives on `NiDaqServiceImpl` and is acquired in the async context _before_ entering `spawn_blocking`. This means:
- Only one Comedi FFI call runs at a time
- Waiting RPCs queue cooperatively in Tokio (no threads consumed)
- The streaming RPC (`stream_analog_input`) releases the permit after setup, so background streaming doesn't block other RPCs

## Remaining issues

1. **`set_analog_output` uses `Settable` trait** (not direct `ComediDevice::open`), so it bypasses the semaphore. If `Settable::set_value` internally opens `/dev/comedi0`, there could still be a race with the semaphored RPCs. Worth checking during testing.

2. **`get_daq_status` does non-`spawn_blocking` FFI** — after `ComediDevice::open()` in `spawn_blocking`, it calls `device.info()`, `device.analog_input_subdevice()`, etc. directly in async context. These are blocking FFI calls on the Tokio runtime. The semaphore prevents concurrency issues, but ideally all FFI should be in `spawn_blocking`.
