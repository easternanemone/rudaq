#!/usr/bin/env bash
# Periodic target/ cleanup helper to keep local disk usage under control.
#
# Usage examples:
#   scripts/hygiene/target-maintenance.sh
#   scripts/hygiene/target-maintenance.sh --mode partial --threshold-gb 20
#   scripts/hygiene/target-maintenance.sh --force --mode full

set -euo pipefail

MODE="full"
THRESHOLD_GB=30
FORCE=false
DRY_RUN=false

usage() {
  cat <<'EOF'
Usage: scripts/hygiene/target-maintenance.sh [options]

Options:
  --mode <full|partial>    Cleanup mode (default: full)
  --threshold-gb <N>       Only run if target/ is >= N GiB (default: 30)
  --force                  Ignore threshold and run cleanup
  --dry-run                Print what would run, without changing files
  -h, --help               Show this help

Modes:
  full:    cargo clean
  partial: remove target/debug/incremental + cargo clean -p ui -p server
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode)
      MODE="${2:-}"
      shift 2
      ;;
    --threshold-gb)
      THRESHOLD_GB="${2:-}"
      shift 2
      ;;
    --force)
      FORCE=true
      shift
      ;;
    --dry-run)
      DRY_RUN=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ "$MODE" != "full" && "$MODE" != "partial" ]]; then
  echo "Invalid mode: $MODE (expected full or partial)" >&2
  exit 2
fi

if ! [[ "$THRESHOLD_GB" =~ ^[0-9]+$ ]]; then
  echo "Invalid --threshold-gb value: $THRESHOLD_GB" >&2
  exit 2
fi

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$REPO_ROOT"

if [[ ! -d target ]]; then
  echo "No target/ directory found at $REPO_ROOT. Nothing to do."
  exit 0
fi

before_kb="$(du -sk target | awk '{print $1}')"
before_human="$(du -sh target | awk '{print $1}')"
threshold_kb="$((THRESHOLD_GB * 1024 * 1024))"

echo "Target maintenance mode: $MODE"
echo "Target size before cleanup: $before_human"
echo "Threshold: ${THRESHOLD_GB}G"

if [[ "$FORCE" == "false" && "$before_kb" -lt "$threshold_kb" ]]; then
  echo "Target is below threshold. Skipping cleanup."
  exit 0
fi

if [[ "$DRY_RUN" == "true" ]]; then
  if [[ "$MODE" == "full" ]]; then
    echo "[dry-run] cargo clean"
  else
    echo "[dry-run] rm -rf target/debug/incremental"
    echo "[dry-run] cargo clean -p ui -p server"
  fi
  exit 0
fi

if [[ "$MODE" == "full" ]]; then
  cargo clean
else
  rm -rf target/debug/incremental
  cargo clean -p ui -p server
fi

after_kb=0
if [[ -d target ]]; then
  after_kb="$(du -sk target | awk '{print $1}')"
fi

reclaimed_kb=$((before_kb - after_kb))
if [[ "$reclaimed_kb" -lt 0 ]]; then
  reclaimed_kb=0
fi

after_human="0B"
if [[ -d target ]]; then
  after_human="$(du -sh target | awk '{print $1}')"
fi

reclaimed_human="$(awk -v kb="$reclaimed_kb" 'BEGIN {
  split("KB MB GB TB", u, " ");
  v = kb;
  i = 1;
  while (v >= 1024 && i < 4) { v /= 1024; i++; }
  printf "%.1f%s", v, u[i];
}')"

echo "Target size after cleanup: $after_human"
echo "Reclaimed: $reclaimed_human"
