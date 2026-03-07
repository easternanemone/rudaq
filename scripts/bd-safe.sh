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

# Resolve the canonical repo root via git (works from any worktree).
# --path-format=absolute requires Git >= 2.31; fallback resolves via cd.
common_git_dir="$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null || git rev-parse --git-common-dir)"
common_git_dir="$(cd "$common_git_dir" && pwd -P)"
common_root="$(cd "$common_git_dir/.." && pwd -P)"

# Discover the canonical DB path from bd itself (supports SQLite and Dolt backends).
# The pipeline is guarded with `|| true` so that failures in `bd where` or the parser
# don't abort the script under `set -euo pipefail` — the fallback probe below handles it.
canonical_db="$(
  cd "$common_root" && bd where --json 2>/dev/null | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
    path = data.get("database_path", "")
    if isinstance(path, str):
        print(path)
except Exception:
    pass
' || true
)"

# Fallback: if bd where fails, probe known locations in priority order.
if [[ -z "$canonical_db" ]]; then
  if [[ -d "$common_root/.beads/dolt" ]]; then
    canonical_db="$common_root/.beads/dolt"
  elif [[ -f "$common_root/.beads/beads.db" ]]; then
    canonical_db="$common_root/.beads/beads.db"
  else
    echo "error: cannot find canonical beads DB under $common_root/.beads/" >&2
    echo "  tried: bd where --json, .beads/dolt, .beads/beads.db" >&2
    exit 1
  fi
fi

if [[ "${1:-}" == "--print-db" ]]; then
  echo "$canonical_db"
  exit 0
fi

if [[ "${1:-}" == "--print-root" ]]; then
  echo "$common_root"
  exit 0
fi

if [[ ! -e "$canonical_db" ]]; then
  echo "error: canonical beads DB not found at $canonical_db" >&2
  exit 1
fi

exec bd --no-daemon --no-auto-import --db "$canonical_db" "$@"
