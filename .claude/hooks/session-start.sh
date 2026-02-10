#!/bin/bash
# SessionStart: show lightweight task context and tool health.

INPUT=$(cat)
TRANSCRIPT_PATH=$(echo "$INPUT" | jq -r '.transcript_path // empty')
AGENT_TYPE=$(echo "$INPUT" | jq -r '.agent_type // empty')

if [[ -n "$TRANSCRIPT_PATH" ]]; then
  SESSION_DIR="${TRANSCRIPT_PATH%.jsonl}"
  MARKER_FILE="$SESSION_DIR/.is_subagent"
  if [[ -n "$AGENT_TYPE" ]]; then
    mkdir -p "$SESSION_DIR"
    echo "$AGENT_TYPE" > "$MARKER_FILE"
  fi
fi

# Subagents should stay quiet.
if [[ -n "$AGENT_TYPE" ]]; then
  exit 0
fi

PROJECT_DIR="${CLAUDE_PROJECT_DIR:-$(pwd)}"
BEADS_DIR="$PROJECT_DIR/.beads"
BEADS_DB="$BEADS_DIR/beads.db"

if [[ ! -d "$BEADS_DIR" ]]; then
  echo "No .beads directory found. Run 'bd init' to initialize."
  exit 0
fi

if ! command -v bd &>/dev/null; then
  echo "beads CLI (bd) not found. Install from: https://github.com/steveyegge/beads"
  exit 0
fi

# Worktree-safe default from AGENTS.md.
if [[ -f "$BEADS_DB" ]]; then
  BD_CMD=(bd --no-daemon --no-auto-import --db "$BEADS_DB")
else
  BD_CMD=(bd)
fi

# Health check: ensure configured bd command is usable.
if ! "${BD_CMD[@]}" where --json >/dev/null 2>&1; then
  BD_CMD=(bd)
fi

if command -v grepai &>/dev/null; then
  if ! grepai status >/dev/null 2>&1; then
    echo "grepai: backend is unavailable (semantic search degraded)."
    echo "  Try: grepai watch --background"
    echo "  Check: .grepai/config.yaml backend/port settings"
    echo ""
  fi
else
  echo "grepai: not installed (fall back to rg/grep for code search)."
  echo ""
fi

REPO_ROOT=$(git -C "$PROJECT_DIR" rev-parse --show-toplevel 2>/dev/null)
WORKTREES_DIR="$PROJECT_DIR/.worktrees"
if [[ -d "$WORKTREES_DIR" && -n "$REPO_ROOT" ]]; then
  while IFS= read -r worktree; do
    [[ -z "$worktree" ]] && continue
    BEAD_ID=$(basename "$worktree" | sed 's/bd-//')
    BRANCH=$(basename "$worktree")

    if git -C "$REPO_ROOT" branch --merged main 2>/dev/null | grep -q "$BRANCH"; then
      echo "$BRANCH was merged; consider cleanup:"
      echo "  git worktree remove \"$worktree\" && bd close \"$BEAD_ID\" --reason \"Completed\" --json"
      echo ""
    fi
  done < <(git -C "$REPO_ROOT" worktree list --porcelain 2>/dev/null | awk '/^worktree .*\.worktrees\/bd-/{print $2}')
fi

if command -v gh &>/dev/null; then
  OPEN_PRS=$(gh pr list --author "@me" --state open --json number,title,headRefName 2>/dev/null)
  if [[ -n "$OPEN_PRS" && "$OPEN_PRS" != "[]" ]]; then
    echo "Open PRs:"
    echo "$OPEN_PRS" | jq -r '.[] | "  #\(.number) \(.title) (\(.headRefName))"' 2>/dev/null
    echo ""
  fi
fi

echo ""
echo "## Task Status"
echo ""

IN_PROGRESS=$("${BD_CMD[@]}" list --status in_progress 2>/dev/null | head -5)
if [[ -n "$IN_PROGRESS" ]]; then
  echo "### In Progress"
  echo "$IN_PROGRESS"
  echo ""
fi

READY=$("${BD_CMD[@]}" ready 2>/dev/null | head -5)
if [[ -n "$READY" ]]; then
  echo "### Ready"
  echo "$READY"
  echo ""
fi

BLOCKED=$("${BD_CMD[@]}" blocked 2>/dev/null | head -3)
if [[ -n "$BLOCKED" ]]; then
  echo "### Blocked"
  echo "$BLOCKED"
  echo ""
fi

STALE=$("${BD_CMD[@]}" stale --days 3 2>/dev/null | head -3)
if [[ -n "$STALE" ]]; then
  echo "### Stale (3+ days)"
  echo "$STALE"
  echo ""
fi

if [[ -z "$IN_PROGRESS" && -z "$READY" && -z "$BLOCKED" && -z "$STALE" ]]; then
  echo "No active beads. Create one with: bd create \"Task title\" -t task -p 2 --json"
fi

echo ""
