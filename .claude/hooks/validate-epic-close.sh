#!/bin/bash
# Hook: Validate all epic children are complete before closing
# Prevents closing an epic when children are still open

set -euo pipefail

TOOL_INPUT="${CLAUDE_TOOL_INPUT:-}"

# Only check Bash commands containing "bd close"
if ! echo "$TOOL_INPUT" | jq -e '.command' >/dev/null 2>&1; then
  exit 0
fi

COMMAND=$(echo "$TOOL_INPUT" | jq -r '.command // ""')

# Check if this is a bd close command
if ! echo "$COMMAND" | grep -qE 'bd\s+close'; then
  exit 0
fi

# Extract the ID being closed (handles: bd close ID, bd close ID && ..., etc.)
CLOSE_ID=$(echo "$COMMAND" | sed -E 's/.*bd\s+close\s+([A-Za-z0-9._-]+).*/\1/')

if [ -z "$CLOSE_ID" ]; then
  exit 0
fi

# Check if this is an epic by looking for children
CHILDREN=$(bd show "$CLOSE_ID" --json 2>/dev/null | jq -r '.[0].children // empty' 2>/dev/null || echo "")

if [ -z "$CHILDREN" ] || [ "$CHILDREN" = "null" ]; then
  # Not an epic or no children, allow close
  exit 0
fi

# This is an epic - check if all children are complete
INCOMPLETE=$(bd list --json 2>/dev/null | jq -r --arg epic "$CLOSE_ID" '
  [.[] | select(.parent == $epic and .status != "done" and .status != "closed")] | length
' 2>/dev/null || echo "0")

if [ "$INCOMPLETE" != "0" ] && [ "$INCOMPLETE" != "" ]; then
  # Get list of incomplete children for the error message
  INCOMPLETE_LIST=$(bd list --json 2>/dev/null | jq -r --arg epic "$CLOSE_ID" '
    [.[] | select(.parent == $epic and .status != "done" and .status != "closed")] | .[] | "\(.id) (\(.status))"
  ' 2>/dev/null | tr '\n' ', ' | sed 's/,$//')

  cat << EOF
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"Cannot close epic '$CLOSE_ID' - has $INCOMPLETE incomplete children: $INCOMPLETE_LIST. Mark all children as done first."}}
EOF
  exit 0
fi

# All children complete, allow close
exit 0
