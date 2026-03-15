#!/bin/bash
# Tier 3: Full quality gate before push.
# PreToolUse hook for Bash — blocks push if fmt/clippy/tests fail.

set +e

input=$(cat)
command=$(echo "$input" | jq -r '.tool_input.command // empty')

# Only intercept git push commands.
if ! echo "$command" | grep -qE '(^|\s|&&|\|)git\s+push(\s|$)'; then
  exit 0
fi

echo "Pre-push: Running cargo fmt --all --check..." >&2
fmt_output=$(cargo fmt --all --check 2>&1)
fmt_exit=$?

if [[ $fmt_exit -ne 0 ]]; then
  escaped_fmt=$(echo "$fmt_output" | jq -R -s .)
  cat <<JSON
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"cargo fmt --all --check failed. Fix formatting before pushing:\n${escaped_fmt}"}}
JSON
  exit 0
fi

echo "Pre-push: Running cargo clippy..." >&2
clippy_output=$(cargo clippy --workspace --all-targets --exclude ui --exclude comedi-sys --exclude driver-comedi -- -D warnings 2>&1)
clippy_exit=$?

if [[ $clippy_exit -ne 0 ]]; then
  if [[ $(echo "$clippy_output" | wc -l) -gt 100 ]]; then
    clippy_output="[...truncated...]\n$(echo "$clippy_output" | tail -100)"
  fi
  escaped_clippy=$(echo "$clippy_output" | jq -R -s .)
  cat <<JSON
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"cargo clippy failed. Fix warnings before pushing:\n${escaped_clippy}"}}
JSON
  exit 0
fi

echo "Pre-push: Running tests (excluding ui and integration-tests)..." >&2
if command -v cargo-nextest &>/dev/null; then
  test_output=$(cargo nextest run --workspace --exclude ui --exclude integration-tests --profile ci 2>&1)
  test_exit=$?
else
  test_output=$(cargo test --workspace --exclude ui --exclude integration-tests 2>&1)
  test_exit=$?
fi

if [[ $test_exit -ne 0 ]]; then
  if [[ $(echo "$test_output" | wc -l) -gt 100 ]]; then
    test_output="[...truncated...]\n$(echo "$test_output" | tail -100)"
  fi
  escaped_test=$(echo "$test_output" | jq -R -s .)
  cat <<JSON
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"Tests failed. Fix before pushing:\n${escaped_test}"}}
JSON
  exit 0
fi

echo "Pre-push: Full quality gate passed." >&2
exit 0
