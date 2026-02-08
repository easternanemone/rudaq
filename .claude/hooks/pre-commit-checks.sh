#!/bin/bash
# Tier 2: Lint gate before commit
# PreToolUse hook for Bash — blocks commit if fmt or clippy fails

set +e

input=$(cat)

command=$(echo "$input" | jq -r '.tool_input.command // empty')

# Only intercept git commit commands
if ! echo "$command" | grep -qE '(^|\s|&&|\|)git\s+commit(\s|$)'; then
  exit 0
fi

# Skip if --no-verify is explicitly requested (user override)
if echo "$command" | grep -q -- '--no-verify'; then
  exit 0
fi

echo "Pre-commit: Running cargo fmt check..." >&2
fmt_output=$(cargo fmt --all --check 2>&1)
fmt_exit=$?

if [[ $fmt_exit -ne 0 ]]; then
  escaped_fmt=$(echo "$fmt_output" | jq -R -s .)
  cat <<EOF
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"cargo fmt --all --check failed. Fix formatting before committing:\n${escaped_fmt}"}}
EOF
  exit 0
fi

echo "Pre-commit: Running ast-grep structural lint..." >&2
sg_output=$(sg scan --report-style short 2>&1)
sg_exit=$?

if [[ $sg_exit -ne 0 ]]; then
  # Only block on error-severity findings (warnings are informational)
  error_lines=$(echo "$sg_output" | grep 'error\[')
  if [[ -n "$error_lines" ]]; then
    escaped_sg=$(echo "$error_lines" | jq -R -s .)
    cat <<EOF
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"ast-grep found error-level issues in production code:\n${escaped_sg}"}}
EOF
    exit 0
  fi
fi

echo "Pre-commit: Running cargo test..." >&2
test_output=$(cargo test --workspace --all-features 2>&1)
test_exit=$?

if [[ $test_exit -ne 0 ]]; then
  escaped_test=$(echo "$test_output" | jq -R -s .)
  cat <<EOF
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"cargo test failed. Fix tests before committing:\n${escaped_test}"}}
EOF
  exit 0
fi

echo "Pre-commit: Running cargo clippy..." >&2
clippy_output=$(cargo clippy --workspace --all-targets --all-features -- -D warnings 2>&1)
clippy_exit=$?

if [[ $clippy_exit -ne 0 ]]; then
  escaped_clippy=$(echo "$clippy_output" | jq -R -s .)
  cat <<EOF
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"cargo clippy failed. Fix warnings before committing:\n${escaped_clippy}"}}
EOF
  exit 0
fi

echo "Pre-commit: All checks passed." >&2
exit 0
