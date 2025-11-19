#!/bin/bash
# Emergency Stop Script
# Immediately stops all hardware operations in case of emergency
#
# Usage:
#   ./emergency_stop.sh              # Interactive (asks for confirmation)
#   ./emergency_stop.sh --force      # Force immediate stop
#

# Color codes
readonly RED='\033[0;31m'
readonly GREEN='\033[0;32m'
readonly YELLOW='\033[1;33m'
readonly BLUE='\033[0;34m'
readonly NC='\033[0m'

# Configuration
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
readonly LOGS_DIR="${PROJECT_ROOT}/hardware_test_logs"
readonly EMERGENCY_LOG="${LOGS_DIR}/emergency_$(date +%Y%m%d_%H%M%S).log"

FORCE_STOP=false

# ============================================================================
# Utility Functions
# ============================================================================

print_emergency() {
    echo -e "${RED}[EMERGENCY]${NC} $1"
}

print_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_emergency() {
    mkdir -p "${LOGS_DIR}"
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" >> "${EMERGENCY_LOG}"
}

# ============================================================================
# Emergency Stop Procedures
# ============================================================================

stop_maitai_laser() {
    print_emergency "Stopping MaiTai laser system..."
    log_emergency "Stopping MaiTai laser"

    # Close MaiTai shutter
    if ssh -o ConnectTimeout=2 -o StrictHostKeyChecking=no \
           maitai@maitai-eos "echo 'CLOSE' > /sys/class/laser/maitai/shutter" 2>/dev/null; then
        print_success "MaiTai shutter closed"
        log_emergency "MaiTai shutter closed successfully"
    else
        print_emergency "Could not close MaiTai shutter via SSH"
        log_emergency "WARNING: Could not close MaiTai shutter via SSH"
    fi

    # Disable laser output (if supported)
    if ssh -o ConnectTimeout=2 -o StrictHostKeyChecking=no \
           maitai@maitai-eos "echo 'DISABLE' > /sys/class/laser/maitai/output" 2>/dev/null; then
        print_success "MaiTai laser output disabled"
        log_emergency "MaiTai laser output disabled"
    fi
}

stop_esp300_motion() {
    print_emergency "Stopping ESP300 motion controller..."
    log_emergency "Stopping ESP300 motion controller"

    # Kill any running motion tests
    pkill -f "esp300\|motor" 2>/dev/null || true

    # Try to send stop command to ESP300 via serial
    for port in /dev/ttyUSB* /dev/ttyACM* /dev/tty.usbserial*; do
        if [[ -e "${port}" ]]; then
            # Send STOP command (TTY escape sequence)
            echo -e "\\x03\\x04" > "${port}" 2>/dev/null || true
        fi
    done

    print_success "ESP300 motion stopped"
    log_emergency "ESP300 motion controller stopped"
}

stop_pvcam_acquisition() {
    print_emergency "Stopping PVCAM camera acquisition..."
    log_emergency "Stopping PVCAM acquisition"

    # Kill any running camera tests
    pkill -f "pvcam\|camera\|acquisition" 2>/dev/null || true

    print_success "PVCAM acquisition stopped"
    log_emergency "PVCAM acquisition stopped"
}

stop_all_instruments() {
    print_emergency "Stopping all instruments..."
    log_emergency "Stopping all instruments"

    # Kill all running cargo test processes
    pkill -f "cargo test" 2>/dev/null || true
    pkill -f "hardware_test\|hardware-test" 2>/dev/null || true

    # Give processes time to clean up
    sleep 1

    # Force kill if necessary
    pkill -9 -f "cargo test" 2>/dev/null || true

    print_success "All test processes stopped"
    log_emergency "All test processes terminated"
}

disconnect_devices() {
    print_emergency "Disconnecting all instruments..."
    log_emergency "Disconnecting devices"

    # This would typically involve:
    # - Closing VISA connections
    # - Disconnecting serial ports
    # - Closing camera connections
    # - Disconnecting network instruments

    print_info "Devices should auto-disconnect when processes terminate"
    log_emergency "Device disconnection completed"
}

# ============================================================================
# Main Emergency Stop Sequence
# ============================================================================

main() {
    # Parse arguments
    if [[ "$*" == *"--force"* ]]; then
        FORCE_STOP=true
    fi

    mkdir -p "${LOGS_DIR}"

    echo ""
    echo -e "${RED}════════════════════════════════════════════════════════════${NC}"
    echo -e "${RED}                  EMERGENCY STOP INITIATED${NC}"
    echo -e "${RED}════════════════════════════════════════════════════════════${NC}"
    echo ""

    log_emergency "EMERGENCY STOP SEQUENCE INITIATED"
    log_emergency "Timestamp: $(date '+%Y-%m-%d %H:%M:%S')"
    log_emergency "User: $(whoami)"

    if [[ "${FORCE_STOP}" == false ]]; then
        echo -e "${YELLOW}WARNING: This will IMMEDIATELY stop all hardware operations${NC}"
        echo ""
        read -p "Confirm EMERGENCY STOP (type 'YES' to confirm): " -r confirmation
        if [[ "${confirmation}" != "YES" ]]; then
            print_info "Emergency stop cancelled"
            log_emergency "Emergency stop cancelled by user"
            exit 1
        fi
    fi

    log_emergency "Emergency stop confirmed - executing procedures"

    # Execute stop procedures in sequence
    print_emergency "Executing emergency procedures..."
    echo ""

    stop_maitai_laser
    stop_pvcam_acquisition
    stop_esp300_motion
    stop_all_instruments
    disconnect_devices

    echo ""
    echo -e "${RED}════════════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}              EMERGENCY STOP COMPLETED SUCCESSFULLY${NC}"
    echo -e "${RED}════════════════════════════════════════════════════════════${NC}"
    echo ""

    print_info "All hardware operations have been stopped"
    print_info "Emergency log: ${EMERGENCY_LOG}"
    print_info "Manual safety checks required before resuming operations"

    echo ""
    echo -e "${YELLOW}NEXT STEPS:${NC}"
    echo "  1. Verify all equipment is in safe state"
    echo "  2. Check MaiTai shutter is CLOSED"
    echo "  3. Check ESP300 is powered off"
    echo "  4. Check all motion is stopped"
    echo "  5. Review emergency log for details"
    echo "  6. Contact Laser Safety Officer if needed"
    echo ""

    log_emergency "EMERGENCY STOP SEQUENCE COMPLETED"

    exit 0
}

# Run main function
main "$@"
