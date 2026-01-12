#!/bin/bash
# Environment Validation Script for rust-daq
#
# This script validates that all required environment variables and dependencies
# are properly configured before building or testing.
#
# Usage:
#   source scripts/env-check.sh          # Validate AND set up environment
#   ./scripts/env-check.sh --check       # Validate only (no modification)
#   ./scripts/env-check.sh --help        # Show help
#
# Exit codes:
#   0 - All checks passed (or environment was set up successfully)
#   1 - Critical error (missing SDK, libraries, etc.)
#   2 - Warning (optional components missing)

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# Configuration - adjust these for your system
PVCAM_ROOT="${PVCAM_ROOT:-/opt/pvcam}"
PVCAM_SDK_DEFAULT="${PVCAM_ROOT}/sdk"
PVCAM_LIB_DEFAULT="${PVCAM_ROOT}/library/x86_64"
PVCAM_UMD_DEFAULT="${PVCAM_ROOT}/drivers/user-mode"
PVCAM_INI_PATH="${PVCAM_ROOT}/pvcam.ini"

# Track issues
ERRORS=0
WARNINGS=0
CHECK_ONLY=false

# Logging functions
log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[OK]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; ((WARNINGS++)) || true; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; ((ERRORS++)) || true; }
log_header() { echo -e "\n${BOLD}${CYAN}=== $1 ===${NC}"; }
log_fix() { echo -e "  ${YELLOW}Fix:${NC} $1"; }

show_help() {
    cat << 'EOF'
rust-daq Environment Validation Script

USAGE:
    source scripts/env-check.sh     # Validate AND set up environment (recommended)
    ./scripts/env-check.sh --check  # Validate only, don't modify environment
    ./scripts/env-check.sh --help   # Show this help

MODES:
    Sourced mode (source scripts/env-check.sh):
        - Validates all environment requirements
        - Sets missing environment variables to defaults
        - Sources /etc/profile.d/pvcam.sh if available
        - Ready to build immediately after

    Check-only mode (./scripts/env-check.sh --check):
        - Validates all environment requirements
        - Reports issues but doesn't modify environment
        - Returns exit code 0 (success) or 1 (errors)

WHAT IT CHECKS:
    1. PVCAM SDK installation (/opt/pvcam/sdk)
    2. PVCAM libraries (/opt/pvcam/library/x86_64)
    3. PVCAM_VERSION environment variable
    4. LD_LIBRARY_PATH includes PVCAM libraries
    5. LIBRARY_PATH for build-time linking
    6. pvcam.ini configuration file
    7. Rust toolchain availability

ENVIRONMENT VARIABLES:
    PVCAM_SDK_DIR     Path to PVCAM SDK (default: /opt/pvcam/sdk)
    PVCAM_LIB_DIR     Path to PVCAM libs (default: /opt/pvcam/library/x86_64)
    PVCAM_VERSION     PVCAM library version (required at runtime)
    LIBRARY_PATH      Build-time library search path
    LD_LIBRARY_PATH   Runtime library search path

EXAMPLES:
    # Set up environment and build
    source scripts/env-check.sh && cargo build --features pvcam_sdk

    # Validate before running tests
    source scripts/env-check.sh && cargo test --features hardware_tests

    # CI/pre-commit validation
    ./scripts/env-check.sh --check || exit 1

EOF
    exit 0
}

# Detect if script is being sourced
is_sourced() {
    [[ "${BASH_SOURCE[0]}" != "${0}" ]]
}

# Check if a directory exists and is readable
check_dir() {
    local path="$1"
    local desc="$2"
    local required="${3:-true}"

    if [[ -d "$path" && -r "$path" ]]; then
        log_success "$desc: $path"
        return 0
    elif [[ "$required" == "true" ]]; then
        log_error "$desc not found or not readable: $path"
        return 1
    else
        log_warn "$desc not found (optional): $path"
        return 0
    fi
}

# Check if a file exists
check_file() {
    local path="$1"
    local desc="$2"
    local required="${3:-true}"

    if [[ -f "$path" && -r "$path" ]]; then
        log_success "$desc: $path"
        return 0
    elif [[ "$required" == "true" ]]; then
        log_error "$desc not found: $path"
        return 1
    else
        log_warn "$desc not found (optional): $path"
        return 0
    fi
}

# Check environment variable
check_env() {
    local var="$1"
    local desc="$2"
    local required="${3:-true}"
    local value="${!var:-}"

    if [[ -n "$value" ]]; then
        log_success "$var=$value"
        return 0
    elif [[ "$required" == "true" ]]; then
        log_error "$var is not set ($desc)"
        return 1
    else
        log_warn "$var is not set (optional: $desc)"
        return 0
    fi
}

# Check if path is in a PATH-like variable
check_in_path() {
    local pathvar="$1"
    local required_path="$2"
    local desc="$3"
    local pathvalue="${!pathvar:-}"

    if [[ ":$pathvalue:" == *":$required_path:"* ]]; then
        log_success "$required_path is in $pathvar"
        return 0
    else
        log_warn "$required_path is NOT in $pathvar ($desc)"
        return 1
    fi
}

# Extract PVCAM_VERSION from pvcam.ini if it exists
get_pvcam_version_from_ini() {
    if [[ -f "$PVCAM_INI_PATH" ]]; then
        grep -E "^PVCAM_VERSION=" "$PVCAM_INI_PATH" 2>/dev/null | cut -d= -f2 || true
    fi
}

# Main validation logic
validate_environment() {
    local feature="${1:-}"

    log_header "Checking Host Information"
    echo "  Hostname: $(hostname)"
    echo "  Date: $(date)"
    echo "  User: $(whoami)"
    echo "  PWD: $(pwd)"

    log_header "Checking Rust Toolchain"
    if command -v rustc &>/dev/null; then
        log_success "rustc: $(rustc --version)"
    else
        log_error "rustc not found in PATH"
        log_fix "Install Rust: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    fi

    if command -v cargo &>/dev/null; then
        log_success "cargo: $(cargo --version)"
    else
        log_error "cargo not found in PATH"
    fi

    # Check for PVCAM features - only validate PVCAM env if building with those features
    log_header "Checking PVCAM Environment"

    # First, try to source the system PVCAM profile if available
    if [[ -f /etc/profile.d/pvcam.sh ]]; then
        log_success "Found /etc/profile.d/pvcam.sh"
        if ! $CHECK_ONLY && is_sourced; then
            log_info "Sourcing /etc/profile.d/pvcam.sh..."
            # shellcheck source=/dev/null
            source /etc/profile.d/pvcam.sh
        fi
    else
        log_warn "/etc/profile.d/pvcam.sh not found (PVCAM may not be installed system-wide)"
    fi

    # Check PVCAM directories
    check_dir "$PVCAM_SDK_DEFAULT" "PVCAM SDK include" false || true
    check_dir "$PVCAM_LIB_DEFAULT" "PVCAM libraries" false || true
    check_dir "$PVCAM_UMD_DEFAULT" "PVCAM user-mode drivers" false || true

    # Check pvcam.ini
    if [[ -f "$PVCAM_INI_PATH" ]]; then
        log_success "pvcam.ini found: $PVCAM_INI_PATH"
        local ini_version
        ini_version=$(get_pvcam_version_from_ini)
        if [[ -n "$ini_version" ]]; then
            log_info "  PVCAM_VERSION in ini: $ini_version"
        fi
    else
        log_warn "pvcam.ini not found at $PVCAM_INI_PATH"
        log_fix "Create /opt/pvcam/pvcam.ini with PVCAM_VERSION=<version>"
    fi

    # Check required environment variables
    log_header "Checking Environment Variables"

    # PVCAM_VERSION is critical at runtime
    if ! check_env "PVCAM_VERSION" "Required for PVCAM runtime" false; then
        local ini_version
        ini_version=$(get_pvcam_version_from_ini)
        if [[ -n "$ini_version" ]]; then
            log_info "  Suggestion: export PVCAM_VERSION=$ini_version"
            if ! $CHECK_ONLY && is_sourced; then
                export PVCAM_VERSION="$ini_version"
                log_success "  Set PVCAM_VERSION=$ini_version"
            fi
        else
            log_fix "Set PVCAM_VERSION to your installed version (e.g., 7.1.1.118)"
        fi
    fi

    # PVCAM_SDK_DIR needed for building with pvcam-sdk feature
    if ! check_env "PVCAM_SDK_DIR" "Required for cargo build --features pvcam_sdk" false; then
        if [[ -d "$PVCAM_SDK_DEFAULT" ]]; then
            if ! $CHECK_ONLY && is_sourced; then
                export PVCAM_SDK_DIR="$PVCAM_SDK_DEFAULT"
                log_success "  Set PVCAM_SDK_DIR=$PVCAM_SDK_DEFAULT"
            else
                log_fix "export PVCAM_SDK_DIR=$PVCAM_SDK_DEFAULT"
            fi
        fi
    fi

    # LIBRARY_PATH for build-time linking
    log_header "Checking Library Paths"

    if ! check_in_path "LIBRARY_PATH" "$PVCAM_LIB_DEFAULT" "Build-time linking"; then
        if ! $CHECK_ONLY && is_sourced; then
            export LIBRARY_PATH="${PVCAM_LIB_DEFAULT}:${LIBRARY_PATH:-}"
            log_success "  Added $PVCAM_LIB_DEFAULT to LIBRARY_PATH"
        else
            log_fix "export LIBRARY_PATH=$PVCAM_LIB_DEFAULT:\$LIBRARY_PATH"
        fi
    fi

    if ! check_in_path "LD_LIBRARY_PATH" "$PVCAM_LIB_DEFAULT" "Runtime linking"; then
        if ! $CHECK_ONLY && is_sourced; then
            export LD_LIBRARY_PATH="${PVCAM_LIB_DEFAULT}:${LD_LIBRARY_PATH:-}"
            log_success "  Added $PVCAM_LIB_DEFAULT to LD_LIBRARY_PATH"
        else
            log_fix "export LD_LIBRARY_PATH=$PVCAM_LIB_DEFAULT:\$LD_LIBRARY_PATH"
        fi
    fi

    # Check for UMD in LD_LIBRARY_PATH (needed for USB camera support)
    if [[ -d "$PVCAM_UMD_DEFAULT" ]]; then
        if ! check_in_path "LD_LIBRARY_PATH" "$PVCAM_UMD_DEFAULT" "USB camera support"; then
            if ! $CHECK_ONLY && is_sourced; then
                export LD_LIBRARY_PATH="${PVCAM_UMD_DEFAULT}:${LD_LIBRARY_PATH:-}"
                log_success "  Added $PVCAM_UMD_DEFAULT to LD_LIBRARY_PATH"
            else
                log_fix "export LD_LIBRARY_PATH=$PVCAM_UMD_DEFAULT:\$LD_LIBRARY_PATH"
            fi
        fi
    fi

    # Check for libpvcam.so specifically
    log_header "Checking PVCAM Library Files"
    local libpvcam="${PVCAM_LIB_DEFAULT}/libpvcam.so"
    if [[ -f "$libpvcam" ]]; then
        log_success "libpvcam.so found: $libpvcam"
        # Show library version info if available
        if command -v readelf &>/dev/null; then
            local soname
            soname=$(readelf -d "$libpvcam" 2>/dev/null | grep SONAME | sed 's/.*\[\(.*\)\]/\1/' || true)
            if [[ -n "$soname" ]]; then
                log_info "  SONAME: $soname"
            fi
        fi
    else
        log_warn "libpvcam.so not found at $libpvcam"
    fi
}

# Summary and exit
print_summary() {
    log_header "Summary"

    if [[ $ERRORS -eq 0 && $WARNINGS -eq 0 ]]; then
        echo -e "${GREEN}${BOLD}All checks passed!${NC}"
        echo ""
        echo "Ready to build with PVCAM features:"
        echo "  cargo build --features pvcam_sdk"
        echo "  cargo test --features hardware_tests"
        return 0
    elif [[ $ERRORS -eq 0 ]]; then
        echo -e "${YELLOW}${BOLD}$WARNINGS warnings, but no critical errors.${NC}"
        echo "Build may work for non-PVCAM features."
        return 0
    else
        echo -e "${RED}${BOLD}$ERRORS errors, $WARNINGS warnings.${NC}"
        echo ""
        echo "Fix the errors above before building with PVCAM features."
        echo ""
        echo "Quick fix on maitai:"
        echo "  source /etc/profile.d/pvcam.sh"
        echo "  export PVCAM_SDK_DIR=/opt/pvcam/sdk"
        echo "  export LIBRARY_PATH=/opt/pvcam/library/x86_64:\$LIBRARY_PATH"
        return 1
    fi
}

# Quick setup function (can be called separately)
quick_setup() {
    log_info "Quick environment setup for PVCAM development"

    # Source system profile if available
    if [[ -f /etc/profile.d/pvcam.sh ]]; then
        # shellcheck source=/dev/null
        source /etc/profile.d/pvcam.sh
    fi

    # Set defaults
    export PVCAM_SDK_DIR="${PVCAM_SDK_DIR:-$PVCAM_SDK_DEFAULT}"
    export LIBRARY_PATH="${PVCAM_LIB_DEFAULT}:${LIBRARY_PATH:-}"
    export LD_LIBRARY_PATH="${PVCAM_LIB_DEFAULT}:${PVCAM_UMD_DEFAULT}:${LD_LIBRARY_PATH:-}"

    # Try to get version from ini if not set
    if [[ -z "${PVCAM_VERSION:-}" ]]; then
        local ini_version
        ini_version=$(get_pvcam_version_from_ini)
        if [[ -n "$ini_version" ]]; then
            export PVCAM_VERSION="$ini_version"
        fi
    fi

    log_success "Environment configured"
    echo "  PVCAM_SDK_DIR=$PVCAM_SDK_DIR"
    echo "  PVCAM_VERSION=${PVCAM_VERSION:-<not set>}"
    echo "  LIBRARY_PATH includes: $PVCAM_LIB_DEFAULT"
    echo "  LD_LIBRARY_PATH includes: $PVCAM_LIB_DEFAULT, $PVCAM_UMD_DEFAULT"
}

# Parse arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --check)
                CHECK_ONLY=true
                shift
                ;;
            --quick)
                quick_setup
                exit 0
                ;;
            --help|-h)
                show_help
                ;;
            *)
                echo "Unknown option: $1"
                show_help
                ;;
        esac
    done
}

# Main entry point
main() {
    parse_args "$@"

    echo -e "${BOLD}rust-daq Environment Validation${NC}"
    echo "=================================="

    if $CHECK_ONLY; then
        echo "(Check-only mode - environment will not be modified)"
    elif is_sourced; then
        echo "(Sourced mode - environment will be configured automatically)"
    else
        echo "(Run mode - validating only. Use 'source $0' to also configure environment)"
    fi

    validate_environment
    print_summary
}

# Run main if not being sourced OR if being sourced with arguments
if ! is_sourced || [[ $# -gt 0 ]]; then
    main "$@"
elif is_sourced; then
    # Being sourced without arguments - do quick setup
    main
fi
