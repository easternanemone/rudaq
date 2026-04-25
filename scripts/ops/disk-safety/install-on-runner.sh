#!/bin/bash
# Install the disk-watchdog on the current self-hosted runner host.
# Idempotent: re-running just refreshes files and re-enables the timer.
#
# Usage:
#   sudo bash scripts/ops/disk-safety/install-on-runner.sh
#
# Verifies installation via systemctl status and a synchronous first run.

set -euo pipefail

if [[ "$EUID" -ne 0 ]]; then
  echo "Run as root (sudo bash $0)" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "Installing disk-watchdog from $SCRIPT_DIR"

install -m 0755 "$SCRIPT_DIR/disk-watchdog.sh"      /usr/local/bin/disk-watchdog.sh
install -m 0644 "$SCRIPT_DIR/disk-watchdog.service" /etc/systemd/system/disk-watchdog.service
install -m 0644 "$SCRIPT_DIR/disk-watchdog.timer"   /etc/systemd/system/disk-watchdog.timer
install -d -m 0755 /usr/local/share/doc/disk-safety
install -m 0644 "$SCRIPT_DIR/README.md"             /usr/local/share/doc/disk-safety/README.md

mkdir -p /var/lib/disk-watchdog

# Seed default tunables file if absent. Operators edit /etc/default/disk-watchdog
# to override; reinstall does not clobber it.
if [[ ! -f /etc/default/disk-watchdog ]]; then
  cat > /etc/default/disk-watchdog <<'EOF'
# Tunables for /usr/local/bin/disk-watchdog.sh — uncomment and edit to override.
# WARN_GIB=25            # below this, clean idle target/ dirs
# CRIT_GIB=10            # below this, stop runner and clean everything
# RESUME_GIB=30          # above this, restart runner if watchdog stopped it
# STALE_MINUTES=30       # at WARN, only delete target/ idle this long
# PARTITION=/
# Comma-separated absolute paths:
# TARGETS=/home/maitai/code/rust-daq/target,/home/maitai/actions-runner-rust-daq/_work/rust-daq/rust-daq/target
# Optional explicit override; if unset, the script auto-discovers actions.runner.*.service:
# RUNNER_SERVICE=actions.runner.TheFermiSea-rust-daq.maitai-eos.service
EOF
  echo "Seeded /etc/default/disk-watchdog (defaults active)"
fi

systemctl daemon-reload
systemctl enable --now disk-watchdog.timer

echo
echo "--- timer status ---"
systemctl status disk-watchdog.timer --no-pager || true

echo
echo "--- one synchronous run to verify ---"
systemctl start disk-watchdog.service
sleep 1
journalctl -t disk-watchdog -n 10 --no-pager || true

echo
echo "Install complete. Tail live:  journalctl -t disk-watchdog -f"
