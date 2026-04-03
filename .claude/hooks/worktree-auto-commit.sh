#!/usr/bin/env bash
# PostToolUse hook: auto-commit file changes in agent worktrees.
#
# When an Edit or Write tool modifies files inside an agent worktree,
# this hook immediately commits and pushes the change. This prevents
# work loss when the worktree is auto-cleaned after the agent exits.
#
# Without this, agents that edit files but forget to `git commit` lose
# all their work when the worktree directory is removed.

set +e

input=$(cat)
tool_name=$(echo "$input" | jq -r '.tool_name // empty')

# Only act on file-modifying tools
case "$tool_name" in
    Edit|Write) ;;
    *) exit 0 ;;
esac

# Determine which file was modified
file_path=$(echo "$input" | jq -r '.tool_input.file_path // empty')
if [[ -z "$file_path" ]]; then
    exit 0
fi

# Check if the file is inside an agent worktree
case "$file_path" in
    */.claude/worktrees/agent-*) ;;
    *) exit 0 ;;
esac

# Extract the worktree root (up to and including the agent-XXXX directory)
worktree_dir=$(echo "$file_path" | sed 's|\(.*/.claude/worktrees/agent-[^/]*\)/.*|\1|')

if [[ ! -d "$worktree_dir/.git" && ! -f "$worktree_dir/.git" ]]; then
    exit 0
fi

# Auto-commit the changed file (synchronous — must complete before worktree cleanup)
(
    cd "$worktree_dir" || exit 0
    git add "$file_path" 2>/dev/null

    # Only commit if there are staged changes
    if ! git diff --cached --quiet 2>/dev/null; then
        rel_path="${file_path#$worktree_dir/}"
        git commit -m "auto: save agent edit to $rel_path" --no-verify 2>/dev/null

        # Push synchronously (NOT in background — must finish before cleanup)
        branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null)
        if [[ -n "$branch" && "$branch" != "HEAD" ]]; then
            git push origin "HEAD:$branch" 2>/dev/null
        fi
    fi
)

exit 0
