#!/bin/bash
# Build script for maitai lab machine with COMPLETE REAL HARDWARE SUPPORT
#
# Usage: bash scripts/ops/build-maitai.sh
#
# The 'maitai' feature flag enables native SDK hardware drivers:
#   - PVCAM (real SDK, not mock)
#   - Comedi DAQ card
#   - Serial port communication
#
# Serial/SCPI devices (ELL14, ESP300, MaiTai, 1830-C, Red Pitaya PID)
# use driver-universal with TOML manifests from config/devices/.
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
echo "   Enabled native SDK drivers:"
echo "     ✓ PVCAM camera (real SDK)"
echo "     ✓ Comedi DAQ card"
echo "     ✓ Serial port communication"
echo ""
echo "   Serial/SCPI devices (via driver-universal TOML manifests):"
echo "     ✓ ELL14 rotators, ESP300 motion, MaiTai laser, 1830-C power meter, etc."
echo ""

# Clean build artifacts to avoid feature flag caching issues
# NOTE: Full clean is required because feature flags are baked into dependencies.
# Partial cleaning (cargo clean -p <crate>) doesn't properly invalidate transitive deps.
echo "🧹 Cleaning build artifacts (full clean for feature flag reliability)..."
cargo clean 2>/dev/null || true

# Build with maitai profile (includes pvcam_hardware)
echo "🔨 Building with maitai profile..."
cargo build --release -p bin --features maitai

echo ""
echo "═══════════════════════════════════════════════════════"
echo "  Build complete! To restart the daemon:"
echo ""
echo "    systemctl --user restart rust-daq-daemon"
echo ""
echo "  To check status:"
echo ""
echo "    systemctl --user status rust-daq-daemon"
echo "    journalctl --user -u rust-daq-daemon -f"
echo "═══════════════════════════════════════════════════════"
echo ""
echo "📋 Verification checklist:"
echo "   1. Check daemon log for: 'pvcam_sdk feature enabled: true'"
echo "   2. Verify: 'Successfully opened camera' with real handle (not mock)"
echo "   3. Confirm: 'Registered 7 device(s)' including:"
echo "      - prime_bsi (PVCAM camera)"
echo "      - maitai (laser)"
echo "      - power_meter (Newport 1830-C)"
echo "      - rotator_2, rotator_3, rotator_8 (ELL14)"
echo "      - esp300_axis1 (Newport ESP300)"
echo ""
echo "❌ If daemon shows 'using mock mode', the build is INCORRECT - rebuild!"
