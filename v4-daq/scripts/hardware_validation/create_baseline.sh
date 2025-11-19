#!/bin/bash
#
# Hardware Validation Baseline Creation Script
#
# This script runs the hardware validation test suite and captures the results
# as a baseline for regression testing. Future test runs will be compared against
# this baseline to detect regressions.
#
# Usage:
#   ./scripts/hardware_validation/create_baseline.sh [OPTIONS]
#
# Options:
#   --system-id <ID>       System identifier (default: maitai-eos)
#   --output-dir <DIR>     Output directory (default: test-results)
#   --compare              Compare with existing baseline
#   --verbose              Enable verbose output
#   --help                 Show this help message

set -e

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SYSTEM_ID="maitai-eos"
OUTPUT_DIR="${PROJECT_ROOT}/test-results"
BASELINE_FILE="${OUTPUT_DIR}/baseline.json"
COMPARE_BASELINE=false
VERBOSE=false

# Color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Logging functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_verbose() {
    if [ "$VERBOSE" = true ]; then
        echo -e "${BLUE}[VERBOSE]${NC} $1"
    fi
}

# Print help
print_help() {
    grep '^#' "$0" | grep -v '#!/bin/bash' | sed 's/^# //'
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --system-id)
            SYSTEM_ID="$2"
            shift 2
            ;;
        --output-dir)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        --compare)
            COMPARE_BASELINE=true
            shift
            ;;
        --verbose)
            VERBOSE=true
            shift
            ;;
        --help)
            print_help
            exit 0
            ;;
        *)
            log_error "Unknown option: $1"
            print_help
            exit 1
            ;;
    esac
done

# Main script
main() {
    log_info "Hardware Validation Baseline Creation"
    log_info "System ID: $SYSTEM_ID"
    log_info "Output Directory: $OUTPUT_DIR"

    # Create output directory
    mkdir -p "$OUTPUT_DIR"
    log_verbose "Created output directory: $OUTPUT_DIR"

    # Check if cargo is available
    if ! command -v cargo &> /dev/null; then
        log_error "cargo not found. Please install Rust."
        exit 1
    fi
    log_verbose "cargo found: $(cargo --version)"

    # Change to project root
    cd "$PROJECT_ROOT"
    log_verbose "Changed to project directory: $PROJECT_ROOT"

    # Build the project
    log_info "Building project..."
    if cargo build --example generate_test_report 2>&1 | {
        if [ "$VERBOSE" = true ]; then
            cat
        else
            grep -E "error|warning|Compiling|Finished" || true
        fi
    }; then
        log_success "Build completed successfully"
    else
        log_error "Build failed"
        exit 1
    fi

    # Run the test report generator
    log_info "Generating test report..."
    REPORT_OUTPUT=$(mktemp)
    if cargo run --example generate_test_report -- \
        --system-id "$SYSTEM_ID" \
        --output "$OUTPUT_DIR" > "$REPORT_OUTPUT" 2>&1; then
        log_success "Test report generated successfully"
        if [ "$VERBOSE" = true ]; then
            cat "$REPORT_OUTPUT"
        fi
    else
        log_error "Test report generation failed"
        cat "$REPORT_OUTPUT"
        rm "$REPORT_OUTPUT"
        exit 1
    fi
    rm "$REPORT_OUTPUT"

    # Find the most recent report
    LATEST_REPORT=$(find "$OUTPUT_DIR" -name "report.json" -type f -printf '%T@ %p\n' | sort -rn | head -1 | cut -d' ' -f2-)

    if [ -z "$LATEST_REPORT" ]; then
        log_error "No report found in $OUTPUT_DIR"
        exit 1
    fi

    log_verbose "Latest report: $LATEST_REPORT"

    # Create baseline if it doesn't exist or overwrite if requested
    if [ ! -f "$BASELINE_FILE" ]; then
        cp "$LATEST_REPORT" "$BASELINE_FILE"
        log_success "Baseline created: $BASELINE_FILE"
    else
        if [ "$COMPARE_BASELINE" = true ]; then
            log_info "Comparing with existing baseline..."
            compare_results "$BASELINE_FILE" "$LATEST_REPORT"
        else
            log_warning "Baseline already exists at $BASELINE_FILE"
            log_info "Use --compare flag to compare with existing baseline"
        fi

        # Option to update baseline
        read -p "Update baseline file? (y/n) " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            cp "$LATEST_REPORT" "$BASELINE_FILE"
            log_success "Baseline updated"
        else
            log_info "Baseline unchanged"
        fi
    fi

    # Summary
    echo ""
    log_info "Summary:"
    local -r baseline_size=$(du -h "$BASELINE_FILE" | cut -f1)
    echo "  Baseline file: $BASELINE_FILE (${baseline_size})"
    echo "  Report directory: $(dirname "$LATEST_REPORT")"
    echo "  Markdown report: $(dirname "$LATEST_REPORT")/report.md"
    echo "  CSV export: $(dirname "$LATEST_REPORT")/report.csv"

    # Extract test statistics
    if command -v jq &> /dev/null; then
        local -r total=$(jq '.total_tests' "$BASELINE_FILE" 2>/dev/null || echo "?")
        local -r passed=$(jq '.total_passed' "$BASELINE_FILE" 2>/dev/null || echo "?")
        local -r failed=$(jq '.total_failed' "$BASELINE_FILE" 2>/dev/null || echo "?")

        echo ""
        log_info "Test Statistics:"
        echo "  Total Tests: $total"
        echo "  Passed: $passed"
        echo "  Failed: $failed"
    fi

    log_success "Baseline creation complete!"
}

compare_results() {
    local baseline=$1
    local current=$2

    if ! command -v jq &> /dev/null; then
        log_warning "jq not found, skipping detailed comparison"
        return
    fi

    local baseline_passed=$(jq '.total_passed' "$baseline" 2>/dev/null || echo 0)
    local baseline_failed=$(jq '.total_failed' "$baseline" 2>/dev/null || echo 0)
    local current_passed=$(jq '.total_passed' "$current" 2>/dev/null || echo 0)
    local current_failed=$(jq '.total_failed' "$current" 2>/dev/null || echo 0)

    echo ""
    log_info "Regression Analysis:"
    echo "  Baseline - Passed: $baseline_passed, Failed: $baseline_failed"
    echo "  Current  - Passed: $current_passed, Failed: $current_failed"

    if [ "$current_failed" -gt "$baseline_failed" ]; then
        local new_failures=$((current_failed - baseline_failed))
        log_warning "$new_failures new test failure(s) detected!"
    elif [ "$current_passed" -gt "$baseline_passed" ]; then
        local new_passes=$((current_passed - baseline_passed))
        log_success "$new_passes new test(s) now passing!"
    else
        log_info "No changes from baseline"
    fi
}

# Run main function
main "$@"
