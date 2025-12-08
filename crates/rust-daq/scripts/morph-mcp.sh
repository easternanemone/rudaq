#!/usr/bin/env bash
# Repo-local launcher for Morph MCP server.
# Requires MORPH_API_KEY in env.
#
# Usage:
#   ./scripts/morph-mcp.sh              # Default: edit_file only
#   ./scripts/morph-mcp.sh --all        # All tools enabled
#   ENABLED_TOOLS=all ./scripts/morph-mcp.sh
#
# Tools available with --all:
#   edit_file, read_file, codebase_search, grep_search, list_dir

set -euo pipefail

if [[ -z "${MORPH_API_KEY:-}" ]]; then
  echo "MORPH_API_KEY is not set. Export it before running." >&2
  echo "Get your key at: https://morphllm.com/dashboard/api-keys" >&2
  exit 1
fi

# Parse args
if [[ "${1:-}" == "--all" ]] || [[ "${1:-}" == "-a" ]]; then
  export ENABLED_TOOLS="all"
else
  export ENABLED_TOOLS="${ENABLED_TOOLS:-edit_file}"
fi

echo "Starting Morph MCP (tools: $ENABLED_TOOLS)..." >&2
exec npx -y @morphllm/morphmcp
