#!/bin/bash
# SubagentStart: Create marker when subagent spawns
INPUT=$(cat)
SESSION_ID=$(echo "$INPUT" | jq -r '.session_id')
AGENT_ID=$(echo "$INPUT" | jq -r '.agent_id')

MARKER_DIR="/tmp/claude-subagents/$SESSION_ID"
mkdir -p "$MARKER_DIR"
touch "$MARKER_DIR/$AGENT_ID"
exit 0
