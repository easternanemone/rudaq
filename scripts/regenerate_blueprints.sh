#!/usr/bin/env bash
set -euo pipefail

# Regenerate Rerun blueprints using an isolated venv.
# Usage: scripts/regenerate_blueprints.sh

BLUEPRINT_DIR="crates/daq-server/blueprints"
VENV="$BLUEPRINT_DIR/.venv"

cd "$(dirname "$0")/.."

if [ ! -d "$VENV" ]; then
  python3 -m venv "$VENV"
fi

"$VENV/bin/pip" install -q --upgrade pip
"$VENV/bin/pip" install -q rerun-sdk

"$VENV/bin/python" "$BLUEPRINT_DIR/generate_blueprints.py"

echo "Blueprints regenerated in $BLUEPRINT_DIR"
