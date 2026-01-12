#!/bin/bash
# Build rust-daq daemon with real PVCAM SDK support for lab hardware
#
# Usage: ./scripts/build-lab.sh [--release]
#
# This script builds the daemon with pvcam_sdk feature enabled,
# which is required for real Prime BSI camera streaming.
# Without this feature, the daemon uses mock camera data.

set -e

# Source environment if available
if [ -f "scripts/env-check.sh" ]; then
    source scripts/env-check.sh 2>/dev/null || true
elif [ -f "config/hosts/maitai.env" ]; then
    source config/hosts/maitai.env
fi

# Parse arguments
PROFILE=""
if [ "${1:-}" = "--release" ]; then
    PROFILE="--release"
    echo "Building in RELEASE mode with pvcam_sdk..."
else
    echo "Building in DEBUG mode with pvcam_sdk..."
fi

# Build with real PVCAM SDK
cargo build --features pvcam_sdk -p daq-bin $PROFILE

echo ""
echo "Build complete!"
echo ""
echo "To run with lab hardware:"
if [ "${1:-}" = "--release" ]; then
    echo "  ./target/release/rust-daq-daemon daemon --port 50051 --lab-hardware"
else
    echo "  ./target/debug/rust-daq-daemon daemon --port 50051 --lab-hardware"
fi
