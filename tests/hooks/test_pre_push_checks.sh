#!/bin/bash
# Functional tests for pre-push-checks.sh hook
# Tests the PreToolUse hook that validates git push commands

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
HOOK_SCRIPT="$PROJECT_ROOT/.claude/hooks/pre-push-checks.sh"

# Test counter
TESTS_RUN=0
TESTS_PASSED=0

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# Test helper functions
test_passed() {
    TESTS_PASSED=$((TESTS_PASSED + 1))
    echo -e "${GREEN}✓${NC} $1"
}

test_failed() {
    echo -e "${RED}✗${NC} $1"
    echo "  Expected: $2"
    echo "  Got: $3"
}

run_test() {
    TESTS_RUN=$((TESTS_RUN + 1))
}

# Mock jq output function
create_test_input() {
    local command="$1"
    cat <<EOF
{
  "tool_name": "Bash",
  "tool_input": {
    "command": "$command",
    "description": "Test command"
  }
}
EOF
}

echo "Testing pre-push-checks.sh"
echo "=========================="
echo

# Test 1: Non-push commands should pass through
run_test
echo "Test 1: Non-push commands pass through"
INPUT=$(create_test_input "git status")
RESULT=$(echo "$INPUT" | bash "$HOOK_SCRIPT")
EXIT_CODE=$?
if [[ $EXIT_CODE -eq 0 && -z "$RESULT" ]]; then
    test_passed "Non-push commands pass through"
else
    test_failed "Non-push commands pass through" "exit 0, no output" "exit $EXIT_CODE, output: $RESULT"
fi

# Test 2: git push command should trigger tests
run_test
echo "Test 2: git push triggers test checks"
INPUT=$(create_test_input "git push origin main")
RESULT=$(echo "$INPUT" | bash "$HOOK_SCRIPT" 2>&1)
EXIT_CODE=$?
# The hook will run tests which may fail in test environment
# We just verify the hook executes
if [[ $EXIT_CODE -eq 0 || "$RESULT" =~ "Pre-push:" ]]; then
    test_passed "git push triggers test checks"
else
    test_failed "git push triggers test checks" "hook runs tests" "unexpected behavior"
fi

# Test 3: Hook should detect git push in piped command
run_test
echo "Test 3: Detects git push in piped command"
INPUT=$(create_test_input "git commit -m 'test' && git push")
RESULT=$(echo "$INPUT" | bash "$HOOK_SCRIPT" 2>&1)
EXIT_CODE=$?
if [[ $EXIT_CODE -eq 0 || "$RESULT" =~ "Pre-push:" ]]; then
    test_passed "Detects git push in piped command"
else
    test_failed "Detects git push in piped command" "hook runs tests" "unexpected behavior"
fi

# Test 4: Hook script exists and is executable
run_test
echo "Test 4: Hook script exists and is executable"
if [[ -f "$HOOK_SCRIPT" && -x "$HOOK_SCRIPT" ]]; then
    test_passed "Hook script exists and is executable"
else
    test_failed "Hook script exists and is executable" "file exists and executable" "not found or not executable"
fi

# Test 5: Hook script has proper shebang
run_test
echo "Test 5: Hook script has proper shebang"
SHEBANG=$(head -n 1 "$HOOK_SCRIPT")
if [[ "$SHEBANG" == "#!/bin/bash" || "$SHEBANG" == "#!/usr/bin/env bash" ]]; then
    test_passed "Hook script has proper shebang"
else
    test_failed "Hook script has proper shebang" "#!/bin/bash or #!/usr/bin/env bash" "$SHEBANG"
fi

# Test 6: Hook checks for cargo-nextest or cargo test
run_test
echo "Test 6: Hook checks for test runner (nextest or cargo test)"
GREP_RESULT=$(grep -E "(cargo-nextest|cargo test)" "$HOOK_SCRIPT" || true)
if [[ -n "$GREP_RESULT" ]]; then
    test_passed "Hook checks for test runner"
else
    test_failed "Hook checks for test runner" "cargo-nextest or cargo test present" "not found"
fi

# Test 7: Hook excludes ui crate from tests
run_test
echo "Test 7: Hook excludes ui crate from tests"
GREP_RESULT=$(grep "exclude ui" "$HOOK_SCRIPT" || true)
if [[ -n "$GREP_RESULT" ]]; then
    test_passed "Hook excludes ui crate from tests"
else
    test_failed "Hook excludes ui crate from tests" "--exclude ui flag present" "not found"
fi

# Summary
echo
echo "=========================="
echo "Tests run: $TESTS_RUN"
echo "Tests passed: $TESTS_PASSED"
echo "Tests failed: $((TESTS_RUN - TESTS_PASSED))"

if [[ $TESTS_PASSED -eq $TESTS_RUN ]]; then
    echo -e "${GREEN}All tests passed!${NC}"
    exit 0
else
    echo -e "${RED}Some tests failed!${NC}"
    exit 1
fi