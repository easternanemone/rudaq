#!/usr/bin/env bash
# Build the mcp-accesskit MCP server (Swift bridge + TypeScript server)
set -euo pipefail
cd "$(dirname "$0")"

echo "==> Compiling Swift AX bridge..."
swiftc -O -o ax-bridge ax-bridge.swift -framework Cocoa -framework ApplicationServices

echo "==> Installing npm dependencies..."
npm install --silent

echo "==> Compiling TypeScript..."
npx tsc

echo "==> Done. Start with: node dist/index.js"
