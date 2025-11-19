#!/bin/bash
set -euo pipefail

# Hardware Test Results Analysis Script
# Parses test output, calculates metrics, generates reports
#
# Usage:
#   ./analyze_results.sh                              # Analyze latest results
#   ./analyze_results.sh --report <report-file>      # Analyze specific report
#   ./analyze_results.sh --baseline <baseline.json>  # Compare against baseline
#   ./analyze_results.sh --issues                    # Generate GitHub issues
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
readonly ANALYSIS_REPORT="${LOGS_DIR}/analysis_${TIMESTAMP}.txt"
readonly METRICS_FILE="${LOGS_DIR}/metrics_${TIMESTAMP}.json"

# Analysis state
REPORT_FILE=""
BASELINE_FILE=""
GENERATE_ISSUES=false
declare -A PHASE_STATS=()
declare -a FAILURES=()
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

log_message() {
    local level=$1
    shift
    local timestamp=$(date '+%Y-%m-%d %H:%M:%S')
    echo "[${timestamp}] [${level}] $*" >> "${ANALYSIS_REPORT}"
}

setup_logging() {
    mkdir -p "${LOGS_DIR}"
    touch "${ANALYSIS_REPORT}"
    log_message "INFO" "Analysis started"
}

# ============================================================================
# Report Discovery
# ============================================================================

find_latest_report() {
    # Find the most recent test report
    local latest_report=$(find "${LOGS_DIR}" -name "test_report_*.txt" -type f | sort -r | head -1)

    if [[ -z "${latest_report}" ]]; then
        print_error "No test reports found in ${LOGS_DIR}"
        return 1
    fi

    REPORT_FILE="${latest_report}"
    print_info "Using report: $(basename "${REPORT_FILE}")"
    log_message "INFO" "Report file: ${REPORT_FILE}"
    return 0
}

# ============================================================================
# Log Parsing
# ============================================================================

parse_test_phase() {
    local phase=$1
    local phase_log="${LOGS_DIR}/${phase}_${TIMESTAMP}.log"

    # Find the actual log file (may have different timestamp)
    local actual_log=$(find "${LOGS_DIR}" -name "${phase}_*.log" -type f | sort -r | head -1)

    if [[ ! -f "${actual_log}" ]]; then
        print_warning "No log file found for phase: ${phase}"
        PHASE_STATS["${phase}"]="{\"status\": \"NOT_FOUND\", \"tests\": 0, \"passed\": 0, \"failed\": 0}"
        return 1
    fi

    print_info "Parsing ${phase} results from: $(basename "${actual_log}")"

    # Count test results
    local total_tests=$(grep -c "test.*ok\|test.*FAILED" "${actual_log}" || true)
    local passed_tests=$(grep -c "test.*ok" "${actual_log}" || true)
    local failed_tests=$(grep -c "test.*FAILED" "${actual_log}" || true)

    # Extract error messages
    local errors=$(grep "error\|Error\|ERROR" "${actual_log}" || true)

    # Build JSON stats
    PHASE_STATS["${phase}"]=$(cat <<EOF
{
  "phase": "${phase}",
  "status": "$([ ${failed_tests} -eq 0 ] && echo 'PASS' || echo 'FAIL')",
  "total_tests": ${total_tests},
  "passed": ${passed_tests},
  "failed": ${failed_tests},
  "pass_rate": $((total_tests > 0 ? passed_tests * 100 / total_tests : 0))
}
EOF
    )

    if (( failed_tests > 0 )); then
        FAILURES+=("${phase}: ${failed_tests} test(s) failed")
    fi

    log_message "INFO" "${phase} parsed: ${passed_tests}/${total_tests} passed"
}

parse_all_phases() {
    print_header "Parsing Test Results"

    local phases=("scpi" "newport" "esp300" "pvcam" "maitai")

    for phase in "${phases[@]}"; do
        parse_test_phase "${phase}" || true
    done
}

# ============================================================================
# Metrics Calculation
# ============================================================================

calculate_metrics() {
    print_header "Calculating Test Metrics"

    local total_passed=0
    local total_failed=0
    local total_tests=0
    local phases_passed=0
    local phases_total=0

    # Aggregate metrics
    for phase in "${!PHASE_STATS[@]}"; do
        local stats="${PHASE_STATS[${phase}]}"

        # Extract values from JSON
        local passed=$(echo "${stats}" | grep -o '"passed": [0-9]*' | cut -d' ' -f2 || echo 0)
        local failed=$(echo "${stats}" | grep -o '"failed": [0-9]*' | cut -d' ' -f2 || echo 0)
        local status=$(echo "${stats}" | grep -o '"status": "[^"]*' | cut -d'"' -f4)

        ((total_passed += passed))
        ((total_failed += failed))
        ((total_tests += passed + failed))
        ((phases_total++))

        if [[ "${status}" == "PASS" ]]; then
            ((phases_passed++))
        fi
    done

    local overall_pass_rate=0
    if (( total_tests > 0 )); then
        overall_pass_rate=$((total_passed * 100 / total_tests))
    fi

    local phase_pass_rate=0
    if (( phases_total > 0 )); then
        phase_pass_rate=$((phases_passed * 100 / phases_total))
    fi

    print_success "Total Tests: ${total_tests}"
    print_success "Tests Passed: ${total_passed}"
    print_error "Tests Failed: ${total_failed}"
    print_info "Overall Pass Rate: ${overall_pass_rate}%"
    print_info "Phase Pass Rate: ${phases_passed}/${phases_total} (${phase_pass_rate}%)"

    log_message "INFO" "Total tests: ${total_tests}"
    log_message "INFO" "Tests passed: ${total_passed}"
    log_message "INFO" "Tests failed: ${total_failed}"
    log_message "INFO" "Overall pass rate: ${overall_pass_rate}%"
}

# ============================================================================
# Baseline Comparison
# ============================================================================

compare_against_baseline() {
    if [[ ! -f "${BASELINE_FILE}" ]]; then
        print_warning "Baseline file not found: ${BASELINE_FILE}"
        log_message "WARN" "Baseline comparison skipped"
        return 1
    fi

    print_header "Baseline Comparison"

    print_info "Comparing against baseline: $(basename "${BASELINE_FILE}")"

    # Parse baseline metrics
    local baseline_passed=$(grep -o '"passed": [0-9]*' "${BASELINE_FILE}" | head -1 | cut -d' ' -f2 || echo 0)
    local baseline_failed=$(grep -o '"failed": [0-9]*' "${BASELINE_FILE}" | head -1 | cut -d' ' -f2 || echo 0)
    local baseline_rate=$(grep -o '"success_rate": [0-9]*' "${BASELINE_FILE}" | head -1 | cut -d' ' -f2 || echo 0)

    # Calculate current metrics
    local current_passed=0
    local current_failed=0

    for stats in "${PHASE_STATS[@]}"; do
        local passed=$(echo "${stats}" | grep -o '"passed": [0-9]*' | cut -d' ' -f2 || echo 0)
        local failed=$(echo "${stats}" | grep -o '"failed": [0-9]*' | cut -d' ' -f2 || echo 0)
        ((current_passed += passed))
        ((current_failed += failed))
    done

    local current_rate=0
    if (( current_passed + current_failed > 0 )); then
        current_rate=$((current_passed * 100 / (current_passed + current_failed)))
    fi

    # Compare
    echo "Baseline vs Current:"
    echo "  Baseline: ${baseline_passed}/${baseline_passed+baseline_failed} (${baseline_rate}%)"
    echo "  Current:  ${current_passed}/${current_passed+current_failed} (${current_rate}%)"
    echo ""

    # Calculate deltas
    local passed_delta=$((current_passed - baseline_passed))
    local failed_delta=$((current_failed - baseline_failed))
    local rate_delta=$((current_rate - baseline_rate))

    if (( passed_delta > 0 )); then
        print_success "Tests Passed: +${passed_delta} improvement"
    elif (( passed_delta < 0 )); then
        print_error "Tests Passed: ${passed_delta} regression"
        WARNINGS+=("Regression detected: ${passed_delta} fewer tests passed")
    fi

    if (( failed_delta > 0 )); then
        print_error "Tests Failed: +${failed_delta} regression"
        WARNINGS+=("Regression detected: ${failed_delta} more tests failed")
    elif (( failed_delta < 0 )); then
        print_success "Tests Failed: ${failed_delta} improvement"
    fi

    if (( rate_delta > 0 )); then
        print_success "Pass Rate: +${rate_delta}% improvement"
    elif (( rate_delta < 0 )); then
        print_error "Pass Rate: ${rate_delta}% regression"
    fi

    log_message "INFO" "Baseline comparison completed"
    log_message "INFO" "Passed delta: ${passed_delta}"
    log_message "INFO" "Failed delta: ${failed_delta}"
    log_message "INFO" "Rate delta: ${rate_delta}%"
}

# ============================================================================
# Issue Generation
# ============================================================================

generate_github_issues() {
    if (( ${#FAILURES[@]} == 0 )); then
        print_info "No failures detected - no issues to generate"
        return 0
    fi

    print_header "Generating GitHub Issues"

    local issue_dir="${LOGS_DIR}/github_issues_${TIMESTAMP}"
    mkdir -p "${issue_dir}"

    print_info "Creating ${#FAILURES[@]} issue(s) in: ${issue_dir}"

    for i in "${!FAILURES[@]}"; do
        local failure="${FAILURES[$i]}"
        local issue_num=$((i + 1))
        local issue_file="${issue_dir}/issue_${issue_num}.md"

        cat > "${issue_file}" << EOF
# Hardware Test Failure: ${failure}

## Description
Hardware validation test failed during test run on ${TIMESTAMP}.

**Failure:** ${failure}

## Test Phase
See test logs for detailed output.

## Steps to Reproduce
1. Run hardware validation test suite
2. Check phase logs for detailed errors

## Expected Behavior
All hardware tests should pass.

## Actual Behavior
Test phase failed - see attached logs.

## Environment
- Test Run: ${TIMESTAMP}
- Analysis Report: ${ANALYSIS_REPORT}
- Log Directory: ${LOGS_DIR}

## Labels
- hardware-test
- needs-investigation

## Related
- Analysis: analysis_${TIMESTAMP}.txt
EOF

        print_info "Created: $(basename "${issue_file}")"
        log_message "INFO" "Issue created: ${issue_file}"
    done

    print_success "Issues generated in: ${issue_dir}"
    echo ""
    echo "To create GitHub issues:"
    echo "  gh issue create --title '<title>' --body '<body>' --label 'hardware-test'"
    echo ""
    log_message "INFO" "GitHub issues generated"
}

# ============================================================================
# Report Generation
# ============================================================================

generate_analysis_report() {
    print_header "Generating Analysis Report"

    cat > "${ANALYSIS_REPORT}" << 'EOF'
================================================================================
                     HARDWARE TEST ANALYSIS REPORT
================================================================================

Test Run Information
--------------------
Timestamp: TIMESTAMP_PLACEHOLDER
Analysis Timestamp: ANALYSIS_TIMESTAMP_PLACEHOLDER
Report File: REPORT_PLACEHOLDER

Test Results Summary
--------------------
Total Phases: 5
Phases Passed: PHASES_PASSED_PLACEHOLDER
Phases Failed: PHASES_FAILED_PLACEHOLDER
Overall Pass Rate: PASS_RATE_PLACEHOLDER%

Phase Results
-------------
PHASE_DETAILS_PLACEHOLDER

Detailed Findings
-----------------
FINDINGS_PLACEHOLDER

Failures Detected
-----------------
FAILURES_PLACEHOLDER

Warnings
--------
WARNINGS_PLACEHOLDER

Recommendations
----------------
RECOMMENDATIONS_PLACEHOLDER

================================================================================
EOF

    # Generate phase details
    local phase_details=""
    for phase in scpi newport esp300 pvcam maitai; do
        if [[ -n "${PHASE_STATS[${phase}]:-}" ]]; then
            local stats="${PHASE_STATS[${phase}]}"
            phase_details+="  ${phase}: $(echo "${stats}" | grep -o '"status": "[^"]*' | cut -d'"' -f4)\n"
        fi
    done

    # Replace placeholders
    sed -i.bak "s|TIMESTAMP_PLACEHOLDER|${TIMESTAMP}|g" "${ANALYSIS_REPORT}"
    sed -i.bak "s|ANALYSIS_TIMESTAMP_PLACEHOLDER|$(date '+%Y-%m-%d %H:%M:%S')|g" "${ANALYSIS_REPORT}"
    sed -i.bak "s|REPORT_PLACEHOLDER|${REPORT_FILE}|g" "${ANALYSIS_REPORT}"
    sed -i.bak "s|PHASE_DETAILS_PLACEHOLDER|${phase_details}|g" "${ANALYSIS_REPORT}"

    # Add failures
    local failures_text=""
    if (( ${#FAILURES[@]} > 0 )); then
        for failure in "${FAILURES[@]}"; do
            failures_text+="  - ${failure}\n"
        done
    else
        failures_text="  None detected"
    fi
    sed -i.bak "s|FAILURES_PLACEHOLDER|${failures_text}|g" "${ANALYSIS_REPORT}"

    # Add warnings
    local warnings_text=""
    if (( ${#WARNINGS[@]} > 0 )); then
        for warning in "${WARNINGS[@]}"; do
            warnings_text+="  - ${warning}\n"
        done
    else
        warnings_text="  None detected"
    fi
    sed -i.bak "s|WARNINGS_PLACEHOLDER|${warnings_text}|g" "${ANALYSIS_REPORT}"

    # Add recommendations
    local recommendations=""
    if (( ${#FAILURES[@]} > 0 )); then
        recommendations+="1. Review and fix all failed test phases\n"
        recommendations+="2. Check hardware connections and configurations\n"
        recommendations+="3. Verify equipment calibration\n"
        recommendations+="4. Re-run tests after fixes\n"
        recommendations+="5. Compare results against baseline if available\n"
    fi
    if (( ${#WARNINGS[@]} > 0 )); then
        recommendations+="- Address all warnings before production use\n"
    fi
    if (( ${#FAILURES[@]} == 0 )) && (( ${#WARNINGS[@]} == 0 )); then
        recommendations+="- All tests passed successfully\n"
        recommendations+="- Hardware is ready for use\n"
        recommendations+="- Archive this report for baseline comparison\n"
    fi

    sed -i.bak "s|RECOMMENDATIONS_PLACEHOLDER|${recommendations}|g" "${ANALYSIS_REPORT}"

    rm -f "${ANALYSIS_REPORT}.bak"

    print_success "Analysis report generated: ${ANALYSIS_REPORT}"
    log_message "INFO" "Analysis report generated"
}

generate_metrics_json() {
    print_info "Generating metrics JSON..."

    local json_content='{\n'
    json_content+='  "timestamp": "'"${TIMESTAMP}"'",\n'
    json_content+='  "phases": {\n'

    local first=true
    for phase in scpi newport esp300 pvcam maitai; do
        if [[ -n "${PHASE_STATS[${phase}]:-}" ]]; then
            if [[ "${first}" == false ]]; then
                json_content+=',\n'
            fi
            json_content+='    "'"${phase}"'": '"${PHASE_STATS[${phase}]}"
            first=false
        fi
    done

    json_content+='\n  }\n'
    json_content+='}\n'

    echo -e "${json_content}" > "${METRICS_FILE}"
    print_success "Metrics file generated: ${METRICS_FILE}"
    log_message "INFO" "Metrics JSON generated"
}

print_analysis_summary() {
    print_header "Analysis Summary"

    echo "Analysis Report: ${ANALYSIS_REPORT}"
    echo "Metrics File: ${METRICS_FILE}"
    echo ""

    if (( ${#FAILURES[@]} > 0 )); then
        echo -e "${RED}Failures Detected:${NC}"
        for failure in "${FAILURES[@]}"; do
            echo "  - ${failure}"
        done
        echo ""
    fi

    if (( ${#WARNINGS[@]} > 0 )); then
        echo -e "${YELLOW}Warnings:${NC}"
        for warning in "${WARNINGS[@]}"; do
            echo "  - ${warning}"
        done
        echo ""
    fi

    if (( ${#FAILURES[@]} == 0 )); then
        echo -e "${GREEN}No failures detected - all tests passed${NC}"
    fi
}

# ============================================================================
# Main Execution
# ============================================================================

main() {
    # Parse arguments
    while [[ $# -gt 0 ]]; do
        case $1 in
            --report)
                REPORT_FILE="$2"
                shift 2
                ;;
            --baseline)
                BASELINE_FILE="$2"
                shift 2
                ;;
            --issues)
                GENERATE_ISSUES=true
                shift
                ;;
            -h|--help)
                echo "Usage: $0 [OPTIONS]"
                echo ""
                echo "Options:"
                echo "  --report FILE      Analyze specific test report"
                echo "  --baseline FILE    Compare against baseline metrics"
                echo "  --issues           Generate GitHub issues for failures"
                echo "  -h, --help         Show this help message"
                exit 0
                ;;
            *)
                print_error "Unknown option: $1"
                exit 1
                ;;
        esac
    done

    setup_logging
    print_header "Hardware Test Results Analysis"

    # Find report if not specified
    if [[ -z "${REPORT_FILE}" ]]; then
        if ! find_latest_report; then
            exit 1
        fi
    else
        if [[ ! -f "${REPORT_FILE}" ]]; then
            print_error "Report file not found: ${REPORT_FILE}"
            exit 1
        fi
        print_info "Using specified report: ${REPORT_FILE}"
    fi

    # Parse results
    parse_all_phases
    calculate_metrics

    # Compare against baseline if provided
    if [[ -n "${BASELINE_FILE}" ]]; then
        compare_against_baseline
    fi

    # Generate reports
    generate_analysis_report
    generate_metrics_json

    # Generate GitHub issues if requested
    if [[ "${GENERATE_ISSUES}" == true ]]; then
        generate_github_issues
    fi

    # Print summary
    print_analysis_summary

    # Exit with appropriate code
    if (( ${#FAILURES[@]} > 0 )); then
        exit 1
    fi
    exit 0
}

main "$@"
