#!/bin/bash
# Quality gate: runs fmt + clippy + tests before bd close
# PreToolUse hook for Bash — blocks bd close if any check fails

set +e

input=$(cat)

command=$(echo "$input" | jq -r '.tool_input.command // empty')

# Only intercept bd close commands
if ! echo "$command" | grep -qE '(^|\s|&&|\|)bd\s+close(\s|$)'; then
  exit 0
fi

echo "Quality gate: Running cargo fmt --all..." >&2
cargo fmt --all 2>/dev/null

echo "Quality gate: Checking formatting..." >&2
fmt_output=$(cargo fmt --all --check 2>&1)
fmt_exit=$?

if [[ $fmt_exit -ne 0 ]]; then
  escaped_fmt=$(echo "$fmt_output" | jq -R -s .)
  cat <<EOF
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"cargo fmt --all --check failed after auto-fix attempt:\n${escaped_fmt}"}}
EOF
  exit 0
fi

echo "Quality gate: Running cargo clippy..." >&2
clippy_output=$(cargo clippy --workspace --all-targets -- -D warnings 2>&1)
clippy_exit=$?

if [[ $clippy_exit -ne 0 ]]; then
  if [[ $(echo "$clippy_output" | wc -l) -gt 100 ]]; then
    clippy_output="[...truncated...]\n$(echo "$clippy_output" | tail -100)"
  fi
  escaped_clippy=$(echo "$clippy_output" | jq -R -s .)
  cat <<EOF
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"cargo clippy failed. Fix warnings before closing:\n${escaped_clippy}"}}
EOF
  exit 0
fi

echo "Quality gate: Running tests..." >&2
if command -v cargo-nextest &>/dev/null; then
  test_output=$(cargo nextest run --workspace --exclude ui 2>&1)
  test_exit=$?
else
  test_output=$(cargo test --workspace --exclude ui 2>&1)
  test_exit=$?
fi

if [[ $test_exit -ne 0 ]]; then
  if [[ $(echo "$test_output" | wc -l) -gt 100 ]]; then
    test_output="[...truncated...]\n$(echo "$test_output" | tail -100)"
  fi
  escaped_test=$(echo "$test_output" | jq -R -s .)
  cat <<EOF
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"Tests failed. Fix before closing:\n${escaped_test}"}}
EOF
  exit 0
fi

echo "Quality gate: All checks passed." >&2
exit 0
