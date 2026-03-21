#!/usr/bin/env bash
# deploy-leabs.sh — Thin wrapper around deploy.sh for leabs-dev target
#
# Usage:
#   bash scripts/deploy/deploy-leabs.sh                           # Full deploy from main
#   bash scripts/deploy/deploy-leabs.sh --branch feat/my-feature  # Deploy a feature branch
#   bash scripts/deploy/deploy-leabs.sh --gui-only                # Just launch GUI (daemon running)
#   bash scripts/deploy/deploy-leabs.sh --skip-build --daemon-only  # Restart daemon, skip build
#   bash scripts/deploy/deploy-leabs.sh --wasm-gui                # Build+serve WASM GUI on port 8080
#
# All flags are forwarded to deploy.sh. See: bash scripts/deploy/deploy.sh --help

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec bash "${SCRIPT_DIR}/deploy.sh" --target leabs-dev "$@"
