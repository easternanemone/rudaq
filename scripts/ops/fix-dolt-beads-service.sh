#!/usr/bin/env bash
# Fix the dolt-beads systemd service to point to the correct shared-server directory.
# Run this script directly in a terminal (not through Claude Code) so sudo can prompt for your password.
#
# Usage: bash scripts/ops/fix-dolt-beads-service.sh
set -euo pipefail

echo "=== Fixing dolt-beads.service ==="
echo ""
echo "Current service points to: /home/brian/code/beefcake-swarm/.beads/dolt (stale)"
echo "Will update to:            /home/brian/.beads/shared-server/dolt"
echo ""

# 1. Stop the crash-looping service
echo "[1/5] Stopping crash-looping service..."
sudo systemctl stop dolt-beads.service

# 2. Kill the manually-started Dolt server if still running
MANUAL_PID="$(pgrep -f 'dolt sql-server --config config.yaml' || true)"
if [[ -n "$MANUAL_PID" ]]; then
  echo "[2/5] Killing manually-started Dolt server (PID $MANUAL_PID)..."
  kill "$MANUAL_PID" 2>/dev/null || true
  sleep 2
  # Verify it's gone
  if kill -0 "$MANUAL_PID" 2>/dev/null; then
    echo "  Warning: process still alive, sending SIGKILL..."
    kill -9 "$MANUAL_PID" 2>/dev/null || true
  fi
else
  echo "[2/5] No manual Dolt server found, skipping."
fi

# 3. Update the service file
echo "[3/5] Updating systemd service file..."
sudo tee /etc/systemd/system/dolt-beads.service > /dev/null << 'EOF'
[Unit]
Description=Dolt SQL Server for Beads Issue Sync
After=network.target

[Service]
Type=simple
User=brian
WorkingDirectory=/home/brian/.beads/shared-server/dolt
ExecStart=/usr/local/bin/dolt sql-server --config config.yaml
Restart=always
RestartSec=5
StandardOutput=append:/var/log/dolt-beads.log
StandardError=append:/var/log/dolt-beads.log

[Install]
WantedBy=multi-user.target
EOF

# 4. Reload and start
echo "[4/5] Reloading systemd and starting service..."
sudo systemctl daemon-reload
sudo systemctl start dolt-beads.service
sleep 2

# 5. Verify
echo "[5/5] Verifying..."
sudo systemctl status dolt-beads.service --no-pager
echo ""
echo "=== Done. Service should now be running from ~/.beads/shared-server/dolt ==="
echo ""
echo "Verify beads connectivity:"
echo "  cd ~/code/rust-daq && bd dolt show"
echo "  bd dolt push"
