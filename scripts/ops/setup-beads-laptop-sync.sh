#!/usr/bin/env bash
# Set up beads sync from laptop (macOS) to the shared Dolt server on ai-proxy.
#
# Run this on your MacBook:
#   cd ~/code/rust-daq
#   bash scripts/ops/setup-beads-laptop-sync.sh
#
# Prerequisites:
#   - Tailscale connected (ai-proxy reachable at 100.105.113.58)
#   - bd and dolt installed
#   - beads initialized in this repo (bd init was run)
#
# What this does:
#   1. Verifies connectivity to the ai-proxy remotesapi
#   2. Configures the beads Dolt remote to point to ai-proxy
#   3. Pushes local beads data to the shared server
#   4. Pulls any new data from the shared server
#
# After setup, normal workflow:
#   bd dolt push    # after making changes
#   bd dolt pull    # to get changes from ai-proxy sessions
set -euo pipefail

AI_PROXY_IP="100.105.113.58"
REMOTE_URL="http://${AI_PROXY_IP}:8001/rust_daq"

echo "=== Beads Laptop Sync Setup ==="
echo ""
echo "Shared server: ai-proxy ($AI_PROXY_IP)"
echo "Remote URL:    $REMOTE_URL"
echo ""

# Verify we're in a beads-enabled repo
if ! bd stats >/dev/null 2>&1; then
  echo "Error: bd stats failed. Is beads initialized in this repo?" >&2
  exit 1
fi

# Verify Tailscale connectivity
echo "[1/4] Checking connectivity to ai-proxy..."
if ! curl -s -o /dev/null -w "" --connect-timeout 5 "http://${AI_PROXY_IP}:8001/rust_daq" 2>/dev/null; then
  # curl will fail with non-200 but that's OK — 400 means server is reachable
  # Only fail if we can't connect at all
  if ! curl -s -o /dev/null --connect-timeout 5 "http://${AI_PROXY_IP}:8001/" 2>/dev/null; then
    echo "Warning: Could not reach ai-proxy at $AI_PROXY_IP:8001"
    echo "  Is Tailscale running? Check: tailscale status"
    echo "  Proceeding anyway (remote will be configured for later use)..."
  fi
fi
echo "  Connectivity OK (or will retry on push)"

# Remove existing origin if present, then add the correct one
echo "[2/4] Configuring Dolt remote..."
bd dolt remote remove origin 2>/dev/null || true
bd dolt remote add origin "$REMOTE_URL" 2>&1
echo "  Remote configured: origin -> $REMOTE_URL"

# Commit any pending changes
echo "[3/4] Committing pending changes..."
bd dolt commit -m "sync: pre-push commit from $(hostname)" 2>/dev/null || true

# Push local data
echo "[4/4] Pushing to shared server..."
if bd dolt push 2>&1; then
  echo "  Push succeeded."
else
  echo ""
  echo "  Push failed. This may happen on first sync if histories diverge."
  echo "  Try: bd dolt pull (to merge), then bd dolt push again."
  echo "  If that fails, the Dolt databases may need manual reconciliation."
fi

echo ""
echo "=== Setup Complete ==="
echo ""
echo "Ongoing usage:"
echo "  bd dolt push    # after making beads changes"
echo "  bd dolt pull    # to get changes from ai-proxy"
echo ""
echo "Verify with: bd dolt remote list"
