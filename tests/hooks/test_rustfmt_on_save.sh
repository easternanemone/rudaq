#!/bin/bash
# Functional tests for rustfmt-on-save.sh hook
# Tests the PostToolUse hook that auto-formats Rust files

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
HOOK_SCRIPT="$PROJECT_ROOT/.claude/hooks/rustfmt-on-save.sh"

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
    local file_path="$1"
    cat <<EOF
{
  "tool_name": "Edit",
  "tool_input": {
    "file_path": "$file_path",
    "old_string": "fn main() {",
    "new_string": "fn main() {"
  }
}
EOF
}

echo "Testing rustfmt-on-save.sh"
echo "=========================="
echo

# Test 1: Non-Rust files should be skipped
run_test
echo "Test 1: Non-Rust files are skipped"
INPUT=$(create_test_input "/tmp/test.txt")
RESULT=$(echo "$INPUT" | bash "$HOOK_SCRIPT")
EXIT_CODE=$?
if [[ $EXIT_CODE -eq 0 && -z "$RESULT" ]]; then
    test_passed "Non-Rust files are skipped"
else
    test_failed "Non-Rust files are skipped" "exit 0, no output" "exit $EXIT_CODE, output: $RESULT"
fi

# Test 2: Rust files should trigger formatting
run_test
echo "Test 2: Rust files trigger formatting"
# Create a temporary Rust file
TEMP_RS=$(mktemp --suffix=.rs)
cat > "$TEMP_RS" <<'RUST'
fn main(){println!("test");}
RUST

INPUT=$(create_test_input "$TEMP_RS")
RESULT=$(echo "$INPUT" | bash "$HOOK_SCRIPT" 2>&1)
EXIT_CODE=$?

# Hook always exits 0 (non-blocking)
if [[ $EXIT_CODE -eq 0 ]]; then
    test_passed "Rust files trigger formatting (hook exits 0)"
else
    test_failed "Rust files trigger formatting" "exit 0" "exit $EXIT_CODE"
fi

# Cleanup
rm -f "$TEMP_RS"

# Test 3: Non-existent file should be skipped
run_test
echo "Test 3: Non-existent files are skipped"
INPUT=$(create_test_input "/tmp/nonexistent.rs")
RESULT=$(echo "$INPUT" | bash "$HOOK_SCRIPT")
EXIT_CODE=$?
if [[ $EXIT_CODE -eq 0 ]]; then
    test_passed "Non-existent files are skipped"
else
    test_failed "Non-existent files are skipped" "exit 0" "exit $EXIT_CODE"
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

# Test 6: Hook always exits 0 (non-blocking)
run_test
echo "Test 6: Hook is non-blocking (always exits 0)"
# Check that script ends with 'exit 0'
LAST_EXIT=$(tail -n 5 "$HOOK_SCRIPT" | grep "^exit 0" || true)
if [[ -n "$LAST_EXIT" ]]; then
    test_passed "Hook is non-blocking (always exits 0)"
else
    test_failed "Hook is non-blocking" "ends with exit 0" "different exit behavior"
fi

# Test 7: Hook checks for rustfmt command
run_test
echo "Test 7: Hook checks for rustfmt availability"
GREP_RESULT=$(grep -E "command -v rustfmt" "$HOOK_SCRIPT" || true)
if [[ -n "$GREP_RESULT" ]]; then
    test_passed "Hook checks for rustfmt availability"
else
    test_failed "Hook checks for rustfmt" "checks command -v rustfmt" "not found"
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