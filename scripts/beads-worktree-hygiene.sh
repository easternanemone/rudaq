#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/beads-worktree-hygiene.sh status
  scripts/beads-worktree-hygiene.sh cleanup [--apply]

Commands:
  status    Show canonical DB location and whether this worktree has local stale artifacts.
  cleanup   Move local worktree-only Beads runtime artifacts to a timestamped backup folder.
            Dry-run by default; pass --apply to execute.
EOF
}

if ! command -v git >/dev/null 2>&1; then
  echo "error: git is required" >&2
  exit 1
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
bd_safe="$script_dir/bd-safe.sh"

if [[ ! -x "$bd_safe" ]]; then
  echo "error: expected executable $bd_safe (run chmod +x scripts/bd-safe.sh)" >&2
  exit 1
fi

worktree_root="$(git rev-parse --show-toplevel)"
common_root="$("$bd_safe" --print-root)"

worktree_beads="$worktree_root/.beads"
common_beads="$common_root/.beads"
worktree_db="$worktree_beads/beads.db"
common_db="$common_beads/beads.db"
worktree_issues="$worktree_beads/issues.jsonl"
common_issues="$common_beads/issues.jsonl"

cmd="${1:-status}"

line_count_or_zero() {
  local path="$1"
  if [[ -f "$path" ]]; then
    wc -l < "$path" | tr -d ' '
  else
    echo 0
  fi
}

print_status() {
  echo "worktree_root=$worktree_root"
  echo "canonical_root=$common_root"
  echo "canonical_db=$common_db"
  echo "bd_safe_where=$("$bd_safe" where --json | tr '\n' ' ')"

  if [[ "$worktree_root" == "$common_root" ]]; then
    echo "status=main-worktree (local .beads is canonical)"
    return
  fi

  local worktree_lines
  local common_lines
  worktree_lines="$(line_count_or_zero "$worktree_issues")"
  common_lines="$(line_count_or_zero "$common_issues")"

  echo "worktree_issues_jsonl_lines=$worktree_lines"
  echo "canonical_issues_jsonl_lines=$common_lines"

  if [[ -f "$worktree_db" ]]; then
    echo "worktree_local_db=present (stale risk)"
  else
    echo "worktree_local_db=absent"
  fi
}

cleanup_worktree_artifacts() {
  if [[ "$worktree_root" == "$common_root" ]]; then
    echo "cleanup skipped: main worktree uses canonical .beads"
    return
  fi

  local apply=false
  if [[ "${1:-}" == "--apply" ]]; then
    apply=true
  fi

  local -a candidates=(
    "$worktree_beads/beads.db"
    "$worktree_beads/beads.db-shm"
    "$worktree_beads/beads.db-wal"
    "$worktree_beads/sqlite"
    "$worktree_beads/.jsonl.lock"
    "$worktree_beads/daemon.log"
    "$worktree_beads/daemon.pid"
    "$worktree_beads/daemon.lock"
    "$worktree_beads/last-touched"
    "$worktree_beads/export-state"
  )

  local -a present=()
  local path
  for path in "${candidates[@]}"; do
    if [[ -e "$path" ]]; then
      present+=("$path")
    fi
  done

  if [[ "${#present[@]}" -eq 0 ]]; then
    echo "cleanup: no local runtime artifacts found"
    return
  fi

  echo "cleanup candidates:"
  for path in "${present[@]}"; do
    echo "  - $path"
  done

  if [[ "$apply" != true ]]; then
    echo "dry-run only (pass --apply to move files)"
    return
  fi

  local stamp
  stamp="$(date +%Y%m%d-%H%M%S)"
  local backup_dir="$worktree_beads/worktree-artifacts-backup-$stamp"
  mkdir -p "$backup_dir"

  # Track moved files for rollback on failure
  local -a moved=()
  trap 'for f in "${moved[@]}"; do mv "$backup_dir/$(basename "$f")" "$f" 2>/dev/null || true; done' ERR

  for path in "${present[@]}"; do
    mv "$path" "$backup_dir/"
    moved+=("$path")
  done

  trap - ERR
  echo "moved ${#present[@]} artifact(s) to $backup_dir"
}

# Remove subcommand from $@ so only flags (e.g. --apply) remain.
if [[ $# -gt 0 ]]; then shift; fi
case "$cmd" in
  status)
    print_status
    ;;
  cleanup)
    cleanup_worktree_artifacts "$@"
    ;;
  -h|--help|help)
    usage
    ;;
  *)
    echo "error: unknown command '$cmd'" >&2
    usage
    exit 2
    ;;
esac
