#!/bin/bash
# Run on a self-hosted runner host AFTER recovering from a disk-wedge
# (i.e. you just power-cycled because `/` hit 100% and sshd/journald hung).
#
# This script:
#   1. Frees the obvious offenders (cargo target/ trees, oversized journals)
#   2. Drops VFS caches
#   3. (Re)installs the disk-watchdog so the wedge can't recur
#   4. Restarts the GitHub runner service
#
# Idempotent — safe to run on a healthy host too.
#
# Usage:
#   sudo bash scripts/ops/disk-safety/recover-disk-wedge.sh

set -euo pipefail

if [[ "$EUID" -ne 0 ]]; then
  echo "Run as root (sudo bash $0)" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "--- before ---"
df -h /

# 1. Cargo target/ trees we know about. Add more here if you have multiple checkouts.
TARGETS=(
  /home/maitai/code/rust-daq/target
  /home/maitai/actions-runner-rust-daq/_work/rust-daq/rust-daq/target
  /home/brian/code/rust-daq/target
  /home/brian/actions-runner-rust-daq/_work/rust-daq/rust-daq/target
)
for t in "${TARGETS[@]}"; do
  if [[ -d "$t" ]]; then
    sz=$(du -sh "$t" 2>/dev/null | awk '{print $1}')
    echo "removing $t (size=$sz)"
    rm -rf -- "$t"
  fi
done

# 2. Trim journald to a reasonable size
journalctl --vacuum-size=200M >/dev/null 2>&1 || true

# 3. Drop pagecache (the actual blocks were freed by rm; this just releases the inode cache)
sync
echo 3 > /proc/sys/vm/drop_caches 2>/dev/null || true

echo
echo "--- after free ---"
df -h /

# 4. Install (or refresh) the watchdog
echo
echo "--- installing watchdog ---"
bash "$SCRIPT_DIR/install-on-runner.sh"

# 5. Restart any actions runner that's installed but not running
mapfile -t RUNNERS < <(systemctl list-unit-files --type=service --no-legend 2>/dev/null \
  | awk '/^actions\.runner\..*\.service/ {print $1}')

if [[ "${#RUNNERS[@]}" -eq 0 ]]; then
  echo "no actions.runner.*.service found"
else
  for svc in "${RUNNERS[@]}"; do
    echo "starting $svc"
    systemctl start "$svc" || echo "warning: failed to start $svc"
    systemctl status "$svc" --no-pager --lines=0 || true
  done
fi

echo
echo "Recovery complete."
