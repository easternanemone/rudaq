#!/usr/bin/env bash
set -euo pipefail

if ! command -v bd >/dev/null 2>&1; then
  echo "error: bd is not installed or not on PATH" >&2
  exit 1
fi

if ! command -v git >/dev/null 2>&1; then
  echo "error: git is required" >&2
  exit 1
fi

# --path-format=absolute requires Git >= 2.31; fallback resolves via cd.
common_git_dir="$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null || git rev-parse --git-common-dir)"
common_git_dir="$(cd "$common_git_dir" && pwd -P)"
common_root="$(cd "$common_git_dir/.." && pwd -P)"
canonical_db="$common_root/.beads/beads.db"

if [[ "${1:-}" == "--print-db" ]]; then
  echo "$canonical_db"
  exit 0
fi

if [[ "${1:-}" == "--print-root" ]]; then
  echo "$common_root"
  exit 0
fi

if [[ ! -f "$canonical_db" ]]; then
  echo "error: canonical beads DB not found at $canonical_db" >&2
  exit 1
fi

exec bd --no-daemon --no-auto-import --db "$canonical_db" "$@"
