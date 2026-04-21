#!/usr/bin/env bash
# bd-61aej — DH3P flat-field fixture matrix capture (DoE v2).
#
# Drives an ALREADY-RUNNING rust-daq-daemon through a sweep of
# (MCP gain, exposure, accumulations) settings to produce a range of
# DH3P continuum flat-field fixtures. Mirrors capture_hgar_matrix.sh's
# per-frame --after-set MCP reset contract for safety.
#
# DH3P on leabs-dev is an EXCEEDINGLY WEAK lamp (v1 DoE at gain 0-200
# produced pure noise-floor frames, pmax=125-140 vs 101 baseline). The
# existing reference flat for the merged-spectrum golden runs at
# gain=2000 / 5000ms / acc10 = 50 s effective integration. User
# confirmed g=2000 is safe on this lamp (weak enough that the usual
# MCP-dwell budget does not apply).
#
# v2 spans gain 500 → 2000 and exposure 1000 → 5000 ms, with on-sensor
# accumulation (AccumulateCount) to boost SNR without single-frame
# saturation. AccumulateCount=N means the camera sums N exposures before
# readout; effective integration = N * exposure_ms.
#
# Usage (on leabs-dev, with daemon already running on :50051):
#   bash scripts/hardware/capture_dh3p_matrix.sh [OUT_DIR]

set -euo pipefail

# ── Configuration ──────────────────────────────────────────────────────
DAEMON="${DAEMON:-http://localhost:50051}"
DEVICE="${DEVICE:-istar_camera}"
BINARY="${BINARY:-/home/brian/code/rust-daq/target/release/rust-daq-daemon}"
OUT_DIR="${1:-/home/brian/dh3p_matrix_$(date +%Y%m%d_%H%M%S)}"

# ── Capture matrix — DoE v3 (bd-61aej) ────────────────────────────────
# Each row: <mcp_gain>,<exposure_ms>,<accumulate_count>.
#
# v1 (gain 0-200) + v2 (gain 500-2000, max 100 s integration) both
# failed to resolve order structure. Root cause: smaller slit recently
# installed on the Mechelle 5000 dropped throughput ~3× vs. the state
# in which the reference flat (g=2000 / 5 s / acc=10 = 50 s) was taken.
# v3 pushes integration 3×-12× beyond the old reference to compensate,
# and also spans gain=2000-4000 to test whether higher amplification
# recovers order contrast at fixed integration.
#
# Rows (effective integration = exposure × acc):
#   gain=2000, exp=30 s,  acc=5   = 150 s   (3× reference)
#   gain=2000, exp=30 s,  acc=10  = 300 s   (6× reference)
#   gain=2000, exp=30 s,  acc=20  = 600 s   (12× reference, 10 min)
#   gain=3000, exp=30 s,  acc=10  = 300 s   (higher gain, mid integration)
#   gain=3000, exp=30 s,  acc=20  = 600 s   (higher gain, 10 min)
#   gain=4000, exp=30 s,  acc=10  = 300 s   (max-gain sanity check)
#
# Total wall time: ~38 min of exposure + readout overhead.
MATRIX=(
  "2000,30000,5"
  "2000,30000,10"
  "2000,30000,20"
  "3000,30000,10"
  "3000,30000,20"
  "4000,30000,10"
)

# ── Helpers ────────────────────────────────────────────────────────────

reset_mcp() {
  "$BINARY" snapshot "$DEVICE" \
    --set "mcp_gain=0" \
    --set "AccumulateCount=1" \
    --exposure-ms 100 \
    --output /tmp/bd-61aej_mcp_reset.tiff \
    --format tiff \
    --addr "$DAEMON" >/dev/null 2>&1
}

cleanup_mcp() {
  printf '[cleanup] resetting MCP gain to 0 and AccumulateCount to 1...\n'
  if reset_mcp; then
    printf '[cleanup] MCP gain = 0 / AccumulateCount = 1 confirmed\n'
  else
    printf '[cleanup] WARNING: reset call failed — verify manually before disconnect\n' >&2
  fi
}
trap cleanup_mcp EXIT INT TERM

# ── Main loop ──────────────────────────────────────────────────────────

mkdir -p "$OUT_DIR"
printf 'Output: %s\n' "$OUT_DIR"
printf 'Daemon: %s (device=%s)\n' "$DAEMON" "$DEVICE"
printf 'Matrix: %d captures (DoE v2, gain up to 2000, accumulate up to 20)\n\n' "${#MATRIX[@]}"

META="$OUT_DIR/matrix.json"
printf '[\n' > "$META"
FIRST=1

for row in "${MATRIX[@]}"; do
  IFS=',' read -r gain exp_ms acc <<< "$row"
  eff_ms=$((exp_ms * acc))
  label="dh3p_g${gain}_t${exp_ms}ms_acc${acc}"
  out_tiff="$OUT_DIR/${label}.tiff"

  printf '=== %-36s (MCP=%s, exp=%s ms × %s = %s ms eff) ===\n' \
    "$label" "$gain" "$exp_ms" "$acc" "$eff_ms"

  t_start="$(date +%s)"
  # Continuum lamp: Internal trigger + CW gate-on is correct (no gating
  # needed for broadband source). --after-set forces MCP → 0 and
  # AccumulateCount → 1 before snapshot exits, so the next row always
  # starts from a safe state.
  "$BINARY" snapshot \
    "$DEVICE" \
    --set "trigger_mode=Internal" \
    --set "gate_mode=CWOn" \
    --set "mcp_gain=${gain}" \
    --set "AccumulateCount=${acc}" \
    --exposure-ms "$exp_ms" \
    --after-set "mcp_gain=0" \
    --after-set "AccumulateCount=1" \
    --output "$out_tiff" \
    --format tiff \
    --addr "$DAEMON"
  t_end="$(date +%s)"
  elapsed=$((t_end - t_start))
  size_kb="$(du -k "$out_tiff" | awk '{print $1}')"

  if [[ $FIRST -eq 0 ]]; then
    printf ',\n' >> "$META"
  fi
  FIRST=0
  printf '  {"label":"%s","mcp_gain":%s,"exposure_ms":%s,"accumulate_count":%s,"effective_ms":%s,"tiff":"%s","elapsed_s":%s,"size_kb":%s,"captured_at":"%s"}' \
    "$label" "$gain" "$exp_ms" "$acc" "$eff_ms" "${label}.tiff" "$elapsed" "$size_kb" "$(date -Iseconds)" \
    >> "$META"

  printf '  → %s (%s KB, %ss)\n' "$out_tiff" "$size_kb" "$elapsed"
done

printf '\n]\n' >> "$META"

printf '\n=== summary ===\n'
printf 'Captured %d frames to %s\n' "${#MATRIX[@]}" "$OUT_DIR"
printf 'Metadata: %s\n' "$META"
printf 'NOTE: cleanup trap will reset MCP gain to 0 and AccumulateCount to 1 on exit.\n'
