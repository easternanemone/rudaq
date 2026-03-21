#!/usr/bin/env bash
# Collect diagnostic information after a rust-daq-daemon crash has already happened.
# Complementary to `scripts/repro/leabs-daemon-crash-wrapper.sh` (which wraps a live
# daemon process to capture artifacts at crash-time). This script is run manually
# after discovering a crash — it vacuums up system state, journal excerpts,
# core dump metadata, network state, and deployment info into a single timestamped
# directory for offline analysis.
#
# Works on both maitai and leabs-dev Linux machines. Each collection step is
# best-effort (`|| true`) so the script never fails on missing commands.
#
# Examples:
#   bash scripts/ops/post-crash-forensics.sh
#   bash scripts/ops/post-crash-forensics.sh --since "2h ago" --output-dir /data/crashes
#   bash scripts/ops/post-crash-forensics.sh --service-name rust-daq-daemon

set -euo pipefail

# ── defaults ──────────────────────────────────────────────────────────────────
OUTPUT_DIR="/tmp/rust-daq-forensics"
SERVICE_NAME="rust-daq-daemon"
SINCE="1h ago"

# ── option parsing ────────────────────────────────────────────────────────────
usage() {
  cat <<'EOF'
Usage: post-crash-forensics.sh [options]

Collect post-crash diagnostic artifacts into a timestamped directory.

Options:
  --output-dir <dir>      Root output directory (default: /tmp/rust-daq-forensics)
  --service-name <name>   systemd service unit name (default: rust-daq-daemon)
  --since <timespec>      How far back to look in journals (default: "1h ago")
  -h, --help              Show help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output-dir)
      OUTPUT_DIR="$2"
      shift 2
      ;;
    --service-name)
      SERVICE_NAME="$2"
      shift 2
      ;;
    --since)
      SINCE="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

# ── timestamped session directory ─────────────────────────────────────────────
ts="$(date +%Y%m%dT%H%M%S%z)"
SESSION_DIR="${OUTPUT_DIR}/forensics_${ts}_$$"
mkdir -p "$SESSION_DIR"

# Locate the repo root (script lives in <repo>/scripts/)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Track red flags for the summary
RED_FLAGS=()

log() {
  printf '[forensics] %s\n' "$*"
}

log "session directory: $SESSION_DIR"
log "service name:      $SERVICE_NAME"
log "journal lookback:  $SINCE"
log "repo root:         $REPO_ROOT"
echo

# ── collect: system state ─────────────────────────────────────────────────────
collect_system_state() {
  log "collecting system state..."
  local out="${SESSION_DIR}/system_state.txt"
  {
    echo "# uname -a"
    uname -a || true
    echo

    echo "# uptime"
    uptime || true
    echo

    echo "# free -m"
    free -m 2>/dev/null || echo "(free not available)" || true
    echo

    echo "# df -h (filtered)"
    df -h 2>/dev/null | grep -E '(Filesystem|/$|/tmp|/home|/data|/var)' || echo "(df not available)" || true
    echo

    echo "# /proc/sys/kernel/core_pattern"
    cat /proc/sys/kernel/core_pattern 2>/dev/null || echo "(not available — not Linux or no /proc)" || true
    echo

    echo "# sysctl kernel.nmi_watchdog kernel.hung_task_panic"
    sysctl kernel.nmi_watchdog kernel.hung_task_panic 2>/dev/null || echo "(sysctl not available)" || true
  } > "$out"
}

# ── collect: service state ────────────────────────────────────────────────────
collect_service_state() {
  log "collecting service state..."
  local status_out="${SESSION_DIR}/service_status.txt"
  local journal_out="${SESSION_DIR}/service_journal.log"
  local journal_err_out="${SESSION_DIR}/service_journal_errors.log"

  {
    echo "# systemctl --user status $SERVICE_NAME"
    systemctl --user status "$SERVICE_NAME" 2>&1 || true
  } > "$status_out"

  # Detect failed state
  if grep -qiE '(failed|inactive \(dead\))' "$status_out" 2>/dev/null; then
    RED_FLAGS+=("service '$SERVICE_NAME' is in failed/dead state")
  fi

  {
    echo "# journalctl --user -u $SERVICE_NAME --since \"$SINCE\" --no-pager"
    journalctl --user -u "$SERVICE_NAME" --since "$SINCE" --no-pager 2>&1 || true
  } > "$journal_out"

  {
    echo "# journalctl --user -u $SERVICE_NAME --since \"$SINCE\" --priority=err --no-pager"
    journalctl --user -u "$SERVICE_NAME" --since "$SINCE" --priority=err --no-pager 2>&1 || true
  } > "$journal_err_out"
}

# ── collect: kernel messages ──────────────────────────────────────────────────
collect_kernel_messages() {
  log "collecting kernel messages..."
  local dmesg_out="${SESSION_DIR}/dmesg.log"
  local dmesg_flags_out="${SESSION_DIR}/dmesg_red_flags.log"

  {
    echo "# dmesg (recent)"
    # Prefer --time-format iso; fall back to -T (human-readable timestamps)
    if dmesg --time-format iso 2>/dev/null | tail -n 500; then
      :
    else
      dmesg -T 2>/dev/null | tail -n 500 || dmesg 2>/dev/null | tail -n 500 || echo "(dmesg not available)" || true
    fi
  } > "$dmesg_out"

  {
    echo "# dmesg grep: OOM, panic, watchdog, segfault, killed"
    grep -iE '(oom|out of memory|panic|watchdog|segfault|killed process|invoked oom-killer)' "$dmesg_out" 2>/dev/null || echo "(no matches)"
  } > "$dmesg_flags_out"

  # Record red flags from dmesg
  if grep -qiE '(oom|out of memory|invoked oom-killer)' "$dmesg_out" 2>/dev/null; then
    RED_FLAGS+=("OOM killer activity detected in dmesg")
  fi
  if grep -qi 'segfault' "$dmesg_out" 2>/dev/null; then
    RED_FLAGS+=("segfault detected in dmesg")
  fi
  if grep -qi 'watchdog' "$dmesg_out" 2>/dev/null; then
    RED_FLAGS+=("watchdog trigger detected in dmesg")
  fi
}

# ── collect: core dump info ───────────────────────────────────────────────────
collect_coredump_info() {
  log "collecting core dump info..."
  local out="${SESSION_DIR}/coredumpctl.txt"
  local cores_out="${SESSION_DIR}/core_files.txt"

  {
    echo "# coredumpctl list --no-pager"
    coredumpctl list --no-pager 2>&1 || echo "(coredumpctl not available)" || true
    echo

    echo "# coredumpctl info $SERVICE_NAME --no-pager"
    coredumpctl info "$SERVICE_NAME" --no-pager 2>&1 || echo "(no coredump for $SERVICE_NAME)" || true
  } > "$out"

  {
    echo "# core files in common locations"
    for search_dir in . /tmp /var/lib/systemd/coredump /var/crash "$HOME"; do
      find "$search_dir" -maxdepth 3 -type f \( -name 'core' -o -name 'core.*' -o -name '*.core' \) \
        -printf '%TY-%Tm-%Td %TT %p %s bytes\n' 2>/dev/null || true
    done
  } > "$cores_out"

  if [[ -s "$cores_out" ]] && grep -q 'bytes' "$cores_out" 2>/dev/null; then
    RED_FLAGS+=("core dump file(s) found — see core_files.txt")
  fi
}

# ── collect: process and network state ────────────────────────────────────────
collect_process_network_state() {
  log "collecting process/network state..."
  local net_out="${SESSION_DIR}/network_state.txt"
  local proc_out="${SESSION_DIR}/process_state.txt"

  {
    echo "# ss -tlnp (listening sockets)"
    ss -tlnp 2>/dev/null || echo "(ss not available)" || true
    echo

    echo "# port 50051 specifically"
    ss -tlnp 2>/dev/null | grep -E '(:50051\b)' || echo "(port 50051 not listening)" || true
  } > "$net_out"

  {
    echo "# rust-daq processes"
    ps aux 2>/dev/null | grep -E '(rust-daq|PID)' | grep -v 'grep' || echo "(no rust-daq processes found)" || true
    echo

    echo "# lsof open file descriptor count for rust-daq"
    local fd_count
    fd_count="$(lsof -c rust-daq 2>/dev/null | wc -l || echo 0)"
    printf 'open fds: %s\n' "$fd_count"
  } > "$proc_out"
}

# ── collect: deployment info ──────────────────────────────────────────────────
collect_deployment_info() {
  log "collecting deployment info..."
  local out="${SESSION_DIR}/deployment_info.txt"

  {
    echo "# git revision"
    git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo "(not a git repo or git unavailable)" || true
    echo

    echo "# git branch"
    git -C "$REPO_ROOT" branch --show-current 2>/dev/null || true
    echo

    echo "# git status (short)"
    git -C "$REPO_ROOT" status --short 2>/dev/null || true
    echo

    echo "# git log -5 --oneline"
    git -C "$REPO_ROOT" log -5 --oneline 2>/dev/null || true
  } > "$out"

  # Copy hardware config files if they exist
  local hw_configs=(
    "config/leabs_hardware.toml"
    "config/maitai_universal.toml"
    "config/libs_hardware.toml"
  )
  for cfg in "${hw_configs[@]}"; do
    local full_path="${REPO_ROOT}/${cfg}"
    if [[ -f "$full_path" ]]; then
      local basename
      basename="$(basename "$cfg")"
      cp "$full_path" "${SESSION_DIR}/config_${basename}" 2>/dev/null || true
      log "  copied $cfg"
    fi
  done

  # Copy feature flags if they exist
  local ff_path="${REPO_ROOT}/config/feature_flags.toml"
  if [[ -f "$ff_path" ]]; then
    cp "$ff_path" "${SESSION_DIR}/config_feature_flags.toml" 2>/dev/null || true
    log "  copied config/feature_flags.toml"
  fi
}

# ── summary ───────────────────────────────────────────────────────────────────
print_summary() {
  local summary_out="${SESSION_DIR}/SUMMARY.txt"

  {
    echo "═══════════════════════════════════════════════════════════════"
    echo "  Post-Crash Forensic Collection Summary"
    echo "  $(date --iso-8601=seconds 2>/dev/null || date '+%Y-%m-%dT%H:%M:%S%z')"
    echo "═══════════════════════════════════════════════════════════════"
    echo
    echo "Session directory: $SESSION_DIR"
    echo "Service name:      $SERVICE_NAME"
    echo "Journal lookback:  $SINCE"
    echo
    echo "── Collected Files ──"
    ls -1 "$SESSION_DIR" 2>/dev/null || true
    echo
    echo "── Red Flags ──"
    if [[ ${#RED_FLAGS[@]} -eq 0 ]]; then
      echo "  (none detected)"
    else
      for flag in "${RED_FLAGS[@]}"; do
        printf '  *** %s\n' "$flag"
      done
    fi
    echo
    echo "═══════════════════════════════════════════════════════════════"
  } | tee "$summary_out"
}

# ── main ──────────────────────────────────────────────────────────────────────
collect_system_state
collect_service_state
collect_kernel_messages
collect_coredump_info
collect_process_network_state
collect_deployment_info

echo
print_summary

log "done. All artifacts written to: $SESSION_DIR"
