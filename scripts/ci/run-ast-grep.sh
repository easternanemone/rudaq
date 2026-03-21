#!/usr/bin/env bash
# Run ast-grep using either `ast-grep` (npm) or `sg` (cargo), whichever exists.
#
# For pre-commit hooks: `scan` command only fails on error-severity rules.
# Warnings and help-level diagnostics are displayed but don't block the commit.

set -euo pipefail

# Find the ast-grep binary
SG=""
if command -v ast-grep >/dev/null 2>&1; then
  SG="ast-grep"
elif command -v sg >/dev/null 2>&1 && sg --version 2>&1 | grep -q 'ast-grep'; then
  SG="sg"
else
  echo "Error: ast-grep not found. Install one of:" >&2
  echo "  npm install -g @ast-grep/cli" >&2
  echo "  cargo install ast-grep" >&2
  exit 127
fi

# For `scan` command: run and display all findings, but only fail on errors.
# This makes the pre-commit hook block on error-severity rules (panic, etc.)
# while allowing warning/help-severity rules (unwrap, unsafe audit) to pass.
if [[ "${1:-}" == "scan" ]]; then
  shift

  # Capture exit status separately so tool/config failures aren't
  # silently swallowed. Exit code 1 from sg scan means "findings exist"
  # (expected); any other non-zero exit indicates a tool failure.
  SCAN_EXIT=0
  OUTPUT=$("$SG" scan "$@" 2>&1) || SCAN_EXIT=$?
  if [[ -n "$OUTPUT" ]]; then
    echo "$OUTPUT"
  fi

  # Tool execution error (not just findings) — fail hard
  if [[ $SCAN_EXIT -gt 1 ]]; then
    echo ""
    echo "ast-grep: tool execution failed (exit code $SCAN_EXIT)"
    exit "$SCAN_EXIT"
  fi

  # Only fail if error-severity findings exist
  if echo "$OUTPUT" | grep -q 'error\['; then
    echo ""
    echo "ast-grep: error-severity findings detected (fix required)"
    exit 1
  fi
  exit 0
fi

# For other commands (run, test, etc.), pass through directly
exec "$SG" "$@"
