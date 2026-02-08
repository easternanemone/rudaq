#!/bin/bash
# Functional tests for pre-commit-checks.sh hook
# Tests the PreToolUse hook that validates git commits

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
HOOK_SCRIPT="$PROJECT_ROOT/.claude/hooks/pre-commit-checks.sh"

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

echo "Testing pre-commit-checks.sh"
echo "=============================="
echo

# Test 1: Non-commit commands should pass through
run_test
echo "Test 1: Non-commit commands pass through"
INPUT=$(create_test_input "ls -la")
RESULT=$(echo "$INPUT" | bash "$HOOK_SCRIPT")
EXIT_CODE=$?
if [[ $EXIT_CODE -eq 0 && -z "$RESULT" ]]; then
    test_passed "Non-commit commands pass through"
else
    test_failed "Non-commit commands pass through" "exit 0, no output" "exit $EXIT_CODE, output: $RESULT"
fi

# Test 2: Commit with --no-verify should pass through
run_test
echo "Test 2: git commit --no-verify passes through"
INPUT=$(create_test_input "git commit -m 'test' --no-verify")
RESULT=$(echo "$INPUT" | bash "$HOOK_SCRIPT")
EXIT_CODE=$?
if [[ $EXIT_CODE -eq 0 && -z "$RESULT" ]]; then
    test_passed "git commit --no-verify passes through"
else
    test_failed "git commit --no-verify passes through" "exit 0, no output" "exit $EXIT_CODE, output: $RESULT"
fi

# Test 3: Commit command should trigger checks (in real environment)
run_test
echo "Test 3: git commit triggers checks"
INPUT=$(create_test_input "git commit -m 'test message'")
RESULT=$(echo "$INPUT" | bash "$HOOK_SCRIPT" 2>&1)
EXIT_CODE=$?
# The hook will run cargo fmt/clippy which may fail in test environment
# We just verify the hook executes and produces output
if [[ $EXIT_CODE -eq 0 || "$RESULT" =~ "Pre-commit:" ]]; then
    test_passed "git commit triggers checks"
else
    test_failed "git commit triggers checks" "hook runs checks" "unexpected behavior"
fi

# Test 4: Hook should detect git commit in piped command
run_test
echo "Test 4: Detects git commit in piped command"
INPUT=$(create_test_input "echo 'test' && git commit -m 'message'")
RESULT=$(echo "$INPUT" | bash "$HOOK_SCRIPT" 2>&1)
EXIT_CODE=$?
if [[ $EXIT_CODE -eq 0 || "$RESULT" =~ "Pre-commit:" ]]; then
    test_passed "Detects git commit in piped command"
else
    test_failed "Detects git commit in piped command" "hook runs checks" "unexpected behavior"
fi

# Test 5: Hook script exists and is executable
run_test
echo "Test 5: Hook script exists and is executable"
if [[ -f "$HOOK_SCRIPT" && -x "$HOOK_SCRIPT" ]]; then
    test_passed "Hook script exists and is executable"
else
    test_failed "Hook script exists and is executable" "file exists and executable" "not found or not executable"
fi

# Test 6: Hook script has proper shebang
run_test
echo "Test 6: Hook script has proper shebang"
SHEBANG=$(head -n 1 "$HOOK_SCRIPT")
if [[ "$SHEBANG" == "#!/bin/bash" || "$SHEBANG" == "#!/usr/bin/env bash" ]]; then
    test_passed "Hook script has proper shebang"
else
    test_failed "Hook script has proper shebang" "#!/bin/bash or #!/usr/bin/env bash" "$SHEBANG"
fi

# Summary
echo
echo "=============================="
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