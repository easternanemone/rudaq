# Self-Hosted Hardware Qualification Runner Plan

Infrastructure design for running hardware-gated tests on physical lab machines via GitHub Actions self-hosted runners.

> **Status**: Plan (bd-eq67) | **Last updated**: 2026-03-13

## Current State

| Capability | Status |
|-----------|--------|
| Self-hosted runner on leabs-dev | Active (`self-hosted, Linux, X64, leabs` label) |
| Tailscale SSH to maitai from leabs | Working (`hardware-tailscale.yml`) |
| Nightly hardware smoke (both targets) | Working (`nightly-hardware-smoke.yml`, cron `0 7:15 UTC`) |
| PR-gated hardware tests | Not implemented |
| Hardware resource locking | Not implemented (concurrency groups only) |
| Test result artifact retention | Not implemented |
| Windows LIBS runner | Not provisioned |

## Target Architecture

```
GitHub Actions
  ├── ubuntu-latest (cloud)           ← ci.yml, feature-matrix.yml (mock-only)
  ├── self-hosted, Linux, leabs       ← leabs-dev machine (Andor, IPG, Thorlabs PM400)
  │   ├── Local nextest --profile hardware --features andor_hardware
  │   └── SSH → maitai for remote tests
  ├── self-hosted, Linux, maitai      ← maitai machine (PVCAM, Comedi, ELL14, ESP300, MaiTai, Newport)
  │   └── Local nextest --profile hardware --features pvcam_hardware,comedi_hardware
  └── self-hosted, Windows, libs      ← Future: Windows LIBS workstation (Dover Motion)
      └── Local nextest --profile libs-hardware --features dover_hardware
```

## Phase 1: Runner Setup

### leabs-dev (already running)

The leabs-dev machine already has a GitHub Actions runner registered. Current configuration:
- Labels: `self-hosted, Linux, X64, leabs`
- Runner user: `brian`
- Toolchain: Rust stable via rustup, cargo-nextest installed

**Remaining work:**
- Install `actions-runner` as a systemd service for automatic restart
- Pin Rust toolchain version (match CI)
- Create dedicated `runner` user with hardware group membership (`dialout`, `video`)

### maitai (new runner)

Maitai currently receives work via SSH from leabs-dev. Adding a local runner enables faster test execution without SSH overhead.

**Setup steps:**
1. Install GitHub Actions runner (`actions-runner-linux-x64-*.tar.gz`)
2. Register with labels: `self-hosted, Linux, X64, maitai`
3. Create `runner` user, add to groups: `dialout` (serial), `comedi` (DAQ), `video` (camera)
4. Install as systemd service: `./svc.sh install runner`
5. Pin Rust toolchain, install cargo-nextest
6. Set environment variables: `PVCAM_VERSION`, `PVCAM_SDK_PATH`

### Windows LIBS (future)

Deferred until Dover Motion driver is wired into driver-registry (see driver-capability-matrix.md gap). Requirements:
- Windows 10/11, USB access for Dover SmartStage
- Dover Motion SDK installed
- GitHub Actions runner with labels: `self-hosted, Windows, X64, libs`

## Phase 2: Hardware Resource Locking

### Problem

Hardware devices are physical singletons — only one test suite can access a camera or serial port at a time. GitHub Actions concurrency groups prevent overlapping *workflow runs*, but don't prevent two different workflows from accessing the same hardware simultaneously.

### Solution: File-Based Lock + Nextest Test Groups

**Layer 1: GitHub Actions concurrency groups** (already in use)
```yaml
concurrency:
  group: hardware-${{ runner.labels }}-${{ github.ref }}
  cancel-in-progress: false  # Never cancel hardware tests mid-run
```

**Layer 2: File lock on the runner machine**

A wrapper script that acquires a flock before running tests:

```bash
#!/usr/bin/env bash
# scripts/hardware-test-lock.sh
set -euo pipefail

LOCKFILE="/var/lock/rust-daq-hardware-tests.lock"
TIMEOUT_SEC="${HARDWARE_LOCK_TIMEOUT:-600}"  # 10 min default

exec 9>"${LOCKFILE}"
if ! flock --timeout "${TIMEOUT_SEC}" 9; then
  echo "::error::Could not acquire hardware lock after ${TIMEOUT_SEC}s"
  exit 1
fi

echo "Acquired hardware lock (PID $$)"
"$@"
```

Usage in CI:
```yaml
- name: Run hardware tests
  run: |
    bash scripts/hardware-test-lock.sh \
      cargo nextest run --profile hardware --features pvcam_hardware \
      --filter-expr 'test(/pvcam/) | test(/comedi/)'
```

**Layer 3: Nextest test groups** (already configured)

Test groups in `.config/nextest.toml` serialize tests within a profile:
- `pvcam-hardware`: max-threads=1
- `andor-hardware`: max-threads=1
- `serial-hardware`: max-threads=1
- `elliptec-hardware`: max-threads=1

## Phase 3: Test Isolation

### Pre-test device health check

Before running the test suite, verify devices are accessible:

```bash
#!/usr/bin/env bash
# scripts/hardware-preflight.sh
set -euo pipefail

TARGET="${1:?Usage: hardware-preflight.sh <maitai|leabs>}"

check_device() {
  local desc="$1" cmd="$2"
  printf "  %-30s " "${desc}..."
  if eval "${cmd}" >/dev/null 2>&1; then
    echo "OK"
  else
    echo "FAIL"
    return 1
  fi
}

FAILURES=0

case "${TARGET}" in
  maitai)
    echo "=== Maitai device preflight ==="
    check_device "PVCAM SDK"         "test -f /opt/pvcam/lib/libpvcam.so" || ((FAILURES++))
    check_device "Comedi device"     "test -c /dev/comedi0" || ((FAILURES++))
    check_device "ELL14 serial"      "test -c /dev/ttyUSB0" || ((FAILURES++))
    check_device "ESP300 serial"     "test -c /dev/ttyS0" || ((FAILURES++))
    ;;
  leabs)
    echo "=== Leabs device preflight ==="
    check_device "Andor SDK3"        "test -f /usr/local/lib/libatcore.so" || ((FAILURES++))
    check_device "IPG serial"        "test -c /dev/ttyUSB0" || ((FAILURES++))
    check_device "PM400 serial"      "test -c /dev/ttyUSB1" || ((FAILURES++))
    ;;
esac

if (( FAILURES > 0 )); then
  echo "::warning::${FAILURES} device(s) unavailable — hardware tests may fail"
fi
exit 0  # Don't block CI on missing hardware
```

### Post-test cleanup

After each test run, ensure devices are left in a safe state:
- Close all open serial ports
- Stop any running camera acquisitions
- Zero all analog outputs
- Restore DIO to input mode

This is handled by the daemon's existing `SafetyShutdown` path — tests should start/stop the daemon, not leave it running.

## Phase 4: Artifact Retention

### What to capture

| Artifact | Retention | Purpose |
|----------|-----------|---------|
| nextest JUnit XML | 30 days | Test result history, flake tracking |
| Daemon logs (stderr) | 7 days | Debug failing hardware tests |
| Device health preflight output | 7 days | Correlate failures with hardware state |
| Camera test frames (first failure) | 7 days | Visual regression (dark frame, pattern) |
| Core dumps (if any) | 30 days | Post-mortem for segfaults |

### GitHub Actions artifact upload

```yaml
- name: Upload test results
  if: always()
  uses: actions/upload-artifact@v4
  with:
    name: hardware-test-results-${{ matrix.target }}
    path: |
      target/nextest/ci/junit.xml
      /tmp/rust-daq-test-*.log
      /tmp/rust-daq-test-frames/
    retention-days: 30
```

## Phase 5: CI Integration

### Workflow: `hardware-qualification.yml`

Triggered on:
- **Nightly schedule** (existing `nightly-hardware-smoke.yml` — extend, don't replace)
- **Manual dispatch** (for pre-release qualification)
- **Label trigger** (`needs-hardware-test` label on PR)

```yaml
name: Hardware Qualification

on:
  schedule:
    - cron: "0 6 * * 1"  # Weekly Monday 6am UTC
  workflow_dispatch:
    inputs:
      target:
        type: choice
        options: [maitai, leabs, both]
        default: both
  pull_request:
    types: [labeled]

jobs:
  qualification:
    if: >-
      github.event_name == 'schedule' ||
      github.event_name == 'workflow_dispatch' ||
      contains(github.event.pull_request.labels.*.name, 'needs-hardware-test')
    strategy:
      fail-fast: false
      matrix:
        include:
          - target: maitai
            runner: [self-hosted, Linux, X64, maitai]
            features: pvcam_hardware,comedi_hardware
            profile: hardware
          - target: leabs
            runner: [self-hosted, Linux, X64, leabs]
            features: andor_hardware
            profile: hardware
    runs-on: ${{ matrix.runner }}
    concurrency:
      group: hw-qual-${{ matrix.target }}
      cancel-in-progress: false
    timeout-minutes: 60
    steps:
      - uses: actions/checkout@v4
      - name: Preflight
        run: bash scripts/hardware-preflight.sh ${{ matrix.target }}
      - name: Build
        run: cargo build --features ${{ matrix.features }}
      - name: Test
        run: |
          bash scripts/hardware-test-lock.sh \
            cargo nextest run --profile ${{ matrix.profile }} \
            --features ${{ matrix.features }}
      - name: Upload results
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: hw-qual-${{ matrix.target }}
          path: target/nextest/*/junit.xml
          retention-days: 30
```

## Implementation Order

1. **scripts/hardware-test-lock.sh** — file lock wrapper (trivial, unlocks Phase 2)
2. **scripts/hardware-preflight.sh** — device health check (unlocks Phase 3)
3. **Maitai runner provisioning** — install + register GitHub Actions runner
4. **Extend nightly smoke** — add artifact upload to existing workflow
5. **hardware-qualification.yml** — weekly full qualification + PR label trigger
6. **Windows LIBS runner** — deferred until Dover driver is wired in

## Open Questions

- **Runner auto-update**: Should runners auto-update or pin to a specific version? Auto-update risks breaking hardware-sensitive tests.
- **Daemon lifecycle**: Should tests start their own daemon, or expect one running? Starting per-suite is safer but slower.
- **Notification**: Should qualification failures page (PagerDuty/Slack) or just fail the workflow? Currently nightly smoke is silent on failure.
