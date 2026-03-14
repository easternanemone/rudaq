#!/usr/bin/env bash
# 12-hour echelle extraction stability soak test.
#
# Verifies: no memory leaks, no frame drops, extraction latency budget,
# no sidecar crashes, no daemon panics.
#
# This script:
#  1) verifies daemon reachability via gRPC health check
#  2) starts camera frame streaming (StartStream + StreamFrames)
#  3) periodically samples daemon RSS, frame counters, latency, sidecar status
#  4) logs all samples to a CSV time-series
#  5) computes summary statistics and PASS/FAIL against thresholds
#
# Requires:
#  - local: ssh, grpcurl, timeout, bc (or awk for math)
#  - remote (leabs-dev): running rust-daq daemon with istar_camera
#
# Usage:
#   ./overnight-soak.sh [--hours 12] [--daemon-url 10.0.0.40:50051] [--output-dir /tmp/soak-results]

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PROTO_DIR="${ROOT_DIR}/crates/protocol/proto"

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
SOAK_HOURS=12
DAEMON_URL="10.0.0.40:50051"
SSH_HOST="leabs-dev"
DEVICE_ID="istar_camera"
SAMPLE_INTERVAL_SECONDS=60
EXPOSURE_MS=10
MAX_FPS=10
QUALITY="FAST"
OUTPUT_DIR=""

# Thresholds
THRESHOLD_MEMORY_GROWTH_KB_PER_HOUR=1024   # 1 MB/hour
THRESHOLD_FRAME_DROPS=0
THRESHOLD_SIDECAR_CRASHES=0
THRESHOLD_P99_LATENCY_MS=500

# Internal state
USE_SSH_TUNNEL=false
LOCAL_PORT=""

# ---------------------------------------------------------------------------
# Usage
# ---------------------------------------------------------------------------
usage() {
  cat <<'EOF'
Usage: overnight-soak.sh [options]

Echelle extraction stability soak test. Streams frames from a camera via the
rust-daq daemon and samples health metrics at a regular cadence.

Options:
  --hours <n>                 Soak duration in hours (default: 12)
  --daemon-url <host:port>    Daemon address (default: 10.0.0.40:50051)
  --ssh-host <host>           SSH host for remote metrics (default: leabs-dev)
  --device <id>               Camera device id (default: istar_camera)
  --sample-interval <secs>    Metrics sampling cadence (default: 60)
  --exposure-ms <ms>          Camera exposure (default: 10)
  --max-fps <n>               Stream max FPS (default: 10)
  --quality <full|preview|fast>  Stream quality (default: fast)
  --output-dir <path>         Output directory (default: /tmp/echelle-soak-YYYYMMDD-HHMMSS)
  --use-ssh-tunnel            Route gRPC through an SSH tunnel to localhost
  --local-port <n>            Local tunnel port when using SSH tunnel (default: 50051)
  --threshold-mem-kb <n>      Max memory growth KB/hour (default: 1024)
  --threshold-frame-drops <n> Max allowed frame drops (default: 0)
  --threshold-p99-ms <n>      Max P99 extraction latency ms (default: 500)
  -h, --help                  Show help
EOF
}

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
  case "$1" in
    --hours) SOAK_HOURS="$2"; shift 2 ;;
    --daemon-url) DAEMON_URL="$2"; shift 2 ;;
    --ssh-host) SSH_HOST="$2"; shift 2 ;;
    --device) DEVICE_ID="$2"; shift 2 ;;
    --sample-interval) SAMPLE_INTERVAL_SECONDS="$2"; shift 2 ;;
    --exposure-ms) EXPOSURE_MS="$2"; shift 2 ;;
    --max-fps) MAX_FPS="$2"; shift 2 ;;
    --quality) QUALITY="$(echo "$2" | tr '[:lower:]' '[:upper:]')"; shift 2 ;;
    --output-dir) OUTPUT_DIR="$2"; shift 2 ;;
    --use-ssh-tunnel) USE_SSH_TUNNEL=true; shift ;;
    --local-port) LOCAL_PORT="$2"; shift 2 ;;
    --threshold-mem-kb) THRESHOLD_MEMORY_GROWTH_KB_PER_HOUR="$2"; shift 2 ;;
    --threshold-frame-drops) THRESHOLD_FRAME_DROPS="$2"; shift 2 ;;
    --threshold-p99-ms) THRESHOLD_P99_LATENCY_MS="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

# Resolve quality enum
case "$QUALITY" in
  FULL) QUALITY_ENUM="STREAM_QUALITY_FULL" ;;
  PREVIEW) QUALITY_ENUM="STREAM_QUALITY_PREVIEW" ;;
  FAST) QUALITY_ENUM="STREAM_QUALITY_FAST" ;;
  *) echo "Invalid --quality '$QUALITY' (use full|preview|fast)" >&2; exit 2 ;;
esac

SOAK_SECONDS=$(( SOAK_HOURS * 3600 ))

if [[ -z "$OUTPUT_DIR" ]]; then
  OUTPUT_DIR="/tmp/echelle-soak-$(date +%Y%m%d-%H%M%S)"
fi
mkdir -p "$OUTPUT_DIR"

# Determine gRPC target address
if $USE_SSH_TUNNEL; then
  LOCAL_PORT="${LOCAL_PORT:-50051}"
  GRPC_ADDR="127.0.0.1:${LOCAL_PORT}"
else
  GRPC_ADDR="$DAEMON_URL"
fi

# ---------------------------------------------------------------------------
# Logging & helpers
# ---------------------------------------------------------------------------
LOG_FILE="${OUTPUT_DIR}/soak.log"
CSV_FILE="${OUTPUT_DIR}/samples.csv"
SUMMARY_FILE="${OUTPUT_DIR}/summary.txt"

exec > >(tee -a "$LOG_FILE") 2>&1

log() {
  printf '[%s] %s\n' "$(date '+%F %T')" "$*"
}

iso_now() {
  date '+%Y-%m-%dT%H:%M:%S%z'
}

ssh_remote() {
  ssh -o BatchMode=yes -o ConnectTimeout=10 -o ServerAliveInterval=30 "$SSH_HOST" "$@"
}

grpcurl_cmd() {
  grpcurl -plaintext \
    -connect-timeout 5 \
    -import-path "$PROTO_DIR" \
    -proto daq.proto \
    -proto hardware.proto \
    -proto health.proto \
    -max-msg-sz 67108864 \
    "$@"
}

# ---------------------------------------------------------------------------
# Health check
# ---------------------------------------------------------------------------
health_check() {
  local rc
  set +e
  grpcurl_cmd -max-time 4 "$GRPC_ADDR" daq.ControlService/GetDaemonInfo >/dev/null 2>&1
  rc=$?
  set -e
  return "$rc"
}

# ---------------------------------------------------------------------------
# SSH tunnel management
# ---------------------------------------------------------------------------
ensure_tunnel() {
  if ! $USE_SSH_TUNNEL; then
    return 0
  fi
  if health_check; then
    log "Reusing existing healthy tunnel on ${GRPC_ADDR}"
    return 0
  fi

  local tunnel_pattern
  tunnel_pattern="[s]sh .* -L 127.0.0.1:${LOCAL_PORT}:localhost:${LOCAL_PORT} ${SSH_HOST}"
  if pgrep -f "$tunnel_pattern" >/dev/null 2>&1; then
    log "Replacing stale SSH tunnel(s) on 127.0.0.1:${LOCAL_PORT}"
    pkill -f "$tunnel_pattern" >/dev/null 2>&1 || true
    sleep 1
  fi
  log "Starting SSH tunnel 127.0.0.1:${LOCAL_PORT} -> ${SSH_HOST}:localhost:${LOCAL_PORT}"
  ssh -fN \
    -o ExitOnForwardFailure=yes \
    -o ServerAliveInterval=30 \
    -o ServerAliveCountMax=3 \
    -L "127.0.0.1:${LOCAL_PORT}:localhost:${LOCAL_PORT}" \
    "$SSH_HOST"
}

# ---------------------------------------------------------------------------
# Remote daemon PID discovery
# ---------------------------------------------------------------------------
discover_daemon_pid() {
  ssh_remote "pgrep -f '[r]ust-daq-daemon daemon' | head -1" 2>/dev/null || echo ""
}

# ---------------------------------------------------------------------------
# Metric samplers
# ---------------------------------------------------------------------------

# Get daemon RSS in KB via SSH
sample_rss_kb() {
  local pid="$1"
  if [[ -z "$pid" ]]; then
    echo ""
    return
  fi
  local rss
  rss="$(ssh_remote "ps -o rss= -p $pid 2>/dev/null || true" 2>/dev/null | tr -d ' ')"
  echo "${rss:-}"
}

# Get frame count from the last StreamFrames response metrics
# We parse the streaming stderr/json for the latest frame_number.
# Also query StopStreamResponse.frames_captured is not useful here (would stop stream).
# Instead we parse the health/streaming metrics from the FrameData.metrics field.
# Since StreamFrames is a long-running server stream, we query daemon info for frame count
# as a proxy, or count frames from the stream tap.
sample_frame_metrics() {
  # Query the current StreamFrames tap with a very short timeout to get the latest frame
  # We use a 2-second max-time gRPC call that will receive at most one frame
  local output
  set +e
  output="$(grpcurl_cmd -max-time 2 \
    -d "{\"deviceId\":\"${DEVICE_ID}\",\"maxFps\":1,\"quality\":\"STREAM_QUALITY_FAST\"}" \
    "$GRPC_ADDR" daq.HardwareService/StreamFrames 2>/dev/null)"
  set -e

  local frame_number=""
  local frames_sent=""
  local frames_dropped=""
  local avg_latency_ms=""

  if [[ -n "$output" ]]; then
    # Extract frame_number from the last frame in the output
    frame_number="$(echo "$output" | awk -F'"' '/"frameNumber"/{print $4}' | tail -1)"
    # Extract metrics fields if present
    frames_sent="$(echo "$output" | awk -F'"' '/"framesSent"/{print $4}' | tail -1)"
    frames_dropped="$(echo "$output" | awk -F'"' '/"framesDropped"/{print $4}' | tail -1)"
    avg_latency_ms="$(echo "$output" | awk -F': ' '/"avgLatencyMs"/{gsub(/[^0-9.]/, "", $2); print $2}' | tail -1)"
  fi

  printf '%s,%s,%s,%s' \
    "${frame_number:-}" \
    "${frames_sent:-}" \
    "${frames_dropped:-}" \
    "${avg_latency_ms:-}"
}

# Check if the sidecar extraction process is alive on remote
sample_sidecar_alive() {
  local alive
  set +e
  alive="$(ssh_remote "pgrep -f 'echelle.*sidecar\|echelle.*extract' >/dev/null 2>&1 && echo 1 || echo 0" 2>/dev/null)"
  set -e
  # If SSH itself failed, report unknown
  echo "${alive:-unknown}"
}

# Get system load average from remote
sample_load_avg() {
  local load
  set +e
  load="$(ssh_remote "cat /proc/loadavg 2>/dev/null | awk '{print \$1}'" 2>/dev/null)"
  set -e
  echo "${load:-}"
}

# ---------------------------------------------------------------------------
# Streaming control
# ---------------------------------------------------------------------------
STREAM_PID=""

start_streaming() {
  log "Setting exposure to ${EXPOSURE_MS} ms"
  grpcurl_cmd -d "{\"deviceId\":\"${DEVICE_ID}\",\"exposureMs\":${EXPOSURE_MS}}" \
    "$GRPC_ADDR" daq.HardwareService/SetExposure \
    > "${OUTPUT_DIR}/set_exposure.json" 2>&1 || true

  log "Calling StartStream on ${DEVICE_ID}"
  grpcurl_cmd -d "{\"deviceId\":\"${DEVICE_ID}\"}" \
    "$GRPC_ADDR" daq.HardwareService/StartStream \
    > "${OUTPUT_DIR}/start_stream.json" 2> "${OUTPUT_DIR}/start_stream.stderr.txt"

  log "Starting long-running StreamFrames (stdout discarded, soak for ${SOAK_SECONDS}s)"
  set +e
  timeout --signal=INT --kill-after=10s "${SOAK_SECONDS}" \
    grpcurl -plaintext \
      -import-path "$PROTO_DIR" \
      -proto hardware.proto \
      -max-msg-sz 67108864 \
      -d "{\"deviceId\":\"${DEVICE_ID}\",\"maxFps\":${MAX_FPS},\"quality\":\"${QUALITY_ENUM}\"}" \
      "$GRPC_ADDR" daq.HardwareService/StreamFrames \
      >/dev/null 2>"${OUTPUT_DIR}/stream_frames.stderr.txt" &
  STREAM_PID=$!
  set -e
  echo "$STREAM_PID" > "${OUTPUT_DIR}/stream_grpcurl.pid"
  log "StreamFrames PID: ${STREAM_PID}"
}

stop_streaming() {
  if [[ -n "${STREAM_PID:-}" ]] && kill -0 "$STREAM_PID" 2>/dev/null; then
    log "Stopping StreamFrames process (PID ${STREAM_PID})"
    kill "$STREAM_PID" 2>/dev/null || true
    wait "$STREAM_PID" 2>/dev/null || true
  fi
  grpcurl_cmd -d "{\"deviceId\":\"${DEVICE_ID}\"}" \
    "$GRPC_ADDR" daq.HardwareService/StopStream >/dev/null 2>&1 || true
}

# ---------------------------------------------------------------------------
# Cleanup
# ---------------------------------------------------------------------------
cleanup() {
  local ec=$?
  log "Cleanup (exit code=${ec})"
  stop_streaming
  exit "$ec"
}
trap cleanup EXIT INT TERM

# ---------------------------------------------------------------------------
# Analysis helpers (pure awk -- no bc/python dependency for the bash script)
# ---------------------------------------------------------------------------

# Simple linear regression slope via awk.
# Input: lines of "x y" pairs on stdin.
# Output: slope value.
linreg_slope() {
  awk '
  {
    n++; sx += $1; sy += $2; sxy += $1*$2; sxx += $1*$1
  }
  END {
    if (n < 2) { print 0; exit }
    denom = n*sxx - sx*sx
    if (denom == 0) { print 0; exit }
    printf "%.6f\n", (n*sxy - sx*sy) / denom
  }'
}

# Percentile calculator via awk.
# Usage: echo "values..." | percentile <p>  (p in 0-100)
# Input: one value per line on stdin.
percentile() {
  local p="${1:-50}"
  sort -n | awk -v p="$p" '
  { a[NR] = $1 }
  END {
    if (NR == 0) { print ""; exit }
    idx = p/100.0 * (NR - 1) + 1
    lo = int(idx); hi = lo + 1
    if (hi > NR) hi = NR
    frac = idx - lo
    printf "%.2f\n", a[lo] + frac * (a[hi] - a[lo])
  }'
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
log "=========================================="
log "Echelle Extraction Overnight Soak Test"
log "=========================================="
log "Output dir:   ${OUTPUT_DIR}"
log "Daemon:       ${DAEMON_URL} (gRPC target: ${GRPC_ADDR})"
log "SSH host:     ${SSH_HOST}"
log "Device:       ${DEVICE_ID}"
log "Duration:     ${SOAK_HOURS}h (${SOAK_SECONDS}s)"
log "Sample every: ${SAMPLE_INTERVAL_SECONDS}s"
log "Quality:      ${QUALITY_ENUM}"
log "Exposure:     ${EXPOSURE_MS}ms"
log "Max FPS:      ${MAX_FPS}"
log "Thresholds:   mem_growth<${THRESHOLD_MEMORY_GROWTH_KB_PER_HOUR}KB/h  frame_drops<=${THRESHOLD_FRAME_DROPS}  p99_latency<${THRESHOLD_P99_LATENCY_MS}ms"

# Verify daemon reachability
if $USE_SSH_TUNNEL; then
  ensure_tunnel
fi

log "Checking daemon health at ${GRPC_ADDR}..."
HEALTH_RETRIES=5
for i in $(seq 1 $HEALTH_RETRIES); do
  if health_check; then
    log "Daemon health check OK"
    break
  fi
  if [[ $i -eq $HEALTH_RETRIES ]]; then
    log "FATAL: Daemon not reachable after ${HEALTH_RETRIES} attempts"
    exit 1
  fi
  log "Health check attempt ${i}/${HEALTH_RETRIES} failed, retrying..."
  sleep 3
done

# Capture daemon info
grpcurl_cmd "$GRPC_ADDR" daq.ControlService/GetDaemonInfo \
  > "${OUTPUT_DIR}/daemon_info.json" 2>&1 || true
grpcurl_cmd -d '{}' "$GRPC_ADDR" daq.HardwareService/ListDevices \
  > "${OUTPUT_DIR}/list_devices.json" 2>&1 || true

# Discover daemon PID for RSS sampling
DAEMON_PID="$(discover_daemon_pid)"
if [[ -n "$DAEMON_PID" ]]; then
  log "Daemon PID on remote: ${DAEMON_PID}"
else
  log "WARNING: Could not discover daemon PID; RSS sampling will be empty"
fi

# Write CSV header
printf 'timestamp,elapsed_s,rss_kb,frame_number,frames_sent,frames_dropped,avg_latency_ms,sidecar_alive,load_avg,daemon_healthy\n' > "$CSV_FILE"

# Start streaming
start_streaming

# ---------------------------------------------------------------------------
# Sampling loop
# ---------------------------------------------------------------------------
START_EPOCH="$(date +%s)"
DEADLINE_EPOCH=$(( START_EPOCH + SOAK_SECONDS ))
SAMPLE_COUNT=0
DAEMON_PANICS=0
SIDECAR_CRASH_COUNT=0
PREV_SIDECAR_ALIVE=""

log "Sampling loop started (deadline: $(date -d "@${DEADLINE_EPOCH}" '+%F %T' 2>/dev/null || date -r "${DEADLINE_EPOCH}" '+%F %T' 2>/dev/null || echo "${DEADLINE_EPOCH}"))"

while true; do
  NOW_EPOCH="$(date +%s)"
  if (( NOW_EPOCH >= DEADLINE_EPOCH )); then
    log "Reached soak deadline"
    break
  fi

  # Check if the stream process is still alive
  if [[ -n "${STREAM_PID:-}" ]] && ! kill -0 "$STREAM_PID" 2>/dev/null; then
    log "WARNING: StreamFrames process exited unexpectedly"
    # Try to restart it
    set +e
    wait "$STREAM_PID" 2>/dev/null
    stream_ec=$?
    set -e
    log "StreamFrames exit code: ${stream_ec}"
    STREAM_PID=""
    if health_check; then
      log "Daemon still healthy; restarting StreamFrames"
      start_streaming
    else
      log "FATAL: Daemon unreachable after StreamFrames exit"
      DAEMON_PANICS=$((DAEMON_PANICS + 1))
      break
    fi
  fi

  ELAPSED=$(( NOW_EPOCH - START_EPOCH ))

  # Sample all metrics (tolerate individual failures)
  rss_kb="$(sample_rss_kb "$DAEMON_PID")"
  frame_metrics="$(sample_frame_metrics)"
  sidecar_alive="$(sample_sidecar_alive)"
  load_avg="$(sample_load_avg)"

  # Parse frame_metrics (frame_number,frames_sent,frames_dropped,avg_latency_ms)
  IFS=',' read -r frame_number frames_sent frames_dropped avg_latency_ms <<< "$frame_metrics"

  # Daemon health check
  daemon_healthy=1
  if ! health_check; then
    daemon_healthy=0
    DAEMON_PANICS=$((DAEMON_PANICS + 1))
    log "WARNING: Daemon health check FAILED at elapsed=${ELAPSED}s"
  fi

  # Track sidecar crashes (transition from alive to not-alive)
  if [[ "$PREV_SIDECAR_ALIVE" == "1" && "$sidecar_alive" == "0" ]]; then
    SIDECAR_CRASH_COUNT=$((SIDECAR_CRASH_COUNT + 1))
    log "WARNING: Sidecar crash detected at elapsed=${ELAPSED}s (count=${SIDECAR_CRASH_COUNT})"
  fi
  PREV_SIDECAR_ALIVE="$sidecar_alive"

  # Write CSV row
  printf '%s,%d,%s,%s,%s,%s,%s,%s,%s,%d\n' \
    "$(iso_now)" \
    "$ELAPSED" \
    "${rss_kb:-}" \
    "${frame_number:-}" \
    "${frames_sent:-}" \
    "${frames_dropped:-}" \
    "${avg_latency_ms:-}" \
    "${sidecar_alive:-}" \
    "${load_avg:-}" \
    "$daemon_healthy" \
    >> "$CSV_FILE"

  SAMPLE_COUNT=$((SAMPLE_COUNT + 1))

  # Periodic progress log (every 10 samples)
  if (( SAMPLE_COUNT % 10 == 0 )); then
    log "Progress: ${SAMPLE_COUNT} samples, elapsed=${ELAPSED}s/${SOAK_SECONDS}s, RSS=${rss_kb:-?}KB, frame#=${frame_number:-?}, drops=${frames_dropped:-?}"
  fi

  # Abort early if daemon is dead
  if [[ $daemon_healthy -eq 0 ]]; then
    log "Attempting to continue despite health failure..."
  fi

  sleep "$SAMPLE_INTERVAL_SECONDS"
done

# Stop streaming
stop_streaming

# ---------------------------------------------------------------------------
# Analysis
# ---------------------------------------------------------------------------
log ""
log "=========================================="
log "Soak Test Analysis"
log "=========================================="

PASS=true

# 1) Memory growth rate (KB/hour via linear regression on elapsed_s vs rss_kb)
MEM_GROWTH_KB_PER_HOUR="N/A"
if [[ -f "$CSV_FILE" ]]; then
  # Extract elapsed_s and rss_kb columns (skip header, skip empty rss values)
  mem_data="$(awk -F',' 'NR>1 && $3 != "" { print $2, $3 }' "$CSV_FILE")"
  if [[ -n "$mem_data" ]]; then
    # slope is KB/second; multiply by 3600 for KB/hour
    slope_per_sec="$(echo "$mem_data" | linreg_slope)"
    MEM_GROWTH_KB_PER_HOUR="$(awk "BEGIN { printf \"%.1f\", ${slope_per_sec} * 3600 }")"
    log "Memory growth rate: ${MEM_GROWTH_KB_PER_HOUR} KB/hour (threshold: ${THRESHOLD_MEMORY_GROWTH_KB_PER_HOUR} KB/hour)"

    # Check threshold (compare absolute value)
    abs_growth="$(awk "BEGIN { v = ${MEM_GROWTH_KB_PER_HOUR}; print (v < 0 ? -v : v) }")"
    if awk "BEGIN { exit !(${abs_growth} > ${THRESHOLD_MEMORY_GROWTH_KB_PER_HOUR}) }"; then
      log "FAIL: Memory growth ${MEM_GROWTH_KB_PER_HOUR} KB/hour exceeds threshold ${THRESHOLD_MEMORY_GROWTH_KB_PER_HOUR} KB/hour"
      PASS=false
    else
      log "PASS: Memory growth within threshold"
    fi
  else
    log "WARNING: No RSS data collected; memory growth check skipped"
  fi
fi

# 2) Frame drops
TOTAL_FRAME_DROPS=0
if [[ -f "$CSV_FILE" ]]; then
  # Get the last non-empty frames_dropped value
  last_drops="$(awk -F',' 'NR>1 && $6 != "" { v=$6 } END { print v+0 }' "$CSV_FILE")"
  TOTAL_FRAME_DROPS="${last_drops:-0}"
  log "Total frame drops: ${TOTAL_FRAME_DROPS} (threshold: ${THRESHOLD_FRAME_DROPS})"
  if (( TOTAL_FRAME_DROPS > THRESHOLD_FRAME_DROPS )); then
    log "FAIL: ${TOTAL_FRAME_DROPS} frame drops exceed threshold ${THRESHOLD_FRAME_DROPS}"
    PASS=false
  else
    log "PASS: Frame drops within threshold"
  fi
fi

# 3) Sidecar crashes
log "Sidecar crashes: ${SIDECAR_CRASH_COUNT} (threshold: ${THRESHOLD_SIDECAR_CRASHES})"
if (( SIDECAR_CRASH_COUNT > THRESHOLD_SIDECAR_CRASHES )); then
  log "FAIL: ${SIDECAR_CRASH_COUNT} sidecar crashes exceed threshold ${THRESHOLD_SIDECAR_CRASHES}"
  PASS=false
else
  log "PASS: Sidecar crash count within threshold"
fi

# 4) Daemon panics / health losses
log "Daemon health losses: ${DAEMON_PANICS}"
if (( DAEMON_PANICS > 0 )); then
  log "FAIL: ${DAEMON_PANICS} daemon health losses detected"
  PASS=false
else
  log "PASS: No daemon health losses"
fi

# 5) Extraction latency P50/P99
LATENCY_P50="N/A"
LATENCY_P99="N/A"
if [[ -f "$CSV_FILE" ]]; then
  latency_values="$(awk -F',' 'NR>1 && $7 != "" { print $7 }' "$CSV_FILE")"
  if [[ -n "$latency_values" ]]; then
    LATENCY_P50="$(echo "$latency_values" | percentile 50)"
    LATENCY_P99="$(echo "$latency_values" | percentile 99)"
    log "Extraction latency P50: ${LATENCY_P50} ms"
    log "Extraction latency P99: ${LATENCY_P99} ms (threshold: ${THRESHOLD_P99_LATENCY_MS} ms)"
    if awk "BEGIN { exit !(${LATENCY_P99} > ${THRESHOLD_P99_LATENCY_MS}) }"; then
      log "FAIL: P99 latency ${LATENCY_P99} ms exceeds threshold ${THRESHOLD_P99_LATENCY_MS} ms"
      PASS=false
    else
      log "PASS: Extraction latency within threshold"
    fi
  else
    log "WARNING: No latency data collected; latency check skipped"
  fi
fi

# 6) Total frames captured
TOTAL_FRAMES="N/A"
if [[ -f "$CSV_FILE" ]]; then
  last_frame_number="$(awk -F',' 'NR>1 && $4 != "" { v=$4 } END { print v }' "$CSV_FILE")"
  first_frame_number="$(awk -F',' 'NR>1 && $4 != "" { print $4; exit }' "$CSV_FILE")"
  if [[ -n "$last_frame_number" && -n "$first_frame_number" ]]; then
    TOTAL_FRAMES=$(( last_frame_number - first_frame_number ))
  fi
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
RESULT="PASS"
if ! $PASS; then
  RESULT="FAIL"
fi

{
  printf '==========================================\n'
  printf 'Echelle Extraction Soak Test Summary\n'
  printf '==========================================\n'
  printf 'Date:                  %s\n' "$(date '+%F %T %z')"
  printf 'Duration:              %d hours (%d seconds)\n' "$SOAK_HOURS" "$SOAK_SECONDS"
  printf 'Samples collected:     %d\n' "$SAMPLE_COUNT"
  printf 'Device:                %s\n' "$DEVICE_ID"
  printf 'Daemon:                %s\n' "$DAEMON_URL"
  printf '\n'
  printf 'Memory growth rate:    %s KB/hour (threshold: %d KB/hour)\n' "$MEM_GROWTH_KB_PER_HOUR" "$THRESHOLD_MEMORY_GROWTH_KB_PER_HOUR"
  printf 'Total frame drops:     %d (threshold: %d)\n' "$TOTAL_FRAME_DROPS" "$THRESHOLD_FRAME_DROPS"
  printf 'Total frames:          %s\n' "$TOTAL_FRAMES"
  printf 'Sidecar crashes:       %d (threshold: %d)\n' "$SIDECAR_CRASH_COUNT" "$THRESHOLD_SIDECAR_CRASHES"
  printf 'Daemon health losses:  %d\n' "$DAEMON_PANICS"
  printf 'Latency P50:           %s ms\n' "$LATENCY_P50"
  printf 'Latency P99:           %s ms (threshold: %d ms)\n' "$LATENCY_P99" "$THRESHOLD_P99_LATENCY_MS"
  printf '\n'
  printf 'RESULT: %s\n' "$RESULT"
  printf '==========================================\n'
} | tee "$SUMMARY_FILE"

log "Artifacts: ${OUTPUT_DIR}"
log "  CSV:     ${CSV_FILE}"
log "  Summary: ${SUMMARY_FILE}"
log "  Log:     ${LOG_FILE}"

if [[ "$RESULT" == "FAIL" ]]; then
  exit 1
fi
exit 0
