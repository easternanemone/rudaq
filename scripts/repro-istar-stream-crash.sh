#!/usr/bin/env bash
# Reproduce and capture iSTAR streaming daemon crashes on leabs-dev.
#
# This script:
#  1) (optionally) restarts the daemon on leabs-dev under the crash wrapper
#  2) ensures a local SSH tunnel to the daemon
#  3) drives StartStream + StreamFrames via grpcurl for a configurable soak duration
#  4) polls daemon health while streaming
#  5) collects remote crash artifacts and local command logs
#
# Requires:
#  - local: ssh, grpcurl, timeout
#  - remote (leabs-dev): rust-daq repo + built daemon binary + grpcurl (optional)

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROTO_DIR="${ROOT_DIR}/crates/protocol/proto"

SSH_HOST="leabs-dev"
REMOTE_DIR="/home/brian/code/rust-daq"
ENV_FILE="config/hosts/leabs-dev.env"
HARDWARE_CONFIG="config/leabs_hardware.toml"
REMOTE_CAPTURE_ROOT="/tmp/rust-daq-daemon-crash"
LOCAL_ARTIFACT_ROOT="${ROOT_DIR}/tmp/istar-crash-repro"

PORT=50051
DEVICE_ID="istar_camera"
EXPOSURE_MS="10"
QUALITY="FAST"   # FULL | PREVIEW | FAST
MAX_FPS=10
SOAK_SECONDS=600
HEALTH_POLL_SECONDS=5
RUNTIME_MODE="universal"

RESTART_DAEMON=true
STOP_DAEMON_ON_EXIT=false
BUILD_REMOTE=false

usage() {
  cat <<'EOF'
Usage: repro-istar-stream-crash.sh [options]

Options:
  --ssh-host <host>           SSH host (default: leabs-dev)
  --remote-dir <path>         Remote rust-daq repo path
  --port <n>                  Daemon port (default: 50051)
  --device <id>               Camera device id (default: istar_camera)
  --exposure-ms <ms>          Exposure to set before stream (default: 10)
  --quality <full|preview|fast>
  --max-fps <n>               Stream max_fps request (default: 10)
  --soak-seconds <n>          Stream soak duration (default: 600)
  --health-poll-seconds <n>   Health check cadence (default: 5)
  --runtime-mode <mode>       Daemon runtime mode (default: universal)
  --build-remote              Build remote daemon before running
  --no-restart-daemon         Reuse currently running remote daemon
  --stop-daemon-on-exit       Stop remote daemon at end of script
  -h, --help                  Show help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --ssh-host) SSH_HOST="$2"; shift 2 ;;
    --remote-dir) REMOTE_DIR="$2"; shift 2 ;;
    --port) PORT="$2"; shift 2 ;;
    --device) DEVICE_ID="$2"; shift 2 ;;
    --exposure-ms) EXPOSURE_MS="$2"; shift 2 ;;
    --quality) QUALITY="$(echo "$2" | tr '[:lower:]' '[:upper:]')"; shift 2 ;;
    --max-fps) MAX_FPS="$2"; shift 2 ;;
    --soak-seconds) SOAK_SECONDS="$2"; shift 2 ;;
    --health-poll-seconds) HEALTH_POLL_SECONDS="$2"; shift 2 ;;
    --runtime-mode) RUNTIME_MODE="$2"; shift 2 ;;
    --build-remote) BUILD_REMOTE=true; shift ;;
    --no-restart-daemon) RESTART_DAEMON=false; shift ;;
    --stop-daemon-on-exit) STOP_DAEMON_ON_EXIT=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

case "$QUALITY" in
  FULL) QUALITY_ENUM="STREAM_QUALITY_FULL" ;;
  PREVIEW) QUALITY_ENUM="STREAM_QUALITY_PREVIEW" ;;
  FAST) QUALITY_ENUM="STREAM_QUALITY_FAST" ;;
  *) echo "Invalid --quality '$QUALITY' (use full|preview|fast)" >&2; exit 2 ;;
esac

mkdir -p "$LOCAL_ARTIFACT_ROOT"
RUN_ID="$(date +%Y%m%dT%H%M%S%z)"
RUN_DIR="${LOCAL_ARTIFACT_ROOT}/${RUN_ID}"
mkdir -p "$RUN_DIR"

SUMMARY_FILE="${RUN_DIR}/summary.txt"
exec > >(tee -a "$SUMMARY_FILE") 2>&1

log() {
  printf '[%s] %s\n' "$(date '+%F %T')" "$*"
}

iso_now() {
  # Portable timestamp (GNU/Linux + macOS)
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
  python3 -c 'import socket,sys; p=int(sys.argv[1]); s=socket.socket(socket.AF_INET, socket.SOCK_STREAM); s.settimeout(2.0); ok=0
try:
    s.connect(("127.0.0.1", p))
except OSError:
    ok=1
finally:
    s.close()
sys.exit(ok)' "$PORT" >/dev/null 2>&1
  rc=$?
  set -e
  return "$rc"
}

ensure_tunnel() {
  if health_check; then
    log "Reusing existing healthy local tunnel on 127.0.0.1:${PORT}"
    return 0
  fi

  # Avoid lsof here: it has hung under detached nohup runs on macOS. Instead, best-effort
  # kill prior SSH tunnels for this exact local forward spec and recreate one.
  local tunnel_pattern
  tunnel_pattern="[s]sh .* -L 127.0.0.1:${PORT}:localhost:${PORT} ${SSH_HOST}"
  if pgrep -f "$tunnel_pattern" >/dev/null 2>&1; then
    log "Local tunnel health check failed; replacing existing SSH tunnel(s) for 127.0.0.1:${PORT}"
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
  log "Installing crash wrapper script on remote repo checkout"
  scp -q "${ROOT_DIR}/scripts/leabs-daemon-crash-wrapper.sh" \
    "${SSH_HOST}:${REMOTE_DIR}/scripts/leabs-daemon-crash-wrapper.sh"
  ssh_remote "chmod +x ${REMOTE_DIR}/scripts/leabs-daemon-crash-wrapper.sh"
}

remote_start_wrapped_daemon() {
  local daemon_cmd
  local remote_session_hint=""
  local remote_start_ec=0
  daemon_cmd="./target/release/rust-daq-daemon daemon --port ${PORT} --hardware-config ${HARDWARE_CONFIG}"

  log "Stopping any existing remote daemon"
  ssh_remote "pkill -f '[r]ust-daq-daemon daemon' >/dev/null 2>&1 || true"

  if $BUILD_REMOTE; then
    log "Building daemon on remote (features: leabs_hardware)"
    ssh_remote "bash -lc 'source \$HOME/.cargo/env && cd ${REMOTE_DIR} && source ${ENV_FILE} && cargo build --release --bin rust-daq-daemon --features leabs_hardware'"
  fi

  log "Starting wrapped daemon on remote"
  set +e
  remote_session_hint="$(ssh_remote "bash -lc '
    cd ${REMOTE_DIR} &&
    source \$HOME/.cargo/env &&
    source ${ENV_FILE} &&
    setsid -f bash scripts/leabs-daemon-crash-wrapper.sh \
      --capture-root ${REMOTE_CAPTURE_ROOT} \
      --label istar_stream_repro \
      -- ${daemon_cmd} \
      > /tmp/rust-daq-daemon-wrapper.out 2>&1 < /dev/null
    sleep 1
    cat ${REMOTE_CAPTURE_ROOT}/latest_session_path.txt 2>/dev/null || true
  '")"
  remote_start_ec=$?
  set -e

  printf '%s\n' "$remote_session_hint" | tee "${RUN_DIR}/remote_wrapper_session_hint.txt"
  if [[ $remote_start_ec -ne 0 ]]; then
    log "Remote wrapper launch ssh exited ${remote_start_ec} (continuing; wrapper session hint captured above)"
  fi
}

wait_for_daemon() {
  log "Waiting for daemon on 127.0.0.1:${PORT}"
  local i
  for i in $(seq 1 60); do
    if health_check; then
      log "Daemon health check OK"
      return 0
    fi
    sleep 1
  done
  log "Daemon failed health check after startup wait"
  return 1
}

collect_remote_artifacts() {
  log "Collecting remote crash artifacts"
  ssh_remote "cat ${REMOTE_CAPTURE_ROOT}/latest_session_path.txt 2>/dev/null || true" \
    | tee "${RUN_DIR}/remote_latest_session_path.txt"

  local remote_session_dir
  remote_session_dir="$(tail -n1 "${RUN_DIR}/remote_latest_session_path.txt" | tr -d '\r')"

  ssh_remote "tail -200 /tmp/rust-daq-daemon-wrapper.out 2>/dev/null || true" \
    > "${RUN_DIR}/remote_wrapper.out.tail.txt" || true
  ssh_remote "tail -200 /tmp/rust-daq-daemon.log 2>/dev/null || true" \
    > "${RUN_DIR}/remote_daemon.log.tail.txt" || true
  ssh_remote "journalctl -n 200 --no-pager 2>/dev/null || true" \
    > "${RUN_DIR}/remote_journal.tail.txt" || true

  if [[ -n "$remote_session_dir" ]]; then
    log "Fetching wrapper session artifacts from ${remote_session_dir}"
    mkdir -p "${RUN_DIR}/remote_wrapper_session"
    scp -q -r "${SSH_HOST}:${remote_session_dir}/." "${RUN_DIR}/remote_wrapper_session/" || true
  else
    log "No remote wrapper session path found"
  fi
}

best_effort_stop_stream() {
  grpcurl_cmd -d "{\"deviceId\":\"${DEVICE_ID}\"}" \
    "127.0.0.1:${PORT}" daq.HardwareService/StopStream >/dev/null 2>&1 || true
}

cleanup() {
  local ec=$?
  log "Cleanup (exit code=${ec})"
  best_effort_stop_stream
  if $STOP_DAEMON_ON_EXIT; then
    log "Stopping remote daemon (requested)"
    ssh_remote "pkill -f '[r]ust-daq-daemon daemon' >/dev/null 2>&1 || true" || true
  fi
  collect_remote_artifacts || true
  exit "$ec"
}
trap cleanup EXIT

log "Run dir: ${RUN_DIR}"
log "Config: device=${DEVICE_ID} quality=${QUALITY_ENUM} max_fps=${MAX_FPS} soak=${SOAK_SECONDS}s exposure=${EXPOSURE_MS}ms"

ensure_tunnel
install_remote_wrapper

if $RESTART_DAEMON; then
  remote_start_wrapped_daemon
fi
wait_for_daemon

log "Capturing daemon/device preflight info"
grpcurl_cmd "127.0.0.1:${PORT}" daq.ControlService/GetDaemonInfo \
  > "${RUN_DIR}/daemon_info.json" 2> "${RUN_DIR}/daemon_info.stderr.txt" || true
grpcurl_cmd -d '{}' "127.0.0.1:${PORT}" daq.HardwareService/ListDevices \
  > "${RUN_DIR}/list_devices.json" 2> "${RUN_DIR}/list_devices.stderr.txt" || true

log "Setting exposure to ${EXPOSURE_MS} ms (best effort)"
grpcurl_cmd -d "{\"deviceId\":\"${DEVICE_ID}\",\"exposureMs\":${EXPOSURE_MS}}" \
  "127.0.0.1:${PORT}" daq.HardwareService/SetExposure \
  > "${RUN_DIR}/set_exposure.json" 2> "${RUN_DIR}/set_exposure.stderr.txt" || true

log "StartStream on ${DEVICE_ID}"
grpcurl_cmd -d "{\"deviceId\":\"${DEVICE_ID}\"}" \
  "127.0.0.1:${PORT}" daq.HardwareService/StartStream \
  > "${RUN_DIR}/start_stream.json" 2> "${RUN_DIR}/start_stream.stderr.txt"

STREAM_STDERR="${RUN_DIR}/stream_frames.stderr.txt"
HEALTH_LOG="${RUN_DIR}/health_poll.log"
STREAM_STDOUT="${RUN_DIR}/stream_frames.stdout.discarded.note.txt"
printf 'StreamFrames stdout intentionally discarded to avoid large frame dumps.\n' > "$STREAM_STDOUT"

log "Starting StreamFrames soak for ${SOAK_SECONDS}s (stdout discarded; stderr logged)"
stream_cmd=(
  grpcurl
  -plaintext
  -import-path "$PROTO_DIR"
  -proto daq.proto
  -max-msg-sz 67108864
  -d "{\"deviceId\":\"${DEVICE_ID}\",\"maxFps\":${MAX_FPS},\"quality\":\"${QUALITY_ENUM}\"}"
  "127.0.0.1:${PORT}"
  daq.HardwareService/StreamFrames
)
set +e
timeout --signal=INT --kill-after=5s "${SOAK_SECONDS}" \
  "${stream_cmd[@]}" >/dev/null 2>"${STREAM_STDERR}" &
STREAM_PID=$!
set -e
printf '%s\n' "$STREAM_PID" > "${RUN_DIR}/stream_grpcurl.pid"

stream_exit_status="running"
daemon_died=false
start_watch_epoch="$(date +%s)"
while kill -0 "$STREAM_PID" 2>/dev/null; do
  if health_check; then
    printf '%s ok\n' "$(iso_now)" >> "$HEALTH_LOG"
  else
    printf '%s FAIL\n' "$(iso_now)" >> "$HEALTH_LOG"
    daemon_died=true
    log "Health check failed while stream process still running"
    break
  fi
  sleep "$HEALTH_POLL_SECONDS"
done

if $daemon_died && kill -0 "$STREAM_PID" 2>/dev/null; then
  kill "$STREAM_PID" >/dev/null 2>&1 || true
fi

set +e
wait "$STREAM_PID"
stream_ec=$?
set -e
printf 'stream_grpcurl_exit_code=%s\n' "$stream_ec" > "${RUN_DIR}/stream_exit.env"

elapsed_watch=$(( $(date +%s) - start_watch_epoch ))
log "Stream process exit code: ${stream_ec} (watch duration ${elapsed_watch}s)"

if ! $daemon_died; then
  if health_check; then
    log "Daemon still healthy after stream run"
  else
    daemon_died=true
    log "Daemon health check failed after stream process exited"
  fi
fi

best_effort_stop_stream

if $daemon_died; then
  log "RESULT: reproduced daemon crash/health loss during stream soak"
  exit 1
fi

log "RESULT: daemon remained healthy for configured soak duration"
