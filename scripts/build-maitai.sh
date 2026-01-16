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

# Clean the relevant crates to avoid caching issues
echo "🧹 Cleaning cached build artifacts..."
cargo clean -p daq-bin -p rust_daq -p daq-driver-pvcam 2>/dev/null || true

# Build with maitai profile (includes pvcam_hardware)
echo "🔨 Building with maitai profile..."
cargo build --release -p daq-bin --features maitai

echo ""
echo "✅ Build complete!"
echo ""
echo "To start the daemon:"
echo "  ./target/release/rust-daq-daemon daemon --port 50051 --hardware-config config/maitai_hardware.toml"
