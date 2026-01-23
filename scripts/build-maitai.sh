#!/bin/bash
# Build script for maitai lab machine with COMPLETE REAL HARDWARE SUPPORT
#
# Usage: bash scripts/build-maitai.sh
#
# The 'maitai' feature flag enables ALL hardware drivers:
#   - PVCAM (real SDK, not mock)
#   - Thorlabs ELL14 rotators
#   - Newport ESP300 motion controller
#   - Newport 1830-C power meter
#   - Spectra-Physics MaiTai laser
#   - Serial port communication
#
# This script ensures proper build by:
#   1. Loading PVCAM environment variables
#   2. Performing full clean (critical for feature flag changes)
#   3. Building with --features maitai

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

echo "🔧 Building daemon with ALL REAL HARDWARE (maitai feature)..."
echo "   PVCAM_SDK_DIR=$PVCAM_SDK_DIR"
echo "   PVCAM_VERSION=$PVCAM_VERSION"
echo ""
echo "   Enabled drivers:"
echo "     ✓ PVCAM camera (real SDK)"
echo "     ✓ Thorlabs ELL14 rotators"
echo "     ✓ Newport ESP300 motion controller"
echo "     ✓ Newport 1830-C power meter"
echo "     ✓ Spectra-Physics MaiTai laser"
echo "     ✓ Serial port communication"
echo ""

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
echo "📋 Verification checklist:"
echo "   1. Start daemon with: ./target/release/rust-daq-daemon daemon --port 50051 --hardware-config config/maitai_hardware.toml"
echo "   2. Check daemon log for: 'pvcam_sdk feature enabled: true'"
echo "   3. Verify: 'Successfully opened camera' with real handle (not mock)"
echo "   4. Confirm: 'Registered 7 device(s)' including:"
echo "      - prime_bsi (PVCAM camera)"
echo "      - maitai (laser)"
echo "      - power_meter (Newport 1830-C)"
echo "      - rotator_2, rotator_3, rotator_8 (ELL14)"
echo "      - esp300_axis1 (Newport ESP300)"
echo ""
echo "❌ If daemon shows 'using mock mode', the build is INCORRECT - rebuild!"
