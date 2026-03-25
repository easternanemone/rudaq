#!/usr/bin/env bash
# One-time (per clone / machine) setup so `bd dolt push` works.
#
# Background:
# - `bd dolt push` requires a Dolt remote named `origin`.
# - Embedded Dolt also expects a CLI checkout at `.beads/dolt/beads/`; if that
#   directory is missing, remotes may be "SQL only" and `dolt remote -v` fails.
#
# Default: add `origin` as a local file remote under
#   $XDG_DATA_HOME/rust-daq/beads-dolt-origin (fallback: ~/.local/share/...).
# Override for Dolthub / Hosted Dolt:
#   BEADS_DOLT_ORIGIN='https://doltremoteapi.dolthub.com/org/db' bash scripts/ops/setup-beads-dolt-remote.sh
#   BEADS_DOLT_ORIGIN='TheFermiSea/your-db'  # Dolthub short form also works with `dolt remote add`
#
# Hosted Dolt auth: set DOLT_REMOTE_USER and DOLT_REMOTE_PASSWORD if required.
#
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$REPO_ROOT" ]]; then
  echo "error: run from a git checkout (git rev-parse --show-toplevel failed)" >&2
  exit 1
fi

ORIGIN_URL="${BEADS_DOLT_ORIGIN:-}"

if [[ -z "$ORIGIN_URL" ]]; then
  DATA_ROOT="${XDG_DATA_HOME:-$HOME/.local/share}"
  DATA_DIR="$DATA_ROOT/rust-daq/beads-dolt-origin"
  mkdir -p "$DATA_DIR"
  (cd "$DATA_DIR" && [[ -d .dolt ]] || dolt init)
  ORIGIN_URL="file://$(cd "$DATA_DIR" && pwd -P)"
fi

# CLI parity: bd runs `dolt remote` from this subdirectory when present.
CLI_DIR="$REPO_ROOT/.beads/dolt/beads"
mkdir -p "$CLI_DIR"
(cd "$CLI_DIR" && [[ -d .dolt ]] || dolt init)

cd "$REPO_ROOT"

if bd dolt remote list --json 2>/dev/null | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(1)
sys.exit(0 if any(r.get("name") == "origin" for r in data) else 1)
' 2>/dev/null; then
  echo "bd dolt remote 'origin' is already configured."
else
  bd dolt remote add origin "$ORIGIN_URL"
  echo "Added bd dolt remote 'origin' -> $ORIGIN_URL"
fi

echo "Verifying: bd dolt remote list"
bd dolt remote list

echo ""
echo "Next: bd dolt push   (run after issue changes; hooks may call this automatically)"
