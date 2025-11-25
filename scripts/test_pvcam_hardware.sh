#!/bin/bash
# PVCAM Hardware Test Script for maitai@100.117.5.12
#
# This script tests the PVCAM continuous streaming implementation
# on the remote machine with actual camera hardware.
#
# Usage:
#   Local:  ./scripts/test_pvcam_hardware.sh          # Run via SSH
#   Remote: ./scripts/test_pvcam_hardware.sh --local  # Run directly on maitai

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
REMOTE_HOST="maitai@100.117.5.12"
REMOTE_DIR="/home/maitai/rust-daq"
PVCAM_SDK_DIR="/opt/pvcam/sdk"
PVCAM_LIB_DIR="/opt/pvcam/library/x86_64"
PVCAM_UMD_PATH="/opt/pvcam/drivers/user-mode"

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[PASS]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[FAIL]${NC} $1"
}

run_remote_tests() {
    log_info "Running PVCAM hardware tests on $REMOTE_HOST"

    # SSH to remote and execute tests
    ssh -t "$REMOTE_HOST" bash << 'REMOTE_SCRIPT'
#!/bin/bash
set -e

cd /home/maitai/rust-daq || { echo "ERROR: rust-daq directory not found"; exit 1; }

# Set up PVCAM environment (all required for SDK to work)
export PVCAM_SDK_DIR="/opt/pvcam/sdk"
export PVCAM_LIB_DIR="/opt/pvcam/library/x86_64"
export PVCAM_UMD_PATH="/opt/pvcam/drivers/user-mode"
export LD_LIBRARY_PATH="$PVCAM_LIB_DIR:$LD_LIBRARY_PATH"

echo "=== PVCAM Hardware Test Suite ==="
echo "Date: $(date)"
echo "Host: $(hostname)"
echo ""

# Check PVCAM SDK installation
echo "--- Checking PVCAM SDK ---"
if [ -d "$PVCAM_SDK_DIR" ]; then
    echo "PVCAM_SDK_DIR: $PVCAM_SDK_DIR (exists)"
else
    echo "ERROR: PVCAM_SDK_DIR not found at $PVCAM_SDK_DIR"
    exit 1
fi

if [ -d "$PVCAM_LIB_DIR" ]; then
    echo "PVCAM_LIB_DIR: $PVCAM_LIB_DIR (exists)"
    ls -la "$PVCAM_LIB_DIR"/*.so 2>/dev/null | head -5 || echo "No .so files found"
else
    echo "ERROR: PVCAM_LIB_DIR not found at $PVCAM_LIB_DIR"
    exit 1
fi
echo ""

# Pull latest code
echo "--- Pulling latest code ---"
git fetch origin
git status
echo ""

# Check for uncommitted changes
if [ -n "$(git status --porcelain)" ]; then
    echo "WARNING: Uncommitted changes detected"
    git diff --stat
fi
echo ""

# Build with PVCAM hardware feature
echo "--- Building with PVCAM hardware features ---"
cargo build --features 'instrument_photometrics,pvcam_hardware' 2>&1 | tail -20
echo ""

# Run PVCAM unit tests (mock mode for sanity check)
echo "--- Running mock tests (sanity check) ---"
cargo test --features 'instrument_photometrics' hardware::pvcam 2>&1 | tail -20
echo ""

# Run PVCAM hardware tests
echo "--- Running HARDWARE tests ---"
echo "These tests require a connected Photometrics camera"
echo ""

# Test 1: Camera detection
echo "=== Test 1: Camera Detection ==="
cargo test --features 'instrument_photometrics,pvcam_hardware' hardware::pvcam::tests::test_camera_detection 2>&1 || {
    echo "Camera detection test failed - is camera connected?"
}
echo ""

# Test 2: Full hardware test suite
echo "=== Test 2: Full Hardware Test Suite ==="
cargo test --features 'instrument_photometrics,pvcam_hardware,hardware_tests' pvcam 2>&1 | tee /tmp/pvcam_test_results.txt
echo ""

# Test 3: Streaming tests (run sequentially to avoid SDK conflicts)
echo "=== Test 3: Streaming Tests ==="
cargo test --features 'instrument_photometrics,pvcam_hardware,hardware_tests' --test pvcam_streaming_test -- --test-threads=1 2>&1 | tee /tmp/pvcam_streaming_results.txt || {
    echo "Streaming tests failed - check output above"
}
echo ""

# Summary
echo "=== Test Summary ==="
echo ""
echo "Hardware validation tests:"
if grep -q "test result: ok" /tmp/pvcam_test_results.txt 2>/dev/null; then
    echo "  [PASS] Hardware validation tests passed"
else
    # Check how many passed vs failed
    passed=$(grep -oP '\d+ passed' /tmp/pvcam_test_results.txt 2>/dev/null | head -1 || echo "0 passed")
    failed=$(grep -oP '\d+ failed' /tmp/pvcam_test_results.txt 2>/dev/null | head -1 || echo "0 failed")
    echo "  [PARTIAL] Hardware validation: $passed, $failed"
fi

echo ""
echo "Streaming tests:"
if grep -q "test result: ok" /tmp/pvcam_streaming_results.txt 2>/dev/null; then
    echo "  [PASS] Streaming tests passed"
else
    echo "  [FAIL] Some streaming tests failed - check output above"
fi

echo ""
echo "Test complete at $(date)"
REMOTE_SCRIPT
}

run_local_tests() {
    log_info "Running PVCAM hardware tests locally"

    # Set up PVCAM environment (all required for SDK to work)
    export PVCAM_SDK_DIR="$PVCAM_SDK_DIR"
    export PVCAM_LIB_DIR="$PVCAM_LIB_DIR"
    export PVCAM_UMD_PATH="$PVCAM_UMD_PATH"
    export LD_LIBRARY_PATH="$PVCAM_LIB_DIR:$LD_LIBRARY_PATH"

    echo "=== PVCAM Hardware Test Suite (Local) ==="
    echo "Date: $(date)"
    echo "Host: $(hostname)"
    echo ""

    # Check PVCAM SDK
    if [ ! -d "$PVCAM_SDK_DIR" ]; then
        log_error "PVCAM SDK not found at $PVCAM_SDK_DIR"
        exit 1
    fi

    # Build
    log_info "Building with PVCAM hardware features..."
    cargo build --features 'instrument_photometrics,pvcam_hardware'

    # Run tests
    log_info "Running hardware tests..."
    cargo test --features 'instrument_photometrics,pvcam_hardware,hardware_tests' pvcam 2>&1 | tee /tmp/pvcam_test_results.txt

    # Summary
    if grep -q "test result: ok" /tmp/pvcam_test_results.txt; then
        log_success "All PVCAM hardware tests PASSED"
    else
        log_error "Some tests failed - check output above"
    fi
}

sync_code_to_remote() {
    log_info "Syncing code to remote machine..."

    # Use rsync to sync code (excluding target directory)
    rsync -avz --exclude 'target' --exclude '.git' --exclude '*.log' \
        "$(dirname "$0")/../" "$REMOTE_HOST:$REMOTE_DIR/"

    log_success "Code synced to $REMOTE_HOST:$REMOTE_DIR"
}

show_help() {
    echo "PVCAM Hardware Test Script"
    echo ""
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --local       Run tests locally (on maitai machine)"
    echo "  --sync        Sync code to remote before testing"
    echo "  --help        Show this help message"
    echo ""
    echo "Examples:"
    echo "  $0              # Run tests via SSH on maitai"
    echo "  $0 --sync       # Sync code then run tests"
    echo "  $0 --local      # Run directly on maitai (when already SSH'd in)"
}

# Main
case "${1:-}" in
    --local)
        run_local_tests
        ;;
    --sync)
        sync_code_to_remote
        run_remote_tests
        ;;
    --help|-h)
        show_help
        ;;
    *)
        run_remote_tests
        ;;
esac
