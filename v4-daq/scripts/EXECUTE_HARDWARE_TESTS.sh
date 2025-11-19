#!/bin/bash
# Hardware Test Execution Script for maitai-eos
# This script must be run ON maitai-eos after SSH connection

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LOG_DIR="$PROJECT_ROOT/test-results/$(date +%Y-%m-%d_%H-%M-%S)"
CURRENT_USER=$(whoami)
EXPECTED_HOST="maitai-eos"

# Create log directory
mkdir -p "$LOG_DIR"
LOG_FILE="$LOG_DIR/execution.log"

# Logging function
log() {
    echo -e "${BLUE}[$(date '+%Y-%m-%d %H:%M:%S')]${NC} $*" | tee -a "$LOG_FILE"
}

log_success() {
    echo -e "${GREEN}[$(date '+%Y-%m-%d %H:%M:%S')] ✓${NC} $*" | tee -a "$LOG_FILE"
}

log_error() {
    echo -e "${RED}[$(date '+%Y-%m-%d %H:%M:%S')] ✗${NC} $*" | tee -a "$LOG_FILE"
}

log_warning() {
    echo -e "${YELLOW}[$(date '+%Y-%m-%d %H:%M:%S')] ⚠${NC} $*" | tee -a "$LOG_FILE"
}

# Banner
echo ""
echo "╔════════════════════════════════════════════════════════════════╗"
echo "║          Rust DAQ V4 Hardware Validation Execution            ║"
echo "║                                                                ║"
echo "║  This script will execute 94 hardware test scenarios across   ║"
echo "║  5 V4 actors with comprehensive safety verification.          ║"
echo "║                                                                ║"
echo "║  Estimated Time: 6-7 hours                                     ║"
echo "║  Requires: Laser Safety Officer approval for MaiTai testing   ║"
echo "╚════════════════════════════════════════════════════════════════╝"
echo ""

log "Starting hardware validation execution"
log "User: $CURRENT_USER"
log "Host: $(hostname)"
log "Project: $PROJECT_ROOT"
log "Log Directory: $LOG_DIR"

# Step 1: Verify we're on the correct system
log "Step 1: Verifying system environment"
HOSTNAME=$(hostname)
if [[ ! "$HOSTNAME" == *"maitai"* ]]; then
    log_error "This script must be run on maitai-eos!"
    log_error "Current hostname: $HOSTNAME"
    log_error "Please SSH to maitai-eos first: ssh maitai@maitai-eos"
    exit 1
fi
log_success "Running on correct system: $HOSTNAME"

# Step 2: Verify hardware
log ""
log "Step 2: Hardware Verification"
log "Running hardware verification script..."

if [ -f "$SCRIPT_DIR/hardware_validation/verify_hardware.sh" ]; then
    if bash "$SCRIPT_DIR/hardware_validation/verify_hardware.sh" --quick 2>&1 | tee -a "$LOG_FILE"; then
        log_success "Hardware verification PASSED"
    else
        log_error "Hardware verification FAILED"
        log_error "Please check hardware connections and try again"
        exit 1
    fi
else
    log_warning "Hardware verification script not found, skipping..."
fi

# Step 3: Safety Verification
log ""
log "Step 3: Safety Verification"
log_warning "CRITICAL: Safety verification is MANDATORY before testing"

if [ -f "$SCRIPT_DIR/hardware_validation/safety_check.sh" ]; then
    if bash "$SCRIPT_DIR/hardware_validation/safety_check.sh" 2>&1 | tee -a "$LOG_FILE"; then
        log_success "Safety verification PASSED"
    else
        log_error "Safety verification FAILED"
        log_error "Cannot proceed without passing safety checks"
        exit 1
    fi
else
    log_warning "Safety check script not found"
    read -p "Do you confirm all safety checks have been completed manually? (yes/no): " safety_confirm
    if [ "$safety_confirm" != "yes" ]; then
        log_error "Safety not confirmed. Aborting."
        exit 1
    fi
fi

# Step 4: Build Project
log ""
log "Step 4: Building Project"
cd "$PROJECT_ROOT"

if cargo build --release 2>&1 | tee -a "$LOG_FILE"; then
    log_success "Project build PASSED"
else
    log_error "Project build FAILED"
    exit 1
fi

# Step 5: Run Tests in Safe Order
log ""
log "Step 5: Executing Hardware Tests"
log "Tests will run in order from safest to most critical:"
log "  1. SCPI (17 tests, ~20 min) - LOW RISK"
log "  2. Newport 1830-C (14 tests, ~20 min) - LOW RISK"
log "  3. ESP300 (16 tests, ~45 min) - MEDIUM RISK"
log "  4. PVCAM (28 tests, ~30 min) - MEDIUM RISK"
log "  5. MaiTai (19 tests, ~90 min) - CRITICAL RISK"
log ""

TEST_START_TIME=$(date +%s)

# Phase 1: SCPI (LOW RISK)
log ""
log "════════════════════════════════════════════════════════════════"
log "Phase 1: SCPI Hardware Validation (17 tests, ~20 min)"
log "Risk Level: LOW"
log "════════════════════════════════════════════════════════════════"
PHASE_START=$(date +%s)

if cargo test --test hardware_validation_test -- --ignored scpi 2>&1 | tee "$LOG_DIR/scpi_tests.log"; then
    PHASE_END=$(date +%s)
    PHASE_DURATION=$((PHASE_END - PHASE_START))
    log_success "SCPI tests PASSED (${PHASE_DURATION}s)"
else
    log_error "SCPI tests FAILED"
    log_error "Check logs: $LOG_DIR/scpi_tests.log"
    read -p "Continue with remaining tests? (yes/no): " continue_tests
    if [ "$continue_tests" != "yes" ]; then
        exit 1
    fi
fi

# Phase 2: Newport 1830-C (LOW RISK)
log ""
log "════════════════════════════════════════════════════════════════"
log "Phase 2: Newport 1830-C Hardware Validation (14 tests, ~20 min)"
log "Risk Level: LOW"
log "════════════════════════════════════════════════════════════════"
PHASE_START=$(date +%s)

if cargo test --test hardware_validation_test -- --ignored newport 2>&1 | tee "$LOG_DIR/newport_tests.log"; then
    PHASE_END=$(date +%s)
    PHASE_DURATION=$((PHASE_END - PHASE_START))
    log_success "Newport tests PASSED (${PHASE_DURATION}s)"
else
    log_error "Newport tests FAILED"
    log_error "Check logs: $LOG_DIR/newport_tests.log"
    read -p "Continue with remaining tests? (yes/no): " continue_tests
    if [ "$continue_tests" != "yes" ]; then
        exit 1
    fi
fi

# Safety Check Before Medium Risk Tests
log ""
log_warning "════════════════════════════════════════════════════════════════"
log_warning "SAFETY CHECKPOINT: About to begin MEDIUM RISK tests"
log_warning "  - ESP300: Motion control with physical movement"
log_warning "  - PVCAM: High-power camera operations"
log_warning "════════════════════════════════════════════════════════════════"
read -p "Confirm workspace is clear and emergency stop is accessible (yes/no): " safety_medium
if [ "$safety_medium" != "yes" ]; then
    log_error "Safety not confirmed. Aborting."
    exit 1
fi

# Phase 3: ESP300 (MEDIUM RISK)
log ""
log "════════════════════════════════════════════════════════════════"
log "Phase 3: ESP300 Hardware Validation (16 tests, ~45 min)"
log "Risk Level: MEDIUM"
log "Safety: Emergency stop must be accessible"
log "════════════════════════════════════════════════════════════════"
PHASE_START=$(date +%s)

if cargo test --test hardware_validation_test -- --ignored esp300 2>&1 | tee "$LOG_DIR/esp300_tests.log"; then
    PHASE_END=$(date +%s)
    PHASE_DURATION=$((PHASE_END - PHASE_START))
    log_success "ESP300 tests PASSED (${PHASE_DURATION}s)"
else
    log_error "ESP300 tests FAILED"
    log_error "Check logs: $LOG_DIR/esp300_tests.log"
    read -p "Continue with remaining tests? (yes/no): " continue_tests
    if [ "$continue_tests" != "yes" ]; then
        exit 1
    fi
fi

# Phase 4: PVCAM (MEDIUM RISK)
log ""
log "════════════════════════════════════════════════════════════════"
log "Phase 4: PVCAM Hardware Validation (28 tests, ~30 min)"
log "Risk Level: MEDIUM"
log "════════════════════════════════════════════════════════════════"
PHASE_START=$(date +%s)

if cargo test --test hardware_validation_test -- --ignored pvcam 2>&1 | tee "$LOG_DIR/pvcam_tests.log"; then
    PHASE_END=$(date +%s)
    PHASE_DURATION=$((PHASE_END - PHASE_START))
    log_success "PVCAM tests PASSED (${PHASE_DURATION}s)"
else
    log_error "PVCAM tests FAILED"
    log_error "Check logs: $LOG_DIR/pvcam_tests.log"
    read -p "Continue with remaining tests? (yes/no): " continue_tests
    if [ "$continue_tests" != "yes" ]; then
        exit 1
    fi
fi

# CRITICAL Safety Check Before MaiTai
log ""
log_error "════════════════════════════════════════════════════════════════"
log_error "CRITICAL SAFETY CHECKPOINT: MaiTai Laser Testing"
log_error "════════════════════════════════════════════════════════════════"
log_error "REQUIREMENTS:"
log_error "  1. Laser Safety Officer MUST be present"
log_error "  2. Safety briefing MUST be completed"
log_error "  3. Shutter MUST be verified CLOSED"
log_error "  4. Emergency stop MUST be tested"
log_error "  5. Eye protection MUST be available"
log_error "  6. Warning signs MUST be posted"
log_error ""

read -p "Is Laser Safety Officer present? (yes/no): " lso_present
if [ "$lso_present" != "yes" ]; then
    log_error "Laser Safety Officer not present. Cannot proceed with MaiTai testing."
    log "Skipping MaiTai tests. Run manually later with supervisor."
    SKIP_MAITAI=1
else
    read -p "Has safety briefing been completed? (yes/no): " briefing_done
    read -p "Is shutter verified CLOSED? (yes/no): " shutter_closed
    read -p "Has emergency stop been tested? (yes/no): " estop_tested

    if [ "$briefing_done" != "yes" ] || [ "$shutter_closed" != "yes" ] || [ "$estop_tested" != "yes" ]; then
        log_error "Safety requirements not met. Cannot proceed with MaiTai testing."
        log "Skipping MaiTai tests. Run manually later with supervisor."
        SKIP_MAITAI=1
    else
        SKIP_MAITAI=0
    fi
fi

# Phase 5: MaiTai (CRITICAL RISK)
if [ "${SKIP_MAITAI:-0}" -eq 0 ]; then
    log ""
    log_error "════════════════════════════════════════════════════════════════"
    log_error "Phase 5: MaiTai Laser Hardware Validation (19 tests, ~90 min)"
    log_error "Risk Level: CRITICAL"
    log_error "Supervisor: Laser Safety Officer"
    log_error "════════════════════════════════════════════════════════════════"
    PHASE_START=$(date +%s)

    if cargo test --test hardware_validation_test -- --ignored maitai 2>&1 | tee "$LOG_DIR/maitai_tests.log"; then
        PHASE_END=$(date +%s)
        PHASE_DURATION=$((PHASE_END - PHASE_START))
        log_success "MaiTai tests PASSED (${PHASE_DURATION}s)"
    else
        log_error "MaiTai tests FAILED"
        log_error "Check logs: $LOG_DIR/maitai_tests.log"
    fi

    # Final MaiTai Safety Check
    log_warning ""
    log_warning "FINAL MAITAI SAFETY CHECK"
    read -p "Confirm shutter is CLOSED after testing (yes/no): " final_shutter
    if [ "$final_shutter" != "yes" ]; then
        log_error "CRITICAL: Shutter state not confirmed!"
        log_error "Manually verify and close shutter immediately!"
    fi
else
    log_warning "MaiTai tests skipped - run manually with Laser Safety Officer"
fi

# Step 6: Analyze Results
TEST_END_TIME=$(date +%s)
TOTAL_DURATION=$((TEST_END_TIME - TEST_START_TIME))
TOTAL_HOURS=$((TOTAL_DURATION / 3600))
TOTAL_MINS=$(((TOTAL_DURATION % 3600) / 60))
TOTAL_SECS=$((TOTAL_DURATION % 60))

log ""
log "════════════════════════════════════════════════════════════════"
log "Test Execution Complete"
log "════════════════════════════════════════════════════════════════"
log "Total Duration: ${TOTAL_HOURS}h ${TOTAL_MINS}m ${TOTAL_SECS}s"
log "Log Directory: $LOG_DIR"
log ""

if [ -f "$SCRIPT_DIR/hardware_validation/analyze_results.sh" ]; then
    log "Analyzing results..."
    if bash "$SCRIPT_DIR/hardware_validation/analyze_results.sh" "$LOG_DIR" 2>&1 | tee -a "$LOG_FILE"; then
        log_success "Result analysis complete"
    else
        log_warning "Result analysis encountered issues"
    fi
fi

# Generate Report
log ""
log "Generating test report..."
cd "$PROJECT_ROOT"
if cargo run --example generate_test_report -- --system-id maitai-eos --output "$LOG_DIR" 2>&1 | tee -a "$LOG_FILE"; then
    log_success "Test report generated: $LOG_DIR/report.md"
else
    log_warning "Report generation encountered issues"
fi

# Final Summary
log ""
log "════════════════════════════════════════════════════════════════"
log "                      EXECUTION SUMMARY"
log "════════════════════════════════════════════════════════════════"
log ""
log "Test Logs:"
log "  - SCPI:     $LOG_DIR/scpi_tests.log"
log "  - Newport:  $LOG_DIR/newport_tests.log"
log "  - ESP300:   $LOG_DIR/esp300_tests.log"
log "  - PVCAM:    $LOG_DIR/pvcam_tests.log"
if [ "${SKIP_MAITAI:-0}" -eq 0 ]; then
    log "  - MaiTai:   $LOG_DIR/maitai_tests.log"
else
    log "  - MaiTai:   SKIPPED (run manually with supervisor)"
fi
log ""
log "Reports:"
log "  - Execution Log: $LOG_FILE"
log "  - Test Report:   $LOG_DIR/report.md"
log ""
log_success "Hardware validation execution complete!"
log ""
log "Next steps:"
log "  1. Review test report: cat $LOG_DIR/report.md"
log "  2. Create baseline: ./scripts/hardware_validation/create_baseline.sh"
log "  3. Update beads tracker with results"
if [ "${SKIP_MAITAI:-0}" -eq 1 ]; then
    log_warning "  4. Schedule MaiTai testing with Laser Safety Officer"
fi
log ""
