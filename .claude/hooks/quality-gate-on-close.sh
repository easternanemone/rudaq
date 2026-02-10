#!/bin/bash
# Tier 2: Lightweight quality gate before bd close.
# PreToolUse hook for Bash — quick structural checks only.

set +e

input=$(cat)
command=$(echo "$input" | jq -r '.tool_input.command // empty')

# Only intercept bd close commands.
if ! echo "$command" | grep -qE '(^|\s|&&|\|)bd\s+close(\s|$)'; then
  exit 0
fi

echo "Quality gate: Running cargo fmt --all --check..." >&2
fmt_output=$(cargo fmt --all --check 2>&1)
fmt_exit=$?

if [[ $fmt_exit -ne 0 ]]; then
  escaped_fmt=$(echo "$fmt_output" | jq -R -s .)
  cat <<JSON
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"cargo fmt --all --check failed. Fix formatting before closing:\n${escaped_fmt}"}}
JSON
  exit 0
fi

if command -v sg &>/dev/null; then
  echo "Quality gate: Running ast-grep structural lint..." >&2
  sg_output=$(sg scan --report-style short 2>&1)
  sg_exit=$?

  if [[ $sg_exit -ne 0 ]]; then
    error_lines=$(echo "$sg_output" | grep 'error\[')
    if [[ -n "$error_lines" ]]; then
      escaped_sg=$(echo "$error_lines" | jq -R -s .)
      cat <<JSON
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"ast-grep found error-level issues:\n${escaped_sg}"}}
JSON
      exit 0
    fi
  fi
fi

echo "Quality gate: Lightweight checks passed." >&2
exit 0
