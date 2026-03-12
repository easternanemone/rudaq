#!/usr/bin/env bash
#
# fast-check.sh - Quick workspace smoke test (bd-pman.3.1)
# Skips slow UI WASM builds and hardware-dependent tests.

set -euo pipefail

echo "🚀 Running fast-check smoke test..."

echo "[1/3] Checking workspace compilation (excluding UI)..."
cargo check --workspace --exclude ui

echo "[2/3] Running crate-level unit tests..."
cargo nextest run --workspace --exclude ui --exclude comedi-sys --exclude driver-comedi --profile ci

echo "[3/3] Running doc tests..."
cargo test --doc --workspace --exclude ui

echo "✅ Fast-check passed! For full verification, use scripts/build-maitai.sh or CI."
