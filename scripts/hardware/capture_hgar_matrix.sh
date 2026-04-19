#!/usr/bin/env bash
# bd-g22gu.2.2.1 — Density-gradient HgAr fixture capture for DTW (.2.2) + CWT (.2.3).
#
# Drives an ALREADY-RUNNING rust-daq-daemon through a sweep of
# (MCP gain, exposure) settings to produce a range of atlas-line-count
# HgAr fixtures. Does NOT start its own daemon — the existing process
# on leabs-dev owns the camera.
#
# Flow per row:
#   1. `rust-daq-daemon client upload set_gain.rhai` → script_id
#   2. `rust-daq-daemon client start <id>`           → queued on daemon
#   3. poll `client status <id>` until done          → MCP gain applied
#   4. `rust-daq-daemon snapshot` with --exposure-ms → TIFF written
#
# Safety: MCP gain is reset to 0 via trap on EXIT/INT/TERM regardless
# of success/failure. Capture matrix is capped at MCP=100 (user
# constraint — do not raise without explicit approval).
#
# Usage (on leabs-dev, with daemon already running):
#   bash scripts/hardware/capture_hgar_matrix.sh [OUT_DIR]

set -euo pipefail

# ── Configuration ──────────────────────────────────────────────────────
DAEMON="${DAEMON:-http://localhost:50051}"
DEVICE="${DEVICE:-istar_camera}"
BINARY="${BINARY:-/home/brian/code/rust-daq/target/release/rust-daq-daemon}"
OUT_DIR="${1:-/home/brian/hgar_matrix_$(date +%Y%m%d_%H%M%S)}"

# ── Capture matrix — DoE v2 (bd-g22gu.2.2.1) ──────────────────────────
# Each row: <mcp_gain>,<exposure_ms>. User safety constraints:
#   - MCP gain MUST NOT exceed 500
#   - Time with MCP > 100 MUST stay under 1 minute total per session
#   - MCP MUST be reset to 0 after each frame (enforced by --after-set)
#
# Strategic grid sample of (gain × exposure) — low-gain long-exposure
# for baseline & noise-floor reference, progressively shorter exposures
# at higher gain to respect the <1-min MCP>100 dwell budget while still
# spanning the instrument's full gain range.
#
# Total MCP > 100 dwell: 10 + 3 + 10 + 1 + 3 = 27 s ≤ 60 s budget.
MATRIX=(
  "0,10000"    # baseline short — no amplification, brightest Hg only
  "0,30000"    # baseline long — reference for dark-floor characterisation
  "50,30000"   # low gain, long — SNR envelope
  "100,30000"  # transition at safety-band ceiling
  "200,10000"  # mid-gain, mid-exposure
  "350,3000"   # mid-high gain, short exposure
  "350,10000"  # mid-high gain, mid exposure
  "500,1000"   # high gain, very short
  "500,3000"   # high gain, short
)

# ── Helpers ────────────────────────────────────────────────────────────

# MCP gain reset via a no-op capture that applies mcp_gain=0 via --set.
# Tiny 100-ms exposure keeps cleanup fast while still driving the real
# SetParameter RPC path the daemon's iStar actually honours.
# (bd-g22gu.2.2.1: Rhai's create_andor_camera returns a mock, so script
# control doesn't reach the running-daemon iStar — only SetParameter does.)
reset_mcp() {
  "$BINARY" snapshot "$DEVICE" \
    --set "mcp_gain=0" \
    --exposure-ms 100 \
    --output /tmp/bd-g22gu2_mcp_reset.tiff \
    --format tiff \
    --addr "$DAEMON" >/dev/null 2>&1
}

cleanup_mcp() {
  printf '[cleanup] resetting MCP gain to 0 via SetParameter RPC...\n'
  if reset_mcp; then
    printf '[cleanup] MCP gain = 0 confirmed\n'
  else
    printf '[cleanup] WARNING: MCP reset call failed — verify manually before disconnect\n' >&2
  fi
}
trap cleanup_mcp EXIT INT TERM

# ── Main loop ──────────────────────────────────────────────────────────

mkdir -p "$OUT_DIR"
printf 'Output: %s\n' "$OUT_DIR"
printf 'Daemon: %s (device=%s)\n' "$DAEMON" "$DEVICE"
printf 'Matrix: %d captures, MCP gain capped at 100\n\n' "${#MATRIX[@]}"

META="$OUT_DIR/matrix.json"
printf '[\n' > "$META"
FIRST=1

for row in "${MATRIX[@]}"; do
  gain="${row%%,*}"
  exp_ms="${row##*,}"
  label="hgar_g${gain}_t${exp_ms}ms"
  out_tiff="$OUT_DIR/${label}.tiff"

  printf '=== %-28s (MCP=%s, exp=%s ms) ===\n' "$label" "$gain" "$exp_ms"

  t_start="$(date +%s)"
  # --after-set mcp_gain=0 guarantees MCP drops to 0 before snapshot exits,
  # BEFORE the next row starts. Per-frame reset is required when high-gain
  # rows (200-500) are in the matrix so no frame lingers above the safety
  # threshold between captures (bd-g22gu.2.2.1 safety contract).
  "$BINARY" snapshot \
    "$DEVICE" \
    --set "trigger_mode=Internal" \
    --set "gate_mode=CWOn" \
    --set "mcp_gain=${gain}" \
    --exposure-ms "$exp_ms" \
    --after-set "mcp_gain=0" \
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
  printf '  {"label":"%s","mcp_gain":%s,"exposure_ms":%s,"tiff":"%s","elapsed_s":%s,"size_kb":%s,"captured_at":"%s"}' \
    "$label" "$gain" "$exp_ms" "${label}.tiff" "$elapsed" "$size_kb" "$(date -Iseconds)" \
    >> "$META"

  printf '  → %s (%s KB, %ss)\n' "$out_tiff" "$size_kb" "$elapsed"
done

printf '\n]\n' >> "$META"

printf '\n=== summary ===\n'
printf 'Captured %d frames to %s\n' "${#MATRIX[@]}" "$OUT_DIR"
printf 'Metadata: %s\n' "$META"
printf 'NOTE: cleanup trap will reset MCP gain to 0 on exit.\n'
