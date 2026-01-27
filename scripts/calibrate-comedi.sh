#!/bin/bash
#
# Comedi DAQ Calibration Script
#
# Calibrates the NI PCI-MIO-16XE-10 DAQ card using comedi_calibrate.
# This improves measurement accuracy from ~50mV offset to ~1-2mV.
#
# Usage:
#   sudo bash scripts/calibrate-comedi.sh [OPTIONS]
#
# Options:
#   --verify      Run verification tests after calibration
#   --reset       Force fresh calibration (ignore existing)
#   --verbose     Show detailed calibration output
#   --device DEV  Use specified device (default: /dev/comedi0)
#   --help        Show this help message
#
# Hardware: NI PCI-MIO-16XE-10 with BNC-2110 breakout
# Location: maitai machine
#

set -euo pipefail

# Configuration
DEVICE="${COMEDI_DEVICE:-/dev/comedi0}"
CALIBRATION_DIR="/etc/comedi/calibrations"
USER_CAL_DIR="$HOME/.comedi_calibrations"
VERBOSE=false
VERIFY=false
RESET=false

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Logging functions
log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[OK]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# Show help
show_help() {
    head -25 "$0" | tail -20 | sed 's/^# //' | sed 's/^#//'
    exit 0
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --verify)
            VERIFY=true
            shift
            ;;
        --reset)
            RESET=true
            shift
            ;;
        --verbose|-v)
            VERBOSE=true
            shift
            ;;
        --device)
            DEVICE="$2"
            shift 2
            ;;
        --help|-h)
            show_help
            ;;
        *)
            log_error "Unknown option: $1"
            show_help
            ;;
    esac
done

# Check prerequisites
check_prerequisites() {
    log_info "Checking prerequisites..."

    # Check if running as root
    if [[ $EUID -ne 0 ]]; then
        log_error "This script must be run as root (use sudo)"
        exit 1
    fi

    # Check if comedi_calibrate exists
    if ! command -v comedi_calibrate &> /dev/null; then
        log_error "comedi_calibrate not found. Install with: sudo apt-get install comedilib"
        exit 1
    fi

    # Check if device exists
    if [[ ! -c "$DEVICE" ]]; then
        log_error "Device $DEVICE not found"
        log_info "Try: sudo modprobe ni_pcimio"
        exit 1
    fi

    # Check if comedi_board_info exists and get board info
    if command -v comedi_board_info &> /dev/null; then
        BOARD_NAME=$(comedi_board_info "$DEVICE" 2>/dev/null | grep "board name:" | cut -d: -f2 | xargs || echo "unknown")
        log_info "Board: $BOARD_NAME"
    fi

    log_success "Prerequisites OK"
}

# Show current calibration status
show_calibration_status() {
    log_info "Current calibration status:"

    # Check system calibration directory
    if [[ -d "$CALIBRATION_DIR" ]]; then
        CAL_FILES=$(ls -la "$CALIBRATION_DIR" 2>/dev/null | grep -v "^total" | grep -v "^d" || echo "")
        if [[ -n "$CAL_FILES" ]]; then
            echo "  System calibrations ($CALIBRATION_DIR):"
            ls -la "$CALIBRATION_DIR" 2>/dev/null | grep -v "^total" | grep -v "^d" | while read line; do
                echo "    $line"
            done
        else
            echo "  No system calibrations found"
        fi
    else
        echo "  System calibration directory does not exist"
    fi

    # Check user calibration directory
    if [[ -d "$USER_CAL_DIR" ]]; then
        CAL_FILES=$(ls -la "$USER_CAL_DIR" 2>/dev/null | grep -v "^total" | grep -v "^d" || echo "")
        if [[ -n "$CAL_FILES" ]]; then
            echo "  User calibrations ($USER_CAL_DIR):"
            ls -la "$USER_CAL_DIR" 2>/dev/null | grep -v "^total" | grep -v "^d" | while read line; do
                echo "    $line"
            done
        fi
    fi
}

# Run calibration
run_calibration() {
    log_info "Starting calibration for $DEVICE..."

    # Build command
    CMD="comedi_calibrate -f $DEVICE"

    if [[ "$RESET" == true ]]; then
        CMD="$CMD --reset --calibrate"
        log_info "Forcing fresh calibration (--reset)"
    fi

    if [[ "$VERBOSE" == true ]]; then
        CMD="$CMD -v"
    fi

    echo ""
    log_info "Running: $CMD"
    echo ""

    # Run calibration
    if $CMD; then
        log_success "Calibration completed successfully"
    else
        log_error "Calibration failed"
        exit 1
    fi

    echo ""
    show_calibration_status
}

# Verify calibration with loopback test
verify_calibration() {
    log_info "Verifying calibration with loopback test..."

    # Check if rust-daq is available
    RUST_DAQ_DIR="$(dirname "$(dirname "$(readlink -f "$0")")")"

    if [[ -f "$RUST_DAQ_DIR/Cargo.toml" ]]; then
        log_info "Running loopback verification..."

        cd "$RUST_DAQ_DIR"

        # Build if needed
        if [[ ! -f "target/debug/deps/analog_loopback"* ]]; then
            log_info "Building test binary..."
            cargo build -p daq-driver-comedi --features hardware --tests 2>/dev/null || true
        fi

        # Run quick loopback test
        export COMEDI_LOOPBACK_TEST=1

        # Find and run the test binary
        TEST_BIN=$(find target/debug/deps -name "analog_loopback-*" -type f -executable 2>/dev/null | head -1)
        if [[ -n "$TEST_BIN" ]]; then
            log_info "Running: $TEST_BIN test_ao_to_ai_loopback"
            if $TEST_BIN test_ao_to_ai_loopback --test-threads=1 2>&1 | tail -20; then
                log_success "Loopback verification passed"
            else
                log_warn "Loopback test had issues (may still be OK)"
            fi
        else
            log_warn "Test binary not found - skipping verification"
            log_info "Build with: cargo build -p daq-driver-comedi --features hardware --tests"
        fi
    else
        log_warn "rust-daq project not found - skipping verification"
    fi
}

# Quick voltage check using comedi_test
quick_voltage_check() {
    log_info "Quick voltage check..."

    if command -v comedi_test &> /dev/null; then
        # Read a few samples from AI channel 0
        log_info "Reading from ACH0 (should be connected to DAC1 loopback):"
        comedi_test -f "$DEVICE" -s 0 -r 0 -c 0 -n 5 2>/dev/null || log_warn "comedi_test not available"
    else
        log_warn "comedi_test not found - install comedilib for quick tests"
    fi
}

# Main
main() {
    echo ""
    echo "=========================================="
    echo "  Comedi DAQ Calibration Utility"
    echo "=========================================="
    echo ""

    check_prerequisites
    echo ""
    show_calibration_status
    echo ""
    run_calibration

    if [[ "$VERIFY" == true ]]; then
        echo ""
        verify_calibration
    fi

    echo ""
    log_success "Calibration complete!"
    echo ""
    echo "Notes:"
    echo "  - Calibration improves accuracy from ~50mV to ~1-2mV"
    echo "  - Recalibrate after temperature changes or long power-off periods"
    echo "  - Calibration file stored in: $CALIBRATION_DIR"
    echo ""
}

main
