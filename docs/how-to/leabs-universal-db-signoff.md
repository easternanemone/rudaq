# LEABS Universal+DB Hardware Signoff Runbook

Purpose: operational signoff checklist for `hybrid-db` runtime mode on LEABS hardware.

## Preconditions

1. Access to `leabs-dev` via Tailscale SSH (`ssh leabs-dev`).
2. Andor SDK3 environment available (`source config/hosts/leabs-dev.env`).
3. Daemon built with DB support (`db`) and LEABS hardware features.

## Hardware Profile

| Device ID | Name | Driver Type | Classification | Transport |
|-----------|------|-------------|----------------|-----------|
| `ipg_laser` | IPG YLPP-200-1-50-R Fiber Laser | `universal_ipg_ylpp-200-1-50-r` | universal | TCP 10.0.0.15:10001 |
| `istar_camera` | Andor iStar sCMOS | `andor_istar` | native exception | USB (Andor SDK3) |
| `power_meter` | Thorlabs PM400 Power Meter | `universal_thorlabs_pm400` | universal | USB TMC |

Expected runtime policy: `universal=2, native_exception=1, deprecated_native=0`

## Build and Launch

### Option A: Full Deploy Script

```bash
# From local machine — pulls, builds, launches daemon + GUI
bash scripts/deploy/deploy-leabs.sh --with-db
```

### Option B: Manual Build on leabs-dev

```bash
# On leabs-dev
cd ~/code/rust-daq
source config/hosts/leabs-dev.env

# Build with LEABS hardware + SQLite
cargo build --release -p bin --features "leabs_hardware,db"

# Ensure SQLite data directory exists
mkdir -p data/sqlite-leabs

# Start daemon with LEABS hardware config and persistent database
./target/release/rust-daq-daemon daemon \
  --port 50051 \
  --hardware-config config/leabs_hardware.toml \
  --db-path data/sqlite-leabs/daq.db
```

Expected startup indicators:

- `Initializing database (SQLite)...`
- `Engine: file (data/sqlite-leabs/daq.db)`
- `Database ready (schema v9, ...)`
- Device registration list including all 3 instruments

## Smoke Validation

### 1. Device Inventory

```bash
./target/release/rust-daq-daemon client config-list --addr http://127.0.0.1:50051
```

Expected:

- 3 instruments present: `ipg_laser`, `istar_camera`, `power_meter`.
- Driver types match hardware profile table above.

### 2. Metadata Integrity

```bash
./target/release/rust-daq-daemon client config-export --addr http://127.0.0.1:50051 | rg "type|id"
```

Expected:

- Exported config includes `universal_ipg_ylpp-200-1-50-r`, `andor_istar`, and `universal_thorlabs_pm400` driver types.

### 3. IPG Laser Smoke

- Emission enable/disable toggle via advanced widget or CLI.
- Telemetry refresh shows laser status parameters (power, temperature, fault state).
- Verify universal driver command/status metadata is populated in DB.

### 4. Andor iStar Camera Smoke

- Cooling initialization: camera reaches target temperature (`TemperatureControl = 0.00`).
- Temperature readback via sensor status parameter.
- Single frame acquisition (if GUI available) or camera status check.
- Note: on macOS builds, camera runs in mock mode. Real SDK validation requires Linux.

### 5. Thorlabs PM400 Power Meter Smoke

- Power read returns a numeric value (may be zero with no input beam).
- Wavelength parameter set/get round-trip.
- Verify USB TMC path (`/dev/usbtmc0`) is accessible (udev rule in place).

### 6. DB Persistence Check

- Restart daemon and verify SQLite-persisted config survives restart.
- After restart, run `config-list` and verify all 3 instruments are present without re-import from TOML.

### 7. DB Health Check

```bash
./target/release/rust-daq-daemon client config-info --addr http://127.0.0.1:50051
```

Expected:

- `Healthy: true`
- `Instruments: 3`
- `Engine: file`

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

- Andor iStar runs in mock mode on macOS. Real SDK validation requires `leabs-dev` Linux VM with `andor_hardware` feature.
- IPG laser TCP control requires network access to 10.0.0.15. Verify Tailscale routing if commands time out.
- PM400 USB TMC requires `usbtmc` kernel module loaded and udev rule for non-root access.
- `hybrid-db` requires daemon built with `db` feature (enabled by default).
