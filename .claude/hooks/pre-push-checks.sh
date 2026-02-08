#!/bin/bash
# Tier 3: Test gate before push
# PreToolUse hook for Bash — blocks push if tests fail

set +e

input=$(cat)

command=$(echo "$input" | jq -r '.tool_input.command // empty')

# Only intercept git push commands
if ! echo "$command" | grep -qE '(^|\s|&&|\|)git\s+push(\s|$)'; then
  exit 0
fi

echo "Pre-push: Running tests..." >&2

# Prefer cargo nextest, fall back to cargo test
if command -v cargo-nextest &>/dev/null; then
  test_output=$(cargo nextest run --workspace --all-features --exclude ui 2>&1)
  test_exit=$?
else
  test_output=$(cargo test --workspace --all-features --exclude ui 2>&1)
  test_exit=$?
fi

if [[ $test_exit -ne 0 ]]; then
  # Truncate output if very long (keep last 100 lines for relevance)
  if [[ $(echo "$test_output" | wc -l) -gt 100 ]]; then
    test_output="[...truncated...]\n$(echo "$test_output" | tail -100)"
  fi
  escaped_output=$(echo "$test_output" | jq -R -s .)
  cat <<EOF
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"Tests failed. Fix before pushing:\n${escaped_output}"}}
EOF
  exit 0
fi

echo "Pre-push: All tests passed." >&2
exit 0
