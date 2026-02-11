#!/usr/bin/env bash
# Dispatch Bash PreToolUse checks to only hooks relevant to the command.

set +e

input=$(cat)
command=$(echo "$input" | jq -r '.tool_input.command // empty')

if [[ -z "$command" ]]; then
  exit 0
fi

run_hook() {
  local hook_script="$1"
  local output

  if [[ ! -x "$hook_script" ]]; then
    return 0
  fi

  output=$(printf '%s' "$input" | "$hook_script")
  if [[ -n "$output" ]]; then
    echo "$output"
    return 1
  fi

  return 0
}

if echo "$command" | grep -qE '(^|[[:space:]]|&&|\|)bd[[:space:]]+close([[:space:]]|$)'; then
  run_hook ".claude/hooks/validate-epic-close.sh" || exit 0
  run_hook ".claude/hooks/quality-gate-on-close.sh" || exit 0
fi

if echo "$command" | grep -qE '(^|[[:space:]]|&&|\|)git[[:space:]]+push([[:space:]]|$)'; then
  run_hook ".claude/hooks/pre-push-checks.sh" || exit 0
fi

# Guard: git worktree remove destroys the target directory. If the Bash tool's
# persisted CWD is inside that directory, ALL subsequent Bash commands will
# silently fail (exit 1, no output). Require an explicit cd before removal.
if echo "$command" | grep -qE 'git[[:space:]]+worktree[[:space:]]+remove'; then
  if ! echo "$command" | grep -qE '^cd[[:space:]]'; then
    echo "BLOCKED: 'git worktree remove' without a leading 'cd' will break the Bash tool if the CWD is inside the worktree. Prepend: cd /Users/briansquires/code/rust-daq && git worktree remove ..."
    exit 1
  fi
fi

exit 0
