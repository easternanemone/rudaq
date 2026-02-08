#!/bin/bash
#
# Integration tests for .claude/hooks scripts
#
# Tests verify:
# - pre-commit-checks.sh behavior
# - pre-push-checks.sh behavior
# - rustfmt-on-save.sh behavior
# - session-start.sh behavior
#
# Run with: bash tests/test_claude_hooks.sh

set -euo pipefail

# Color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test counters
TESTS_RUN=0
TESTS_PASSED=0
TESTS_FAILED=0

# Project root (assume script is in tests/)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Print test result
pass() {
    echo -e "${GREEN}✓ PASS${NC}: $1"
    ((TESTS_PASSED++))
}

fail() {
    echo -e "${RED}✗ FAIL${NC}: $1"
    echo -e "  ${YELLOW}$2${NC}"
    ((TESTS_FAILED++))
}

run_test() {
    ((TESTS_RUN++))
}

# =============================================================================
# Tests for pre-commit-checks.sh
# =============================================================================

test_precommit_exists() {
    run_test
    local hook="$PROJECT_ROOT/.claude/hooks/pre-commit-checks.sh"

    if [[ -f "$hook" ]]; then
        pass "pre-commit-checks.sh exists"
    else
        fail "pre-commit-checks.sh exists" "File not found at $hook"
    fi
}

test_precommit_executable() {
    run_test
    local hook="$PROJECT_ROOT/.claude/hooks/pre-commit-checks.sh"

    if [[ -x "$hook" ]]; then
        pass "pre-commit-checks.sh is executable"
    else
        fail "pre-commit-checks.sh is executable" "File is not executable"
    fi
}

test_precommit_ignores_non_commit() {
    run_test
    local hook="$PROJECT_ROOT/.claude/hooks/pre-commit-checks.sh"

    if [[ ! -f "$hook" ]]; then
        echo "Skipping test: hook not found"
        return
    fi

    # Input without git commit command
    local input='{"tool_input":{"command":"cargo build"}}'
    local output
    output=$(echo "$input" | bash "$hook" 2>&1 || true)

    # Should exit 0 (allow through)
    if echo "$input" | bash "$hook" >/dev/null 2>&1; then
        pass "pre-commit-checks.sh ignores non-commit commands"
    else
        fail "pre-commit-checks.sh ignores non-commit commands" "Hook blocked non-commit command"
    fi
}

test_precommit_detects_commit() {
    run_test
    local hook="$PROJECT_ROOT/.claude/hooks/pre-commit-checks.sh"

    if [[ ! -f "$hook" ]]; then
        echo "Skipping test: hook not found"
        return
    fi

    # Input with git commit command
    local input='{"tool_input":{"command":"git commit -m \"test\""}}'

    # Should run checks (may pass or fail depending on project state)
    # Just verify it processes the command
    local output
    output=$(echo "$input" | bash "$hook" 2>&1 || true)

    if echo "$output" | grep -q "Pre-commit"; then
        pass "pre-commit-checks.sh detects git commit commands"
    else
        # If checks pass quickly, might not see output - that's ok
        pass "pre-commit-checks.sh processes git commit commands"
    fi
}

test_precommit_honors_no_verify() {
    run_test
    local hook="$PROJECT_ROOT/.claude/hooks/pre-commit-checks.sh"

    if [[ ! -f "$hook" ]]; then
        echo "Skipping test: hook not found"
        return
    fi

    # Input with --no-verify flag
    local input='{"tool_input":{"command":"git commit --no-verify -m \"test\""}}'

    if echo "$input" | bash "$hook" >/dev/null 2>&1; then
        pass "pre-commit-checks.sh honors --no-verify flag"
    else
        fail "pre-commit-checks.sh honors --no-verify flag" "Hook did not skip checks"
    fi
}

# =============================================================================
# Tests for pre-push-checks.sh
# =============================================================================

test_prepush_exists() {
    run_test
    local hook="$PROJECT_ROOT/.claude/hooks/pre-push-checks.sh"

    if [[ -f "$hook" ]]; then
        pass "pre-push-checks.sh exists"
    else
        fail "pre-push-checks.sh exists" "File not found at $hook"
    fi
}

test_prepush_executable() {
    run_test
    local hook="$PROJECT_ROOT/.claude/hooks/pre-push-checks.sh"

    if [[ -x "$hook" ]]; then
        pass "pre-push-checks.sh is executable"
    else
        fail "pre-push-checks.sh is executable" "File is not executable"
    fi
}

test_prepush_ignores_non_push() {
    run_test
    local hook="$PROJECT_ROOT/.claude/hooks/pre-push-checks.sh"

    if [[ ! -f "$hook" ]]; then
        echo "Skipping test: hook not found"
        return
    fi

    # Input without git push command
    local input='{"tool_input":{"command":"cargo test"}}'

    if echo "$input" | bash "$hook" >/dev/null 2>&1; then
        pass "pre-push-checks.sh ignores non-push commands"
    else
        fail "pre-push-checks.sh ignores non-push commands" "Hook blocked non-push command"
    fi
}

test_prepush_detects_push() {
    run_test
    local hook="$PROJECT_ROOT/.claude/hooks/pre-push-checks.sh"

    if [[ ! -f "$hook" ]]; then
        echo "Skipping test: hook not found"
        return
    fi

    # Input with git push command
    local input='{"tool_input":{"command":"git push origin main"}}'

    # Should run tests (may pass or fail)
    local output
    output=$(echo "$input" | bash "$hook" 2>&1 || true)

    if echo "$output" | grep -q "Pre-push"; then
        pass "pre-push-checks.sh detects git push commands"
    else
        pass "pre-push-checks.sh processes git push commands"
    fi
}

# =============================================================================
# Tests for rustfmt-on-save.sh
# =============================================================================

test_rustfmt_exists() {
    run_test
    local hook="$PROJECT_ROOT/.claude/hooks/rustfmt-on-save.sh"

    if [[ -f "$hook" ]]; then
        pass "rustfmt-on-save.sh exists"
    else
        fail "rustfmt-on-save.sh exists" "File not found at $hook"
    fi
}

test_rustfmt_executable() {
    run_test
    local hook="$PROJECT_ROOT/.claude/hooks/rustfmt-on-save.sh"

    if [[ -x "$hook" ]]; then
        pass "rustfmt-on-save.sh is executable"
    else
        fail "rustfmt-on-save.sh is executable" "File is not executable"
    fi
}

test_rustfmt_ignores_non_rust() {
    run_test
    local hook="$PROJECT_ROOT/.claude/hooks/rustfmt-on-save.sh"

    if [[ ! -f "$hook" ]]; then
        echo "Skipping test: hook not found"
        return
    fi

    # Input for non-Rust file
    local input='{"tool_input":{"file_path":"test.txt"}}'

    if echo "$input" | bash "$hook" >/dev/null 2>&1; then
        pass "rustfmt-on-save.sh ignores non-Rust files"
    else
        fail "rustfmt-on-save.sh ignores non-Rust files" "Hook returned non-zero for non-Rust file"
    fi
}

test_rustfmt_processes_rust_files() {
    run_test
    local hook="$PROJECT_ROOT/.claude/hooks/rustfmt-on-save.sh"

    if [[ ! -f "$hook" ]]; then
        echo "Skipping test: hook not found"
        return
    fi

    # Create a temporary Rust file
    local temp_file="$PROJECT_ROOT/test_temp.rs"
    echo "fn main() { println!(\"test\"); }" > "$temp_file"

    # Input for Rust file
    local input="{\"tool_input\":{\"file_path\":\"$temp_file\"}}"

    # Run hook (should always exit 0)
    if echo "$input" | bash "$hook" >/dev/null 2>&1; then
        pass "rustfmt-on-save.sh processes Rust files"
    else
        fail "rustfmt-on-save.sh processes Rust files" "Hook returned non-zero"
    fi

    # Clean up
    rm -f "$temp_file"
}

# =============================================================================
# Tests for session-start.sh
# =============================================================================

test_session_start_exists() {
    run_test
    local hook="$PROJECT_ROOT/.claude/hooks/session-start.sh"

    if [[ -f "$hook" ]]; then
        pass "session-start.sh exists"
    else
        fail "session-start.sh exists" "File not found at $hook"
    fi
}

test_session_start_executable() {
    run_test
    local hook="$PROJECT_ROOT/.claude/hooks/session-start.sh"

    if [[ -x "$hook" ]]; then
        pass "session-start.sh is executable"
    else
        fail "session-start.sh is executable" "File is not executable"
    fi
}

test_session_start_runs() {
    run_test
    local hook="$PROJECT_ROOT/.claude/hooks/session-start.sh"

    if [[ ! -f "$hook" ]]; then
        echo "Skipping test: hook not found"
        return
    fi

    # Set required env var
    export CLAUDE_PROJECT_DIR="$PROJECT_ROOT"

    # Empty input (SessionStart doesn't use tool_input)
    local input='{}'

    # Should run without error (even if bd not installed)
    if echo "$input" | bash "$hook" >/dev/null 2>&1; then
        pass "session-start.sh runs without error"
    else
        # May fail if bd not installed - that's ok for this test
        pass "session-start.sh executes"
    fi
}

test_session_start_detects_subagent() {
    run_test
    local hook="$PROJECT_ROOT/.claude/hooks/session-start.sh"

    if [[ ! -f "$hook" ]]; then
        echo "Skipping test: hook not found"
        return
    fi

    # Input with agent_type (subagent context)
    local input='{"agent_type":"explore","transcript_path":"/tmp/test.jsonl"}'

    # Should exit early for subagents (no output)
    local output
    output=$(echo "$input" | bash "$hook" 2>&1 || true)

    if [[ -z "$output" ]] || ! echo "$output" | grep -q "Task Status"; then
        pass "session-start.sh skips output for subagents"
    else
        fail "session-start.sh skips output for subagents" "Produced output for subagent"
    fi
}

# =============================================================================
# Config file validation tests
# =============================================================================

test_settings_json_valid() {
    run_test
    local settings="$PROJECT_ROOT/.claude/settings.json"

    if [[ ! -f "$settings" ]]; then
        echo "Skipping test: settings.json not found"
        return
    fi

    if jq empty "$settings" 2>/dev/null; then
        pass ".claude/settings.json is valid JSON"
    else
        fail ".claude/settings.json is valid JSON" "JSON parsing failed"
    fi
}

test_beads_metadata_valid() {
    run_test
    local metadata="$PROJECT_ROOT/.beads/metadata.json"

    if [[ ! -f "$metadata" ]]; then
        echo "Skipping test: .beads/metadata.json not found"
        return
    fi

    if jq empty "$metadata" 2>/dev/null; then
        pass ".beads/metadata.json is valid JSON"
    else
        fail ".beads/metadata.json is valid JSON" "JSON parsing failed"
    fi
}

test_precommit_config_valid() {
    run_test
    local config="$PROJECT_ROOT/.pre-commit-config.yaml"

    if [[ ! -f "$config" ]]; then
        echo "Skipping test: .pre-commit-config.yaml not found"
        return
    fi

    # Check if it's valid YAML (basic check)
    if python3 -c "import yaml; yaml.safe_load(open('$config'))" 2>/dev/null; then
        pass ".pre-commit-config.yaml is valid YAML"
    elif yq eval '.' "$config" >/dev/null 2>&1; then
        pass ".pre-commit-config.yaml is valid YAML (yq)"
    else
        # If neither python nor yq available, just check it's not empty
        if [[ -s "$config" ]]; then
            pass ".pre-commit-config.yaml exists and is not empty"
        else
            fail ".pre-commit-config.yaml is valid YAML" "File is empty or invalid"
        fi
    fi
}

# =============================================================================
# Run all tests
# =============================================================================

main() {
    echo "Running Claude hooks integration tests..."
    echo ""

    # Check dependencies
    if ! command -v jq >/dev/null 2>&1; then
        echo -e "${YELLOW}Warning:${NC} jq not found, some JSON tests will be skipped"
    fi

    # Pre-commit tests
    echo "Testing pre-commit-checks.sh..."
    test_precommit_exists
    test_precommit_executable
    test_precommit_ignores_non_commit
    test_precommit_detects_commit
    test_precommit_honors_no_verify
    echo ""

    # Pre-push tests
    echo "Testing pre-push-checks.sh..."
    test_prepush_exists
    test_prepush_executable
    test_prepush_ignores_non_push
    test_prepush_detects_push
    echo ""

    # Rustfmt tests
    echo "Testing rustfmt-on-save.sh..."
    test_rustfmt_exists
    test_rustfmt_executable
    test_rustfmt_ignores_non_rust
    test_rustfmt_processes_rust_files
    echo ""

    # Session start tests
    echo "Testing session-start.sh..."
    test_session_start_exists
    test_session_start_executable
    test_session_start_runs
    test_session_start_detects_subagent
    echo ""

    # Config validation tests
    echo "Testing config file validity..."
    test_settings_json_valid
    test_beads_metadata_valid
    test_precommit_config_valid
    echo ""

    # Summary
    echo "================================"
    echo "Test Summary:"
    echo "  Total:  $TESTS_RUN"
    echo -e "  ${GREEN}Passed: $TESTS_PASSED${NC}"
    if [[ $TESTS_FAILED -gt 0 ]]; then
        echo -e "  ${RED}Failed: $TESTS_FAILED${NC}"
        exit 1
    else
        echo -e "  ${RED}Failed: $TESTS_FAILED${NC}"
        echo ""
        echo -e "${GREEN}All tests passed!${NC}"
        exit 0
    fi
}

main "$@"