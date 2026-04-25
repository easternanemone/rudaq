# Disk-safety for self-hosted runners

The two self-hosted GitHub Actions runners (`maitai-eos`, `leabs-dev`)
both carry the `leabs` label, so any `runs-on: [self-hosted, Linux, X64, leabs]`
job can land on either runner. When developer-launched cargo builds run on
the same host as a CI job, two `target/` trees grow against the same
filesystem and neither side notices the other. We have wedged `maitai-eos`
twice this way: root hits 100%, journald and sshd hang on writes, the box
needs a physical power cycle.

This directory contains a defense-in-depth fix that runs **on the runner
host itself**, independent of any GitHub Action.

## Layers

1. **Workflow-level cleanup** (covered separately by
   `.github/actions/reclaim-disk` + the changes in `.github/workflows/*.yml`).
   Runs once at the start of each self-hosted CI job; only protects the
   in-job target tree.

2. **Host-level disk watchdog** (this directory). A systemd timer fires
   every minute. The script:
   - logs `free=NNGiB partition=/ warn=… crit=…` to journald (`-t disk-watchdog`);
   - at **WARN** (default 25 GiB free), removes any tracked `target/` tree
     that has had no file activity for ≥ 30 min — safe under in-flight
     builds because mid-compile mtimes are recent;
   - at **CRIT** (default 10 GiB free), stops the GitHub runner systemd
     unit and unconditionally removes every tracked `target/` tree. A
     state file `/var/lib/disk-watchdog/runner-stopped-by-watchdog` records
     that the watchdog was the one that stopped it;
   - once free climbs back to **RESUME** (default 30 GiB), restarts the
     runner if the watchdog had stopped it. This gives natural hysteresis.

## Files

| File | Goes to | Purpose |
|------|---------|---------|
| `disk-watchdog.sh`      | `/usr/local/bin/disk-watchdog.sh`           | The script run by the timer |
| `disk-watchdog.service` | `/etc/systemd/system/disk-watchdog.service` | Oneshot service unit |
| `disk-watchdog.timer`   | `/etc/systemd/system/disk-watchdog.timer`   | Fires every minute |
| `install-on-runner.sh`  | run from repo                                | Idempotent installer |
| `recover-disk-wedge.sh` | run from repo                                | Post-reboot recovery |
| `README.md`             | `/usr/local/share/doc/disk-safety/README.md` | This file |

## Install

On the runner host (`maitai-eos` or `leabs-dev`), with the repo checked out:

```bash
sudo bash scripts/ops/disk-safety/install-on-runner.sh
```

The installer is idempotent — re-run it after a `git pull` to refresh.
It does not clobber `/etc/default/disk-watchdog` once seeded.

## Tune

Override defaults in `/etc/default/disk-watchdog`. All thresholds are in
GiB. Examples:

```bash
WARN_GIB=30
CRIT_GIB=15
STALE_MINUTES=20
TARGETS=/home/maitai/code/rust-daq/target,/home/maitai/actions-runner-rust-daq/_work/rust-daq/rust-daq/target
RUNNER_SERVICE=actions.runner.TheFermiSea-rust-daq.maitai-eos.service
```

The watchdog auto-discovers the runner service if `RUNNER_SERVICE` is
unset. It auto-discovers nothing about target dirs — you must list them.

## Verify

```bash
# Tail the watchdog log
journalctl -t disk-watchdog -f

# See the timer schedule
systemctl status disk-watchdog.timer

# Force a synchronous run
sudo systemctl start disk-watchdog.service
journalctl -t disk-watchdog -n 5
```

## Recover from a wedge

If the host has already wedged (sshd/journald frozen), there is no remote
fix — the disk has to be freed before anything starts working again.
Power-cycle the box, log in locally, then:

```bash
sudo bash scripts/ops/disk-safety/recover-disk-wedge.sh
```

That script frees the known offenders, vacuums journald, (re)installs the
watchdog, and restarts the runner. Idempotent.

## Limitations

- The watchdog is **per host**. It cannot prevent a single workflow from
  filling its own job-local target/ if a single test produces tens of
  gigabytes — the workflow-level reclaim-disk action exists for that.
- If a developer creates a brand-new `target/` outside the tracked list,
  the watchdog won't clean it. Add new paths to `TARGETS=` in
  `/etc/default/disk-watchdog` when you check out a new repo.
- `STALE_MINUTES=30` is conservative; if your typical compile is
  longer-running than that, raise it to avoid mid-build cleanup at WARN.
  CRIT always wins regardless.
