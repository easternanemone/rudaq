#!/bin/bash
# Tier 1: Auto-format .rs files after Edit/Write
# PostToolUse hook — non-blocking (always exits 0)

set +e

input=$(cat)

file_path=$(echo "$input" | jq -r '.tool_input.file_path // empty')

# Skip if not a .rs file
if [[ -z "$file_path" || "$file_path" != *.rs ]]; then
  exit 0
fi

# Skip if file doesn't exist (deleted or moved)
if [[ ! -f "$file_path" ]]; then
  exit 0
fi

# Format just this file (~100ms)
if command -v rustfmt &>/dev/null; then
  if ! rustfmt "$file_path" 2>/dev/null; then
    echo "rustfmt warning: failed to format $file_path (syntax error?)" >&2
  fi
fi

exit 0
