#!/usr/bin/env bash
# stress-test-comedi-concurrent.sh — bd-ucyu.4.2
#
# Validates concurrent AI + AO access on Comedi hardware.
# Hammers ReadAnalogInput while simultaneously commanding SetAnalogOutput.
#
# Acceptance criteria:
#   - No kernel panic (dmesg clean)
#   - No EBUSY errors from Comedi driver
#   - All AI reads return success=true
#   - All AO writes return success=true
#   - Completes within timeout
#
# Usage:
#   bash scripts/stress-test-comedi-concurrent.sh [--daemon-url URL] [--device-id ID] [--duration SECS] [--rate HZ]
#
# Requires: grpcurl, jq, bash 4+
#
# Example:
#   bash scripts/stress-test-comedi-concurrent.sh --daemon-url 100.117.5.12:50051 --duration 30 --rate 50
set -euo pipefail

# Defaults
DAEMON_URL="${DAEMON_URL:-100.117.5.12:50051}"
DEVICE_ID="${DEVICE_ID:-ni_daq}"
DURATION_SECS=30
RATE_HZ=100
AI_CHANNEL=0
AO_CHANNEL=0
AI_RANGE=0
AO_RANGE=0
DMESG_BEFORE=""
FAILURES=0
AI_COUNT=0
AO_COUNT=0
AI_ERRORS=0
AO_ERRORS=0
RESULTS_DIR="/tmp/comedi-stress-$(date +%Y%m%d-%H%M%S)"

# Parse args
while [[ $# -gt 0 ]]; do
  case "$1" in
    --daemon-url) DAEMON_URL="$2"; shift 2 ;;
    --device-id)  DEVICE_ID="$2"; shift 2 ;;
    --duration)   DURATION_SECS="$2"; shift 2 ;;
    --rate)       RATE_HZ="$2"; shift 2 ;;
    --ai-channel) AI_CHANNEL="$2"; shift 2 ;;
    --ao-channel) AO_CHANNEL="$2"; shift 2 ;;
    *) echo "Unknown arg: $1"; exit 1 ;;
  esac
done

SLEEP_US=$(( 1000000 / RATE_HZ ))
TOTAL_OPS=$(( DURATION_SECS * RATE_HZ ))

mkdir -p "${RESULTS_DIR}"

echo "=== Comedi Concurrent AI+AO Stress Test (bd-ucyu.4.2) ==="
echo "  Daemon:     ${DAEMON_URL}"
echo "  Device:     ${DEVICE_ID}"
echo "  Duration:   ${DURATION_SECS}s"
echo "  Rate:       ${RATE_HZ} Hz (${SLEEP_US}µs interval)"
echo "  Total ops:  ~${TOTAL_OPS} per channel"
echo "  AI channel: ${AI_CHANNEL}, AO channel: ${AO_CHANNEL}"
echo "  Results:    ${RESULTS_DIR}"
echo ""

# Pre-flight: check daemon is reachable
echo "--- Preflight ---"
if ! grpcurl -plaintext "${DAEMON_URL}" list >/dev/null 2>&1; then
  echo "FAIL: Cannot reach daemon at ${DAEMON_URL}"
  exit 1
fi
echo "  Daemon reachable: OK"

# Capture dmesg before test
DMESG_BEFORE=$(dmesg | tail -50)
echo "  dmesg baseline captured"

# Check device status
STATUS=$(grpcurl -plaintext -d "{\"device_id\": \"${DEVICE_ID}\"}" \
  "${DAEMON_URL}" daq.ni_daq.NiDaqService/GetDAQStatus 2>&1) || true
if echo "${STATUS}" | jq -e '.online == true' >/dev/null 2>&1; then
  BOARD=$(echo "${STATUS}" | jq -r '.boardName // "unknown"')
  echo "  Device ${DEVICE_ID}: online (${BOARD})"
else
  echo "WARN: Could not confirm device online — proceeding anyway"
  echo "  Status response: ${STATUS}" | head -3
fi
echo ""

# --- AI reader (background) ---
ai_reader() {
  local count=0
  local errors=0
  local end_time=$(( $(date +%s) + DURATION_SECS ))
  local log="${RESULTS_DIR}/ai_reads.jsonl"

  while [[ $(date +%s) -lt ${end_time} ]]; do
    RESP=$(grpcurl -plaintext -d "{\"device_id\": \"${DEVICE_ID}\", \"channel\": ${AI_CHANNEL}, \"range_index\": ${AI_RANGE}}" \
      "${DAEMON_URL}" daq.ni_daq.NiDaqService/ReadAnalogInput 2>&1) || true

    echo "${RESP}" >> "${log}"

    if echo "${RESP}" | jq -e '.success == true' >/dev/null 2>&1; then
      count=$((count + 1))
    else
      errors=$((errors + 1))
      echo "AI_ERROR[${errors}]: ${RESP}" >> "${RESULTS_DIR}/ai_errors.log"
    fi

    # Rate limit
    usleep "${SLEEP_US}" 2>/dev/null || sleep "0.$(printf '%06d' ${SLEEP_US})" 2>/dev/null || true
  done

  echo "${count}" > "${RESULTS_DIR}/ai_count"
  echo "${errors}" > "${RESULTS_DIR}/ai_errors"
}

# --- AO writer (background) ---
ao_writer() {
  local count=0
  local errors=0
  local end_time=$(( $(date +%s) + DURATION_SECS ))
  local log="${RESULTS_DIR}/ao_writes.jsonl"
  local step=0

  while [[ $(date +%s) -lt ${end_time} ]]; do
    # Sweep voltage: 0V → 1V → 0V (triangle wave)
    local phase=$(( step % 200 ))
    local voltage
    if [[ ${phase} -lt 100 ]]; then
      voltage=$(echo "scale=4; ${phase} * 0.01" | bc)
    else
      voltage=$(echo "scale=4; (200 - ${phase}) * 0.01" | bc)
    fi

    RESP=$(grpcurl -plaintext -d "{\"device_id\": \"${DEVICE_ID}\", \"channel\": ${AO_CHANNEL}, \"voltage\": ${voltage}, \"range_index\": ${AO_RANGE}}" \
      "${DAEMON_URL}" daq.ni_daq.NiDaqService/SetAnalogOutput 2>&1) || true

    echo "${RESP}" >> "${log}"

    if echo "${RESP}" | jq -e '.success == true' >/dev/null 2>&1; then
      count=$((count + 1))
    else
      errors=$((errors + 1))
      echo "AO_ERROR[${errors}]: ${RESP}" >> "${RESULTS_DIR}/ao_errors.log"
    fi

    step=$((step + 1))
    usleep "${SLEEP_US}" 2>/dev/null || sleep "0.$(printf '%06d' ${SLEEP_US})" 2>/dev/null || true
  done

  echo "${count}" > "${RESULTS_DIR}/ao_count"
  echo "${errors}" > "${RESULTS_DIR}/ao_errors"
}

# --- Run both concurrently ---
echo "--- Running stress test (${DURATION_SECS}s) ---"
echo "  AI reader: channel ${AI_CHANNEL} @ ${RATE_HZ} Hz"
echo "  AO writer: channel ${AO_CHANNEL} @ ${RATE_HZ} Hz (0-1V triangle)"
echo ""

START_TS=$(date +%s)
ai_reader &
AI_PID=$!
ao_writer &
AO_PID=$!

# Progress indicator
ELAPSED=0
while [[ ${ELAPSED} -lt ${DURATION_SECS} ]]; do
  sleep 5
  ELAPSED=$(( $(date +%s) - START_TS ))
  printf "  [%3ds / %ds] AI PID=%d AO PID=%d\n" "${ELAPSED}" "${DURATION_SECS}" "${AI_PID}" "${AO_PID}"
done

wait "${AI_PID}" 2>/dev/null || true
wait "${AO_PID}" 2>/dev/null || true

END_TS=$(date +%s)
ACTUAL_DURATION=$(( END_TS - START_TS ))

# Read results
AI_COUNT=$(cat "${RESULTS_DIR}/ai_count" 2>/dev/null || echo 0)
AO_COUNT=$(cat "${RESULTS_DIR}/ao_count" 2>/dev/null || echo 0)
AI_ERRORS=$(cat "${RESULTS_DIR}/ai_errors" 2>/dev/null || echo 0)
AO_ERRORS=$(cat "${RESULTS_DIR}/ao_errors" 2>/dev/null || echo 0)

echo ""
echo "=== Results ==="
echo "  Duration:    ${ACTUAL_DURATION}s"
echo "  AI reads:    ${AI_COUNT} success, ${AI_ERRORS} errors"
echo "  AO writes:   ${AO_COUNT} success, ${AO_ERRORS} errors"
echo "  AI rate:     $(( AI_COUNT / (ACTUAL_DURATION > 0 ? ACTUAL_DURATION : 1) )) ops/s"
echo "  AO rate:     $(( AO_COUNT / (ACTUAL_DURATION > 0 ? ACTUAL_DURATION : 1) )) ops/s"
echo ""

# --- Post-flight checks ---
echo "--- Acceptance Checks ---"

# Check 1: No kernel panic (dmesg diff)
DMESG_AFTER=$(dmesg | tail -50)
DMESG_NEW=$(diff <(echo "${DMESG_BEFORE}") <(echo "${DMESG_AFTER}") 2>/dev/null | grep '^>' | sed 's/^> //' || true)
if echo "${DMESG_NEW}" | grep -qi 'panic\|oops\|comedi.*error\|EBUSY\|segfault' 2>/dev/null; then
  echo "  FAIL: Kernel/driver issues detected in dmesg:"
  echo "${DMESG_NEW}" | grep -i 'panic\|oops\|comedi\|EBUSY\|segfault'
  FAILURES=$((FAILURES + 1))
else
  echo "  PASS: No kernel panic or driver errors in dmesg"
fi

# Check 2: No EBUSY in error logs
if grep -qi 'EBUSY\|resource busy' "${RESULTS_DIR}"/*.log 2>/dev/null; then
  echo "  FAIL: EBUSY errors detected"
  grep -i 'EBUSY\|resource busy' "${RESULTS_DIR}"/*.log | head -5
  FAILURES=$((FAILURES + 1))
else
  echo "  PASS: No EBUSY errors"
fi

# Check 3: AI reads succeeded
if [[ ${AI_ERRORS} -eq 0 ]] && [[ ${AI_COUNT} -gt 0 ]]; then
  echo "  PASS: All ${AI_COUNT} AI reads succeeded"
else
  echo "  FAIL: ${AI_ERRORS} AI read errors (${AI_COUNT} success)"
  FAILURES=$((FAILURES + 1))
fi

# Check 4: AO writes succeeded
if [[ ${AO_ERRORS} -eq 0 ]] && [[ ${AO_COUNT} -gt 0 ]]; then
  echo "  PASS: All ${AO_COUNT} AO writes succeeded"
else
  echo "  FAIL: ${AO_ERRORS} AO write errors (${AO_COUNT} success)"
  FAILURES=$((FAILURES + 1))
fi

# Check 5: Reasonable throughput (at least 10% of target rate)
MIN_OPS=$(( TOTAL_OPS / 10 ))
if [[ ${AI_COUNT} -lt ${MIN_OPS} ]]; then
  echo "  WARN: AI throughput low: ${AI_COUNT} < ${MIN_OPS} (10% of target)"
fi
if [[ ${AO_COUNT} -lt ${MIN_OPS} ]]; then
  echo "  WARN: AO throughput low: ${AO_COUNT} < ${MIN_OPS} (10% of target)"
fi

echo ""
if [[ ${FAILURES} -eq 0 ]]; then
  echo "=== ALL ACCEPTANCE CHECKS PASSED ==="
  echo "Results saved to: ${RESULTS_DIR}"
  exit 0
else
  echo "=== ${FAILURES} ACCEPTANCE CHECK(S) FAILED ==="
  echo "Results saved to: ${RESULTS_DIR}"
  exit 1
fi
