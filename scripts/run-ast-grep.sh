#!/usr/bin/env bash
# Run ast-grep using either `ast-grep` (npm) or `sg` (cargo), whichever exists.

set -euo pipefail

if command -v ast-grep >/dev/null 2>&1; then
  exec ast-grep "$@"
fi

if command -v sg >/dev/null 2>&1 && sg --version 2>&1 | grep -q 'ast-grep'; then
  exec sg "$@"
fi

echo "Error: ast-grep not found. Install one of:" >&2
echo "  npm install -g @ast-grep/cli" >&2
echo "  cargo install ast-grep" >&2
exit 127
