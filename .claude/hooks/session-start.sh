#!/bin/bash
# SessionStart: show lightweight task context.

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

# Subagents stay quiet.
if [[ -n "$AGENT_TYPE" ]]; then
  exit 0
fi

PROJECT_DIR="${CLAUDE_PROJECT_DIR:-$(pwd)}"
BEADS_DIR="$PROJECT_DIR/.beads"
BEADS_DB="$BEADS_DIR/beads.db"

if [[ ! -d "$BEADS_DIR" ]] || ! command -v bd &>/dev/null; then
  exit 0
fi

# Worktree-safe default.
if [[ -f "$BEADS_DB" ]]; then
  BD_CMD=(bd --no-daemon --no-auto-import --db "$BEADS_DB")
else
  BD_CMD=(bd)
fi

if ! "${BD_CMD[@]}" where --json >/dev/null 2>&1; then
  BD_CMD=(bd)
fi

IN_PROGRESS=$("${BD_CMD[@]}" list --status in_progress 2>/dev/null | head -3)
if [[ -n "$IN_PROGRESS" ]]; then
  echo "### In Progress"
  echo "$IN_PROGRESS"
  echo ""
fi

READY=$("${BD_CMD[@]}" ready 2>/dev/null | head -3)
if [[ -n "$READY" ]]; then
  echo "### Ready"
  echo "$READY"
  echo ""
fi
