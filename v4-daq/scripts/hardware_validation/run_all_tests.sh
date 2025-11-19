#!/bin/bash
set -euo pipefail

# Hardware Validation Test Runner
# Executes all hardware tests in safe order with checkpoints
#
# Usage:
#   ./run_all_tests.sh                    # Interactive mode (default)
#   ./run_all_tests.sh --auto            # Non-interactive mode
#   ./run_all_tests.sh --resume <phase>  # Resume from specific phase
#

# Color codes for output
readonly RED='\033[0;31m'
readonly GREEN='\033[0;32m'
readonly YELLOW='\033[1;33m'
readonly BLUE='\033[0;34m'
readonly CYAN='\033[0;36m'
readonly NC='\033[0m' # No Color

# Configuration
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
readonly LOGS_DIR="${PROJECT_ROOT}/hardware_test_logs"
readonly TIMESTAMP="$(date +%Y%m%d_%H%M%S)"
readonly REPORT_FILE="${LOGS_DIR}/test_report_${TIMESTAMP}.txt"
readonly SUMMARY_FILE="${LOGS_DIR}/test_summary_${TIMESTAMP}.json"

# Test phases in execution order
declare -A PHASES=(
    [1]="scpi"
    [2]="newport"
    [3]="esp300"
    [4]="pvcam"
    [5]="maitai"
)

declare -A PHASE_TIMES=(
    [scpi]="20"
    [newport]="20"
    [esp300]="45"
    [pvcam]="30"
    [maitai]="90"
)

declare -A PHASE_RISKS=(
    [scpi]="LOW"
    [newport]="LOW"
    [esp300]="MEDIUM"
    [pvcam]="MEDIUM"
    [maitai]="CRITICAL"
)

# Execution state
INTERACTIVE_MODE=true
RESUME_FROM=""
declare -A PHASE_RESULTS=()
START_TIME=$(date +%s)

# ============================================================================
# Utility Functions
# ============================================================================

print_header() {
    echo -e "\n${CYAN}════════════════════════════════════════════════════════════${NC}"
    echo -e "${CYAN}  $1${NC}"
    echo -e "${CYAN}════════════════════════════════════════════════════════════${NC}\n"
}

print_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[PASS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

print_progress() {
    local current=$1
    local total=$2
    local text=$3
    local percent=$((current * 100 / total))
    echo -e "${BLUE}[${percent}%]${NC} ${text}"
}

log_message() {
    local level=$1
    shift
    local message="$@"
    local timestamp=$(date '+%Y-%m-%d %H:%M:%S')
    echo "[${timestamp}] [${level}] ${message}" >> "${REPORT_FILE}"
}

# Create logs directory
setup_logging() {
    mkdir -p "${LOGS_DIR}"
    touch "${REPORT_FILE}"
    log_message "INFO" "Hardware validation test run started"
    log_message "INFO" "Interactive mode: ${INTERACTIVE_MODE}"
    print_info "Logs will be saved to: ${LOGS_DIR}"
}

# ============================================================================
# Safety Checks
# ============================================================================

run_safety_checks() {
    local phase=$1
    print_header "Safety Checks for ${phase}"

    if [[ "${PHASE_RISKS[${phase}]}" == "CRITICAL" ]]; then
        print_warning "This phase has CRITICAL safety implications"
        print_warning "MaiTai laser safety protocol must be followed"

        if [[ "${INTERACTIVE_MODE}" == true ]]; then
            echo -e "${YELLOW}Safety Requirements:${NC}"
            echo "  1. MaiTai shutter MUST be CLOSED"
            echo "  2. Laser Safety Officer must be present"
            echo "  3. Safety interlocks must be engaged"
            echo "  4. Emergency stop procedure reviewed"
            echo ""
            read -p "I confirm all safety requirements are met (y/n): " -r
            if [[ ! $REPLY =~ ^[Yy]$ ]]; then
                print_error "Safety confirmation refused. Aborting critical phase."
                return 1
            fi
        fi
    elif [[ "${PHASE_RISKS[${phase}]}" == "MEDIUM" ]]; then
        print_warning "This phase has medium risk equipment"
        print_info "Verify soft limits and emergency procedures before proceeding"
    fi

    log_message "INFO" "Safety checks passed for phase: ${phase}"
    print_success "Safety checks passed"
    return 0
}

verify_prerequisites() {
    print_header "Verifying Prerequisites"

    local errors=0

    # Check SSH connectivity
    print_info "Checking SSH connectivity to maitai@maitai-eos..."
    if ssh -o ConnectTimeout=5 -o StrictHostKeyChecking=no maitai@maitai-eos "echo 'SSH test'" &>/dev/null; then
        print_success "SSH connectivity verified"
        log_message "INFO" "SSH connectivity OK"
    else
        print_error "Cannot connect to maitai@maitai-eos"
        log_message "ERROR" "SSH connectivity failed"
        ((errors++))
    fi

    # Check Rust environment
    print_info "Checking Rust environment..."
    if command -v cargo &>/dev/null; then
        print_success "Cargo found"
        log_message "INFO" "Cargo available"
    else
        print_error "Cargo not found"
        log_message "ERROR" "Cargo not available"
        ((errors++))
    fi

    # Check test binaries exist
    print_info "Checking test binaries..."
    if [[ -f "${PROJECT_ROOT}/target/release/elliptec_hardware_test" ]]; then
        print_success "Elliptec test binary found"
        log_message "INFO" "Elliptec test binary available"
    else
        print_warning "Elliptec test binary not found - will attempt to build"
        log_message "WARN" "Elliptec test binary missing"
    fi

    if (( errors > 0 )); then
        print_error "Prerequisites check failed with ${errors} error(s)"
        log_message "ERROR" "Prerequisites check failed"
        return 1
    fi

    print_success "All prerequisites verified"
    return 0
}

# ============================================================================
# Test Execution
# ============================================================================

run_scpi_tests() {
    print_header "Phase 1: SCPI Tests (LOW RISK)"
    print_progress 1 5 "Running SCPI device tests"

    run_safety_checks "scpi" || return 1

    local test_log="${LOGS_DIR}/scpi_${TIMESTAMP}.log"
    local start_time=$(date +%s)

    print_info "SCPI test phase started"
    log_message "INFO" "Starting SCPI test phase"

    # Run SCPI device tests
    if cargo test --release --test "*scpi*" -- --nocapture > "${test_log}" 2>&1; then
        print_success "SCPI tests passed"
        PHASE_RESULTS[scpi]="PASS"
        log_message "INFO" "SCPI tests completed successfully"
    else
        print_error "SCPI tests failed"
        PHASE_RESULTS[scpi]="FAIL"
        log_message "ERROR" "SCPI tests failed"
        tail -20 "${test_log}"
    fi

    local end_time=$(date +%s)
    local duration=$((end_time - start_time))
    print_info "SCPI phase completed in ${duration}s"
    log_message "INFO" "SCPI phase duration: ${duration}s"
}

run_newport_tests() {
    print_header "Phase 2: Newport 1830-C Tests (LOW RISK)"
    print_progress 2 5 "Running Newport motion control tests"

    run_safety_checks "newport" || return 1

    local test_log="${LOGS_DIR}/newport_${TIMESTAMP}.log"
    local start_time=$(date +%s)

    print_info "Newport test phase started"
    log_message "INFO" "Starting Newport test phase"

    # Run Newport device tests
    if cargo test --release --test "*newport*" -- --nocapture > "${test_log}" 2>&1; then
        print_success "Newport tests passed"
        PHASE_RESULTS[newport]="PASS"
        log_message "INFO" "Newport tests completed successfully"
    else
        print_error "Newport tests failed"
        PHASE_RESULTS[newport]="FAIL"
        log_message "ERROR" "Newport tests failed"
        tail -20 "${test_log}"
    fi

    local end_time=$(date +%s)
    local duration=$((end_time - start_time))
    print_info "Newport phase completed in ${duration}s"
    log_message "INFO" "Newport phase duration: ${duration}s"
}

run_esp300_tests() {
    print_header "Phase 3: ESP300 Motor Controller Tests (MEDIUM RISK)"
    print_progress 3 5 "Running ESP300 tests"

    run_safety_checks "esp300" || return 1

    local test_log="${LOGS_DIR}/esp300_${TIMESTAMP}.log"
    local start_time=$(date +%s)

    print_warning "ESP300 tests may cause motor movement"
    print_info "Ensure lab area is clear before proceeding"

    if [[ "${INTERACTIVE_MODE}" == true ]]; then
        read -p "Press Enter to start ESP300 tests, or Ctrl+C to abort: " -r
    fi

    log_message "INFO" "Starting ESP300 test phase"

    # Run ESP300 device tests
    if cargo test --release --test "*esp300*" -- --nocapture > "${test_log}" 2>&1; then
        print_success "ESP300 tests passed"
        PHASE_RESULTS[esp300]="PASS"
        log_message "INFO" "ESP300 tests completed successfully"
    else
        print_error "ESP300 tests failed"
        PHASE_RESULTS[esp300]="FAIL"
        log_message "ERROR" "ESP300 tests failed"
        tail -20 "${test_log}"
    fi

    local end_time=$(date +%s)
    local duration=$((end_time - start_time))
    print_info "ESP300 phase completed in ${duration}s"
    log_message "INFO" "ESP300 phase duration: ${duration}s"
}

run_pvcam_tests() {
    print_header "Phase 4: PVCAM Camera Tests (MEDIUM RISK)"
    print_progress 4 5 "Running PVCAM camera tests"

    run_safety_checks "pvcam" || return 1

    local test_log="${LOGS_DIR}/pvcam_${TIMESTAMP}.log"
    local start_time=$(date +%s)

    print_warning "PVCAM tests will trigger camera acquisition"
    print_info "Ensure optical path is safe before proceeding"

    if [[ "${INTERACTIVE_MODE}" == true ]]; then
        read -p "Press Enter to start PVCAM tests, or Ctrl+C to abort: " -r
    fi

    log_message "INFO" "Starting PVCAM test phase"

    # Run PVCAM device tests
    if cargo test --release --test "*pvcam*" -- --nocapture > "${test_log}" 2>&1; then
        print_success "PVCAM tests passed"
        PHASE_RESULTS[pvcam]="PASS"
        log_message "INFO" "PVCAM tests completed successfully"
    else
        print_error "PVCAM tests failed"
        PHASE_RESULTS[pvcam]="FAIL"
        log_message "ERROR" "PVCAM tests failed"
        tail -20 "${test_log}"
    fi

    local end_time=$(date +%s)
    local duration=$((end_time - start_time))
    print_info "PVCAM phase completed in ${duration}s"
    log_message "INFO" "PVCAM phase duration: ${duration}s"
}

run_maitai_tests() {
    print_header "Phase 5: MaiTai Laser Tests (CRITICAL RISK)"
    print_progress 5 5 "Running MaiTai laser tests"

    run_safety_checks "maitai" || return 1

    local test_log="${LOGS_DIR}/maitai_${TIMESTAMP}.log"
    local start_time=$(date +%s)

    print_error "CRITICAL PHASE: MaiTai Laser System"
    echo -e "${RED}Safety Protocol:${NC}"
    echo "  1. Laser Safety Officer MUST be present"
    echo "  2. All safety interlocks ENGAGED"
    echo "  3. MaiTai shutter MUST be CLOSED"
    echo "  4. Lab access restricted"
    echo "  5. Emergency stop accessible"
    echo ""

    if [[ "${INTERACTIVE_MODE}" == true ]]; then
        read -p "Laser Safety Officer approval (name): " -r officer_name
        if [[ -z "${officer_name}" ]]; then
            print_error "Laser Safety Officer approval required"
            log_message "ERROR" "MaiTai tests aborted - no LSO approval"
            PHASE_RESULTS[maitai]="ABORTED"
            return 1
        fi
        log_message "INFO" "MaiTai tests approved by LSO: ${officer_name}"

        read -p "Confirm MaiTai shutter is CLOSED (yes/no): " -r shutter_confirm
        if [[ "${shutter_confirm}" != "yes" ]]; then
            print_error "Shutter confirmation failed"
            log_message "ERROR" "MaiTai tests aborted - shutter not confirmed closed"
            PHASE_RESULTS[maitai]="ABORTED"
            return 1
        fi
    fi

    log_message "INFO" "Starting MaiTai test phase"
    print_warning "MaiTai tests in progress - DO NOT INTERRUPT"

    # Run MaiTai device tests
    if timeout 120 cargo test --release --test "*maitai*" -- --nocapture > "${test_log}" 2>&1; then
        print_success "MaiTai tests passed"
        PHASE_RESULTS[maitai]="PASS"
        log_message "INFO" "MaiTai tests completed successfully"
    else
        local exit_code=$?
        if (( exit_code == 124 )); then
            print_error "MaiTai tests timed out"
            PHASE_RESULTS[maitai]="TIMEOUT"
            log_message "ERROR" "MaiTai tests timed out"
        else
            print_error "MaiTai tests failed"
            PHASE_RESULTS[maitai]="FAIL"
            log_message "ERROR" "MaiTai tests failed"
        fi
        tail -20 "${test_log}"
    fi

    local end_time=$(date +%s)
    local duration=$((end_time - start_time))
    print_info "MaiTai phase completed in ${duration}s"
    log_message "INFO" "MaiTai phase duration: ${duration}s"
}

# ============================================================================
# Reporting
# ============================================================================

generate_json_summary() {
    local total_time=$(($(date +%s) - START_TIME))

    cat > "${SUMMARY_FILE}" << 'EOF'
{
  "test_run": {
    "timestamp": "TIMESTAMP_PLACEHOLDER",
    "duration_seconds": DURATION_PLACEHOLDER,
    "interactive_mode": INTERACTIVE_PLACEHOLDER,
    "results": {
      "scpi": "SCPI_RESULT",
      "newport": "NEWPORT_RESULT",
      "esp300": "ESP300_RESULT",
      "pvcam": "PVCAM_RESULT",
      "maitai": "MAITAI_RESULT"
    },
    "summary": {
      "total_phases": 5,
      "passed": PASSED_COUNT,
      "failed": FAILED_COUNT,
      "aborted": ABORTED_COUNT,
      "success_rate": "SUCCESS_RATE%"
    }
  }
}
EOF

    # Replace placeholders
    sed -i.bak "s/TIMESTAMP_PLACEHOLDER/${TIMESTAMP}/g" "${SUMMARY_FILE}"
    sed -i.bak "s/DURATION_PLACEHOLDER/${total_time}/g" "${SUMMARY_FILE}"
    sed -i.bak "s/INTERACTIVE_PLACEHOLDER/${INTERACTIVE_MODE}/g" "${SUMMARY_FILE}"
    sed -i.bak "s/SCPI_RESULT/${PHASE_RESULTS[scpi]:-SKIPPED}/g" "${SUMMARY_FILE}"
    sed -i.bak "s/NEWPORT_RESULT/${PHASE_RESULTS[newport]:-SKIPPED}/g" "${SUMMARY_FILE}"
    sed -i.bak "s/ESP300_RESULT/${PHASE_RESULTS[esp300]:-SKIPPED}/g" "${SUMMARY_FILE}"
    sed -i.bak "s/PVCAM_RESULT/${PHASE_RESULTS[pvcam]:-SKIPPED}/g" "${SUMMARY_FILE}"
    sed -i.bak "s/MAITAI_RESULT/${PHASE_RESULTS[maitai]:-SKIPPED}/g" "${SUMMARY_FILE}"

    local passed=0 failed=0 aborted=0
    for phase in "${!PHASE_RESULTS[@]}"; do
        case "${PHASE_RESULTS[${phase}]}" in
            PASS) ((passed++)) ;;
            FAIL) ((failed++)) ;;
            ABORTED) ((aborted++)) ;;
        esac
    done

    local total=$((passed + failed + aborted))
    local success_rate=0
    if (( total > 0 )); then
        success_rate=$((passed * 100 / total))
    fi

    sed -i.bak "s/PASSED_COUNT/${passed}/g" "${SUMMARY_FILE}"
    sed -i.bak "s/FAILED_COUNT/${failed}/g" "${SUMMARY_FILE}"
    sed -i.bak "s/ABORTED_COUNT/${aborted}/g" "${SUMMARY_FILE}"
    sed -i.bak "s/SUCCESS_RATE/${success_rate}/g" "${SUMMARY_FILE}"

    rm -f "${SUMMARY_FILE}.bak"
}

print_test_report() {
    print_header "Test Execution Report"

    local total_time=$(($(date +%s) - START_TIME))
    local minutes=$((total_time / 60))
    local seconds=$((total_time % 60))

    echo "Test Run Summary:"
    echo "  Timestamp: ${TIMESTAMP}"
    echo "  Duration: ${minutes}m ${seconds}s"
    echo "  Interactive Mode: ${INTERACTIVE_MODE}"
    echo ""

    echo "Phase Results:"
    local passed=0 failed=0 aborted=0 skipped=0
    for phase in scpi newport esp300 pvcam maitai; do
        local result="${PHASE_RESULTS[${phase}]:-SKIPPED}"
        case "${result}" in
            PASS)
                echo -e "  ${GREEN}✓${NC} ${phase}: ${result}"
                ((passed++))
                ;;
            FAIL)
                echo -e "  ${RED}✗${NC} ${phase}: ${result}"
                ((failed++))
                ;;
            ABORTED)
                echo -e "  ${YELLOW}⊘${NC} ${phase}: ${result}"
                ((aborted++))
                ;;
            SKIPPED)
                echo -e "  ${CYAN}-${NC} ${phase}: ${result}"
                ((skipped++))
                ;;
        esac
    done
    echo ""

    local total=$((passed + failed + aborted))
    local success_rate=0
    if (( total > 0 )); then
        success_rate=$((passed * 100 / total))
    fi

    echo "Summary:"
    echo "  Total Phases: 5"
    echo "  Passed: ${passed}"
    echo "  Failed: ${failed}"
    echo "  Aborted: ${aborted}"
    echo "  Skipped: ${skipped}"
    echo "  Success Rate: ${success_rate}%"
    echo ""
    echo "Logs: ${LOGS_DIR}"
    echo "Report: ${REPORT_FILE}"
    echo "Summary JSON: ${SUMMARY_FILE}"

    log_message "INFO" "Test run completed successfully"
}

# ============================================================================
# Main Execution
# ============================================================================

main() {
    # Parse command line arguments
    while [[ $# -gt 0 ]]; do
        case $1 in
            --auto)
                INTERACTIVE_MODE=false
                shift
                ;;
            --resume)
                RESUME_FROM="$2"
                shift 2
                ;;
            -h|--help)
                echo "Usage: $0 [OPTIONS]"
                echo ""
                echo "Options:"
                echo "  --auto              Non-interactive mode (no prompts)"
                echo "  --resume PHASE      Resume from specific phase (scpi/newport/esp300/pvcam/maitai)"
                echo "  -h, --help          Show this help message"
                exit 0
                ;;
            *)
                print_error "Unknown option: $1"
                exit 1
                ;;
        esac
    done

    setup_logging
    print_header "Hardware Validation Test Suite"

    print_info "Test execution order:"
    echo "  1. SCPI (LOW RISK) - 20 min"
    echo "  2. Newport 1830-C (LOW RISK) - 20 min"
    echo "  3. SAFETY CHECKPOINT"
    echo "  4. ESP300 (MEDIUM RISK) - 45 min"
    echo "  5. PVCAM (MEDIUM RISK) - 30 min"
    echo "  6. CRITICAL SAFETY CHECKPOINT"
    echo "  7. MaiTai (CRITICAL RISK) - 90 min"
    echo ""

    # Verify prerequisites
    if ! verify_prerequisites; then
        print_error "Prerequisites verification failed"
        log_message "ERROR" "Test suite aborted - prerequisites not met"
        exit 1
    fi

    print_info "Prerequisite verification complete - proceeding with tests"
    echo ""

    # Execute test phases
    local start_phase=1
    if [[ -n "${RESUME_FROM}" ]]; then
        case "${RESUME_FROM}" in
            scpi) start_phase=1 ;;
            newport) start_phase=2 ;;
            esp300) start_phase=3 ;;
            pvcam) start_phase=4 ;;
            maitai) start_phase=5 ;;
            *)
                print_error "Unknown phase: ${RESUME_FROM}"
                exit 1
                ;;
        esac
        print_info "Resuming from phase ${start_phase}: ${RESUME_FROM}"
    fi

    # Run phases
    for phase_num in {1..5}; do
        if (( phase_num < start_phase )); then
            continue
        fi

        local phase="${PHASES[${phase_num}]}"

        case "${phase}" in
            scpi)
                run_scpi_tests || true
                ;;
            newport)
                run_newport_tests || true
                ;;
            esp300)
                run_esp300_tests || true
                ;;
            pvcam)
                run_pvcam_tests || true
                ;;
            maitai)
                run_maitai_tests || true
                ;;
        esac
    done

    # Generate reports
    generate_json_summary
    print_test_report

    # Exit with appropriate code
    local failed=0
    for phase in "${!PHASE_RESULTS[@]}"; do
        if [[ "${PHASE_RESULTS[${phase}]}" == "FAIL" ]]; then
            ((failed++))
        fi
    done

    if (( failed > 0 )); then
        exit 1
    fi
    exit 0
}

# Run main function
main "$@"
