#!/bin/bash
# Master test runner for all hook tests

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo "========================================"
echo "Running All Hook Tests"
echo "========================================"
echo

TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

run_test_suite() {
    local test_script="$1"
    local test_name=$(basename "$test_script" .sh)

    echo -e "${YELLOW}Running $test_name...${NC}"

    if bash "$test_script"; then
        echo -e "${GREEN}✓ $test_name passed${NC}"
        echo
        return 0
    else
        echo -e "${RED}✗ $test_name failed${NC}"
        echo
        return 1
    fi
}

# Run all test suites
TEST_SUITES=(
    "test_pre_commit_checks.sh"
    "test_pre_push_checks.sh"
    "test_rustfmt_on_save.sh"
    "test_session_start.sh"
)

for suite in "${TEST_SUITES[@]}"; do
    if [[ -f "$SCRIPT_DIR/$suite" ]]; then
        if run_test_suite "$SCRIPT_DIR/$suite"; then
            PASSED_TESTS=$((PASSED_TESTS + 1))
        else
            FAILED_TESTS=$((FAILED_TESTS + 1))
        fi
        TOTAL_TESTS=$((TOTAL_TESTS + 1))
    else
        echo -e "${YELLOW}Warning: $suite not found, skipping${NC}"
    fi
done

# Summary
echo "========================================"
echo "Test Summary"
echo "========================================"
echo "Total test suites: $TOTAL_TESTS"
echo -e "${GREEN}Passed: $PASSED_TESTS${NC}"
if [[ $FAILED_TESTS -gt 0 ]]; then
    echo -e "${RED}Failed: $FAILED_TESTS${NC}"
fi
echo

if [[ $FAILED_TESTS -eq 0 ]]; then
    echo -e "${GREEN}All test suites passed!${NC}"
    exit 0
else
    echo -e "${RED}Some test suites failed!${NC}"
    exit 1
fi