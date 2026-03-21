#!/usr/bin/env bash
# Run an overnight iSTAR stream repro matrix on leabs-dev using the existing repro harness.
#
# This script repeatedly invokes scripts/repro/repro-istar-stream-crash.sh across a parameter grid,
# records case results, and automatically restarts/rebuilds the daemon under the crash wrapper
# when needed. It is intended for long unattended soak runs.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REPRO_SCRIPT="${ROOT_DIR}/scripts/repro/repro-istar-stream-crash.sh"
LOCAL_RUN_ROOT="${ROOT_DIR}/tmp/istar-overnight-matrix"

SSH_HOST="leabs-dev"
REMOTE_DIR="/home/brian/code/rust-daq"
PORT=50051
DEVICE_ID="istar_camera"
RUNTIME_MODE="universal"
HEALTH_POLL_SECONDS=5

TOTAL_SECONDS=$((10 * 60 * 60))
SOAK_SECONDS=300
PAUSE_SECONDS=5
BATCH_SIZE=6
MAX_CASES=0
BUILD_REMOTE_ON_FIRST_START=true
STOP_ON_FIRST_FAILURE=false

QUALITIES=("full" "preview" "fast")
FPS_VALUES=("5" "10" "30")
EXPOSURE_VALUES=("1" "10" "100")

usage() {
  cat <<'EOF'
Usage: istar-stream-overnight-matrix.sh [options]

Runs repeated stream soak cases using scripts/repro/repro-istar-stream-crash.sh over a parameter matrix.

Options:
  --ssh-host <host>              SSH host for leabs machine (default: leabs-dev)
  --remote-dir <path>            Remote rust-daq checkout path
  --port <n>                     Daemon port (default: 50051)
  --device <id>                  Camera device id (default: istar_camera)
  --runtime-mode <mode>          Daemon runtime mode (default: universal)
  --hours <n>                    Total runtime in hours (default: 10)
  --total-seconds <n>            Total runtime in seconds (overrides --hours)
  --soak-seconds <n>             Per-case StreamFrames duration (default: 300)
  --pause-seconds <n>            Sleep between cases (default: 5)
  --batch-size <n>               Restart daemon every N cases (default: 6)
  --health-poll-seconds <n>      Repro harness health polling cadence (default: 5)
  --max-cases <n>                Stop after N cases (0 = no explicit case limit)
  --qualities <csv>              Example: full,fast
  --fps <csv>                    Example: 5,10,30
  --exposures <csv>              Example: 1,10,100
  --no-build-remote-first        Do not rebuild remote daemon on first start
  --stop-on-first-failure        Stop matrix after first failing case
  -h, --help                     Show help

Artifacts:
  tmp/istar-overnight-matrix/<run-id>/
    matrix.log
    cases.csv
    latest_case.log
    per-case/<case-id>.log
EOF
}

split_csv_into_array() {
  local csv="$1"
  local out_name="$2"
  local IFS=','
  local tmp_array=()
  local quoted=""
  local item
  read -r -a tmp_array <<<"$csv"
  for item in "${tmp_array[@]}"; do
    quoted+="$(printf '%q ' "$item")"
  done
  eval "$out_name=($quoted)"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --ssh-host) SSH_HOST="$2"; shift 2 ;;
    --remote-dir) REMOTE_DIR="$2"; shift 2 ;;
    --port) PORT="$2"; shift 2 ;;
    --device) DEVICE_ID="$2"; shift 2 ;;
    --runtime-mode) RUNTIME_MODE="$2"; shift 2 ;;
    --hours) TOTAL_SECONDS=$(( $2 * 60 * 60 )); shift 2 ;;
    --total-seconds) TOTAL_SECONDS="$2"; shift 2 ;;
    --soak-seconds) SOAK_SECONDS="$2"; shift 2 ;;
    --pause-seconds) PAUSE_SECONDS="$2"; shift 2 ;;
    --batch-size) BATCH_SIZE="$2"; shift 2 ;;
    --health-poll-seconds) HEALTH_POLL_SECONDS="$2"; shift 2 ;;
    --max-cases) MAX_CASES="$2"; shift 2 ;;
    --qualities) split_csv_into_array "$2" QUALITIES; shift 2 ;;
    --fps) split_csv_into_array "$2" FPS_VALUES; shift 2 ;;
    --exposures) split_csv_into_array "$2" EXPOSURE_VALUES; shift 2 ;;
    --no-build-remote-first) BUILD_REMOTE_ON_FIRST_START=false; shift ;;
    --stop-on-first-failure) STOP_ON_FIRST_FAILURE=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ ! -x "$REPRO_SCRIPT" ]]; then
  echo "Repro script not found or not executable: $REPRO_SCRIPT" >&2
  exit 1
fi
if [[ "$RUNTIME_MODE" != "universal" ]]; then
  echo "ERROR: --runtime-mode '$RUNTIME_MODE' is not currently supported by the repro harness." >&2
  echo "Use --runtime-mode universal (default)." >&2
  exit 2
fi
if [[ ${#QUALITIES[@]} -eq 0 || ${#FPS_VALUES[@]} -eq 0 || ${#EXPOSURE_VALUES[@]} -eq 0 ]]; then
  echo "Matrix dimensions may not be empty" >&2
  exit 2
fi

mkdir -p "$LOCAL_RUN_ROOT"
RUN_ID="$(date +%Y%m%dT%H%M%S%z)"
RUN_DIR="${LOCAL_RUN_ROOT}/${RUN_ID}"
mkdir -p "${RUN_DIR}/per-case"

LOG_FILE="${RUN_DIR}/matrix.log"
LATEST_CASE_LOG="${RUN_DIR}/latest_case.log"
CSV_FILE="${RUN_DIR}/cases.csv"

exec > >(tee -a "$LOG_FILE") 2>&1

log() {
  printf '[%s] %s\n' "$(date '+%F %T')" "$*"
}

csv_escape() {
  local s="$1"
  s="${s//\"/\"\"}"
  printf '"%s"' "$s"
}

extract_case_run_dir() {
  local f="$1"
  sed -n 's/.*Run dir: //p' "$f" | tail -n1
}

extract_result_line() {
  local f="$1"
  sed -n 's/.*RESULT: //p' "$f" | tail -n1
}

extract_stream_exit_code() {
  local f="$1"
  sed -n 's/.*Stream process exit code: \([0-9][0-9]*\).*/\1/p' "$f" | tail -n1
}

contains_repro_marker() {
  local f="$1"
  local marker="$2"
  if command -v rg >/dev/null 2>&1; then
    rg -q --fixed-strings "$marker" "$f"
  else
    grep -Fq "$marker" "$f"
  fi
}

printf 'case_index,start_iso,end_iso,quality,max_fps,exposure_ms,restarted_daemon,built_remote,case_exit_code,stream_exit_code,result_line,repro_run_dir,resource_exhausted,daemon_health_loss\n' > "$CSV_FILE"

deadline_epoch=$(( $(date +%s) + TOTAL_SECONDS ))
case_index=0
combo_index=0
cases_since_restart=0
need_restart=true
need_build=false

if $BUILD_REMOTE_ON_FIRST_START; then
  need_build=true
fi

total_combos=$(( ${#QUALITIES[@]} * ${#FPS_VALUES[@]} * ${#EXPOSURE_VALUES[@]} ))
log "Overnight matrix run dir: ${RUN_DIR}"
log "Matrix size: ${#QUALITIES[@]} qualities x ${#FPS_VALUES[@]} fps x ${#EXPOSURE_VALUES[@]} exposures = ${total_combos} combos"
log "Runtime target: ${TOTAL_SECONDS}s (deadline: $(date -r "$deadline_epoch" '+%F %T' 2>/dev/null || date '+%F %T'))"
log "Per-case soak: ${SOAK_SECONDS}s, pause: ${PAUSE_SECONDS}s, batch size before daemon restart: ${BATCH_SIZE}"
log "Tip: tail -f ${LOG_FILE}"

while :; do
  now_epoch="$(date +%s)"
  if (( now_epoch >= deadline_epoch )); then
    log "Reached runtime deadline; stopping matrix"
    break
  fi
  if (( MAX_CASES > 0 && case_index >= MAX_CASES )); then
    log "Reached max cases (${MAX_CASES}); stopping matrix"
    break
  fi

  q_idx=$(( combo_index % ${#QUALITIES[@]} ))
  f_idx=$(( (combo_index / ${#QUALITIES[@]}) % ${#FPS_VALUES[@]} ))
  e_idx=$(( (combo_index / (${#QUALITIES[@]} * ${#FPS_VALUES[@]})) % ${#EXPOSURE_VALUES[@]} ))
  quality="${QUALITIES[$q_idx]}"
  max_fps="${FPS_VALUES[$f_idx]}"
  exposure_ms="${EXPOSURE_VALUES[$e_idx]}"
  combo_index=$((combo_index + 1))
  case_index=$((case_index + 1))

  restarted_daemon=false
  built_remote=false
  if $need_restart; then
    restarted_daemon=true
    cases_since_restart=0
  fi
  if $need_build; then
    built_remote=true
  fi

  case_id="$(printf '%04d' "$case_index")"
  case_log="${RUN_DIR}/per-case/${case_id}_${quality}_${max_fps}fps_${exposure_ms}ms.log"
  cp /dev/null "$LATEST_CASE_LOG"

  start_iso="$(date '+%Y-%m-%dT%H:%M:%S%z')"
  log "CASE ${case_id}: quality=${quality} max_fps=${max_fps} exposure_ms=${exposure_ms} restart=${restarted_daemon} build=${built_remote}"

  cmd=(
    bash "$REPRO_SCRIPT"
    --ssh-host "$SSH_HOST"
    --remote-dir "$REMOTE_DIR"
    --port "$PORT"
    --device "$DEVICE_ID"
    --runtime-mode "$RUNTIME_MODE"
    --health-poll-seconds "$HEALTH_POLL_SECONDS"
    --quality "$quality"
    --max-fps "$max_fps"
    --exposure-ms "$exposure_ms"
    --soak-seconds "$SOAK_SECONDS"
  )
  if ! $restarted_daemon; then
    cmd+=(--no-restart-daemon)
  fi
  if $built_remote; then
    cmd+=(--build-remote)
  fi

  set +e
  "${cmd[@]}" 2>&1 | tee "$case_log" | tee "$LATEST_CASE_LOG"
  case_ec=${PIPESTATUS[0]}
  set -e
  end_iso="$(date '+%Y-%m-%dT%H:%M:%S%z')"

  repro_run_dir="$(extract_case_run_dir "$case_log")"
  result_line="$(extract_result_line "$case_log")"
  stream_exit_code="$(extract_stream_exit_code "$case_log")"
  resource_exhausted=false
  daemon_health_loss=false
  if contains_repro_marker "$case_log" "ResourceExhausted"; then
    resource_exhausted=true
  fi
  if contains_repro_marker "$case_log" "RESULT: reproduced daemon crash/health loss during stream soak"; then
    daemon_health_loss=true
  fi

  printf '%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
    "$case_index" \
    "$start_iso" \
    "$end_iso" \
    "$quality" \
    "$max_fps" \
    "$exposure_ms" \
    "$restarted_daemon" \
    "$built_remote" \
    "$case_ec" \
    "${stream_exit_code:-}" \
    "$(csv_escape "${result_line:-}")" \
    "$(csv_escape "${repro_run_dir:-}")" \
    "$resource_exhausted" \
    "$daemon_health_loss" \
    >> "$CSV_FILE"

  if (( case_ec == 0 )); then
    cases_since_restart=$((cases_since_restart + 1))
    if (( BATCH_SIZE > 0 && cases_since_restart >= BATCH_SIZE )); then
      need_restart=true
      need_build=false
      log "CASE ${case_id}: success; scheduling daemon restart before next case (batch size reached)"
    else
      need_restart=false
      need_build=false
      log "CASE ${case_id}: success"
    fi
  else
    need_restart=true
    need_build=false
    log "CASE ${case_id}: FAILED (exit=${case_ec}); will restart daemon next case"
    if $STOP_ON_FIRST_FAILURE; then
      log "Stopping matrix on first failure (requested)"
      break
    fi
  fi

  now_epoch="$(date +%s)"
  if (( now_epoch >= deadline_epoch )); then
    break
  fi
  sleep "$PAUSE_SECONDS"
done

log "Matrix completed"
log "Artifacts:"
log "  log:  ${LOG_FILE}"
log "  csv:  ${CSV_FILE}"
log "  per-case logs: ${RUN_DIR}/per-case"
