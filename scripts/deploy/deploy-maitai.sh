#!/usr/bin/env bash
# deploy-maitai.sh — Thin wrapper around deploy.sh for maitai target
#
# Usage:
#   bash scripts/deploy/deploy-maitai.sh                           # Full deploy from main
#   bash scripts/deploy/deploy-maitai.sh --branch feat/my-feature  # Deploy a feature branch
#   bash scripts/deploy/deploy-maitai.sh --with-db                 # Enable SQLite persistence
#   bash scripts/deploy/deploy-maitai.sh --gui-only                # Just launch GUI (daemon running)
#   bash scripts/deploy/deploy-maitai.sh --skip-build --daemon-only  # Restart daemon, skip build
#
# All flags are forwarded to deploy.sh. See: bash scripts/deploy/deploy.sh --help

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec bash "${SCRIPT_DIR}/deploy.sh" --target maitai "$@"
