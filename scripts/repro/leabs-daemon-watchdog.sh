#!/usr/bin/env bash
# Local watchdog for rust-daq-daemon on leabs-dev with crash-wrapper integration.
#
# This script is intended for long interactive lab sessions (GUI/manual use). It:
# - ensures a local SSH tunnel to the remote daemon
# - starts/restarts the daemon under scripts/repro/leabs-daemon-crash-wrapper.sh
# - polls daemon health via gRPC
# - logs restart events and collects wrapper/journal artifacts on every restart
#
# It does NOT stream frames itself; use it alongside the GUI or other clients.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PROTO_DIR="${ROOT_DIR}/crates/protocol/proto"
REMOTE_WRAPPER="${ROOT_DIR}/scripts/repro/leabs-daemon-crash-wrapper.sh"

SSH_HOST="leabs-dev"
REMOTE_DIR="/home/brian/code/rust-daq"
ENV_FILE="config/hosts/leabs-dev.env"
HARDWARE_CONFIG="config/leabs_hardware.toml"
REMOTE_CAPTURE_ROOT="/tmp/rust-daq-daemon-crash"

PORT=50051
RUNTIME_MODE="universal"
CHECK_INTERVAL_SECONDS=5
RESTART_COOLDOWN_SECONDS=3
BUILD_REMOTE_ON_START=false
AUTO_RESTART=true
ENSURE_TUNNEL=true
RUN_ONCE=false

LOCAL_RUN_ROOT="${ROOT_DIR}/tmp/leabs-daemon-watchdog"

usage() {
  cat <<'EOF'
Usage: leabs-daemon-watchdog.sh [options]

Options:
  --ssh-host <host>              SSH host (default: leabs-dev)
  --remote-dir <path>            Remote rust-daq repo path
  --env-file <path>              Remote env file to source (default: config/hosts/leabs-dev.env)
  --hardware-config <path>       Remote hardware config path
  --capture-root <path>          Remote crash wrapper capture root
  --port <n>                     Daemon port (default: 50051)
  --runtime-mode <mode>          Daemon runtime mode (default: universal)
  --check-interval <n>           Health poll cadence seconds (default: 5)
  --restart-cooldown <n>         Sleep after restart before health polls (default: 3)
  --build-remote-on-start        Rebuild daemon before initial start
  --no-auto-restart              Exit on health loss instead of restarting
  --no-tunnel                    Do not manage local SSH tunnel
  --once                         Start/check/collect once, then exit
  -h, --help                     Show help

Artifacts:
  tmp/leabs-daemon-watchdog/<run-id>/
    watchdog.log
    events.csv
    remote_wrapper_session_latest/
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --ssh-host) SSH_HOST="$2"; shift 2 ;;
    --remote-dir) REMOTE_DIR="$2"; shift 2 ;;
    --env-file) ENV_FILE="$2"; shift 2 ;;
    --hardware-config) HARDWARE_CONFIG="$2"; shift 2 ;;
    --capture-root) REMOTE_CAPTURE_ROOT="$2"; shift 2 ;;
    --port) PORT="$2"; shift 2 ;;
    --runtime-mode) RUNTIME_MODE="$2"; shift 2 ;;
    --check-interval) CHECK_INTERVAL_SECONDS="$2"; shift 2 ;;
    --restart-cooldown) RESTART_COOLDOWN_SECONDS="$2"; shift 2 ;;
    --build-remote-on-start) BUILD_REMOTE_ON_START=true; shift ;;
    --no-auto-restart) AUTO_RESTART=false; shift ;;
    --no-tunnel) ENSURE_TUNNEL=false; shift ;;
    --once) RUN_ONCE=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

mkdir -p "$LOCAL_RUN_ROOT"
RUN_ID="$(date +%Y%m%dT%H%M%S%z)"
RUN_DIR="${LOCAL_RUN_ROOT}/${RUN_ID}"
mkdir -p "$RUN_DIR"

LOG_FILE="${RUN_DIR}/watchdog.log"
EVENTS_CSV="${RUN_DIR}/events.csv"

exec > >(tee -a "$LOG_FILE") 2>&1

log() {
  printf '[%s] %s\n' "$(date '+%F %T')" "$*"
}

iso_now() {
  date '+%Y-%m-%dT%H:%M:%S%z'
}

ssh_remote() {
  ssh -o BatchMode=yes -o ConnectTimeout=10 "$SSH_HOST" "$@"
}

grpcurl_cmd() {
  grpcurl -plaintext \
    -connect-timeout 2 \
    -import-path "$PROTO_DIR" \
    -proto daq.proto \
    -max-msg-sz 67108864 \
    "$@"
}

health_check() {
  local rc
  set +e
  grpcurl_cmd -max-time 4 "127.0.0.1:${PORT}" daq.ControlService/GetDaemonInfo >/dev/null 2>&1
  rc=$?
  set -e
  return "$rc"
}

ensure_tunnel() {
  if ! $ENSURE_TUNNEL; then
    return 0
  fi
  if health_check; then
    log "Reusing existing healthy local tunnel on 127.0.0.1:${PORT}"
    return 0
  fi

  local tunnel_pattern
  tunnel_pattern="[s]sh .* -L 127.0.0.1:${PORT}:localhost:${PORT} ${SSH_HOST}"
  if pgrep -f "$tunnel_pattern" >/dev/null 2>&1; then
    log "Replacing stale local SSH tunnel(s) on 127.0.0.1:${PORT}"
    pkill -f "$tunnel_pattern" >/dev/null 2>&1 || true
    sleep 1
  fi
  log "Starting SSH tunnel 127.0.0.1:${PORT} -> ${SSH_HOST}:localhost:${PORT}"
  ssh -fN \
    -o ExitOnForwardFailure=yes \
    -o ServerAliveInterval=30 \
    -o ServerAliveCountMax=3 \
    -L "127.0.0.1:${PORT}:localhost:${PORT}" \
    "$SSH_HOST"
}

install_remote_wrapper() {
  log "Installing crash wrapper on remote"
  scp -q "$REMOTE_WRAPPER" "${SSH_HOST}:${REMOTE_DIR}/scripts/leabs-daemon-crash-wrapper.sh"
  ssh_remote bash -s -- "$REMOTE_DIR" <<'EOS'
set -euo pipefail
remote_dir="$1"
chmod +x "${remote_dir}/scripts/leabs-daemon-crash-wrapper.sh"
EOS
}

start_wrapped_daemon() {
  local build_remote="$1"
  local daemon_cmd
  if [[ "${RUNTIME_MODE}" != "universal" ]]; then
    log "ERROR: RUNTIME_MODE='${RUNTIME_MODE}' is not supported with --hardware-config. Use 'universal'."
    exit 2
  fi
  # Current daemon CLI rejects --hardware-config combined with --runtime-mode on this path.
  daemon_cmd=(
    "./target/release/rust-daq-daemon"
    "daemon"
    "--port" "${PORT}"
    "--hardware-config" "${HARDWARE_CONFIG}"
  )

  log "Stopping any existing remote daemon"
  ssh_remote "pkill -f '[r]ust-daq-daemon daemon' >/dev/null 2>&1 || true"

  if [[ "$build_remote" == "true" ]]; then
    log "Building remote daemon (features: leabs_hardware)"
    ssh_remote bash -s -- "$REMOTE_DIR" "$ENV_FILE" <<'EOS'
set -euo pipefail
remote_dir="$1"
env_file="$2"
source "$HOME/.cargo/env"
cd "$remote_dir"
source "$env_file"
cargo build --release --bin rust-daq-daemon --features leabs_hardware
EOS
  fi

  log "Starting wrapped daemon on remote"
  ssh_remote bash -s -- \
    "$REMOTE_DIR" \
    "$ENV_FILE" \
    "$REMOTE_CAPTURE_ROOT" \
    "$PORT" \
    "$HARDWARE_CONFIG" <<'EOS' | tee -a "${RUN_DIR}/wrapper_session_paths.log"
set -euo pipefail
remote_dir="$1"
env_file="$2"
capture_root="$3"
port="$4"
hardware_config="$5"
daemon_cmd=(
  "./target/release/rust-daq-daemon"
  "daemon"
  "--port" "$port"
  "--hardware-config" "$hardware_config"
)
source "$HOME/.cargo/env"
cd "$remote_dir"
source "$env_file"
setsid -f bash scripts/leabs-daemon-crash-wrapper.sh \
  --capture-root "$capture_root" \
  --label watchdog \
  -- "${daemon_cmd[@]}" \
  > /tmp/rust-daq-daemon-wrapper.out 2>&1 < /dev/null
sleep 1
cat "${capture_root}/latest_session_path.txt" 2>/dev/null || true
EOS
}

wait_for_daemon() {
  local attempts="${1:-60}"
  local i
  for i in $(seq 1 "$attempts"); do
    if health_check; then
      log "Daemon health check OK"
      return 0
    fi
    sleep 1
  done
  log "Daemon failed health check"
  return 1
}

collect_remote_state() {
  local tag="$1"
  local out_dir="${RUN_DIR}/events/${tag}"
  mkdir -p "$out_dir"

  ssh_remote "pgrep -af '[r]ust-daq-daemon daemon' || true" > "${out_dir}/pgrep.txt" || true
  ssh_remote "tail -200 /tmp/rust-daq-daemon-wrapper.out 2>/dev/null || true" > "${out_dir}/wrapper.tail.txt" || true
  ssh_remote "tail -200 /tmp/rust-daq-daemon.log 2>/dev/null || true" > "${out_dir}/daemon.tail.txt" || true
  ssh_remote "journalctl -n 400 --no-pager 2>/dev/null || true" > "${out_dir}/journal.tail.txt" || true
  ssh_remote "cat ${REMOTE_CAPTURE_ROOT}/latest_session_path.txt 2>/dev/null || true" > "${out_dir}/latest_session_path.txt" || true

  local remote_session_dir
  remote_session_dir="$(tail -n1 "${out_dir}/latest_session_path.txt" 2>/dev/null | tr -d '\r')"
  if [[ -n "$remote_session_dir" ]]; then
    mkdir -p "${out_dir}/remote_wrapper_session"
    scp -q -r "${SSH_HOST}:${remote_session_dir}/." "${out_dir}/remote_wrapper_session/" || true
    rm -rf "${RUN_DIR}/remote_wrapper_session_latest"
    mkdir -p "${RUN_DIR}/remote_wrapper_session_latest"
    cp -R "${out_dir}/remote_wrapper_session/." "${RUN_DIR}/remote_wrapper_session_latest/" || true
  fi
}

record_event() {
  local kind="$1"
  local detail="$2"
  printf '%s,%s,%s\n' "$(iso_now)" "$kind" "$(printf '%s' "$detail" | tr '\n' ' ')" >> "$EVENTS_CSV"
}

printf 'timestamp_iso,event,detail\n' > "$EVENTS_CSV"
log "Watchdog run dir: ${RUN_DIR}"

ensure_tunnel
install_remote_wrapper
start_wrapped_daemon "$BUILD_REMOTE_ON_START"
wait_for_daemon 60
record_event "daemon_started" "initial wrapped daemon start complete"
collect_remote_state "startup"

if $RUN_ONCE; then
  log "Run-once mode complete"
  exit 0
fi

consecutive_failures=0
restart_count=0

while :; do
  if health_check; then
    consecutive_failures=0
    log "Health OK"
    record_event "health_ok" "daemon reachable on localhost:${PORT}"
  else
    consecutive_failures=$((consecutive_failures + 1))
    log "Health FAILED (consecutive=${consecutive_failures})"
    record_event "health_failed" "consecutive=${consecutive_failures}"
    collect_remote_state "health_failed_$(date +%Y%m%dT%H%M%S)"

    if ! $AUTO_RESTART; then
      log "Auto-restart disabled; exiting on health failure"
      exit 1
    fi

    restart_count=$((restart_count + 1))
    log "Restarting daemon (restart_count=${restart_count})"
    record_event "daemon_restart" "restart_count=${restart_count}"
    start_wrapped_daemon false
    sleep "$RESTART_COOLDOWN_SECONDS"
    if wait_for_daemon 30; then
      record_event "daemon_recovered" "restart_count=${restart_count}"
      collect_remote_state "recovered_$(date +%Y%m%dT%H%M%S)"
    else
      record_event "daemon_restart_failed" "restart_count=${restart_count}"
      collect_remote_state "restart_failed_$(date +%Y%m%dT%H%M%S)"
    fi
  fi
  sleep "$CHECK_INTERVAL_SECONDS"
done
