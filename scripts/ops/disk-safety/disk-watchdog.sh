#!/bin/bash
# disk-watchdog.sh — runs every minute via systemd timer.
#
# Watches free space on the root filesystem of a self-hosted runner host.
# At a "warn" threshold it deletes stale cargo target/ dirs (no recent mtime).
# At a "critical" threshold it stops the GitHub runner service and deletes
# every target/ dir it knows about, then waits for headroom before letting
# the runner come back.
#
# Why this exists: the rust-daq self-hosted runners (maitai-eos, leabs-dev)
# share the `leabs` label, so concurrent CI jobs and developer-launched
# builds can both write to target/ on the same filesystem. Without an
# external watchdog, neither participant notices the other and the host can
# wedge at 100% disk — sshd, journald, and the runner itself fail to write,
# requiring a physical reboot.
#
# All thresholds are tunable via /etc/default/disk-watchdog.

set -euo pipefail

PARTITION="${PARTITION:-/}"
WARN_GIB="${WARN_GIB:-25}"
CRIT_GIB="${CRIT_GIB:-10}"
RESUME_GIB="${RESUME_GIB:-30}"     # restart runner once free climbs back here
STALE_MINUTES="${STALE_MINUTES:-30}"  # at WARN, only clean targets idle this long

STATE_DIR="${STATE_DIR:-/var/lib/disk-watchdog}"
STOP_FLAG="$STATE_DIR/runner-stopped-by-watchdog"

# Auto-discover the GitHub Actions runner systemd unit if not set explicitly.
if [[ -z "${RUNNER_SERVICE:-}" ]]; then
  RUNNER_SERVICE=$(systemctl list-unit-files --type=service --no-legend 2>/dev/null \
    | awk '/^actions\.runner\..*\.service/ {print $1; exit}')
fi

# Comma-separated list of target/ dirs to clean.
TARGETS_DEFAULT="/home/maitai/code/rust-daq/target,/home/maitai/actions-runner-rust-daq/_work/rust-daq/rust-daq/target,/home/brian/code/rust-daq/target,/home/brian/actions-runner-rust-daq/_work/rust-daq/rust-daq/target"
TARGETS_RAW="${TARGETS:-$TARGETS_DEFAULT}"
IFS=',' read -ra TARGETS_ARR <<<"$TARGETS_RAW"

mkdir -p "$STATE_DIR"

log() { logger -t disk-watchdog -- "$1"; }

avail_gib() {
  local kb
  kb=$(df --output=avail "$PARTITION" | tail -1 | tr -d ' ')
  echo $(( kb / 1024 / 1024 ))
}

dir_has_recent_activity() {
  # Returns 0 if any file under $1 has been modified within $STALE_MINUTES.
  local d="$1"
  [[ -d "$d" ]] || return 1
  find "$d" -mmin "-$STALE_MINUTES" -print -quit 2>/dev/null | grep -q .
}

clean_target() {
  local t="$1"
  [[ -d "$t" ]] || return 0
  local sz
  sz=$(du -sh "$t" 2>/dev/null | awk '{print $1}')
  log "removing $t (size=$sz)"
  rm -rf -- "$t"
}

stop_runner() {
  [[ -n "${RUNNER_SERVICE:-}" ]] || { log "no runner service detected; skip stop"; return; }
  if systemctl is-active --quiet "$RUNNER_SERVICE"; then
    log "stopping $RUNNER_SERVICE"
    systemctl stop "$RUNNER_SERVICE" || log "stop failed"
    : > "$STOP_FLAG"
  fi
}

maybe_restart_runner() {
  [[ -f "$STOP_FLAG" ]] || return 0
  [[ -n "${RUNNER_SERVICE:-}" ]] || return 0
  log "free space recovered; starting $RUNNER_SERVICE"
  systemctl start "$RUNNER_SERVICE" || log "start failed"
  rm -f "$STOP_FLAG"
}

main() {
  local g
  g=$(avail_gib)
  log "free=${g}GiB partition=$PARTITION warn=${WARN_GIB} crit=${CRIT_GIB} runner_service=${RUNNER_SERVICE:-<none>}"

  if (( g < CRIT_GIB )); then
    log "CRITICAL (${g}GiB < ${CRIT_GIB}GiB): stopping runner; force-cleaning all targets"
    stop_runner
    for t in "${TARGETS_ARR[@]}"; do clean_target "$t"; done
    log "after critical clean: free=$(avail_gib)GiB"
    return
  fi

  if (( g < WARN_GIB )); then
    log "WARN (${g}GiB < ${WARN_GIB}GiB): cleaning idle targets only"
    for t in "${TARGETS_ARR[@]}"; do
      if dir_has_recent_activity "$t"; then
        log "skipping $t (active in last ${STALE_MINUTES}min)"
        continue
      fi
      clean_target "$t"
    done
    log "after warn clean: free=$(avail_gib)GiB"
    return
  fi

  # Healthy. If we previously stopped the runner, see if we have enough room to bring it back.
  if (( g >= RESUME_GIB )); then
    maybe_restart_runner
  fi
}

main
