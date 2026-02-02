#!/bin/bash
#
# PreToolUse: Block orchestrator from implementation tools
#
# Orchestrators investigate and delegate - they don't implement.
# But they CAN use basic CLI tools for workflow operations.
#

INPUT=$(cat)
TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // empty')

# Always allow Task (delegation)
[[ "$TOOL_NAME" == "Task" ]] && exit 0

# Detect SUBAGENT context by checking if we're in a worktree
# Supervisors work in .worktrees/bd-*/ directories
CWD=$(echo "$INPUT" | jq -r '.cwd // empty')
if [[ -z "$CWD" ]]; then
  CWD=$(pwd)
fi

# Check if cwd contains .worktrees/bd- pattern (supervisor worktree)
if [[ "$CWD" == *".worktrees/bd-"* ]]; then
  exit 0  # Allow all tools for supervisors in worktrees
fi

# Also check tool_input.file_path for Edit/Write operations
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // empty')
if [[ "$FILE_PATH" == *".worktrees/bd-"* ]]; then
  exit 0  # Allow edits to worktree files
fi

# Allow Edit/Write for .claude/ configuration files (hooks, settings, etc.)
if [[ "$FILE_PATH" == *"/.claude/"* ]]; then
  exit 0  # Allow orchestrator to manage Claude configuration
fi

# Allow Edit/Write for documentation files (scribe can edit docs without worktree)
if [[ "$FILE_PATH" == */docs/* ]] || [[ "$FILE_PATH" == *.md ]]; then
  exit 0  # Allow documentation edits
fi

# DENYLIST: Block implementation tools for orchestrator (code files only)
BLOCKED="Edit|Write|NotebookEdit"

if [[ "$TOOL_NAME" =~ ^($BLOCKED)$ ]]; then
  echo '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"Tool '"'$TOOL_NAME'"' blocked for code files. Orchestrators investigate and delegate via Task(). Supervisors implement."}}'
  exit 0
fi

# Validate provider_delegator agent invocations - block implementation agents
if [[ "$TOOL_NAME" == "mcp__provider_delegator__invoke_agent" ]]; then
  AGENT=$(echo "$INPUT" | jq -r '.tool_input.agent // empty')
  CODEX_ALLOWED="scout|detective|architect|scribe|code-reviewer"

  if [[ ! "$AGENT" =~ ^($CODEX_ALLOWED)$ ]]; then
    echo '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"Agent '"'$AGENT'"' cannot be invoked via Codex. Implementation agents (*-supervisor, discovery) must use Task() with BEAD_ID for beads workflow."}}'
    exit 0
  fi
fi

# Validate Bash commands for orchestrator
if [[ "$TOOL_NAME" == "Bash" ]]; then
  COMMAND=$(echo "$INPUT" | jq -r '.tool_input.command // empty')
  FIRST_WORD="${COMMAND%% *}"

  # ALLOW git commands (orchestrator can manage git workflow)
  if [[ "$FIRST_WORD" == "git" ]]; then
    exit 0
  fi

  # ALLOW gh (GitHub CLI) commands
  if [[ "$FIRST_WORD" == "gh" ]]; then
    exit 0
  fi

  # ALLOW beads commands (with validation)
  if [[ "$FIRST_WORD" == "bd" ]]; then
    SECOND_WORD=$(echo "$COMMAND" | awk '{print $2}')

    # Validate bd create requires description
    if [[ "$SECOND_WORD" == "create" ]] || [[ "$SECOND_WORD" == "new" ]]; then
      if [[ "$COMMAND" != *"-d "* ]] && [[ "$COMMAND" != *"--description "* ]] && [[ "$COMMAND" != *"--description="* ]]; then
        echo '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"bd create requires description (-d or --description) for supervisor context."}}'
        exit 0
      fi
    fi

    exit 0
  fi

  # Allow other bash commands (cargo, npm, rg, sg, fd, ls, etc. for investigation)
  exit 0
fi

# Allow everything else
exit 0
