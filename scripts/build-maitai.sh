#!/bin/bash
# Build script for maitai lab machine with real PVCAM hardware support
#
# Usage: source scripts/build-maitai.sh
#
# This script ensures the daemon is built with real PVCAM SDK support,
# avoiding the common issue where cached builds use mock mode.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

# Source environment if available
if [[ -f "config/hosts/maitai.env" ]]; then
    echo "📋 Loading maitai environment..."
    source config/hosts/maitai.env
fi

# Verify PVCAM environment
if [[ -z "$PVCAM_SDK_DIR" ]]; then
    echo "⚠️  PVCAM_SDK_DIR not set. Run: source config/hosts/maitai.env"
    exit 1
fi

echo "🔧 Building daemon with PVCAM hardware support..."
echo "   PVCAM_SDK_DIR=$PVCAM_SDK_DIR"
echo "   PVCAM_VERSION=$PVCAM_VERSION"

# Clean build artifacts to avoid feature flag caching issues
# NOTE: Full clean is required because feature flags are baked into dependencies.
# Partial cleaning (cargo clean -p <crate>) doesn't properly invalidate transitive deps.
echo "🧹 Cleaning build artifacts (full clean for feature flag reliability)..."
cargo clean 2>/dev/null || true

# Build with maitai profile (includes pvcam_hardware)
echo "🔨 Building with maitai profile..."
cargo build --release -p daq-bin --features maitai

echo ""
echo "✅ Build complete!"
echo ""
echo "To start the daemon:"
echo "  ./target/release/rust-daq-daemon daemon --port 50051 --hardware-config config/maitai_hardware.toml"
