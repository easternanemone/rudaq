#!/bin/bash
# SubagentStop: Remove marker when subagent finishes
INPUT=$(cat)
SESSION_ID=$(echo "$INPUT" | jq -r '.session_id')
AGENT_ID=$(echo "$INPUT" | jq -r '.agent_id')

rm -f "/tmp/claude-subagents/$SESSION_ID/$AGENT_ID"
rmdir "/tmp/claude-subagents/$SESSION_ID" 2>/dev/null
exit 0
