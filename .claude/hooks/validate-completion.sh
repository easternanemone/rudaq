#!/bin/bash
#
# SubagentStop: Enforce bead lifecycle - work verification
#

INPUT=$(cat)
AGENT_TRANSCRIPT=$(echo "$INPUT" | jq -r '.agent_transcript_path // empty')
MAIN_TRANSCRIPT=$(echo "$INPUT" | jq -r '.transcript_path // empty')
AGENT_ID=$(echo "$INPUT" | jq -r '.agent_id // empty')

[[ -z "$AGENT_TRANSCRIPT" || ! -f "$AGENT_TRANSCRIPT" ]] && echo '{"decision":"approve"}' && exit 0

# === SIMPLE TASK BYPASS ===
# If prompt contains SIMPLE_TASK marker, skip strict validation
# Only check for basic completion signal to prevent 67k+ token waste on trivial fixes
if [[ -n "$MAIN_TRANSCRIPT" && -f "$MAIN_TRANSCRIPT" ]]; then
  HAS_SIMPLE_TASK=$(grep -c "SIMPLE_TASK: true" "$MAIN_TRANSCRIPT" 2>/dev/null || echo "0")
  if [[ "$HAS_SIMPLE_TASK" -gt 0 ]]; then
    # Extract last response (needs to be defined here for early exit)
    SIMPLE_TASK_RESPONSE=$(tail -200 "$AGENT_TRANSCRIPT" | jq -rs '
      [.[] | select(.message?.role == "assistant" and .message?.content != null)
       | .message.content[] | select(.text != null) | .text] | last // ""
    ' 2>/dev/null || echo "")
    # Check for any completion signal
    if echo "$SIMPLE_TASK_RESPONSE" | grep -qiE "(complete|finished|done)"; then
      echo '{"decision":"approve"}' && exit 0
    fi
  fi
fi

# Extract last assistant text response
LAST_RESPONSE=$(tail -200 "$AGENT_TRANSCRIPT" | jq -rs '
  [.[] | select(.message?.role == "assistant" and .message?.content != null)
   | .message.content[] | select(.text != null) | .text] | last // ""
' 2>/dev/null || echo "")

# === LAYER 1: Extract subagent_type from transcript (fail open) ===
SUBAGENT_TYPE=""
if [[ -n "$AGENT_ID" && -n "$MAIN_TRANSCRIPT" && -f "$MAIN_TRANSCRIPT" ]]; then
  PARENT_TOOL_USE_ID=$(grep "\"agentId\":\"$AGENT_ID\"" "$MAIN_TRANSCRIPT" 2>/dev/null | head -1 | jq -r '.parentToolUseID // empty' 2>/dev/null)
  if [[ -n "$PARENT_TOOL_USE_ID" ]]; then
    SUBAGENT_TYPE=$(grep "\"id\":\"$PARENT_TOOL_USE_ID\"" "$MAIN_TRANSCRIPT" 2>/dev/null | \
      grep '"name":"Task"' | \
      jq -r '.message.content[]? | select(.type == "tool_use" and .id == "'"$PARENT_TOOL_USE_ID"'") | .input.subagent_type // empty' 2>/dev/null | \
      head -1)
  fi
fi

# === LAYER 2: Check completion format (backup detection) ===
# More lenient patterns - accept variations to avoid feedback loops
HAS_BEAD_COMPLETE=$(echo "$LAST_RESPONSE" | grep -ciE "(BEAD|bd-).*[Cc]ompl" 2>/dev/null || true)
HAS_WORKTREE_OR_BRANCH=$(echo "$LAST_RESPONSE" | grep -ciE "(worktree|branch|\.worktrees/bd-|origin/bd-)" 2>/dev/null || true)
[[ -z "$HAS_BEAD_COMPLETE" ]] && HAS_BEAD_COMPLETE=0
[[ -z "$HAS_WORKTREE_OR_BRANCH" ]] && HAS_WORKTREE_OR_BRANCH=0

# Determine if this is a supervisor (Layer 1) or has completion format (Layer 2)
IS_SUPERVISOR="false"
[[ "$SUBAGENT_TYPE" == *"supervisor"* ]] && IS_SUPERVISOR="true"

NEEDS_VERIFICATION="false"
[[ "$IS_SUPERVISOR" == "true" ]] && NEEDS_VERIFICATION="true"
[[ "$HAS_BEAD_COMPLETE" -ge 1 && "$HAS_WORKTREE_OR_BRANCH" -ge 1 ]] && NEEDS_VERIFICATION="true"

# Skip verification if not needed
[[ "$NEEDS_VERIFICATION" == "false" ]] && echo '{"decision":"approve"}' && exit 0

# Worker supervisor is exempt
[[ "$SUBAGENT_TYPE" == *"worker"* ]] && echo '{"decision":"approve"}' && exit 0

# === VERIFICATION CHECKS ===

# Check 1: Completion format required for supervisors (relaxed - accept variations)
if [[ "$IS_SUPERVISOR" == "true" ]] && [[ "$HAS_BEAD_COMPLETE" -lt 1 || "$HAS_WORKTREE_OR_BRANCH" -lt 1 ]]; then
  cat << 'EOF'
{"decision":"block","reason":"Work verification failed: completion report missing.\n\nRequired (flexible format):\n- ANY mention of 'complete' or 'completed' with bead ID\n- ANY mention of worktree path or branch name\n\nExamples that work:\n  'Completed bd-xyz in .worktrees/bd-xyz'\n  'BEAD bd-xyz COMPLETE, Worktree: .worktrees/bd-xyz'\n  'Fixed in branch bd-xyz, work complete'"}
EOF
  exit 0
fi

# Extract BEAD_ID from response
BEAD_ID_FROM_RESPONSE=$(echo "$LAST_RESPONSE" | grep -oE "BEAD [A-Za-z0-9._-]+" | head -1 | awk '{print $2}')
IS_EPIC_CHILD="false"
[[ "$BEAD_ID_FROM_RESPONSE" == *"."* ]] && IS_EPIC_CHILD="true"

# Check 2: Comment required (documents work for audit trail)
HAS_COMMENT=$(grep -c '"bd comment\|"command":"bd comment' "$AGENT_TRANSCRIPT" 2>/dev/null) || HAS_COMMENT=0
if [[ "$HAS_COMMENT" -lt 1 ]]; then
  cat << EOF
{"decision":"block","reason":"Work verification failed: no comment on bead '${BEAD_ID_FROM_RESPONSE}'.\n\nWHY: Comments create an audit trail of what was changed and why.\n\nRun: bd comment ${BEAD_ID_FROM_RESPONSE} \"Completed: [brief summary]\"\n\nExample: bd comment ${BEAD_ID_FROM_RESPONSE} \"Fixed overflow with saturating_add\""}
EOF
  exit 0
fi

# Check 3: Worktree verification
REPO_ROOT=$(cd "$(git rev-parse --git-common-dir)/.." 2>/dev/null && pwd)
WORKTREE_PATH="$REPO_ROOT/.worktrees/bd-${BEAD_ID_FROM_RESPONSE}"

if [[ ! -d "$WORKTREE_PATH" ]]; then
  cat << EOF
{"decision":"block","reason":"Work verification failed: worktree not found.\n\nExpected: ${WORKTREE_PATH}\n\nWHY: Supervisors must work in isolated worktrees, not on main.\n\nCreate: git worktree add .worktrees/bd-${BEAD_ID_FROM_RESPONSE} -b bd-${BEAD_ID_FROM_RESPONSE} main"}
EOF
  exit 0
fi

# Check 4: Uncommitted changes
UNCOMMITTED=$(git -C "$WORKTREE_PATH" status --porcelain 2>/dev/null)
if [[ -n "$UNCOMMITTED" ]]; then
  UNCOMMITTED_PREVIEW=$(echo "$UNCOMMITTED" | head -3 | tr '\n' ' ')
  UNCOMMITTED_COUNT=$(echo "$UNCOMMITTED" | wc -l | tr -d ' ')
  cat << EOF
{"decision":"block","reason":"Work verification failed: ${UNCOMMITTED_COUNT} uncommitted file(s).\n\nFiles: ${UNCOMMITTED_PREVIEW}\n\nWHY: All changes must be committed before completion.\n\nRun: cd ${WORKTREE_PATH} && git add -A && git commit -m \"fix: ...\""}
EOF
  exit 0
fi

# Check 5: Remote push
HAS_REMOTE=$(git -C "$WORKTREE_PATH" remote get-url origin 2>/dev/null)
if [[ -n "$HAS_REMOTE" ]]; then
  BRANCH="bd-${BEAD_ID_FROM_RESPONSE}"
  REMOTE_EXISTS=$(git -C "$WORKTREE_PATH" ls-remote --heads origin "$BRANCH" 2>/dev/null)
  if [[ -z "$REMOTE_EXISTS" ]]; then
    cat << EOF
{"decision":"block","reason":"Work verification failed: branch '${BRANCH}' not pushed.\n\nWHY: Work must be pushed for review and merge.\n\nRun: git -C ${WORKTREE_PATH} push -u origin ${BRANCH}"}
EOF
    exit 0
  fi
fi

# Check 6: Bead status
BEAD_STATUS=$(bd show "$BEAD_ID_FROM_RESPONSE" --json 2>/dev/null | jq -r '.[0].status // "unknown"')
EXPECTED_STATUS="inreview"
# Epic children also use inreview (done status not supported in bd)
if [[ "$BEAD_STATUS" != "$EXPECTED_STATUS" ]]; then
  cat << EOF
{"decision":"block","reason":"Work verification failed: bead status is '${BEAD_STATUS}', expected '${EXPECTED_STATUS}'.\n\nWHY: Status 'inreview' signals work is ready for merge.\n\nRun: bd update ${BEAD_ID_FROM_RESPONSE} --status ${EXPECTED_STATUS}"}
EOF
  exit 0
fi

# Check 7: Verbosity limit (relaxed - increased from 15/800 to 50/3000)
# Some fixes need context; this is a reasonable limit that prevents abuse but allows explanation
DECODED_RESPONSE=$(printf '%b' "$LAST_RESPONSE")
LINE_COUNT=$(echo "$DECODED_RESPONSE" | wc -l | tr -d ' ')
CHAR_COUNT=${#DECODED_RESPONSE}

if [[ "$LINE_COUNT" -gt 50 ]] || [[ "$CHAR_COUNT" -gt 3000 ]]; then
  cat << EOF
{"decision":"block","reason":"Work verification failed: response too verbose (${LINE_COUNT} lines, ${CHAR_COUNT} chars). Max: 50 lines, 3000 chars.\n\nWHY: Concise reports reduce token waste. Details go in bead comments.\n\nFormat:\n  BEAD {ID} COMPLETE\n  Worktree: .worktrees/bd-{ID}\n  Files: [names]\n  Summary: [1 line]"}
EOF
  exit 0
fi

echo '{"decision":"approve"}'
