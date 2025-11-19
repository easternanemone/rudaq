#!/bin/bash
set -euo pipefail

# Hardware Verification Script
# Verifies all hardware prerequisites before running tests
#
# Usage:
#   ./verify_hardware.sh              # Full verification
#   ./verify_hardware.sh --quick      # Quick connectivity check only
#   ./verify_hardware.sh --verbose    # Detailed output

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
readonly VERIFY_REPORT="${LOGS_DIR}/verify_report_${TIMESTAMP}.txt"

# Verification state
VERBOSE_MODE=false
QUICK_MODE=false
declare -a ERRORS=()
declare -a WARNINGS=()

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

verbose_print() {
    if [[ "${VERBOSE_MODE}" == true ]]; then
        echo -e "${BLUE}[VERBOSE]${NC} $1"
    fi
}

add_error() {
    ERRORS+=("$1")
    print_error "$1"
}

add_warning() {
    WARNINGS+=("$1")
    print_warning "$1"
}

log_message() {
    local level=$1
    shift
    local timestamp=$(date '+%Y-%m-%d %H:%M:%S')
    echo "[${timestamp}] [${level}] $*" >> "${VERIFY_REPORT}"
}

setup_logging() {
    mkdir -p "${LOGS_DIR}"
    touch "${VERIFY_REPORT}"
    log_message "INFO" "Hardware verification started"
}

# ============================================================================
# SSH Connectivity Checks
# ============================================================================

check_ssh_connectivity() {
    print_header "SSH Connectivity Verification"

    local ssh_host="maitai@maitai-eos"
    print_info "Checking SSH connectivity to ${ssh_host}..."

    if timeout 10 ssh -o ConnectTimeout=5 \
                      -o StrictHostKeyChecking=no \
                      -o UserKnownHostsFile=/dev/null \
                      "${ssh_host}" "echo 'SSH test'" &>/dev/null; then
        print_success "SSH connection successful"
        log_message "INFO" "SSH connectivity OK"
        return 0
    else
        add_error "Cannot connect to ${ssh_host}"
        log_message "ERROR" "SSH connectivity failed"
        return 1
    fi
}

check_ssh_key_setup() {
    print_info "Checking SSH key setup..."

    if [[ -f ~/.ssh/id_rsa ]] || [[ -f ~/.ssh/id_ed25519 ]]; then
        print_success "SSH key found"
        log_message "INFO" "SSH key exists"

        # Check key permissions
        local key_file=""
        if [[ -f ~/.ssh/id_rsa ]]; then
            key_file=~/.ssh/id_rsa
        else
            key_file=~/.ssh/id_ed25519
        fi

        local permissions=$(stat -f "%A" "${key_file}" 2>/dev/null || stat -c "%a" "${key_file}" 2>/dev/null)
        if [[ "${permissions}" == "600" ]] || [[ "${permissions}" == "rw-------" ]]; then
            print_success "SSH key permissions correct"
            log_message "INFO" "SSH key permissions OK"
            return 0
        else
            add_warning "SSH key permissions may be incorrect (${permissions})"
            log_message "WARN" "SSH key permissions: ${permissions}"
            return 0
        fi
    else
        add_error "SSH key not found (~/.ssh/id_rsa or ~/.ssh/id_ed25519)"
        log_message "ERROR" "SSH key not found"
        return 1
    fi
}

# ============================================================================
# VISA Resource Checks
# ============================================================================

check_visa_resources() {
    print_header "VISA Resource Verification"

    print_info "Checking for VISA libraries..."

    # Check common VISA library locations
    local visa_found=false
    for lib_path in /usr/local/lib /usr/lib /opt/agilent/visa/lib /usr/local/visa/lib; do
        if [[ -d "${lib_path}" ]]; then
            if ls "${lib_path}"/libvisa* 2>/dev/null | grep -q .; then
                print_success "VISA library found in ${lib_path}"
                log_message "INFO" "VISA library found: ${lib_path}"
                visa_found=true
                break
            fi
        fi
    done

    if [[ "${visa_found}" == false ]]; then
        add_warning "VISA libraries not found in standard locations"
        add_warning "VISA may need to be installed or NI-VISA configured"
        log_message "WARN" "VISA libraries not found"
    fi

    # Check LD_LIBRARY_PATH
    if [[ -n "${LD_LIBRARY_PATH:-}" ]]; then
        verbose_print "LD_LIBRARY_PATH: ${LD_LIBRARY_PATH}"
        log_message "INFO" "LD_LIBRARY_PATH set"
    else
        add_warning "LD_LIBRARY_PATH not set - VISA may not be found at runtime"
        log_message "WARN" "LD_LIBRARY_PATH not set"
    fi
}

check_visa_devices() {
    print_info "Checking for VISA devices..."

    # Try to list VISA resources (requires PyVISA or visa command-line tool)
    if command -v pyvisa-shell &>/dev/null; then
        verbose_print "PyVISA shell available"
        if pyvisa-shell list 2>/dev/null | grep -q "TCPIP"; then
            print_success "VISA devices detected"
            log_message "INFO" "VISA devices available"
        else
            add_warning "No VISA devices detected"
            log_message "WARN" "No VISA devices found"
        fi
    else
        verbose_print "PyVISA not installed - skipping device enumeration"
        log_message "INFO" "PyVISA not available for device check"
    fi
}

# ============================================================================
# Serial Port Checks
# ============================================================================

check_serial_ports() {
    print_header "Serial Port Verification"

    print_info "Scanning for serial ports..."

    local serial_ports=()

    # macOS
    if [[ -d /dev/tty.* ]]; then
        while IFS= read -r port; do
            serial_ports+=("${port}")
        done < <(ls -1 /dev/tty.* 2>/dev/null || true)
    fi

    # Linux
    if [[ -d /dev/ttyUSB* ]] || [[ -d /dev/ttyACM* ]]; then
        while IFS= read -r port; do
            serial_ports+=("${port}")
        done < <(ls -1 /dev/ttyUSB* /dev/ttyACM* 2>/dev/null || true)
    fi

    if (( ${#serial_ports[@]} > 0 )); then
        print_success "Found ${#serial_ports[@]} serial port(s)"
        for port in "${serial_ports[@]}"; do
            echo -e "  ${GREEN}○${NC} ${port}"
            log_message "INFO" "Serial port found: ${port}"
        done
    else
        add_warning "No serial ports detected"
        add_warning "Newport 1830-C and ESP300 may use serial connections"
        log_message "WARN" "No serial ports found"
    fi
}

# ============================================================================
# Camera Detection
# ============================================================================

check_pvcam_camera() {
    print_header "PVCAM Camera Verification"

    print_info "Checking for PVCAM camera..."

    # Check PVCAM library
    local pvcam_found=false
    for lib_path in /usr/local/lib /opt/pvcam /usr/lib; do
        if [[ -f "${lib_path}/libpv.so" ]] || [[ -f "${lib_path}/libpv.dylib" ]]; then
            print_success "PVCAM library found: ${lib_path}"
            log_message "INFO" "PVCAM library found"
            pvcam_found=true
            break
        fi
    done

    if [[ "${pvcam_found}" == false ]]; then
        add_warning "PVCAM library not found - camera may not be accessible"
        log_message "WARN" "PVCAM library not found"
    fi

    # Check for camera device files (Linux)
    if [[ -d /dev/video* ]]; then
        local video_count=$(ls -1 /dev/video* 2>/dev/null | wc -l)
        print_success "Found ${video_count} video device(s)"
        log_message "INFO" "Video devices: ${video_count}"
    fi

    # Check camera permissions
    if [[ -e /dev/video0 ]]; then
        if [[ -r /dev/video0 ]] && [[ -w /dev/video0 ]]; then
            print_success "Camera device is readable and writable"
            log_message "INFO" "Camera device permissions OK"
        else
            add_warning "Camera device permissions may be insufficient"
            add_warning "May need to be added to 'video' group or run with sudo"
            log_message "WARN" "Camera device permissions issue"
        fi
    fi
}

# ============================================================================
# Network Checks
# ============================================================================

check_network_connectivity() {
    print_header "Network Connectivity Verification"

    print_info "Checking network connectivity..."

    # Check general connectivity
    if ping -c 1 -W 2 8.8.8.8 &>/dev/null 2>&1; then
        print_success "Internet connectivity OK"
        log_message "INFO" "Internet connectivity available"
    else
        add_warning "No internet connectivity detected"
        log_message "WARN" "No internet access"
    fi

    # Check local network
    print_info "Checking local network interfaces..."
    local interface_count=0
    if [[ "$(uname)" == "Darwin" ]]; then
        interface_count=$(ifconfig | grep "inet " | wc -l)
    else
        interface_count=$(ip addr show | grep "inet " | wc -l)
    fi

    if (( interface_count > 0 )); then
        print_success "Found ${interface_count} active network interface(s)"
        log_message "INFO" "Network interfaces: ${interface_count}"
    else
        add_error "No active network interfaces detected"
        log_message "ERROR" "No network interfaces"
    fi
}

check_maitai_eos_network() {
    print_info "Checking maitai-eos network accessibility..."

    if timeout 5 ping -c 1 maitai-eos &>/dev/null 2>&1; then
        print_success "maitai-eos is reachable via network"
        log_message "INFO" "maitai-eos network reachable"
    else
        add_warning "Cannot ping maitai-eos - may be off or unreachable"
        add_warning "Will attempt SSH connection as fallback"
        log_message "WARN" "Cannot ping maitai-eos"
    fi
}

# ============================================================================
# Disk Space Checks
# ============================================================================

check_disk_space() {
    print_header "Disk Space Verification"

    print_info "Checking disk space..."

    local project_path="${PROJECT_ROOT}"
    local available_kb=$(df "${project_path}" | tail -1 | awk '{print $4}')
    local available_gb=$((available_kb / 1024 / 1024))

    print_info "Available disk space: ${available_gb}GB"

    # Check log directory space
    if [[ -d "${LOGS_DIR}" ]]; then
        local logs_size_kb=$(du -sk "${LOGS_DIR}" 2>/dev/null | awk '{print $1}')
        local logs_size_mb=$((logs_size_kb / 1024))
        print_info "Existing logs: ${logs_size_mb}MB"
    fi

    # Requirement: 1GB for test outputs
    if (( available_gb < 1 )); then
        add_error "Insufficient disk space (need 1GB, have ${available_gb}GB)"
        log_message "ERROR" "Insufficient disk space: ${available_gb}GB"
        return 1
    else
        print_success "Sufficient disk space available"
        log_message "INFO" "Disk space OK: ${available_gb}GB"
        return 0
    fi
}

# ============================================================================
# Rust Environment Checks
# ============================================================================

check_rust_environment() {
    print_header "Rust Environment Verification"

    print_info "Checking Rust installation..."

    if ! command -v rustc &>/dev/null; then
        add_error "Rust compiler (rustc) not found"
        log_message "ERROR" "rustc not found"
        return 1
    fi

    local rust_version=$(rustc --version)
    print_success "${rust_version}"
    log_message "INFO" "Rust version: ${rust_version}"

    if ! command -v cargo &>/dev/null; then
        add_error "Cargo not found"
        log_message "ERROR" "Cargo not found"
        return 1
    fi

    local cargo_version=$(cargo --version)
    print_success "${cargo_version}"
    log_message "INFO" "Cargo version: ${cargo_version}"

    # Check if project builds
    print_info "Verifying project compilation..."
    if cargo check --release 2>&1 | head -5; then
        print_success "Project compiles successfully"
        log_message "INFO" "Project compilation OK"
    else
        add_warning "Project compilation may have warnings"
        log_message "WARN" "Project compilation issues"
    fi
}

# ============================================================================
# Report Generation
# ============================================================================

print_verification_report() {
    print_header "Hardware Verification Report"

    local timestamp=$(date '+%Y-%m-%d %H:%M:%S')
    echo "Verification Report"
    echo "  Timestamp: ${timestamp}"
    echo "  System: $(uname -s)"
    echo ""

    if (( ${#ERRORS[@]} == 0 )); then
        echo -e "${GREEN}✓ All critical checks passed${NC}"
    else
        echo -e "${RED}✗ ${#ERRORS[@]} critical error(s) found:${NC}"
        for error in "${ERRORS[@]}"; do
            echo -e "  ${RED}•${NC} ${error}"
        done
        echo ""
    fi

    if (( ${#WARNINGS[@]} > 0 )); then
        echo -e "${YELLOW}⚠ ${#WARNINGS[@]} warning(s):${NC}"
        for warning in "${WARNINGS[@]}"; do
            echo -e "  ${YELLOW}•${NC} ${warning}"
        done
        echo ""
    fi

    echo "Recommendations:"
    if (( ${#ERRORS[@]} > 0 )); then
        echo "  1. Fix all critical errors before running tests"
        echo "  2. Review error messages above for details"
        echo "  3. Check log file for more information"
    fi
    if (( ${#WARNINGS[@]} > 0 )); then
        echo "  1. Review warnings - may affect test reliability"
        echo "  2. Install missing dependencies if required"
    fi
    if (( ${#ERRORS[@]} == 0 )) && (( ${#WARNINGS[@]} == 0 )); then
        echo "  System ready for hardware testing!"
    fi

    echo ""
    echo "Detailed log: ${VERIFY_REPORT}"
}

# ============================================================================
# Main Execution
# ============================================================================

main() {
    # Parse arguments
    while [[ $# -gt 0 ]]; do
        case $1 in
            --quick)
                QUICK_MODE=true
                shift
                ;;
            --verbose)
                VERBOSE_MODE=true
                shift
                ;;
            -h|--help)
                echo "Usage: $0 [OPTIONS]"
                echo ""
                echo "Options:"
                echo "  --quick    Quick connectivity check only"
                echo "  --verbose  Detailed output"
                echo "  -h, --help Show this help message"
                exit 0
                ;;
            *)
                print_error "Unknown option: $1"
                exit 1
                ;;
        esac
    done

    setup_logging
    print_header "Hardware Verification Suite"

    if [[ "${QUICK_MODE}" == true ]]; then
        print_info "Running QUICK verification (connectivity only)..."
        log_message "INFO" "Quick verification mode"

        check_ssh_connectivity || true
        check_ssh_key_setup || true
        check_network_connectivity || true
    else
        print_info "Running FULL verification..."
        log_message "INFO" "Full verification mode"

        # Network and SSH checks
        check_network_connectivity || true
        check_ssh_key_setup || true
        check_ssh_connectivity || true
        check_maitai_eos_network || true

        # Hardware checks
        check_serial_ports || true
        check_visa_resources || true
        check_visa_devices || true
        check_pvcam_camera || true

        # System checks
        check_disk_space || true
        check_rust_environment || true
    fi

    # Print report
    print_verification_report

    # Determine exit code
    if (( ${#ERRORS[@]} > 0 )); then
        log_message "ERROR" "Verification failed with ${#ERRORS[@]} critical error(s)"
        exit 1
    fi

    log_message "INFO" "Verification completed"
    exit 0
}

main "$@"
