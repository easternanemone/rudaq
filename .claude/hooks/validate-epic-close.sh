#!/bin/bash
# Validate that an epic has no incomplete children before allowing `bd close`.

set +e

input=$(cat)
if [[ -z "$input" ]]; then
  input="${CLAUDE_TOOL_INPUT:-}"
fi

command=$(echo "$input" | jq -r '.tool_input.command // .command // empty' 2>/dev/null)

# Only check bd close commands.
if ! echo "$command" | grep -qE '(^|\s|&&|\|)bd\s+close(\s|$)'; then
  exit 0
fi

# Extract the first close target.
close_id=$(echo "$command" | sed -nE 's/.*bd[[:space:]]+close[[:space:]]+([A-Za-z0-9._-]+).*/\1/p' | head -1)
if [[ -z "$close_id" ]]; then
  exit 0
fi

project_dir="${CLAUDE_PROJECT_DIR:-$(pwd)}"
beads_db="$project_dir/.beads/beads.db"
if [[ -f "$beads_db" ]]; then
  BD_CMD=(bd --no-daemon --no-auto-import --db "$beads_db")
else
  BD_CMD=(bd)
fi

# Check if this item has children (epic-like).
children=$("${BD_CMD[@]}" show "$close_id" --json 2>/dev/null | jq -r '.[0].children // empty' 2>/dev/null)
if [[ -z "$children" || "$children" == "null" ]]; then
  exit 0
fi

# Canonical status is `closed`; keep `done` as legacy compatibility.
incomplete=$("${BD_CMD[@]}" list --json 2>/dev/null | jq -r --arg epic "$close_id" '
  [.[] | select(.parent == $epic and .status != "closed" and .status != "done")] | length
' 2>/dev/null)

if [[ -n "$incomplete" && "$incomplete" != "0" ]]; then
  incomplete_list=$("${BD_CMD[@]}" list --json 2>/dev/null | jq -r --arg epic "$close_id" '
    [.[] | select(.parent == $epic and .status != "closed" and .status != "done")]
    | .[] | "\(.id) (\(.status))"
  ' 2>/dev/null | tr '\n' ', ' | sed 's/, $//')

  cat <<JSON
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"Cannot close '$close_id' - $incomplete child issue(s) are not closed: $incomplete_list. Close all children first."}}
JSON
fi

exit 0
