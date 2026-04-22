#!/usr/bin/env bash
# bd-cpph3 — Halogen-only flat-field fixture matrix capture.
#
# Companion to capture_d2_matrix.sh. Same Mechelle 5000 + iStar sCMOS
# rig on leabs-dev, same smaller slit (installed 2026-04-21), but with
# the D2 source of the DH3P switched OFF so only the tungsten-halogen
# lamp illuminates the spectrograph. Halogen is a ~3000 K blackbody
# source — peaks in the NIR (900-1100 nm), falls off exponentially
# toward UV, and is MUCH brighter than D2 at every wavelength it
# reaches. Target band: visible + NIR orders (m ≈ 25-55, λ ≈ 365-810 nm).
#
# Strategy is inverted from the D2 matrix: halogen is photon-rich so we
# use LOW gain and SHORT exposure to avoid saturation. No accumulation
# is needed for SNR — accumulation here is purely for averaging out
# residual dark-current / readout-noise structure.
#
# Usage (on leabs-dev, with daemon already running on :50051):
#   bash scripts/hardware/capture_halogen_matrix.sh [OUT_DIR]
#
# Safety contracts (inherited):
#   - MCP MUST be reset to 0 between frames (enforced by --after-set).
#   - Matrix caps gain at 500 (bright lamp — no need to amplify further).

set -euo pipefail

# ── Configuration ──────────────────────────────────────────────────────
DAEMON="${DAEMON:-http://localhost:50051}"
DEVICE="${DEVICE:-istar_camera}"
BINARY="${BINARY:-/home/brian/code/rust-daq/target/release/rust-daq-daemon}"
OUT_DIR="${1:-/home/brian/halogen_matrix_$(date +%Y%m%d_%H%M%S)}"

# ── Capture matrix — DoE v1 (bd-cpph3) ────────────────────────────────
# Each row: <mcp_gain>,<exposure_ms>,<accumulate_count>.
#
# Rows:
#   g=0,   exp=500 ms,  acc=1  = 500 ms   baseline, probe saturation at g=0
#   g=0,   exp=2000 ms, acc=1  = 2000 ms  baseline long — near saturation?
#   g=100, exp=100 ms,  acc=1  = 100 ms   low-gain short — avoid saturation
#   g=100, exp=500 ms,  acc=5  = 2500 ms  low-gain accumulate for noise
#   g=500, exp=50 ms,   acc=1  = 50 ms    mid-gain very short
#   g=500, exp=200 ms,  acc=5  = 1000 ms  mid-gain accumulate
#
# Total wall time: a few minutes (dominated by readout/FFI overhead, not exposure).
MATRIX=(
  "0,500,1"
  "0,2000,1"
  "100,100,1"
  "100,500,5"
  "500,50,1"
  "500,200,5"
)

# ── Helpers ────────────────────────────────────────────────────────────

reset_mcp() {
  "$BINARY" snapshot "$DEVICE" \
    --set "mcp_gain=0" \
    --set "AccumulateCount=1" \
    --exposure-ms 100 \
    --output /tmp/bd-cpph3-halogen_mcp_reset.tiff \
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
printf 'Matrix: %d captures (halogen-only, low gain, short exposure)\n\n' "${#MATRIX[@]}"

META="$OUT_DIR/matrix.json"
printf '[\n' > "$META"
FIRST=1

for row in "${MATRIX[@]}"; do
  IFS=',' read -r gain exp_ms acc <<< "$row"
  eff_ms=$((exp_ms * acc))
  label="halogen_g${gain}_t${exp_ms}ms_acc${acc}"
  out_tiff="$OUT_DIR/${label}.tiff"

  printf '=== %-36s (MCP=%s, exp=%s ms × %s = %s ms eff) ===\n' \
    "$label" "$gain" "$exp_ms" "$acc" "$eff_ms"

  t_start="$(date +%s)"
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
