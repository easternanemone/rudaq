#!/bin/bash
# Run tests on maitai-eos and download results
#
# Usage: ./scripts/remote/run_tests_remote.sh [OPTIONS]
#
# Options:
#   --suite SUITE        Test suite: all|lib|integration|hardware (default: all)
#   --release            Run in release mode (slower compile, faster tests)
#   --no-capture         Show println! output
#   --threads N          Number of test threads (default: parallel)
#   --timeout SECONDS    Timeout for entire test run (default: 3600)
#   --no-download        Don't download results
#   --help               Show this help

set -e

# Configuration
REMOTE_HOST="maitai-eos"
REMOTE_DIR="~/rust-daq"
TEST_SUITE="all"
RELEASE_MODE=false
NO_CAPTURE=false
TEST_THREADS=""
TIMEOUT=3600
DOWNLOAD_RESULTS=true
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULTS_DIR="./test_results/${TIMESTAMP}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Helper functions
log() {
    echo -e "${BLUE}[$(date +'%H:%M:%S')]${NC} $1"
}

success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
    exit 1
}

warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

header() {
    echo ""
    echo -e "${YELLOW}=== $1 ===${NC}"
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --suite)
            TEST_SUITE="$2"
            shift 2
            ;;
        --release)
            RELEASE_MODE=true
            shift
            ;;
        --no-capture)
            NO_CAPTURE=true
            shift
            ;;
        --threads)
            TEST_THREADS="$2"
            shift 2
            ;;
        --timeout)
            TIMEOUT="$2"
            shift 2
            ;;
        --no-download)
            DOWNLOAD_RESULTS=false
            shift
            ;;
        --help)
            grep "^#" "$0" | head -20
            exit 0
            ;;
        *)
            error "Unknown option: $1"
            ;;
    esac
done

# Validate test suite
case "$TEST_SUITE" in
    all|lib|integration|hardware)
        ;;
    *)
        error "Invalid test suite: $TEST_SUITE. Use: all, lib, integration, or hardware"
        ;;
esac

# Verify prerequisites
header "Pre-Test Checks"

log "Checking SSH connection..."
if ! ssh -o ConnectTimeout=5 "$REMOTE_HOST" 'echo "OK"' > /dev/null 2>&1; then
    error "Cannot connect to $REMOTE_HOST"
fi
success "SSH OK"

log "Checking remote directory..."
if ! ssh "$REMOTE_HOST" "test -d $REMOTE_DIR"; then
    error "Remote directory not found: $REMOTE_DIR. Run deploy_to_maitai.sh first."
fi
success "Remote directory exists"

# Create results directory
mkdir -p "$RESULTS_DIR"
log "Results will be saved to: $RESULTS_DIR"

# Build cargo test command
header "Building Test Command"

TEST_CMD="cd $REMOTE_DIR && "

# Build arguments
CARGO_ARGS="test"

if [ "$RELEASE_MODE" = true ]; then
    CARGO_ARGS="$CARGO_ARGS --release"
    log "Mode: Release (slower compile, faster execution)"
else
    log "Mode: Debug (faster compile)"
fi

case "$TEST_SUITE" in
    lib)
        CARGO_ARGS="$CARGO_ARGS --lib"
        log "Suite: Library unit tests only"
        ;;
    integration)
        CARGO_ARGS="$CARGO_ARGS --test '*'"
        log "Suite: Integration tests only"
        ;;
    hardware)
        CARGO_ARGS="$CARGO_ARGS --test '*hardware*'"
        log "Suite: Hardware integration tests only"
        ;;
    all)
        log "Suite: All tests (unit + integration)"
        ;;
esac

# Add test arguments
CARGO_ARGS="$CARGO_ARGS -- --nocapture"

if [ -n "$TEST_THREADS" ]; then
    CARGO_ARGS="$CARGO_ARGS --test-threads=$TEST_THREADS"
    log "Threads: $TEST_THREADS"
else
    log "Threads: Parallel (default)"
fi

log "Full command: cargo $CARGO_ARGS"

# Run tests
header "Running Tests"

TEST_CMD="timeout $TIMEOUT cargo $CARGO_ARGS 2>&1 | tee test_output.log"

log "Starting test run (timeout: ${TIMEOUT}s)..."
echo ""

# Create a script to run tests on remote
if ssh "$REMOTE_HOST" "cd $REMOTE_DIR && $TEST_CMD"; then
    TEST_RESULT=0
    success "Tests completed"
else
    TEST_RESULT=$?
    if [ $TEST_RESULT -eq 124 ]; then
        warning "Tests timed out after ${TIMEOUT}s"
    else
        warning "Tests failed with exit code: $TEST_RESULT"
    fi
fi

echo ""

# Download results
if [ "$DOWNLOAD_RESULTS" = true ]; then
    header "Downloading Results"

    log "Copying test output..."
    if scp "$REMOTE_HOST:$REMOTE_DIR/test_output.log" "$RESULTS_DIR/test_output.log" 2>/dev/null; then
        success "Test output downloaded"
    else
        warning "Could not download test output"
    fi

    log "Copying any result files..."
    scp -r "$REMOTE_HOST:$REMOTE_DIR/results/" "$RESULTS_DIR/" 2>/dev/null || true

    log "Copying logs..."
    scp "$REMOTE_HOST:$REMOTE_DIR"/*.log "$RESULTS_DIR/" 2>/dev/null || true

    # Parse results
    if [ -f "$RESULTS_DIR/test_output.log" ]; then
        log "Parsing test results..."

        PASSED=$(grep -c "test.*ok" "$RESULTS_DIR/test_output.log" 2>/dev/null || echo "0")
        FAILED=$(grep -c "test.*FAILED" "$RESULTS_DIR/test_output.log" 2>/dev/null || echo "0")

        echo ""
        echo "Test Summary:"
        echo "  Passed: $PASSED"
        echo "  Failed: $FAILED"
        echo ""
    fi
fi

# Create manifest
cat > "$RESULTS_DIR/manifest.txt" << EOF
Test Run Results
================

Date: $(date)
Remote Host: $REMOTE_HOST
Test Suite: $TEST_SUITE
Release Mode: $RELEASE_MODE
Timeout: ${TIMEOUT}s
Exit Code: $TEST_RESULT

Files:
  test_output.log - Full test output
  results/        - Test result files
  *.log           - Additional logs
EOF

log "Manifest saved to: $RESULTS_DIR/manifest.txt"

# Summary
header "Summary"

if [ $TEST_RESULT -eq 0 ]; then
    success "All tests passed!"
    echo ""
    echo "Results location: $RESULTS_DIR"
    echo ""
    ls -lah "$RESULTS_DIR"
    exit 0
else
    warning "Some tests failed"
    echo ""
    echo "Results location: $RESULTS_DIR"
    echo ""

    # Show failures
    if [ -f "$RESULTS_DIR/test_output.log" ]; then
        echo "Failed tests:"
        grep "test.*FAILED" "$RESULTS_DIR/test_output.log" 2>/dev/null || echo "No failures found in log"
    fi

    exit $TEST_RESULT
fi
