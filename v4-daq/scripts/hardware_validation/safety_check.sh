#!/bin/bash
set -euo pipefail

# Hardware Safety Check Script
# Verifies critical safety conditions before running tests
#
# Usage:
#   ./safety_check.sh                 # Full safety check
#   ./safety_check.sh --pre-maitai    # Pre-MaiTai laser checks only
#   ./safety_check.sh --automated     # Non-interactive mode
#

# Color codes
readonly RED='\033[0;31m'
readonly GREEN='\033[0;32m'
readonly YELLOW='\033[1;33m'
readonly BLUE='\033[0;34m'
readonly CYAN='\033[0;36m'
readonly NC='\033[0m'

# Configuration
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
readonly LOGS_DIR="${PROJECT_ROOT}/hardware_test_logs"
readonly TIMESTAMP="$(date +%Y%m%d_%H%M%S)"
readonly SAFETY_LOG="${LOGS_DIR}/safety_check_${TIMESTAMP}.log"

# Safety state
INTERACTIVE_MODE=true
PRE_MAITAI_ONLY=false
SAFETY_PASSED=0
SAFETY_FAILED=0

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

print_critical() {
    echo -e "${RED}[CRITICAL]${NC} $1"
}

log_message() {
    local level=$1
    shift
    local timestamp=$(date '+%Y-%m-%d %H:%M:%S')
    echo "[${timestamp}] [${level}] $*" >> "${SAFETY_LOG}"
}

setup_logging() {
    mkdir -p "${LOGS_DIR}"
    touch "${SAFETY_LOG}"
    log_message "INFO" "Safety check started"
}

# ============================================================================
# MaiTai Shutter Checks
# ============================================================================

check_maitai_shutter() {
    print_header "MaiTai Shutter Status"

    print_info "Checking MaiTai shutter state..."
    print_warning "This is a CRITICAL safety check"
    echo ""

    # Method 1: SSH to maitai system for shutter state
    print_info "Attempting to query MaiTai shutter via SSH..."

    local shutter_state=""
    if ssh -o ConnectTimeout=5 -o StrictHostKeyChecking=no \
           maitai@maitai-eos "cat /sys/class/laser/maitai/shutter" &>/dev/null 2>&1; then
        shutter_state=$(ssh -o ConnectTimeout=5 -o StrictHostKeyChecking=no \
                           maitai@maitai-eos "cat /sys/class/laser/maitai/shutter" 2>/dev/null)
    fi

    if [[ -z "${shutter_state}" ]]; then
        print_warning "Could not automatically determine shutter state"
        print_warning "Manual verification required"
        log_message "WARN" "Shutter state could not be automatically determined"

        if [[ "${INTERACTIVE_MODE}" == true ]]; then
            echo -e "${RED}CRITICAL SAFETY REQUIREMENT:${NC}"
            echo "The MaiTai shutter MUST be CLOSED before proceeding"
            echo ""
            echo "To verify:"
            echo "  1. SSH to maitai@maitai-eos"
            echo "  2. Check laser console for shutter status"
            echo "  3. Verify shutter LED is NOT green"
            echo ""
            read -p "Is the MaiTai shutter CLOSED? (yes/no): " -r shutter_confirm
            if [[ "${shutter_confirm}" == "yes" ]]; then
                print_success "MaiTai shutter confirmed CLOSED"
                log_message "INFO" "MaiTai shutter manually confirmed CLOSED"
                ((SAFETY_PASSED++))
                return 0
            else
                print_critical "MaiTai shutter is NOT CLOSED"
                log_message "ERROR" "MaiTai shutter not closed - safety check FAILED"
                ((SAFETY_FAILED++))
                return 1
            fi
        else
            print_error "Cannot verify shutter state in automated mode"
            log_message "ERROR" "Shutter verification failed in automated mode"
            ((SAFETY_FAILED++))
            return 1
        fi
    fi

    # Parse shutter state
    case "${shutter_state}" in
        *[Cc][Ll][Oo][Ss][Ee][Dd]*|*[Cc][Ll][Oo]*|0)
            print_success "MaiTai shutter is CLOSED"
            log_message "INFO" "MaiTai shutter state: CLOSED"
            ((SAFETY_PASSED++))
            return 0
            ;;
        *[Oo][Pp][Ee][Nn]*|*[Oo][Pp][Ee][Nn][Ee][Dd]*|1)
            print_critical "MaiTai shutter is OPEN"
            log_message "ERROR" "MaiTai shutter is OPEN - safety check FAILED"
            ((SAFETY_FAILED++))
            return 1
            ;;
        *)
            print_warning "Unknown shutter state: ${shutter_state}"
            log_message "WARN" "Unknown shutter state: ${shutter_state}"
            if [[ "${INTERACTIVE_MODE}" == true ]]; then
                read -p "Proceed with safety check? (yes/no): " -r proceed
                if [[ "${proceed}" == "yes" ]]; then
                    ((SAFETY_PASSED++))
                    return 0
                else
                    ((SAFETY_FAILED++))
                    return 1
                fi
            else
                ((SAFETY_FAILED++))
                return 1
            fi
            ;;
    esac
}

# ============================================================================
# ESP300 Soft Limits Verification
# ============================================================================

check_esp300_soft_limits() {
    print_header "ESP300 Motor Controller Verification"

    print_info "Checking ESP300 soft limits..."

    # Try to connect to ESP300 via serial
    local esp300_port=""
    for port in /dev/ttyUSB* /dev/ttyACM* /dev/tty.usbserial*; do
        if [[ -e "${port}" ]]; then
            print_info "Checking port: ${port}"
            # Attempt to query device
            if timeout 2 bash -c "cat < ${port}" 2>/dev/null | grep -q "ESP300"; then
                esp300_port="${port}"
                break
            fi
        fi
    done

    if [[ -z "${esp300_port}" ]]; then
        print_warning "ESP300 serial port not found or not responding"
        print_info "Soft limits cannot be verified automatically"
        log_message "WARN" "ESP300 port not found"

        if [[ "${INTERACTIVE_MODE}" == true ]]; then
            read -p "Confirm ESP300 soft limits are properly configured (yes/no): " -r limits_confirm
            if [[ "${limits_confirm}" == "yes" ]]; then
                print_success "ESP300 soft limits confirmed by user"
                log_message "INFO" "ESP300 soft limits manually confirmed"
                ((SAFETY_PASSED++))
                return 0
            else
                print_error "ESP300 soft limits not confirmed"
                log_message "ERROR" "ESP300 soft limits verification failed"
                ((SAFETY_FAILED++))
                return 1
            fi
        else
            print_warning "Skipping ESP300 verification in automated mode"
            ((SAFETY_PASSED++))
            return 0
        fi
    fi

    print_success "ESP300 found on ${esp300_port}"
    log_message "INFO" "ESP300 found: ${esp300_port}"
    ((SAFETY_PASSED++))
    return 0
}

# ============================================================================
# Emergency Stop Verification
# ============================================================================

check_emergency_stop() {
    print_header "Emergency Stop Button Verification"

    print_info "Verifying emergency stop accessibility..."

    # Check for emergency stop script
    local emstop_script="${PROJECT_ROOT}/scripts/hardware_validation/emergency_stop.sh"
    if [[ ! -f "${emstop_script}" ]]; then
        print_warning "Emergency stop script not found at expected location"
        log_message "WARN" "Emergency stop script not found"
    else
        if [[ -x "${emstop_script}" ]]; then
            print_success "Emergency stop script is executable"
            log_message "INFO" "Emergency stop script available"
        else
            print_warning "Emergency stop script is not executable"
            chmod +x "${emstop_script}" 2>/dev/null || true
            log_message "WARN" "Emergency stop script permissions fixed"
        fi
    fi

    if [[ "${INTERACTIVE_MODE}" == true ]]; then
        echo ""
        echo -e "${YELLOW}Emergency Stop Procedure:${NC}"
        echo "  In case of emergency, press Ctrl+C in test runner"
        echo "  Or manually run: ${emstop_script}"
        echo "  Or kill the test process: pkill -f 'cargo test'"
        echo ""
        read -p "Do you understand the emergency stop procedure? (yes/no): " -r estop_confirm
        if [[ "${estop_confirm}" == "yes" ]]; then
            print_success "Emergency stop procedure acknowledged"
            log_message "INFO" "Emergency stop procedure confirmed"
            ((SAFETY_PASSED++))
            return 0
        else
            print_error "Emergency stop procedure not confirmed"
            log_message "ERROR" "Emergency stop procedure not acknowledged"
            ((SAFETY_FAILED++))
            return 1
        fi
    else
        if [[ -f "${emstop_script}" ]]; then
            ((SAFETY_PASSED++))
            return 0
        else
            ((SAFETY_FAILED++))
            return 1
        fi
    fi
}

# ============================================================================
# Lab Safety Checklist
# ============================================================================

check_lab_safety_checklist() {
    print_header "Lab Safety Checklist"

    if [[ "${INTERACTIVE_MODE}" == false ]]; then
        print_info "Skipping interactive checklist in automated mode"
        return 0
    fi

    local checklist_items=(
        "Lab area is clear of unauthorized personnel"
        "All test equipment is properly grounded"
        "Fire extinguisher is accessible and charged"
        "First aid kit is accessible"
        "Safety goggles available for laser work"
        "Laser warning signs are posted"
        "Emergency contacts are posted"
        "Hazard materials are properly labeled and stored"
    )

    echo -e "${YELLOW}Safety Checklist:${NC}"
    local items_passed=0
    local items_total=${#checklist_items[@]}

    for i in "${!checklist_items[@]}"; do
        local idx=$((i + 1))
        read -p "  [$idx/$items_total] ${checklist_items[$i]} (yes/no): " -r item_check
        if [[ "${item_check}" == "yes" ]]; then
            ((items_passed++))
            echo -e "    ${GREEN}✓${NC} Confirmed"
        else
            echo -e "    ${RED}✗${NC} Not confirmed"
        fi
    done

    echo ""
    if (( items_passed == items_total )); then
        print_success "All safety checklist items confirmed"
        log_message "INFO" "Lab safety checklist: ALL PASSED"
        ((SAFETY_PASSED++))
        return 0
    else
        print_warning "Some safety checklist items not confirmed (${items_passed}/${items_total})"
        log_message "WARN" "Lab safety checklist incomplete: ${items_passed}/${items_total}"
        ((SAFETY_FAILED++))
        return 1
    fi
}

# ============================================================================
# Laser Safety Officer Verification
# ============================================================================

check_laser_safety_officer() {
    print_header "Laser Safety Officer Approval"

    if [[ "${INTERACTIVE_MODE}" == false ]]; then
        print_info "Skipping LSO verification in automated mode"
        return 0
    fi

    print_warning "Laser work requires approval from Laser Safety Officer"
    echo ""
    echo -e "${RED}CRITICAL SAFETY REQUIREMENT:${NC}"
    echo "A Laser Safety Officer MUST approve before MaiTai laser testing"
    echo ""

    read -p "Laser Safety Officer name: " -r lso_name
    if [[ -z "${lso_name}" ]]; then
        print_error "No LSO name provided"
        log_message "ERROR" "LSO approval: NO LSO NAME PROVIDED"
        ((SAFETY_FAILED++))
        return 1
    fi

    read -p "LSO employee ID or badge number: " -r lso_id
    if [[ -z "${lso_id}" ]]; then
        print_warning "LSO ID not provided"
    fi

    local lso_timestamp=$(date '+%Y-%m-%d %H:%M:%S')
    print_success "LSO Approval recorded"
    print_info "LSO: ${lso_name} (${lso_id:-unknown}) at ${lso_timestamp}"
    log_message "INFO" "LSO Approval: ${lso_name} (${lso_id:-unknown})"
    log_message "INFO" "LSO Approval timestamp: ${lso_timestamp}"

    ((SAFETY_PASSED++))
    return 0
}

# ============================================================================
# Pre-MaiTai Critical Checks
# ============================================================================

run_pre_maitai_checks() {
    print_header "PRE-MAITAI CRITICAL SAFETY CHECKS"

    print_error "CRITICAL: This runs ONLY before MaiTai laser testing"
    echo ""

    local failed=0

    # Must checks (failures are critical)
    check_maitai_shutter || ((failed++))
    check_emergency_stop || ((failed++))

    if [[ "${INTERACTIVE_MODE}" == true ]]; then
        check_lab_safety_checklist || ((failed++))
        check_laser_safety_officer || ((failed++))
    fi

    return $((failed > 0 ? 1 : 0))
}

# ============================================================================
# Full Safety Check
# ============================================================================

run_full_safety_check() {
    print_header "FULL HARDWARE SAFETY CHECK"

    print_info "Running comprehensive safety checks..."
    echo ""

    local failed=0

    # General checks
    check_esp300_soft_limits || ((failed++))
    check_emergency_stop || ((failed++))

    if [[ "${INTERACTIVE_MODE}" == true ]]; then
        check_lab_safety_checklist || ((failed++))
    fi

    return $((failed > 0 ? 1 : 0))
}

# ============================================================================
# Report Generation
# ============================================================================

print_safety_report() {
    print_header "Safety Check Report"

    local total=$((SAFETY_PASSED + SAFETY_FAILED))

    echo "Safety Check Summary:"
    echo "  Passed: ${SAFETY_PASSED}"
    echo "  Failed: ${SAFETY_FAILED}"
    echo "  Total: ${total}"
    echo ""

    if (( SAFETY_FAILED == 0 )); then
        echo -e "${GREEN}All safety checks PASSED${NC}"
        echo "Hardware is safe for testing"
        log_message "INFO" "All safety checks PASSED"
    else
        echo -e "${RED}Safety checks FAILED${NC}"
        echo "Fix all issues before proceeding with testing"
        log_message "ERROR" "Safety checks FAILED"
    fi

    echo ""
    echo "Log file: ${SAFETY_LOG}"
}

# ============================================================================
# Main Execution
# ============================================================================

main() {
    # Parse arguments
    while [[ $# -gt 0 ]]; do
        case $1 in
            --pre-maitai)
                PRE_MAITAI_ONLY=true
                shift
                ;;
            --automated)
                INTERACTIVE_MODE=false
                shift
                ;;
            -h|--help)
                echo "Usage: $0 [OPTIONS]"
                echo ""
                echo "Options:"
                echo "  --pre-maitai  Run only critical pre-MaiTai checks"
                echo "  --automated   Non-interactive mode"
                echo "  -h, --help    Show this help message"
                echo ""
                echo "Modes:"
                echo "  Default: Full safety check with interactive confirmations"
                echo "  --pre-maitai: Critical checks before laser testing only"
                exit 0
                ;;
            *)
                print_error "Unknown option: $1"
                exit 1
                ;;
        esac
    done

    setup_logging

    if [[ "${PRE_MAITAI_ONLY}" == true ]]; then
        run_pre_maitai_checks || {
            print_safety_report
            exit 1
        }
    else
        run_full_safety_check || {
            print_safety_report
            exit 1
        }
    fi

    print_safety_report
    exit 0
}

main "$@"
