#!/usr/bin/env bash
#
# fast-check.sh — Quick developer smoke test (convenience, NOT a gate)
#
# Use this for fast iteration feedback during development.
# For the canonical pre-push quality gate, use: scripts/ci/pre-push-gate.sh
#
# Differences from pre-push-gate.sh:
#   - Runs cargo check instead of clippy (faster, no lint)
#   - Includes doctests (pre-push-gate does not)
#   - Does NOT run cargo fmt --check
#   - Uses CI nextest profile (3 retries)

set -euo pipefail

echo "🚀 Running fast-check smoke test..."

echo "[1/3] Checking workspace compilation (excluding UI)..."
cargo check --workspace --exclude ui

echo "[2/3] Running crate-level unit tests..."
cargo nextest run --workspace --exclude ui --exclude comedi-sys --exclude driver-comedi --profile ci

echo "[3/3] Running doc tests..."
cargo test --doc --workspace --exclude ui

echo "✅ Fast-check passed! For full verification, use scripts/ops/build-maitai.sh or CI."
